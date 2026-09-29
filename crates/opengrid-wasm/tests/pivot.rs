//! The engine's pivot on the host (issue #28): every pivot conformance case,
//! and the refusals a page sees.

mod pivot_cases;

#[test]
fn the_engine_answers_every_pivot_case() {
    for case in pivot_cases::CASES {
        pivot_cases::check(case);
    }
}

#[test]
fn a_pivot_the_engine_cannot_answer_says_why() {
    let engine = pivot_cases::engine();
    let unknown = engine
        .pivot_result(r#"{"source":"nope","rows":["country"],"values":[{"fn":"count","as":"n"}]}"#)
        .unwrap_err();
    assert_eq!(unknown, "unknown source \"nope\"");
    let no_measure = engine
        .pivot_result(r#"{"source":"orders","rows":["country"],"values":[]}"#)
        .unwrap_err();
    assert!(no_measure.contains("measure"), "{no_measure}");
    // A query is not a pivot: the pivot reader names what it expected.
    let query = engine
        .pivot_result(r#"{"source":"orders","select":["id"]}"#)
        .unwrap_err();
    assert!(
        query.starts_with("pivot JSON: unknown field `select`"),
        "{query}"
    );
}
