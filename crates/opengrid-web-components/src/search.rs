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
//! # `or` and parentheses (issue #324)
//!
//! ```text
//! expression = term   { OR  term }
//! term       = factor { AND factor }
//! factor     = "(" expression ")" | condition
//! condition  = column operator value
//! ```
//!
//! `and` binds tighter than `or`. An expression with an `or` cannot live in
//! the filter row — one condition per column, and-ed — so it becomes the
//! grid's nested filter, a tree in the wire form
//! ([`opengrid_grid::filter_tree`]). One with `and` alone still goes to the
//! row, exactly as before: [`Parsed`] says which.
//!
//! A value runs to the next joining word, to the parenthesis that closes its
//! group, or to the end; parentheses that pair up inside it are its own, and
//! in double quotes it is taken whole.
//!
//! # What does not parse is said, not guessed
//!
//! An expression that names no column, a column the grid does not show, an
//! operator the column's type does not take, a value that is not one of the
//! column's, a parenthesis without its partner — each is a sentence in the
//! status line. Falling back to a free-text search instead would look like it
//! worked and show the wrong rows.
//!
//! # Free text is values, not display
//!
//! The prototype searched the *formatted* text. That would put formatting into a
//! query, which phase E ruled out: sorting is binary (S4), a decimal is exact
//! (S8), and a search for "31.12." does not find a date. It is `contains` on the
//! values of the shown text columns, or-ed — and case-sensitive, because V1's
//! string operators are (S5).

use opengrid_grid::filter_tree;
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

/// How deep parentheses nest before the expression is refused: far beyond
/// what a person writes or a source takes, and well inside the stack — a
/// pasted wall of `(` must not end in a trap.
const MAX_DEPTH: usize = 64;

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
    /// A `(` without its `)`.
    Unclosed,
    /// A `)` without its `(`.
    Unopened,
    /// Nothing where a condition belongs: `()`.
    MissingCondition,
    /// Text where only a joining word can stand: `(a = 1) b = 2`. The text.
    MissingJoin(String),
}

/// What an expression became.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    /// `and` alone: the filter row's entries, one per column.
    Row(Vec<FilterEntry>),
    /// With an `or`: the nested filter.
    Tree(FilterExpr),
}

/// The start of a condition, split but not yet checked: `column`, the
/// operator's symbol, and where the value begins in `text`.
fn head(text: &str) -> Option<(&str, &'static str, usize)> {
    let start = text
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map(|(at, _)| at)
        .unwrap_or(text.len());
    let column = &text[..start];
    if column.is_empty() {
        return None;
    }
    let rest = text[start..].trim_start();
    let (symbol, _) = OPERATORS
        .iter()
        .find(|(symbol, _)| rest.starts_with(symbol))?;
    Some((column, *symbol, text.len() - rest.len() + symbol.len()))
}

/// The joining words, lower-cased, each with whether it is an `or`: the
/// page's translations and the English ones — both are accepted, so an
/// English habit still works on a German page.
///
/// Without an `or_word` nothing joins with `or`: the pivot's field, where
/// `or` stays a word of a value, as it was.
fn joins(and_word: &str, or_word: Option<&str>) -> Vec<(String, bool)> {
    [(and_word, false), ("and", false)]
        .into_iter()
        .chain(
            or_word
                .into_iter()
                .flat_map(|word| [(word, true), ("or", true)]),
        )
        .map(|(word, any)| (word.trim().to_lowercase(), any))
        .filter(|(word, _)| !word.is_empty())
        .collect()
}

/// The joining word that starts at `at`, if one does: whether it is an `or`,
/// and its length. It ends at whitespace or an opening parenthesis.
///
/// Compared on the original, not on a lower-cased copy: lower-casing can
/// change a character's length (`İ`), and a byte position of the input would
/// then land mid-character in the copy.
fn join_at(input: &str, at: usize, joins: &[(String, bool)]) -> Option<(bool, usize)> {
    joins.iter().find_map(|(word, any)| {
        let end = at + word.len();
        let here = input.get(at..end)?;
        let after = input[end..].chars().next()?;
        (here.to_lowercase() == *word && (after.is_whitespace() || after == '('))
            .then_some((*any, word.len()))
    })
}

/// A parsed expression before it is decided where it lives.
enum Node {
    /// A condition, as the filter row's entry and as the typed comparison.
    Leaf(FilterEntry, FilterExpr),
    /// Two or more members; `true` is `or`.
    Group(bool, Vec<Node>),
}

impl Node {
    fn has_or(&self) -> bool {
        match self {
            Node::Leaf(..) => false,
            Node::Group(any, members) => *any || members.iter().any(Node::has_or),
        }
    }

    /// The conditions in the order written — what an expression without `or`
    /// comes down to.
    fn entries(self, out: &mut Vec<FilterEntry>) {
        match self {
            Node::Leaf(entry, _) => {
                // One entry per column, like the filter row: the last clause
                // on a column is the one that stands.
                out.retain(|other| other.column != entry.column);
                out.push(entry);
            }
            Node::Group(_, members) => members.into_iter().for_each(|member| member.entries(out)),
        }
    }

    fn tree(self) -> Option<FilterExpr> {
        match self {
            Node::Leaf(_, filter) => Some(filter),
            Node::Group(any, members) => {
                filter_tree::group(any, members.into_iter().filter_map(Node::tree).collect())
            }
        }
    }
}

struct Parser<'a> {
    input: &'a str,
    at: usize,
    depth: usize,
    joins: Vec<(String, bool)>,
    schema: &'a Schema,
}

impl<'a> Parser<'a> {
    fn skip_whitespace(&mut self) {
        self.at = self.input.len() - self.input[self.at..].trim_start().len();
    }

    fn peek(&self) -> Option<char> {
        self.input[self.at..].chars().next()
    }

    /// `term { OR term }` when `any`, `factor { AND factor }` otherwise.
    fn joined(&mut self, any: bool) -> Result<Node, Problem> {
        let mut members = Vec::new();
        loop {
            members.push(if any {
                self.joined(false)?
            } else {
                self.factor()?
            });
            self.skip_whitespace();
            match join_at(self.input, self.at, &self.joins) {
                Some((kind, length)) if kind == any => self.at += length,
                _ => break,
            }
        }
        Ok(if members.len() == 1 {
            members.remove(0)
        } else {
            Node::Group(any, members)
        })
    }

    fn factor(&mut self) -> Result<Node, Problem> {
        self.skip_whitespace();
        if self.peek() != Some('(') {
            return self.condition();
        }
        self.at += 1;
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Problem::Unclosed);
        }
        let inner = self.joined(true)?;
        self.depth -= 1;
        match self.peek() {
            Some(')') => {
                self.at += 1;
                Ok(inner)
            }
            None => Err(Problem::Unclosed),
            Some(_) => Err(Problem::MissingJoin(self.input[self.at..].to_owned())),
        }
    }

    fn condition(&mut self) -> Result<Node, Problem> {
        let input = self.input;
        let text = &input[self.at..];
        let Some((column, symbol, value_at)) = head(text) else {
            let word = text
                .split(|c: char| c.is_whitespace() || c == ')')
                .next()
                .unwrap_or("");
            return Err(if word.is_empty() {
                Problem::MissingCondition
            } else if self.schema.field(word).is_some() {
                Problem::MissingValue(word.to_owned())
            } else {
                Problem::UnknownColumn(word.to_owned())
            });
        };
        let value = self.value(self.at + value_at);
        let Some(field) = self.schema.field(column) else {
            return Err(Problem::UnknownColumn(column.to_owned()));
        };
        if value.is_empty() {
            return Err(Problem::MissingValue(column.to_owned()));
        }
        let token = OPERATORS
            .iter()
            .find(|(candidate, _)| *candidate == symbol)
            .map_or("eq", |(_, token)| *token);
        let op = FilterOp::parse(token)
            .filter(|_| crate::grid::operators_for(field.data_type, true).contains(&token));
        let Some(op) = op else {
            return Err(Problem::WrongOperator {
                column: column.to_owned(),
                operator: symbol.to_owned(),
            });
        };
        let entry = FilterEntry {
            column: column.to_owned(),
            op,
            value: value.to_owned(),
        };
        match filter_tree::condition(&entry, self.schema) {
            Ok(Some(filter)) => Ok(Node::Leaf(entry, filter)),
            _ => Err(Problem::WrongValue {
                column: entry.column,
                value: entry.value,
            }),
        }
    }

    /// The value that starts at `from`, and the parser moved past it.
    ///
    /// In double quotes it is what stands between them. Otherwise it runs to
    /// the next joining word, to a `)` that has no `(` inside the value — the
    /// one that closes the group, or one too many, which the caller names —
    /// or to the end.
    fn value(&mut self, from: usize) -> &'a str {
        let input = self.input;
        let from = input.len() - input[from..].trim_start().len();
        if let Some(quoted) = input[from..].strip_prefix('"')
            && let Some(length) = quoted.find('"')
        {
            self.at = from + length + 2;
            return &quoted[..length];
        }
        let mut open = 0usize;
        let mut end = input.len();
        for (offset, c) in input[from..].char_indices() {
            let at = from + offset;
            match c {
                '(' => open += 1,
                ')' if open > 0 => open -= 1,
                ')' => {
                    end = at;
                    break;
                }
                c if c.is_whitespace()
                    && join_at(input, at + c.len_utf8(), &self.joins).is_some() =>
                {
                    end = at;
                    break;
                }
                _ => {}
            }
        }
        self.at = end;
        input[from..end].trim()
    }
}

/// The input without the parentheses and whitespace it starts with.
fn unopened(input: &str) -> &str {
    input.trim_start_matches(|c: char| c == '(' || c.is_whitespace())
}

/// Whether the input reads as a filter expression rather than free text: it
/// starts with a word and an operator — after any opening parentheses. What
/// the field shows as its hint ("Query · Enter") depends on this, before
/// anything is parsed in full.
///
/// The word need **not** be a column (point 72): `colour = red` is a filter
/// with a typo, and parsing it says "colour is not a column of this grid". A
/// check against the columns here would send it to the free-text search
/// instead — silently, and with no rows to show for it (E30).
pub fn looks_like_query(input: &str) -> bool {
    head(unopened(input)).is_some()
}

/// Parses an expression, checked against the schema: the filter row's entries
/// when it joins with `and` alone, the nested filter when an `or` is in it —
/// which, without an `or_word`, it never is.
pub fn parse(
    input: &str,
    and_word: &str,
    or_word: Option<&str>,
    schema: &Schema,
) -> Result<Parsed, Problem> {
    let mut parser = Parser {
        input,
        at: 0,
        depth: 0,
        joins: joins(and_word, or_word),
        schema,
    };
    let node = parser.joined(true)?;
    match parser.peek() {
        None => {}
        Some(')') => return Err(Problem::Unopened),
        Some(_) => return Err(Problem::MissingJoin(input[parser.at..].to_owned())),
    }
    if node.has_or() {
        // Every leaf is a condition, so a tree with an `or` has members.
        return node
            .tree()
            .map(Parsed::Tree)
            .ok_or(Problem::MissingCondition);
    }
    let mut entries = Vec::new();
    node.entries(&mut entries);
    Ok(Parsed::Row(entries))
}

/// The word being typed: what follows the last joining word and the
/// parentheses opened after it.
fn last_word<'a>(input: &'a str, and_word: &str, or_word: &str) -> &'a str {
    let joins = joins(and_word, Some(or_word));
    let mut from = 0;
    for (at, c) in input.char_indices() {
        if c.is_whitespace()
            && let Some((_, length)) = join_at(input, at + c.len_utf8(), &joins)
        {
            from = at + c.len_utf8() + length;
        }
    }
    unopened(&input[from..])
}

/// The columns to offer while the last clause is still a bare word — a prefix
/// of a shown column and nothing after it yet.
pub fn suggestions(input: &str, and_word: &str, or_word: &str, columns: &[String]) -> Vec<String> {
    let last = last_word(input, and_word, or_word);
    if last.is_empty() || !last.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
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
pub fn complete(input: &str, and_word: &str, or_word: &str, column: &str) -> String {
    let keep = input.len() - last_word(input, and_word, or_word).len();
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
            value: opengrid_json::Json::String(text.to_owned()),
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
    use opengrid_json::{Json, ToJson, json};
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

    /// An expression with the English words, as the row's entries.
    fn row(input: &str) -> Result<Vec<FilterEntry>, Problem> {
        match parse(input, "and", Some("or"), &schema())? {
            Parsed::Row(entries) => Ok(entries),
            Parsed::Tree(tree) => panic!("{input:?} is a tree: {}", tree.to_json()),
        }
    }

    /// An expression with the English words, as the nested filter's JSON.
    fn tree(input: &str) -> Json {
        match parse(input, "and", Some("or"), &schema()) {
            Ok(Parsed::Tree(tree)) => tree.to_json(),
            other => panic!("{input:?} is no tree: {other:?}"),
        }
    }

    fn problem(input: &str) -> Problem {
        parse(input, "and", Some("or"), &schema()).expect_err(input)
    }

    fn is(column: &str, op: &str, value: impl Into<Json>) -> Json {
        json!({ "field": column, "op": op, "value": value.into() })
    }

    #[test]
    fn the_prototype_example_becomes_two_filter_row_entries() {
        assert_eq!(
            parse(
                "country = DE und amount \u{2265} 10",
                "und",
                Some("oder"),
                &schema()
            ),
            Ok(Parsed::Row(vec![
                entry("country", "eq", "DE"),
                entry("amount", "gte", "10")
            ]))
        );
    }

    #[test]
    fn the_english_word_works_on_a_translated_page_too() {
        assert_eq!(
            parse(
                "country = DE and amount >= 10",
                "und",
                Some("oder"),
                &schema()
            ),
            Ok(Parsed::Row(vec![
                entry("country", "eq", "DE"),
                entry("amount", "gte", "10")
            ]))
        );
        let german = parse(
            "country = DE oder amount >= 10",
            "und",
            Some("oder"),
            &schema(),
        );
        let english = parse(
            "country = DE or amount >= 10",
            "und",
            Some("oder"),
            &schema(),
        );
        assert!(matches!(german, Ok(Parsed::Tree(_))));
        assert_eq!(german, english);
    }

    /// The pivot's field joins with `and` alone: there `or` is a word of a
    /// value, as it always was, and nothing becomes a tree.
    #[test]
    fn without_an_or_word_nothing_joins_with_or() {
        assert_eq!(
            parse(
                "customer = this or that and amount > 1",
                "and",
                None,
                &schema()
            ),
            Ok(Parsed::Row(vec![
                entry("customer", "eq", "this or that"),
                entry("amount", "gt", "1")
            ]))
        );
    }

    #[test]
    fn an_operator_needs_no_spaces() {
        assert_eq!(row("amount>=10"), Ok(vec![entry("amount", "gte", "10")]));
        assert_eq!(
            row("customer~lph"),
            Ok(vec![entry("customer", "contains", "lph")])
        );
    }

    #[test]
    fn three_clauses() {
        let got = row("country != US and amount < 5 and customer ^ Al").unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[2], entry("customer", "starts_with", "Al"));
    }

    #[test]
    fn what_does_not_parse_is_named() {
        assert_eq!(problem(""), Problem::MissingCondition);
        assert_eq!(
            problem("country"),
            Problem::MissingValue("country".to_owned())
        );
        assert_eq!(
            problem("country ="),
            Problem::MissingValue("country".to_owned())
        );
        assert_eq!(
            problem("nope = 1"),
            Problem::UnknownColumn("nope".to_owned())
        );
        assert_eq!(
            problem("amount ~ 1"),
            Problem::WrongOperator {
                column: "amount".to_owned(),
                operator: "~".to_owned()
            }
        );
        assert_eq!(
            problem("amount > abc"),
            Problem::WrongValue {
                column: "amount".to_owned(),
                value: "abc".to_owned()
            }
        );
        // In the second half of an `or`, and inside parentheses, too.
        assert_eq!(
            problem("country = DE or nope = 1"),
            Problem::UnknownColumn("nope".to_owned())
        );
        assert_eq!(
            problem("(country = DE or amount > abc)"),
            Problem::WrongValue {
                column: "amount".to_owned(),
                value: "abc".to_owned()
            }
        );
    }

    #[test]
    fn the_last_clause_on_a_column_stands() {
        assert_eq!(
            row("amount > 1 and amount < 5"),
            Ok(vec![entry("amount", "lt", "5")])
        );
    }

    #[test]
    fn an_or_is_the_nested_filter_with_typed_values() {
        assert_eq!(
            tree("country = DE or amount \u{2265} 1000"),
            json!({ "or": [is("country", "eq", "DE"), is("amount", "gte", 1000)] })
        );
        // Both conditions on one column stand: a tree holds what the row cannot.
        assert_eq!(
            tree("amount < 5 or amount > 100"),
            json!({ "or": [is("amount", "lt", 5), is("amount", "gt", 100)] })
        );
    }

    #[test]
    fn and_binds_tighter_than_or() {
        let expected = json!({ "or": [
            is("country", "eq", "DE"),
            { "and": [is("amount", "gt", 1), is("customer", "contains", "Al")] },
        ] });
        assert_eq!(
            tree("country = DE or amount > 1 and customer ~ Al"),
            expected
        );
        // The same, said with parentheses.
        assert_eq!(
            tree("country = DE or (amount > 1 and customer ~ Al)"),
            expected
        );
        // And the other way round, whichever side the `and` stands on.
        assert_eq!(
            tree("amount > 1 and customer ~ Al or country = DE"),
            json!({ "or": [
                { "and": [is("amount", "gt", 1), is("customer", "contains", "Al")] },
                is("country", "eq", "DE"),
            ] })
        );
    }

    #[test]
    fn parentheses_group_against_the_precedence_and_nest() {
        assert_eq!(
            tree("(country = DE or amount > 1) and customer ~ Al"),
            json!({ "and": [
                { "or": [is("country", "eq", "DE"), is("amount", "gt", 1)] },
                is("customer", "contains", "Al"),
            ] })
        );
        assert_eq!(
            tree("((country = DE or (amount > 1 and (customer ~ Al or customer ~ Be))))"),
            json!({ "or": [
                is("country", "eq", "DE"),
                { "and": [
                    is("amount", "gt", 1),
                    { "or": [is("customer", "contains", "Al"), is("customer", "contains", "Be")] },
                ] },
            ] })
        );
        // Groups of the same kind are one group.
        assert_eq!(
            tree("country = DE or (amount > 1 or customer ~ Al)"),
            json!({ "or": [
                is("country", "eq", "DE"),
                is("amount", "gt", 1),
                is("customer", "contains", "Al"),
            ] })
        );
        // No spaces are needed around a parenthesis, and the words read in any case.
        assert_eq!(
            tree("(country=DE)OR(amount>1)"),
            json!({ "or": [is("country", "eq", "DE"), is("amount", "gt", 1)] })
        );
    }

    /// Parentheses that group nothing change nothing: `and` alone is the row's.
    #[test]
    fn parentheses_around_ands_still_go_to_the_filter_row() {
        assert_eq!(
            row("(country = DE and (amount >= 10))"),
            Ok(vec![
                entry("country", "eq", "DE"),
                entry("amount", "gte", "10")
            ])
        );
        assert_eq!(
            row("(country = DE)"),
            Ok(vec![entry("country", "eq", "DE")])
        );
    }

    #[test]
    fn a_parenthesis_without_its_partner_is_named() {
        assert_eq!(problem("(country = DE or amount > 1"), Problem::Unclosed);
        assert_eq!(problem("country = DE or (amount > 1"), Problem::Unclosed);
        assert_eq!(problem("((country = DE)"), Problem::Unclosed);
        assert_eq!(problem("country = DE or amount > 1)"), Problem::Unopened);
        assert_eq!(problem("(country = DE))"), Problem::Unopened);
        assert_eq!(problem("()"), Problem::MissingCondition);
        assert_eq!(problem("(country = DE or )"), Problem::MissingCondition);
        assert_eq!(
            problem("(country = DE) amount > 1"),
            Problem::MissingJoin("amount > 1".to_owned())
        );
        assert_eq!(
            problem("country = \"DE\" x or amount > 1"),
            Problem::MissingJoin("x or amount > 1".to_owned())
        );
        // A wall of parentheses is refused, not followed to the end of the stack.
        assert_eq!(problem(&"(".repeat(100_000)), Problem::Unclosed);
    }

    #[test]
    fn a_value_keeps_its_own_parentheses_and_quotes_keep_everything() {
        assert_eq!(
            row("customer = Acme (EU) and amount > 1"),
            Ok(vec![
                entry("customer", "eq", "Acme (EU)"),
                entry("amount", "gt", "1")
            ])
        );
        assert_eq!(
            tree("(customer = Acme (EU) or amount > 1)"),
            json!({ "or": [is("customer", "eq", "Acme (EU)"), is("amount", "gt", 1)] })
        );
        assert_eq!(
            tree("customer = \"Rock and Roll\" or customer ~ \":) or (\""),
            json!({ "or": [
                is("customer", "eq", "Rock and Roll"),
                is("customer", "contains", ":) or ("),
            ] })
        );
        // A quote that is not closed is a character, as it always was.
        assert_eq!(
            row("customer ~ 5\" pipe"),
            Ok(vec![entry("customer", "contains", "5\" pipe")])
        );
        assert_eq!(
            row("customer = \"x"),
            Ok(vec![entry("customer", "eq", "\"x")])
        );
        assert_eq!(
            problem("customer = \"\""),
            Problem::MissingValue("customer".to_owned())
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
        // Parentheses before the first condition do not make it a search.
        assert!(looks_like_query("(country = DE or amount > 1)"));
        assert!(looks_like_query("( (amount>"));
        assert!(!looks_like_query("(Alpha)"));
    }

    #[test]
    fn suggestions_follow_the_last_bare_word() {
        let offered = |input: &str| suggestions(input, "und", "oder", &columns());
        assert_eq!(offered("c"), ["customer", "country"]);
        assert_eq!(offered("country = DE and am"), ["amount"]);
        assert_eq!(offered("country = DE or am"), ["amount"]);
        assert_eq!(offered("country = DE ODER (am"), ["amount"]);
        assert_eq!(offered("((c"), ["customer", "country"]);
        // Once an operator follows, the column is chosen.
        assert!(offered("amount >").is_empty());
        assert!(offered("amount").is_empty());
        assert!(offered("country = DE or ").is_empty());
        assert_eq!(
            complete("country = DE and am", "and", "or", "amount"),
            "country = DE and amount "
        );
        assert_eq!(
            complete("country = DE or (am", "and", "or", "amount"),
            "country = DE or (amount "
        );
    }

    /// A character whose lower case is longer than itself must not throw the
    /// clause split off its character boundaries.
    #[test]
    fn a_character_that_grows_when_lower_cased_does_not_break_the_split() {
        assert_eq!(
            row("customer = İstanbul and amount > 1"),
            Ok(vec![
                entry("customer", "eq", "İstanbul"),
                entry("amount", "gt", "1")
            ])
        );
        assert_eq!(row("customer = X AND amount > 1").map(|e| e.len()), Ok(2));
        assert_eq!(
            tree("customer = İİ or amount > 1"),
            json!({ "or": [is("customer", "eq", "İİ"), is("amount", "gt", 1)] })
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
