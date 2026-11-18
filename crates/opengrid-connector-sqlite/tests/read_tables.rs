//! Reading a schema from a SQLite file (issue #486): every supported type is
//! read as the type the connector serves it as, and a column with a type that
//! has no mapping is named and left out.

use opengrid_connector::introspect::{NO_MAPPING, NOT_A_FIELD_NAME};
use opengrid_connector_sqlite::{Connection, read_tables};
use opengrid_types::DataType;

fn database() -> Connection {
    let connection = Connection::open_in_memory().expect("memory");
    connection
        .execute_batch(
            "CREATE TABLE every_type (
                id INTEGER PRIMARY KEY,
                big BIGINT NOT NULL,
                ratio REAL,
                score DOUBLE PRECISION,
                label TEXT,
                code VARCHAR(12) NOT NULL,
                done BOOLEAN,
                price DECIMAL(12,2),
                born DATE,
                seen DATETIME,
                payload BLOB,
                loose,
                \"two words\" TEXT
            );
            CREATE TABLE second (a INTEGER);",
        )
        .expect("tables");
    connection
}

#[test]
fn a_table_with_every_supported_type_is_read_as_those_types() {
    let tables = read_tables(&database()).expect("read");
    assert_eq!(
        tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["every_type", "second"]
    );
    let table = &tables[0];
    let fields: Vec<(&str, DataType, bool)> = table
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.data_type, f.nullable))
        .collect();
    assert_eq!(
        fields,
        [
            ("id", DataType::Int64, false),
            ("big", DataType::Int64, false),
            ("ratio", DataType::Float64, true),
            ("score", DataType::Float64, true),
            ("label", DataType::Utf8, true),
            ("code", DataType::Utf8, false),
            ("done", DataType::Bool, true),
        ]
    );
}

#[test]
fn a_column_with_an_unsupported_type_is_named_and_left_out() {
    let tables = read_tables(&database()).expect("read");
    let left_out: Vec<(&str, &str, &str)> = tables[0]
        .left_out
        .iter()
        .map(|l| (l.column.as_str(), l.database_type.as_str(), l.reason))
        .collect();
    assert_eq!(
        left_out,
        [
            ("price", "DECIMAL(12,2)", NO_MAPPING),
            ("born", "DATE", NO_MAPPING),
            ("seen", "DATETIME", NO_MAPPING),
            ("payload", "BLOB", NO_MAPPING),
            ("loose", "", NO_MAPPING),
            ("two words", "TEXT", NOT_A_FIELD_NAME),
        ]
    );
}
