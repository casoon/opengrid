//! A text column with few distinct texts is stored as a dictionary (issue #451). What a query
//! answers must not depend on that: the same texts as a dictionary and as a flat column filter,
//! sort, group and aggregate alike, and as the plain texts do. The expected answers are worked out
//! here from a `Vec<Option<String>>`, not by the engine, and the flat twin of a table is made by
//! `Table::slice`, which gathers into flat buffers.

use std::collections::BTreeMap;

use opengrid_columns::{Strs, Values};
use opengrid_engine::Table;
use opengrid_engine::execute::execute;
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema, Value};

fn schema() -> Schema {
    Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        Field::new(FieldName::new("t").unwrap(), DataType::Utf8),
    ])
}

/// A few texts with ties and a NULL among them: the empty text, text with a quote, multi-byte text,
/// texts that are the start of others, and the texts a filter looks for.
fn few() -> Vec<Option<String>> {
    let texts = [
        "DE",
        "FR",
        "",
        "Dé",
        "DEU",
        "d",
        "日本語",
        "say \"hi\"",
        "Alpha",
        "alpha",
        "ß",
    ];
    (0..600)
        .map(|row| {
            if row % 13 == 5 {
                None
            } else {
                Some(texts[(row * 7 + row / 11) % texts.len()].to_owned())
            }
        })
        .collect()
}

fn table(texts: &[Option<String>]) -> Table {
    let ids: Vec<Value> = (0..texts.len() as i64).map(Value::Int64).collect();
    let cells: Vec<Value> = texts
        .iter()
        .map(|text| text.clone().map_or(Value::Null, Value::Utf8))
        .collect();
    Table::from_values(&schema(), &[ids, cells]).unwrap()
}

fn is_dictionary(table: &Table) -> bool {
    matches!(table.column_at(1).values(), Values::Utf8(Strs::Dict { .. }))
}

fn run(table: &Table, json: &str) -> Vec<Vec<Value>> {
    let query: Query = opengrid_json::from_str(json).expect("a well-formed query");
    let query = query
        .validate(&schema(), &Limits::default())
        .expect("a valid query");
    let result = execute(table, &query).expect("the engine answers");
    (0..result.table.num_rows())
        .map(|row| {
            (0..result.table.schema().len())
                .map(|column| result.table.column_at(column).value(row))
                .collect()
        })
        .collect()
}

fn ids(rows: &[Vec<Value>]) -> Vec<i64> {
    rows.iter()
        .map(|row| match row[0] {
            Value::Int64(id) => id,
            ref other => panic!("an id, got {other:?}"),
        })
        .collect()
}

/// The ids of the rows whose text satisfies `keep`, ascending.
fn expected(texts: &[Option<String>], keep: impl Fn(&str) -> bool) -> Vec<i64> {
    texts
        .iter()
        .enumerate()
        .filter(|(_, text)| text.as_deref().is_some_and(&keep))
        .map(|(row, _)| row as i64)
        .collect()
}

fn filtered(table: &Table, op: &str, value: &str) -> Vec<i64> {
    ids(&run(
        table,
        &format!(
            r#"{{"source":"t","select":["id"],"filter":{{"field":"t","op":"{op}","value":{value}}},"sort":[{{"field":"id"}}]}}"#
        ),
    ))
}

#[test]
fn the_few_texts_are_a_dictionary_and_their_twin_is_flat() {
    let texts = few();
    let dictionary = table(&texts);
    let flat = dictionary.slice(0, dictionary.num_rows());
    assert!(is_dictionary(&dictionary));
    assert!(!is_dictionary(&flat));
    assert!(dictionary.byte_size() < flat.byte_size());
    assert_eq!(dictionary, flat, "the same texts, whatever the layout");
}

#[test]
fn filters_answer_the_same_for_a_dictionary_and_a_flat_column() {
    let texts = few();
    let dictionary = table(&texts);
    let flat = dictionary.slice(0, dictionary.num_rows());
    let probes = ["DE", "de", "D", "", "Dé", "日", "alpha", "say", "zzz", "ß"];
    for table in [&dictionary, &flat] {
        for probe in probes {
            let json = format!("{probe:?}");
            for (op, keep) in [
                (
                    "eq",
                    (|text: &str, probe: &str| text == probe) as fn(&str, &str) -> bool,
                ),
                ("ne", |text, probe| text != probe),
                ("lt", |text, probe| text.as_bytes() < probe.as_bytes()),
                ("lte", |text, probe| text.as_bytes() <= probe.as_bytes()),
                ("gt", |text, probe| text.as_bytes() > probe.as_bytes()),
                ("gte", |text, probe| text.as_bytes() >= probe.as_bytes()),
                ("contains", |text, probe| text.contains(probe)),
                ("starts_with", |text, probe| text.starts_with(probe)),
                ("icontains", |text, probe| {
                    text.to_lowercase().contains(&probe.to_lowercase())
                }),
                ("istarts_with", |text, probe| {
                    text.to_lowercase().starts_with(&probe.to_lowercase())
                }),
            ] {
                assert_eq!(
                    filtered(table, op, &json),
                    expected(&texts, |text| keep(text, probe)),
                    "t {op} {probe:?}, a dictionary: {}",
                    is_dictionary(table)
                );
            }
        }
        let members = r#"["FR","","日本語","nowhere"]"#;
        assert_eq!(
            filtered(table, "in", members),
            expected(&texts, |text| ["FR", "", "日本語", "nowhere"]
                .contains(&text))
        );
    }
}

/// Three-valued logic over a dictionary: a NULL is unknown and `not` keeps it unknown, while a plain
/// miss is false and its `not` true.
#[test]
fn negations_answer_the_same_for_a_dictionary_and_a_flat_column() {
    let texts = few();
    let dictionary = table(&texts);
    let flat = dictionary.slice(0, dictionary.num_rows());
    for table in [&dictionary, &flat] {
        let negated = |filter: &str| {
            ids(&run(
                table,
                &format!(
                    r#"{{"source":"t","select":["id"],"filter":{{"not":{filter}}},"sort":[{{"field":"id"}}]}}"#
                ),
            ))
        };
        assert_eq!(
            negated(r#"{"field":"t","op":"eq","value":"DE"}"#),
            expected(&texts, |text| text != "DE")
        );
        assert_eq!(
            negated(r#"{"field":"t","op":"contains","value":"a"}"#),
            expected(&texts, |text| !text.contains('a'))
        );
        assert_eq!(
            negated(r#"{"field":"t","op":"in","value":["FR","Alpha"]}"#),
            expected(&texts, |text| !["FR", "Alpha"].contains(&text))
        );
    }
}

#[test]
fn sorts_answer_the_same_for_a_dictionary_and_a_flat_column() {
    let texts = few();
    let dictionary = table(&texts);
    let flat = dictionary.slice(0, dictionary.num_rows());
    for table in [&dictionary, &flat] {
        for (direction, nulls) in [
            ("asc", "first"),
            ("asc", "last"),
            ("desc", "first"),
            ("desc", "last"),
        ] {
            for (offset, limit) in [(0, 1000), (0, 7), (250, 40), (590, 50)] {
                let got = ids(&run(
                    table,
                    &format!(
                        r#"{{"source":"t","select":["id","t"],"sort":[{{"field":"t","direction":"{direction}","nulls":"{nulls}"}}],"offset":{offset},"limit":{limit}}}"#
                    ),
                ));
                let mut want: Vec<usize> = (0..texts.len()).collect();
                want.sort_by(|a, b| {
                    let order = match (&texts[*a], &texts[*b]) {
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, _) => {
                            return if nulls == "first" {
                                std::cmp::Ordering::Less
                            } else {
                                std::cmp::Ordering::Greater
                            };
                        }
                        (_, None) => {
                            return if nulls == "first" {
                                std::cmp::Ordering::Greater
                            } else {
                                std::cmp::Ordering::Less
                            };
                        }
                        (Some(a), Some(b)) => a.as_bytes().cmp(b.as_bytes()),
                    };
                    (if direction == "desc" {
                        order.reverse()
                    } else {
                        order
                    })
                    .then(a.cmp(b))
                });
                let want: Vec<i64> = want
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(|row| row as i64)
                    .collect();
                assert_eq!(
                    got, want,
                    "{direction}, nulls {nulls}, from {offset}, {limit} rows"
                );
            }
        }
        // Two keys: the text, then the id descending.
        let got = ids(&run(
            table,
            r#"{"source":"t","select":["id","t"],"sort":[{"field":"t"},{"field":"id","direction":"desc"}],"limit":300}"#,
        ));
        let mut want: Vec<usize> = (0..texts.len()).collect();
        want.sort_by(|a, b| {
            let order = match (&texts[*a], &texts[*b]) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, _) => std::cmp::Ordering::Greater,
                (_, None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.as_bytes().cmp(b.as_bytes()),
            };
            order.then(b.cmp(a))
        });
        let want: Vec<i64> = want.into_iter().take(300).map(|row| row as i64).collect();
        assert_eq!(got, want);
    }
}

#[test]
fn groups_and_aggregates_answer_the_same_for_a_dictionary_and_a_flat_column() {
    let texts = few();
    let dictionary = table(&texts);
    let flat = dictionary.slice(0, dictionary.num_rows());
    let mut counts: BTreeMap<Option<Vec<u8>>, i64> = BTreeMap::new();
    for text in &texts {
        *counts
            .entry(text.as_ref().map(|text| text.as_bytes().to_vec()))
            .or_default() += 1;
    }
    // The default order puts NULL last.
    let mut want: Vec<Vec<Value>> = counts
        .iter()
        .filter(|(text, _)| text.is_some())
        .map(|(text, count)| {
            vec![
                Value::Utf8(String::from_utf8(text.clone().unwrap()).unwrap()),
                Value::Int64(*count),
            ]
        })
        .collect();
    want.push(vec![Value::Null, Value::Int64(counts[&None])]);
    for table in [&dictionary, &flat] {
        let got = run(
            table,
            r#"{"source":"t","select":["t"],"group":["t"],"aggregate":[{"fn":"count","as":"c"}],"sort":[{"field":"t"}]}"#,
        );
        assert_eq!(got, want);
        let lowest = run(
            table,
            r#"{"source":"t","select":[],"aggregate":[{"fn":"min","field":"t","as":"lo"},{"fn":"max","field":"t","as":"hi"},{"fn":"count","field":"t","as":"n"}]}"#,
        );
        let present: Vec<&String> = texts.iter().flatten().collect();
        assert_eq!(
            lowest,
            vec![vec![
                Value::Utf8(
                    present
                        .iter()
                        .min_by(|a, b| a.as_bytes().cmp(b.as_bytes()))
                        .unwrap()
                        .to_string()
                ),
                Value::Utf8(
                    present
                        .iter()
                        .max_by(|a, b| a.as_bytes().cmp(b.as_bytes()))
                        .unwrap()
                        .to_string()
                ),
                Value::Int64(present.len() as i64),
            ]]
        );
    }
}

#[test]
fn texts_that_are_distinct_make_a_flat_column_and_answer_alike() {
    // More rows than the first look at the dictionary, and every text new: flat, and the same answers.
    let texts: Vec<Option<String>> = (0..9000)
        .map(|row| {
            if row % 50 == 7 {
                None
            } else {
                Some(format!("note {}", (row * 37) % 9000))
            }
        })
        .collect();
    let built = table(&texts);
    assert!(!is_dictionary(&built));
    assert_eq!(
        filtered(&built, "eq", r#""note 74""#),
        expected(&texts, |text| text == "note 74")
    );
    assert_eq!(
        filtered(&built, "contains", r#""note 12""#),
        expected(&texts, |text| text.contains("note 12"))
    );
    // And a column of few texts past the first look stays a dictionary.
    let many: Vec<Option<String>> = (0..9000)
        .map(|row| Some(format!("c{}", row % 40)))
        .collect();
    let kept = table(&many);
    assert!(is_dictionary(&kept));
    assert_eq!(
        filtered(&kept, "eq", r#""c7""#),
        expected(&many, |text| text == "c7")
    );
}
