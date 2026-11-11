//! `GET /watch/{source}` — a stream that says when a source changed
//! (issue #328).
//!
//! **Notification only.** The server says "this source changed, ask again"; it
//! sends no rows, so nothing a row filter keeps from a caller can travel on it,
//! and the client asks the ordinary `/query` — with its token, row filter and
//! allowlist — for what it shows. The stream needs the same bearer token as
//! every other endpoint and names a source the registry knows.
//!
//! The wire is Server-Sent Events: `event: changed` with `{"source": name}`,
//! a comment every 20 seconds to keep proxies from closing an idle stream, and
//! `retry: 5000` so a client that is cut off asks again after five seconds.
//! The application says when a source changed with
//! [`Server::notify_changed`](crate::Server::notify_changed); the server cannot
//! know, since the data lives behind a connector.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use http_body_util::channel::Channel;
use tokio::sync::broadcast::error::RecvError;

use crate::api::{AppState, Failure, authorize};
use opengrid_datasource::wire::{ErrorCode, WireError};

/// Pieces waiting for a slow reader; a client that stops reading is dropped.
const BUFFERED_PIECES: usize = 8;
const KEEP_ALIVE: Duration = Duration::from_secs(20);

#[derive(Debug)]
pub(crate) struct Gone;

impl std::fmt::Display for Gone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the watching client is gone")
    }
}

impl std::error::Error for Gone {}

pub(crate) async fn watch(
    State(state): State<Arc<AppState>>,
    Path(source_name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, Failure> {
    authorize(&state, &headers)?;
    if state.registry.get(&source_name).is_none() {
        return Err(WireError::new(
            ErrorCode::UnknownSource,
            format!("unknown source {source_name:?}"),
        )
        .into());
    }

    let mut changes = state.changes.subscribe();
    let (mut sender, channel) = Channel::<Bytes, Gone>::new(BUFFERED_PIECES);
    tokio::spawn(async move {
        let ready = Bytes::from_static(b"retry: 5000\n\n: ready\n\n");
        if sender.send_data(ready).await.is_err() {
            return;
        }
        loop {
            let piece = tokio::select! {
                changed = changes.recv() => match changed {
                    Ok(name) if name == source_name => Some(changed_event(&name)),
                    Ok(_) => None,
                    // Missed some: the client asks again either way.
                    Err(RecvError::Lagged(_)) => Some(changed_event(&source_name)),
                    Err(RecvError::Closed) => return,
                },
                () = tokio::time::sleep(KEEP_ALIVE) => {
                    Some(Bytes::from_static(b": keep-alive\n\n"))
                }
            };
            if let Some(piece) = piece
                && sender.send_data(piece).await.is_err()
            {
                return;
            }
        }
    });

    let mut response = Response::new(Body::new(channel));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    Ok(response.into_response())
}

fn changed_event(source: &str) -> Bytes {
    let data = opengrid_json::json!({ "source": source });
    Bytes::from(format!("event: changed\ndata: {data}\n\n"))
}
