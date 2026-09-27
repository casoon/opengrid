//! One typed column: a value buffer and, when it holds NULLs, a validity bitmap.

use opengrid_types::{DataType, Date, Decimal, Timestamp, Value};

use crate::Bitmap;

/// The values of a column, one buffer per type of the query model.
///
/// A NULL cell holds the type's zero value (`0`, `false`, `""`) here; whether
/// the cell *is* NULL is the column's validity bitmap's business, never the
/// buffer's.
#[derive(Clone, Debug, PartialEq)]
pub enum Values {
    Bool(Vec<bool>),
    Int64(Vec<i64>),
    /// NaN is a value, not a NULL (rule S7).
    Float64(Vec<f64>),
    /// The coefficient; the scale comes from the column's type.
    Decimal(Vec<i128>),
    /// All strings of the column in one buffer; string `i` is
    /// `text[offsets[i]..offsets[i + 1]]`.
    Utf8 {
        offsets: Vec<u32>,
        text: String,
    },
    /// Days since 1970-01-01.
    Date(Vec<i32>),
    /// Microseconds since 1970-01-01T00:00:00Z (rule S9).
    Timestamp(Vec<i64>),
}

impl Values {
    fn with_capacity(data_type: DataType, capacity: usize) -> Self {
        match data_type {
            DataType::Bool => Values::Bool(Vec::with_capacity(capacity)),
            DataType::Int64 => Values::Int64(Vec::with_capacity(capacity)),
            DataType::Float64 => Values::Float64(Vec::with_capacity(capacity)),
            DataType::Decimal { .. } => Values::Decimal(Vec::with_capacity(capacity)),
            DataType::Utf8 => {
                let mut offsets = Vec::with_capacity(capacity + 1);
                offsets.push(0);
                Values::Utf8 {
                    offsets,
                    text: String::new(),
                }
            }
            DataType::Date => Values::Date(Vec::with_capacity(capacity)),
            DataType::Timestamp => Values::Timestamp(Vec::with_capacity(capacity)),
        }
    }

    fn len(&self) -> usize {
        match self {
            Values::Bool(values) => values.len(),
            Values::Int64(values) | Values::Timestamp(values) => values.len(),
            Values::Float64(values) => values.len(),
            Values::Decimal(values) => values.len(),
            Values::Utf8 { offsets, .. } => offsets.len() - 1,
            Values::Date(values) => values.len(),
        }
    }

    /// Appends the zero value of the type — the placeholder under a NULL.
    fn push_zero(&mut self) {
        match self {
            Values::Bool(values) => values.push(false),
            Values::Int64(values) | Values::Timestamp(values) => values.push(0),
            Values::Float64(values) => values.push(0.0),
            Values::Decimal(values) => values.push(0),
            Values::Utf8 { offsets, text } => offsets.push(text.len() as u32),
            Values::Date(values) => values.push(0),
        }
    }

    /// The values at `positions`, in that order.
    fn gather(&self, positions: impl Iterator<Item = usize> + Clone) -> Values {
        match self {
            Values::Bool(values) => Values::Bool(positions.map(|at| values[at]).collect()),
            Values::Int64(values) => Values::Int64(positions.map(|at| values[at]).collect()),
            Values::Float64(values) => Values::Float64(positions.map(|at| values[at]).collect()),
            Values::Decimal(values) => Values::Decimal(positions.map(|at| values[at]).collect()),
            Values::Utf8 { offsets, text } => {
                let bytes: usize = positions
                    .clone()
                    .map(|at| (offsets[at + 1] - offsets[at]) as usize)
                    .sum();
                let mut out = String::with_capacity(bytes);
                let mut out_offsets = Vec::with_capacity(offsets.len());
                out_offsets.push(0);
                for at in positions {
                    out.push_str(&text[offsets[at] as usize..offsets[at + 1] as usize]);
                    out_offsets.push(out.len() as u32);
                }
                Values::Utf8 {
                    offsets: out_offsets,
                    text: out,
                }
            }
            Values::Date(values) => Values::Date(positions.map(|at| values[at]).collect()),
            Values::Timestamp(values) => {
                Values::Timestamp(positions.map(|at| values[at]).collect())
            }
        }
    }
}

/// One column of a [`Table`](crate::Table).
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub(crate) data_type: DataType,
    /// `None` when no cell is NULL.
    pub(crate) validity: Option<Bitmap>,
    pub(crate) values: Values,
}

impl Column {
    /// A column of `len` NULLs.
    pub fn nulls(data_type: DataType, len: usize) -> Self {
        let mut builder = ColumnBuilder::new(data_type, len);
        for _ in 0..len {
            builder.push_null();
        }
        builder.finish()
    }

    /// The column's type.
    pub fn data_type(&self) -> DataType {
        self.data_type
    }

    /// The value buffer.
    pub fn values(&self) -> &Values {
        &self.values
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when the column has no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the cell at `row` is NULL.
    pub fn is_null(&self, row: usize) -> bool {
        self.validity
            .as_ref()
            .is_some_and(|validity| !validity.get(row))
    }

    /// Number of NULL cells.
    pub fn null_count(&self) -> usize {
        self.validity.as_ref().map_or(0, Bitmap::count_cleared)
    }

    /// The string at `row` of a `Utf8` column — the empty string under a NULL.
    ///
    /// # Panics
    /// When the column is not `Utf8`; the caller has checked the type.
    pub fn str(&self, row: usize) -> &str {
        match &self.values {
            Values::Utf8 { offsets, text } => {
                &text[offsets[row] as usize..offsets[row + 1] as usize]
            }
            _ => panic!("str() on a {} column", self.data_type),
        }
    }

    /// The cell at `row` as a [`Value`].
    pub fn value(&self, row: usize) -> Value {
        if self.is_null(row) {
            return Value::Null;
        }
        match &self.values {
            Values::Bool(values) => Value::Bool(values[row]),
            Values::Int64(values) => Value::Int64(values[row]),
            Values::Float64(values) => Value::Float64(values[row]),
            Values::Decimal(values) => {
                let DataType::Decimal { scale, .. } = self.data_type else {
                    unreachable!("a decimal buffer belongs to a decimal column")
                };
                Value::Decimal(Decimal::new(values[row], scale))
            }
            Values::Utf8 { .. } => Value::Utf8(self.str(row).to_owned()),
            Values::Date(values) => Value::Date(Date::from_days_since_epoch(values[row])),
            Values::Timestamp(values) => Value::Timestamp(Timestamp::from_micros(values[row])),
        }
    }

    /// The rows at `positions`, in that order.
    pub fn take(&self, positions: &[u32]) -> Column {
        let positions = positions.iter().map(|at| *at as usize);
        Column {
            data_type: self.data_type,
            validity: self.validity.as_ref().map(|validity| {
                let mut out = Bitmap::with_capacity(positions.len());
                for at in positions.clone() {
                    out.push(validity.get(at));
                }
                out
            }),
            values: self.values.gather(positions),
        }
    }

    /// The rows at `positions`, with a NULL wherever a position is `None`.
    pub fn take_optional(&self, positions: &[Option<u32>]) -> Column {
        let mut validity = Bitmap::with_capacity(positions.len());
        for position in positions {
            validity.push(position.is_some_and(|at| !self.is_null(at as usize)));
        }
        // A missing position reads row 0 as its placeholder: its value is never
        // looked at, because the bitmap says NULL. An empty column has no row 0,
        // and then every position is missing.
        if self.is_empty() {
            return Column::nulls(self.data_type, positions.len());
        }
        Column {
            data_type: self.data_type,
            validity: (validity.count_cleared() > 0).then_some(validity),
            values: self
                .values
                .gather(positions.iter().map(|at| at.unwrap_or(0) as usize)),
        }
    }

    /// `len` rows from `start` on.
    pub fn slice(&self, start: usize, len: usize) -> Column {
        let validity = self.validity.as_ref().map(|validity| {
            let mut out = Bitmap::with_capacity(len);
            for at in start..start + len {
                out.push(validity.get(at));
            }
            out
        });
        Column {
            data_type: self.data_type,
            validity,
            values: self.values.gather(start..start + len),
        }
    }
}

/// Builds a [`Column`] one cell at a time.
pub struct ColumnBuilder {
    data_type: DataType,
    validity: Bitmap,
    values: Values,
}

impl ColumnBuilder {
    /// A builder for a column of `data_type`, sized for `capacity` rows.
    pub fn new(data_type: DataType, capacity: usize) -> Self {
        Self {
            data_type,
            validity: Bitmap::with_capacity(capacity),
            values: Values::with_capacity(data_type, capacity),
        }
    }

    /// Appends a NULL.
    pub fn push_null(&mut self) {
        self.validity.push(false);
        self.values.push_zero();
    }

    /// Appends a value, which has to be of the column's type or NULL.
    ///
    /// A mismatch is an error, not a panic: ingest is the path untrusted input
    /// takes. The message reads after the column's name ("expects …").
    pub fn push(&mut self, value: &Value) -> Result<(), String> {
        match (&mut self.values, value) {
            (_, Value::Null) => {
                self.push_null();
                return Ok(());
            }
            (Values::Bool(values), Value::Bool(value)) => values.push(*value),
            (Values::Int64(values), Value::Int64(value)) => values.push(*value),
            (Values::Float64(values), Value::Float64(value)) => values.push(*value),
            (Values::Decimal(values), Value::Decimal(value)) => values.push(value.value()),
            (Values::Utf8 { offsets, text }, Value::Utf8(value)) => {
                text.push_str(value);
                let end = u32::try_from(text.len()).map_err(|_| {
                    "holds more than 4 GiB of text, more than a column can".to_owned()
                })?;
                offsets.push(end);
            }
            (Values::Date(values), Value::Date(value)) => values.push(value.days_since_epoch()),
            (Values::Timestamp(values), Value::Timestamp(value)) => values.push(value.micros()),
            (_, other) => {
                return Err(format!("expects {}, found {other:?}", self.data_type));
            }
        }
        self.validity.push(true);
        Ok(())
    }

    /// The column.
    pub fn finish(self) -> Column {
        let validity = (self.validity.count_cleared() > 0).then_some(self.validity);
        Column {
            data_type: self.data_type,
            validity,
            values: self.values,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf8(values: &[Option<&str>]) -> Column {
        let mut builder = ColumnBuilder::new(DataType::Utf8, values.len());
        for value in values {
            match value {
                Some(value) => builder.push(&Value::Utf8((*value).to_owned())).unwrap(),
                None => builder.push_null(),
            }
        }
        builder.finish()
    }

    fn read(column: &Column) -> Vec<Value> {
        (0..column.len()).map(|row| column.value(row)).collect()
    }

    #[test]
    fn take_keeps_values_and_nulls_in_the_given_order() {
        let column = utf8(&[Some("a"), None, Some(""), Some("ü")]);
        let taken = column.take(&[3, 1, 2, 3, 0]);
        assert_eq!(
            read(&taken),
            vec![
                Value::Utf8("ü".into()),
                Value::Null,
                Value::Utf8(String::new()),
                Value::Utf8("ü".into()),
                Value::Utf8("a".into()),
            ]
        );
        assert_eq!(taken.null_count(), 1);
    }

    #[test]
    fn a_missing_position_is_null_also_over_an_empty_column() {
        let column = utf8(&[Some("x"), None]);
        assert_eq!(
            read(&column.take_optional(&[None, Some(0), Some(1)])),
            vec![Value::Null, Value::Utf8("x".into()), Value::Null]
        );
        let empty = utf8(&[]);
        assert_eq!(
            read(&empty.take_optional(&[None, None])),
            vec![Value::Null, Value::Null]
        );
    }

    #[test]
    fn slice_rebases_the_string_offsets() {
        let column = utf8(&[Some("ab"), Some("cde"), None, Some("f")]);
        assert_eq!(
            read(&column.slice(1, 3)),
            vec![
                Value::Utf8("cde".into()),
                Value::Null,
                Value::Utf8("f".into())
            ]
        );
    }

    #[test]
    fn a_value_of_another_type_is_refused() {
        let mut builder = ColumnBuilder::new(DataType::Int64, 1);
        assert_eq!(
            builder.push(&Value::Utf8("1".into())).unwrap_err(),
            "expects int64, found Utf8(\"1\")"
        );
    }

    #[test]
    fn a_column_without_nulls_carries_no_bitmap() {
        let mut builder = ColumnBuilder::new(DataType::Int64, 2);
        builder.push(&Value::Int64(1)).unwrap();
        builder.push(&Value::Int64(2)).unwrap();
        let column = builder.finish();
        assert!(column.validity.is_none());
        assert_eq!(column.null_count(), 0);
    }
}
