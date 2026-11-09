//! A pooled connection that cancels what it runs when its request is dropped
//! (issue #217).
//!
//! The server drops a query's future when its timeout runs out or the client
//! leaves. Dropping a `tokio-postgres` future does not stop the statement: it
//! keeps running in the database, holding a pooled connection and its CPU,
//! while the client sends the next request. A [`Lease`] that is dropped
//! without [`release`](Lease::release) — which only a dropped future does —
//! sends a cancel request for its connection and then **discards** the
//! connection instead of handing it back: a cancel that arrived after the
//! connection was reused would end somebody else's statement.

use deadpool_postgres::Object;
use tokio_postgres::{CancelToken, NoTls};

pub(crate) struct Lease {
    client: Option<Object>,
    cancel: CancelToken,
}

impl Lease {
    pub(crate) fn new(client: Object) -> Self {
        let cancel = client.cancel_token();
        Self {
            client: Some(client),
            cancel,
        }
    }

    pub(crate) fn client(&mut self) -> &mut Object {
        self.client.as_mut().expect("held until released")
    }

    /// The work is over, whatever its outcome: the connection goes back to the
    /// pool (when the caller drops it) and nothing is cancelled.
    pub(crate) fn release(mut self) -> Object {
        self.client.take().expect("held until released")
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let Some(client) = self.client.take() else {
            return;
        };
        // Outside a runtime there is nothing to send the request from; the
        // connection goes back as it is.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let cancel = self.cancel.clone();
        runtime.spawn(async move {
            let _ = cancel.cancel_query(NoTls).await;
            // Closes the connection; the pool opens a new one when needed.
            drop(Object::take(client));
        });
    }
}
