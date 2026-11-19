//! The shared compiler under its dialects.
//!
//! PostgreSQL's statements are pinned by `opengrid-datasource-postgres`; the
//! SQLite dialect's are pinned here, and the two meet in one query: the same
//! `ValidatedQuery` compiles to the same shape, with each dialect's placeholders,
//! collation and NULL handling.

use std::path::Path;

use opengrid_sql::{CompileError, CompiledQuery, Compiler, MySql, QueryCompiler};
use opengrid_types::Schema;

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
    let mut checked =
        opengrid_conformance::check_dir(&suite().join("opengrid-conformance/cases"), &schema())
            .expect("the conformance cases");
    checked.sort_by(|a, b| a.case.id.cmp(&b.case.id));
    checked
}

fn render(compiled: &CompiledQuery) -> String {
    let mut out = compiled.sql.clone();
    for (index, value) in compiled.params.iter().enumerate() {
        out.push_str(&format!("\n  {} = {value:?}", index + 1));
    }
    out
}

#[test]
fn every_conformance_case_compiles_for_sqlite() {
    let compiler = Compiler::sqlite("orders", schema());
    let mut out = String::new();
    for checked in cases() {
        let compiled = compiler
            .compile(&checked.query)
            .unwrap_or_else(|error| panic!("{}: {error}", checked.case.id));
        out.push_str(&format!("# {}\n{}\n\n", checked.case.id, render(&compiled)));
    }
    insta::assert_snapshot!("sqlite_sql", out);
}

/// The two dialects write the same statement for the same query, apart from
/// what a dialect is for: one query, both outputs.
#[test]
fn one_query_two_dialects() {
    let postgres = Compiler::postgres("orders", schema());
    let sqlite = Compiler::sqlite("orders", schema());
    let checked = cases()
        .into_iter()
        .find(|checked| checked.query.filter.is_some() && !checked.query.sort.is_empty())
        .expect("a case with a filter and a sort");
    let (pg, lite) = (
        postgres.compile(&checked.query).expect("postgres"),
        sqlite.compile(&checked.query).expect("sqlite"),
    );
    assert!(pg.sql.contains("$1") && !pg.sql.contains('?'), "{}", pg.sql);
    assert!(
        lite.sql.contains('?') && !lite.sql.contains('$'),
        "{}",
        lite.sql
    );
    assert_eq!(pg.params, lite.params);
    assert!(pg.sql.contains("NULLS ") && lite.sql.contains("NULLS "));
}

#[test]
fn what_a_dialect_cannot_do_is_a_capability() {
    let sqlite = Compiler::sqlite("orders", schema());
    assert!(!opengrid_sql::Dialect::grouping_sets(sqlite.dialect()));
    let postgres = Compiler::postgres("orders", schema());
    assert!(opengrid_sql::Dialect::grouping_sets(postgres.dialect()));
    let mysql = Compiler::<MySql>::new("orders", schema());
    assert!(!opengrid_sql::Dialect::grouping_sets(mysql.dialect()));
    assert!(!opengrid_sql::Dialect::recursive(mysql.dialect()));
}

/// Every conformance case MySQL can answer compiles; the statements are
/// pinned, and what it cannot answer as the engine would is `Unsupported`,
/// never a statement that answers differently (issue #501).
#[test]
fn every_conformance_case_compiles_for_mysql() {
    let compiler = Compiler::<MySql>::new("orders", schema());
    let mut out = String::new();
    for checked in cases() {
        let compiled = compiler
            .compile(&checked.query)
            .unwrap_or_else(|error| panic!("{}: {error}", checked.case.id));
        out.push_str(&format!("# {}\n{}\n\n", checked.case.id, render(&compiled)));
    }
    insta::assert_snapshot!("mysql_sql", out);
}

#[test]
fn mysql_refuses_what_it_cannot_answer_as_the_engine_would() {
    let compiler = Compiler::<MySql>::new("orders", schema());
    let compile = |json: &str| {
        let query: opengrid_query::Query = opengrid_json::from_str(json).expect("a query");
        let query = query
            .validate(&schema(), &opengrid_query::Limits::default())
            .expect("valid");
        compiler.compile(&query)
    };
    let unsupported = |result: Result<CompiledQuery, CompileError>| {
        matches!(result, Err(CompileError::Unsupported { .. }))
    };
    // Case folding (S5a) and a locale MySQL has no collation for (S4a).
    assert!(unsupported(compile(
        r#"{"source":"orders","select":["id"],"filter":{"field":"customer","op":"icontains","value":"a"}}"#
    )));
    assert!(unsupported(compile(
        r#"{"source":"orders","select":["id","customer"],"sort":[{"field":"customer","collation":"ja"}]}"#
    )));
    // A locale it has: its collation, then the bytes.
    let sorted = compile(
        r#"{"source":"orders","select":["id","customer"],"sort":[{"field":"customer","collation":"sv","direction":"desc"}]}"#,
    )
    .expect("Swedish is MySQL's");
    assert!(
        sorted.sql.ends_with(
            "ORDER BY (\"customer\" IS NULL) ASC, \"customer\" COLLATE utf8mb4_sv_0900_as_cs DESC, \
             \"customer\" COLLATE utf8mb4_0900_bin DESC"
        ),
        "{}",
        sorted.sql
    );
}

#[test]
fn an_identifier_with_a_quote_is_refused() {
    let compiler = Compiler::sqlite("orders\"; DROP TABLE orders; --", schema());
    let checked = &cases()[0];
    assert!(matches!(
        compiler.compile(&checked.query),
        Err(CompileError::Identifier { .. })
    ));
}
