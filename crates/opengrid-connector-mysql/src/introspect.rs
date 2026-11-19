//! Reading a schema from the catalogue (issues #486, #501).
//!
//! `information_schema` answers, per table, the columns and their types; each
//! type is mapped to an opengrid type where MySQL's value reads back as exactly
//! that, and anything else is named and left out, never guessed:
//!
//! - `TINYINT(1)` is MySQL's boolean; the other integers are 64-bit, except a
//!   `BIGINT UNSIGNED`, which does not fit.
//! - `DECIMAL(p,s)` keeps its precision and scale, up to opengrid's 38 digits.
//! - A string is mapped only in `utf8mb4`: the compiler compares under
//!   `utf8mb4_0900_bin`, which another character set does not take.
//! - `TIMESTAMP` is the instant (read in UTC); `DATETIME` is a wall clock,
//!   which names no instant, and is left out as PostgreSQL's `timestamp`
//!   without a zone is.
//! - `JSON` holds anything: a list column is named by hand in the file.
//!
//! Reading the catalogue releases nothing: only what a configuration names is
//! served.

use mysql_async::Opts;
use mysql_async::prelude::Queryable;
use opengrid_connector::introspect::TableProposal;
use opengrid_datasource::DataSourceError;
use opengrid_types::DataType;

use crate::backend;

/// One column of the catalogue, every field as MySQL's text.
type Column = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
);

/// The base tables of the database `url` names (`mysql://…/database`), with
/// their columns in table order.
pub async fn read_tables(url: &str) -> Result<Vec<TableProposal>, DataSourceError> {
    let opts = Opts::from_url(url).map_err(backend)?;
    if opts.db_name().is_none() {
        return Err(backend(
            "the connection string names no database: mysql://…/<database>",
        ));
    }
    let mut connection = mysql_async::Conn::new(opts).await.map_err(backend)?;
    let columns: Vec<Column> = connection
        .query(
            "SELECT CAST(c.TABLE_NAME AS CHAR), CAST(c.COLUMN_NAME AS CHAR), \
                    CAST(c.DATA_TYPE AS CHAR), CAST(c.COLUMN_TYPE AS CHAR), \
                    CAST(c.NUMERIC_PRECISION AS CHAR), CAST(c.NUMERIC_SCALE AS CHAR), \
                    CAST(c.IS_NULLABLE AS CHAR), CAST(c.CHARACTER_SET_NAME AS CHAR) \
             FROM information_schema.COLUMNS c \
             JOIN information_schema.TABLES t \
               ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME \
             WHERE c.TABLE_SCHEMA = DATABASE() AND t.TABLE_TYPE = 'BASE TABLE' \
             ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION",
        )
        .await
        .map_err(backend)?;
    connection.disconnect().await.map_err(backend)?;

    let mut tables: Vec<TableProposal> = Vec::new();
    for (table, column, data_type, column_type, precision, scale, nullable, charset) in columns {
        if tables.last().is_none_or(|last| last.name != table) {
            tables.push(TableProposal::new(None, &table));
        }
        let named = match &charset {
            Some(charset) if charset != "utf8mb4" => {
                format!("{column_type} character set {charset}")
            }
            _ => column_type.clone(),
        };
        let mapped = mapped(
            &data_type,
            &column_type,
            precision.and_then(|p| p.parse().ok()),
            scale.and_then(|s| s.parse().ok()),
            charset.as_deref(),
        );
        tables
            .last_mut()
            .expect("pushed")
            .column(&column, &named, mapped, nullable == "YES");
    }
    Ok(tables)
}

/// The opengrid type a MySQL column reads back as.
fn mapped(
    data_type: &str,
    column_type: &str,
    precision: Option<u8>,
    scale: Option<u8>,
    charset: Option<&str>,
) -> Option<DataType> {
    let unsigned = column_type.contains("unsigned");
    match data_type {
        "tinyint" if column_type.starts_with("tinyint(1)") => Some(DataType::Bool),
        "bigint" if unsigned => None,
        "tinyint" | "smallint" | "mediumint" | "int" | "bigint" => Some(DataType::Int64),
        "float" | "double" => Some(DataType::Float64),
        "decimal" => DataType::decimal(precision?, scale?).ok(),
        "char" | "varchar" | "tinytext" | "text" | "mediumtext" | "longtext" => {
            (charset == Some("utf8mb4")).then_some(DataType::Utf8)
        }
        "date" => Some(DataType::Date),
        "timestamp" => Some(DataType::Timestamp),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_map_where_the_value_reads_back_exactly() {
        let utf8 = Some("utf8mb4");
        assert_eq!(
            mapped("tinyint", "tinyint(1)", None, None, None),
            Some(DataType::Bool)
        );
        assert_eq!(
            mapped("tinyint", "tinyint", None, None, None),
            Some(DataType::Int64)
        );
        assert_eq!(
            mapped("int", "int unsigned", None, None, None),
            Some(DataType::Int64)
        );
        assert_eq!(mapped("bigint", "bigint unsigned", None, None, None), None);
        assert_eq!(
            mapped("decimal", "decimal(12,2)", Some(12), Some(2), None),
            DataType::decimal(12, 2).ok()
        );
        assert_eq!(
            mapped("decimal", "decimal(65,2)", Some(65), Some(2), None),
            None
        );
        assert_eq!(
            mapped("varchar", "varchar(20)", None, None, utf8),
            Some(DataType::Utf8)
        );
        assert_eq!(
            mapped("varchar", "varchar(20)", None, None, Some("latin1")),
            None
        );
        assert_eq!(
            mapped("timestamp", "timestamp(6)", None, None, None),
            Some(DataType::Timestamp)
        );
        assert_eq!(mapped("datetime", "datetime(6)", None, None, None), None);
        assert_eq!(mapped("json", "json", None, None, None), None);
    }
}
