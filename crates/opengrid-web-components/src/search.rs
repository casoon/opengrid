//! The search field (plan point 67): free text, or a filter written out.
//!
//! # One input method, not a second truth (F5, decided 2026-09-24)
//!
//! `country = DE and amount ≥ 10` is parsed into **exactly the filter entries
//! the filter row holds** — the same `FilterEntry` values, written into the same
//! fields, applied by the same code. The field empties afterwards. So there is
//! still one place a filter lives, and the chips, the view and the filter row
//! all show what the query said.
//!
//! # What does not parse is said, not guessed
//!
//! An expression that names no column, a column the grid does not show, an
//! operator the column's type does not take, or a value that is not one of the
//! column's — each is a sentence in the status line. Falling back to a free-text
//! search instead would look like it worked and show the wrong rows.
//!
//! # Free text is values, not display
//!
//! The prototype searched the *formatted* text. That would put formatting into a
//! query, which phase E ruled out: sorting is binary (S4), a decimal is exact
//! (S8), and a search for "31.12." does not find a date. It is `contains` on the
//! values of the shown text columns, or-ed — and case-sensitive, because V1's
//! string operators are (S5).

use opengrid_query::{CmpOp, FilterExpr};
use opengrid_types::{DataType, FieldName, Schema};

use crate::grid::{FilterEntry, FilterOp};

/// The operators the language knows, longest first so `>=` wins over `>`.
const OPERATORS: &[(&str, &str)] = &[
    ("!=", "ne"),
    (">=", "gte"),
    ("<=", "lte"),
    ("\u{2260}", "ne"),
    ("\u{2265}", "gte"),
    ("\u{2264}", "lte"),
    ("=", "eq"),
    (">", "gt"),
    ("<", "lt"),
    ("~", "contains"),
    ("^", "starts_with"),
];

/// Why an expression did not become a filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The column is not one the grid shows.
    UnknownColumn(String),
    /// A clause has a column and an operator but nothing after it.
    MissingValue(String),
    /// The column's type does not take this operator (`~` on a number).
    WrongOperator { column: String, operator: String },
    /// The value is not one of the column's (`amount > abc`).
    WrongValue { column: String, value: String },
}

/// One clause, split but not yet checked: `column`, operator symbol, value.
fn split_clause(clause: &str) -> Option<(&str, &str, &str)> {
    let clause = clause.trim();
    let start = clause
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map(|(at, _)| at)
        .unwrap_or(clause.len());
    let column = &clause[..start];
    if column.is_empty() {
        return None;
    }
    let rest = clause[start..].trim_start();
    let (symbol, _) = OPERATORS
        .iter()
        .find(|(symbol, _)| rest.starts_with(symbol))?;
    let value = rest[symbol.len()..].trim();
    Some((column, symbol, value))
}

/// The clauses of an input, split on the joining word (`and`, or the page's
/// translation of it — both are accepted, so an English habit still works on a
/// German page).
fn clauses<'a>(input: &'a str, and_word: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let words: Vec<String> = [and_word.trim().to_lowercase(), "and".to_owned()]
        .into_iter()
        .filter(|word| !word.is_empty())
        .collect();
    let mut from = 0;
    let mut at = 0;
    while at < input.len() {
        let mut matched = None;
        for word in &words {
            let needle = format!(" {word} ");
            // Compared on the original, not on a lower-cased copy: lower-casing
            // can change a character's length (`İ`), and a byte position of the
            // input would then land mid-character in the copy.
            if input
                .get(at..at + needle.len())
                .is_some_and(|here| here.to_lowercase() == needle)
            {
                matched = Some(needle.len());
                break;
            }
        }
        match matched {
            Some(length) => {
                out.push(&input[from..at]);
                at += length;
                from = at;
            }
            None => {
                at += input[at..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    out.push(&input[from..]);
    out
}

/// Whether the input reads as a filter expression rather than free text: it
/// starts with a word and an operator. What the field shows as its hint
/// ("Query · Enter") depends on this, before anything is parsed in full.
///
/// The word need **not** be a column (point 72): `colour = red` is a filter
/// with a typo, and parsing it says "colour is not a column of this grid". A
/// check against the columns here would send it to the free-text search
/// instead — silently, and with no rows to show for it (E30).
pub fn looks_like_query(input: &str) -> bool {
    split_clause(input).is_some()
}

/// Parses an expression into filter entries, checked against the schema.
pub fn parse(input: &str, and_word: &str, schema: &Schema) -> Result<Vec<FilterEntry>, Problem> {
    let mut out: Vec<FilterEntry> = Vec::new();
    for clause in clauses(input, and_word) {
        let clause = clause.trim();
        let Some((column, symbol, value)) = split_clause(clause) else {
            let column = clause.split_whitespace().next().unwrap_or(clause);
            return Err(
                if schema
                    .fields()
                    .iter()
                    .any(|field| field.name.as_str() == column)
                {
                    Problem::MissingValue(column.to_owned())
                } else {
                    Problem::UnknownColumn(column.to_owned())
                },
            );
        };
        let Some(field) = schema
            .fields()
            .iter()
            .find(|field| field.name.as_str() == column)
        else {
            return Err(Problem::UnknownColumn(column.to_owned()));
        };
        if value.is_empty() {
            return Err(Problem::MissingValue(column.to_owned()));
        }
        let token = OPERATORS
            .iter()
            .find(|(candidate, _)| *candidate == symbol)
            .map(|(_, token)| *token)
            .unwrap_or("eq");
        if !crate::grid::operators_for(field.data_type, true).contains(&token) {
            return Err(Problem::WrongOperator {
                column: column.to_owned(),
                operator: symbol.to_owned(),
            });
        }
        if crate::grid::literal(value, field.data_type).is_none() {
            return Err(Problem::WrongValue {
                column: column.to_owned(),
                value: value.to_owned(),
            });
        }
        let Some(op) = FilterOp::parse(token) else {
            continue;
        };
        // One entry per column, like the filter row: the last clause on a
        // column is the one that stands.
        out.retain(|entry| entry.column != column);
        out.push(FilterEntry {
            column: column.to_owned(),
            op,
            value: value.to_owned(),
        });
    }
    Ok(out)
}

/// The columns to offer while the last clause is still a bare word — a prefix
/// of a shown column and nothing after it yet.
pub fn suggestions(input: &str, and_word: &str, columns: &[String]) -> Vec<String> {
    let last = clauses(input, and_word).pop().unwrap_or("").trim_start();
    if last.is_empty() || last.contains(char::is_whitespace) {
        return Vec::new();
    }
    if !last.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Vec::new();
    }
    columns
        .iter()
        .filter(|name| name.starts_with(last) && name.as_str() != last)
        .cloned()
        .collect()
}

/// The input with its last bare word replaced by `column` and a space — what
/// taking a suggestion does.
pub fn complete(input: &str, and_word: &str, column: &str) -> String {
    let last = clauses(input, and_word).pop().unwrap_or("");
    let keep = input.len() - last.trim_start().len();
    format!("{}{column} ", &input[..keep])
}

/// Free text as a filter: `contains` on every shown text column, or-ed.
///
/// On values, not display (phase E), and case-sensitive (S5).
pub fn free_text(text: &str, schema: &Schema) -> Option<FilterExpr> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut parts: Vec<FilterExpr> = schema
        .fields()
        .iter()
        .filter(|field| field.data_type == DataType::Utf8)
        .filter_map(|field| FieldName::new(field.name.as_str()).ok())
        .map(|field| FilterExpr::Cmp {
            field,
            op: CmpOp::Contains,
            value: serde_json::Value::String(text.to_owned()),
        })
        .collect();
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(FilterExpr::Or(parts)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::Field;

    fn schema() -> Schema {
        Schema::new(vec![
            Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("country").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("amount").unwrap(), DataType::Int64),
            Field::new(FieldName::new("ordered_on").unwrap(), DataType::Date),
        ])
    }

    fn columns() -> Vec<String> {
        ["customer", "country", "amount", "ordered_on"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn entry(column: &str, op: &str, value: &str) -> FilterEntry {
        FilterEntry {
            column: column.to_owned(),
            op: FilterOp::parse(op).unwrap(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn the_prototype_example_becomes_two_filter_row_entries() {
        assert_eq!(
            parse("country = DE und amount \u{2265} 10", "und", &schema()),
            Ok(vec![
                entry("country", "eq", "DE"),
                entry("amount", "gte", "10")
            ])
        );
    }

    #[test]
    fn the_english_word_works_on_a_translated_page_too() {
        assert_eq!(
            parse("country = DE and amount >= 10", "und", &schema()),
            Ok(vec![
                entry("country", "eq", "DE"),
                entry("amount", "gte", "10")
            ])
        );
    }

    #[test]
    fn an_operator_needs_no_spaces() {
        assert_eq!(
            parse("amount>=10", "and", &schema()),
            Ok(vec![entry("amount", "gte", "10")])
        );
        assert_eq!(
            parse("customer~lph", "and", &schema()),
            Ok(vec![entry("customer", "contains", "lph")])
        );
    }

    #[test]
    fn three_clauses() {
        let got = parse(
            "country != US and amount < 5 and customer ^ Al",
            "and",
            &schema(),
        )
        .unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[2], entry("customer", "starts_with", "Al"));
    }

    #[test]
    fn what_does_not_parse_is_named() {
        assert_eq!(
            parse("", "and", &schema()),
            Err(Problem::UnknownColumn(String::new()))
        );
        assert_eq!(
            parse("country", "and", &schema()),
            Err(Problem::MissingValue("country".to_owned()))
        );
        assert_eq!(
            parse("country =", "and", &schema()),
            Err(Problem::MissingValue("country".to_owned()))
        );
        assert_eq!(
            parse("nope = 1", "and", &schema()),
            Err(Problem::UnknownColumn("nope".to_owned()))
        );
        assert_eq!(
            parse("amount ~ 1", "and", &schema()),
            Err(Problem::WrongOperator {
                column: "amount".to_owned(),
                operator: "~".to_owned()
            })
        );
        assert_eq!(
            parse("amount > abc", "and", &schema()),
            Err(Problem::WrongValue {
                column: "amount".to_owned(),
                value: "abc".to_owned()
            })
        );
    }

    #[test]
    fn the_last_clause_on_a_column_stands() {
        assert_eq!(
            parse("amount > 1 and amount < 5", "and", &schema()),
            Ok(vec![entry("amount", "lt", "5")])
        );
    }

    #[test]
    fn a_query_is_told_from_free_text_by_its_start() {
        assert!(looks_like_query("country = DE"));
        assert!(looks_like_query("amount>"));
        assert!(!looks_like_query("Alpha"));
        assert!(!looks_like_query("Alpha Beta"));
        // A typo in the column is still a filter — named by the parser, never
        // quietly searched as text (point 72, E30).
        assert!(looks_like_query("nope = 1"));
    }

    #[test]
    fn suggestions_follow_the_last_bare_word() {
        assert_eq!(suggestions("c", "and", &columns()), ["customer", "country"]);
        assert_eq!(
            suggestions("country = DE and am", "and", &columns()),
            ["amount"]
        );
        // Once an operator follows, the column is chosen.
        assert!(suggestions("amount >", "and", &columns()).is_empty());
        assert!(suggestions("amount", "and", &columns()).is_empty());
        assert_eq!(
            complete("country = DE and am", "and", "amount"),
            "country = DE and amount "
        );
    }

    /// A character whose lower case is longer than itself must not throw the
    /// clause split off its character boundaries.
    #[test]
    fn a_character_that_grows_when_lower_cased_does_not_break_the_split() {
        assert_eq!(
            parse("customer = İstanbul and amount > 1", "and", &schema()),
            Ok(vec![
                entry("customer", "eq", "İstanbul"),
                entry("amount", "gt", "1")
            ])
        );
        assert_eq!(
            parse("customer = X AND amount > 1", "and", &schema()).map(|e| e.len()),
            Ok(2)
        );
    }

    #[test]
    fn free_text_is_contains_on_every_text_column() {
        let Some(FilterExpr::Or(parts)) = free_text("Al", &schema()) else {
            panic!("two text columns are an or");
        };
        assert_eq!(parts.len(), 2);
        assert!(free_text("  ", &schema()).is_none());
    }
}
