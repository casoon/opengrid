//! Rows that arrive as columns: a table read from packs of typed columns.
//!
//! A page that holds its rows as objects used to hand them over as one JSON text, which the
//! engine read again cell by cell (issue #445). The page turns them into columns instead — numbers
//! as numbers, text as bytes — and the engine appends a pack of them to its column builders:
//! no text of the whole table, no tree, no row of values.
//!
//! # The pack
//!
//! All integers little-endian. A pack holds `rows` rows of every **stored** column of the schema,
//! in schema order (a derived column is computed, never sent):
//!
//! ```text
//! u32 rows
//! per column:  u32 flags   bit 0: a validity bitmap follows
//!              u32 text    the bytes of text of a text column, else 0
//! per column, each section starting at a multiple of 8 bytes from the start of the pack:
//!     [validity: ceil(rows / 8) bytes, bit i set = row i holds a value]   (when flagged)
//!     (padding to a multiple of 8)
//!     payload by the column's type:
//!       bool                     rows bytes, 0 or 1
//!       int64, float64           rows × f64
//!       utf8, decimal, date,     (rows + 1) × u32 offsets into the text, then the text;
//!       timestamp                a decimal, a date and a timestamp as the text the JSON
//!                                route carries: the engine's own parser reads them
//! (padding to a multiple of 8 at the end of the pack)
//! ```
//!
//! An `int64` travels as an `f64` because that is what a page has; the page sends only integers
//! it can hold exactly (below 2^53), and the engine refuses anything else.
//!
//! The table this makes is the table [`super::load_json`] makes of the same rows — the page falls
//! back to the JSON route for anything unusual, and for any error of this one, so the JSON route
//! owns every message (`a_table_of_columns_is_the_table_of_json` in `tests/columns.rs`).

use opengrid_columns::Table;
use opengrid_types::{DataType, DatePart, Schema};

use super::{Builders, IngestError, Plan, cell};

/// The most rows one pack may carry: a bound on what a page can ask the engine to allocate in
/// one piece.
const MAX_PACK_ROWS: usize = 1 << 24;

/// A table read from packs of columns: [`ColumnsLoader::feed`] for each, then
/// [`ColumnsLoader::finish`].
pub struct ColumnsLoader {
    stored: Schema,
    plan: Plan,
    builders: Builders<'static>,
    /// Per stored column: the derived columns that are a part of it, with the part.
    derived: Vec<Vec<(usize, DatePart)>>,
    /// The rows the table will have: it is sized by them, and a table of another size is refused.
    expected: usize,
    got: usize,
}

impl ColumnsLoader {
    /// A loader for `rows` rows of `schema`. A list column is not read from columns: its error
    /// sends the page to the JSON route.
    pub fn new(schema: &Schema, rows: usize) -> Result<Self, IngestError> {
        if schema
            .fields()
            .iter()
            .any(|field| matches!(field.data_type, DataType::ListUtf8 | DataType::ListInt64))
        {
            return Err(IngestError::Input {
                message: "a list column is not read from columns".to_owned(),
            });
        }
        if rows > MAX_PACK_ROWS * 64 {
            return Err(IngestError::Input {
                message: format!("{rows} rows are too many for one table"),
            });
        }
        let stored = schema.stored();
        let plan = Plan::of(schema, &stored);
        // A derived column reads the kept cell of its slot: the slots are the kept cells in order.
        let kept: Vec<usize> = (0..plan.cells.len())
            .filter(|index| plan.cells[*index].kept)
            .collect();
        let mut derived = vec![Vec::new(); plan.cells.len()];
        for part in &plan.parts {
            derived[kept[part.kept]].push((part.column, part.part));
        }
        Ok(Self {
            stored,
            plan,
            builders: Builders::with(schema.clone(), rows),
            derived,
            expected: rows,
            got: 0,
        })
    }

    /// The next pack.
    pub fn feed(&mut self, pack: &[u8]) -> Result<(), IngestError> {
        let fields = self.stored.fields();
        let width = fields.len();
        let head = 4 + 8 * width;
        let rows = read_u32(pack, 0)? as usize;
        if rows > MAX_PACK_ROWS || self.got + rows > self.expected {
            return Err(input(format!(
                "{rows} rows after {} of {} are too many",
                self.got, self.expected
            )));
        }
        if pack.len() < head {
            return Err(input("the pack is shorter than its header".to_owned()));
        }
        let mut at = head.next_multiple_of(8);
        for (index, cell) in self.plan.cells.iter().enumerate() {
            let flags = read_u32(pack, 4 + 8 * index)?;
            let text_len = read_u32(pack, 8 + 8 * index)? as usize;
            let name = fields[index].name.as_str();
            let validity = if flags & 1 == 1 {
                let bytes = take(pack, at, rows.div_ceil(8))?;
                at = (at + bytes.len()).next_multiple_of(8);
                Some(bytes)
            } else {
                None
            };
            let sources = &self.derived[index];
            let is_valid =
                |row: usize| validity.is_none_or(|bits| (bits[row / 8] >> (row % 8)) & 1 == 1);
            let err = |row: usize, message: String| IngestError::Json {
                row: self.got + row,
                field: Some(name.to_owned()),
                message,
            };
            // The NULL of a row: refused in a required column, else a NULL in the column and in
            // every column derived from it.
            macro_rules! null_row {
                ($row:expr) => {{
                    if !cell.nullable {
                        return Err(err(
                            $row,
                            "NULL is not allowed in a non-nullable column".to_owned(),
                        ));
                    }
                    self.builders.columns[cell.column].push_null();
                    for (column, _) in sources {
                        self.builders.columns[*column].push_null();
                    }
                }};
            }
            match cell.data_type {
                DataType::Bool => {
                    let values = take(pack, at, rows)?;
                    at = (at + rows).next_multiple_of(8);
                    for (row, byte) in values.iter().enumerate() {
                        if !is_valid(row) {
                            null_row!(row);
                        } else if *byte > 1 {
                            return Err(err(row, format!("{byte} is not a bool")));
                        } else {
                            self.builders.columns[cell.column]
                                .push_bool(*byte == 1)
                                .map_err(|message| err(row, message))?;
                        }
                    }
                }
                DataType::Int64 | DataType::Float64 => {
                    let values = take(pack, at, rows * 8)?;
                    at += rows * 8;
                    for (row, bytes) in values.as_chunks::<8>().0.iter().enumerate() {
                        if !is_valid(row) {
                            null_row!(row);
                            continue;
                        }
                        let number = f64::from_le_bytes(*bytes);
                        let builder = &mut self.builders.columns[cell.column];
                        let pushed = if cell.data_type == DataType::Int64 {
                            if number.fract() != 0.0 || number.abs() > 9_007_199_254_740_992.0 {
                                return Err(err(row, format!("{number} is not an int64")));
                            }
                            builder.push_i64(number as i64)
                        } else if number.is_finite() {
                            builder.push_f64(number)
                        } else {
                            return Err(err(row, format!("{number} is not a float64")));
                        };
                        pushed.map_err(|message| err(row, message))?;
                    }
                }
                DataType::Utf8
                | DataType::Decimal { .. }
                | DataType::Date
                | DataType::Timestamp => {
                    let offsets = take(pack, at, (rows + 1) * 4)?;
                    at += (rows + 1) * 4;
                    let text = take(pack, at, text_len)?;
                    at = (at + text_len).next_multiple_of(8);
                    let text = std::str::from_utf8(text)
                        .map_err(|_| err(0, "the text is not UTF-8".to_owned()))?;
                    let offset = |row: usize| {
                        u32::from_le_bytes(
                            offsets[row * 4..row * 4 + 4]
                                .try_into()
                                .expect("four bytes"),
                        ) as usize
                    };
                    if offset(0) != 0 || offset(rows) != text_len {
                        return Err(err(0, "the offsets do not span the text".to_owned()));
                    }
                    for row in 0..rows {
                        if !is_valid(row) {
                            null_row!(row);
                            continue;
                        }
                        let (from, to) = (offset(row), offset(row + 1));
                        let Some(cell_text) = text.get(from..to).filter(|_| from <= to) else {
                            return Err(err(
                                row,
                                "an offset is not a character boundary".to_owned(),
                            ));
                        };
                        let builder = &mut self.builders.columns[cell.column];
                        if cell.data_type == DataType::Utf8 {
                            builder
                                .push_str(cell_text)
                                .map_err(|message| err(row, message))?;
                        } else if sources.is_empty() {
                            cell::push_text(builder, cell_text, cell.data_type)
                                .map_err(|message| err(row, message))?;
                        } else {
                            // A derived column reads this cell: the value, which both are made of.
                            let value = cell::from_text(cell_text, cell.data_type)
                                .map_err(|message| err(row, message))?;
                            builder.push(&value).map_err(|message| err(row, message))?;
                            for (column, part) in sources {
                                self.builders.columns[*column]
                                    .push(&part.of(&value))
                                    .map_err(|message| err(row, message))?;
                            }
                        }
                    }
                }
                DataType::ListUtf8 | DataType::ListInt64 => unreachable!("refused by `new`"),
            }
        }
        if at != pack.len() {
            return Err(input(format!(
                "the pack is {} bytes, its header says {at}",
                pack.len()
            )));
        }
        self.got += rows;
        Ok(())
    }

    /// The end of the table: the rows must be the rows announced.
    pub fn finish(self) -> Result<Table, IngestError> {
        if self.got != self.expected {
            return Err(input(format!(
                "{} rows arrived, {} were announced",
                self.got, self.expected
            )));
        }
        self.builders.finish()
    }
}

fn input(message: String) -> IngestError {
    IngestError::Input { message }
}

fn read_u32(pack: &[u8], at: usize) -> Result<u32, IngestError> {
    pack.get(at..at + 4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four bytes")))
        .ok_or_else(|| input("the pack is shorter than its header".to_owned()))
}

fn take(pack: &[u8], at: usize, len: usize) -> Result<&[u8], IngestError> {
    at.checked_add(len)
        .and_then(|end| pack.get(at..end))
        .ok_or_else(|| input("a column runs past the end of the pack".to_owned()))
}
