//! Reading the schema and writing the proposal.

use opengrid_connector::introspect::TableProposal;
use opengrid_json::{Json, ToJson};
use opengrid_types::{Schema, is_valid_identifier};

use crate::config::{ConfigError, Database};
use crate::pretty::pretty;

/// The variable the proposal names for the connection string.
pub const DEFAULT_CONNECTION_ENV: &str = "OPENGRID_DATABASE_URL";
/// The variable the proposal names for the token.
pub const DEFAULT_TOKEN_ENV: &str = "OPENGRID_TOKEN";

/// The proposal for `tables` of `database`, as the text of the file.
///
/// Every table with a usable name becomes a source and every column with a
/// mapped type a field; a column without one is named under `left_out`. The
/// file starts with `"proposal": true`, so it serves nothing until the owner
/// has read it. Tables whose name is no identifier are answered in the second
/// place: they cannot be sources.
pub fn proposal(
    database: Database,
    connection_env: &str,
    tables: &[TableProposal],
) -> (String, Vec<String>) {
    let mut skipped = Vec::new();
    let mut sources = Vec::new();
    for table in tables {
        if !is_valid_identifier(&table.name) {
            skipped.push(table.name.clone());
            continue;
        }
        let mut source = Json::object([
            ("name", Json::from(table.name.as_str())),
            ("database", Json::from(database.as_str())),
            ("connection", Json::from(connection_env)),
        ]);
        let object = source.as_object_mut().expect("an object");
        if let Some(namespace) = &table.namespace {
            object.insert("namespace", Json::from(namespace.as_str()));
        }
        object.insert("table", Json::from(table.name.as_str()));
        object.insert("schema", Schema::new(table.fields.clone()).to_json());
        if !table.left_out.is_empty() {
            object.insert(
                "left_out",
                Json::Array(
                    table
                        .left_out
                        .iter()
                        .map(|left| {
                            Json::object([
                                ("column", Json::from(left.column.as_str())),
                                ("database_type", Json::from(left.database_type.as_str())),
                                ("reason", Json::from(left.reason)),
                            ])
                        })
                        .collect(),
                ),
            );
        }
        sources.push(source);
    }
    let file = Json::object([
        ("proposal", Json::Bool(true)),
        ("listen", Json::from("127.0.0.1:8080")),
        ("allow_origins", Json::Array(Vec::new())),
        (
            "tokens",
            Json::Array(vec![Json::object([
                ("env", Json::from(DEFAULT_TOKEN_ENV)),
                ("context", Json::object::<&str, Json>([])),
            ])]),
        ),
        ("sources", Json::Array(sources)),
    ]);
    (pretty(&file), skipped)
}

/// Reads the schema behind `connection` (a libpq string or `sqlite:<path>`)
/// and writes the proposal for it. `namespace` is PostgreSQL's schema.
pub async fn read_proposal(
    connection: &str,
    connection_env: &str,
    namespace: &str,
) -> Result<(String, Vec<String>), ConfigError> {
    let database = Database::of_connection(connection);
    let tables = match database {
        Database::Sqlite => {
            let path = connection
                .strip_prefix("sqlite://")
                .or_else(|| connection.strip_prefix("sqlite:"))
                .unwrap_or(connection);
            if !std::path::Path::new(path).is_file() {
                return Err(ConfigError(format!("there is no SQLite file at {path:?}")));
            }
            let connection = opengrid_connector_sqlite::Connection::open(path)
                .map_err(|e| ConfigError(e.to_string()))?;
            opengrid_connector_sqlite::read_tables(&connection)
                .map_err(|e| ConfigError(e.to_string()))?
        }
        Database::Postgres => opengrid_datasource_postgres::read_tables(connection, namespace)
            .await
            .map_err(|e| ConfigError(e.to_string()))?,
    };
    Ok(proposal(database, connection_env, &tables))
}
