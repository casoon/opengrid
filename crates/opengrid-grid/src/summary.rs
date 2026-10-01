//! What a group row or the total shows for a column (point 63) — the token a
//! page and a view name it with. Portable since issue #144.

use opengrid_query::AggregateFn;

/// What a group row shows for a column (points 63 and F7).
///
/// One of the query model's aggregates, or a **range**: the smallest and the
/// largest value, shown as "from – to" (F7, decided 2026-09-24 — the prototype
/// shows the dates a group spans). A range is not a new aggregate of the query
/// model: it is asked as `min` and `max`, and only the grid puts them together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summary {
    Fn(AggregateFn),
    Range,
}

impl Summary {
    /// The token a page and a view use.
    pub fn as_str(self) -> &'static str {
        match self {
            Summary::Fn(function) => function.as_str(),
            Summary::Range => "range",
        }
    }

    /// The query aggregates it is asked as, in order.
    pub fn functions(self) -> &'static [AggregateFn] {
        match self {
            Summary::Fn(AggregateFn::Count) => &[AggregateFn::Count],
            Summary::Fn(AggregateFn::Sum) => &[AggregateFn::Sum],
            Summary::Fn(AggregateFn::Avg) => &[AggregateFn::Avg],
            Summary::Fn(AggregateFn::Min) => &[AggregateFn::Min],
            Summary::Fn(AggregateFn::Max) => &[AggregateFn::Max],
            Summary::Range => &[AggregateFn::Min, AggregateFn::Max],
        }
    }
}

pub fn aggregate_from(token: &str) -> Option<Summary> {
    Some(match token {
        "sum" => Summary::Fn(AggregateFn::Sum),
        "avg" => Summary::Fn(AggregateFn::Avg),
        "count" => Summary::Fn(AggregateFn::Count),
        "min" => Summary::Fn(AggregateFn::Min),
        "max" => Summary::Fn(AggregateFn::Max),
        "range" => Summary::Range,
        _ => return None,
    })
}
