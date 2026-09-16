//! Repo tooling and the **deterministic** synthetic dataset behind the benchmarks
//! (plan point 11, step 1).
//!
//! `orders_csv(rows, seed)` answers the same bytes for the same arguments on every
//! machine — a fixed-seed `splitmix64`, no `rand`, no clock, no hash iteration
//! order. Native criterion benches and the WASM benches call it directly, so both
//! sides measure the same rows; the `xtask` binary writes it to a file for humans.
//!
//! The columns mirror the conformance fixture (`crates/opengrid-conformance/data/orders.csv`):
//! an `Int64` id, the nullable measurement columns, a `Decimal(12, 2)` amount, a
//! `Date` and a µs `Timestamp`. Every nullable column carries NULLs — the ingest
//! path must be measured with them, not on a clean grid. `\N` is the NULL marker.

use opengrid_types::{DataType, Date, Decimal, Field, FieldName, Schema, Timestamp};

/// The schema [`orders_csv`] writes.
pub fn orders_schema() -> Schema {
    fn name(value: &str) -> FieldName {
        FieldName::new(value).expect("a valid identifier")
    }
    Schema::new(vec![
        Field::required(name("id"), DataType::Int64),
        Field::new(name("customer"), DataType::Utf8),
        Field::new(name("country"), DataType::Utf8),
        Field::new(
            name("amount"),
            DataType::decimal(12, 2).expect("12,2 is a valid decimal"),
        ),
        Field::new(name("qty"), DataType::Int64),
        Field::new(name("ratio"), DataType::Float64),
        Field::new(name("flag"), DataType::Bool),
        Field::new(name("ordered_on"), DataType::Date),
        Field::new(name("created_at"), DataType::Timestamp),
        Field::new(name("note"), DataType::Utf8),
    ])
}

/// A header line for [`orders_csv`], derived from [`orders_schema`].
pub fn orders_header() -> String {
    orders_schema()
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

const CUSTOMERS: [&str; 8] = [
    "Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta", "Theta",
];
const COUNTRIES: [&str; 4] = ["DE", "FR", "US", "GB"];
const NOTE_ALPHABET: &[u8] = b"abcdefgxyz";

/// Deterministic `rows`-row orders dataset as CSV, NULL as `\N`.
///
/// The generator streams the text; one million rows are about 30 MiB of CSV. The
/// operations the benches run (filter on `country`, sort on `amount`, group by
/// `country`) see a realistic spread: every country appears, amounts cover
/// negative and large positive values.
pub fn orders_csv(rows: usize, seed: u64) -> String {
    let mut rng = Rng::new(seed);
    let mut out = String::with_capacity(rows * 32 + 64);
    out.push_str(&orders_header());
    out.push('\n');

    // 2025-01-01, the lower bound of the `Date` and `Timestamp` columns.
    const BASE_DAYS: i32 = 20_089;
    const BASE_MICROS: i64 = 20_089 * 86_400 * 1_000_000;

    for row in 0..rows {
        let id = row as i64 + 1;

        let customer = if rng.chance(1, 32) {
            NULL
        } else {
            CUSTOMERS[(rng.below(CUSTOMERS.len() as u64)) as usize]
        };

        let country = if rng.chance(1, 64) {
            NULL
        } else {
            COUNTRIES[(rng.below(COUNTRIES.len() as u64)) as usize]
        };

        let amount = if rng.chance(1, 16) {
            NULL.to_owned()
        } else {
            // Coefficients in [-1234567, 99999999999] at scale 2 → [-12345.67, 999999999.99].
            let coefficient = rng.below(100_001_234_567) as i128 - 1_234_567;
            Decimal::new(coefficient, 2).to_string()
        };

        let qty = if rng.chance(1, 16) {
            NULL.to_owned()
        } else {
            (rng.below(1_000) + 1).to_string()
        };

        let ratio = if rng.chance(1, 16) {
            NULL.to_owned()
        } else if rng.chance(1, 256) {
            // S7: `NaN` travels as a string, not as `null`.
            "NaN".to_owned()
        } else {
            let magnitude = rng.below(1_000_000) as f64 / 1000.0;
            let sign = if rng.chance(1, 2) { -1.0 } else { 1.0 };
            format!("{:.3}", sign * magnitude)
        };

        let flag = if rng.chance(1, 16) {
            NULL.to_owned()
        } else if rng.chance(1, 2) {
            "true".to_owned()
        } else {
            "false".to_owned()
        };

        let ordered_on = if rng.chance(1, 16) {
            NULL.to_owned()
        } else {
            let days = BASE_DAYS + rng.below(730) as i32;
            Date::from_days_since_epoch(days).to_string()
        };

        let created_at = if rng.chance(1, 16) {
            NULL.to_owned()
        } else {
            let micros = BASE_MICROS + rng.below(730 * 86_400 * 1_000_000) as i64;
            Timestamp::from_micros(micros).to_string()
        };

        let note = if rng.chance(1, 16) {
            NULL.to_owned()
        } else {
            let length = rng.below(6) as usize;
            let mut note = String::with_capacity(length);
            for _ in 0..length {
                note.push(NOTE_ALPHABET[rng.below(NOTE_ALPHABET.len() as u64) as usize] as char);
            }
            note
        };

        // No field contains a separator, a quote or a newline by construction, so a
        // plain join is a valid CSV row here.
        out.push_str(&format!(
            "{id},{customer},{country},{amount},{qty},{ratio},{flag},{ordered_on},{created_at},{note}\n"
        ));
    }
    out
}

const NULL: &str = r"\N";

/// `splitmix64` — small, fast, deterministic, and dependency-free.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        debug_assert!(bound > 0);
        self.next_u64() % bound
    }

    /// True with probability `1/denominator`.
    fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        self.below(denominator) < numerator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(orders_csv(100, 7), orders_csv(100, 7));
        assert_ne!(orders_csv(100, 7), orders_csv(100, 8));
    }

    #[test]
    fn rows_have_the_header_width() {
        let csv = orders_csv(50, 1);
        let width = orders_header().split(',').count();
        let mut lines = csv.lines();
        assert_eq!(lines.next().unwrap(), orders_header());
        for line in lines {
            let cells: Vec<&str> = line.split(',').collect();
            assert_eq!(cells.len(), width, "row {line:?}");
        }
    }

    #[test]
    fn nullable_columns_carry_null_markers() {
        let csv = orders_csv(500, 1);
        // Every nullable column (`customer` … `note`) has at least one `\N`.
        let mut lines = csv.lines().skip(1);
        let first = lines.next().unwrap();
        let width = first.split(',').count();
        let mut nulls = vec![false; width];
        for line in std::iter::once(first).chain(lines) {
            for (column, cell) in line.split(',').enumerate() {
                if cell == NULL {
                    nulls[column] = true;
                }
            }
        }
        assert!(!nulls[0], "id is not nullable");
        for (column, has_null) in nulls.iter().enumerate().skip(1) {
            assert!(has_null, "column {column} has no NULL in 500 rows");
        }
    }
}
