//! The one measurement of plan point 08 (its DoD asks for the number in the
//! commit, as a guide, not as a gate): one million rows, one group key, native
//! and in release mode.
//!
//! `#[ignore]`d so the ordinary test run stays fast:
//!
//! ```console
//! cargo test --release -p opengrid-arrow-engine --test benchmark -- --ignored --nocapture
//! ```

use std::time::Instant;

use opengrid_arrow_engine::execute::execute;
use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema};

const ROWS: usize = 1_000_000;
const GROUPS: usize = 1_000;

fn field(name: &str) -> FieldName {
    FieldName::new(name).expect("a valid identifier")
}

#[test]
#[ignore = "measurement, not a gate — run it explicitly with --ignored"]
fn one_million_rows_with_one_group_key() {
    let schema = Schema::new(vec![
        Field::new(field("grp"), DataType::Utf8),
        Field::new(field("amount"), DataType::Int64),
    ]);

    let mut text = String::with_capacity(ROWS * 24);
    text.push_str("grp,amount\n");
    for row in 0..ROWS {
        text.push('g');
        text.push_str(&(row % GROUPS).to_string());
        text.push(',');
        text.push_str(&(row % 977).to_string());
        text.push('\n');
    }

    let ingest_started = Instant::now();
    let batches = load_csv(text.as_bytes(), &schema, CsvOptions::default()).expect("the CSV loads");
    let ingest = ingest_started.elapsed();

    let query: Query = serde_json::from_str(
        r#"{"source":"orders","select":["grp","total"],
            "group":["grp"],
            "aggregate":[{"field":"amount","fn":"sum","as":"total"}],
            "sort":[{"field":"total","direction":"desc"}]}"#,
    )
    .expect("the query parses");
    let query = query
        .validate(&schema, &Limits::default())
        .expect("the query validates");

    let query_started = Instant::now();
    let result = execute(&batches, &query).expect("the engine answers");
    let answered = query_started.elapsed();

    let bytes = text.len();
    println!(
        "1M rows / {GROUPS} groups ({:.1} MiB CSV): ingest {:?}, execute {:?} (group+sum+sort), \
         {} result rows",
        bytes as f64 / (1024.0 * 1024.0),
        ingest,
        answered,
        result
            .batches
            .iter()
            .map(|batch| batch.num_rows())
            .sum::<usize>()
    );

    assert_eq!(result.total_count, GROUPS as u64);
}
