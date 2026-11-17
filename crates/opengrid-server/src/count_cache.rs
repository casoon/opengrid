//! The count a window repeats (issue #457).
//!
//! A reader scrolling asks for window after window of one query; only `offset`
//! changes, and with it nothing about how many rows match. The database would
//! count them again each time — 168 ms of a 181 ms window on ten million rows.
//! The server remembers the count per source and per query without its paging
//! and sorting, and tells the connector, which then skips its second statement.
//!
//! What drops an entry: [`Server::notify_changed`](crate::Server::notify_changed)
//! for that source, and its age — the server cannot know when a database
//! changes behind its back, so an entry lives `ttl` and no longer. A `ttl` of
//! zero switches the cache off.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use opengrid_query::ValidatedQuery;

/// How many counts are kept at most; the oldest goes first.
const ENTRIES: usize = 256;

pub(crate) struct CountCache {
    ttl: Duration,
    entries: Mutex<HashMap<(String, String), (u64, Instant)>>,
}

impl CountCache {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// What the count depends on: the filter (the row filter of the token is
    /// part of it), the grouping and the aggregates. Not the sort, the page or
    /// the columns selected. A tree's count is its own business: no key.
    fn key(query: &ValidatedQuery) -> Option<String> {
        if query.tree.is_some() {
            return None;
        }
        Some(format!(
            "{:?}|{:?}|{:?}",
            query.filter, query.group, query.aggregate
        ))
    }

    pub(crate) fn get(&self, source: &str, query: &ValidatedQuery) -> Option<u64> {
        if self.ttl.is_zero() {
            return None;
        }
        let key = (source.to_owned(), Self::key(query)?);
        let entries = self.entries.lock().ok()?;
        let (count, stored) = entries.get(&key)?;
        (stored.elapsed() < self.ttl).then_some(*count)
    }

    pub(crate) fn put(&self, source: &str, query: &ValidatedQuery, count: u64) {
        if self.ttl.is_zero() {
            return;
        }
        let Some(key) = Self::key(query) else { return };
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let ttl = self.ttl;
        if entries.len() >= ENTRIES {
            entries.retain(|_, (_, stored)| stored.elapsed() < ttl);
        }
        if entries.len() >= ENTRIES {
            let oldest = entries
                .iter()
                .min_by_key(|(_, (_, stored))| *stored)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert((source.to_owned(), key), (count, Instant::now()));
    }

    /// The source changed: its counts are not to be trusted any more.
    pub(crate) fn forget(&self, source: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|(name, _), _| name != source);
        }
    }
}
