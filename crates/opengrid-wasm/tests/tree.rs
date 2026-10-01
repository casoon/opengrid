//! The engine's tree on the host (plan point 121): every tree conformance case.

mod tree_cases;

#[test]
fn the_engine_answers_every_tree_case() {
    for case in tree_cases::CASES {
        tree_cases::check(case);
    }
}
