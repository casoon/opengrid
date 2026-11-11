//! A query result as an Excel workbook (issue #72) — server-side only, behind
//! the feature `xlsx`, so the browser modules never carry it.
//!
//! **Excel types where Excel holds the value exactly, text where it cannot.**
//! XLSX is the format for people in a spreadsheet; CSV and JSON stay the exact
//! ones. No value changes on the way:
//!
//! | type | written as |
//! |---|---|
//! | `bool` | a boolean |
//! | `int64` | a number; **text** past ±2^53, where a double loses digits |
//! | `decimal` | a number with its scale as the format (`0.00`); **text** past 15 significant digits, Excel's precision |
//! | `float64` | a number; `NaN`, `Infinity`, `-Infinity` as the text the wire uses |
//! | `date` | an Excel date, `yyyy-mm-dd`; **text** outside Excel's years 1900–9999 |
//! | `timestamp` | an Excel date-time in UTC, `yyyy-mm-dd hh:mm:ss.000`; **text** (ISO) with sub-millisecond digits or outside those years |
//! | `utf8` | a string — never a formula, so no formula guard is needed |
//! | NULL | an empty cell |
//!
//! **One thing a workbook cannot tell apart:** the empty string and NULL.
//! Excel has no empty text apart from an empty cell, so both are one; CSV and
//! JSON keep them apart (rule S14).
//!
//! **Excel's limits are errors, never a shortened file**: more rows than a
//! sheet holds, or a text longer than a cell holds.
//!
//! A pivot is one flat sheet too ([`XlsxWriter::write_pivot`], issue #331):
//! the header line and the rows of its CSV, with the cells typed as above.
//!
//! The workbook is written as the rows come, and [`XlsxWriter::finish`]
//! assembles the file. With the feature `xlsx-streaming` (the server's) a
//! finished row goes to a temporary file, so memory does not grow with the
//! rows; without it (the tab's) the workbook is held in memory until `finish`.

use opengrid_datasource::QueryResult;
use opengrid_pivot::PivotResult;
use opengrid_types::{DataType, Field, Value};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook, Worksheet};

use crate::pivot::{PivotLabels, header_names, total_label};

/// The media type of an XLSX file.
pub const XLSX_MEDIA_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/// The rows one sheet holds under its header row.
pub const XLSX_MAX_ROWS: u64 = 1_048_575;

/// The columns one sheet holds.
const MAX_COLUMNS: usize = 16_384;

/// The characters one cell holds.
const MAX_TEXT: usize = 32_767;

/// The largest integer a double holds exactly, and the most significant
/// digits Excel keeps.
const EXACT_INTEGER: u64 = 1 << 53;
const EXCEL_DIGITS: u32 = 15;

/// Writes a workbook of one sheet, a result piece at a time.
pub struct XlsxWriter {
    workbook: Workbook,
    /// The next row to write; row 0 is the header.
    row: u32,
    header: Format,
    integer: Format,
    date: Format,
    timestamp: Format,
    /// Per column, the number format of a decimal column (its scale).
    decimals: Vec<Option<Format>>,
    /// Header texts by field name (issue #353).
    headers: std::collections::BTreeMap<String, String>,
}

impl XlsxWriter {
    /// A workbook whose one sheet is named `sheet` — the source's name, or
    /// `export` when Excel would not take it as a sheet name.
    pub fn new(sheet: &str) -> Result<Self, String> {
        let mut workbook = Workbook::new();
        // On the server a finished row goes to a temporary file; in the tab there
        // is none, and the workbook is held whole.
        #[cfg(feature = "xlsx-streaming")]
        let worksheet = workbook
            .add_worksheet_with_constant_memory()
            .map_err(|error| error.to_string())?;
        #[cfg(not(feature = "xlsx-streaming"))]
        let worksheet = workbook.add_worksheet();
        if worksheet.set_name(sheet).is_err() {
            worksheet
                .set_name("export")
                .map_err(|error| error.to_string())?;
        }
        worksheet
            .set_freeze_panes(1, 0)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            workbook,
            row: 0,
            header: Format::new().set_bold(),
            integer: Format::new().set_num_format("0"),
            date: Format::new().set_num_format("yyyy-mm-dd"),
            timestamp: Format::new().set_num_format("yyyy-mm-dd hh:mm:ss.000"),
            decimals: Vec::new(),
            headers: std::collections::BTreeMap::new(),
        })
    }

    /// Header texts by field name: a banded column is `Band · Column`.
    pub fn with_headers(mut self, headers: std::collections::BTreeMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Sets the header texts of a workbook that is already started.
    pub fn set_headers(&mut self, headers: std::collections::BTreeMap<String, String>) {
        self.headers = headers;
    }

    /// The next piece; the first also writes the header row.
    pub fn write(&mut self, result: &QueryResult) -> Result<(), String> {
        // A flat tree brings its level, path and match columns (T8).
        let result = &*crate::with_tree_columns(result);
        if self.row == 0 {
            let headers = self.headers.clone();
            let names: Vec<&str> = result
                .schema
                .fields()
                .iter()
                .map(|field| crate::csv::header_text(field.name.as_str(), &headers))
                .collect();
            self.head(result.schema.fields(), &names)?;
        }

        for index in 0..result.row_count() {
            if u64::from(self.row) > XLSX_MAX_ROWS {
                return Err(format!(
                    "more than the {XLSX_MAX_ROWS} rows an Excel sheet holds"
                ));
            }
            for (column, values) in result.columns.iter().enumerate() {
                self.cell(column, &values[index], &result.schema.fields()[column].name)?;
            }
            self.row += 1;
        }
        Ok(())
    }

    /// A whole pivot as the one flat sheet its CSV is ([`crate::pivot_csv`],
    /// issue #331): the same header line, the same rows in the same order, no
    /// merged cells and no outline levels — so the sheet sorts and filters.
    ///
    /// What differs from the CSV is what a workbook is for: a dimension value
    /// and a measure are cells of their type (the table above), and a NULL
    /// measure is an empty cell. A label — a total's, or the word that stands
    /// for a NULL or empty dimension value — is a string like every text here,
    /// never a formula; the dimension columns a total spans are empty cells.
    ///
    /// **Precondition**: the shape [`crate::pivot_csv`] names.
    pub fn write_pivot(
        &mut self,
        pivot: &PivotResult,
        labels: &impl PivotLabels,
    ) -> Result<(), String> {
        let fields = pivot.data.schema.fields();
        if fields.len() > MAX_COLUMNS {
            return Err(format!(
                "the pivot has {} columns, more than the {MAX_COLUMNS} an Excel sheet holds",
                fields.len()
            ));
        }
        let dimensions = fields.len() - pivot.columns.len();
        let names = header_names(pivot, labels);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        self.head(fields, &names)?;

        for (row, level) in pivot.row_levels.iter().enumerate() {
            if u64::from(self.row) > XLSX_MAX_ROWS {
                return Err(format!(
                    "more than the {XLSX_MAX_ROWS} rows an Excel sheet holds"
                ));
            }
            let level = usize::from(*level);
            if level < dimensions {
                // A subtotal, or the grand total: its label where the row
                // header starts; the columns it spans stay empty.
                let label = total_label(pivot, row, level, labels);
                self.text(0, &label, &fields[0].name)?;
            } else {
                for (column, field) in fields.iter().enumerate().take(dimensions) {
                    match &pivot.data.columns[column][row] {
                        // NULL and the empty string are named, as in the
                        // table: an empty cell here means "spanned".
                        Value::Null => self.text(column, &labels.dimension(None), &field.name)?,
                        Value::Utf8(text) if text.is_empty() => {
                            self.text(column, &labels.dimension(Some("")), &field.name)?;
                        }
                        value => self.cell(column, value, &field.name)?,
                    }
                }
            }
            for (column, field) in fields.iter().enumerate().skip(dimensions) {
                self.cell(column, &pivot.data.columns[column][row], &field.name)?;
            }
            self.row += 1;
        }
        Ok(())
    }

    /// The file.
    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        self.workbook
            .save_to_buffer()
            .map_err(|error| error.to_string())
    }

    /// The header row, and each decimal column's format from its scale.
    fn head(&mut self, fields: &[Field], names: &[&str]) -> Result<(), String> {
        self.decimals = fields
            .iter()
            .map(|field| match field.data_type {
                DataType::Decimal { scale, .. } => {
                    Some(Format::new().set_num_format(if scale == 0 {
                        "0".to_owned()
                    } else {
                        format!("0.{}", "0".repeat(usize::from(scale)))
                    }))
                }
                _ => None,
            })
            .collect();
        let header = self.header.clone();
        let sheet = self.sheet()?;
        for (column, name) in names.iter().enumerate() {
            sheet
                .write_string_with_format(0, column as u16, *name, &header)
                .map_err(|error| error.to_string())?;
        }
        self.row = 1;
        Ok(())
    }

    fn sheet(&mut self) -> Result<&mut Worksheet, String> {
        self.workbook
            .worksheet_from_index(0)
            .map_err(|error| error.to_string())
    }

    fn cell(
        &mut self,
        column: usize,
        value: &Value,
        field: &opengrid_types::FieldName,
    ) -> Result<(), String> {
        let row = self.row;
        let col = column as u16;
        let written = match value {
            Value::Null => return Ok(()),
            Value::Bool(flag) => {
                let flag = *flag;
                self.sheet()?.write_boolean(row, col, flag).map(|_| ())
            }
            Value::Int64(value) if value.unsigned_abs() <= EXACT_INTEGER => {
                let format = self.integer.clone();
                self.sheet()?
                    .write_number_with_format(row, col, *value as f64, &format)
                    .map(|_| ())
            }
            Value::Float64(value) if value.is_finite() => {
                let value = *value;
                self.sheet()?.write_number(row, col, value).map(|_| ())
            }
            Value::Decimal(decimal) if decimal.digit_count() <= EXCEL_DIGITS => {
                // The decimal's own text, read as the nearest double: shown
                // with Excel's 15 digits, it is the decimal again.
                let value: f64 = decimal
                    .to_string()
                    .parse()
                    .expect("a decimal's text is a number");
                let format = self.decimals[column]
                    .clone()
                    .expect("a decimal column has its format");
                self.sheet()?
                    .write_number_with_format(row, col, value, &format)
                    .map(|_| ())
            }
            Value::Date(date) => match excel_date(date.ymd()) {
                Some(day) => {
                    let format = self.date.clone();
                    self.sheet()?
                        .write_datetime_with_format(row, col, &day, &format)
                        .map(|_| ())
                }
                None => return self.text(column, &date.to_string(), field),
            },
            Value::Timestamp(timestamp) => match excel_timestamp(timestamp.micros()) {
                Some(moment) => {
                    let format = self.timestamp.clone();
                    self.sheet()?
                        .write_datetime_with_format(row, col, &moment, &format)
                        .map(|_| ())
                }
                None => return self.text(column, &timestamp.to_string(), field),
            },
            Value::Utf8(text) => return self.text(column, text, field),
            // Past what Excel holds exactly: the value's own text, unchanged.
            other => {
                let text = match opengrid_json::ToJson::to_json(other) {
                    opengrid_json::Json::String(text) => text,
                    number => number.to_string(),
                };
                return self.text(column, &text, field);
            }
        };
        written.map_err(|error| error.to_string())
    }

    fn text(
        &mut self,
        column: usize,
        text: &str,
        field: &opengrid_types::FieldName,
    ) -> Result<(), String> {
        let length = text.chars().count();
        if length > MAX_TEXT {
            return Err(format!(
                "a value of {field} has {length} characters, more than the {MAX_TEXT} an Excel cell holds"
            ));
        }
        let row = self.row;
        self.sheet()?
            .write_string(row, column as u16, text)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// A date Excel holds: its years 1900–9999.
fn excel_date((year, month, day): (i32, u32, u32)) -> Option<ExcelDateTime> {
    let year = u16::try_from(year)
        .ok()
        .filter(|year| (1900..=9999).contains(year))?;
    ExcelDateTime::from_ymd(year, month as u8, day as u8).ok()
}

/// A UTC instant Excel holds: in its years, and to the millisecond.
fn excel_timestamp(micros: i64) -> Option<ExcelDateTime> {
    if micros.rem_euclid(1000) != 0 {
        return None;
    }
    let timestamp = opengrid_types::Timestamp::from_micros(micros);
    let day = excel_date(timestamp.date().ymd())?;
    let within = micros.rem_euclid(86_400_000_000);
    let (hour, minute) = (within / 3_600_000_000, within / 60_000_000 % 60);
    let second = within / 1_000_000 % 60;
    let milli = within / 1000 % 1000;
    day.and_hms_milli(hour as u16, minute as u8, second as u8, milli as u16)
        .ok()
}
