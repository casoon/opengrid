//! Calculated measures against a fixture reckoned by hand (issue #329).
//!
//! The conformance cases pin rules P10–P12 over the shared dataset; this file
//! is the small table a reader can check with a pencil. Every expected number
//! below was written down from the table, not printed by the code under test.
//!
//! ```text
//! zone region year amount units
//! A    N      2023  10.00     1
//! A    N      2024  30.00     3
//! A    N      2025   NULL  NULL
//! A    S      2023   NULL  NULL
//! A    S      2024  20.00     2
//! A    S      2025   NULL  NULL
//! B    W      2023   5.50     5
//! B    W      2024  -5.50    -5
//! B    E      2024   NULL  NULL
//! ```
//!
//! `sum(units)` by zone and region, across the years:
//!
//! ```text
//!            2023  2024  2025   row total
//! A N           1     3  NULL       4
//! A S        NULL     2  NULL       2
//! A             1     5  NULL       6
//! B E        NULL  NULL  NULL    NULL   (E has no row in 2023 and 2025)
//! B W           5    -5  NULL       0   (W has no row in 2025)
//! B             5    -5  NULL       0
//! Total         6     0  NULL       6
//! ```
//!
//! So there is a row whose total is zero (W, and B), one whose total is NULL
//! (E), a column whose total is zero (2024) and one whose total is NULL (2025).

use opengrid_conformance::block_on;
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_pivot::{PivotError, PivotLimits, PivotQuery, PivotResult, execute};
use opengrid_query::Limits;
use opengrid_types::{DataType, Decimal, Field, FieldName, Schema, Value};

const TABLE: &str = "zone,region,year,amount,units
A,N,2023,10.00,1
A,N,2024,30.00,3
A,N,2025,\\N,\\N
A,S,2023,\\N,\\N
A,S,2024,20.00,2
A,S,2025,\\N,\\N
B,W,2023,5.50,5
B,W,2024,-5.50,-5
B,E,2024,\\N,\\N
";

fn name(text: &str) -> FieldName {
    FieldName::new(text).expect("a field name")
}

fn schema() -> Schema {
    Schema::new(vec![
        Field::new(name("zone"), DataType::Utf8),
        Field::new(name("region"), DataType::Utf8),
        Field::new(name("year"), DataType::Int64),
        Field::new(
            name("amount"),
            DataType::Decimal {
                precision: 10,
                scale: 2,
            },
        ),
        Field::new(name("units"), DataType::Int64),
    ])
}

fn validate(
    values: &str,
    limits: &PivotLimits,
) -> Result<opengrid_pivot::ValidatedPivotQuery, PivotError> {
    let pivot: PivotQuery = opengrid_json::from_str(&format!(
        r#"{{"source":"sales","rows":["zone","region"],"columns":["year"],"values":{values}}}"#
    ))
    .expect("a pivot");
    pivot.validate(&schema(), limits, &Limits::default())
}

fn answer(values: &str) -> PivotResult {
    let table = load_csv(TABLE.as_bytes(), &schema(), CsvOptions::default()).expect("the table");
    let validated = validate(values, &PivotLimits::default()).expect("a valid pivot");
    block_on(execute(&LocalDataSource::new(table), &validated)).expect("the engine answers")
}

/// The seven rows, in the pivot's order.
const ROWS: [&str; 7] = ["A N", "A S", "A", "B E", "B W", "B", "Total"];

/// The cells of the generated columns, row by row, against `expected` —
/// floats to within 1e-12, everything else exactly.
fn assert_cells(result: &PivotResult, expected: [[Value; 3]; 7]) {
    assert_eq!(result.row_levels, vec![2, 2, 1, 2, 2, 1, 0]);
    for (row, cells) in expected.iter().enumerate() {
        for (column, wanted) in cells.iter().enumerate() {
            let actual = &result.data.columns[2 + column][row];
            let same = match (actual, wanted) {
                (Value::Float64(a), Value::Float64(b)) => (a - b).abs() <= 1e-12,
                _ => actual == wanted,
            };
            assert!(
                same,
                "{}, column {}: expected {wanted:?}, got {actual:?}",
                ROWS[row],
                2023 + column
            );
        }
    }
}

const NULL: Value = Value::Null;

fn f(number: f64) -> Value {
    Value::Float64(number)
}

fn int(number: i64) -> Value {
    Value::Int64(number)
}

/// Hundredths as a decimal of scale 2: `dec(4000)` is 40.00.
fn dec(hundredths: i128) -> Value {
    Value::Decimal(Decimal::new(hundredths, 2))
}

/// The table itself, before any calculation — the numbers the rest divides.
#[test]
fn the_plain_sums_are_the_table_above() {
    let result = answer(r#"[{"field":"units","fn":"sum","as":"u"}]"#);
    assert_cells(
        &result,
        [
            [int(1), int(3), NULL],
            [NULL, int(2), NULL],
            [int(1), int(5), NULL],
            [NULL, NULL, NULL],
            [int(5), int(-5), NULL],
            [int(5), int(-5), NULL],
            [int(6), int(0), NULL],
        ],
    );
}

/// P11: each cell over its row's total. W's and B's total is zero and E's is
/// NULL, so their shares are NULL; the grand total row divides by 6.
#[test]
fn a_share_of_the_row_total() {
    let result = answer(r#"[{"field":"units","fn":"sum","as":"s","calc":"share_of_row"}]"#);
    assert_cells(
        &result,
        [
            [f(0.25), f(0.75), NULL],
            [NULL, f(1.0), NULL],
            [f(1.0 / 6.0), f(5.0 / 6.0), NULL],
            [NULL, NULL, NULL],
            [NULL, NULL, NULL],
            [NULL, NULL, NULL],
            [f(1.0), f(0.0), NULL],
        ],
    );
    for field in &result.data.schema.fields()[2..] {
        assert_eq!(field.data_type, DataType::Float64, "a share is a fraction");
    }
    // A row's shares add up to one, where it has a total to share.
    for row in [0, 1, 2, 6] {
        let sum: f64 = (2..5)
            .filter_map(|column| match result.data.columns[column][row] {
                Value::Float64(share) => Some(share),
                _ => None,
            })
            .sum();
        assert!((sum - 1.0).abs() <= 1e-12, "{}: {sum}", ROWS[row]);
    }
}

/// P11: each cell over its column's cell in the grand total row — 6, 0 and
/// NULL. Only 2023 has a total to share; the total row itself is 1 there.
#[test]
fn a_share_of_the_column_total() {
    let result = answer(r#"[{"field":"units","fn":"sum","as":"s","calc":"share_of_column"}]"#);
    assert_cells(
        &result,
        [
            [f(1.0 / 6.0), NULL, NULL],
            [NULL, NULL, NULL],
            [f(1.0 / 6.0), NULL, NULL],
            [NULL, NULL, NULL],
            [f(5.0 / 6.0), NULL, NULL],
            [f(5.0 / 6.0), NULL, NULL],
            [f(1.0), NULL, NULL],
        ],
    );
}

/// P11: each cell over the grand total, 6. A negative cell is a negative
/// share; the subtotals follow the same rule.
#[test]
fn a_share_of_the_grand_total() {
    let result = answer(r#"[{"field":"units","fn":"sum","as":"s","calc":"share_of_total"}]"#);
    assert_cells(
        &result,
        [
            [f(1.0 / 6.0), f(0.5), NULL],
            [NULL, f(1.0 / 3.0), NULL],
            [f(1.0 / 6.0), f(5.0 / 6.0), NULL],
            [NULL, NULL, NULL],
            [f(5.0 / 6.0), f(-5.0 / 6.0), NULL],
            [f(5.0 / 6.0), f(-5.0 / 6.0), NULL],
            [f(1.0), f(0.0), NULL],
        ],
    );
}

/// P11 over a count: an empty cell counts 0, and 0 of a total is 0, not NULL.
/// `count(*)`: N 1·1·1, S 1·1·1, A 2·2·2, E 0·1·0, W 1·1·0, B 1·2·0.
#[test]
fn a_share_of_a_count_is_zero_where_nothing_was_counted() {
    let result = answer(r#"[{"fn":"count","as":"s","calc":"share_of_row"}]"#);
    let third = 1.0 / 3.0;
    assert_cells(
        &result,
        [
            [f(third), f(third), f(third)],
            [f(third), f(third), f(third)],
            [f(third), f(third), f(third)],
            [f(0.0), f(1.0), f(0.0)],
            [f(0.5), f(0.5), f(0.0)],
            [f(third), f(2.0 * third), f(0.0)],
            [f(third), f(4.0 / 9.0), f(2.0 / 9.0)],
        ],
    );
}

/// P12 over `sum(amount)`: 10.00·30.00·NULL for N, NULL·20.00·NULL for S,
/// 5.50·-5.50 for W. S is NULL before its first value and keeps 20.00 after
/// it; E never has one. Decimals stay decimals, to the cent.
#[test]
fn a_running_total_along_the_years() {
    let result = answer(r#"[{"field":"amount","fn":"sum","as":"r","calc":"running_total"}]"#);
    assert_cells(
        &result,
        [
            [dec(1000), dec(4000), dec(4000)],
            [NULL, dec(2000), dec(2000)],
            [dec(1000), dec(6000), dec(6000)],
            [NULL, NULL, NULL],
            [dec(550), dec(0), dec(0)],
            [dec(550), dec(0), dec(0)],
            [dec(1550), dec(6000), dec(6000)],
        ],
    );
    for field in &result.data.schema.fields()[2..] {
        assert_eq!(
            field.data_type,
            DataType::Decimal {
                precision: 38,
                scale: 2
            },
            "the type of the sum (S12)"
        );
    }
}

/// P12 over an integer sum and over a count, side by side with the plain sum:
/// a calculation changes its own measure's cells and no other's.
#[test]
fn a_running_total_beside_its_measure() {
    let result = answer(
        r#"[{"field":"units","fn":"sum","as":"u"},
            {"field":"units","fn":"sum","as":"r","calc":"running_total"},
            {"fn":"count","as":"n","calc":"running_total"}]"#,
    );
    // Per year: units, running units, running count.
    let expected: [[Value; 9]; 7] = [
        [
            int(1),
            int(1),
            int(1),
            int(3),
            int(4),
            int(2),
            NULL,
            int(4),
            int(3),
        ],
        [
            NULL,
            NULL,
            int(1),
            int(2),
            int(2),
            int(2),
            NULL,
            int(2),
            int(3),
        ],
        [
            int(1),
            int(1),
            int(2),
            int(5),
            int(6),
            int(4),
            NULL,
            int(6),
            int(6),
        ],
        [NULL, NULL, int(0), NULL, NULL, int(1), NULL, NULL, int(1)],
        [
            int(5),
            int(5),
            int(1),
            int(-5),
            int(0),
            int(2),
            NULL,
            int(0),
            int(2),
        ],
        [
            int(5),
            int(5),
            int(1),
            int(-5),
            int(0),
            int(3),
            NULL,
            int(0),
            int(3),
        ],
        [
            int(6),
            int(6),
            int(3),
            int(0),
            int(6),
            int(7),
            NULL,
            int(6),
            int(9),
        ],
    ];
    for (row, cells) in expected.iter().enumerate() {
        for (column, wanted) in cells.iter().enumerate() {
            assert_eq!(
                &result.data.columns[2 + column][row],
                wanted,
                "{}, cell {column}",
                ROWS[row]
            );
        }
    }
    let measures: Vec<&str> = result
        .columns
        .iter()
        .take(3)
        .map(|column| column.measure.as_str())
        .collect();
    assert_eq!(measures, ["u", "r", "n"]);
}

/// P10: a share is defined for `sum` and `count`. The average, the smallest
/// and the largest of the parts are not those of the whole, so a share of
/// them is refused — by name, before anything runs. A running total adds up
/// what the cells show and takes all five.
#[test]
fn a_share_is_refused_for_what_cannot_be_added_up() {
    for function in ["avg", "min", "max"] {
        for calc in ["share_of_row", "share_of_column", "share_of_total"] {
            let error = validate(
                &format!(r#"[{{"field":"units","fn":"{function}","as":"m","calc":"{calc}"}}]"#),
                &PivotLimits::default(),
            )
            .expect_err("refused");
            let message = error.to_string();
            assert!(
                matches!(error, PivotError::Calc { .. })
                    && message.contains("measure m")
                    && message.contains(calc)
                    && message.contains(function),
                "{function}/{calc}: {message}"
            );
        }
    }
    for function in ["sum", "count"] {
        assert!(
            validate(
                &format!(
                    r#"[{{"field":"units","fn":"{function}","as":"m","calc":"share_of_row"}}]"#
                ),
                &PivotLimits::default(),
            )
            .is_ok(),
            "{function}"
        );
    }
    for function in ["sum", "count", "avg", "min", "max"] {
        for field in ["units", "amount"] {
            assert!(
                validate(
                    &format!(
                        r#"[{{"field":"{field}","fn":"{function}","as":"m","calc":"running_total"}}]"#
                    ),
                    &PivotLimits::default(),
                )
                .is_ok(),
                "a running total of {function}({field})"
            );
        }
    }
}

/// P10: a running total is a sum, so the cells have to be numbers. The
/// smallest or largest text cannot be added up; counting it can.
#[test]
fn a_running_total_of_what_is_not_a_number_is_refused() {
    for function in ["min", "max"] {
        let error = validate(
            &format!(r#"[{{"field":"region","fn":"{function}","as":"m","calc":"running_total"}}]"#),
            &PivotLimits::default(),
        )
        .expect_err("text is not added up");
        let message = error.to_string();
        assert!(
            matches!(error, PivotError::Calc { .. })
                && message.contains("measure m")
                && message.contains("running_total needs a number")
                && message.contains(&format!("{function} of region")),
            "{function}: {message}"
        );
        // Without the calculation the same measure is fine.
        assert!(
            validate(
                &format!(r#"[{{"field":"region","fn":"{function}","as":"m"}}]"#),
                &PivotLimits::default()
            )
            .is_ok()
        );
    }
    assert!(
        validate(
            r#"[{"field":"region","fn":"count","as":"m","calc":"running_total"}]"#,
            &PivotLimits::default()
        )
        .is_ok(),
        "a count of text is a number"
    );
}

/// P12 over an average: the cells are `avg(units)` — N 1·3·NULL, S NULL·2·NULL,
/// A 1·2.5·NULL, W 5·-5, B 5·-5·NULL, Total 3·0·NULL — and the running total
/// is their sum so far, a float. Each row adds up its own cells: the total
/// row's 3·3·3 is not the sum of the rows above it.
#[test]
fn a_running_total_of_averages() {
    let result = answer(r#"[{"field":"units","fn":"avg","as":"r","calc":"running_total"}]"#);
    assert_cells(
        &result,
        [
            [f(1.0), f(4.0), f(4.0)],
            [NULL, f(2.0), f(2.0)],
            [f(1.0), f(3.5), f(3.5)],
            [NULL, NULL, NULL],
            [f(5.0), f(0.0), f(0.0)],
            [f(5.0), f(0.0), f(0.0)],
            [f(3.0), f(3.0), f(3.0)],
        ],
    );
    for field in &result.data.schema.fields()[2..] {
        assert_eq!(field.data_type, DataType::Float64, "the type of an average");
    }
}

/// P12 over the smallest and the largest value. `min(units)`: N 1·3·NULL,
/// S NULL·2·NULL, A 1·2·NULL, W 5·-5, B 5·-5·NULL, Total 1·-5·NULL.
/// `max(amount)`: N 10.00·30.00·NULL, S NULL·20.00·NULL, A 10.00·30.00·NULL,
/// W 5.50·-5.50, B the same, Total 10.00·30.00·NULL. The type stays the
/// column's: an integer, and the decimal of ten digits.
#[test]
fn a_running_total_of_the_smallest_and_the_largest() {
    let least = answer(r#"[{"field":"units","fn":"min","as":"r","calc":"running_total"}]"#);
    assert_cells(
        &least,
        [
            [int(1), int(4), int(4)],
            [NULL, int(2), int(2)],
            [int(1), int(3), int(3)],
            [NULL, NULL, NULL],
            [int(5), int(0), int(0)],
            [int(5), int(0), int(0)],
            [int(1), int(-4), int(-4)],
        ],
    );
    let most = answer(r#"[{"field":"amount","fn":"max","as":"r","calc":"running_total"}]"#);
    assert_cells(
        &most,
        [
            [dec(1000), dec(4000), dec(4000)],
            [NULL, dec(2000), dec(2000)],
            [dec(1000), dec(4000), dec(4000)],
            [NULL, NULL, NULL],
            [dec(550), dec(0), dec(0)],
            [dec(550), dec(0), dec(0)],
            [dec(1000), dec(4000), dec(4000)],
        ],
    );
    for field in &most.data.schema.fields()[2..] {
        assert_eq!(
            field.data_type,
            DataType::Decimal {
                precision: 10,
                scale: 2
            },
            "min and max keep the column's type (S12)"
        );
    }
}

/// The largest value keeps its column's ten digits, so a running total of
/// two of the largest amounts that fit does not: refused, not widened.
#[test]
fn a_running_total_past_its_decimals_digits_is_refused() {
    let csv = "zone,region,year,amount,units\nA,N,2023,99999999.99,1\nA,N,2024,0.01,1\n";
    let run = |values: &str| {
        let table = load_csv(csv.as_bytes(), &schema(), CsvOptions::default()).expect("the table");
        let validated = validate(values, &PivotLimits::default()).expect("valid");
        block_on(execute(&LocalDataSource::new(table), &validated))
    };
    let message = run(r#"[{"field":"amount","fn":"max","as":"r","calc":"running_total"}]"#)
        .expect_err("eleven digits in a decimal of ten")
        .to_string();
    assert!(
        message.contains("measure r") && message.contains("running_total"),
        "{message}"
    );
    // The sum has 38 digits (S12), and there it fits.
    assert!(run(r#"[{"field":"amount","fn":"sum","as":"r","calc":"running_total"}]"#).is_ok());
}

/// A calculation the wire does not know is not a pivot: the reader says which
/// ones there are.
#[test]
fn an_unknown_calculation_does_not_read() {
    let error = opengrid_json::from_str::<PivotQuery>(
        r#"{"source":"sales","values":[{"fn":"count","as":"n","calc":"share_of_parent"}]}"#,
    )
    .expect_err("not a calculation");
    let message = error.to_string();
    assert!(
        message.contains("share_of_parent") && message.contains("share_of_row"),
        "{message}"
    );
}

/// `calc` travels with its measure and is left out where there is none.
#[test]
fn a_calculated_measure_reads_and_writes_itself() {
    let json = r#"{"source":"sales","rows":["zone"],"columns":["year"],"values":[{"field":"units","fn":"sum","as":"u"},{"field":"units","fn":"sum","as":"s","calc":"running_total"}]}"#;
    let pivot: PivotQuery = opengrid_json::from_str(json).expect("a pivot");
    let written = opengrid_json::to_string(&pivot);
    assert_eq!(written.matches("calc").count(), 1, "{written}");
    assert!(written.contains(r#""calc":"running_total""#), "{written}");
    assert_eq!(
        opengrid_json::from_str::<PivotQuery>(&written).expect("reads back"),
        pivot
    );
}

/// P7 with a calculation: it changes cells, never their number, so the limits
/// count a calculated measure as the column it is — no more, no fewer.
#[test]
fn a_calculation_adds_no_rows_and_no_columns() {
    let plain = answer(r#"[{"field":"units","fn":"sum","as":"s"}]"#);
    let shared = answer(r#"[{"field":"units","fn":"sum","as":"s","calc":"share_of_row"}]"#);
    assert_eq!(shared.columns, plain.columns);
    assert_eq!(shared.row_levels, plain.row_levels);

    let fits = PivotLimits {
        max_columns: 3,
        max_rows: 7,
        max_cells: 21,
        ..PivotLimits::default()
    };
    let values = r#"[{"field":"units","fn":"sum","as":"s","calc":"share_of_row"}]"#;
    let table = || load_csv(TABLE.as_bytes(), &schema(), CsvOptions::default()).expect("the table");
    let run = |limits: PivotLimits| {
        let validated = validate(values, &limits).expect("valid");
        block_on(execute(&LocalDataSource::new(table()), &validated))
    };
    assert!(run(fits).is_ok(), "three columns, seven rows, 21 cells");
    assert!(
        run(PivotLimits {
            max_cells: 20,
            ..fits
        })
        .is_err(),
        "one cell fewer than it has"
    );
}

/// A running total that outgrows its type is an error, not a wrapped number.
#[test]
fn a_running_total_past_its_type_is_refused() {
    let csv = format!(
        "zone,region,year,amount,units\nA,N,2023,1.00,{}\nA,N,2024,1.00,1\n",
        i64::MAX
    );
    let table = load_csv(csv.as_bytes(), &schema(), CsvOptions::default()).expect("the table");
    let validated = validate(
        r#"[{"field":"units","fn":"sum","as":"r","calc":"running_total"}]"#,
        &PivotLimits::default(),
    )
    .expect("valid");
    let message = block_on(execute(&LocalDataSource::new(table), &validated))
        .expect_err("past 64 bits")
        .to_string();
    assert!(
        message.contains("measure r") && message.contains("running_total"),
        "{message}"
    );
}
