//! `GET /watch/{source}` (issue #328): a stream that says a source changed.

use std::sync::Arc;
use std::time::Duration;

use opengrid_conformance::{fixture_csv, suite_dir};
use opengrid_connector::LocalConnector;
use opengrid_server::{Server, SourcePolicy};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "watch-token";

async fn serve() -> (Arc<Server>, String) {
    let connector =
        LocalConnector::from_csv(&fixture_csv(), &suite_dir().join("data/orders.schema.json"))
            .expect("the fixture");
    let server = Server::builder()
        .token(TOKEN, [("country", "DE")])
        .source("orders", connector, SourcePolicy::default())
        .build()
        .await
        .expect("a server");
    let server = Arc::new(server);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = server.router();
    tokio::spawn(async move { axum::serve(listener, router).await });
    (server, address.to_string())
}

async fn open(address: &str, source: &str, token: Option<&str>) -> TcpStream {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let auth = token.map_or(String::new(), |token| {
        format!("Authorization: Bearer {token}\r\n")
    });
    let request = format!(
        "GET /watch/{source} HTTP/1.1\r\nHost: {address}\r\n{auth}Accept: text/event-stream\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    stream
}

/// Reads until `needle` shows or a second passes.
async fn read_until(stream: &mut TcpStream, needle: &str) -> String {
    let mut seen = String::new();
    let mut buffer = [0u8; 1024];
    let wait = tokio::time::timeout(Duration::from_secs(2), async {
        while !seen.contains(needle) {
            let read = stream.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            seen.push_str(&String::from_utf8_lossy(&buffer[..read]));
        }
    });
    let _ = wait.await;
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_reaches_the_watcher_of_that_source_as_one_event() {
    let (server, address) = serve().await;
    let mut stream = open(&address, "orders", Some(TOKEN)).await;
    let head = read_until(&mut stream, ": ready").await;
    assert!(head.contains("200 OK"), "{head}");
    assert!(head.contains("content-type: text/event-stream"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
    assert!(head.contains("retry: 5000"), "{head}");

    // Another source's change is not this watcher's.
    server.notify_changed("elsewhere");
    server.notify_changed("orders");
    let got = read_until(&mut stream, "event: changed").await;
    assert!(
        got.contains("event: changed\ndata: {\"source\":\"orders\"}\n\n"),
        "{got}"
    );
    assert!(!got.contains("elsewhere"), "{got}");
}

#[tokio::test(flavor = "multi_thread")]
async fn it_needs_the_token_and_a_source_the_server_knows() {
    let (_server, address) = serve().await;
    let mut without = open(&address, "orders", None).await;
    let refused = read_until(&mut without, "unauthorized").await;
    assert!(refused.contains("401"), "{refused}");

    let mut wrong = open(&address, "orders", Some("nope")).await;
    assert!(read_until(&mut wrong, "401").await.contains("401"));

    let mut unknown = open(&address, "nothing", Some(TOKEN)).await;
    let got = read_until(&mut unknown, "unknown_source").await;
    assert!(got.contains("404"), "{got}");
}
