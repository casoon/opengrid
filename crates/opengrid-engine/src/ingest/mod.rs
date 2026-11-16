//! Ingest: reading CSV and JSON into a [`Table`].
//!
//! Both formats are read against an explicit [`Schema`] — ingest never guesses
//! column types. [`infer_schema_csv`] proposes a schema, but the caller confirms
//! it.
//!
//! # One coercion path
//!
//! A CSV cell is text, a JSON cell is a JSON scalar, but inside a column they
//! have to mean exactly the same thing. For `decimal`, `date` and `timestamp`
//! the CSV text therefore goes through the wire coercion of `opengrid-types`
//! ([`Value::from_wire_str`], the string branch of [`Value::from_json_typed`],
//! E13), so both formats share one implementation of scale handling (rule S8)
//! and of UTC normalization (rule S9). That is what makes "the same data in both formats yields identical
//! tables" hold by construction instead of by coincidence.
//!
//! # CSV dialect (RFC 4180 style, [`CsvOptions`])
//!
//! * Delimiter: one ASCII byte, `,` by default.
//! * Quote: `"`; a doubled `""` inside a quoted field is one literal quote.
//!   Quoted fields may contain the delimiter and line breaks.
//! * Record separator: `\n`, `\r\n` or `\r`. A leading UTF-8 BOM is ignored.
//! * Completely empty lines are skipped; a line holding only delimiters is a
//!   record of empty fields.
//! * Cells are taken verbatim — no trimming, no type guessing.
//! * An empty cell is the empty string (rule S14), NULL is spelled `\N` by
//!   default (see [`CsvOptions::null_values`]).
//!
//! # Line numbers
//!
//! Errors carry the 1-based physical line of the record they belong to; line 1
//! is the header when [`CsvOptions::has_header`] is set. A quoted field spanning
//! several lines does not shift the numbering: the record reports the line it
//! starts on.
//!
//! # Own tokenizer
//!
//! The engine tokenizes CSV itself and routes JSON through the wire coercion.
//! The arrow-rs readers were measured in point 06 and could not carry ingest
//! (empty cell as NULL, no JSON arrays, no string decimals); since E34 there is
//! no Arrow in the engine at all.

use std::fmt;

use opengrid_columns::{ColumnBuilder, Table};
use opengrid_datasource::QueryResult;
use opengrid_types::{DataType, Field, FieldName, Schema, Value};

mod cell;
mod columns;
mod csv;
mod json;

pub use columns::ColumnsLoader;

/// How to read a CSV source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvOptions {
    /// Field delimiter: one ASCII byte that is not a quote or a line break.
    pub delimiter: u8,
    /// Whether the first record holds the column names. When set, those names
    /// are checked against the schema before any row is read.
    pub has_header: bool,
    /// Cell texts that mean NULL instead of a value. `\N` by default — the
    /// spelling used by the conformance dataset. The empty string is *not* in
    /// here: an empty cell is the empty string (rule S14).
    pub null_values: Vec<String>,
    /// Records [`infer_schema_csv`] looks at; `None` reads all of them.
    /// [`load_csv`] ignores this field.
    pub max_records: Option<usize>,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            has_header: true,
            null_values: vec!["\\N".to_owned()],
            max_records: None,
        }
    }
}

/// Everything that can go wrong while ingesting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestError {
    /// The bytes cannot be used at all: not UTF-8, unusable delimiter, not JSON.
    Input {
        /// What is wrong with the input.
        message: String,
    },
    /// A record does not fit the schema: wrong field count, or the CSV syntax
    /// itself is broken.
    Csv {
        /// Physical line the record starts on (1-based).
        line: usize,
        /// What is wrong.
        message: String,
    },
    /// The header row does not name the schema's columns.
    Header {
        /// Physical line of the header (1).
        line: usize,
        /// What was expected and what was found.
        message: String,
    },
    /// A cell of a row could not be read as the column's type, or it is NULL in
    /// a non-nullable column.
    Row {
        /// Physical line of the record (1-based).
        line: usize,
        /// The column the cell belongs to.
        column: String,
        /// What is wrong with the cell.
        message: String,
    },
    /// A JSON row could not be read as the schema.
    Json {
        /// Zero-based position in the JSON array.
        row: usize,
        /// The field, when the problem belongs to one.
        field: Option<String>,
        /// What is wrong.
        message: String,
    },
    /// The schema cannot be built, or cannot be used for this input.
    Schema {
        /// What is wrong.
        message: String,
    },
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IngestError::Input { message } => write!(f, "input: {message}"),
            IngestError::Csv { line, message } => write!(f, "CSV line {line}: {message}"),
            IngestError::Header { line, message } => write!(f, "CSV line {line}: {message}"),
            IngestError::Row {
                line,
                column,
                message,
            } => write!(f, "CSV line {line}, column {column}: {message}"),
            IngestError::Json {
                row,
                field: Some(field),
                message,
            } => write!(f, "JSON row {row}, field {field}: {message}"),
            IngestError::Json {
                row,
                field: None,
                message,
            } => write!(f, "JSON row {row}: {message}"),
            IngestError::Schema { message } => write!(f, "schema: {message}"),
        }
    }
}

impl std::error::Error for IngestError {}

/// Reads CSV bytes into a table of the given schema.
///
/// An input without records is an **empty table**, not a missing one: a
/// declared schema with no rows. The executor needs the columns to answer a
/// query at all, and rule S11 asks an aggregate without `group` for exactly one
/// row even over an empty input (plan point 08).
pub fn load_csv(bytes: &[u8], schema: &Schema, options: CsvOptions) -> Result<Table, IngestError> {
    check_delimiter(options.delimiter)?;
    // The file holds the stored columns; a derived one is computed, never read
    // (plan point 54). Both ingest paths do it the same way, which is what keeps
    // CSV and JSON yielding identical tables.
    let stored = schema.stored();
    let plan = Plan::of(schema, &stored);

    let to_error = |error: csv::ScanError| IngestError::Csv {
        line: error.line,
        message: error.message,
    };
    let mut records = csv::scan(text(bytes)?, options.delimiter);
    if options.has_header {
        match records.next().transpose().map_err(to_error)? {
            Some(header) => check_header(&stored, &header.fields, header.line)?,
            None => return Builders::new(schema, 0).finish(),
        }
    }
    // Record by record straight into the column builders (point 45): neither
    // the records nor the typed rows of the whole file exist at once.
    // Sized by the line breaks of the file, so the columns are allocated once: grown by doubling
    // they hold the old and the new buffer at the same moment, and the module's memory, which
    // never shrinks, keeps the room of every step (issue #427).
    let mut builders = Builders::new(schema, bytes.iter().filter(|byte| **byte == b'\n').count());
    let mut kept = Vec::new();
    while let Some(line) = records.advance() {
        let line = line.map_err(to_error)?;
        builders.push_csv(&plan, records.fields(), line, &options, &mut kept)?;
    }
    builders.finish()
}

/// A CSV file read piece by piece: what [`load_csv`] does with the whole file at once, for a file
/// that arrives in chunks (a stream, a file read a slice at a time). Only a chunk and the columns
/// are in memory, not the file as well (issue #427).
///
/// The pieces may end anywhere, inside a field or a character; a record that is not complete is
/// kept until the next piece completes it. The result is the table [`load_csv`] makes of the
/// pieces joined, errors and line numbers included.
pub struct CsvLoader {
    stored: Schema,
    plan: Plan,
    options: CsvOptions,
    builders: Builders<'static>,
    /// The bytes after the last whole record of what was fed.
    carry: Vec<u8>,
    /// Physical line the next record starts on.
    line: usize,
    header_pending: bool,
    /// The file's size when the caller knows it: the columns are sized by it.
    size_hint: usize,
    started: bool,
    first: bool,
    /// The values the derived columns of a row are taken from, reused row after row.
    kept: Vec<Value>,
}

impl CsvLoader {
    /// A loader for a file of this schema. `size_hint` is the file's size in bytes, or 0 when it
    /// is not known: it only sizes the columns, a wrong one costs memory, not correctness.
    pub fn new(schema: &Schema, options: CsvOptions) -> Result<Self, IngestError> {
        check_delimiter(options.delimiter)?;
        // The file holds the stored columns; a derived one is computed, never read
        // (plan point 54). Both ingest paths do it the same way, which is what keeps
        // CSV and JSON yielding identical tables.
        let stored = schema.stored();
        let plan = Plan::of(schema, &stored);
        Ok(Self {
            stored,
            plan,
            header_pending: options.has_header,
            options,
            builders: Builders::with(schema.clone(), 0),
            carry: Vec::new(),
            line: 1,
            size_hint: 0,
            started: false,
            first: true,
            kept: Vec::new(),
        })
    }

    /// Tells the loader the file's size, before the first piece.
    pub fn with_size_hint(mut self, bytes: usize) -> Self {
        self.size_hint = bytes;
        self
    }

    /// Sizes the columns for `rows` rows, once.
    fn start(&mut self, rows: usize) {
        if !self.started {
            self.started = true;
            self.builders = Builders::with(self.builders.schema.clone(), rows);
        }
    }

    /// The next piece of the file.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), IngestError> {
        if !self.started && self.size_hint > 0 && !chunk.is_empty() {
            // The rows of the whole file, from the lines of the first piece: an estimate, a
            // little generous, so that the columns are rarely grown.
            let lines = chunk.iter().filter(|byte| **byte == b'\n').count().max(1);
            let estimate = self.size_hint / (chunk.len() / lines).max(1);
            self.start(estimate + estimate / 50);
        }
        let mut joined = std::mem::take(&mut self.carry);
        let whole: &[u8] = if joined.is_empty() {
            chunk
        } else {
            joined.extend_from_slice(chunk);
            &joined
        };
        // The BOM is not a field's first character: the quotes are followed from after it.
        let skip = if self.first && whole.starts_with("\u{feff}".as_bytes()) {
            3
        } else {
            0
        };
        let end = match csv::last_boundary(&whole[skip..], self.options.delimiter) {
            0 => 0,
            end => skip + end,
        };
        self.consume(&whole[..end])?;
        self.carry = whole[end..].to_vec();
        Ok(())
    }

    /// The end of the file: the table.
    pub fn finish(mut self) -> Result<Table, IngestError> {
        let rest = std::mem::take(&mut self.carry);
        self.consume(&rest)?;
        self.builders.finish()
    }

    /// Reads whole records: the bytes end where a record ends, or are the end of the file.
    fn consume(&mut self, bytes: &[u8]) -> Result<(), IngestError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let text = std::str::from_utf8(bytes).map_err(|error| IngestError::Input {
            message: format!(
                "input is not UTF-8 (invalid byte at offset {})",
                error.valid_up_to()
            ),
        })?;
        // A BOM marks the encoding, it is not part of the first column name.
        let text = if std::mem::take(&mut self.first) {
            text.strip_prefix('\u{feff}').unwrap_or(text)
        } else {
            text
        };
        let to_error = |error: csv::ScanError| IngestError::Csv {
            line: error.line,
            message: error.message,
        };
        let Self {
            stored,
            plan,
            options,
            builders,
            line,
            header_pending,
            kept,
            ..
        } = self;
        let mut records = csv::scan_from(text, options.delimiter, *line);
        // Record by record straight into the column builders (point 45): neither
        // the records nor the typed rows of the whole file exist at once.
        while let Some(record_line) = records.advance() {
            let record_line = record_line.map_err(to_error)?;
            if std::mem::take(header_pending) {
                check_header(stored, records.fields(), record_line)?;
                continue;
            }
            builders.push_csv(plan, records.fields(), record_line, options, kept)?;
        }
        *line = records.line();
        Ok(())
    }
}

/// Reads JSON bytes — an array of objects in the column-oriented wire format
/// (E6/E13) — into a table of the given schema.
///
/// An empty array is an empty table, like an empty CSV file.
pub fn load_json(bytes: &[u8], schema: &Schema) -> Result<Table, IngestError> {
    let stored = schema.stored();
    let layout = Layout::of(schema, &stored);
    let text = text(bytes)?;
    // The plain text first, in one pass (issue #426); what it will not read, the general path does.
    if let Some(table) = json::fast(text, schema, &stored, &layout) {
        return Ok(table);
    }
    load_json_general(text, schema, &stored, &layout)
}

/// The general path: the whole text as a JSON tree, then rows, then columns. It reads every shape
/// the contract allows and says what is wrong with the rest.
fn load_json_general(
    text: &str,
    schema: &Schema,
    stored: &Schema,
    layout: &Layout,
) -> Result<Table, IngestError> {
    let rows = json::rows(text, stored)?;
    let mut builders = Builders::new(schema, rows.len());
    for row in rows {
        builders.push(&layout.complete(row))?;
    }
    builders.finish()
}

/// Reads a [`QueryResult`] back into a table — the way a partial result returns
/// from a remote source into the local engine (plan point 28, decision E14).
///
/// E14 calls this "the coercion path of the ingest", and that is exactly what it
/// is: the values are already typed, so nothing is parsed; they go into the
/// same column builders every other ingest path ends in. A result whose columns
/// disagree with its schema is a broken result, and says so.
pub fn load_result(result: &QueryResult) -> Result<Table, IngestError> {
    let schema = &result.schema;
    if result.columns.len() != schema.len() {
        return Err(IngestError::Schema {
            message: format!(
                "result has {} columns for {} fields",
                result.columns.len(),
                schema.len()
            ),
        });
    }
    let row_count = result.row_count();
    let mut columns = Vec::with_capacity(schema.len());
    for (field, values) in schema.fields().iter().zip(&result.columns) {
        if values.len() != row_count {
            return Err(IngestError::Schema {
                message: format!(
                    "column {} has {} values, the first has {row_count}",
                    field.name,
                    values.len()
                ),
            });
        }
        let mut builder = ColumnBuilder::new(field.data_type, row_count);
        for value in values {
            builder
                .push(value)
                .map_err(|message| mismatch(field, message))?;
        }
        columns.push(builder.finish());
    }
    Table::new(schema, columns).map_err(|message| IngestError::Schema { message })
}

/// Builds a table from pieces of rows — the way a source that only hands out
/// rows (issue #46) becomes something the engine can answer.
///
/// Each piece holds the **stored** columns of `schema`, in order; the derived
/// ones are computed here, as on every other ingest path. The rows go straight
/// into the column builders, piece by piece, so the typed rows of the whole
/// source never exist at once.
pub struct PieceBuilder<'a> {
    stored: Schema,
    layout: Layout,
    builders: Builders<'a>,
    rows: u64,
}

impl<'a> PieceBuilder<'a> {
    pub fn new(schema: &'a Schema) -> Self {
        let stored = schema.stored();
        let layout = Layout::of(schema, &stored);
        Self {
            stored,
            layout,
            builders: Builders::new(schema, 0),
            rows: 0,
        }
    }

    /// Appends a piece. Its columns must be the stored columns of the schema,
    /// by name and type, in order.
    pub fn push(&mut self, piece: &QueryResult) -> Result<(), IngestError> {
        let names = |schema: &Schema| -> Vec<(String, DataType)> {
            schema
                .fields()
                .iter()
                .map(|field| (field.name.as_str().to_owned(), field.data_type))
                .collect()
        };
        if names(&piece.schema) != names(&self.stored) {
            return Err(IngestError::Schema {
                message: format!(
                    "a piece has the columns {:?}, the source's stored columns are {:?}",
                    names(&piece.schema),
                    names(&self.stored)
                ),
            });
        }
        let row_count = piece.row_count();
        if piece.columns.iter().any(|column| column.len() != row_count) {
            return Err(IngestError::Schema {
                message: "a piece has columns of different lengths".to_owned(),
            });
        }
        for row in 0..row_count {
            let values = piece
                .columns
                .iter()
                .map(|column| column[row].clone())
                .collect();
            self.builders.push(&self.layout.complete(values))?;
        }
        self.rows += row_count as u64;
        Ok(())
    }

    /// The rows pushed so far.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    pub fn finish(self) -> Result<Table, IngestError> {
        self.builders.finish()
    }
}

/// One builder per column of the schema, filled a row at a time.
struct Builders<'a> {
    schema: Schema,
    lifetime: std::marker::PhantomData<&'a ()>,
    columns: Vec<ColumnBuilder>,
}

impl<'a> Builders<'a> {
    fn new(schema: &'a Schema, capacity: usize) -> Self {
        Self::with(schema.clone(), capacity)
    }

    fn with(schema: Schema, capacity: usize) -> Self {
        let columns = schema
            .fields()
            .iter()
            .map(|field| ColumnBuilder::new(field.data_type, capacity))
            .collect();
        Self {
            schema,
            lifetime: std::marker::PhantomData,
            columns,
        }
    }

    /// Appends one row. The values went through [`cell`] before, so a mismatch
    /// here means the caller is broken — reported as an error rather than a
    /// panic, because ingest is the path untrusted input takes.
    fn push(&mut self, row: &[Value]) -> Result<(), IngestError> {
        for ((builder, field), value) in self.columns.iter_mut().zip(self.schema.fields()).zip(row)
        {
            builder
                .push(value)
                .map_err(|message| mismatch(field, message))?;
        }
        Ok(())
    }

    /// One CSV record, cell by cell into its columns: no row of [`Value`]s in between (issue
    /// #440). The stored cells first, in the order of the file — which is the order the errors
    /// are reported in — then the columns derived from them.
    fn push_csv(
        &mut self,
        plan: &Plan,
        cells: &[std::borrow::Cow<'_, str>],
        line: usize,
        options: &CsvOptions,
        kept: &mut Vec<Value>,
    ) -> Result<(), IngestError> {
        if cells.len() != plan.cells.len() {
            return Err(IngestError::Csv {
                line,
                message: format!(
                    "expected {} fields, found {}",
                    plan.cells.len(),
                    cells.len()
                ),
            });
        }
        let fields = self.schema.fields();
        let in_row = |column: usize, message: String| IngestError::Row {
            line,
            column: fields[column].name.as_str().to_owned(),
            message,
        };
        kept.clear();
        for (cell, raw) in plan.cells.iter().zip(cells) {
            let builder = &mut self.columns[cell.column];
            if is_null(raw, options) {
                if !cell.nullable {
                    return Err(in_row(
                        cell.column,
                        "NULL is not allowed in a non-nullable column".to_owned(),
                    ));
                }
                builder.push_null();
                if cell.kept {
                    kept.push(Value::Null);
                }
                continue;
            }
            if cell.text {
                // A text cell is not made a `Value`: it goes into its column as it is (#429).
                builder
                    .push_str(raw)
                    .map_err(|message| mismatch(&fields[cell.column], message))?;
                continue;
            }
            if !cell.kept {
                // The cell goes into its column as the type it is read as: no `Value` in between
                // (issue #440).
                cell::push_text(builder, raw, cell.data_type)
                    .map_err(|message| in_row(cell.column, message))?;
                continue;
            }
            // A derived column reads this cell: its value is kept for the row.
            let value = cell::from_text(raw, cell.data_type)
                .map_err(|message| in_row(cell.column, message))?;
            builder
                .push(&value)
                .map_err(|message| mismatch(&fields[cell.column], message))?;
            kept.push(value);
        }
        for part in &plan.parts {
            self.columns[part.column]
                .push(&part.part.of(&kept[part.kept]))
                .map_err(|message| mismatch(&fields[part.column], message))?;
        }
        Ok(())
    }

    fn finish(self) -> Result<Table, IngestError> {
        let columns = self
            .columns
            .into_iter()
            .map(ColumnBuilder::finish)
            .collect();
        Table::new(&self.schema, columns).map_err(|message| IngestError::Schema { message })
    }
}

/// The value does not belong to the column it sits in.
fn mismatch(field: &Field, message: String) -> IngestError {
    IngestError::Schema {
        message: format!("column {} {message}", field.name.as_str()),
    }
}

/// Proposes a schema for CSV bytes with a header row.
///
/// The result is a *suggestion*: `decimal` is never inferred (a JSON-free text
/// column with digits after the point reads as `float64`), and a column whose
/// samples all look like something else stays `utf8`. Numbers count as `int64`
/// only when every sample is an integer without a fraction.
///
/// A NULL cell (`\N`, see [`CsvOptions::null_values`]) is not a value, it says
/// "no value here": it is skipped when the types are compared and makes the
/// column nullable. An empty cell *is* a value (rule S14), so it keeps a
/// numeric column from being numeric at all.
///
/// The caller confirms the schema, so this is deliberately not part of
/// [`load_csv`].
pub fn infer_schema_csv(bytes: &[u8], options: CsvOptions) -> Result<Schema, IngestError> {
    check_delimiter(options.delimiter)?;
    if !options.has_header {
        return Err(IngestError::Schema {
            message: "infer_schema_csv needs a header row to name the columns".to_owned(),
        });
    }
    let to_error = |error: csv::ScanError| IngestError::Csv {
        line: error.line,
        message: error.message,
    };
    let mut records = csv::scan(text(bytes)?, options.delimiter);
    let Some(header) = records.next().transpose().map_err(to_error)? else {
        return Err(IngestError::Schema {
            message: "no header row to infer from".to_owned(),
        });
    };
    let samples = records
        .take(options.max_records.unwrap_or(usize::MAX))
        .collect::<Result<Vec<csv::Record>, _>>()
        .map_err(to_error)?;
    let mut fields = Vec::with_capacity(header.fields.len());
    for (position, name) in header.fields.iter().enumerate() {
        let name = FieldName::new(name.as_ref()).map_err(|error| IngestError::Schema {
            message: error.to_string(),
        })?;
        let mut values = Vec::with_capacity(samples.len());
        for record in &samples {
            let raw = record
                .fields
                .get(position)
                .ok_or_else(|| IngestError::Csv {
                    line: record.line,
                    message: format!(
                        "expected {} fields, found {}",
                        header.fields.len(),
                        record.fields.len()
                    ),
                })?;
            values.push(raw.as_ref());
        }
        let data_type = infer(&values, &options);
        let nullable = values.iter().any(|value| is_null(value, &options));
        fields.push(Field {
            name,
            data_type,
            nullable,
            // Inference proposes the columns a file *has*; a derived column is
            // something a person adds on purpose (point 54).
            from: None,
        });
    }
    Ok(Schema::new(fields))
}

/// The type a column of samples reads as, widest guess first.
///
/// NULL cells are left out: they carry no type information, and a single one of
/// them must not turn a column of numbers into text. A column that holds
/// nothing but NULLs stays text — there is nothing to guess from.
fn infer(values: &[&str], options: &CsvOptions) -> DataType {
    let typed: Vec<&str> = values
        .iter()
        .copied()
        .filter(|value| !is_null(value, options))
        .collect();
    if typed.is_empty() {
        return DataType::Utf8;
    }
    [
        DataType::Bool,
        DataType::Int64,
        DataType::Float64,
        DataType::Timestamp,
        DataType::Date,
    ]
    .into_iter()
    .find(|candidate| {
        typed
            .iter()
            .all(|value| cell::from_text(value, *candidate).is_ok())
    })
    .unwrap_or(DataType::Utf8)
}

/// Turns bytes into `&str`, rejecting everything that is not UTF-8.
fn text(bytes: &[u8]) -> Result<&str, IngestError> {
    let text = std::str::from_utf8(bytes).map_err(|error| IngestError::Input {
        message: format!(
            "input is not UTF-8 (invalid byte at offset {})",
            error.valid_up_to()
        ),
    })?;
    // A BOM marks the encoding, it is not part of the first column name.
    Ok(text.strip_prefix('\u{feff}').unwrap_or(text))
}

/// Rejects delimiters the scanner cannot work with.
fn check_delimiter(delimiter: u8) -> Result<(), IngestError> {
    if !delimiter.is_ascii() || matches!(delimiter, b'"' | b'\n' | b'\r') {
        return Err(IngestError::Input {
            message: format!(
                "delimiter {delimiter:#04x} is not usable (expected an ASCII byte that is not a quote or a line break)"
            ),
        });
    }
    Ok(())
}

/// The header names have to be the schema's columns, in order: a shifted column
/// would silently fill the wrong types otherwise.
fn check_header(
    schema: &Schema,
    names: &[std::borrow::Cow<'_, str>],
    line: usize,
) -> Result<(), IngestError> {
    let expected: Vec<&str> = schema.fields().iter().map(|f| f.name.as_str()).collect();
    let found: Vec<&str> = names.iter().map(|name| name.as_ref()).collect();
    if expected != found {
        return Err(IngestError::Header {
            line,
            message: format!(
                "header does not match the schema: expected [{}], found [{}]",
                expected.join(", "),
                found.join(", ")
            ),
        });
    }
    Ok(())
}

/// True when the cell text is one of the NULL spellings.
fn is_null(cell: &str, options: &CsvOptions) -> bool {
    options.null_values.iter().any(|null| null == cell)
}

/// NULL is allowed in a nullable column only.
pub(crate) fn check_null(field: &Field, value: Value) -> Result<Value, String> {
    if value == Value::Null && !field.nullable {
        return Err("NULL is not allowed in a non-nullable column".to_owned());
    }
    Ok(value)
}

/// What a CSV record needs to go into its columns, worked out once per file: per cell the column
/// it belongs to and how it is read, and the columns that are derived from a cell.
struct Plan {
    cells: Vec<PlanCell>,
    parts: Vec<PlanPart>,
}

/// One stored cell of a record.
struct PlanCell {
    /// The column of the schema it goes into.
    column: usize,
    data_type: DataType,
    text: bool,
    nullable: bool,
    /// Whether a derived column reads this cell: its value is kept for the row.
    kept: bool,
}

/// One derived column.
struct PlanPart {
    column: usize,
    /// Which of the kept values of the row it is a part of.
    kept: usize,
    part: opengrid_types::DatePart,
}

impl Plan {
    fn of(schema: &Schema, stored: &Schema) -> Self {
        let column_of = |name: &str| {
            schema
                .index_of(name)
                .expect("a stored field is a field of the schema")
        };
        let sources: Vec<usize> = schema
            .fields()
            .iter()
            .filter_map(|field| field.from.as_ref())
            .map(|derivation| {
                stored
                    .index_of(derivation.field.as_str())
                    .expect("the schema check proved the source exists")
            })
            .collect();
        let cells: Vec<PlanCell> = stored
            .fields()
            .iter()
            .enumerate()
            .map(|(index, field)| PlanCell {
                column: column_of(field.name.as_str()),
                data_type: field.data_type,
                text: field.data_type == DataType::Utf8,
                nullable: field.nullable,
                kept: sources.contains(&index),
            })
            .collect();
        // The kept values of a row are in the order of the cells.
        let slot = |cell: usize| cells[..cell].iter().filter(|cell| cell.kept).count();
        let parts = schema
            .fields()
            .iter()
            .enumerate()
            .filter_map(|(column, field)| {
                let derivation = field.from.as_ref()?;
                let cell = stored
                    .index_of(derivation.field.as_str())
                    .expect("the schema check proved the source exists");
                Some(PlanPart {
                    column,
                    kept: slot(cell),
                    part: derivation.part,
                })
            })
            .collect();
        Plan { cells, parts }
    }
}

/// Where each column of a row comes from — computed once, used per row.
///
/// Without this the derivation would look up a field name per cell; over a
/// million rows that is a linear scan per cell, and ingest is already the
/// slowest step of the engine (12-qualitaet.md §Engine-Latenz).
enum Source {
    /// Position in the stored row.
    Stored(usize),
    /// A part of the value at that position.
    Part(usize, opengrid_types::DatePart),
}

struct Layout(Option<Vec<Source>>);

impl Layout {
    /// `None` when nothing is derived — then a row passes through untouched.
    fn of(schema: &Schema, stored: &Schema) -> Self {
        if !schema.has_derived() {
            return Layout(None);
        }
        let columns = schema
            .fields()
            .iter()
            .map(|field| match &field.from {
                None => Source::Stored(
                    stored
                        .index_of(field.name.as_str())
                        .expect("a field is stored or derived"),
                ),
                Some(derivation) => Source::Part(
                    stored
                        .index_of(derivation.field.as_str())
                        .expect("the schema check proved the source exists"),
                    derivation.part,
                ),
            })
            .collect();
        Layout(Some(columns))
    }

    fn complete(&self, mut row: Vec<Value>) -> Vec<Value> {
        let Some(columns) = &self.0 else {
            return row;
        };
        // Parts first, while every stored value is still in the row; then the stored values move
        // out of it, not cloned — a text cell cloned is an allocation per cell per row.
        let parts: Vec<Value> = columns
            .iter()
            .filter_map(|column| match column {
                Source::Part(index, part) => Some(part.of(&row[*index])),
                Source::Stored(_) => None,
            })
            .collect();
        let mut parts = parts.into_iter();
        columns
            .iter()
            .map(|column| match column {
                Source::Stored(index) => std::mem::replace(&mut row[*index], Value::Null),
                Source::Part(..) => parts.next().expect("one part per derived column"),
            })
            .collect()
    }
}
