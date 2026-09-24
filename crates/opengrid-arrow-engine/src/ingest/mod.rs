//! Ingest: reading CSV and JSON into Arrow [`RecordBatch`]es.
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
//! ([`Value::from_wire_str`], the string branch of [`Value::deserialize_typed`],
//! E13), so both formats share one implementation of scale handling (rule S8)
//! and of UTC normalization (rule S9). That is what makes "the same data in both formats yields identical
//! batches" hold by construction instead of by coincidence.
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
//! # Why not `arrow-csv`/`arrow-json`
//!
//! Point 06 planned the arrow-rs readers. Measured against the conformance
//! dataset (arrow 59.3.0, the version this workspace pins) they cannot carry
//! ingest:
//!
//! * `arrow-csv` reports NULL for an empty cell unless it is handed a
//!   `regex::Regex` as a null marker (which would lose rule S14 and add a
//!   dependency outside E3), and its typed reader answers a record with a
//!   field count that differs from the schema with an endless stream of
//!   `CsvError`s (in debug builds: an abort inside `StringRecord::get`).
//! * `arrow-json` does not read a JSON array at all (it reads newline-delimited
//!   objects), rejects the string spelling of decimals and of non-finite floats,
//!   and refuses a UTC timestamp type without the `chrono-tz` feature.
//!
//! The engine therefore tokenizes CSV itself and routes JSON through the wire
//! coercion, and builds the batches with `arrow-array`. See
//! plan/spezifikation/14-entscheidungen.md (E3).

use std::fmt;

use arrow_array::RecordBatch;
use arrow_schema::Schema as ArrowSchema;
use opengrid_datasource::QueryResult;
use opengrid_types::{DataType, Field, FieldName, Schema, Value};

pub(crate) mod batch;
mod cell;
mod csv;
mod json;

/// How many rows go into one [`RecordBatch`] unless the options say otherwise.
pub const DEFAULT_BATCH_SIZE: usize = 8192;

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
    /// Rows per batch.
    pub batch_size: usize,
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
            batch_size: DEFAULT_BATCH_SIZE,
            max_records: None,
        }
    }
}

/// How to read a JSON source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonOptions {
    /// Rows per batch.
    pub batch_size: usize,
}

impl Default for JsonOptions {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
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

/// Reads CSV bytes into batches of the given schema.
///
/// An input without records still yields **one empty batch**: a declared schema
/// with no rows is an *empty table*, not a missing one. The executor needs the
/// columns to answer a query at all, and rule S11 asks an aggregate without
/// `group` for exactly one row even over an empty input (plan point 08).
pub fn load_csv(
    bytes: &[u8],
    schema: &Schema,
    options: CsvOptions,
) -> Result<Vec<RecordBatch>, IngestError> {
    check_delimiter(options.delimiter)?;
    // The file holds the stored columns; a derived one is computed, never read
    // (plan point 54). Both ingest paths do it the same way, which is what keeps
    // CSV and JSON yielding identical batches.
    let stored = schema.stored();
    let layout = Layout::of(schema, &stored);

    let scanned = csv::scan(text(bytes)?, options.delimiter).map_err(|error| IngestError::Csv {
        line: error.line,
        message: error.message,
    })?;
    let mut records = scanned.into_iter();
    if options.has_header {
        match records.next() {
            Some(header) => check_header(&stored, &header.fields, header.line)?,
            None => return batches(schema, &[], options.batch_size),
        }
    }
    // One batch of rows at a time (point 45): the typed rows of the whole file
    // never exist at once — at 1 M rows × 10 columns that was ten million
    // values held only to be copied into builders. Same `read_row`, same
    // `batch::build`; only the amount held at once changed.
    let arrow: ArrowSchema = schema.into();
    let size = options.batch_size.max(1);
    let mut rows = Vec::with_capacity(size.min(records.len()));
    let mut out = Vec::new();
    for record in records {
        rows.push(layout.complete(read_row(&stored, &record, &options)?));
        if rows.len() == size {
            out.push(build_batch(schema, &arrow, &rows)?);
            rows.clear();
        }
    }
    if !rows.is_empty() || out.is_empty() {
        // The last, partial batch — or the one empty batch of an empty table.
        out.push(build_batch(schema, &arrow, &rows)?);
    }
    Ok(out)
}

fn build_batch(
    schema: &Schema,
    arrow: &ArrowSchema,
    rows: &[Vec<Value>],
) -> Result<RecordBatch, IngestError> {
    batch::build(schema, arrow, rows).map_err(|message| IngestError::Schema { message })
}

/// Reads JSON bytes — an array of objects in the column-oriented wire format
/// (E6/E13) — into batches of the given schema.
///
/// An empty array yields one empty batch, like an empty CSV file.
pub fn load_json(
    bytes: &[u8],
    schema: &Schema,
    options: JsonOptions,
) -> Result<Vec<RecordBatch>, IngestError> {
    let stored = schema.stored();
    let layout = Layout::of(schema, &stored);
    let rows = json::rows(text(bytes)?, &stored)?
        .into_iter()
        .map(|row| layout.complete(row))
        .collect::<Vec<_>>();
    batches(schema, &rows, options.batch_size)
}

/// Reads a [`QueryResult`] back into batches — the way a partial result returns
/// from a remote source into the local engine (plan point 28, decision E14).
///
/// E14 calls this "the coercion path of the ingest", and that is exactly what it
/// is: the values are already typed, so nothing is parsed; they are transposed
/// from the column-oriented result into the row-oriented shape the batch builder
/// takes and handed to the same code every other ingest path ends in. A result
/// whose columns disagree with its schema is a broken result, and says so.
pub fn load_result(
    result: &QueryResult,
    options: JsonOptions,
) -> Result<Vec<RecordBatch>, IngestError> {
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
    let mut rows = Vec::with_capacity(row_count);
    for row in 0..row_count {
        let mut cells = Vec::with_capacity(schema.len());
        for (index, column) in result.columns.iter().enumerate() {
            let value = column.get(row).ok_or_else(|| IngestError::Schema {
                message: format!(
                    "column {} has {} values, the first has {row_count}",
                    schema.fields()[index].name,
                    column.len()
                ),
            })?;
            cells.push(value.clone());
        }
        rows.push(cells);
    }
    batches(schema, &rows, options.batch_size)
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
    let scanned = csv::scan(text(bytes)?, options.delimiter).map_err(|error| IngestError::Csv {
        line: error.line,
        message: error.message,
    })?;
    let mut records = scanned.into_iter();
    let Some(header) = records.next() else {
        return Err(IngestError::Schema {
            message: "no header row to infer from".to_owned(),
        });
    };
    let samples: Vec<csv::Record> = records
        .take(options.max_records.unwrap_or(usize::MAX))
        .collect();
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

/// Reads one CSV record: same field count, then one value per column.
fn read_row(
    schema: &Schema,
    record: &csv::Record,
    options: &CsvOptions,
) -> Result<Vec<Value>, IngestError> {
    let fields = schema.fields();
    if record.fields.len() != fields.len() {
        return Err(IngestError::Csv {
            line: record.line,
            message: format!(
                "expected {} fields, found {}",
                fields.len(),
                record.fields.len()
            ),
        });
    }
    let mut row = Vec::with_capacity(fields.len());
    for (field, raw) in fields.iter().zip(&record.fields) {
        let value = if is_null(raw, options) {
            Value::Null
        } else {
            cell::from_text(raw, field.data_type).map_err(|message| IngestError::Row {
                line: record.line,
                column: field.name.as_str().to_owned(),
                message,
            })?
        };
        row.push(
            check_null(field, value).map_err(|message| IngestError::Row {
                line: record.line,
                column: field.name.as_str().to_owned(),
                message,
            })?,
        );
    }
    Ok(row)
}

/// NULL is allowed in a nullable column only.
pub(crate) fn check_null(field: &Field, value: Value) -> Result<Value, String> {
    if value == Value::Null && !field.nullable {
        return Err("NULL is not allowed in a non-nullable column".to_owned());
    }
    Ok(value)
}

/// Cuts rows into batches.
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

    fn complete(&self, row: Vec<Value>) -> Vec<Value> {
        let Some(columns) = &self.0 else {
            return row;
        };
        columns
            .iter()
            .map(|column| match column {
                Source::Stored(index) => row[*index].clone(),
                Source::Part(index, part) => part.of(&row[*index]),
            })
            .collect()
    }
}

fn batches(
    schema: &Schema,
    rows: &[Vec<Value>],
    batch_size: usize,
) -> Result<Vec<RecordBatch>, IngestError> {
    let arrow: ArrowSchema = schema.into();
    if rows.is_empty() {
        // The empty table keeps its columns — see `load_csv`.
        let batch =
            batch::build(schema, &arrow, &[]).map_err(|message| IngestError::Schema { message })?;
        return Ok(vec![batch]);
    }
    rows.chunks(batch_size.max(1))
        .map(|chunk| {
            batch::build(schema, &arrow, chunk).map_err(|message| IngestError::Schema { message })
        })
        .collect()
}
