//! The thread SQLite runs on (issue #217).
//!
//! SQLite is a library: a query runs on the thread that calls it, to the end.
//! Run on the async thread, one slow query would hold a server worker and every
//! other request, and the server's timeout could not fire before the call
//! returned. So the connection lives on a thread of its own and a query is a
//! job sent to it; the caller gets a future.
//!
//! **A request that is dropped ends its query.** When the future is dropped
//! before the answer came — the server's timeout, a client that left — the job
//! is skipped if it has not started, and interrupted (`sqlite3_interrupt`) if it
//! runs. The interrupt is only sent while that job is the one running, so it
//! cannot reach a query of somebody else.
//!
//! No async runtime is involved: the future is woken through its `Waker`, so
//! any executor drives it.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use opengrid_connector::{DataSourceError, QueryResult};
use rusqlite::{Connection, InterruptHandle};

type Outcome = Result<QueryResult, DataSourceError>;

/// What the caller waits on, and what the thread fills.
#[derive(Default)]
struct Slot {
    outcome: Option<Outcome>,
    waker: Option<Waker>,
}

struct Job {
    run: Box<dyn FnOnce(&Connection) -> Outcome + Send>,
    reply: Arc<Mutex<Slot>>,
    cancelled: Arc<AtomicBool>,
    /// Which job this is, for the interrupt (see [`Running`]).
    id: u64,
}

/// The job the thread is working on, if any. Held while an interrupt is sent
/// and while the thread moves on, so an interrupt never lands on the next job.
type Running = Arc<Mutex<Option<u64>>>;

/// The connection's thread. Dropping it ends the thread and closes the
/// connection.
pub(crate) struct Worker {
    jobs: Sender<Job>,
    interrupt: Arc<InterruptHandle>,
    running: Running,
    next_id: std::sync::atomic::AtomicU64,
}

impl Worker {
    pub(crate) fn start(connection: Connection) -> Self {
        let (jobs, queue) = channel::<Job>();
        let interrupt = Arc::new(connection.get_interrupt_handle());
        let running: Running = Arc::default();
        let on_thread = Arc::clone(&running);
        std::thread::Builder::new()
            .name("opengrid-sqlite".to_owned())
            .spawn(move || {
                for job in queue {
                    *on_thread.lock().expect("not poisoned") = Some(job.id);
                    if !job.cancelled.load(Ordering::SeqCst) {
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                (job.run)(&connection)
                            }))
                            .unwrap_or_else(|_| {
                                Err(DataSourceError::Backend {
                                    message: "SQLite: the query panicked".to_owned(),
                                })
                            });
                        let mut slot = job.reply.lock().expect("not poisoned");
                        slot.outcome = Some(outcome);
                        if let Some(waker) = slot.waker.take() {
                            waker.wake();
                        }
                    }
                    *on_thread.lock().expect("not poisoned") = None;
                }
            })
            .expect("a thread");
        Self {
            jobs,
            interrupt,
            running,
            next_id: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Runs `work` on the connection's thread, behind the jobs sent before it.
    pub(crate) fn submit(
        &self,
        work: impl FnOnce(&Connection) -> Outcome + Send + 'static,
    ) -> Answer {
        let reply: Arc<Mutex<Slot>> = Arc::default();
        let cancelled = Arc::new(AtomicBool::new(false));
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let sent = self.jobs.send(Job {
            run: Box::new(work),
            reply: Arc::clone(&reply),
            cancelled: Arc::clone(&cancelled),
            id,
        });
        if sent.is_err() {
            reply.lock().expect("not poisoned").outcome = Some(Err(DataSourceError::Backend {
                message: "SQLite: the connection's thread has ended".to_owned(),
            }));
        }
        Answer {
            reply,
            cancelled,
            id,
            interrupt: Arc::clone(&self.interrupt),
            running: Arc::clone(&self.running),
            done: false,
        }
    }
}

/// The answer to a job. Dropped before it is ready, it cancels the job.
pub(crate) struct Answer {
    reply: Arc<Mutex<Slot>>,
    cancelled: Arc<AtomicBool>,
    id: u64,
    interrupt: Arc<InterruptHandle>,
    running: Running,
    done: bool,
}

impl Future for Answer {
    type Output = Outcome;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Outcome> {
        let taken = {
            let mut slot = self.reply.lock().expect("not poisoned");
            match slot.outcome.take() {
                Some(outcome) => Some(outcome),
                None => {
                    slot.waker = Some(context.waker().clone());
                    None
                }
            }
        };
        match taken {
            Some(outcome) => {
                self.done = true;
                Poll::Ready(outcome)
            }
            None => Poll::Pending,
        }
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        self.cancelled.store(true, Ordering::SeqCst);
        // Under the lock the thread takes to move on: the job is either the
        // running one (interrupt it) or not running (nothing to interrupt).
        let running = self.running.lock().expect("not poisoned");
        if *running == Some(self.id) {
            self.interrupt.interrupt();
        }
    }
}
