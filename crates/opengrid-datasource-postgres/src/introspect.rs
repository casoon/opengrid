//! Reading a schema from the catalogue (issue #486).
//!
//! `information_schema` answers, per table, the columns and their types; each
//! type is mapped to an opengrid type where PostgreSQL's value reads back as
//! exactly that — `numeric(p,s)` stays a decimal with its precision and scale,
//! `bigint` stays 64-bit, `timestamptz` is the microsecond instant, `date` a
//! date — and anything else is named and left out, never guessed. Reading the
//! catalogue releases nothing: only what a configuration names is served.

use opengrid_connector::introspect::TableProposal;
use opengrid_datasource::DataSourceError;
use opengrid_types::DataType;
use tokio_postgres::NoTls;

/// The base tables of the PostgreSQL schema `namespace` in the database `url`
/// names (a libpq connection string), with their columns in table order.
///
/// A connection that asks for TLS is refused, as for a data source without a
/// TLS connector: it must not end up in clear text.
pub async fn read_tables(
    url: &str,
    namespace: &str,
) -> Result<Vec<TableProposal>, DataSourceError> {
    if crate::source::sslmode(url) == Some(tokio_postgres::config::SslMode::Require) {
        return Err(backend(
            "the connection string asks for TLS (sslmode=require), but this connection has none"
                .to_owned(),
        ));
    }
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .map_err(|error| backend(error.to_string()))?;
    let driver = tokio::spawn(connection);
    let rows = client
        .query(
            "SELECT c.table_name::text, c.column_name::text, c.data_type::text, \
                    c.udt_name::text, c.numeric_precision::int4, c.numeric_scale::int4, \
                    c.is_nullable::text \
             FROM information_schema.columns c \
             JOIN information_schema.tables t \
               ON t.table_schema = c.table_schema AND t.table_name = c.table_name \
             WHERE c.table_schema = $1 AND t.table_type = 'BASE TABLE' \
             ORDER BY c.table_name, c.ordinal_position",
            &[&namespace],
        )
        .await
        .map_err(|error| backend(error.to_string()))?;
    drop(client);
    driver.abort();

    let mut tables: Vec<TableProposal> = Vec::new();
    for row in rows {
        let table: String = row.get(0);
        let column: String = row.get(1);
        let data_type: String = row.get(2);
        let udt: String = row.get(3);
        let precision: Option<i32> = row.get(4);
        let scale: Option<i32> = row.get(5);
        let nullable: String = row.get(6);
        if tables.last().is_none_or(|last| last.name != table) {
            tables.push(TableProposal::new(Some(namespace.to_owned()), &table));
        }
        let table = tables.last_mut().expect("pushed");
        let named = database_type(&data_type, &udt, precision, scale);
        table.column(
            &column,
            &named,
            mapped(&data_type, &udt, precision, scale),
            nullable == "YES",
        );
    }
    Ok(tables)
}

/// The type as PostgreSQL would write it in a `CREATE TABLE`.
fn database_type(data_type: &str, udt: &str, precision: Option<i32>, scale: Option<i32>) -> String {
    match (data_type, precision, scale) {
        ("numeric", Some(precision), Some(scale)) => format!("numeric({precision},{scale})"),
        ("ARRAY", ..) => format!("{}[]", udt.trim_start_matches('_')),
        (other, ..) => other.to_owned(),
    }
}

/// The opengrid type PostgreSQL's `data_type` reads back as.
fn mapped(
    data_type: &str,
    udt: &str,
    precision: Option<i32>,
    scale: Option<i32>,
) -> Option<DataType> {
    match data_type {
        "smallint" | "integer" | "bigint" => Some(DataType::Int64),
        "real" | "double precision" => Some(DataType::Float64),
        // Without a precision, `numeric` holds any number: no type to promise.
        "numeric" => {
            DataType::decimal(u8::try_from(precision?).ok()?, u8::try_from(scale?).ok()?).ok()
        }
        "text" | "character varying" | "character" => Some(DataType::Utf8),
        "boolean" => Some(DataType::Bool),
        "date" => Some(DataType::Date),
        // An instant (S9). `timestamp` without a zone is a wall clock, which
        // names no instant.
        "timestamp with time zone" => Some(DataType::Timestamp),
        "ARRAY" => match udt {
            "_text" | "_varchar" | "_bpchar" => Some(DataType::ListUtf8),
            "_int2" | "_int4" | "_int8" => Some(DataType::ListInt64),
            _ => None,
        },
        _ => None,
    }
}

fn backend(message: String) -> DataSourceError {
    DataSourceError::Backend { message }
}
