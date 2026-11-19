//! The configuration file: the server builder's options as data.
//!
//! ```json
//! {
//!   "listen": "127.0.0.1:8080",
//!   "allow_origins": ["http://localhost:8080"],
//!   "tokens": [{ "env": "OPENGRID_TOKEN", "context": { "tenant": "acme" } }],
//!   "sources": [{
//!     "name": "orders",
//!     "database": "postgres",
//!     "connection": "OPENGRID_DATABASE_URL",
//!     "namespace": "public",
//!     "table": "orders",
//!     "schema": { "fields": [{ "name": "id", "type": "int64", "nullable": false }] },
//!     "allowed_fields": ["id"],
//!     "row_filter": { "field": "tenant_id", "op": "eq", "value": ":tenant" }
//!   }],
//!   "limits": { "timeout_ms": 10000 }
//! }
//! ```
//!
//! A source's `schema` is the schema JSON the library reads everywhere, so a
//! type is edited, or a derived field added, as it is anywhere else. Unknown
//! keys are refused: a typo must not silently drop a restriction.

use std::collections::BTreeMap;
use std::time::Duration;

use opengrid_json::{Fields, FromJson, Json};
use opengrid_server::{RowFilter, Server, ServerBuilder, SourcePolicy};
use opengrid_types::{DataSourceId, Schema};

/// A token shorter than this stops the start: the demo's own warning says
/// the same ("a random secret of at least 32 characters").
pub const MIN_TOKEN_LENGTH: usize = 32;

/// Why a configuration did not start.
#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn error(message: impl Into<String>) -> ConfigError {
    ConfigError(message.into())
}

/// Which database a source reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Database {
    Postgres,
    Sqlite,
    MySql,
}

impl Database {
    pub fn as_str(self) -> &'static str {
        match self {
            Database::Postgres => "postgres",
            Database::Sqlite => "sqlite",
            Database::MySql => "mysql",
        }
    }

    /// The database a connection string names: `sqlite:<path>` is SQLite,
    /// `mysql://…` MySQL, anything else a libpq string or URL.
    pub fn of_connection(connection: &str) -> Self {
        if connection.starts_with("sqlite:") {
            Database::Sqlite
        } else if connection.starts_with("mysql://") {
            Database::MySql
        } else {
            Database::Postgres
        }
    }
}

/// The parsed file. Parsing checks its shape; [`Config::into_builder`] checks
/// it against the environment and the databases.
#[derive(Debug)]
pub struct Config {
    /// Still a proposal: the owner has not read it.
    pub proposal: bool,
    pub listen: String,
    pub allow_origins: Vec<String>,
    pub tokens: Vec<Token>,
    pub sources: Vec<SourceConfig>,
    pub limits: Limits,
}

#[derive(Debug)]
pub struct Token {
    /// The environment variable that holds the token.
    pub env: String,
    pub context: BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct SourceConfig {
    pub name: String,
    pub database: Database,
    /// The environment variable that holds the connection string.
    pub connection: String,
    pub namespace: Option<String>,
    pub table: String,
    pub schema: Schema,
    pub allowed_fields: Vec<String>,
    pub row_filter: Option<RowFilter>,
}

/// The builder's scalar limits; none set keeps the builder's default.
#[derive(Debug, Default)]
pub struct Limits {
    pub max_payload_bytes: Option<usize>,
    pub timeout_ms: Option<u64>,
    pub max_export_rows: Option<u64>,
    pub max_tree_rows: Option<u64>,
    pub max_xlsx_rows: Option<u64>,
    pub count_cache_ttl_ms: Option<u64>,
    pub max_concurrent_exports: Option<usize>,
}

const DEFAULT_LISTEN: &str = "127.0.0.1:8080";

fn json_error(what: &str, error: opengrid_json::Error) -> ConfigError {
    self::error(format!("{what}: {}", error.message()))
}

impl Config {
    /// Reads the file's text.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let json = Json::parse(text).map_err(|e| json_error("the configuration", e))?;
        let top = Fields::of(
            &json,
            "the configuration",
            &[
                "proposal",
                "listen",
                "allow_origins",
                "tokens",
                "sources",
                "limits",
            ],
        )
        .map_err(|e| json_error("the configuration", e))?;
        let wrap = |what: &'static str| move |e| json_error(what, e);

        let proposal = top
            .read_or_default::<bool>("proposal")
            .map_err(wrap("proposal"))?;
        let listen = top
            .read_optional::<String>("listen")
            .map_err(wrap("listen"))?
            .unwrap_or_else(|| DEFAULT_LISTEN.to_owned());
        let allow_origins = top
            .read_or_default::<Vec<String>>("allow_origins")
            .map_err(wrap("allow_origins"))?;

        let mut tokens = Vec::new();
        for token in array(&top, "tokens")? {
            let fields =
                Fields::of(token, "a token", &["env", "context"]).map_err(wrap("tokens"))?;
            tokens.push(Token {
                env: fields.read("env").map_err(wrap("a token's env"))?,
                context: context(fields.optional("context"))?,
            });
        }

        let mut sources = Vec::new();
        for source in array(&top, "sources")? {
            sources.push(SourceConfig::parse(source)?);
        }

        let limits = match top.optional("limits") {
            None => Limits::default(),
            Some(json) => {
                let fields = Fields::of(
                    json,
                    "limits",
                    &[
                        "max_payload_bytes",
                        "timeout_ms",
                        "max_export_rows",
                        "max_tree_rows",
                        "max_xlsx_rows",
                        "count_cache_ttl_ms",
                        "max_concurrent_exports",
                    ],
                )
                .map_err(wrap("limits"))?;
                Limits {
                    max_payload_bytes: fields
                        .read_optional("max_payload_bytes")
                        .map_err(wrap("limits"))?,
                    timeout_ms: fields.read_optional("timeout_ms").map_err(wrap("limits"))?,
                    max_export_rows: fields
                        .read_optional("max_export_rows")
                        .map_err(wrap("limits"))?,
                    max_tree_rows: fields
                        .read_optional("max_tree_rows")
                        .map_err(wrap("limits"))?,
                    max_xlsx_rows: fields
                        .read_optional("max_xlsx_rows")
                        .map_err(wrap("limits"))?,
                    count_cache_ttl_ms: fields
                        .read_optional("count_cache_ttl_ms")
                        .map_err(wrap("limits"))?,
                    max_concurrent_exports: fields
                        .read_optional("max_concurrent_exports")
                        .map_err(wrap("limits"))?,
                }
            }
        };

        Ok(Self {
            proposal,
            listen,
            allow_origins,
            tokens,
            sources,
            limits,
        })
    }

    /// The server this file describes, with secrets and connection strings
    /// from `env`.
    ///
    /// Refused: a file that is still a proposal, a token or connection string
    /// whose variable is unset, a token that is too short. The builder's own
    /// checks follow at `build`: a name twice, an allowed field or row filter
    /// the schema does not have, a context that does not fit a row filter.
    pub fn into_builder(
        &self,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<ServerBuilder, ConfigError> {
        if self.proposal {
            return Err(error(
                "this file is still a proposal. Read it, edit it, and delete \
                 \"proposal\": true — only what it names is served.",
            ));
        }
        let mut builder = Server::builder();
        for origin in &self.allow_origins {
            builder = builder.allow_origin(origin.clone());
        }
        for token in &self.tokens {
            let value = env(&token.env).ok_or_else(|| {
                error(format!("the environment variable {} is not set", token.env))
            })?;
            if value.chars().count() < MIN_TOKEN_LENGTH {
                return Err(error(format!(
                    "the token in {} is shorter than {MIN_TOKEN_LENGTH} characters: use a random secret",
                    token.env
                )));
            }
            builder = builder.token(
                value,
                token.context.iter().map(|(k, v)| (k.clone(), v.clone())),
            );
        }

        for source in &self.sources {
            DataSourceId::new(&source.name).map_err(|_| {
                error(format!(
                    "source {:?}: the name is not a valid identifier",
                    source.name
                ))
            })?;
            source
                .schema
                .check()
                .map_err(|e| error(format!("source {:?}: its schema: {e}", source.name)))?;
            let connection = env(&source.connection).ok_or_else(|| {
                error(format!(
                    "source {:?}: the environment variable {} is not set",
                    source.name, source.connection
                ))
            })?;
            let connector = source.connector(&connection)?;
            builder = builder.shared_source(
                source.name.clone(),
                connector,
                SourcePolicy {
                    allowed_fields: source.allowed_fields.clone(),
                    row_filter: source.row_filter.clone(),
                },
            );
        }

        let l = &self.limits;
        if let Some(bytes) = l.max_payload_bytes {
            builder = builder.max_payload_bytes(bytes);
        }
        if let Some(ms) = l.timeout_ms {
            builder = builder.timeout(Duration::from_millis(ms));
        }
        if let Some(rows) = l.max_export_rows {
            builder = builder.max_export_rows(rows);
        }
        if let Some(rows) = l.max_tree_rows {
            builder = builder.max_tree_rows(rows);
        }
        if let Some(rows) = l.max_xlsx_rows {
            builder = builder.max_xlsx_rows(rows);
        }
        if let Some(ms) = l.count_cache_ttl_ms {
            builder = builder.count_cache_ttl(Duration::from_millis(ms));
        }
        if let Some(exports) = l.max_concurrent_exports {
            builder = builder.max_concurrent_exports(exports);
        }
        Ok(builder)
    }
}

/// A token's context: an object whose values are strings.
fn context(json: Option<&Json>) -> Result<BTreeMap<String, String>, ConfigError> {
    let Some(json) = json else {
        return Ok(BTreeMap::new());
    };
    let Json::Object(object) = json else {
        return Err(json_error(
            "a token's context",
            json.invalid_type("an object"),
        ));
    };
    object
        .iter()
        .map(|(key, value)| match value.as_str() {
            Some(text) => Ok((key.clone(), text.to_owned())),
            None => Err(json_error(
                "a token's context",
                value.invalid_type("a string"),
            )),
        })
        .collect()
}

/// An optional array member as a slice of its items.
fn array<'a>(fields: &Fields<'a>, key: &str) -> Result<&'a [Json], ConfigError> {
    match fields.optional(key) {
        None => Ok(&[]),
        Some(Json::Array(items)) => Ok(items),
        Some(other) => Err(json_error(key, other.invalid_type("an array"))),
    }
}

impl SourceConfig {
    fn parse(json: &Json) -> Result<Self, ConfigError> {
        let wrap = |e| json_error("a source", e);
        let fields = Fields::of(
            json,
            "a source",
            &[
                "name",
                "database",
                "connection",
                "namespace",
                "table",
                "schema",
                "allowed_fields",
                "row_filter",
                "left_out",
            ],
        )
        .map_err(wrap)?;
        let database = match fields.read::<String>("database").map_err(wrap)?.as_str() {
            "postgres" => Database::Postgres,
            "sqlite" => Database::Sqlite,
            "mysql" => Database::MySql,
            other => {
                return Err(error(format!(
                    "a source's database is {other:?}: it is \"postgres\", \"sqlite\" or \"mysql\""
                )));
            }
        };
        let row_filter = match fields.optional("row_filter") {
            None => None,
            Some(json) => {
                let f = Fields::of(json, "row_filter", &["field", "op", "value"])
                    .map_err(|e| json_error("row_filter", e))?;
                let read = |key| {
                    f.read::<String>(key)
                        .map_err(|e| json_error("row_filter", e))
                };
                Some(RowFilter::new(read("field")?, read("op")?, read("value")?))
            }
        };
        // `left_out` is the proposal's note on what it did not carry: read for
        // its shape, then ignored.
        if let Some(note) = fields.optional("left_out")
            && !note.is_array()
        {
            return Err(json_error("left_out", note.invalid_type("an array")));
        }
        Ok(Self {
            name: fields.read("name").map_err(wrap)?,
            database,
            connection: fields.read("connection").map_err(wrap)?,
            namespace: fields.read_optional("namespace").map_err(wrap)?,
            table: fields.read("table").map_err(wrap)?,
            schema: Schema::from_json(fields.required("schema").map_err(wrap)?)
                .map_err(|e| json_error("a source's schema", e))?,
            allowed_fields: fields.read_or_default("allowed_fields").map_err(wrap)?,
            row_filter,
        })
    }

    /// The source's table, over the connection string `connection`.
    fn connector(
        &self,
        connection: &str,
    ) -> Result<std::sync::Arc<dyn opengrid_connector::Connector>, ConfigError> {
        let failed =
            |e: opengrid_connector::DataSourceError| error(format!("source {:?}: {e}", self.name));
        match self.database {
            Database::Sqlite => {
                let path = connection
                    .strip_prefix("sqlite://")
                    .or_else(|| connection.strip_prefix("sqlite:"))
                    .unwrap_or(connection);
                if !std::path::Path::new(path).is_file() {
                    return Err(error(format!(
                        "source {:?}: there is no SQLite file at {path:?}",
                        self.name
                    )));
                }
                Ok(std::sync::Arc::new(
                    opengrid_connector_sqlite::SqliteConnector::open(
                        path,
                        &self.table,
                        self.schema.clone(),
                    )
                    .map_err(failed)?,
                ))
            }
            Database::Postgres => {
                let source = match &self.namespace {
                    Some(namespace) => {
                        opengrid_datasource_postgres::PostgresDataSource::connect_in_schema(
                            connection,
                            namespace,
                            &self.table,
                            self.schema.clone(),
                        )
                    }
                    None => opengrid_datasource_postgres::PostgresDataSource::connect(
                        connection,
                        &self.table,
                        self.schema.clone(),
                    ),
                }
                .map_err(failed)?;
                Ok(std::sync::Arc::new(source))
            }
            // The namespace of a MySQL source is its database, if not the URL's.
            Database::MySql => {
                let source = match &self.namespace {
                    Some(database) => {
                        opengrid_connector_mysql::MySqlConnector::connect_in_database(
                            connection,
                            database,
                            &self.table,
                            self.schema.clone(),
                        )
                    }
                    None => opengrid_connector_mysql::MySqlConnector::connect(
                        connection,
                        &self.table,
                        self.schema.clone(),
                    ),
                }
                .map_err(failed)?;
                Ok(std::sync::Arc::new(source))
            }
        }
    }
}
