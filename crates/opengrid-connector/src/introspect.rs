//! What reading a database's catalogue answers (issue #486): per table, the
//! columns opengrid can serve with the type it would serve them as, and the
//! columns it cannot, named and left out.
//!
//! The catalogue is read next to each dialect (`opengrid-connector-sqlite`,
//! `opengrid-datasource-postgres`); this is the shape they share. Reading it
//! releases nothing: an owner turns it into a configuration, and only what
//! that file names is served.

use opengrid_types::{FieldName, is_valid_identifier};

pub use opengrid_types::Field;

/// A table the catalogue holds, with its columns sorted into the ones that map
/// to an opengrid type and the ones that do not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableProposal {
    /// The database's schema (PostgreSQL's `public`); none for SQLite.
    pub namespace: Option<String>,
    pub name: String,
    /// The columns with a type, in the table's order.
    pub fields: Vec<Field>,
    /// The columns without one: never guessed, never served.
    pub left_out: Vec<LeftOut>,
}

/// A column the proposal does not carry, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    pub column: String,
    /// The type as the database names it.
    pub database_type: String,
    pub reason: &'static str,
}

/// The reason for a type with no mapping.
pub const NO_MAPPING: &str = "the type has no opengrid type";
/// The reason for a name that is no field name (`^[A-Za-z_][A-Za-z0-9_]{0,62}$`).
pub const NOT_A_FIELD_NAME: &str = "the name is not a valid field name";

impl TableProposal {
    pub fn new(namespace: Option<String>, name: impl Into<String>) -> Self {
        Self {
            namespace,
            name: name.into(),
            fields: Vec::new(),
            left_out: Vec::new(),
        }
    }

    /// A column whose type maps (`None` leaves it out, as `database_type`).
    /// A name that cannot be a field name is left out too.
    pub fn column(
        &mut self,
        name: &str,
        database_type: &str,
        mapped: Option<opengrid_types::DataType>,
        nullable: bool,
    ) {
        let leave_out = |table: &mut Self, reason| {
            table.left_out.push(LeftOut {
                column: name.to_owned(),
                database_type: database_type.to_owned(),
                reason,
            });
        };
        let Some(data_type) = mapped else {
            return leave_out(self, NO_MAPPING);
        };
        if !is_valid_identifier(name) {
            return leave_out(self, NOT_A_FIELD_NAME);
        }
        let name = FieldName::new(name).expect("checked");
        self.fields.push(if nullable {
            Field::new(name, data_type)
        } else {
            Field::required(name, data_type)
        });
    }
}
