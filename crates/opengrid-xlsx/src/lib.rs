//! A workbook from the answers of a query, written in the tab (issue #332).
//!
//! A module of its own, not part of the element or the engine: the writer is
//! about 20 KB of brotli that a page which never exports to Excel should not
//! load. `exportRows(…, { format: "xlsx" })` in `loader.js` fetches this module
//! when it needs it, hands it one answer after another — JSON or the binary
//! form, whichever the provider gives — and takes the file at the end.
//!
//! It is the server's writer (`opengrid_export::XlsxWriter`) with the same type
//! rules, without `xlsx-streaming`: the server writes finished rows to a
//! temporary file, which a tab does not have, so here the whole workbook is held
//! in memory until [`Book::finish`]. That is why `exportRows` bounds the rows
//! (`maxRows`) before it fetches more than a piece.
//!
//! The logic is plain Rust ([`Book`]), tested on the host; [`XlsxBook`] is the
//! thin `wasm-bindgen` skin over it.

use opengrid_datasource::QueryResult;
use opengrid_datasource::wire::{result_from_bytes, result_from_json};
use opengrid_export::XlsxWriter;
use wasm_bindgen::prelude::*;

/// One workbook of one sheet, a result piece at a time.
pub struct Book {
    writer: Option<XlsxWriter>,
}

impl Book {
    /// A workbook whose sheet is named `sheet`.
    pub fn new(sheet: &str) -> Result<Self, String> {
        Ok(Self {
            writer: Some(XlsxWriter::new(sheet)?),
        })
    }

    /// The header texts by field name (issue #353): a banded column is
    /// `Band · Column`. Before the first piece, or it has no effect.
    pub fn set_headers(&mut self, headers: std::collections::BTreeMap<String, String>) {
        if let Some(writer) = self.writer.as_mut() {
            writer.set_headers(headers);
        }
    }

    /// The next piece; the first also writes the header row.
    pub fn add(&mut self, result: &QueryResult) -> Result<(), String> {
        self.writer
            .as_mut()
            .ok_or_else(|| FINISHED.to_owned())?
            .write(result)
    }

    /// The next piece, as the JSON form of a result.
    pub fn add_json(&mut self, json: &str) -> Result<(), String> {
        let result = result_from_json(json).map_err(|error| error.to_string())?;
        self.add(&result)
    }

    /// The next piece, as the binary form (E35) — a tree's piece with its part.
    pub fn add_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let result = result_from_bytes(bytes)?;
        self.add(&result)
    }

    /// The file. A book is finished once.
    pub fn finish(&mut self) -> Result<Vec<u8>, String> {
        self.writer
            .take()
            .ok_or_else(|| FINISHED.to_owned())?
            .finish()
    }
}

const FINISHED: &str = "the workbook is finished already";

/// The answer of a provider as the page gives it: the result JSON, or the
/// binary form as a `Uint8Array` or an `ArrayBuffer`.
enum Answer {
    Json(String),
    Binary(Vec<u8>),
}

fn answer(value: &JsValue) -> Option<Answer> {
    if let Some(json) = value.as_string() {
        return Some(Answer::Json(json));
    }
    if let Some(bytes) = value.dyn_ref::<js_sys::Uint8Array>() {
        return Some(Answer::Binary(bytes.to_vec()));
    }
    if value.is_instance_of::<js_sys::ArrayBuffer>() {
        return Some(Answer::Binary(js_sys::Uint8Array::new(value).to_vec()));
    }
    None
}

/// A workbook being written; what `loader.js` drives.
#[wasm_bindgen]
pub struct XlsxBook {
    book: Book,
}

#[wasm_bindgen]
impl XlsxBook {
    /// `sheet` names the one sheet — the source's name; Excel's rules for a
    /// sheet name are the writer's, which falls back to `export`.
    #[wasm_bindgen(constructor)]
    pub fn new(sheet: &str) -> Result<XlsxBook, JsError> {
        Book::new(sheet)
            .map(|book| Self { book })
            .map_err(|error| JsError::new(&error))
    }

    /// Header texts by field name, `{ column: "Band · Column" }` (issue #353),
    /// before the first piece. Anything but strings is ignored.
    pub fn headers(&mut self, headers: JsValue) {
        let mut map = std::collections::BTreeMap::new();
        if headers.is_object() {
            for entry in js_sys::Object::entries(&js_sys::Object::from(headers)).iter() {
                let pair = js_sys::Array::from(&entry);
                if let (Some(name), Some(text)) = (pair.get(0).as_string(), pair.get(1).as_string())
                {
                    map.insert(name, text);
                }
            }
        }
        self.book.set_headers(map);
    }

    /// The next piece of the result: the answer of the provider, in either form.
    pub fn add(&mut self, piece: JsValue) -> Result<(), JsError> {
        match answer(&piece) {
            Some(Answer::Json(json)) => self.book.add_json(&json),
            Some(Answer::Binary(bytes)) => self.book.add_bytes(&bytes),
            None => Err("provider returned neither result JSON nor the binary form".to_owned()),
        }
        .map_err(|error| JsError::new(&error))
    }

    /// The workbook as a file, a `Uint8Array`.
    pub fn finish(&mut self) -> Result<Vec<u8>, JsError> {
        self.book.finish().map_err(|error| JsError::new(&error))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    const FIRST: &str = r#"{"total_count":3,"row_count":2,"columns":[
        {"name":"id","type":"int64","nullable":false,"values":[1,2]},
        {"name":"name","type":"utf8","nullable":true,"values":["Alpha","=cmd"]}]}"#;
    const SECOND: &str = r#"{"total_count":3,"row_count":1,"columns":[
        {"name":"id","type":"int64","nullable":false,"values":[3]},
        {"name":"name","type":"utf8","nullable":true,"values":[null]}]}"#;

    fn sheet(file: &[u8]) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(file)).expect("a zip");
        let mut xml = String::new();
        archive
            .by_name("xl/worksheets/sheet1.xml")
            .expect("sheet 1")
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    #[test]
    fn pieces_make_one_sheet_with_one_header() {
        let mut book = Book::new("orders").unwrap();
        book.add_json(FIRST).unwrap();
        book.add_json(SECOND).unwrap();
        let file = book.finish().unwrap();
        assert_eq!(&file[..2], b"PK");
        let xml = sheet(&file);
        // The header row and three rows: four `<row`s, not seven.
        assert_eq!(xml.matches("<row ").count(), 4, "{xml}");
    }

    fn shared_strings(file: &[u8]) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(file)).expect("a zip");
        let mut xml = String::new();
        archive
            .by_name("xl/sharedStrings.xml")
            .expect("shared strings")
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    /// A banded column's header says `Band · Column` (issue #353); the other
    /// keeps its field name.
    #[test]
    fn a_header_text_replaces_the_field_name() {
        let mut book = Book::new("orders").unwrap();
        book.set_headers(
            [("name".to_owned(), "Who \u{b7} name".to_owned())]
                .into_iter()
                .collect(),
        );
        book.add_json(FIRST).unwrap();
        let strings = shared_strings(&book.finish().unwrap());
        assert!(strings.contains("Who \u{b7} name"), "{strings}");
        assert!(strings.contains(">id<"), "{strings}");
        assert!(!strings.contains(">name<"), "{strings}");
    }

    #[test]
    fn a_book_is_finished_once() {
        let mut book = Book::new("orders").unwrap();
        book.add_json(FIRST).unwrap();
        book.finish().unwrap();
        assert_eq!(book.finish().unwrap_err(), FINISHED);
        assert_eq!(book.add_json(SECOND).unwrap_err(), FINISHED);
    }

    #[test]
    fn what_is_not_a_result_is_refused_with_a_sentence() {
        let mut book = Book::new("orders").unwrap();
        assert!(book.add_json("not json").is_err());
        assert!(book.add_bytes(&[1, 2, 3]).is_err());
    }
}
