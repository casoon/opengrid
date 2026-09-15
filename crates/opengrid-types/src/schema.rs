use crate::{DataType, FieldName};

/// One column of a [`Schema`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: FieldName,
    pub data_type: DataType,
    pub nullable: bool,
}

impl Field {
    /// A nullable field.
    pub fn new(name: FieldName, data_type: DataType) -> Self {
        Self {
            name,
            data_type,
            nullable: true,
        }
    }

    /// A non-nullable field.
    pub fn required(name: FieldName, data_type: DataType) -> Self {
        Self {
            name,
            data_type,
            nullable: false,
        }
    }
}

/// An ordered list of [`Field`]s, looked up by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schema {
    fields: Vec<Field>,
}

impl Schema {
    /// Builds a schema from fields in projection order.
    pub fn new(fields: Vec<Field>) -> Self {
        Self { fields }
    }

    /// The fields in projection order.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Number of fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// True when the schema has no fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// The field with the given name, if present.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name.as_str() == name)
    }

    /// The position of the field with the given name, if present.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name.as_str() == name)
    }

    /// The type of the field with the given name, if present.
    pub fn data_type(&self, name: &str) -> Option<DataType> {
        self.field(name).map(|f| f.data_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        Schema::new(vec![
            Field::required(FieldName::new("id").unwrap(), DataType::Int64),
            Field::new(FieldName::new("country").unwrap(), DataType::Utf8),
            Field::new(
                FieldName::new("amount").unwrap(),
                DataType::decimal(12, 2).unwrap(),
            ),
        ])
    }

    #[test]
    fn looks_up_by_name() {
        let s = schema();
        assert_eq!(s.len(), 3);
        assert!(!s.is_empty());
        assert_eq!(s.index_of("country"), Some(1));
        assert_eq!(
            s.data_type("amount"),
            Some(DataType::Decimal {
                precision: 12,
                scale: 2
            })
        );
        assert!(s.field("missing").is_none());
        assert!(!s.field("id").unwrap().nullable);
        assert!(s.field("country").unwrap().nullable);
    }
}
