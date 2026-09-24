//! The CSV scanner: bytes in, records with line numbers out.
//!
//! Deliberately small. It knows quotes, doubled quotes, embedded line breaks
//! and the three record separators, and nothing else — no type guessing, no
//! trimming, no escape sequences. See the module docs of [`super`] for the
//! dialect and for why the arrow-rs CSV reader is not used here.

use std::borrow::Cow;

/// One parsed record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record<'a> {
    /// The fields, verbatim — borrowed from the input wherever a field is one
    /// contiguous stretch of it, which is every field without a doubled quote
    /// (point 45: no allocation per cell).
    pub(crate) fields: Vec<Cow<'a, str>>,
    /// Physical line the record starts on (1-based).
    pub(crate) line: usize,
}

/// A syntax error, pinned to the line it starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanError {
    /// Physical line (1-based).
    pub(crate) line: usize,
    /// What is wrong.
    pub(crate) message: String,
}

/// A field being read: the stretches of the input it consists of.
///
/// One stretch stays a borrow; only a second one — after a doubled quote, or
/// text following a closing quote — makes it an owned string.
#[derive(Default)]
struct Field {
    owned: Option<String>,
    span: Option<(usize, usize)>,
}

impl Field {
    fn push(&mut self, input: &str, start: usize, end: usize) {
        if let Some(owned) = &mut self.owned {
            owned.push_str(&input[start..end]);
            return;
        }
        match self.span {
            None => self.span = Some((start, end)),
            Some((from, to)) if start == end => self.span = Some((from, to)),
            Some((from, to)) if from == to => self.span = Some((start, end)),
            Some((from, to)) => {
                let mut owned = String::with_capacity(to - from + end - start);
                owned.push_str(&input[from..to]);
                owned.push_str(&input[start..end]);
                self.owned = Some(owned);
                self.span = None;
            }
        }
    }

    /// A literal quote from a doubled `""` — never part of the input as is.
    fn push_quote(&mut self, input: &str) {
        let owned = self.owned.get_or_insert_with(|| match self.span {
            Some((from, to)) => input[from..to].to_owned(),
            None => String::new(),
        });
        owned.push('"');
        self.span = None;
    }

    fn is_empty(&self) -> bool {
        match (&self.owned, self.span) {
            (Some(owned), _) => owned.is_empty(),
            (None, Some((from, to))) => from == to,
            (None, None) => true,
        }
    }

    fn take<'a>(&mut self, input: &'a str) -> Cow<'a, str> {
        match (self.owned.take(), self.span.take()) {
            (Some(owned), _) => Cow::Owned(owned),
            (None, Some((from, to))) => Cow::Borrowed(&input[from..to]),
            (None, None) => Cow::Borrowed(""),
        }
    }
}

/// Splits `input` into records.
pub(crate) fn scan(input: &str, delimiter: u8) -> Result<Vec<Record<'_>>, ScanError> {
    let bytes = input.as_bytes();
    let mut records = Vec::new();
    let mut fields: Vec<Cow<'_, str>> = Vec::new();
    let mut field = Field::default();
    let mut start = 0usize;
    let mut index = 0usize;
    let mut line = 1usize;
    let mut record_line = 1usize;
    let mut quoted = false;
    let mut quote_line = 1usize;
    let mut field_start = true;
    let mut started = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if quoted {
            if byte == b'"' {
                if bytes.get(index + 1) == Some(&b'"') {
                    field.push(input, start, index);
                    field.push_quote(input);
                    index += 2;
                    start = index;
                    continue;
                }
                field.push(input, start, index);
                quoted = false;
                index += 1;
                start = index;
                continue;
            }
            // Line breaks inside a quoted field count, they are not separators.
            if byte == b'\n' || (byte == b'\r' && bytes.get(index + 1) != Some(&b'\n')) {
                line += 1;
            }
            index += 1;
            continue;
        }
        if byte == delimiter {
            field.push(input, start, index);
            fields.push(field.take(input));
            index += 1;
            start = index;
            field_start = true;
            started = true;
        } else if byte == b'\n' || byte == b'\r' {
            let separator = if byte == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            field.push(input, start, index);
            // A line without any content is not a record.
            if started || !field.is_empty() || !fields.is_empty() {
                fields.push(field.take(input));
                // The next record most likely has as many fields: sized once
                // instead of grown from nothing for every record (point 45).
                let width = fields.len();
                records.push(Record {
                    fields: std::mem::replace(&mut fields, Vec::with_capacity(width)),
                    line: record_line,
                });
            } else {
                field.take(input);
            }
            index += separator;
            line += 1;
            record_line = line;
            start = index;
            field_start = true;
            started = false;
        } else if byte == b'"' && field_start {
            quoted = true;
            quote_line = line;
            index += 1;
            start = index;
            field_start = false;
            started = true;
        } else {
            field_start = false;
            started = true;
            index += 1;
        }
    }

    if quoted {
        return Err(ScanError {
            line: quote_line,
            message: "unterminated quoted field".to_owned(),
        });
    }
    if started || !field.is_empty() || !fields.is_empty() {
        field.push(input, start, input.len());
        fields.push(field.take(input));
        records.push(Record {
            fields: std::mem::take(&mut fields),
            line: record_line,
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields<'a>(records: &'a [Record<'a>]) -> Vec<Vec<&'a str>> {
        records
            .iter()
            .map(|record| record.fields.iter().map(|field| field.as_ref()).collect())
            .collect()
    }

    #[test]
    fn splits_plain_records() {
        let records = scan("a,b\nc,d\n", b',').unwrap();
        assert_eq!(fields(&records), vec![vec!["a", "b"], vec!["c", "d"]]);
        assert_eq!(
            records.iter().map(|r| r.line).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn keeps_the_last_record_without_a_final_break() {
        let records = scan("a,b\nc,d", b',').unwrap();
        assert_eq!(fields(&records), vec![vec!["a", "b"], vec!["c", "d"]]);
    }

    #[test]
    fn understands_quotes_and_doubled_quotes() {
        let records = scan("\"a,b\",\"say \"\"hi\"\"\",\"\"\n", b',').unwrap();
        assert_eq!(fields(&records), vec![vec!["a,b", "say \"hi\"", ""]]);
    }

    #[test]
    fn a_quoted_field_may_span_lines() {
        let input = "a,\"one\ntwo\"\nb,c\n";
        let records = scan(input, b',').unwrap();
        assert_eq!(
            fields(&records),
            vec![vec!["a", "one\ntwo"], vec!["b", "c"]]
        );
        // The second record starts on the third physical line.
        assert_eq!(records[1].line, 3);
    }

    #[test]
    fn counts_carriage_returns_and_crlf() {
        let records = scan("a,b\r\nc,d\re,f\n", b',').unwrap();
        assert_eq!(
            fields(&records),
            vec![vec!["a", "b"], vec!["c", "d"], vec!["e", "f"]]
        );
        assert_eq!(
            records.iter().map(|r| r.line).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn skips_empty_lines_but_keeps_empty_fields() {
        let records = scan("a\n\n,\n\nb\n", b',').unwrap();
        assert_eq!(fields(&records), vec![vec!["a"], vec!["", ""], vec!["b"]]);
        assert_eq!(
            records.iter().map(|r| r.line).collect::<Vec<_>>(),
            vec![1, 3, 5]
        );
    }

    #[test]
    fn keeps_a_quote_inside_an_unquoted_field() {
        let records = scan("a\"b,c\n", b',').unwrap();
        assert_eq!(fields(&records), vec![vec!["a\"b", "c"]]);
    }

    #[test]
    fn keeps_multi_byte_text_intact() {
        let records = scan("ä,é\n", b',').unwrap();
        assert_eq!(fields(&records), vec![vec!["ä", "é"]]);
    }

    #[test]
    fn other_delimiters_work() {
        let records = scan("a;b\tc\n", b';').unwrap();
        assert_eq!(fields(&records), vec![vec!["a", "b\tc"]]);
    }

    #[test]
    fn reports_an_unterminated_quote_with_its_line() {
        let error = scan("a,b\nc,\"open\n", b',').unwrap_err();
        assert_eq!(error.line, 2);
        assert_eq!(error.message, "unterminated quoted field");
    }

    #[test]
    fn an_empty_input_has_no_records() {
        assert!(scan("", b',').unwrap().is_empty());
    }
}
