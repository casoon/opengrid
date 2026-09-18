//! The server's configuration file (plan/spezifikation/07-server.md).
//!
//! TOML, read once at startup. An incomplete or contradictory configuration is a
//! **startup** error, never a runtime surprise: a gateway that comes up with a
//! half-built source would answer requests it should refuse.
//!
//! ```toml
//! [server]
//! address = "127.0.0.1:8081"
//!
//! [[tokens]]
//! value = "${ORDERS_API_TOKEN}"
//! context = { tenant = "acme" }
//!
//! [[datasources]]
//! name = "orders"
//! type = "local-csv"
//! path = "data/orders.csv"
//! schema = "data/orders.schema.json"
//! allowed_fields = ["id", "customer", "country", "amount"]
//! row_filter = { field = "country", op = "eq", value = ":tenant" }
//! ```
//!
//! `${VAR}` in a token value reads the environment — a secret belongs in the
//! deployment, not in a file that gets committed (E15).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The whole file.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    /// The tokens that may query. An empty list means **nobody** may — a server
    /// without tokens answers every request with `unauthorized`, which is the
    /// safe reading of "none configured" (E15).
    #[serde(default)]
    pub tokens: Vec<TokenConfig>,
    #[serde(default)]
    pub datasources: Vec<SourceConfig>,
}

/// Limits and the address, all optional.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_address")]
    pub address: String,
    /// Largest request body, in bytes (07-server.md §Sicherheit).
    #[serde(default = "default_max_payload")]
    pub max_payload_bytes: usize,
    /// How long a single query may take before it is cut off.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// `Limits::max_limit` for every source (02-query-modell.md).
    #[serde(default)]
    pub max_limit: Option<u64>,
    /// `Limits::max_depth` for every source.
    #[serde(default)]
    pub max_depth: Option<usize>,
    /// Origins a browser may call this server from. Empty means **none**: no
    /// CORS headers are sent, and a page on another origin cannot read the
    /// answer. Opt in per origin, never `*` — a wildcard plus a bearer token is
    /// a token handed to every site the user visits.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: default_address(),
            max_payload_bytes: default_max_payload(),
            timeout_ms: default_timeout_ms(),
            max_limit: None,
            max_depth: None,
            allowed_origins: Vec::new(),
        }
    }
}

fn default_address() -> String {
    "127.0.0.1:8081".to_owned()
}

fn default_max_payload() -> usize {
    64 * 1024
}

fn default_timeout_ms() -> u64 {
    10_000
}

/// One accepted bearer token and the context it stands for.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenConfig {
    /// The secret, or `${VAR}` to read it from the environment.
    pub value: String,
    /// Values a `row_filter` may reference as `:name` — the tenant, the account,
    /// whatever separates one caller's rows from another's (E16).
    #[serde(default)]
    pub context: BTreeMap<String, String>,
}

/// One configured data source.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    pub name: String,
    /// `local-csv` in point 24. `postgres` arrives with point 26.
    #[serde(rename = "type")]
    pub kind: String,
    pub path: PathBuf,
    pub schema: PathBuf,
    /// The columns a client may name. Empty means every column of the schema.
    #[serde(default)]
    pub allowed_fields: Vec<String>,
    /// The filter the server always adds (E16).
    #[serde(default)]
    pub row_filter: Option<RowFilterConfig>,
}

/// A mandatory row filter: `field op value`, where `value` may be `:key` to take
/// the caller's context value.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowFilterConfig {
    pub field: String,
    pub op: String,
    pub value: String,
}

/// Why a configuration could not be used.
#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    Env {
        variable: String,
    },
    Invalid {
        message: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Read { path, source } => write!(f, "{}: {source}", path.display()),
            ConfigError::Parse { path, source } => write!(f, "{}: {source}", path.display()),
            ConfigError::Env { variable } => {
                write!(f, "environment variable {variable} is not set")
            }
            ConfigError::Invalid { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Reads a configuration file and resolves `${VAR}` references.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut config: Config = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        for token in &mut config.tokens {
            token.value = interpolate(&token.value, |name| std::env::var(name).ok())?;
        }
        config.check()?;
        Ok(config)
    }

    /// The checks that must hold before the first request arrives.
    fn check(&self) -> Result<(), ConfigError> {
        if self.datasources.is_empty() {
            return Err(ConfigError::Invalid {
                message: "no [[datasources]] configured".to_owned(),
            });
        }
        let mut seen = Vec::new();
        for source in &self.datasources {
            if seen.contains(&source.name) {
                return Err(ConfigError::Invalid {
                    message: format!("datasource {:?} is configured twice", source.name),
                });
            }
            seen.push(source.name.clone());
        }
        for token in &self.tokens {
            if token.value.trim().is_empty() {
                return Err(ConfigError::Invalid {
                    message: "a token value is empty".to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Replaces a whole `${VAR}` value from `lookup` (the environment in production).
///
/// Deliberately all-or-nothing: a value is either a literal or exactly one
/// variable. Substring interpolation would invite secrets that are half in the
/// file and half in the environment.
///
/// The source is a parameter so this is testable without touching the process
/// environment — which in edition 2024 is `unsafe`, and the workspace forbids
/// `unsafe`.
fn interpolate(
    value: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, ConfigError> {
    let Some(variable) = value
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return Ok(value.to_owned());
    };
    lookup(variable).ok_or_else(|| ConfigError::Env {
        variable: variable.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn a_literal_token_stays_as_it_is() {
        assert_eq!(interpolate("plain", env(&[])).unwrap(), "plain");
    }

    #[test]
    fn a_variable_is_read_from_the_environment() {
        let lookup = env(&[("ORDERS_API_TOKEN", "s3cret")]);
        assert_eq!(
            interpolate("${ORDERS_API_TOKEN}", lookup).unwrap(),
            "s3cret"
        );
    }

    /// A secret that is not there must stop the server, not become the literal
    /// string `${ORDERS_API_TOKEN}` — which would then be a valid token.
    #[test]
    fn a_missing_variable_is_a_startup_error() {
        let error = interpolate("${ORDERS_API_TOKEN}", env(&[])).unwrap_err();
        assert!(matches!(error, ConfigError::Env { .. }));
    }
}
