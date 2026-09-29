use opengrid_json::{Error, Fields, FromJson, Json, ToJson};

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
/// The flags travel: a browser that plans against a remote source has to be
/// told what that source can do (plan point 28). Every field defaults to `false`
/// on the way in, so a reader that learns a capability later still parses an
/// older declaration.
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

const FLAGS: [&str; 8] = [
    "filter",
    "sort",
    "group",
    "aggregate",
    "paging",
    "pivot",
    "calculated_fields",
    "streaming",
];

impl FromJson for DataSourceCapabilities {
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(json, "struct DataSourceCapabilities", &FLAGS)?;
        Ok(Self {
            filter: fields.read_or_default("filter")?,
            sort: fields.read_or_default("sort")?,
            group: fields.read_or_default("group")?,
            aggregate: fields.read_or_default("aggregate")?,
            paging: fields.read_or_default("paging")?,
            pivot: fields.read_or_default("pivot")?,
            calculated_fields: fields.read_or_default("calculated_fields")?,
            streaming: fields.read_or_default("streaming")?,
        })
    }
}

impl ToJson for DataSourceCapabilities {
    fn to_json(&self) -> Json {
        opengrid_json::json!({
            "filter": self.filter,
            "sort": self.sort,
            "group": self.group,
            "aggregate": self.aggregate,
            "paging": self.paging,
            "pivot": self.pivot,
            "calculated_fields": self.calculated_fields,
            "streaming": self.streaming,
        })
    }
}
