//! The black-box runner (issue #48) against this server, over a real socket:
//! what a .NET or Node server would be held to, held to our own first.

use opengrid_conformance::{check_endpoint, fixture_csv, suite_dir};
use opengrid_connector::LocalConnector;
use opengrid_server::{RowFilter, Server, SourcePolicy};

const TOKEN: &str = "protocol-token";

async fn serve() -> String {
    serve_with(SourcePolicy::default()).await
}

async fn serve_with(policy: SourcePolicy) -> String {
    let connector =
        LocalConnector::from_csv(&fixture_csv(), &suite_dir().join("data/orders.schema.json"))
            .expect("the fixture");
    let server = Server::builder()
        .token(TOKEN, [("country", "DE")])
        .source("orders", connector, policy)
        .build()
        .await
        .expect("a server");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, server.router()).await });
    format!("http://{address}")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_library_server_passes_the_runner_in_both_forms() {
    let endpoint = serve().await;
    let report = tokio::task::spawn_blocking(move || check_endpoint(&endpoint, TOKEN))
        .await
        .unwrap()
        .expect("the runner ran");
    report.assert_ok();
    assert!(report.cases >= 40, "{report}");
}

/// A wrong token is not a pass: every case fails, each saying why.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_refuses_is_named_case_by_case() {
    let endpoint = serve().await;
    let report = tokio::task::spawn_blocking(move || check_endpoint(&endpoint, "wrong"))
        .await
        .unwrap()
        .expect("the runner ran");
    assert_eq!(report.failures.len(), report.cases * 2, "{report}");
    assert!(report.failures[0].contains("status 401"), "{report}");
}

/// A server that answers with other rows — here a tenant filter the suite
/// does not expect — fails the cases whose answers differ, in both forms.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_with_other_answers_fails_those_cases() {
    let endpoint = serve_with(SourcePolicy {
        allowed_fields: Vec::new(),
        row_filter: Some(RowFilter::new("country", "eq", ":country")),
    })
    .await;
    let report = tokio::task::spawn_blocking(move || check_endpoint(&endpoint, TOKEN))
        .await
        .unwrap()
        .expect("the runner ran");
    assert!(!report.is_ok(), "{report}");
    assert!(
        report.failures.iter().any(|line| line.contains("(json)")),
        "{report}"
    );
    assert!(
        report.failures.iter().any(|line| line.contains("(binary)")),
        "{report}"
    );
}
