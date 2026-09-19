use serde::{Deserialize, Serialize};

use crate::{DataType, FieldName, SchemaError, Value};

/// The part of a date or timestamp a column can be derived from (plan point 54).
///
/// A **closed** list, deliberately: this is not an expression language. V1 has
/// no calculated fields, and the two parts here exist because a pivot by year
/// or month is otherwise not expressible at all
/// (plan/spezifikation/06-pivot.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatePart {
    /// The civil year in UTC, e.g. `2026`.
    Year,
    /// The civil month in UTC, `1..=12`.
    Month,
}

impl DatePart {
    /// The wire name, identical to the JSON string.
    pub fn as_str(self) -> &'static str {
        match self {
            DatePart::Year => "year",
            DatePart::Month => "month",
        }
    }

    /// The part of `value`, as the [`Int64`](DataType::Int64) it always is.
    ///
    /// NULL stays NULL — a row without a date has no year. The computation is in
    /// **UTC** (rule S9): the engine never converts time zones, so the year of
    /// `2025-12-31T23:59:59.000000Z` is 2025 and the year of the microsecond
    /// after it is 2026.
    ///
    /// The schema check guarantees the source column is a `Date` or a
    /// `Timestamp`; anything else here is a broken invariant, not a data error.
    pub fn of(self, value: &Value) -> Value {
        let (year, month, _) = match value {
            Value::Null => return Value::Null,
            Value::Date(date) => date.ymd(),
            Value::Timestamp(timestamp) => timestamp.date().ymd(),
            other => panic!("a derived column reads a date or a timestamp, not {other:?}"),
        };
        Value::Int64(match self {
            DatePart::Year => i64::from(year),
            DatePart::Month => i64::from(month),
        })
    }
}

/// Where a field's values come from, when they are not in the data (point 54).
///
/// JSON form: `"from": { "part": "year", "field": "ordered_on" }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Derivation {
    pub part: DatePart,
    pub field: FieldName,
}

/// One column of a [`Schema`].
///
/// The JSON form is `{ "name", "type", "nullable" }` (plan point 23) — the shape
/// `crates/opengrid-conformance/data/orders.schema.json` has used since point 05.
/// A missing `nullable` reads as `false`: a column is required unless the schema
/// says otherwise, which is the safer default of the two.
///
/// Since point 54 a field may also carry `"from"` and then have **no column in
/// the data**: its values are computed from another field. Everything downstream
/// — validation, the planner, filters, sorting, the grid — sees an ordinary
/// column, which is the whole point of putting the derivation here instead of
/// into the query model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: FieldName,
    #[serde(rename = "type")]
    pub data_type: DataType,
    #[serde(default)]
    pub nullable: bool,
    /// The field this one is computed from, if it is not in the data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Derivation>,
}

impl Field {
    /// A nullable field.
    pub fn new(name: FieldName, data_type: DataType) -> Self {
        Self {
            name,
            data_type,
            nullable: true,
            from: None,
        }
    }

    /// A non-nullable field.
    pub fn required(name: FieldName, data_type: DataType) -> Self {
        Self {
            name,
            data_type,
            nullable: false,
            from: None,
        }
    }

    /// A field computed from `source`, always [`Int64`](DataType::Int64).
    ///
    /// Nullable, because the source can be NULL and then so is the part.
    pub fn derived(name: FieldName, part: DatePart, source: FieldName) -> Self {
        Self {
            name,
            data_type: DataType::Int64,
            nullable: true,
            from: Some(Derivation {
                part,
                field: source,
            }),
        }
    }

    /// Whether this column is computed rather than stored.
    pub fn is_derived(&self) -> bool {
        self.from.is_some()
    }
}

/// An ordered list of [`Field`]s, looked up by name.
///
/// JSON form: `{ "fields": [ … ] }` (plan point 23).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

    /// Whether any field is computed rather than stored.
    pub fn has_derived(&self) -> bool {
        self.fields.iter().any(Field::is_derived)
    }

    /// The columns that are actually **in the data** — the file, the table.
    ///
    /// This is the schema a CSV header is checked against and a row is read
    /// with; the derived columns are computed afterwards.
    pub fn stored(&self) -> Schema {
        Schema::new(
            self.fields
                .iter()
                .filter(|field| !field.is_derived())
                .cloned()
                .collect(),
        )
    }

    /// The same columns, with the derivations resolved away.
    ///
    /// What a batch looks like once ingest has run: the derived column exists
    /// and holds values, so it is no longer derived from anything. Comparing a
    /// declared schema against one read back from data goes through here.
    pub fn materialized(&self) -> Schema {
        Schema::new(
            self.fields
                .iter()
                .map(|field| Field {
                    from: None,
                    ..field.clone()
                })
                .collect(),
        )
    }

    /// Checks the derivations — every place that loads a schema calls this.
    ///
    /// Four ways to get it wrong, all of them a configuration error and none of
    /// them something to discover later as a column full of NULL:
    /// an unknown source column, a source that is not a date or a timestamp, a
    /// derivation of a derivation, a declared type that is not `Int64`, and a
    /// required column derived from one that may be NULL.
    pub fn check(&self) -> Result<(), SchemaError> {
        for field in &self.fields {
            let Some(derivation) = &field.from else {
                continue;
            };
            let name = field.name.to_string();
            let source = self.field(derivation.field.as_str()).ok_or_else(|| {
                SchemaError::UnknownSource {
                    field: name.clone(),
                    source: derivation.field.to_string(),
                }
            })?;
            if source.is_derived() {
                return Err(SchemaError::DerivedSource {
                    field: name,
                    source: source.name.to_string(),
                });
            }
            if !matches!(source.data_type, DataType::Date | DataType::Timestamp) {
                return Err(SchemaError::NotTemporal {
                    field: name,
                    source: source.name.to_string(),
                    data_type: source.data_type,
                });
            }
            if field.data_type != DataType::Int64 {
                return Err(SchemaError::WrongType {
                    field: name,
                    data_type: field.data_type,
                });
            }
            if source.nullable && !field.nullable {
                return Err(SchemaError::NotNullable {
                    field: name,
                    source: source.name.to_string(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Date, Timestamp};

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

    fn name(raw: &str) -> FieldName {
        FieldName::new(raw).expect("a test field name")
    }

    /// With a derived column: the data has nine columns, the schema ten.
    fn derived_schema() -> Schema {
        Schema::new(vec![
            Field::required(name("id"), DataType::Int64),
            Field::new(name("ordered_on"), DataType::Date),
            Field::new(name("created_at"), DataType::Timestamp),
            Field::derived(name("ordered_year"), DatePart::Year, name("ordered_on")),
            Field::derived(name("created_month"), DatePart::Month, name("created_at")),
        ])
    }

    /// The year is the UTC year, and one microsecond decides it (S9).
    #[test]
    fn a_date_part_is_computed_in_utc() {
        let last = Value::Timestamp(Timestamp::from_micros(1_767_225_599_999_999));
        let first = Value::Timestamp(Timestamp::from_micros(1_767_225_600_000_000));
        assert_eq!(DatePart::Year.of(&last), Value::Int64(2025));
        assert_eq!(DatePart::Year.of(&first), Value::Int64(2026));
        assert_eq!(DatePart::Month.of(&last), Value::Int64(12));
        assert_eq!(DatePart::Month.of(&first), Value::Int64(1));

        let day = Value::Date(Date::from_ymd(2026, 3, 9).unwrap());
        assert_eq!(DatePart::Year.of(&day), Value::Int64(2026));
        assert_eq!(DatePart::Month.of(&day), Value::Int64(3));
    }

    /// A row without a date has no year.
    #[test]
    fn a_null_source_stays_null() {
        assert_eq!(DatePart::Year.of(&Value::Null), Value::Null);
        assert_eq!(DatePart::Month.of(&Value::Null), Value::Null);
    }

    /// Before the epoch the day is floored, not truncated.
    #[test]
    fn a_timestamp_before_the_epoch_keeps_its_day() {
        // 1969-12-31T23:59:59.999999Z — one microsecond before the epoch.
        let value = Value::Timestamp(Timestamp::from_micros(-1));
        assert_eq!(DatePart::Year.of(&value), Value::Int64(1969));
        assert_eq!(DatePart::Month.of(&value), Value::Int64(12));
    }

    /// `stored` is the data, `materialized` is the data after ingest.
    #[test]
    fn a_derived_column_is_not_in_the_data() {
        let schema = derived_schema();
        assert!(schema.has_derived());
        assert_eq!(schema.len(), 5);

        let stored = schema.stored();
        assert_eq!(stored.len(), 3, "the file has three columns");
        assert!(stored.field("ordered_year").is_none());

        let materialized = schema.materialized();
        assert_eq!(materialized.len(), 5, "after ingest the column is there");
        assert!(!materialized.has_derived());
        assert_eq!(
            materialized.field("ordered_year").unwrap().data_type,
            DataType::Int64
        );
    }

    /// Four ways to declare a derivation that cannot work.
    #[test]
    fn a_broken_derivation_is_caught_where_the_schema_is_loaded() {
        assert_eq!(derived_schema().check(), Ok(()));

        let unknown = Schema::new(vec![Field::derived(
            name("y"),
            DatePart::Year,
            name("nope"),
        )]);
        assert!(matches!(
            unknown.check(),
            Err(SchemaError::UnknownSource { .. })
        ));

        let not_temporal = Schema::new(vec![
            Field::new(name("customer"), DataType::Utf8),
            Field::derived(name("y"), DatePart::Year, name("customer")),
        ]);
        assert!(matches!(
            not_temporal.check(),
            Err(SchemaError::NotTemporal { .. })
        ));

        let chained = Schema::new(vec![
            Field::new(name("ordered_on"), DataType::Date),
            Field::derived(name("y"), DatePart::Year, name("ordered_on")),
            Field::derived(name("z"), DatePart::Year, name("y")),
        ]);
        assert!(matches!(
            chained.check(),
            Err(SchemaError::DerivedSource { .. })
        ));

        let mut wrong_type = derived_schema();
        wrong_type.fields[3].data_type = DataType::Utf8;
        assert!(matches!(
            wrong_type.check(),
            Err(SchemaError::WrongType { .. })
        ));

        // A required column out of a nullable date would be a column that
        // cannot hold what it is given.
        let mut required = derived_schema();
        required.fields[3].nullable = false;
        assert!(matches!(
            required.check(),
            Err(SchemaError::NotNullable { .. })
        ));
    }

    /// The JSON form of an ordinary field is unchanged — no `from` key appears.
    #[test]
    fn only_a_derived_field_carries_its_origin() {
        let plain = serde_json::to_string(&Field::required(name("id"), DataType::Int64)).unwrap();
        assert_eq!(plain, r#"{"name":"id","type":"int64","nullable":false}"#);

        let derived = Field::derived(name("ordered_year"), DatePart::Year, name("ordered_on"));
        let json = serde_json::to_string(&derived).unwrap();
        assert_eq!(
            json,
            r#"{"name":"ordered_year","type":"int64","nullable":true,"from":{"part":"year","field":"ordered_on"}}"#
        );
        assert_eq!(serde_json::from_str::<Field>(&json).unwrap(), derived);
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
