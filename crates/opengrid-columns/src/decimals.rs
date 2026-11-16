//! Decimal coefficients stored as wide as the column's precision needs.
//!
//! An exact decimal is a coefficient (`12.34` at scale 2 is `1234`). Up to 18 digits of precision
//! it fits an `i64`, and an amount, a rate or a price is `decimal(12, 2)`: 8 bytes a row hold what
//! 16 held (issue #450). Above 18 digits the coefficient is an `i128`.
//!
//! The *type* decides, not the data: a column of precision 18 or less is narrow from its first
//! value to its last, so there is no widening and no surprise for a reader. [`Decimals::get`] is
//! the `i128` that was pushed, whichever storage holds it.

/// The most digits a coefficient can have and still fit an `i64`.
const NARROW_DIGITS: u8 = 18;

/// The coefficients of a decimal column.
#[derive(Clone, Debug, PartialEq)]
pub enum Decimals {
    /// Precision of at most 18 digits.
    I64(Vec<i64>),
    /// Precision of 19 to 38 digits.
    I128(Vec<i128>),
}

impl Decimals {
    /// An empty buffer for a column of `precision` digits.
    pub fn with_capacity(precision: u8, capacity: usize) -> Self {
        if precision <= NARROW_DIGITS {
            Decimals::I64(Vec::with_capacity(capacity))
        } else {
            Decimals::I128(Vec::with_capacity(capacity))
        }
    }

    /// The coefficients of `values`, in the storage a column of `precision` digits uses. A value
    /// that does not fit a narrow storage is an error.
    pub fn from_vec(precision: u8, values: Vec<i128>) -> Result<Self, String> {
        if precision > NARROW_DIGITS {
            return Ok(Decimals::I128(values));
        }
        values
            .into_iter()
            .map(|value| i64::try_from(value).map_err(|_| too_wide(precision)))
            .collect::<Result<_, _>>()
            .map(Decimals::I64)
    }

    pub fn len(&self) -> usize {
        match self {
            Decimals::I64(values) => values.len(),
            Decimals::I128(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the coefficients are stored as `i64`.
    pub fn is_narrow(&self) -> bool {
        matches!(self, Decimals::I64(_))
    }

    /// The coefficient at `row`.
    #[inline]
    pub fn get(&self, row: usize) -> i128 {
        match self {
            Decimals::I64(values) => i128::from(values[row]),
            Decimals::I128(values) => values[row],
        }
    }

    /// Appends a coefficient; one a narrow storage cannot hold is an error.
    #[inline]
    pub fn push(&mut self, value: i128) -> Result<(), String> {
        match self {
            Decimals::I64(values) => {
                values.push(i64::try_from(value).map_err(|_| too_wide(NARROW_DIGITS))?)
            }
            Decimals::I128(values) => values.push(value),
        }
        Ok(())
    }

    /// Appends the placeholder under a NULL.
    pub fn push_zero(&mut self) {
        match self {
            Decimals::I64(values) => values.push(0),
            Decimals::I128(values) => values.push(0),
        }
    }

    /// The bytes the coefficients take.
    pub fn byte_size(&self) -> usize {
        match self {
            Decimals::I64(values) => values.len() * 8,
            Decimals::I128(values) => values.len() * 16,
        }
    }

    /// The coefficients at `positions`, in that order.
    pub fn gather(&self, positions: impl Iterator<Item = usize>) -> Self {
        match self {
            Decimals::I64(values) => Decimals::I64(positions.map(|at| values[at]).collect()),
            Decimals::I128(values) => Decimals::I128(positions.map(|at| values[at]).collect()),
        }
    }

    /// All coefficients, in row order.
    pub fn iter(&self) -> impl Iterator<Item = i128> + '_ {
        (0..self.len()).map(|row| self.get(row))
    }
}

fn too_wide(precision: u8) -> String {
    format!("a coefficient has more than {precision} digits")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_precision_decides_the_storage() {
        assert!(Decimals::with_capacity(1, 0).is_narrow());
        assert!(Decimals::with_capacity(18, 0).is_narrow());
        assert!(!Decimals::with_capacity(19, 0).is_narrow());
        assert!(!Decimals::with_capacity(38, 0).is_narrow());
    }

    #[test]
    fn a_narrow_storage_takes_eight_bytes_a_value_and_a_wide_one_sixteen() {
        let mut narrow = Decimals::with_capacity(12, 3);
        let mut wide = Decimals::with_capacity(30, 3);
        for value in [-5, 0, 7] {
            narrow.push(value).unwrap();
            wide.push(value).unwrap();
        }
        assert_eq!(narrow.byte_size(), 24);
        assert_eq!(wide.byte_size(), 48);
    }

    #[test]
    fn the_ends_of_eighteen_and_nineteen_digits_read_back() {
        let nines_18 = 10i128.pow(18) - 1;
        let nines_19 = 10i128.pow(19) - 1;
        let mut narrow = Decimals::with_capacity(18, 2);
        narrow.push(nines_18).unwrap();
        narrow.push(-nines_18).unwrap();
        assert_eq!(narrow.iter().collect::<Vec<_>>(), [nines_18, -nines_18]);

        let mut wide = Decimals::with_capacity(19, 2);
        wide.push(nines_19).unwrap();
        wide.push(-nines_19).unwrap();
        assert_eq!(wide.iter().collect::<Vec<_>>(), [nines_19, -nines_19]);
    }

    #[test]
    fn a_narrow_storage_refuses_what_does_not_fit() {
        let mut narrow = Decimals::with_capacity(18, 0);
        assert!(narrow.push(i128::from(i64::MAX) + 1).is_err());
        assert!(narrow.push(i128::from(i64::MIN) - 1).is_err());
        assert_eq!(narrow.len(), 0, "a refused value is not stored");
        assert!(Decimals::from_vec(18, vec![i128::from(i64::MAX) + 1]).is_err());
        assert!(Decimals::from_vec(19, vec![i128::from(i64::MAX) + 1]).is_ok());
    }

    #[test]
    fn gather_keeps_the_storage_and_the_order() {
        let narrow = Decimals::from_vec(12, vec![10, 20, 30]).unwrap();
        let picked = narrow.gather([2, 0, 0].into_iter());
        assert!(picked.is_narrow());
        assert_eq!(picked.iter().collect::<Vec<_>>(), [30, 10, 10]);
        let wide = Decimals::from_vec(30, vec![10, 20, 30]).unwrap();
        assert!(!wide.gather([1].into_iter()).is_narrow());
    }
}
