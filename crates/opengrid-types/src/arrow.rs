//! Mapping between [`DataType`] and `arrow_schema::DataType`.
//!
//! Only active with the `arrow` feature and only depends on `arrow-schema`
//! (plan/spezifikation/14-entscheidungen.md E3).

use std::sync::Arc;

use arrow_schema::{DataType as ArrowDataType, TimeUnit};

use crate::DataType;

/// The Arrow timestamp timezone used for opengrid timestamps, which are always
/// UTC (semantics rule S9).
const UTC: &str = "UTC";

impl From<DataType> for ArrowDataType {
    fn from(data_type: DataType) -> Self {
        match data_type {
            DataType::Bool => ArrowDataType::Boolean,
            DataType::Int64 => ArrowDataType::Int64,
            DataType::Float64 => ArrowDataType::Float64,
            DataType::Decimal { precision, scale } => {
                ArrowDataType::Decimal128(precision, scale as i8)
            }
            DataType::Utf8 => ArrowDataType::Utf8,
            DataType::Date => ArrowDataType::Date32,
            DataType::Timestamp => {
                ArrowDataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from(UTC)))
            }
        }
    }
}

impl DataType {
    /// Maps an Arrow type back to an opengrid type, or `None` when opengrid V1
    /// has no equivalent.
    pub fn from_arrow(data_type: &ArrowDataType) -> Option<Self> {
        match data_type {
            ArrowDataType::Boolean => Some(DataType::Bool),
            ArrowDataType::Int64 => Some(DataType::Int64),
            ArrowDataType::Float64 => Some(DataType::Float64),
            ArrowDataType::Decimal128(precision, scale) if *scale >= 0 => {
                DataType::decimal(*precision, *scale as u8).ok()
            }
            ArrowDataType::Utf8 => Some(DataType::Utf8),
            ArrowDataType::Date32 => Some(DataType::Date),
            ArrowDataType::Timestamp(TimeUnit::Microsecond, _) => Some(DataType::Timestamp),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_type() -> Vec<DataType> {
        vec![
            DataType::Bool,
            DataType::Int64,
            DataType::Float64,
            DataType::decimal(38, 0).unwrap(),
            DataType::decimal(12, 2).unwrap(),
            DataType::Utf8,
            DataType::Date,
            DataType::Timestamp,
        ]
    }

    #[test]
    fn every_type_maps_to_arrow_and_back() {
        for data_type in every_type() {
            let arrow: ArrowDataType = data_type.into();
            assert_eq!(DataType::from_arrow(&arrow), Some(data_type), "{data_type}");
        }
    }

    #[test]
    fn expected_arrow_types() {
        assert_eq!(ArrowDataType::from(DataType::Bool), ArrowDataType::Boolean);
        assert_eq!(ArrowDataType::from(DataType::Int64), ArrowDataType::Int64);
        assert_eq!(
            ArrowDataType::from(DataType::Float64),
            ArrowDataType::Float64
        );
        assert_eq!(ArrowDataType::from(DataType::Utf8), ArrowDataType::Utf8);
        assert_eq!(ArrowDataType::from(DataType::Date), ArrowDataType::Date32);
        assert_eq!(
            ArrowDataType::from(DataType::decimal(12, 2).unwrap()),
            ArrowDataType::Decimal128(12, 2)
        );
        assert_eq!(
            ArrowDataType::from(DataType::Timestamp),
            ArrowDataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from("UTC")))
        );
    }

    #[test]
    fn unsupported_arrow_types_map_to_none() {
        assert_eq!(DataType::from_arrow(&ArrowDataType::Int32), None);
        assert_eq!(DataType::from_arrow(&ArrowDataType::LargeUtf8), None);
        assert_eq!(
            DataType::from_arrow(&ArrowDataType::Decimal128(10, -1)),
            None
        );
    }
}
