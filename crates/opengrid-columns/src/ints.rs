//! Integers stored as wide as their values need.
//!
//! An `int64` column of quantities, years or ids does not hold sixty-four bits in a cell: a count of
//! 0–39 fits one byte and an id up to a million fits four. [`Ints`] keeps the values in the
//! narrowest of `i8`, `i16`, `i32` and `i64` that holds all of them (issue #447). A builder starts
//! narrow and widens when a value does not fit — once, and early for most data — so the buffer that
//! is allocated at the end is the one that is kept, not a wide one first and a narrow one after: the
//! engine module's memory does not shrink, so only what is never allocated is saved.
//!
//! What a reader sees does not change: [`Ints::get`] is the `i64` that was pushed. The width
//! follows from the data (the smallest that holds every value, whatever the order they came in),
//! and equality is by value.

/// A buffer of `i64` values in the narrowest width that holds them all.
#[derive(Clone, Debug)]
pub enum Ints {
    I8(Vec<i8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
}

/// The bytes one value takes in each width.
const WIDTHS: [usize; 4] = [1, 2, 4, 8];

impl Ints {
    /// An empty buffer that will hold `capacity` values, in the narrowest width.
    pub fn with_capacity(capacity: usize) -> Self {
        Ints::I8(Vec::with_capacity(capacity))
    }

    /// The values of `values`, in the narrowest width that holds them.
    pub fn from_vec(values: Vec<i64>) -> Self {
        let (low, high) = values.iter().fold((0, 0), |(low, high), value| {
            (low.min(*value), high.max(*value))
        });
        match Self::rank(low).max(Self::rank(high)) {
            0 => Ints::I8(values.iter().map(|value| *value as i8).collect()),
            1 => Ints::I16(values.iter().map(|value| *value as i16).collect()),
            2 => Ints::I32(values.iter().map(|value| *value as i32).collect()),
            _ => Ints::I64(values),
        }
    }

    /// The width, as an index into [`WIDTHS`], that holds `value`.
    fn rank(value: i64) -> usize {
        if i8::try_from(value).is_ok() {
            0
        } else if i16::try_from(value).is_ok() {
            1
        } else if i32::try_from(value).is_ok() {
            2
        } else {
            3
        }
    }

    fn current(&self) -> usize {
        match self {
            Ints::I8(_) => 0,
            Ints::I16(_) => 1,
            Ints::I32(_) => 2,
            Ints::I64(_) => 3,
        }
    }

    /// Number of values.
    pub fn len(&self) -> usize {
        match self {
            Ints::I8(values) => values.len(),
            Ints::I16(values) => values.len(),
            Ints::I32(values) => values.len(),
            Ints::I64(values) => values.len(),
        }
    }

    /// True when there are no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes the values take.
    pub fn byte_size(&self) -> usize {
        self.len() * WIDTHS[self.current()]
    }

    /// The value at `row`.
    #[inline]
    pub fn get(&self, row: usize) -> i64 {
        match self {
            Ints::I8(values) => i64::from(values[row]),
            Ints::I16(values) => i64::from(values[row]),
            Ints::I32(values) => i64::from(values[row]),
            Ints::I64(values) => values[row],
        }
    }

    /// The values, in order.
    pub fn iter(&self) -> impl Iterator<Item = i64> + '_ {
        (0..self.len()).map(|row| self.get(row))
    }

    /// The values as a `Vec<i64>`.
    pub fn to_vec(&self) -> Vec<i64> {
        self.iter().collect()
    }

    /// Appends a value, widening the buffer when it does not fit.
    #[inline]
    pub fn push(&mut self, value: i64) {
        match self {
            Ints::I8(values) => {
                if let Ok(narrow) = i8::try_from(value) {
                    return values.push(narrow);
                }
            }
            Ints::I16(values) => {
                if let Ok(narrow) = i16::try_from(value) {
                    return values.push(narrow);
                }
            }
            Ints::I32(values) => {
                if let Ok(narrow) = i32::try_from(value) {
                    return values.push(narrow);
                }
            }
            Ints::I64(values) => return values.push(value),
        }
        self.widen(Self::rank(value));
        self.push(value);
    }

    /// Moves the values into a buffer of width `rank`, which keeps the capacity the old one had.
    #[cold]
    fn widen(&mut self, rank: usize) {
        let capacity = match self {
            Ints::I8(values) => values.capacity(),
            Ints::I16(values) => values.capacity(),
            Ints::I32(values) => values.capacity(),
            Ints::I64(values) => values.capacity(),
        }
        .max(self.len() + 1);
        let mut wider = match rank {
            1 => Ints::I16(Vec::with_capacity(capacity)),
            2 => Ints::I32(Vec::with_capacity(capacity)),
            _ => Ints::I64(Vec::with_capacity(capacity)),
        };
        for row in 0..self.len() {
            match &mut wider {
                Ints::I16(values) => values.push(self.get(row) as i16),
                Ints::I32(values) => values.push(self.get(row) as i32),
                Ints::I64(values) => values.push(self.get(row)),
                Ints::I8(_) => unreachable!("a buffer is only ever widened"),
            }
        }
        *self = wider;
    }
}

impl FromIterator<i64> for Ints {
    fn from_iter<I: IntoIterator<Item = i64>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut out = Ints::with_capacity(iter.size_hint().0);
        for value in iter {
            out.push(value);
        }
        out
    }
}

/// By value: two buffers that hold the same integers are equal whatever their widths.
impl PartialEq for Ints {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Ints::I8(a), Ints::I8(b)) => a == b,
            (Ints::I16(a), Ints::I16(b)) => a == b,
            (Ints::I32(a), Ints::I32(b)) => a == b,
            (Ints::I64(a), Ints::I64(b)) => a == b,
            _ => self.len() == other.len() && self.iter().eq(other.iter()),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Values around every edge of every width, and the ends of `i64`.
    fn edges() -> Vec<i64> {
        let mut out = vec![0, 1, -1, i64::MIN, i64::MAX, i64::MIN + 1, i64::MAX - 1];
        for bits in [7u32, 15, 31] {
            let limit = 1i64 << bits;
            out.extend([limit - 1, limit, -limit, -limit - 1]);
        }
        out
    }

    #[test]
    fn a_value_is_read_back_at_every_edge_of_every_width() {
        let mut ints = Ints::with_capacity(0);
        let values = edges();
        for value in &values {
            ints.push(*value);
        }
        assert_eq!(ints.to_vec(), values);
        // Each value alone takes the narrowest width that holds it, at the edges of every width.
        for (value, bytes) in [
            (0, 1),
            (127, 1),
            (-128, 1),
            (128, 2),
            (-129, 2),
            (32_767, 2),
            (-32_768, 2),
            (32_768, 4),
            (-32_769, 4),
            (i64::from(i32::MAX), 4),
            (i64::from(i32::MIN), 4),
            (i64::from(i32::MAX) + 1, 8),
            (i64::from(i32::MIN) - 1, 8),
            (i64::MAX, 8),
            (i64::MIN, 8),
        ] {
            let alone = Ints::from_vec(vec![value]);
            assert_eq!(alone.byte_size(), bytes, "{value}");
            assert_eq!(alone.get(0), value);
            let mut pushed = Ints::with_capacity(1);
            pushed.push(value);
            assert_eq!(pushed.byte_size(), bytes, "{value} pushed");
        }
    }

    #[test]
    fn the_width_follows_the_data_not_the_order() {
        let narrow_first = [1, 2, 3, 70_000, 4];
        let wide_first = [70_000, 1, 2, 3, 4];
        let a: Ints = narrow_first.into_iter().collect();
        let b: Ints = wide_first.into_iter().collect();
        assert!(matches!((&a, &b), (Ints::I32(_), Ints::I32(_))));
        assert_eq!(a.byte_size(), 5 * 4);
        // A column of small numbers stays small, and its byte size says so.
        let small: Ints = (0..1000).map(|value| value % 40).collect();
        assert_eq!(small.byte_size(), 1000);
    }

    #[test]
    fn equal_values_are_equal_whatever_the_width() {
        let narrow = Ints::I8(vec![1, 2, 3]);
        let wide = Ints::I64(vec![1, 2, 3]);
        assert_eq!(narrow, wide);
        assert_ne!(narrow, Ints::I64(vec![1, 2, 4]));
        assert_ne!(narrow, Ints::I64(vec![1, 2]));
    }

    #[test]
    fn widening_keeps_the_capacity_it_was_given() {
        let mut ints = Ints::with_capacity(1000);
        ints.push(1);
        ints.push(1 << 40);
        let Ints::I64(values) = &ints else {
            panic!("widened to i64")
        };
        assert!(values.capacity() >= 1000, "{}", values.capacity());
    }

    proptest! {
        /// Whatever integers are pushed, in whatever order, they are read back in order, and
        /// `from_vec` and the iterator make the same buffer as pushing.
        #[test]
        fn pushed_integers_are_read_back(values in prop::collection::vec(
            prop_oneof![
                -3i64..3,
                any::<i8>().prop_map(i64::from),
                any::<i16>().prop_map(i64::from),
                any::<i32>().prop_map(i64::from),
                any::<i64>(),
                Just(i64::MIN),
                Just(i64::MAX),
            ],
            0..200,
        )) {
            let mut pushed = Ints::with_capacity(values.len());
            for value in &values {
                pushed.push(*value);
            }
            prop_assert_eq!(pushed.to_vec(), values.clone());
            prop_assert_eq!(pushed.len(), values.len());
            let made = Ints::from_vec(values.clone());
            prop_assert_eq!(&pushed, &made);
            prop_assert_eq!(pushed.byte_size(), made.byte_size());
            let collected: Ints = values.iter().copied().collect();
            prop_assert_eq!(&collected, &made);
            // The width is the narrowest that holds every value.
            let fits = |low: i64, high: i64| values.iter().all(|value| (low..=high).contains(value));
            let bytes = if fits(i64::from(i8::MIN), i64::from(i8::MAX)) {
                1
            } else if fits(i64::from(i16::MIN), i64::from(i16::MAX)) {
                2
            } else if fits(i64::from(i32::MIN), i64::from(i32::MAX)) {
                4
            } else {
                8
            };
            prop_assert_eq!(pushed.byte_size(), values.len() * bytes);
        }
    }
}
