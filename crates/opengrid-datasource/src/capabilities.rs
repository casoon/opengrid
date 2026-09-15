/// What a data source can answer.
///
/// The planner splits a query between client and server from these flags
/// (plan/spezifikation/03-datasource.md §Capabilities, point 28). They are a
/// declaration, not a promise: a source that reports a capability and then fails
/// a query for it reports that as a [`DataSourceError`](crate::DataSourceError).
///
/// The local engine reports [`ALL`](Self::ALL) — everything runs in WASM
/// (point 09, step 3). PostgreSQL reports everything but `pivot` and
/// `calculated_fields` (point 24).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DataSourceCapabilities {
    pub filter: bool,
    pub sort: bool,
    pub group: bool,
    pub aggregate: bool,
    pub paging: bool,
    pub pivot: bool,
    pub calculated_fields: bool,
    pub streaming: bool,
}

impl DataSourceCapabilities {
    /// Every capability: the in-memory engine answers all operations itself.
    pub const ALL: Self = Self {
        filter: true,
        sort: true,
        group: true,
        aggregate: true,
        paging: true,
        pivot: true,
        calculated_fields: true,
        streaming: true,
    };
}
