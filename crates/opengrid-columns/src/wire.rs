//! The binary result form (decision E35): a table as bytes, and back.
//!
//! It is the serialised form of this crate's columns, so writing is little
//! more than copying buffers and reading needs no parser. One codec for both
//! sides: the server writes it, the browser reads it — and the engine in a
//! worker hands it to the page as a transferable `ArrayBuffer`.
//!
//! # Layout, version 1
//!
//! Little-endian throughout; every section starts on an 8-byte boundary, so a
//! reader in JavaScript can view a value buffer as a `BigInt64Array` or
//! `Float64Array` without copying.
//!
//! ```text
//! header   "OGC" · version u8 (1) · kind u8 (0 result, 1 pivot, 2 tree) · 3 × 0
//! table    total_count u64 · row_count u64 · column_count u32 · u32 0
//! column   name: length u32 + UTF-8, padded
//!          type u8 · precision u8 · scale u8 · nullable u8 · has_validity u8 · 3 × 0
//!          validity: ⌈rows / 64⌉ u64 words, bit i = row i holds a value (if present)
//!          values:
//!            bool       ⌈rows / 64⌉ u64 words
//!            int64      rows × i64          timestamp  rows × i64 (µs, UTC)
//!            float64    rows × f64 (NaN and ±∞ as themselves)
//!            decimal    rows × i128 (the coefficient; the scale is the type's)
//!            date       rows × i32 (days since 1970-01-01), padded
//!            utf8       (rows + 1) × u32 offsets, padded · offsets[rows] bytes, padded
//! ```
//!
//! Type codes: 1 bool, 2 int64, 3 float64, 4 decimal, 5 utf8, 6 date,
//! 7 timestamp. A pivot appends its own section after the table
//! (`opengrid-pivot`), written with the same [`Writer`] primitives. A tree's
//! level (E38) appends its part:
//!
//! ```text
//! tree     matches u64 · orphans u64 · children: rows × u64 · match: ⌈rows / 64⌉ u64 words
//! [aggregates]  a table (as above, total_count 0) with one row per row of the level —
//!               the subtree aggregates of T7, present only when the query asked for them
//! ```
//!
//! A reader from before the aggregates refuses a level that carries them (the
//! bytes do not end where it expects) rather than dropping them.
//!
//! # Reading is strict
//!
//! The bytes may come from a page's own provider, and whatever reads the table
//! next indexes its buffers. So [`Reader`] checks every length against the
//! bytes it has, every name, every type, every string boundary and every
//! decimal against its precision, and answers a sentence rather than a panic.

use std::fmt;

use opengrid_types::{DataType, Date, Decimal, Field, FieldName, Schema, Timestamp, Value};

use crate::{Bitmap, Column, Table, Values};

/// The media type of the binary form, for `Accept` and `Content-Type`.
pub const MEDIA_TYPE: &str = "application/vnd.opengrid.columns";

const MAGIC: &[u8; 3] = b"OGC";

/// The layout version this crate writes and reads.
pub const VERSION: u8 = 1;

/// What the bytes hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A query result.
    Result,
    /// A pivot answer: a result plus the pivot's own section.
    Pivot,
    /// One level of a tree (E38): a result plus what each row is.
    Tree,
}

impl Kind {
    fn code(self) -> u8 {
        match self {
            Kind::Result => 0,
            Kind::Pivot => 1,
            Kind::Tree => 2,
        }
    }
}

/// Bytes that are not the binary form, or not one this reader can use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireError(String);

impl WireError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// What was wrong.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "binary result: {}", self.0)
    }
}

impl std::error::Error for WireError {}

/// A result as bytes.
pub fn encode_result(table: &Table, total_count: u64) -> Vec<u8> {
    let mut writer = Writer::new(Kind::Result);
    writer.table(table, total_count);
    writer.finish()
}

/// Bytes back to a result: the table and its `total_count`.
///
/// A tree's level is refused, not read as a plain list: what its rows are
/// would be lost. [`decode_answer`] reads both.
pub fn decode_result(bytes: &[u8]) -> Result<(Table, u64), WireError> {
    let (mut reader, kind) = Reader::new(bytes)?;
    match kind {
        Kind::Result => {}
        Kind::Pivot => return Err(WireError::new("this is a pivot answer, not a result")),
        Kind::Tree => {
            return Err(WireError::new(
                "this is a tree's level; read it with what reads its tree part",
            ));
        }
    }
    let result = reader.table()?;
    reader.finish()?;
    Ok(result)
}

/// What a tree's level carries beside its rows (E38) — the binary twin of
/// `opengrid_datasource::TreeLevel`, which this crate does not depend on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeSection {
    /// Per row: its visible children.
    pub children: Vec<u64>,
    /// Per row: a match, or context.
    pub matched: Vec<bool>,
    /// Matches in the whole tree.
    pub matches: u64,
    /// Nodes without an existing parent.
    pub orphans: u64,
    /// The subtree aggregates (T7, issue #165): one row per row of the level,
    /// one column per alias. `None` when the query asked for none.
    pub aggregates: Option<Table>,
}

/// A tree's level as bytes: the result, then its tree part.
pub fn encode_tree_result(table: &Table, total_count: u64, tree: &TreeSection) -> Vec<u8> {
    let mut writer = Writer::new(Kind::Tree);
    writer.table(table, total_count);
    writer.u64(tree.matches);
    writer.u64(tree.orphans);
    for count in &tree.children {
        writer.u64(*count);
    }
    let mut bits = Bitmap::with_capacity(tree.matched.len());
    for matched in &tree.matched {
        bits.push(*matched);
    }
    writer.words(bits.words());
    if let Some(aggregates) = &tree.aggregates {
        writer.table(aggregates, 0);
    }
    writer.finish()
}

/// Bytes back to a result — a plain one, or a tree's level with its part.
pub fn decode_answer(bytes: &[u8]) -> Result<(Table, u64, Option<TreeSection>), WireError> {
    let (mut reader, kind) = Reader::new(bytes)?;
    let (table, total_count) = match kind {
        Kind::Result | Kind::Tree => reader.table()?,
        Kind::Pivot => return Err(WireError::new("this is a pivot answer, not a result")),
    };
    let tree = if kind == Kind::Tree {
        let rows = table.num_rows();
        let matches = reader.u64()?;
        let orphans = reader.u64()?;
        let children = (0..rows)
            .map(|_| reader.u64())
            .collect::<Result<Vec<_>, _>>()?;
        let words = (0..rows.div_ceil(64))
            .map(|_| reader.u64())
            .collect::<Result<Vec<_>, _>>()?;
        let matched = (0..rows)
            .map(|row| words[row / 64] >> (row % 64) & 1 == 1)
            .collect();
        let aggregates = if reader.at_end() {
            None
        } else {
            let (aggregates, _) = reader.table()?;
            if aggregates.num_rows() != rows {
                return Err(WireError::new(
                    "the tree's aggregates are not one row per row of the level",
                ));
            }
            Some(aggregates)
        };
        Some(TreeSection {
            children,
            matched,
            matches,
            orphans,
            aggregates,
        })
    } else {
        None
    };
    reader.finish()?;
    Ok((table, total_count, tree))
}

/// Writes the binary form, section by section.
pub struct Writer {
    out: Vec<u8>,
}

impl Writer {
    /// A writer whose header says `kind`.
    pub fn new(kind: Kind) -> Self {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(kind.code());
        out.extend_from_slice(&[0; 3]);
        Self { out }
    }

    fn pad(&mut self) {
        while !self.out.len().is_multiple_of(8) {
            self.out.push(0);
        }
    }

    /// One `u64`.
    pub fn u64(&mut self, value: u64) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    fn u32_pair(&mut self, value: u32) {
        self.out.extend_from_slice(&value.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]);
    }

    /// A string: its length, its UTF-8, padded.
    pub fn string(&mut self, text: &str) {
        let length = u32::try_from(text.len()).expect("a name or text shorter than 4 GiB");
        self.out.extend_from_slice(&length.to_le_bytes());
        self.out.extend_from_slice(text.as_bytes());
        self.pad();
    }

    /// Small counts, e.g. a pivot's row levels, padded.
    pub fn u16s(&mut self, values: &[u16]) {
        for value in values {
            self.out.extend_from_slice(&value.to_le_bytes());
        }
        self.pad();
    }

    /// A table and the count of rows its query matched.
    pub fn table(&mut self, table: &Table, total_count: u64) {
        let rows = table.num_rows();
        self.u64(total_count);
        self.u64(rows as u64);
        self.u32_pair(u32::try_from(table.schema().len()).expect("fewer than 2^32 columns"));
        for (index, field) in table.schema().fields().iter().enumerate() {
            let column = table.column_at(index);
            self.string(field.name.as_str());
            self.type_header(field.data_type, field.nullable, column.validity.is_some());
            if let Some(validity) = &column.validity {
                self.words(validity.words());
            }
            self.values(&column.values);
        }
    }

    fn type_header(&mut self, data_type: DataType, nullable: bool, has_validity: bool) {
        let (code, precision, scale) = type_code(data_type);
        self.out.extend_from_slice(&[
            code,
            precision,
            scale,
            u8::from(nullable),
            u8::from(has_validity),
            0,
            0,
            0,
        ]);
    }

    fn words(&mut self, words: &[u64]) {
        for word in words {
            self.u64(*word);
        }
    }

    fn values(&mut self, values: &Values) {
        match values {
            Values::Bool(values) => {
                let mut bits = Bitmap::with_capacity(values.len());
                for value in values {
                    bits.push(*value);
                }
                self.words(bits.words());
            }
            Values::Int64(values) | Values::Timestamp(values) => {
                for value in values {
                    self.out.extend_from_slice(&value.to_le_bytes());
                }
            }
            Values::Float64(values) => {
                for value in values {
                    self.out.extend_from_slice(&value.to_le_bytes());
                }
            }
            Values::Decimal(values) => {
                for value in values {
                    self.out.extend_from_slice(&value.to_le_bytes());
                }
            }
            Values::Date(values) => {
                for value in values {
                    self.out.extend_from_slice(&value.to_le_bytes());
                }
                self.pad();
            }
            Values::Utf8 { offsets, text } => {
                for offset in offsets {
                    self.out.extend_from_slice(&offset.to_le_bytes());
                }
                self.pad();
                self.out.extend_from_slice(text.as_bytes());
                self.pad();
            }
        }
    }

    /// One typed scalar — a pivot's column path is made of these.
    pub fn value(&mut self, value: &Value) {
        let (code, precision, scale) = match value {
            Value::Null => (0, 0, 0),
            Value::Bool(_) => (1, 0, 0),
            Value::Int64(_) => (2, 0, 0),
            Value::Float64(_) => (3, 0, 0),
            Value::Decimal(decimal) => (4, 0, decimal.scale()),
            Value::Utf8(_) => (5, 0, 0),
            Value::Date(_) => (6, 0, 0),
            Value::Timestamp(_) => (7, 0, 0),
        };
        self.out
            .extend_from_slice(&[code, precision, scale, 0, 0, 0, 0, 0]);
        match value {
            Value::Null => {}
            Value::Bool(value) => self.u64(u64::from(*value)),
            Value::Int64(value) => self.out.extend_from_slice(&value.to_le_bytes()),
            Value::Float64(value) => self.out.extend_from_slice(&value.to_le_bytes()),
            Value::Decimal(value) => self.out.extend_from_slice(&value.value().to_le_bytes()),
            Value::Utf8(text) => self.string(text),
            Value::Date(date) => self
                .out
                .extend_from_slice(&i64::from(date.days_since_epoch()).to_le_bytes()),
            Value::Timestamp(value) => self.out.extend_from_slice(&value.micros().to_le_bytes()),
        }
    }

    /// The bytes.
    pub fn finish(self) -> Vec<u8> {
        self.out
    }
}

fn type_code(data_type: DataType) -> (u8, u8, u8) {
    match data_type {
        DataType::Bool => (1, 0, 0),
        DataType::Int64 => (2, 0, 0),
        DataType::Float64 => (3, 0, 0),
        DataType::Decimal { precision, scale } => (4, precision, scale),
        DataType::Utf8 => (5, 0, 0),
        DataType::Date => (6, 0, 0),
        DataType::Timestamp => (7, 0, 0),
    }
}

fn data_type(code: u8, precision: u8, scale: u8) -> Result<DataType, WireError> {
    Ok(match code {
        1 => DataType::Bool,
        2 => DataType::Int64,
        3 => DataType::Float64,
        4 => DataType::decimal(precision, scale)
            .map_err(|_| WireError::new(format!("decimal({precision}, {scale}) is not a type")))?,
        5 => DataType::Utf8,
        6 => DataType::Date,
        7 => DataType::Timestamp,
        other => return Err(WireError::new(format!("unknown type code {other}"))),
    })
}

/// Reads the binary form, section by section, checking as it goes.
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    /// A reader past the header, and what the header says the bytes hold.
    pub fn new(bytes: &'a [u8]) -> Result<(Self, Kind), WireError> {
        if bytes.len() < 8 || &bytes[..3] != MAGIC {
            return Err(WireError::new("not the binary form (no OGC header)"));
        }
        if bytes[3] != VERSION {
            return Err(WireError::new(format!(
                "version {} is not one this reader knows (it reads {VERSION})",
                bytes[3]
            )));
        }
        let kind = match bytes[4] {
            0 => Kind::Result,
            1 => Kind::Pivot,
            2 => Kind::Tree,
            other => return Err(WireError::new(format!("unknown kind {other}"))),
        };
        Ok((Self { bytes, at: 8 }, kind))
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .at
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| WireError::new("the bytes end too early"))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn pad(&mut self) -> Result<(), WireError> {
        let padding = (8 - self.at % 8) % 8;
        self.take(padding).map(|_| ())
    }

    fn times(count: usize, size: usize) -> Result<usize, WireError> {
        count
            .checked_mul(size)
            .ok_or_else(|| WireError::new("a length past what this machine can address"))
    }

    /// One `u64`.
    pub fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("eight bytes"),
        ))
    }

    fn u32_pair(&mut self) -> Result<u32, WireError> {
        let pair = self.take(8)?;
        Ok(u32::from_le_bytes(
            pair[..4].try_into().expect("four bytes"),
        ))
    }

    fn count(&mut self) -> Result<usize, WireError> {
        usize::try_from(self.u64()?)
            .map_err(|_| WireError::new("a count past what this machine can address"))
    }

    /// A string written by [`Writer::string`].
    pub fn string(&mut self) -> Result<&'a str, WireError> {
        let length = u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")) as usize;
        let bytes = self.take(length)?;
        self.pad()?;
        std::str::from_utf8(bytes).map_err(|_| WireError::new("a text is not UTF-8"))
    }

    /// `count` small numbers written by [`Writer::u16s`].
    pub fn u16s(&mut self, count: usize) -> Result<Vec<u16>, WireError> {
        let bytes = self.take(Self::times(count, 2)?)?;
        self.pad()?;
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect())
    }

    /// A table and its `total_count`.
    pub fn table(&mut self) -> Result<(Table, u64), WireError> {
        let total_count = self.u64()?;
        let rows = self.count()?;
        let column_count = self.u32_pair()? as usize;
        // Every column has at least its name and type header: a count the bytes
        // cannot hold is refused before anything is allocated for it.
        if Self::times(column_count, 16)? > self.bytes.len() - self.at {
            return Err(WireError::new("the bytes end too early"));
        }
        let mut fields = Vec::with_capacity(column_count);
        let mut columns = Vec::with_capacity(column_count);
        for _ in 0..column_count {
            let name = self.string()?;
            let name = FieldName::new(name)
                .map_err(|error| WireError::new(format!("column {name:?}: {error}")))?;
            let header = self.take(8)?;
            let data_type = data_type(header[0], header[1], header[2])
                .map_err(|error| WireError::new(format!("column {name}: {}", error.message())))?;
            let nullable = header[3] != 0;
            let validity = if header[4] != 0 {
                let words = self.words(rows.div_ceil(64))?;
                Some(Bitmap::from_words(words, rows).ok_or_else(|| {
                    WireError::new(format!("column {name}: the NULL bitmap does not fit"))
                })?)
            } else {
                None
            };
            let values = self
                .values(data_type, rows)
                .map_err(|error| WireError::new(format!("column {name}: {}", error.message())))?;
            fields.push(Field {
                name,
                data_type,
                nullable,
                from: None,
            });
            columns.push(Column {
                data_type,
                validity,
                values,
            });
        }
        let table = Table::new(&Schema::new(fields), columns).map_err(WireError::new)?;
        Ok((table, total_count))
    }

    fn words(&mut self, count: usize) -> Result<Vec<u64>, WireError> {
        Ok(self
            .take(Self::times(count, 8)?)?
            .as_chunks::<8>()
            .0
            .iter()
            .map(|word| u64::from_le_bytes(*word))
            .collect())
    }

    fn values(&mut self, data_type: DataType, rows: usize) -> Result<Values, WireError> {
        Ok(match data_type {
            DataType::Bool => {
                let words = self.words(rows.div_ceil(64))?;
                let bits = Bitmap::from_words(words, rows)
                    .ok_or_else(|| WireError::new("the booleans do not fit"))?;
                Values::Bool((0..rows).map(|row| bits.get(row)).collect())
            }
            DataType::Int64 => Values::Int64(self.fixed(rows, i64::from_le_bytes)?),
            DataType::Timestamp => Values::Timestamp(self.fixed(rows, i64::from_le_bytes)?),
            DataType::Float64 => Values::Float64(self.fixed(rows, f64::from_le_bytes)?),
            DataType::Decimal { precision, .. } => {
                let values = self.fixed(rows, i128::from_le_bytes)?;
                let limit = 10i128.pow(u32::from(precision));
                if values
                    .iter()
                    .any(|value| value.unsigned_abs() >= limit.unsigned_abs())
                {
                    return Err(WireError::new(format!(
                        "a value has more than {precision} digits"
                    )));
                }
                Values::Decimal(values)
            }
            DataType::Date => {
                let values = self.fixed(rows, i32::from_le_bytes)?;
                self.pad()?;
                Values::Date(values)
            }
            DataType::Utf8 => {
                let offsets = self.fixed(Self::times(rows, 1)? + 1, u32::from_le_bytes)?;
                self.pad()?;
                let length = *offsets.last().expect("rows + 1 offsets") as usize;
                let text = std::str::from_utf8(self.take(length)?)
                    .map_err(|_| WireError::new("a text is not UTF-8"))?;
                self.pad()?;
                if offsets[0] != 0
                    || offsets.windows(2).any(|pair| pair[0] > pair[1])
                    || offsets
                        .iter()
                        .any(|offset| !text.is_char_boundary(*offset as usize))
                {
                    return Err(WireError::new("the text offsets do not fit the text"));
                }
                Values::Utf8 {
                    offsets,
                    text: text.to_owned(),
                }
            }
        })
    }

    fn fixed<T, const N: usize>(
        &mut self,
        count: usize,
        read: fn([u8; N]) -> T,
    ) -> Result<Vec<T>, WireError> {
        Ok(self
            .take(Self::times(count, N)?)?
            .as_chunks::<N>()
            .0
            .iter()
            .map(|chunk| read(*chunk))
            .collect())
    }

    /// One typed scalar written by [`Writer::value`].
    pub fn value(&mut self) -> Result<Value, WireError> {
        let header = self.take(8)?;
        let (code, scale) = (header[0], header[2]);
        Ok(match code {
            0 => Value::Null,
            1 => Value::Bool(self.u64()? != 0),
            2 => Value::Int64(self.u64()? as i64),
            3 => Value::Float64(f64::from_bits(self.u64()?)),
            4 => {
                let coefficient =
                    i128::from_le_bytes(self.take(16)?.try_into().expect("sixteen bytes"));
                Value::Decimal(Decimal::new(coefficient, scale))
            }
            5 => Value::Utf8(self.string()?.to_owned()),
            6 => {
                let days = i32::try_from(self.u64()? as i64)
                    .map_err(|_| WireError::new("a date past the calendar"))?;
                Value::Date(Date::from_days_since_epoch(days))
            }
            7 => Value::Timestamp(Timestamp::from_micros(self.u64()? as i64)),
            other => return Err(WireError::new(format!("unknown value code {other}"))),
        })
    }

    /// Checks that nothing is left over.
    /// Whether every byte has been read — an optional section follows if not.
    pub fn at_end(&self) -> bool {
        self.at == self.bytes.len()
    }

    pub fn finish(self) -> Result<(), WireError> {
        if self.at != self.bytes.len() {
            return Err(WireError::new(format!(
                "{} bytes left over after the last section",
                self.bytes.len() - self.at
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    /// A tree's level (E38, #135): its part comes back as it went, and the
    /// plain reader refuses it rather than lose that part.
    #[test]
    fn a_tree_level_round_trips_with_its_part() {
        let schema = Schema::new(vec![Field::required(
            FieldName::new("id").unwrap(),
            DataType::Int64,
        )]);
        let table =
            Table::from_values(&schema, &[(0..70).map(Value::Int64).collect::<Vec<_>>()]).unwrap();
        let tree = TreeSection {
            children: (0..70).map(|n| n % 3).collect(),
            matched: (0..70).map(|n| n % 5 != 0).collect(),
            matches: 56,
            orphans: 2,
            aggregates: None,
        };
        let bytes = encode_tree_result(&table, 71, &tree);
        let (back, total, part) = decode_answer(&bytes).expect("reads");
        assert_eq!(back.num_rows(), 70);
        assert_eq!(total, 71);
        assert_eq!(part, Some(tree));
        assert!(
            decode_result(&bytes).is_err(),
            "a plain reader would lose the part"
        );
        let (_, _, none) = decode_answer(&encode_result(&table, 70)).unwrap();
        assert_eq!(none, None);
    }

    /// The subtree aggregates (T7, #165) come back with the level, and a
    /// reader that knows only the tree part refuses them rather than lose them.
    #[test]
    fn a_tree_level_carries_its_aggregates() {
        let schema = Schema::new(vec![Field::required(
            FieldName::new("id").unwrap(),
            DataType::Int64,
        )]);
        let table = Table::from_values(&schema, &[vec![Value::Int64(1), Value::Int64(2)]]).unwrap();
        let sums = Schema::new(vec![Field::new(
            FieldName::new("revenue_sum").unwrap(),
            DataType::Int64,
        )]);
        let aggregates =
            Table::from_values(&sums, &[vec![Value::Int64(203), Value::Null]]).unwrap();
        let tree = TreeSection {
            children: vec![2, 0],
            matched: vec![true, true],
            matches: 2,
            orphans: 0,
            aggregates: Some(aggregates),
        };
        let bytes = encode_tree_result(&table, 2, &tree);
        let (_, _, part) = decode_answer(&bytes).expect("reads");
        assert_eq!(part, Some(tree));

        // What a reader from before T7 does: the table, the tree part, the end.
        let (mut reader, _) = Reader::new(&bytes).unwrap();
        reader.table().unwrap();
        for _ in 0..(2 + 2 + 1) {
            reader.u64().unwrap();
        }
        assert!(
            reader.finish().is_err(),
            "an older reader refuses the aggregates"
        );

        // Aggregates that do not match the level's rows are refused.
        let mut short = TreeSection {
            children: vec![2, 0],
            matched: vec![true, true],
            matches: 2,
            orphans: 0,
            aggregates: Some(Table::from_values(&sums, &[vec![Value::Int64(1)]]).unwrap()),
        };
        assert!(decode_answer(&encode_tree_result(&table, 2, &short)).is_err());
        short.aggregates = None;
        assert!(decode_answer(&encode_tree_result(&table, 2, &short)).is_ok());
    }

    use proptest::prelude::*;

    use super::*;
    use crate::ColumnBuilder;

    fn field(name: &str, data_type: DataType, nullable: bool) -> Field {
        let name = FieldName::new(name).unwrap();
        if nullable {
            Field::new(name, data_type)
        } else {
            Field::required(name, data_type)
        }
    }

    fn column(data_type: DataType, values: &[Value]) -> Column {
        let mut builder = ColumnBuilder::new(data_type, values.len());
        for value in values {
            builder.push(value).unwrap();
        }
        builder.finish()
    }

    /// Every type, NULLs, NaN and -0.0, the empty string and a multi-byte one,
    /// decimal edges: all of it comes back as it went.
    fn sample() -> Table {
        let decimal = DataType::decimal(38, 3).unwrap();
        let schema = Schema::new(vec![
            field("id", DataType::Int64, false),
            field("flag", DataType::Bool, true),
            field("ratio", DataType::Float64, true),
            field("amount", decimal, true),
            field("note", DataType::Utf8, true),
            field("day", DataType::Date, true),
            field("at", DataType::Timestamp, true),
        ]);
        let wide = 10i128.pow(37) + 7;
        Table::new(
            &schema,
            vec![
                column(
                    DataType::Int64,
                    &[
                        Value::Int64(i64::MIN),
                        Value::Int64(1),
                        Value::Int64(i64::MAX),
                    ],
                ),
                column(
                    DataType::Bool,
                    &[Value::Bool(true), Value::Null, Value::Bool(false)],
                ),
                column(
                    DataType::Float64,
                    &[
                        Value::Float64(f64::NAN),
                        Value::Float64(-0.0),
                        Value::Float64(f64::NEG_INFINITY),
                    ],
                ),
                column(
                    decimal,
                    &[
                        Value::Decimal(Decimal::new(-wide, 3)),
                        Value::Null,
                        Value::Decimal(Decimal::new(wide, 3)),
                    ],
                ),
                column(
                    DataType::Utf8,
                    &[
                        Value::Utf8(String::new()),
                        Value::Utf8("ä\0é".into()),
                        Value::Null,
                    ],
                ),
                column(
                    DataType::Date,
                    &[
                        Value::Null,
                        Value::Date(Date::from_days_since_epoch(-1)),
                        Value::Date(Date::from_days_since_epoch(20_000)),
                    ],
                ),
                column(
                    DataType::Timestamp,
                    &[
                        Value::Timestamp(Timestamp::from_micros(-1)),
                        Value::Null,
                        Value::Timestamp(Timestamp::from_micros(1)),
                    ],
                ),
            ],
        )
        .unwrap()
    }

    fn same(a: &Table, b: &Table) {
        assert_eq!(a.schema(), b.schema());
        let (a, b) = (a.to_values(), b.to_values());
        for (column, (a, b)) in a.iter().zip(&b).enumerate() {
            for (row, (a, b)) in a.iter().zip(b).enumerate() {
                let equal = match (a, b) {
                    (Value::Float64(a), Value::Float64(b)) => a.to_bits() == b.to_bits(),
                    _ => a == b,
                };
                assert!(equal, "column {column}, row {row}: {a:?} != {b:?}");
            }
        }
    }

    #[test]
    fn a_table_comes_back_as_it_went() {
        let table = sample();
        let bytes = encode_result(&table, 41);
        let (back, total) = decode_result(&bytes).unwrap();
        assert_eq!(total, 41);
        same(&table, &back);
        assert_eq!(bytes.len() % 8, 0, "sections end on the boundary");
    }

    #[test]
    fn an_empty_table_keeps_its_columns() {
        let table = sample().slice(0, 0);
        let (back, total) = decode_result(&encode_result(&table, 0)).unwrap();
        assert_eq!((back.num_rows(), total), (0, 0));
        assert_eq!(back.schema(), table.schema());
    }

    /// Truncated anywhere, the bytes are an error — never a panic.
    #[test]
    fn every_truncation_is_an_error() {
        let bytes = encode_result(&sample(), 3);
        for end in 0..bytes.len() {
            assert!(decode_result(&bytes[..end]).is_err(), "cut at {end}");
        }
    }

    #[test]
    fn a_wrong_header_is_named() {
        let mut bytes = encode_result(&sample(), 3);
        bytes[3] = 2;
        assert!(
            decode_result(&bytes)
                .unwrap_err()
                .message()
                .starts_with("version 2")
        );
        assert!(decode_result(b"{\"total_count\":0}").is_err());
        let mut trailing = encode_result(&sample(), 3);
        trailing.extend_from_slice(&[0; 8]);
        assert!(
            decode_result(&trailing)
                .unwrap_err()
                .message()
                .contains("left over")
        );
    }

    /// A string offset inside a multi-byte character would make every later
    /// read of the column panic: it is refused while reading.
    #[test]
    fn an_offset_inside_a_character_is_refused() {
        let schema = Schema::new(vec![field("note", DataType::Utf8, false)]);
        let table = Table::new(
            &schema,
            vec![column(
                DataType::Utf8,
                &[Value::Utf8("ä".into()), Value::Utf8("b".into())],
            )],
        )
        .unwrap();
        let mut bytes = encode_result(&table, 2);
        // header 8 + table 24 + name (4 + 4, padded to 8) + type 8 → offsets
        // [0, 2, 3] at 48. The text "äb" stays valid UTF-8; only the middle
        // offset moves into the "ä".
        assert_eq!(&bytes[52..56], &2u32.to_le_bytes());
        bytes[52..56].copy_from_slice(&1u32.to_le_bytes());
        assert!(
            decode_result(&bytes)
                .unwrap_err()
                .message()
                .contains("offsets do not fit")
        );
    }

    #[test]
    fn a_decimal_past_its_precision_is_refused() {
        let decimal = DataType::decimal(2, 0).unwrap();
        let schema = Schema::new(vec![field("n", decimal, false)]);
        let table = Table::new(
            &schema,
            vec![column(decimal, &[Value::Decimal(Decimal::new(99, 0))])],
        )
        .unwrap();
        let mut bytes = encode_result(&table, 1);
        let at = bytes.len() - 16;
        bytes[at..].copy_from_slice(&100i128.to_le_bytes());
        assert!(
            decode_result(&bytes)
                .unwrap_err()
                .message()
                .contains("more than 2 digits")
        );
    }

    #[test]
    fn scalars_come_back_as_they_went() {
        let values = [
            Value::Null,
            Value::Bool(true),
            Value::Int64(-3),
            Value::Float64(f64::INFINITY),
            Value::Decimal(Decimal::new(-1050, 2)),
            Value::Utf8("x".into()),
            Value::Date(Date::from_days_since_epoch(-5)),
            Value::Timestamp(Timestamp::from_micros(7)),
        ];
        let mut writer = Writer::new(Kind::Pivot);
        for value in &values {
            writer.value(value);
        }
        let bytes = writer.finish();
        let (mut reader, kind) = Reader::new(&bytes).unwrap();
        assert_eq!(kind, Kind::Pivot);
        for value in &values {
            assert_eq!(&reader.value().unwrap(), value);
        }
        reader.finish().unwrap();
    }

    proptest! {
        /// Random bytes after a valid header never panic the reader.
        #[test]
        fn garbage_is_an_error_not_a_panic(tail in proptest::collection::vec(any::<u8>(), 0..200)) {
            let mut bytes = b"OGC\x01\x00\x00\x00\x00".to_vec();
            bytes.extend(tail);
            let _ = decode_result(&bytes);
        }

        /// Random slices of a real table round-trip.
        #[test]
        fn slices_round_trip(start in 0usize..4, length in 0usize..4) {
            let table = sample().slice(start, length);
            let (back, _) = decode_result(&encode_result(&table, 9)).unwrap();
            same(&table, &back);
        }
    }
}
