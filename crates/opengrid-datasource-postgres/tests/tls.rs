//! TLS to the database (issue #220), the parts that need no database: what a
//! connection string may ask for, and the connector the `tls` feature builds.
//!
//! The runs against a real TLS server are in `tls_postgres.rs`.

use opengrid_datasource_postgres::{PoolOptions, PostgresDataSource};
use opengrid_types::{DataType, Field, FieldName, Schema};

fn schema() -> Schema {
    Schema::new(vec![Field::required(
        FieldName::new("id").unwrap(),
        DataType::Int64,
    )])
}

fn message(result: Result<PostgresDataSource, opengrid_datasource::DataSourceError>) -> String {
    match result {
        Ok(_) => panic!("expected a refusal"),
        Err(error) => error.to_string(),
    }
}

/// `sslmode=require` in the string and no connector: refused at once, so the
/// operator learns it at the start and not from the first failing query.
#[test]
fn a_url_that_asks_for_tls_is_refused_without_a_connector() {
    let said = message(PostgresDataSource::connect(
        "host=localhost dbname=postgres sslmode=require",
        "t",
        schema(),
    ));
    assert!(said.contains("connect_with_tls"), "{said}");

    // The default (`prefer`) and an explicit `disable` stay as they were.
    for url in [
        "host=localhost dbname=postgres",
        "host=localhost dbname=postgres sslmode=disable",
    ] {
        PostgresDataSource::connect_with(url, "t", schema(), PoolOptions::default())
            .expect("a source");
    }
}

#[cfg(feature = "tls")]
mod with_the_feature {
    use super::*;
    use opengrid_datasource_postgres::rustls_connector;

    const PRIVATE_CA: &[u8] = include_bytes!("tls/ca.pem");

    /// `connect_with_tls` makes TLS a requirement: a string that says
    /// `disable` contradicts it.
    #[test]
    fn a_url_that_turns_tls_off_is_refused_with_a_connector() {
        let tls = rustls_connector(None).expect("a connector");
        let said = message(PostgresDataSource::connect_with_tls(
            "host=localhost dbname=postgres sslmode=disable",
            "t",
            schema(),
            PoolOptions::default(),
            tls.clone(),
        ));
        assert!(said.contains("contradicts"), "{said}");

        for url in [
            "host=localhost dbname=postgres",
            "host=localhost dbname=postgres sslmode=require",
        ] {
            PostgresDataSource::connect_with_tls(
                url,
                "t",
                schema(),
                PoolOptions::default(),
                tls.clone(),
            )
            .expect("a source");
        }
    }

    #[test]
    fn the_connector_takes_the_roots_of_a_private_authority() {
        rustls_connector(None).expect("the public roots");
        rustls_connector(Some(PRIVATE_CA)).expect("a private root as well");
    }

    #[test]
    fn roots_that_are_not_certificates_are_refused() {
        let said = rustls_connector(Some(b"this is not a PEM file"))
            .err()
            .expect("a refusal")
            .to_string();
        assert!(said.contains("no certificate"), "{said}");

        let broken = b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";
        assert!(rustls_connector(Some(broken)).is_err());
    }
}
