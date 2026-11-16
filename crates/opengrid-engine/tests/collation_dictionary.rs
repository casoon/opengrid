//! A sort by a locale asks the collator once per distinct text of a dictionary column and shares
//! the keys among the rows (issue #467). The answer must not depend on that: the same texts as a
//! dictionary and as a flat column (the flat twin is made by `Table::slice`, which gathers into
//! flat buffers) sort alike, NULLs, ties and pages included.
#![cfg(feature = "icu")]

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

/// Texts the locale orders differently from the bytes: accents, case, an empty text, a text that is
/// the start of another.
fn texts() -> Vec<Option<String>> {
    let words = [
        "apfel", "Apfel", "Äpfel", "Zebra", "zebra", "Éclair", "eclair", "", "ä", "a", "z", "ß",
        "ss",
    ];
    (0..400)
        .map(|row| {
            if row % 11 == 4 {
                None
            } else {
                Some(words[(row * 5 + row / 9) % words.len()].to_owned())
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

fn ids(table: &Table, json: &str) -> Vec<i64> {
    let query: Query = opengrid_json::from_str(json).expect("a well-formed query");
    let query = query
        .validate(&schema(), &Limits::default())
        .expect("a valid query");
    let result = execute(table, &query).expect("the engine answers");
    (0..result.table.num_rows())
        .map(|row| match result.table.column_at(0).value(row) {
            Value::Int64(id) => id,
            other => panic!("an id, got {other:?}"),
        })
        .collect()
}

#[test]
fn a_dictionary_and_its_flat_twin_sort_alike_by_a_locale() {
    let dictionary = table(&texts());
    assert!(matches!(
        dictionary.column_at(1).values(),
        Values::Utf8(Strs::Dict { .. })
    ));
    let twin = dictionary.slice(0, dictionary.num_rows());
    assert!(matches!(
        twin.column_at(1).values(),
        Values::Utf8(Strs::Flat { .. })
    ));
    for locale in ["de", "sv", "en"] {
        for direction in ["asc", "desc"] {
            for nulls in ["first", "last"] {
                for page in [
                    r#""limit":400"#,
                    r#""offset":150,"limit":40"#,
                    r#""limit":7"#,
                ] {
                    let query = format!(
                        r#"{{"source":"t","select":["id","t"],"sort":[{{"field":"t","collation":"{locale}","direction":"{direction}","nulls":"{nulls}"}}],{page}}}"#
                    );
                    assert_eq!(
                        ids(&dictionary, &query),
                        ids(&twin, &query),
                        "{locale} {direction} nulls {nulls}, {page}"
                    );
                }
            }
        }
    }
    // Two keys: the locale first, the id to break ties.
    let query = r#"{"source":"t","select":["id","t"],"sort":[{"field":"t","collation":"de"},{"field":"id","direction":"desc"}],"limit":400}"#;
    assert_eq!(ids(&dictionary, query), ids(&twin, query));
}

#[test]
fn the_german_order_is_the_locales_and_not_the_bytes() {
    let words = ["Zebra", "apfel", "Äpfel", "Apfel", "zebra", "a"];
    let rows: Vec<Option<String>> = words.iter().map(|word| Some((*word).to_owned())).collect();
    let table = table(&rows);
    assert!(matches!(
        table.column_at(1).values(),
        Values::Utf8(Strs::Dict { .. })
    ));
    let sorted: Vec<&str> = ids(
        &table,
        r#"{"source":"t","select":["id","t"],"sort":[{"field":"t","collation":"de"}],"limit":10}"#,
    )
    .into_iter()
    .map(|id| words[id as usize])
    .collect();
    assert_eq!(sorted, ["a", "apfel", "Apfel", "Äpfel", "zebra", "Zebra"]);
}
