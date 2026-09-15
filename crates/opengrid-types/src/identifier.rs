use std::fmt;

use serde::{Deserialize, Serialize};

use crate::InvalidIdentifier;

/// The identifier rule shared by field names, data source ids and (later) SQL
/// identifiers: `^[A-Za-z_][A-Za-z0-9_]{0,62}$`.
///
/// Keeping SQL identifiers on the same rule means a validated identifier is
/// always safe to interpolate, even though the compiler still never takes
/// identifiers from user input (plan/spezifikation/07-server.md §Sicherheit).
pub fn is_valid_identifier(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    let first = bytes[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

macro_rules! identifier {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validates and wraps the given identifier.
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidIdentifier> {
                let value = value.into();
                if is_valid_identifier(&value) {
                    Ok(Self(value))
                } else {
                    Err(InvalidIdentifier(value))
                }
            }

            /// The identifier as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidIdentifier;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = InvalidIdentifier;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

identifier! {
    /// Name of a column in a [`Schema`](crate::Schema).
    FieldName
}

identifier! {
    /// Identifier of a data source in a query (`"source"`).
    DataSourceId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_identifiers() {
        for ok in ["a", "_", "_x", "x1", "amount", "A_b_9", &"a".repeat(63)] {
            assert!(is_valid_identifier(ok), "{ok:?} should be valid");
            assert!(FieldName::new(ok).is_ok());
            assert!(DataSourceId::new(ok).is_ok());
        }
    }

    #[test]
    fn rejects_invalid_identifiers() {
        for bad in [
            "a b",
            "1x",
            "",
            "x;drop",
            "x-1",
            "ä",
            "a.".repeat(2).as_str(),
        ] {
            assert!(!is_valid_identifier(bad), "{bad:?} should be invalid");
        }
        assert!(!is_valid_identifier(&"a".repeat(64)));
    }
}
