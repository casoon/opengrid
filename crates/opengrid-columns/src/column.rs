//! One typed column: a value buffer and, when it holds NULLs, a validity bitmap.

use opengrid_types::{DataType, Date, Decimal, Timestamp, Value};

use crate::{Bitmap, Decimals, Ints, StrBuilder, Strs};

/// The values of a column, one buffer per type of the query model.
///
/// A NULL cell holds the type's zero value (`0`, `false`, `""`) here; whether
/// the cell *is* NULL is the column's validity bitmap's business, never the
/// buffer's.
#[derive(Clone, Debug, PartialEq)]
pub enum Values {
    Bool(Vec<bool>),
    /// In the narrowest width that holds every value (issue #447).
    Int64(Ints),
    /// NaN is a value, not a NULL (rule S7).
    Float64(Vec<f64>),
    /// The coefficient; the scale comes from the column's type. Eight bytes a value up to 18 digits
    /// of precision, sixteen above (issue #450).
    Decimal(Decimals),
    /// The texts: each distinct text once and a code a row where there are few, all of them in one
    /// buffer where there are many (issue #451).
    Utf8(Strs),
    /// Days since 1970-01-01.
    Date(Vec<i32>),
    /// Microseconds since 1970-01-01T00:00:00Z (rule S9).
    Timestamp(Vec<i64>),
    /// Lists of integers (issue #185): all elements of the column in one
    /// buffer; list `i` is `items[offsets[i]..offsets[i + 1]]`.
    ListInt64 {
        offsets: Vec<u32>,
        items: Vec<i64>,
    },
    /// Lists of strings: list `i` is the elements `offsets[i]..offsets[i + 1]`,
    /// and element `k` is `text[item_offsets[k]..item_offsets[k + 1]]`.
    ListUtf8 {
        offsets: Vec<u32>,
        item_offsets: Vec<u32>,
        text: String,
    },
}

impl Values {
    fn with_capacity(data_type: DataType, capacity: usize) -> Self {
        match data_type {
            DataType::Bool => Values::Bool(Vec::with_capacity(capacity)),
            DataType::Int64 => Values::Int64(Ints::with_capacity(capacity)),
            DataType::Float64 => Values::Float64(Vec::with_capacity(capacity)),
            DataType::Decimal { precision, .. } => {
                Values::Decimal(Decimals::with_capacity(precision, capacity))
            }
            // A text column's texts are the builder's until it is finished.
            DataType::Utf8 => Values::Utf8(Strs::from_flat(vec![0], String::new())),
            DataType::Date => Values::Date(Vec::with_capacity(capacity)),
            DataType::Timestamp => Values::Timestamp(Vec::with_capacity(capacity)),
            DataType::ListInt64 => {
                let mut offsets = Vec::with_capacity(capacity + 1);
                offsets.push(0);
                Values::ListInt64 {
                    offsets,
                    items: Vec::new(),
                }
            }
            DataType::ListUtf8 => {
                let mut offsets = Vec::with_capacity(capacity + 1);
                offsets.push(0);
                Values::ListUtf8 {
                    offsets,
                    item_offsets: vec![0],
                    text: String::new(),
                }
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            Values::Bool(values) => values.len(),
            Values::Int64(values) => values.len(),
            Values::Timestamp(values) => values.len(),
            Values::Float64(values) => values.len(),
            Values::Decimal(values) => values.len(),
            Values::Utf8(strs) => strs.len(),
            Values::Date(values) => values.len(),
            Values::ListInt64 { offsets, .. } | Values::ListUtf8 { offsets, .. } => {
                offsets.len() - 1
            }
        }
    }

    /// Appends the zero value of the type — the placeholder under a NULL.
    fn push_zero(&mut self) {
        match self {
            Values::Bool(values) => values.push(false),
            Values::Int64(values) => values.push(0),
            Values::Timestamp(values) => values.push(0),
            Values::Float64(values) => values.push(0.0),
            Values::Decimal(values) => values.push_zero(),
            Values::Utf8(_) => unreachable!("the NULLs of a text column go through its builder"),
            Values::Date(values) => values.push(0),
            // The empty list: the same end as the one before.
            Values::ListInt64 { offsets, .. } | Values::ListUtf8 { offsets, .. } => {
                offsets.push(offsets.last().copied().unwrap_or(0));
            }
        }
    }

    /// The values at `positions`, in that order.
    fn gather(&self, positions: impl Iterator<Item = usize> + Clone) -> Values {
        match self {
            Values::Bool(values) => Values::Bool(positions.map(|at| values[at]).collect()),
            Values::Int64(values) => Values::Int64(positions.map(|at| values.get(at)).collect()),
            Values::Float64(values) => Values::Float64(positions.map(|at| values[at]).collect()),
            Values::Decimal(values) => Values::Decimal(values.gather(positions)),
            Values::Utf8(strs) => Values::Utf8(strs.gather(positions)),
            Values::Date(values) => Values::Date(positions.map(|at| values[at]).collect()),
            Values::Timestamp(values) => {
                Values::Timestamp(positions.map(|at| values[at]).collect())
            }
            Values::ListInt64 { offsets, items } => {
                let mut out_offsets = Vec::with_capacity(offsets.len());
                out_offsets.push(0);
                let mut out_items = Vec::new();
                for at in positions {
                    out_items
                        .extend_from_slice(&items[offsets[at] as usize..offsets[at + 1] as usize]);
                    out_offsets.push(out_items.len() as u32);
                }
                Values::ListInt64 {
                    offsets: out_offsets,
                    items: out_items,
                }
            }
            Values::ListUtf8 {
                offsets,
                item_offsets,
                text,
            } => {
                let mut out_offsets = Vec::with_capacity(offsets.len());
                out_offsets.push(0);
                let mut out_item_offsets = vec![0];
                let mut out_text = String::new();
                for at in positions {
                    for item in offsets[at] as usize..offsets[at + 1] as usize {
                        out_text.push_str(
                            &text[item_offsets[item] as usize..item_offsets[item + 1] as usize],
                        );
                        out_item_offsets.push(out_text.len() as u32);
                    }
                    out_offsets.push((out_item_offsets.len() - 1) as u32);
                }
                Values::ListUtf8 {
                    offsets: out_offsets,
                    item_offsets: out_item_offsets,
                    text: out_text,
                }
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
    /// The bytes its values and its NULL bitmap take — what holding this
    /// column costs, for the resource report (issue #70). Buffers only: the
    /// allocator's slack and the few bytes of the struct are not counted.
    pub fn byte_size(&self) -> usize {
        let values = match &self.values {
            Values::Bool(values) => values.len(),
            Values::Int64(values) => values.byte_size(),
            Values::Timestamp(values) => values.len() * 8,
            Values::Float64(values) => values.len() * 8,
            Values::Decimal(values) => values.byte_size(),
            Values::Utf8(strs) => strs.byte_size(),
            Values::Date(values) => values.len() * 4,
            Values::ListInt64 { offsets, items } => offsets.len() * 4 + items.len() * 8,
            Values::ListUtf8 {
                offsets,
                item_offsets,
                text,
            } => (offsets.len() + item_offsets.len()) * 4 + text.len(),
        };
        values + self.validity.as_ref().map_or(0, Bitmap::byte_size)
    }

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
            Values::Utf8(strs) => strs.get(row),
            _ => panic!("str() on a {} column", self.data_type),
        }
    }

    /// How many elements the list at `row` of a list column holds — none under
    /// a NULL.
    ///
    /// # Panics
    /// When the column is not a list column; the caller has checked the type.
    pub fn list_len(&self, row: usize) -> usize {
        match &self.values {
            Values::ListInt64 { offsets, .. } | Values::ListUtf8 { offsets, .. } => {
                (offsets[row + 1] - offsets[row]) as usize
            }
            _ => panic!("list_len() on a {} column", self.data_type),
        }
    }

    /// Whether the list at `row` of a list column holds `item`: a string for a
    /// `list<utf8>`, an integer for a `list<int64>`; `false` for anything else
    /// and under a NULL.
    pub fn list_has(&self, row: usize, item: &Value) -> bool {
        if self.is_null(row) {
            return false;
        }
        match (&self.values, item) {
            (Values::ListInt64 { offsets, items }, Value::Int64(wanted)) => {
                items[offsets[row] as usize..offsets[row + 1] as usize].contains(wanted)
            }
            (
                Values::ListUtf8 {
                    offsets,
                    item_offsets,
                    text,
                },
                Value::Utf8(wanted),
            ) => (offsets[row] as usize..offsets[row + 1] as usize)
                .any(|k| &text[item_offsets[k] as usize..item_offsets[k + 1] as usize] == wanted),
            _ => false,
        }
    }

    /// The cell at `row` as a [`Value`].
    pub fn value(&self, row: usize) -> Value {
        if self.is_null(row) {
            return Value::Null;
        }
        match &self.values {
            Values::Bool(values) => Value::Bool(values[row]),
            Values::Int64(values) => Value::Int64(values.get(row)),
            Values::Float64(values) => Value::Float64(values[row]),
            Values::Decimal(values) => {
                let DataType::Decimal { scale, .. } = self.data_type else {
                    unreachable!("a decimal buffer belongs to a decimal column")
                };
                Value::Decimal(Decimal::new(values.get(row), scale))
            }
            Values::Utf8(_) => Value::Utf8(self.str(row).to_owned()),
            Values::Date(values) => Value::Date(Date::from_days_since_epoch(values[row])),
            Values::Timestamp(values) => Value::Timestamp(Timestamp::from_micros(values[row])),
            Values::ListInt64 { offsets, items } => Value::List(
                items[offsets[row] as usize..offsets[row + 1] as usize]
                    .iter()
                    .map(|item| Value::Int64(*item))
                    .collect(),
            ),
            Values::ListUtf8 {
                offsets,
                item_offsets,
                text,
            } => Value::List(
                (offsets[row] as usize..offsets[row + 1] as usize)
                    .map(|k| {
                        Value::Utf8(
                            text[item_offsets[k] as usize..item_offsets[k + 1] as usize].to_owned(),
                        )
                    })
                    .collect(),
            ),
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
    /// The texts of a text column while it is built (issue #451): a dictionary until it is not
    /// worth it. `values` holds them once the column is finished.
    text: Option<StrBuilder>,
}

impl ColumnBuilder {
    /// A builder for a column of `data_type`, sized for `capacity` rows.
    pub fn new(data_type: DataType, capacity: usize) -> Self {
        Self {
            data_type,
            validity: Bitmap::with_capacity(capacity),
            values: Values::with_capacity(data_type, capacity),
            text: (data_type == DataType::Utf8).then(|| StrBuilder::with_capacity(capacity)),
        }
    }

    /// Appends a NULL.
    pub fn push_null(&mut self) {
        self.validity.push(false);
        match &mut self.text {
            // The empty text, which a dictionary holds once.
            Some(text) => text
                .push("")
                .expect("an empty text does not overflow a column"),
            None => self.values.push_zero(),
        }
    }

    /// Appends text to a text column without a [`Value`] in between — the text is copied into the
    /// column once, with no `String` made and freed for it.
    ///
    /// An error, not a panic, for any other column type or more than 4 GiB of text.
    pub fn push_str(&mut self, value: &str) -> Result<(), String> {
        let Some(text) = &mut self.text else {
            return Err(format!("expects {}, found text", self.data_type));
        };
        text.push(value)?;
        self.validity.push(true);
        Ok(())
    }

    /// Appends a `bool` to a `bool` column, without a [`Value`] in between.
    ///
    /// The typed pushes — this one and the five after it — are for ingest, which has just read the
    /// cell as that type. An error, not a panic, for a column of another type.
    #[inline]
    pub fn push_bool(&mut self, value: bool) -> Result<(), String> {
        let Values::Bool(values) = &mut self.values else {
            return Err(self.mismatch("bool"));
        };
        values.push(value);
        self.validity.push(true);
        Ok(())
    }

    /// Appends an `int64` to an `int64` column.
    #[inline]
    pub fn push_i64(&mut self, value: i64) -> Result<(), String> {
        let Values::Int64(values) = &mut self.values else {
            return Err(self.mismatch("int64"));
        };
        values.push(value);
        self.validity.push(true);
        Ok(())
    }

    /// Appends a `float64` to a `float64` column. NaN is a value, not a NULL (rule S7).
    #[inline]
    pub fn push_f64(&mut self, value: f64) -> Result<(), String> {
        let Values::Float64(values) = &mut self.values else {
            return Err(self.mismatch("float64"));
        };
        values.push(value);
        self.validity.push(true);
        Ok(())
    }

    /// Appends a decimal, given as its coefficient and its scale, to a decimal column of that
    /// scale: a coefficient of another scale would silently change its meaning.
    #[inline]
    pub fn push_decimal(&mut self, coefficient: i128, scale: u8) -> Result<(), String> {
        let (
            Values::Decimal(values),
            DataType::Decimal {
                precision,
                scale: own,
            },
        ) = (&mut self.values, self.data_type)
        else {
            return Err(self.mismatch("decimal"));
        };
        if scale != own {
            return Err(format!(
                "expects decimal({precision}, {own}), found scale {scale}"
            ));
        }
        values.push(coefficient)?;
        self.validity.push(true);
        Ok(())
    }

    /// Appends a date, as the days since 1970-01-01, to a `date` column.
    #[inline]
    pub fn push_date(&mut self, days: i32) -> Result<(), String> {
        let Values::Date(values) = &mut self.values else {
            return Err(self.mismatch("date"));
        };
        values.push(days);
        self.validity.push(true);
        Ok(())
    }

    /// Appends a timestamp, as the microseconds since 1970-01-01T00:00:00Z (rule S9), to a
    /// `timestamp` column.
    #[inline]
    pub fn push_timestamp(&mut self, micros: i64) -> Result<(), String> {
        let Values::Timestamp(values) = &mut self.values else {
            return Err(self.mismatch("timestamp"));
        };
        values.push(micros);
        self.validity.push(true);
        Ok(())
    }

    fn mismatch(&self, found: &str) -> String {
        format!("expects {}, found {found}", self.data_type)
    }

    /// Appends a value, which has to be of the column's type or NULL.
    ///
    /// A mismatch is an error, not a panic: ingest is the path untrusted input
    /// takes. The message reads after the column's name ("expects …").
    pub fn push(&mut self, value: &Value) -> Result<(), String> {
        if let (Some(text), Value::Utf8(value)) = (&mut self.text, value) {
            text.push(value)?;
            self.validity.push(true);
            return Ok(());
        }
        match (&mut self.values, value) {
            (_, Value::Null) => {
                self.push_null();
                return Ok(());
            }
            (Values::Bool(values), Value::Bool(value)) => values.push(*value),
            (Values::Int64(values), Value::Int64(value)) => values.push(*value),
            (Values::Float64(values), Value::Float64(value)) => values.push(*value),
            (Values::Decimal(values), Value::Decimal(value)) => {
                // The coefficient is stored without its scale, so a value of
                // another scale would silently change its meaning (100 at scale
                // 0 is 1.00 at scale 2).
                if let DataType::Decimal { precision, scale } = self.data_type
                    && value.scale() != scale
                {
                    return Err(format!(
                        "expects decimal({precision}, {scale}), found scale {}",
                        value.scale()
                    ));
                }
                values.push(value.value())?;
            }
            (Values::Date(values), Value::Date(value)) => values.push(value.days_since_epoch()),
            (Values::Timestamp(values), Value::Timestamp(value)) => values.push(value.micros()),
            (Values::ListInt64 { offsets, items }, Value::List(list)) => {
                let before = items.len();
                for item in list {
                    match item {
                        Value::Int64(item) => items.push(*item),
                        other => {
                            items.truncate(before);
                            return Err(format!("expects a list of int64, found {other:?}"));
                        }
                    }
                }
                let end = u32::try_from(items.len())
                    .map_err(|_| "holds more list elements than a column can".to_owned())?;
                offsets.push(end);
            }
            (
                Values::ListUtf8 {
                    offsets,
                    item_offsets,
                    text,
                },
                Value::List(list),
            ) => {
                let (items_before, text_before) = (item_offsets.len(), text.len());
                for item in list {
                    match item {
                        Value::Utf8(item) => {
                            text.push_str(item);
                            let end = u32::try_from(text.len()).map_err(|_| {
                                "holds more than 4 GiB of text, more than a column can".to_owned()
                            })?;
                            item_offsets.push(end);
                        }
                        other => {
                            item_offsets.truncate(items_before);
                            text.truncate(text_before);
                            return Err(format!("expects a list of utf8, found {other:?}"));
                        }
                    }
                }
                offsets.push((item_offsets.len() - 1) as u32);
            }
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
        let values = match self.text {
            Some(text) => Values::Utf8(text.finish()),
            None => self.values,
        };
        Column {
            data_type: self.data_type,
            validity,
            values,
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

    /// Issue #447: an integer column reads back what was pushed at every width and across the
    /// widenings, with NULLs among them, through `value`, `take`, `take_optional` and `slice`, and
    /// is as small as its values allow.
    #[test]
    fn integer_columns_read_back_at_every_width() {
        let cells = [
            Some(3),
            None,
            Some(-200),
            Some(70_000),
            Some(7),
            None,
            Some(i64::MAX),
            Some(i64::MIN),
        ];
        let mut builder = ColumnBuilder::new(DataType::Int64, cells.len());
        for cell in cells {
            match cell {
                Some(value) => builder.push_i64(value).unwrap(),
                None => builder.push_null(),
            }
        }
        let column = builder.finish();
        let value = |cell: Option<i64>| cell.map_or(Value::Null, Value::Int64);
        assert_eq!(
            read(&column),
            cells.iter().map(|cell| value(*cell)).collect::<Vec<_>>()
        );
        assert_eq!(
            read(&column.take(&[7, 0, 3, 1])),
            vec![
                value(cells[7]),
                value(cells[0]),
                value(cells[3]),
                Value::Null
            ]
        );
        assert_eq!(
            read(&column.slice(2, 4)),
            cells[2..6]
                .iter()
                .map(|cell| value(*cell))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            read(&column.take_optional(&[Some(6), None, Some(2)])),
            vec![value(cells[6]), Value::Null, value(cells[2])]
        );
        // Eight bytes each — the widest value decides, whatever the order — and a bitmap word.
        assert_eq!(column.byte_size(), 8 * 8 + 8);
        // A column of small numbers is small, and a slice of a wide one is as small as its own values.
        let small = column.slice(0, 5);
        assert_eq!(small.byte_size(), 5 * 4 + 8);
        let mut bytes = ColumnBuilder::new(DataType::Int64, 3);
        for value in [1, 2, 3] {
            bytes.push_i64(value).unwrap();
        }
        let bytes = bytes.finish();
        assert_eq!(bytes.byte_size(), 3);
        // Taken from a wide column, the same three numbers are as small as the built ones.
        let mut wide = ColumnBuilder::new(DataType::Int64, 4);
        for value in [1, 2, 3, 1 << 40] {
            wide.push_i64(value).unwrap();
        }
        assert_eq!(wide.finish().take(&[0, 1, 2]), bytes);
    }

    /// Issue #451: a text column of a few texts is a dictionary; NULL and the empty text are two
    /// things, as they are in a flat one; and every way of reading it gives the texts back.
    #[test]
    fn a_text_column_of_few_texts_reads_back_as_its_texts() {
        let cells = [
            Some("DE"),
            None,
            Some(""),
            Some("日本語"),
            Some("DE"),
            Some(""),
            None,
        ];
        let mut by_str = ColumnBuilder::new(DataType::Utf8, cells.len());
        let mut by_value = ColumnBuilder::new(DataType::Utf8, cells.len());
        for cell in cells {
            match cell {
                Some(text) => {
                    by_str.push_str(text).unwrap();
                    by_value.push(&Value::Utf8(text.to_owned())).unwrap();
                }
                None => {
                    by_str.push_null();
                    by_value.push(&Value::Null).unwrap();
                }
            }
        }
        let (column, other) = (by_str.finish(), by_value.finish());
        assert_eq!(column, other);
        let Values::Utf8(strs) = column.values() else {
            panic!("text")
        };
        assert!(strs.is_dictionary());
        let value =
            |cell: &Option<&str>| cell.map_or(Value::Null, |text| Value::Utf8(text.to_owned()));
        assert_eq!(read(&column), cells.iter().map(value).collect::<Vec<_>>());
        assert_eq!(column.null_count(), 2);
        // Under a NULL the text is the empty one, and the empty text is not a NULL.
        assert_eq!(column.str(1), "");
        assert!(column.is_null(1) && !column.is_null(2));
        // Taken, sliced, taken with gaps: flat columns of the same texts.
        let taken = column.take(&[3, 0, 1]);
        assert_eq!(
            read(&taken),
            vec![value(&cells[3]), value(&cells[0]), Value::Null]
        );
        assert_eq!(
            read(&column.slice(2, 3)),
            cells[2..5].iter().map(value).collect::<Vec<_>>()
        );
        assert_eq!(
            read(&column.take_optional(&[Some(3), None, Some(4)])),
            vec![value(&cells[3]), Value::Null, value(&cells[4])]
        );
        // A dictionary is smaller than the flat column of the same texts.
        let flat = column.slice(0, cells.len());
        assert_eq!(flat, column);
        assert!(
            column.byte_size() < flat.byte_size() + 1,
            "{} against {}",
            column.byte_size(),
            flat.byte_size()
        );
    }

    /// Issue #429: text pushed as text is what the same text pushed as a value is.
    #[test]
    fn text_pushed_as_text_is_text_pushed_as_a_value() {
        let mut direct = ColumnBuilder::new(DataType::Utf8, 3);
        direct.push_str("a").unwrap();
        direct.push_null();
        direct.push_str("ä b").unwrap();
        assert_eq!(
            read(&direct.finish()),
            read(&utf8(&[Some("a"), None, Some("ä b")]))
        );
        let mut number = ColumnBuilder::new(DataType::Int64, 1);
        assert!(number.push_str("1").is_err());
    }

    /// Issue #440: a cell pushed typed is what the same cell pushed as a value is, between NULLs,
    /// for each of the six types — including a negative zero, a NaN and the extremes.
    #[test]
    fn typed_pushes_are_pushes_of_the_value() {
        let decimal = DataType::decimal(12, 2).unwrap();
        let cases: Vec<(DataType, Vec<Value>)> = vec![
            (DataType::Bool, vec![Value::Bool(true), Value::Bool(false)]),
            (
                DataType::Int64,
                vec![
                    Value::Int64(i64::MIN),
                    Value::Int64(0),
                    Value::Int64(i64::MAX),
                ],
            ),
            (
                DataType::Float64,
                vec![
                    Value::Float64(-0.0),
                    Value::Float64(f64::NAN),
                    Value::Float64(1.5),
                ],
            ),
            (
                decimal,
                vec![
                    Value::Decimal(Decimal::new(-12345, 2)),
                    Value::Decimal(Decimal::new(0, 2)),
                ],
            ),
            (
                DataType::Date,
                vec![
                    Value::Date(Date::from_days_since_epoch(-1)),
                    Value::Date(Date::from_days_since_epoch(19_000)),
                ],
            ),
            (
                DataType::Timestamp,
                vec![
                    Value::Timestamp(Timestamp::from_micros(-5)),
                    Value::Timestamp(Timestamp::from_micros(1_700_000_000_123_456)),
                ],
            ),
        ];
        for (data_type, values) in cases {
            let (mut typed, mut generic) = (
                ColumnBuilder::new(data_type, 5),
                ColumnBuilder::new(data_type, 5),
            );
            for value in &values {
                typed.push_null();
                generic.push_null();
                match value {
                    Value::Bool(v) => typed.push_bool(*v),
                    Value::Int64(v) => typed.push_i64(*v),
                    Value::Float64(v) => typed.push_f64(*v),
                    Value::Decimal(v) => typed.push_decimal(v.value(), v.scale()),
                    Value::Date(v) => typed.push_date(v.days_since_epoch()),
                    Value::Timestamp(v) => typed.push_timestamp(v.micros()),
                    other => panic!("not a case: {other:?}"),
                }
                .unwrap();
                generic.push(value).unwrap();
            }
            let (typed, generic) = (typed.finish(), generic.finish());
            assert_eq!(typed.len(), values.len() * 2, "{data_type}");
            for row in 0..typed.len() {
                // NaN is not equal to itself: compare what a value prints as.
                assert_eq!(
                    format!("{:?}", typed.value(row)),
                    format!("{:?}", generic.value(row)),
                    "{data_type} row {row}"
                );
            }
            assert_eq!(typed.null_count(), values.len(), "{data_type}");
        }
    }

    /// Issue #440: a typed push into a column of another type, or of another scale, is refused
    /// and stores nothing.
    #[test]
    fn a_typed_push_into_the_wrong_column_is_refused() {
        let mut builder = ColumnBuilder::new(DataType::Int64, 1);
        assert_eq!(
            builder.push_bool(true).unwrap_err(),
            "expects int64, found bool"
        );
        assert_eq!(
            builder.push_f64(1.0).unwrap_err(),
            "expects int64, found float64"
        );
        assert_eq!(
            builder.push_date(1).unwrap_err(),
            "expects int64, found date"
        );
        assert_eq!(
            builder.push_timestamp(1).unwrap_err(),
            "expects int64, found timestamp"
        );
        assert_eq!(
            builder.push_decimal(1, 0).unwrap_err(),
            "expects int64, found decimal"
        );
        assert_eq!(builder.finish().len(), 0);

        let mut decimal = ColumnBuilder::new(DataType::decimal(12, 2).unwrap(), 1);
        assert_eq!(
            decimal.push_decimal(100, 0).unwrap_err(),
            "expects decimal(12, 2), found scale 0"
        );
        assert_eq!(
            decimal.push_i64(1).unwrap_err(),
            "expects decimal(12, 2), found int64"
        );
        assert_eq!(decimal.finish().len(), 0);

        let mut text = ColumnBuilder::new(DataType::Utf8, 1);
        assert_eq!(text.push_i64(1).unwrap_err(), "expects utf8, found int64");
    }

    /// Issue #213: a decimal of another scale is refused, not stored with a
    /// different meaning. Connectors build their own decimals.
    #[test]
    fn a_decimal_of_another_scale_is_refused() {
        let mut builder = ColumnBuilder::new(DataType::decimal(12, 2).unwrap(), 2);
        builder.push(&Value::Decimal(Decimal::new(100, 2))).unwrap();
        let error = builder
            .push(&Value::Decimal(Decimal::new(100, 0)))
            .unwrap_err();
        assert_eq!(error, "expects decimal(12, 2), found scale 0");

        let column = builder.finish();
        assert_eq!(column.len(), 1, "the refused value was not stored");
        assert_eq!(column.value(0), Value::Decimal(Decimal::new(100, 2)));
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
