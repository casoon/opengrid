//! `opengrid-planner` — who does which part of a query (plan point 28,
//! plan/spezifikation/05-planner.md).
//!
//! A query names operations; a source declares what it can do
//! ([`DataSourceCapabilities`]). The planner turns those two into an
//! [`ExecutionPlan`]: the part the source runs, and the steps the client runs on
//! what comes back. The plan is a value, not a side effect — it can be read,
//! printed and asserted on, which is what 05-planner.md means by "offengelegt
//! für Debugging".
//!
//! # What V1 can actually split
//!
//! 05-planner.md motivates hybrid execution with a calculated field
//! (`margin / revenue` computed in the browser, so `limit` must not be pushed).
//! **The query model of V1 has no calculated fields** — no expressions beyond
//! filter comparisons and aggregates — so that particular split cannot occur
//! yet. What can occur is a source that lacks a capability: a source without
//! `group` leaves grouping to the client, one without `sort` leaves sorting, and
//! so on.
//!
//! The rule about paging survives that change of cast, and in a stronger form:
//!
//! > `offset`/`limit` may be pushed only when **no** client step is left that
//! > changes which rows there are or what order they are in.
//!
//! Filtering, grouping and aggregating change the set; sorting changes the
//! order. Paging before any of them would page the wrong rows — the same reason
//! 05-planner.md gives for its example, applied to every case that exists today.

use opengrid_datasource::DataSourceCapabilities;
use opengrid_query::ValidatedQuery;

/// Work the client does after the source answered, in the order it happens.
///
/// The order is the query pipeline's: filter, group and aggregate, sort, page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientStep {
    Filter,
    Group,
    Aggregate,
    Sort,
    Page,
}

impl ClientStep {
    /// Whether this step changes which rows exist, or in what order.
    ///
    /// The question paging depends on: everything except paging itself does.
    fn reshapes_rows(self) -> bool {
        !matches!(self, ClientStep::Page)
    }
}

/// Where a query is executed (the `mode` attribute of the element).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Everything runs in the client. The source is asked for rows only.
    Local,
    /// Everything runs in the source; a missing capability is an error rather
    /// than a silent fallback.
    Remote,
    /// Push what the source can do, keep the rest.
    Hybrid,
    /// Decide from the capabilities — which is [`Hybrid`](Self::Hybrid), and
    /// lands on "everything remote" whenever the source can do everything.
    #[default]
    Auto,
}

/// Who does what.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionPlan {
    /// The query handed to the source.
    pub source_query: ValidatedQuery,
    /// What the client does with the answer, in order.
    pub client_steps: Vec<ClientStep>,
    /// The mode this plan was made for.
    pub mode: ExecutionMode,
}

impl ExecutionPlan {
    /// Whether the source answers the whole query by itself.
    pub fn is_fully_pushed(&self) -> bool {
        self.client_steps.is_empty()
    }

    /// A one-line summary for developer tools and logs.
    pub fn describe(&self) -> String {
        let mut source = Vec::new();
        if self.source_query.filter.is_some() {
            source.push("filter");
        }
        if !self.source_query.group.is_empty() {
            source.push("group");
        }
        if !self.source_query.aggregate.is_empty() {
            source.push("aggregate");
        }
        if !self.source_query.sort.is_empty() {
            source.push("sort");
        }
        if self.source_query.limit.is_some() || self.source_query.offset.is_some() {
            source.push("page");
        }
        let client: Vec<&str> = self
            .client_steps
            .iter()
            .map(|step| match step {
                ClientStep::Filter => "filter",
                ClientStep::Group => "group",
                ClientStep::Aggregate => "aggregate",
                ClientStep::Sort => "sort",
                ClientStep::Page => "page",
            })
            .collect();
        format!(
            "source: {} | client: {}",
            if source.is_empty() {
                "scan".to_owned()
            } else {
                source.join(" · ")
            },
            if client.is_empty() {
                "—".to_owned()
            } else {
                client.join(" · ")
            }
        )
    }
}

/// Why a query cannot be planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// `mode="remote"` was asked for, and the source cannot do all of it.
    NotPushable { operation: &'static str },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::NotPushable { operation } => write!(
                f,
                "the source cannot {operation}, which mode=\"remote\" requires"
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// Splits `query` between the source and the client.
pub fn plan(
    query: &ValidatedQuery,
    capabilities: DataSourceCapabilities,
    mode: ExecutionMode,
) -> Result<ExecutionPlan, PlanError> {
    // `Local` means the client is the one with the data: the source is asked for
    // rows, everything else happens here.
    let effective = match mode {
        ExecutionMode::Local => DataSourceCapabilities::default(),
        _ => capabilities,
    };

    if mode == ExecutionMode::Remote {
        for (wanted, can, name) in [
            (query.filter.is_some(), capabilities.filter, "filter"),
            (!query.group.is_empty(), capabilities.group, "group"),
            (
                !query.aggregate.is_empty(),
                capabilities.aggregate,
                "aggregate",
            ),
            (!query.sort.is_empty(), capabilities.sort, "sort"),
            (
                query.limit.is_some() || query.offset.is_some(),
                capabilities.paging,
                "page",
            ),
        ] {
            if wanted && !can {
                return Err(PlanError::NotPushable { operation: name });
            }
        }
    }

    let mut source_query = query.clone();
    let mut client_steps = Vec::new();

    if query.filter.is_some() && !effective.filter {
        source_query.filter = None;
        client_steps.push(ClientStep::Filter);
    }
    if !query.group.is_empty() && !effective.group {
        source_query.group = Vec::new();
        client_steps.push(ClientStep::Group);
    }
    if !query.aggregate.is_empty() && !effective.aggregate {
        source_query.aggregate = Vec::new();
        client_steps.push(ClientStep::Aggregate);
    }
    // Grouping and aggregating belong together: half of them in each place would
    // mean the source returns groups the client then aggregates again.
    if client_steps.contains(&ClientStep::Group) && !query.aggregate.is_empty() {
        source_query.aggregate = Vec::new();
        if !client_steps.contains(&ClientStep::Aggregate) {
            client_steps.push(ClientStep::Aggregate);
        }
    }
    // A sort must follow whatever produced the rows it sorts. When the client
    // groups or aggregates, the source's rows are not the rows being sorted —
    // and a sort key may name an aggregate alias that does not exist before the
    // aggregation at all. This is the rule of 05-planner.md ("sorting depends on
    // a client-computed field"), and grouping is the case V1 actually has.
    let rows_change_locally =
        client_steps.contains(&ClientStep::Group) || client_steps.contains(&ClientStep::Aggregate);
    if !query.sort.is_empty() && (!effective.sort || rows_change_locally) {
        source_query.sort = Vec::new();
        client_steps.push(ClientStep::Sort);
    }

    let pages = query.limit.is_some() || query.offset.is_some();
    // The rule of 05-planner.md, generalised: paging may only be pushed when
    // nothing is left that changes the rows or their order. Paging first and
    // filtering afterwards answers a different question.
    let reshaped_locally = client_steps.iter().any(|step| step.reshapes_rows());
    if pages && (!effective.paging || reshaped_locally) {
        source_query.limit = None;
        source_query.offset = None;
        client_steps.push(ClientStep::Page);
    }

    // Client steps run in pipeline order, whatever order they were found in.
    client_steps.sort_by_key(|step| match step {
        ClientStep::Filter => 0,
        ClientStep::Group => 1,
        ClientStep::Aggregate => 2,
        ClientStep::Sort => 3,
        ClientStep::Page => 4,
    });

    Ok(ExecutionPlan {
        source_query,
        client_steps,
        mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_query::{Limits, Query};
    use opengrid_types::{DataType, Field, FieldName, Schema};

    fn schema() -> Schema {
        Schema::new(vec![
            Field::required(FieldName::new("id").unwrap(), DataType::Int64),
            Field::new(FieldName::new("country").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("amount").unwrap(), DataType::Int64),
        ])
    }

    /// A query with filter, sort and paging.
    fn query(json: &str) -> ValidatedQuery {
        let query: Query = serde_json::from_str(json).expect("valid query JSON");
        query
            .validate(&schema(), &Limits::default())
            .expect("a valid query")
    }

    fn paged() -> ValidatedQuery {
        query(
            r#"{"source":"orders","select":["id"],
                "filter":{"field":"country","op":"eq","value":"DE"},
                "sort":[{"field":"id","direction":"asc"}],
                "offset":10,"limit":20}"#,
        )
    }

    fn grouped() -> ValidatedQuery {
        query(
            r#"{"source":"orders","select":["country"],
                "group":["country"],
                "aggregate":[{"field":"amount","fn":"sum","as":"total"}],
                "sort":[{"field":"total","direction":"desc"}],
                "limit":5}"#,
        )
    }

    fn without(capability: &str) -> DataSourceCapabilities {
        let mut caps = DataSourceCapabilities::ALL;
        match capability {
            "filter" => caps.filter = false,
            "sort" => caps.sort = false,
            "group" => caps.group = false,
            "aggregate" => caps.aggregate = false,
            "paging" => caps.paging = false,
            other => panic!("unknown capability {other}"),
        }
        caps
    }

    /// A source that can do everything gets everything — that is what `auto`
    /// means when nothing stands in the way.
    #[test]
    fn a_capable_source_answers_the_whole_query() {
        let plan = plan(&paged(), DataSourceCapabilities::ALL, ExecutionMode::Auto).unwrap();
        assert!(plan.is_fully_pushed());
        assert_eq!(plan.source_query, paged());
        assert_eq!(plan.describe(), "source: filter · sort · page | client: —");
    }

    /// `local` keeps everything here, whatever the source could do.
    #[test]
    fn local_mode_pushes_nothing() {
        let plan = plan(&paged(), DataSourceCapabilities::ALL, ExecutionMode::Local).unwrap();
        assert_eq!(plan.source_query.filter, None);
        assert!(plan.source_query.sort.is_empty());
        assert_eq!(plan.source_query.limit, None);
        assert_eq!(
            plan.client_steps,
            [ClientStep::Filter, ClientStep::Sort, ClientStep::Page]
        );
    }

    /// `remote` says what it cannot do instead of quietly doing less.
    #[test]
    fn remote_mode_refuses_what_the_source_cannot_do() {
        assert_eq!(
            plan(&paged(), without("sort"), ExecutionMode::Remote),
            Err(PlanError::NotPushable { operation: "sort" })
        );
        // The same query is fine when the source can do it all.
        assert!(plan(&paged(), DataSourceCapabilities::ALL, ExecutionMode::Remote).is_ok());
    }

    /// **The rule of 05-planner.md.** A sort left to the client means paging
    /// must stay with it — paging first would page the unsorted rows.
    #[test]
    fn paging_follows_the_last_step_that_reshapes_the_rows() {
        let plan = plan(&paged(), without("sort"), ExecutionMode::Auto).unwrap();

        assert!(
            plan.source_query.limit.is_none() && plan.source_query.offset.is_none(),
            "limit must not be pushed past a client-side sort"
        );
        assert_eq!(plan.client_steps, [ClientStep::Sort, ClientStep::Page]);
        // The filter *was* pushed: it does not depend on the sort.
        assert!(plan.source_query.filter.is_some());
    }

    /// The same holds for a client-side filter: paging a filtered set is not
    /// paging the unfiltered one.
    #[test]
    fn a_client_filter_also_holds_paging_back() {
        let plan = plan(&paged(), without("filter"), ExecutionMode::Auto).unwrap();
        assert_eq!(plan.source_query.limit, None);
        // The sort stays with the source: filtering afterwards keeps the order
        // it produced, so only the paging has to wait for the client.
        assert_eq!(plan.client_steps, [ClientStep::Filter, ClientStep::Page]);
        assert!(!plan.source_query.sort.is_empty());
    }

    /// A source that can page but has nothing left to page *after* — everything
    /// else was pushed — keeps the paging.
    #[test]
    fn paging_stays_pushed_when_nothing_is_left_to_do() {
        let plan = plan(&paged(), DataSourceCapabilities::ALL, ExecutionMode::Hybrid).unwrap();
        assert_eq!(plan.source_query.limit, Some(20));
        assert_eq!(plan.source_query.offset, Some(10));
        assert!(plan.client_steps.is_empty());
    }

    /// Grouping and aggregating are one step in two names: a source that cannot
    /// group must not aggregate either, or the client would aggregate aggregates.
    #[test]
    fn grouping_and_aggregating_stay_together() {
        let plan = plan(&grouped(), without("group"), ExecutionMode::Auto).unwrap();
        assert!(plan.source_query.group.is_empty());
        assert!(
            plan.source_query.aggregate.is_empty(),
            "an aggregate without its grouping would be a different query"
        );
        assert_eq!(
            plan.client_steps,
            [
                ClientStep::Group,
                ClientStep::Aggregate,
                ClientStep::Sort,
                ClientStep::Page
            ]
        );
    }

    /// A sort key that only exists after a client-side aggregation cannot be
    /// pushed — the source has no column of that name to sort by.
    #[test]
    fn a_sort_over_an_aggregate_follows_the_aggregation() {
        let plan = plan(&grouped(), without("group"), ExecutionMode::Auto).unwrap();
        assert!(
            plan.source_query.sort.is_empty(),
            "sorting by \"total\" before the totals exist is not the same query"
        );
        assert!(plan.client_steps.contains(&ClientStep::Sort));
    }

    /// The plan reads as a sentence — this is what a developer tool shows.
    #[test]
    fn a_plan_describes_itself() {
        let plan = plan(&grouped(), without("sort"), ExecutionMode::Auto).unwrap();
        assert_eq!(
            plan.describe(),
            "source: group · aggregate | client: sort · page"
        );
    }
}
