//! The generated SQL, pinned (plan point 25).
//!
//! Every one of the 48 conformance cases is compiled and its SQL — plus the
//! parameters, listed separately — held in one snapshot. The suite is the same
//! one the local engine answers, so the two sides of risk R4 are driven from a
//! single source of truth.
//!
//! What a reviewer should look for in the snapshot: `NULLS FIRST`/`NULLS LAST`
//! on every sort key (S3), `COLLATE "C"` on every string comparison and string
//! sort (S4), `count(*)` where no field is named and `count("x")` where one is
//! (S11), and the casts on `sum`/`avg` (S12).

use std::path::Path;

use opengrid_datasource_postgres::{CompiledQuery, PostgresCompiler, QueryCompiler};
use opengrid_query::{ValidatedFilter, ValidatedQuery};
use opengrid_types::{DataType, Schema, Value};

/// The conformance suite's directory.
fn suite() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
}

fn schema() -> Schema {
    opengrid_conformance::load_schema(&suite().join("opengrid-conformance/data/orders.schema.json"))
        .expect("the conformance schema")
}

fn cases() -> Vec<opengrid_conformance::Checked> {
    let schema = schema();
    let mut checked =
        opengrid_conformance::check_dir(&suite().join("opengrid-conformance/cases"), &schema)
            .expect("the conformance cases");
    checked.sort_by(|a, b| a.case.id.cmp(&b.case.id));
    checked
}

fn compiler() -> PostgresCompiler {
    PostgresCompiler::new("orders", schema())
}

/// Renders a compiled query the way the snapshot shows it.
fn render(compiled: &CompiledQuery) -> String {
    let mut out = compiled.sql.clone();
    if !compiled.params.is_empty() {
        out.push_str("\n  params:");
        for (index, value) in compiled.params.iter().enumerate() {
            out.push_str(&format!("\n    ${} = {value:?}", index + 1));
        }
    }
    out
}

/// One snapshot for all 48 cases: a single diff to review when the compiler
/// changes, instead of 48 files that have to be opened one by one.
#[test]
fn every_conformance_case_compiles() {
    let compiler = compiler();
    let mut out = String::new();
    for checked in cases() {
        let compiled = compiler
            .compile(&checked.query)
            .unwrap_or_else(|error| panic!("{}: {error}", checked.case.id));
        out.push_str(&format!(
            "# {} [{}]\n{}\n\n",
            checked.case.id,
            checked.case.rule,
            render(&compiled)
        ));
    }
    insta::assert_snapshot!("conformance_sql", out);
}

/// The property that makes injection impossible: no value is ever written into
/// the statement.
///
/// Checked over the whole suite — which contains strings with quotes, NFC/NFD
/// pairs, empty strings, exact decimals and NaN — by looking for the one thing a
/// literal would need: a quote character.
#[test]
fn no_value_reaches_the_sql_text() {
    let compiler = compiler();
    for checked in cases() {
        let compiled = compiler.compile(&checked.query).expect("compiles");
        assert!(
            !compiled.sql.contains('\''),
            "{}: the SQL carries a string literal:\n{}",
            checked.case.id,
            compiled.sql
        );
        // Every placeholder that exists has a value, and every value a placeholder.
        for index in 1..=compiled.params.len() {
            assert!(
                compiled.sql.contains(&format!("${index}")),
                "{}: ${index} is missing from the SQL",
                checked.case.id
            );
        }
        assert!(
            !compiled
                .sql
                .contains(&format!("${}", compiled.params.len() + 1)),
            "{}: more placeholders than parameters",
            checked.case.id
        );
    }
}

/// A string value with a quote in it stays a parameter — the case an injection
/// attempt would use.
#[test]
fn a_quote_in_a_value_is_still_a_parameter() {
    let case = cases()
        .into_iter()
        .find(|checked| checked.case.id.starts_with("s5-contains"))
        .expect("a contains case");
    let compiler = compiler();

    let mut query = case.query.clone();
    query.filter = Some(ValidatedFilter::Cmp {
        field: opengrid_types::FieldName::new("customer").unwrap(),
        data_type: DataType::Utf8,
        op: opengrid_query::CmpOp::Eq,
        value: Value::Utf8("'; DROP TABLE orders; --".to_owned()),
    });

    let compiled = compiler.compile(&query).expect("compiles");
    assert!(!compiled.sql.contains("DROP"), "{}", compiled.sql);
    assert_eq!(
        compiled.params.last(),
        Some(&Value::Utf8("'; DROP TABLE orders; --".to_owned()))
    );
}

/// An identifier that could end the quoting is refused rather than escaped.
#[test]
fn an_identifier_with_a_quote_is_refused() {
    let case = cases().into_iter().next().expect("a case");
    let compiler = PostgresCompiler::new("orders\"; DROP TABLE orders; --", schema());
    assert!(compiler.compile(&case.query).is_err());
}

/// The four places PostgreSQL disagrees with the specification, each pinned on
/// the case that exercises it.
#[test]
fn the_divergences_from_postgresql_defaults_are_explicit() {
    let compiler = compiler();
    let by_id = |id: &str| -> String {
        let checked = cases()
            .into_iter()
            .find(|checked| checked.case.id == id)
            .unwrap_or_else(|| panic!("case {id}"));
        compiler.compile(&checked.query).expect("compiles").sql
    };

    // S3: NULL placement is never left to the default — which differs from ours
    // exactly for DESC.
    let descending = by_id("s3-nulls-last-descending");
    assert!(descending.contains("DESC NULLS LAST"), "{descending}");

    // S4: binary collation on a string sort.
    let binary = by_id("s4-binary-order-ascending");
    assert!(binary.contains("COLLATE \"C\""), "{binary}");

    // S11: the two counts are different queries.
    let counts = by_id("s11-count-star-versus-count-field");
    assert!(counts.contains("count(*)"), "{counts}");
    assert!(counts.contains("count(\"qty\")"), "{counts}");

    // S12: the casts that keep the result type.
    let int_sum = by_id("s12-sum-of-int64-stays-int64");
    assert!(int_sum.contains("::bigint"), "{int_sum}");
    let decimal_sum = by_id("s12-sum-of-decimal-widens-to-38");
    assert!(decimal_sum.contains("::numeric(38,"), "{decimal_sum}");
    let average = by_id("s11-average-of-integers-is-a-float");
    assert!(average.contains("::double precision"), "{average}");
}

/// Paging travels as parameters too, and in the order PostgreSQL expects.
#[test]
fn paging_is_parameterized() {
    let compiler = compiler();
    let checked = cases()
        .into_iter()
        .find(|checked| checked.case.id == "s6-offset-with-sort")
        .expect("the paging case");
    let compiled = compiler.compile(&checked.query).expect("compiles");

    assert!(compiled.sql.contains("OFFSET $"), "{}", compiled.sql);
    assert!(compiled.sql.contains("LIMIT $"), "{}", compiled.sql);
    let offset_at = compiled.sql.find("OFFSET").unwrap();
    let limit_at = compiled.sql.find("LIMIT").unwrap();
    assert!(offset_at < limit_at, "OFFSET comes before LIMIT");
}

/// `total_count` counts before paging — and with a grouping it counts groups,
/// which is the case a window function would get wrong.
#[test]
fn the_count_statement_drops_paging_and_counts_groups() {
    let compiler = compiler();

    // Without a grouping: the filter stays, everything else goes.
    let paged = cases()
        .into_iter()
        .find(|checked| checked.case.id == "s6-offset-with-sort")
        .expect("the paging case");
    let count = CompiledQuery::count_of(&paged.query, &compiler).expect("compiles");
    assert!(
        count
            .sql
            .starts_with("SELECT count(*) AS \"total_count\" FROM \"orders\"")
    );
    assert!(!count.sql.contains("LIMIT"), "{}", count.sql);
    assert!(!count.sql.contains("OFFSET"), "{}", count.sql);
    assert!(!count.sql.contains("ORDER BY"), "{}", count.sql);

    // With a grouping: the rows are the groups, so the grouping has to survive.
    let grouped = cases()
        .into_iter()
        .find(|checked| checked.case.id == "s10-null-forms-its-own-group")
        .expect("a grouping case");
    let count = CompiledQuery::count_of(&grouped.query, &compiler).expect("compiles");
    assert!(count.sql.contains("GROUP BY"), "{}", count.sql);
    assert!(count.sql.contains("AS \"grouped\""), "{}", count.sql);
    assert!(count.sql.starts_with("SELECT count(*)"), "{}", count.sql);
}

/// An empty `and` keeps every row, an empty `or` keeps none — the identity of
/// each, not "no filter".
#[test]
fn an_empty_filter_list_is_not_nothing() {
    let compiler = compiler();
    let checked = cases().into_iter().next().expect("a case");

    let mut query: ValidatedQuery = checked.query.clone();
    query.filter = Some(ValidatedFilter::And(Vec::new()));
    assert!(compiler.compile(&query).unwrap().sql.contains("WHERE TRUE"));

    query.filter = Some(ValidatedFilter::Or(Vec::new()));
    assert!(
        compiler
            .compile(&query)
            .unwrap()
            .sql
            .contains("WHERE FALSE")
    );
}
