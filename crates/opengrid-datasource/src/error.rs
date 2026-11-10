use opengrid_types::Value;

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
    /// The data breaks a rule the query relies on — a tree with a key twice or
    /// a cycle (T1, T3). Not an outage and not the query's fault: the message
    /// names the rows, was written to be read by the caller, and a server may
    /// show it.
    Data { message: String },
}

impl std::fmt::Display for DataSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataSourceError::NoData => f.write_str("the data source holds no data"),
            DataSourceError::Backend { message }
            | DataSourceError::LimitExceeded { message }
            | DataSourceError::Data { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for DataSourceError {}

/// The sentence for a tree that holds a key twice (rule T1).
///
/// One wording for every way a level is answered — the engine, SQLite,
/// PostgreSQL — so that the same data says the same thing, whichever source a
/// page is connected to.
pub fn tree_duplicate_key(key: &Value) -> String {
    format!(
        "tree: the key {} appears more than once (rule T1)",
        named(key)
    )
}

/// The sentence for a tree with a cycle (rule T3); `key` is a node on it.
pub fn tree_cycle(key: &Value) -> String {
    format!(
        "tree: the parents of {} lead back to it — a cycle (rule T3)",
        named(key)
    )
}

/// A key as a sentence names it: text in quotes, every other value as its JSON.
fn named(value: &Value) -> String {
    match value {
        Value::Utf8(text) => format!("{text:?}"),
        other => opengrid_json::to_string(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::DataType;

    fn read(text: &str, data_type: DataType) -> Value {
        Value::from_wire_str(text, &data_type).expect(text)
    }

    /// The spelling every backend used before there was one `named`: written out
    /// per type. The shared one must say the same for every kind of key.
    fn spelled_out(value: &Value) -> String {
        match value {
            Value::Utf8(text) => format!("{text:?}"),
            Value::Int64(number) => number.to_string(),
            Value::Float64(number) if number.is_nan() => "\"NaN\"".to_owned(),
            Value::Float64(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            Value::Decimal(decimal) => format!("{:?}", decimal.to_string()),
            Value::Date(date) => format!("{:?}", date.to_string()),
            Value::Timestamp(timestamp) => format!("{:?}", timestamp.to_string()),
            Value::Null => "null".to_owned(),
            Value::List(_) => opengrid_json::ToJson::to_json(value).to_string(),
        }
    }

    #[test]
    fn a_key_is_named_the_way_every_backend_named_it() {
        let keys = [
            Value::Utf8("a \"quoted\" key".to_owned()),
            Value::Int64(-7),
            Value::Float64(1.5),
            Value::Float64(f64::NAN),
            Value::Bool(true),
            read(
                "12.50",
                DataType::Decimal {
                    precision: 10,
                    scale: 2,
                },
            ),
            read("2026-11-10", DataType::Date),
            read("2026-11-10T12:00:00.000001Z", DataType::Timestamp),
            Value::Null,
            Value::List(vec![Value::Int64(1), Value::Int64(2)]),
        ];
        for key in &keys {
            assert_eq!(named(key), spelled_out(key), "{key:?}");
        }
    }

    #[test]
    fn the_two_sentences_name_the_key() {
        assert_eq!(
            tree_duplicate_key(&Value::Utf8("Sales".to_owned())),
            "tree: the key \"Sales\" appears more than once (rule T1)"
        );
        assert_eq!(
            tree_cycle(&Value::Int64(4)),
            "tree: the parents of 4 lead back to it — a cycle (rule T3)"
        );
    }
}
