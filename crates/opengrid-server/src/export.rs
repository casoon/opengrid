//! `POST /export/{source}` — every row of a query's answer, streamed as a file
//! (issue #2, E33).
//!
//! The request is the one `POST /query` takes: the same body, the same token,
//! and the same path through [`admit`] — the client schema narrowed to
//! `allowed_fields`, the mandatory row filter and-ed in, both validations. The
//! only rule that differs is the bound: `max_export_rows` takes the place of
//! `max_limit`, because an export is every match, not a page.
//!
//! # Format and options
//!
//! `?format=csv` or `?format=json`; without it, whichever of `text/csv` and
//! `application/json` the `Accept` header prefers; without either, CSV. The CSV
//! options are parameters too, spelled as `exportRows` spells them —
//! `delimiter`, `bom`, `protectFormulas`, `null` — and checked with
//! [`CsvOptions::check`] before anything runs. An unknown parameter, and a CSV
//! option on a JSON export, are refused rather than ignored.
//!
//! # Before the first byte
//!
//! Everything that can still be a status code is decided before the first byte
//! goes out: the request, the count, and the first piece. An export with more
//! rows than `max_export_rows` is a `413` then — **counted, not cut off**: a
//! limit in the cursor would only notice after that many rows had been sent,
//! too late for a status, and the file would end short without saying so. The
//! count and the rows come from one snapshot (see `opengrid-datasource-postgres`),
//! so the number in [`ROWS_HEADER`] is the number of rows that come.
//!
//! `timeout_ms` bounds the time to the first byte and then **each** fetch from
//! the source; the whole length is bounded by `max_export_rows`, not by a
//! clock — a million rows to a slow client may take longer than any one query.
//!
//! # Streaming
//!
//! The rows are read a piece at a time and written through `opengrid-export`
//! into a bounded channel that is the response body. The channel holds at most
//! [`BUFFERED_PIECES`] pieces; when the client reads slowly, the next fetch
//! waits, so the server holds a few pieces whatever the export's length.
//!
//! A client that goes away is noticed at the next piece: the send fails, the
//! export is dropped, and PostgreSQL's transaction and cursor end with it.
//!
//! # Three bounds on what an export holds
//!
//! An export holds a pooled connection and a snapshot (its `xmin`, which keeps
//! `VACUUM` from removing rows that are dead since it began) for as long as it
//! runs, and a client decides how long that is. So:
//!
//! 1. **Each piece must be taken within `timeout_ms`.** A client that stops
//!    reading keeps the channel full; when a piece waits longer than that, the
//!    body is broken off and the export dropped — connection out of the pool,
//!    transaction and cursor gone.
//! 2. **At most `max_concurrent_exports` at once** (its default in
//!    `Registry::default_concurrent_exports`, why in
//!    `docs/guides/where-queries-run.md`, "For operators"). One more is a `503`,
//!    code `busy`, before any database work, so exports never take the connections `/query`
//!    needs.
//! 3. **PostgreSQL's own backstop**: the transaction sets
//!    `idle_in_transaction_session_timeout` to [`BACKSTOP_FACTOR`] times
//!    `timeout_ms`, and the database ends it if the server's own bound ever
//!    fails.
//!
//! The local engine holds its whole answer (Arrow, not values) for the length
//! of the download instead of a cursor; the same bounds apply to it.
//!
//! # Never a short file that looks whole
//!
//! A failure **after** the first byte cannot be a status any more; the body is
//! broken off instead, so the connection ends without the end of a chunked
//! body and a client can never mistake half a file for a whole one. The body's
//! sender lives in an [`Outlet`] that breaks the body off when it is dropped
//! without having reached the end — a panic after the first byte included.
//! That holds end to end only over HTTP/1.1 chunked or HTTP/2: a proxy that
//! speaks HTTP/1.0 to the client turns the break into an ordinary end of file
//! (for nginx: `proxy_http_version 1.1`).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query as Parameters, State};
use axum::http::{HeaderMap, HeaderValue, Uri, header};
use axum::response::Response;
use http_body_util::channel::{Channel, Sender};
use opengrid_datasource::wire::{ErrorCode, WireError};
use opengrid_datasource::{DataSourceError, QueryResult};
use opengrid_datasource_postgres::ExportCanceller;
use opengrid_export::{CsvOptions, CsvWriter, JsonWriter};
use opengrid_query::{Limits, ValidatedQuery};
use tokio::sync::{OwnedSemaphorePermit, oneshot};

use crate::api::{AppState, Failure, admit, source_failed};
use crate::registry::Source;

/// Rows read from the source at a time — the `max_limit` a page has by default,
/// and the `chunkSize` `exportRows` fetches in the browser.
const PIECE_ROWS: usize = 10_000;

/// Pieces the body holds ahead of the client. With the one being written and
/// the one being read, this is all an export keeps in memory.
const BUFFERED_PIECES: usize = 2;

/// PostgreSQL's backstop, in multiples of `timeout_ms`. The server bounds each
/// pause in the transaction itself — a piece taken within `timeout_ms`, then a
/// piece written, which takes milliseconds — so twice that is only reached
/// when the server's own bound has failed.
const BACKSTOP_FACTOR: u32 = 2;

/// The response header that carries the number of rows, known before the
/// first byte — a client's progress and its own bound read it.
pub(crate) const ROWS_HEADER: &str = "x-total-count";

/// The CSV options as parameters: the names `exportRows` and `get_pivot` take,
/// so a page spells them one way. A test in `opengrid-web-components`
/// (`api.rs`) compares this list with the loader's.
const CSV_PARAMETERS: [&str; 4] = ["delimiter", "bom", "protectFormulas", "null"];

/// What the export is written as.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Format {
    Csv(CsvOptions),
    Json,
}

impl Format {
    fn content_type(&self) -> &'static str {
        match self {
            Format::Csv(_) => "text/csv; charset=utf-8",
            Format::Json => "application/json",
        }
    }

    fn extension(&self) -> &'static str {
        match self {
            Format::Csv(_) => "csv",
            Format::Json => "json",
        }
    }
}

/// `POST /export/{source}`.
pub(crate) async fn export(
    State(state): State<Arc<AppState>>,
    Path(source_name): Path<String>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    let limits = Limits::new(state.max_export_rows, state.registry.limits.max_depth);
    let (source, query) = admit(&state, &source_name, &headers, &body, &limits)?;
    let format = format_of(&uri, headers.get(header::ACCEPT))?;

    // Before any database work: an export over the bound costs nothing.
    let Ok(permit) = Arc::clone(&state.exports).try_acquire_owned() else {
        // `busy`, a `503`: the same export may pass in a moment.
        return Err(WireError::new(
            ErrorCode::Busy,
            format!(
                "{} exports are running, the most this server runs at once \
                 (max_concurrent_exports); try again later",
                state.max_concurrent_exports
            ),
        )
        .into());
    };

    let (ready, started) = oneshot::channel();
    let (sender, channel) = Channel::<Bytes, Broken>::new(BUFFERED_PIECES);
    tokio::spawn(stream(
        Arc::clone(&source),
        query,
        format.clone(),
        state.max_export_rows,
        state.timeout,
        ready,
        Outlet::new(sender),
        permit,
    ));

    // Dropping `started` — on the timeout, or when the client leaves before
    // the answer — is what tells the task to give up and cancel its statement.
    let rows = match tokio::time::timeout(state.timeout, started).await {
        Err(_) => {
            return Err(WireError::new(
                ErrorCode::LimitExceeded,
                format!(
                    "the export took longer than {} ms to start",
                    state.timeout.as_millis()
                ),
            )
            .into());
        }
        // The task ended without a word, which only a panic does.
        Ok(Err(_)) => {
            return Err(WireError::new(ErrorCode::Backend, "the export failed to start").into());
        }
        Ok(Ok(started)) => started?,
    };

    let mut response = Response::new(Body::new(channel));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(format.content_type()),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        content_disposition(&source.name, format.extension()),
    );
    headers.insert(ROWS_HEADER, HeaderValue::from(rows));
    Ok(response)
}

/// The export itself, in its own task: the handler waits for its first piece
/// (or its failure) through `ready`, and the rest goes to the body. `_permit`
/// is its place among `max_concurrent_exports`, given back when the task ends.
#[allow(clippy::too_many_arguments)]
async fn stream(
    source: Arc<Source>,
    query: ValidatedQuery,
    format: Format,
    max_rows: u64,
    timeout: Duration,
    mut ready: oneshot::Sender<Result<u64, Failure>>,
    mut outlet: Outlet,
    _permit: OwnedSemaphorePermit,
) {
    // Before the first byte: every step races the handler, which gives up on
    // the timeout or when the client leaves — `ready.closed()` resolves then.
    // A failure is still a status then, and a clean one hands the connection
    // back (`close`); a step given up on drops it.
    let backstop = timeout * BACKSTOP_FACTOR;
    let mut rows = match step(source.data.export(&query, backstop), ready.closed(), None).await {
        None => return,
        Some(Err(error)) => {
            let _ = ready.send(Err(source_failed(error).into()));
            return;
        }
        Some(Ok(rows)) => rows,
    };
    let canceller = rows.canceller();

    let count = match step(rows.count(), ready.closed(), canceller.as_ref()).await {
        None => return,
        Some(Err(error)) => {
            let _ = ready.send(Err(source_failed(error).into()));
            rows.close().await;
            return;
        }
        Some(Ok(count)) => count,
    };
    if count > max_rows {
        let _ = ready.send(Err(WireError::new(
            ErrorCode::LimitExceeded,
            format!(
                "the export has {count} rows, more than the {max_rows} allowed (max_export_rows)"
            ),
        )
        .into()));
        rows.close().await;
        return;
    }

    let first = match step(
        rows.next_piece(PIECE_ROWS),
        ready.closed(),
        canceller.as_ref(),
    )
    .await
    {
        None => return,
        Some(Err(error)) => {
            let _ = ready.send(Err(source_failed(error).into()));
            rows.close().await;
            return;
        }
        Some(Ok(first)) => first,
    };

    let mut writer = Writer::new(format);
    let mut last = first.row_count() < PIECE_ROWS;
    // The channel is empty, so this does not wait for the client.
    if outlet.send(writer.write(&first), timeout).await.is_err() {
        return;
    }
    drop(first);
    if ready.send(Ok(count)).is_err() {
        // The handler gave up in the meantime; dropping `rows` ends the export.
        return;
    }

    // After the first byte: each fetch has the timeout to itself, and so has
    // the client to take each piece. Any return from here on without
    // `finish` breaks the body off (`Outlet`).
    while !last {
        let piece = match step(
            rows.next_piece(PIECE_ROWS),
            tokio::time::sleep(timeout),
            canceller.as_ref(),
        )
        .await
        {
            Some(Ok(piece)) => piece,
            Some(Err(error)) => {
                outlet.abort(source_failed(error).message);
                return;
            }
            None => {
                outlet.abort(format!(
                    "a fetch took longer than {} ms",
                    timeout.as_millis()
                ));
                return;
            }
        };
        last = piece.row_count() < PIECE_ROWS;
        if outlet.send(writer.write(&piece), timeout).await.is_err() {
            // The client is gone, or took too long. Dropping `rows` ends the
            // transaction.
            return;
        }
    }
    if let Some(end) = writer.finish()
        && outlet.send(end, timeout).await.is_err()
    {
        return;
    }
    outlet.finish();
}

/// The body's sender, which breaks the body off when it is dropped before the
/// end — on every early return, and on a panic, since unwinding drops it too.
/// Only [`Outlet::finish`] ends the body as a whole file.
struct Outlet {
    /// `Some` until the body is ended, one way or the other.
    sender: Option<Sender<Bytes, Broken>>,
}

impl Outlet {
    fn new(sender: Sender<Bytes, Broken>) -> Self {
        Self {
            sender: Some(sender),
        }
    }

    /// Hands a piece to the client, which has `deadline` to take it — a
    /// client that stops reading keeps the channel full, and would otherwise
    /// keep the export, its connection and its snapshot as long as it liked.
    /// `Err` means the export is over: the client left, or was too slow and
    /// the body is broken off.
    async fn send(&mut self, piece: Bytes, deadline: Duration) -> Result<(), ()> {
        let sender = self.sender.as_mut().expect("sending before the end");
        match tokio::time::timeout(deadline, sender.send_data(piece)).await {
            Ok(Ok(())) => Ok(()),
            // Nobody is reading any more.
            Ok(Err(_)) => Err(()),
            Err(_) => {
                self.abort(format!(
                    "the client took no piece for {} ms",
                    deadline.as_millis()
                ));
                Err(())
            }
        }
    }

    /// Breaks the body off: the client gets an error after what it has, never
    /// the end of a file.
    fn abort(&mut self, reason: String) {
        if let Some(sender) = self.sender.take() {
            sender.abort(Broken(reason));
        }
    }

    /// The end of the file: the body ends as a whole.
    fn finish(mut self) {
        self.sender.take();
    }
}

impl Drop for Outlet {
    fn drop(&mut self) {
        self.abort("the export ended before its last row".to_owned());
    }
}

/// One step against the source, raced against `give_up`. When `give_up` comes
/// first, the step is abandoned — and a statement it left running in the
/// database is cancelled there, not only forgotten here.
async fn step<T>(
    work: impl Future<Output = Result<T, DataSourceError>>,
    give_up: impl Future<Output = ()>,
    canceller: Option<&ExportCanceller>,
) -> Option<Result<T, DataSourceError>> {
    tokio::select! {
        result = work => Some(result),
        () = give_up => {
            if let Some(canceller) = canceller {
                canceller.cancel().await;
            }
            None
        }
    }
}

/// Why a body that had started was broken off. The client sees the connection
/// end without the end of the body, never a file that looks complete.
#[derive(Debug)]
struct Broken(String);

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the export broke off: {}", self.0)
    }
}

impl std::error::Error for Broken {}

/// The writer of the chosen format, piece by piece.
enum Writer {
    Csv(CsvWriter),
    Json(JsonWriter),
}

impl Writer {
    fn new(format: Format) -> Self {
        match format {
            Format::Csv(options) => Writer::Csv(CsvWriter::new(options)),
            Format::Json => Writer::Json(JsonWriter::new()),
        }
    }

    /// The next piece; the first carries the header, or the `[`.
    fn write(&mut self, piece: &QueryResult) -> Bytes {
        Bytes::from(match self {
            Writer::Csv(writer) => writer.write(piece),
            Writer::Json(writer) => writer.write(piece),
        })
    }

    /// What closes the file: the `]` of a JSON array, nothing for a CSV.
    fn finish(self) -> Option<Bytes> {
        match self {
            Writer::Csv(_) => None,
            Writer::Json(writer) => Some(Bytes::from(writer.finish())),
        }
    }
}

/// The format and its options, from the request's query string and `Accept`.
fn format_of(uri: &Uri, accept: Option<&HeaderValue>) -> Result<Format, WireError> {
    let malformed = |message: String| WireError::new(ErrorCode::Malformed, message);

    let Parameters(pairs) = Parameters::<Vec<(String, String)>>::try_from_uri(uri)
        .map_err(|error| malformed(format!("the query string: {error}")))?;

    let mut seen: Vec<&str> = Vec::new();
    for (key, _) in &pairs {
        if key != "format" && !CSV_PARAMETERS.contains(&key.as_str()) {
            return Err(malformed(format!(
                "{key:?} is not a parameter of an export (format, {})",
                CSV_PARAMETERS.join(", ")
            )));
        }
        if seen.contains(&key.as_str()) {
            return Err(malformed(format!("{key:?} is given twice")));
        }
        seen.push(key);
    }
    let value = |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };

    let json = match value("format") {
        Some("csv") => false,
        Some("json") => true,
        Some(other) => {
            return Err(malformed(format!(
                "format is \"csv\" or \"json\", not {other:?}"
            )));
        }
        None => accept.is_some_and(prefers_json),
    };
    if json {
        // A CSV option on a JSON export would do nothing, and do it silently.
        if let Some(stray) = CSV_PARAMETERS.iter().find(|key| value(key).is_some()) {
            return Err(malformed(format!(
                "{stray:?} is a CSV option, and this export is JSON"
            )));
        }
        return Ok(Format::Json);
    }

    let [delimiter_key, bom_key, protect_key, null_key] = CSV_PARAMETERS;
    let flag = |key: &str| match value(key) {
        None => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(other) => Err(malformed(format!("{key} is true or false, not {other:?}"))),
    };
    let mut options = CsvOptions::default();
    if let Some(delimiter) = value(delimiter_key) {
        let mut chars = delimiter.chars();
        match (chars.next(), chars.next()) {
            (Some(one), None) => options.delimiter = one,
            _ => return Err(malformed(format!("{delimiter_key}: one character"))),
        }
    }
    if let Some(bom) = flag(bom_key)? {
        options.bom = bom;
    }
    if let Some(protect) = flag(protect_key)? {
        options.protect_formulas = protect;
    }
    if let Some(null) = value(null_key) {
        options.null = null.to_owned();
    }
    // The writers trust options that passed this; this is the boundary.
    options
        .check()
        .map_err(|message| malformed(message.to_owned()))?;
    Ok(Format::Csv(options))
}

/// Whether `Accept` prefers JSON to CSV. The higher `q` wins, the earlier one
/// on a tie; a header that names neither leaves the default, CSV.
fn prefers_json(accept: &HeaderValue) -> bool {
    let Ok(accept) = accept.to_str() else {
        return false;
    };
    let mut best: Option<(bool, f32)> = None;
    for entry in accept.split(',') {
        let mut parts = entry.split(';');
        let json = match parts.next().map(str::trim) {
            Some(media) if media.eq_ignore_ascii_case("application/json") => true,
            Some(media) if media.eq_ignore_ascii_case("text/csv") => false,
            _ => continue,
        };
        let q = parts
            .filter_map(|part| part.trim().strip_prefix("q="))
            .find_map(|q| q.trim().parse::<f32>().ok())
            .unwrap_or(1.0);
        if q > 0.0 && best.is_none_or(|(_, top)| q > top) {
            best = Some((json, q));
        }
    }
    best.is_some_and(|(json, _)| json)
}

/// `attachment` with a file name from the source: a plain ASCII `filename`
/// for old clients — every character outside `[A-Za-z0-9._-]` becomes `_`, so
/// no quote, separator or line break reaches the header — and the exact name
/// as `filename*`, percent-encoded UTF-8 after RFC 8187.
fn content_disposition(source: &str, extension: &str) -> HeaderValue {
    let plain: String = source
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut exact = String::new();
    for byte in format!("{source}.{extension}").bytes() {
        // RFC 8187 `attr-char`: what may stand unencoded.
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            exact.push(char::from(byte));
        } else {
            exact.push_str(&format!("%{byte:02X}"));
        }
    }
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{plain}.{extension}\"; filename*=UTF-8''{exact}"
    ))
    .expect("only visible ASCII is left")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv(options: CsvOptions) -> Result<Format, WireError> {
        Ok(Format::Csv(options))
    }

    /// `format_of` for a request with this query string, if any.
    fn format(query: Option<&str>, accept: Option<&HeaderValue>) -> Result<Format, WireError> {
        let uri: Uri = match query {
            Some(query) => format!("/export/orders?{query}"),
            None => "/export/orders".to_owned(),
        }
        .parse()
        .expect("a request URI");
        format_of(&uri, accept)
    }

    use http_body_util::BodyExt;

    /// The body a client reads: the pieces, then its end — or an error.
    async fn read(channel: Channel<Bytes, Broken>) -> (String, Option<String>) {
        let mut body = Body::new(channel);
        let mut text = String::new();
        loop {
            match body.frame().await {
                None => return (text, None),
                Some(Ok(frame)) => {
                    text.push_str(std::str::from_utf8(frame.data_ref().unwrap()).unwrap());
                }
                Some(Err(error)) => return (text, Some(error.to_string())),
            }
        }
    }

    /// Only `finish` ends the body as a whole file.
    #[tokio::test]
    async fn only_the_end_ends_the_body() {
        let (sender, channel) = Channel::new(4);
        let mut outlet = Outlet::new(sender);
        outlet
            .send(Bytes::from("a\r\n"), Duration::from_secs(1))
            .await
            .unwrap();
        outlet.finish();
        assert_eq!(read(channel).await, ("a\r\n".to_owned(), None));

        let (sender, channel) = Channel::new(4);
        let mut outlet = Outlet::new(sender);
        outlet
            .send(Bytes::from("a\r\n"), Duration::from_secs(1))
            .await
            .unwrap();
        drop(outlet);
        let (text, error) = read(channel).await;
        assert_eq!(text, "a\r\n");
        assert!(error.is_some_and(|error| error.contains("before its last row")));
    }

    /// **A panic after the first byte breaks the body off**, it does not end
    /// it: unwinding drops the outlet, and the client gets an error after the
    /// rows it has — never a shorter file that looks whole.
    #[tokio::test]
    async fn a_panic_after_the_first_byte_breaks_the_body_off() {
        let (sender, channel) = Channel::new(4);
        let task = tokio::spawn(async move {
            let mut outlet = Outlet::new(sender);
            outlet
                .send(Bytes::from("id\r\n1\r\n"), Duration::from_secs(1))
                .await
                .unwrap();
            panic!("a bug after the first piece");
        });
        assert!(task.await.unwrap_err().is_panic());
        let (text, error) = read(channel).await;
        assert_eq!(text, "id\r\n1\r\n");
        assert!(error.is_some(), "the body ended as if it were whole");
    }

    /// A client that takes no piece within the deadline is cut off: the send
    /// fails, and what the client reads afterwards ends in an error.
    #[tokio::test]
    async fn a_client_that_stops_reading_is_cut_off() {
        let (sender, channel) = Channel::new(1);
        let mut outlet = Outlet::new(sender);
        let deadline = Duration::from_millis(100);
        outlet.send(Bytes::from("one"), deadline).await.unwrap();
        // The channel holds one piece and nobody reads it.
        let started = std::time::Instant::now();
        assert!(outlet.send(Bytes::from("two"), deadline).await.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        let (text, error) = read(channel).await;
        assert_eq!(text, "one");
        assert!(error.is_some_and(|error| error.contains("took no piece for 100 ms")));
    }

    #[test]
    fn the_format_comes_from_the_parameter_then_accept_then_the_default() {
        let accept = |value: &'static str| HeaderValue::from_static(value);
        assert_eq!(format(None, None), csv(CsvOptions::default()));
        assert_eq!(format(Some("format=json"), None), Ok(Format::Json));
        assert_eq!(
            format(Some("format=csv"), Some(&accept("application/json"))),
            csv(CsvOptions::default()),
            "the parameter wins"
        );
        assert_eq!(
            format(None, Some(&accept("application/json"))),
            Ok(Format::Json)
        );
        assert_eq!(
            format(None, Some(&accept("text/csv, application/json"))),
            csv(CsvOptions::default()),
            "the earlier one on a tie"
        );
        assert_eq!(
            format(None, Some(&accept("text/csv;q=0.5, application/json"))),
            Ok(Format::Json),
            "the higher q"
        );
        assert_eq!(
            format(None, Some(&accept("*/*"))),
            csv(CsvOptions::default())
        );
        assert_eq!(
            format(None, Some(&accept("application/json;q=0"))),
            csv(CsvOptions::default()),
            "q=0 means not this one"
        );
    }

    #[test]
    fn the_csv_options_are_read_and_checked() {
        assert_eq!(
            format(
                Some("delimiter=%3B&bom=false&protectFormulas=false&null=%5CN"),
                None
            ),
            csv(CsvOptions {
                delimiter: ';',
                bom: false,
                protect_formulas: false,
                null: "\\N".to_owned(),
            })
        );
        let refused = |query: &str| {
            format(Some(query), None)
                .map(|_| ())
                .expect_err(query)
                .message
        };
        assert!(refused("filename=x").contains("not a parameter"));
        assert!(refused("format=xlsx").contains("xlsx"));
        assert!(refused("format=json&delimiter=%3B").contains("CSV option"));
        assert!(refused("delimiter=ab").contains("one character"));
        assert!(refused("bom=yes").contains("true or false"));
        assert!(refused("bom=true&bom=false").contains("twice"));
        // What `CsvOptions::check` refuses, the boundary refuses.
        assert!(refused("delimiter=%22").contains("delimiter"));
        assert!(refused("null=a%2Cb").contains("null"));
    }

    #[test]
    fn the_file_name_is_the_source_and_nothing_can_break_the_header() {
        assert_eq!(
            content_disposition("orders", "csv"),
            "attachment; filename=\"orders.csv\"; filename*=UTF-8''orders.csv"
        );
        assert_eq!(
            content_disposition("Aufträge 2026", "json"),
            "attachment; filename=\"Auftr_ge_2026.json\"; \
             filename*=UTF-8''Auftr%C3%A4ge%202026.json"
        );
        // A quote, a separator, a line break: none of them survives as itself.
        let hostile = content_disposition("a\"; filename=x.exe\r\nSet-Cookie: y/../z", "csv");
        let text = hostile.to_str().unwrap();
        assert_eq!(text.matches('"').count(), 2, "{text}");
        assert!(!text.contains(['\r', '\n', '/']), "{text}");
        assert!(!text.contains("; filename=x"), "{text}");
    }
}
