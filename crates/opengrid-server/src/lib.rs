//! `opengrid-server` — the data gateway (plan point 24,
//! plan/spezifikation/07-server.md).
//!
//! Browser → HTTPS → this server → a data source. It accepts **only** the query
//! AST of `opengrid-query`, never SQL, and answers in the wire form of point 23.
//!
//! The source in this point is the local engine over a CSV file. That is
//! deliberate: configuration, registry, validation, limits and both security
//! promises are fully testable before a database is involved, and PostgreSQL
//! (point 26) then swaps only what sits behind the registry.
//!
//! Two promises the endpoint keeps, decided in point 22 (E15, E16):
//!
//! * **A bearer token from the configuration** guards every request, compared in
//!   constant time. The token carries a context.
//! * **A mandatory row filter** per source is added to every query, with values
//!   from that context. It is enforced here, not in the client, and no request
//!   can switch it off.

pub mod api;
pub mod config;
pub mod registry;

use std::path::Path;
use std::sync::Arc;

pub use api::{AppState, router};
pub use config::Config;
pub use registry::Registry;

/// Builds the router from a configuration file.
///
/// `base` is the directory relative paths in the configuration resolve against —
/// normally the directory the configuration file lives in, so a configuration
/// plus its data can be moved as one.
pub fn build(config_path: &Path) -> Result<(Arc<AppState>, Config), Box<dyn std::error::Error>> {
    let config = Config::load(config_path)?;
    let base = config_path.parent().unwrap_or(Path::new("."));
    let registry = Registry::build(&config, base)?;
    let state = Arc::new(AppState::new(&config, registry));
    Ok((state, config))
}
