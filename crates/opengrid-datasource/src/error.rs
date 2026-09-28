/// What a data source fails with.
///
/// Deliberately small and backend-neutral: the trait is portable
/// (plan/spezifikation/11-crates.md §Portabilität), so it cannot name the local
/// engine's or a backend's error type. An implementation that has a richer error
/// formats it into [`Backend`](Self::Backend) — the path the local adapter takes
/// for `ExecuteError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataSourceError {
    /// The source holds no data at all: no schema to declare, nothing to run a
    /// query against.
    NoData,
    /// The source failed — engine, backend or transport. The message carries the
    /// diagnosis.
    Backend { message: String },
    /// The query would need more than the source may do for one answer — more
    /// rows to read than it is allowed to hold, say. The caller can narrow the
    /// query; nothing is broken.
    LimitExceeded { message: String },
}

impl std::fmt::Display for DataSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataSourceError::NoData => f.write_str("the data source holds no data"),
            DataSourceError::Backend { message } | DataSourceError::LimitExceeded { message } => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for DataSourceError {}
