//! Native criterion benchmarks for the local engine (plan point 11, step 2).
//!
//! The data comes from [`xtask::orders_csv`] — deterministic in `(rows, seed)`, so
//! the same rows feed the WASM benches (`crates/opengrid-wasm/benches/engine.rs`)
//! and a number can be compared across the two. Each size is ingested once in
//! setup; only the operation under test is timed.
//!
//! ```console
//! just bench-native
//! ```
//!
//! Sizes and operations match `plan/spezifikation/12-qualitaet.md` §Messwerte.

use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use opengrid_arrow_engine::execute::execute;
use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
use opengrid_query::{Limits, Query, ValidatedQuery};
use opengrid_types::Schema;

/// Fixed so a run is reproducible; one seed for all sizes.
const SEED: u64 = 1;
const SIZES: [usize; 3] = [10_000, 100_000, 1_000_000];

/// The five operations the specification lists for the local engine.
struct Queries {
    filter: ValidatedQuery,
    sort: ValidatedQuery,
    multi_sort: ValidatedQuery,
    group_sum: ValidatedQuery,
    aggregate: ValidatedQuery,
}

impl Queries {
    fn new(schema: &Schema) -> Self {
        Self {
            filter: validate(
                r#"{"source":"orders","select":["id","amount"],
                    "filter":{"and":[
                        {"field":"country","op":"eq","value":"DE"},
                        {"field":"amount","op":"gt","value":"100"}]}}"#,
                schema,
            ),
            sort: validate(
                r#"{"source":"orders","select":["id","amount"],"sort":[{"field":"amount"}]}"#,
                schema,
            ),
            multi_sort: validate(
                r#"{"source":"orders","select":["id","country","amount"],
                    "sort":[{"field":"country"},{"field":"amount","direction":"desc"}]}"#,
                schema,
            ),
            group_sum: validate(
                r#"{"source":"orders","select":["country","total"],
                    "group":["country"],
                    "aggregate":[{"field":"amount","fn":"sum","as":"total"}],
                    "sort":[{"field":"total","direction":"desc"}]}"#,
                schema,
            ),
            aggregate: validate(
                r#"{"source":"orders","select":[],
                    "aggregate":[
                        {"field":"amount","fn":"sum","as":"sum"},
                        {"field":"amount","fn":"avg","as":"avg"},
                        {"field":"amount","fn":"count","as":"count"},
                        {"field":"amount","fn":"min","as":"min"},
                        {"field":"amount","fn":"max","as":"max"}]}"#,
                schema,
            ),
        }
    }
}

fn validate(json: &str, schema: &Schema) -> ValidatedQuery {
    let query: Query = serde_json::from_str(json).expect("the query parses");
    query
        .validate(schema, &Limits::default())
        .expect("the query validates")
}

fn benchmarks(c: &mut Criterion) {
    for size in SIZES {
        let label = format!("{size}");
        let schema = xtask::orders_schema();
        let csv = xtask::orders_csv(size, SEED);
        let batches = load_csv(csv.as_bytes(), &schema, CsvOptions::default()).expect("ingest");
        let queries = Queries::new(&schema);

        let mut ingest = c.benchmark_group("ingest");
        ingest.throughput(Throughput::Bytes(csv.len() as u64));
        ingest.bench_with_input(BenchmarkId::from_parameter(&label), &csv, |b, csv| {
            b.iter(|| load_csv(csv.as_bytes(), &schema, CsvOptions::default()).expect("ingest"));
        });
        ingest.finish();

        bench_query(c, "filter", &label, &batches, &queries.filter);
        bench_query(c, "sort", &label, &batches, &queries.sort);
        bench_query(c, "multi-sort", &label, &batches, &queries.multi_sort);
        bench_query(c, "group + sum", &label, &batches, &queries.group_sum);
        bench_query(c, "aggregate", &label, &batches, &queries.aggregate);
    }
}

/// One query over pre-ingested batches. Takes the `Vec` by reference to keep the
/// criterion input `Sized` (`ptr_arg` is about public function signatures, this is
/// a local helper and the callers own the vectors).
#[allow(clippy::ptr_arg)]
fn bench_query(
    c: &mut Criterion,
    name: &str,
    label: &str,
    batches: &Vec<arrow_array::RecordBatch>,
    query: &ValidatedQuery,
) {
    c.bench_with_input(BenchmarkId::new(name, label), batches, |b, batches| {
        b.iter(|| execute(batches, query).expect("the engine answers"));
    });
}

// Few samples and a short measurement window: a 1M query is tens to hundreds of
// milliseconds, and criterion's defaults would run for many minutes.
criterion_group! {
    name = engine;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3));
    targets = benchmarks
}
criterion_main!(engine);
