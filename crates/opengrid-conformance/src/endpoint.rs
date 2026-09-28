//! The suite over HTTP, against any server (issue #48).
//!
//! A server written in .NET, Java or Node cannot run [`check_source`](crate::check_source):
//! it has no Rust source to hand in. It has an endpoint. [`check_endpoint`]
//! sends every case as `POST {endpoint}/query/{source}`, once asking for JSON
//! and once for the binary form (E35), and compares each answer with the
//! expectation. All cases agree in both forms means the server speaks the
//! protocol (`docs/protocol.md`) the way the browser expects.
//!
//! The server must hold the fixture ([`fixture_csv`](crate::fixture_csv) under
//! [`fixture_schema`](crate::fixture_schema)) as a source named `orders`, every
//! column allowed, no row filter, and accept `token`.
//!
//! # A small HTTP client, on purpose
//!
//! Plain HTTP/1.1 over `std::net`, one connection per request — enough for a
//! test run against a server on the same machine or network, and no new
//! dependency. For TLS, put the runner next to the server or behind a proxy.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use opengrid_datasource::QueryResult;
use opengrid_datasource::wire::result_from_json;

use crate::{Report, RowOrder, Table, check_dir, compare, fixture_schema, suite_dir};

/// The binary form's media type (E35).
const MEDIA_TYPE: &str = "application/vnd.opengrid.columns";

/// Runs every case against `endpoint` (e.g. `http://127.0.0.1:8081`), in both
/// result forms. Failures name the case and the form.
pub fn check_endpoint(endpoint: &str, token: &str) -> Result<Report, String> {
    let target = Target::parse(endpoint)?;
    let schema = fixture_schema();
    let mut cases = check_dir(&suite_dir().join("cases"), &schema)
        .map_err(|error| format!("the suite: {error}"))?;
    cases.sort_by(|a, b| a.case.id.cmp(&b.case.id));

    let mut failures = Vec::new();
    for case in &cases {
        let body = opengrid_json::to_string(&case.case.query).into_bytes();
        let path = format!("/query/{}", case.case.query.source.as_str());
        let order = if case.case.ordered {
            RowOrder::Ordered
        } else {
            RowOrder::Unordered
        };
        for form in [Form::Json, Form::Binary] {
            let answer = target
                .post(&path, token, form.accept(), &body)
                .and_then(|answer| form.read(answer));
            match answer {
                Ok(result) => {
                    if let Err(difference) = compare(&case.expected, &Table::from(&result), order) {
                        failures.push(format!("{} ({}): {difference}", case.case.id, form.name()));
                    }
                }
                Err(error) => failures.push(format!("{} ({}): {error}", case.case.id, form.name())),
            }
        }
    }
    Ok(Report {
        cases: cases.len(),
        failures,
    })
}

#[derive(Clone, Copy)]
enum Form {
    Json,
    Binary,
}

impl Form {
    fn name(self) -> &'static str {
        match self {
            Form::Json => "json",
            Form::Binary => "binary",
        }
    }

    fn accept(self) -> &'static str {
        match self {
            Form::Json => "application/json",
            Form::Binary => MEDIA_TYPE,
        }
    }

    /// The answer as a result — or the reason it is not one.
    fn read(self, answer: Answer) -> Result<QueryResult, String> {
        if answer.status != 200 {
            return Err(format!(
                "status {}: {}",
                answer.status,
                String::from_utf8_lossy(&answer.body)
            ));
        }
        let media = answer
            .content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if media != self.accept() {
            return Err(format!("asked for {}, got {media:?}", self.accept()));
        }
        match self {
            Form::Json => {
                let text = std::str::from_utf8(&answer.body).map_err(|error| error.to_string())?;
                result_from_json(text).map_err(|error| error.to_string())
            }
            Form::Binary => {
                let (table, total) = opengrid_columns::wire::decode_result(&answer.body)
                    .map_err(|error| error.to_string())?;
                Ok(QueryResult::new(
                    table.schema().clone(),
                    table.to_values(),
                    total,
                ))
            }
        }
    }
}

/// `http://host:port[/base]`.
struct Target {
    host: String,
    port: u16,
    base: String,
}

struct Answer {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

impl Target {
    fn parse(endpoint: &str) -> Result<Self, String> {
        let rest = endpoint
            .strip_prefix("http://")
            .ok_or_else(|| format!("{endpoint:?}: only http:// endpoints (see the module docs)"))?;
        let (authority, base) = rest.split_once('/').map_or((rest, ""), |(a, b)| (a, b));
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (
                host,
                port.parse()
                    .map_err(|_| format!("{endpoint:?}: not a port: {port:?}"))?,
            ),
            None => (authority, 80),
        };
        Ok(Self {
            host: host.to_owned(),
            port,
            base: format!("/{}", base.trim_end_matches('/'))
                .trim_end_matches('/')
                .to_owned(),
        })
    }

    fn post(&self, path: &str, token: &str, accept: &str, body: &[u8]) -> Result<Answer, String> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))
            .map_err(|error| format!("{}:{}: {error}", self.host, self.port))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|error| error.to_string())?;
        let head = format!(
            "POST {}{path} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {token}\r\n\
             Content-Type: application/json\r\nAccept: {accept}\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            self.base,
            self.host,
            self.port,
            body.len()
        );
        stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(body))
            .map_err(|error| error.to_string())?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|error| error.to_string())?;
        parse_response(&raw)
    }
}

/// Status, `Content-Type` and body of an HTTP/1.1 response read to its end;
/// a chunked body is put back together.
fn parse_response(raw: &[u8]) -> Result<Answer, String> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("the response has no end of headers")?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|error| error.to_string())?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or("the response has no status line")?;
    let mut content_type = String::new();
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-type") {
            content_type = value.trim().to_owned();
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value.trim().eq_ignore_ascii_case("chunked")
        {
            chunked = true;
        }
    }
    let rest = &raw[split + 4..];
    let body = if chunked {
        dechunk(rest)?
    } else {
        rest.to_vec()
    };
    Ok(Answer {
        status,
        content_type,
        body,
    })
}

fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let end = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("a chunk has no size line")?;
        let size_text = std::str::from_utf8(&rest[..end]).map_err(|error| error.to_string())?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| format!("not a chunk size: {size_text:?}"))?;
        rest = &rest[end + 2..];
        if size == 0 {
            return Ok(body);
        }
        if rest.len() < size + 2 {
            return Err("a chunk is cut short".to_owned());
        }
        body.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_is_host_port_and_base() {
        let target = Target::parse("http://127.0.0.1:8081/api/").unwrap();
        assert_eq!(
            (target.host.as_str(), target.port, target.base.as_str()),
            ("127.0.0.1", 8081, "/api")
        );
        let bare = Target::parse("http://localhost").unwrap();
        assert_eq!((bare.port, bare.base.as_str()), (80, ""));
        assert!(Target::parse("https://example.org").is_err());
    }

    #[test]
    fn a_chunked_body_is_put_back_together() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n";
        let answer = parse_response(raw).unwrap();
        assert_eq!(answer.status, 200);
        assert_eq!(answer.body, b"{\"a\":1}");
    }
}
