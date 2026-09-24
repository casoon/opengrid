#![cfg(target_arch = "wasm32")]
//! Browser benchmarks for the local engine (plan point 11, step 3).
//!
//! Same deterministic rows as the native benches (`crates/opengrid-arrow-engine/benches/engine.rs`):
//! both call [`xtask::orders_csv`], so a number here is comparable to the native
//! one. Run headless Chrome via `just bench-wasm`.
//!
//! This is `#[wasm_bindgen_bench]`, the criterion subset the runner carries
//! (decision E4) — no `wasm-pack`, no Playwright. The macro builds the harness
//! from its own `Criterion::default()` (100 samples, 5 s); each function replaces
//! it with a small configuration once, because a 1M ingest in WASM is seconds,
//! not microseconds.
//!
//! Covered: ingest, filter, sort, multi-sort, group + sum at 100k and 1M, and
//! the 40-row windows the grid asks while scrolling a sorted list (point 44). The
//! module footprint is reported from the 1M ingest setup (`memory_size`), which
//! is the only place the numbers are collected; module *startup* is a page metric
//! and stays out of the bench binary.

use std::time::Duration;

use opengrid_arrow_engine::execute::execute;
use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
use opengrid_query::{Limits, Query, ValidatedQuery};
use opengrid_types::Schema;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const SEED: u64 = 1;
const SIZES: [usize; 2] = [100_000, 1_000_000];

/// Ten samples and a short window; see the module comment.
fn configured() -> Criterion {
    Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_secs(2))
}

fn validate(json: &str, schema: &Schema) -> ValidatedQuery {
    let query: Query = serde_json::from_str(json).expect("the query parses");
    query
        .validate(schema, &Limits::default())
        .expect("the query validates")
}

fn schema() -> Schema {
    xtask::orders_schema()
}

/// The queries, verbatim from the native benches so the two stay comparable.
fn filter_query(schema: &Schema) -> ValidatedQuery {
    validate(
        r#"{"source":"orders","select":["id","amount"],
            "filter":{"and":[
                {"field":"country","op":"eq","value":"DE"},
                {"field":"amount","op":"gt","value":"100"}]}}"#,
        schema,
    )
}

fn sort_query(schema: &Schema) -> ValidatedQuery {
    validate(
        r#"{"source":"orders","select":["id","amount"],"sort":[{"field":"amount"}]}"#,
        schema,
    )
}

fn multi_sort_query(schema: &Schema) -> ValidatedQuery {
    validate(
        r#"{"source":"orders","select":["id","country","amount"],
            "sort":[{"field":"country"},{"field":"amount","direction":"desc"}]}"#,
        schema,
    )
}

fn group_sum_query(schema: &Schema) -> ValidatedQuery {
    validate(
        r#"{"source":"orders","select":["country","total"],
            "group":["country"],
            "aggregate":[{"field":"amount","fn":"sum","as":"total"}],
            "sort":[{"field":"total","direction":"desc"}]}"#,
        schema,
    )
}

fn aggregate_query(schema: &Schema) -> ValidatedQuery {
    validate(
        r#"{"source":"orders","select":[],
            "aggregate":[
                {"field":"amount","fn":"sum","as":"sum"},
                {"field":"amount","fn":"avg","as":"avg"},
                {"field":"amount","fn":"count","as":"count"},
                {"field":"amount","fn":"min","as":"min"},
                {"field":"amount","fn":"max","as":"max"}]}"#,
        schema,
    )
}

#[wasm_bindgen_bench]
fn ingest_bench(_: &mut Criterion) {
    let mut c = configured();
    for size in SIZES {
        let schema = schema();
        let csv = xtask::orders_csv(size, SEED);
        c.bench_function(&format!("ingest/{size}"), |b| {
            b.iter(|| load_csv(csv.as_bytes(), &schema, CsvOptions::default()).expect("ingest"));
        });
    }
}

#[wasm_bindgen_bench]
fn filter_bench(_: &mut Criterion) {
    let mut c = configured();
    for size in SIZES {
        let schema = schema();
        let batches = load_csv(
            xtask::orders_csv(size, SEED).as_bytes(),
            &schema,
            CsvOptions::default(),
        )
        .expect("ingest");
        let query = filter_query(&schema);
        c.bench_function(&format!("filter/{size}"), |b| {
            b.iter(|| execute(&batches, &query).expect("the engine answers"));
        });
    }
}

#[wasm_bindgen_bench]
fn sort_bench(_: &mut Criterion) {
    let mut c = configured();
    for size in SIZES {
        let schema = schema();
        let batches = load_csv(
            xtask::orders_csv(size, SEED).as_bytes(),
            &schema,
            CsvOptions::default(),
        )
        .expect("ingest");
        let query = sort_query(&schema);
        c.bench_function(&format!("sort/{size}"), |b| {
            b.iter(|| execute(&batches, &query).expect("the engine answers"));
        });
    }
}

#[wasm_bindgen_bench]
fn multi_sort_bench(_: &mut Criterion) {
    let mut c = configured();
    for size in SIZES {
        let schema = schema();
        let batches = load_csv(
            xtask::orders_csv(size, SEED).as_bytes(),
            &schema,
            CsvOptions::default(),
        )
        .expect("ingest");
        let query = multi_sort_query(&schema);
        c.bench_function(&format!("multi-sort/{size}"), |b| {
            b.iter(|| execute(&batches, &query).expect("the engine answers"));
        });
    }
}

/// A 40-row window from the middle of a sort, over every column the grid shows —
/// what the grid asks on each scroll step (point 44).
fn window_query(schema: &Schema, sort: &str, offset: usize) -> ValidatedQuery {
    validate(
        &format!(
            r#"{{"source":"orders","select":["id","customer","country","amount","qty","ordered_on"],
                "sort":{sort},"offset":{offset},"limit":40}}"#
        ),
        schema,
    )
}

#[wasm_bindgen_bench]
fn window_bench(_: &mut Criterion) {
    let mut c = configured();
    for size in SIZES {
        let schema = schema();
        let batches = load_csv(
            xtask::orders_csv(size, SEED).as_bytes(),
            &schema,
            CsvOptions::default(),
        )
        .expect("ingest");
        for (name, sort) in [
            ("sort window", r#"[{"field":"id"}]"#),
            (
                "multi-sort window",
                r#"[{"field":"customer"},{"field":"amount","direction":"desc"}]"#,
            ),
        ] {
            let query = window_query(&schema, sort, size / 2);
            c.bench_function(&format!("{name}/{size}"), |b| {
                b.iter(|| execute(&batches, &query).expect("the engine answers"));
            });
        }
    }
}

#[wasm_bindgen_bench]
fn group_sum_bench(_: &mut Criterion) {
    let mut c = configured();
    for size in SIZES {
        let schema = schema();
        let batches = load_csv(
            xtask::orders_csv(size, SEED).as_bytes(),
            &schema,
            CsvOptions::default(),
        )
        .expect("ingest");
        let query = group_sum_query(&schema);
        c.bench_function(&format!("group + sum/{size}"), |b| {
            b.iter(|| execute(&batches, &query).expect("the engine answers"));
        });
    }
}

#[wasm_bindgen_bench]
fn aggregate_bench(_: &mut Criterion) {
    let mut c = configured();
    for size in SIZES {
        let schema = schema();
        let batches = load_csv(
            xtask::orders_csv(size, SEED).as_bytes(),
            &schema,
            CsvOptions::default(),
        )
        .expect("ingest");
        let query = aggregate_query(&schema);
        c.bench_function(&format!("aggregate/{size}"), |b| {
            b.iter(|| execute(&batches, &query).expect("the engine answers"));
        });
    }
}

/// The WASM footprint after a 1M ingest, printed once so the number lands in the
/// bench log (plan step 3, "WASM-Speichergröße").
///
/// `memory_size` is the linear memory in 64 KiB pages — the whole module heap,
/// not a per-allocation figure. The setup is outside the timed loop; the trivial
/// benchmark keeps the criterion harness happy.
#[wasm_bindgen_bench]
fn memory_footprint(_: &mut Criterion) {
    let mut c = configured();
    let schema = schema();
    let batches = load_csv(
        xtask::orders_csv(1_000_000, SEED).as_bytes(),
        &schema,
        CsvOptions::default(),
    )
    .expect("ingest");
    let pages = core::arch::wasm32::memory_size::<0>();
    console_log!(
        "wasm linear memory after 1M ingest: {pages} pages = {} MiB",
        (pages * 65_536) / (1024 * 1024)
    );
    c.bench_function("memory/noop", |b| {
        b.iter(|| std::hint::black_box(batches.len()))
    });
}
