//! Reading a schema from PostgreSQL (issue #486): every supported type is read
//! as the type it serves as, and a column with a type that has no mapping is
//! named and left out. Skips itself without a database (`OPENGRID_TEST_PG`), like the rest.

use opengrid_connector::introspect::{NO_MAPPING, NOT_A_FIELD_NAME};
use opengrid_datasource_postgres::read_tables;
use opengrid_types::DataType;

fn url() -> String {
    std::env::var("OPENGRID_TEST_PG")
        .unwrap_or_else(|_| "host=localhost dbname=postgres".to_owned())
}

const SCHEMA: &str = "opengrid_read_tables_test";

#[tokio::test]
async fn a_table_with_every_supported_type_is_read_as_those_types() {
    let (client, connection) = match tokio_postgres::connect(&url(), tokio_postgres::NoTls).await {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("skipping the PostgreSQL schema test: {error}");
            return;
        }
    };
    tokio::spawn(connection);
    client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {SCHEMA} CASCADE; CREATE SCHEMA {SCHEMA};
             CREATE TABLE {SCHEMA}.every_type (
                id bigint PRIMARY KEY,
                small smallint NOT NULL,
                plain integer,
                price numeric(12,2),
                ratio double precision,
                single real,
                label text,
                code varchar(12) NOT NULL,
                fixed char(3),
                done boolean,
                born date,
                seen timestamptz,
                tags text[],
                counts bigint[],
                free numeric,
                wall timestamp,
                payload jsonb,
                ident uuid,
                \"Two Words\" text
             );
             CREATE TABLE {SCHEMA}.second (a int);
             CREATE VIEW {SCHEMA}.not_a_table AS SELECT 1 AS a;"
        ))
        .await
        .expect("fixture");

    let tables = read_tables(&url(), SCHEMA).await.expect("read");
    client
        .batch_execute(&format!("DROP SCHEMA {SCHEMA} CASCADE"))
        .await
        .expect("cleanup");

    assert_eq!(
        tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["every_type", "second"],
        "base tables only"
    );
    let table = &tables[0];
    assert_eq!(table.namespace.as_deref(), Some(SCHEMA));
    let fields: Vec<(&str, DataType, bool)> = table
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.data_type, f.nullable))
        .collect();
    assert_eq!(
        fields,
        [
            ("id", DataType::Int64, false),
            ("small", DataType::Int64, false),
            ("plain", DataType::Int64, true),
            ("price", DataType::decimal(12, 2).unwrap(), true),
            ("ratio", DataType::Float64, true),
            ("single", DataType::Float64, true),
            ("label", DataType::Utf8, true),
            ("code", DataType::Utf8, false),
            ("fixed", DataType::Utf8, true),
            ("done", DataType::Bool, true),
            ("born", DataType::Date, true),
            ("seen", DataType::Timestamp, true),
            ("tags", DataType::ListUtf8, true),
            ("counts", DataType::ListInt64, true),
        ]
    );
    let left_out: Vec<(&str, &str, &str)> = table
        .left_out
        .iter()
        .map(|l| (l.column.as_str(), l.database_type.as_str(), l.reason))
        .collect();
    assert_eq!(
        left_out,
        [
            ("free", "numeric", NO_MAPPING),
            ("wall", "timestamp without time zone", NO_MAPPING),
            ("payload", "jsonb", NO_MAPPING),
            ("ident", "uuid", NO_MAPPING),
            ("Two Words", "text", NOT_A_FIELD_NAME),
        ]
    );
}
