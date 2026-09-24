//! Per-column presentation (plan point 60): `set_columns`.
//!
//! # The rule this module exists to enforce
//!
//! > The configuration **narrows**. It never widens.
//!
//! Three layers, and the order is also the right of way (point 56 §Das Modell):
//! the **schema** is the truth, the **configuration** narrows it, the reader's
//! **view** is what they then chose. So a page may say "show `amount` right
//! aligned, 150 pixels wide, summed in groups" — and may not say "sum this text
//! column", because summing text is not a preference, it is a type error.
//!
//! Two consequences, both deliberate:
//!
//! * **A name that is not in the schema is an error**, reported in the status
//!   line. Silently ignoring it leaves a grid that looks configured and is not,
//!   and nobody finds the typo.
//! * **What the schema already answers is not configurable.** Which filter
//!   operators a column offers, and the default alignment, follow from the type.
//!   A page may override the alignment (it is taste) and may not override the
//!   operators (they are meaning).
//!
//! # What is presentation and what is not
//!
//! `mono`, `emphasis` and `muted` exist because the prototype's `id` is
//! monospaced and grey, its `customer` is bold, and **no schema says so**. That
//! is the whole test for whether something belongs here: could the grid work it
//! out from the data? If yes, it is not configuration.

use opengrid_query::AggregateFn;
use opengrid_types::{DataType, Schema};

/// Which way a column's values line up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    End,
    Center,
}

impl Align {
    /// The wire name, which is also the `data-align` value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Align::Start => "start",
            Align::End => "end",
            Align::Center => "center",
        }
    }

    fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "start" => Align::Start,
            "end" => Align::End,
            "center" => Align::Center,
            _ => return None,
        })
    }

    /// What a type lines up like when nobody said otherwise.
    ///
    /// Numbers right, everything else left — so digits of the same magnitude
    /// stand under each other and can be compared by eye.
    pub fn of(data_type: DataType) -> Self {
        if is_numeric(data_type) {
            Align::End
        } else {
            Align::Start
        }
    }
}

/// How a column is offered as a facet (used by point 66).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacetKind {
    /// Checkboxes with counts.
    List,
    /// Toggle buttons, for a handful of values.
    Pills,
    /// A minimum and a maximum.
    Range,
    /// A from and a to date.
    Period,
}

impl FacetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FacetKind::List => "list",
            FacetKind::Pills => "pills",
            FacetKind::Range => "range",
            FacetKind::Period => "period",
        }
    }

    fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "list" => FacetKind::List,
            "pills" => FacetKind::Pills,
            "range" => FacetKind::Range,
            "period" => FacetKind::Period,
            _ => return None,
        })
    }

    /// The facet a type offers by itself.
    pub fn of(data_type: DataType) -> Option<Self> {
        Some(match data_type {
            DataType::Utf8 | DataType::Bool => FacetKind::List,
            DataType::Int64 | DataType::Float64 | DataType::Decimal { .. } => FacetKind::Range,
            DataType::Date | DataType::Timestamp => FacetKind::Period,
        })
    }

    /// Whether this facet makes sense for this type.
    fn fits(&self, data_type: DataType) -> bool {
        match self {
            FacetKind::List | FacetKind::Pills => {
                matches!(data_type, DataType::Utf8 | DataType::Bool)
            }
            FacetKind::Range => is_numeric(data_type),
            FacetKind::Period => matches!(data_type, DataType::Date | DataType::Timestamp),
        }
    }
}

/// True for the three types a sum or an average means something for.
pub fn is_numeric(data_type: DataType) -> bool {
    matches!(
        data_type,
        DataType::Int64 | DataType::Float64 | DataType::Decimal { .. }
    )
}

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

/// The aggregates a type allows (used by point 63).
///
/// `count` everywhere — counting rows is not a question about the type. `sum`
/// and `avg` only where adding is defined. `min`/`max` — and the range made of
/// both — where there is an order worth taking the end of; `Bool` has two
/// values and no useful extreme.
pub fn aggregates_for(data_type: DataType) -> Vec<Summary> {
    let mut out = vec![Summary::Fn(AggregateFn::Count)];
    if is_numeric(data_type) {
        out.push(Summary::Fn(AggregateFn::Sum));
        out.push(Summary::Fn(AggregateFn::Avg));
    }
    if is_numeric(data_type) || matches!(data_type, DataType::Date | DataType::Timestamp) {
        out.push(Summary::Fn(AggregateFn::Min));
        out.push(Summary::Fn(AggregateFn::Max));
        out.push(Summary::Range);
    }
    out
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

/// Everything a page may say about one column.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnPresentation {
    /// Starting width in pixels. The reader's resize leads after that — the
    /// same attribute/value relationship the view has (point 59).
    pub width: Option<u32>,
    /// Overrides the alignment the type would give.
    pub align: Option<Align>,
    /// Draw the values in the monospaced face, so they line up character by
    /// character. Ids and codes; not prose.
    pub mono: bool,
    /// Draw the values bold: the column a row is recognised by.
    pub emphasis: bool,
    /// Draw the values in the muted ink: present, but not the point.
    pub muted: bool,
    /// The aggregate this column starts with in groups (point 63).
    pub aggregate: Option<Summary>,
    /// How this column is offered as a facet (point 66).
    pub facet: Option<FacetKind>,
}

/// A configuration this grid refuses, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentationProblem {
    pub column: String,
    pub reason: String,
}

impl PresentationProblem {
    fn new(column: &str, reason: impl Into<String>) -> Self {
        Self {
            column: column.to_owned(),
            reason: reason.into(),
        }
    }
}

/// One entry of a `set_columns` call, before it has met the schema.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawColumn {
    pub width: Option<u32>,
    pub align: Option<String>,
    pub mono: Option<bool>,
    pub emphasis: Option<bool>,
    pub muted: Option<bool>,
    pub aggregate: Option<String>,
    pub facet: Option<String>,
}

/// Checks a whole configuration against the schema.
///
/// Answers the accepted presentations, or **every** problem at once — one round
/// trip through the status line beats finding the mistakes one release apart.
pub fn validate(
    raw: &[(String, RawColumn)],
    schema: &Schema,
    declared: &[String],
) -> Result<Vec<(String, ColumnPresentation)>, Vec<PresentationProblem>> {
    let mut out = Vec::new();
    let mut problems = Vec::new();

    for (name, entry) in raw {
        let field = schema
            .fields()
            .iter()
            .find(|field| field.name.as_str() == name);
        if field.is_none() && !declared.iter().any(|column| column == name) {
            problems.push(PresentationProblem::new(
                name,
                "is not a column of this grid",
            ));
            continue;
        }
        // A declared column missing from the schema is **hidden**, not unknown:
        // the result carries only what the query selected. Its type-dependent
        // checks wait until it is shown again — hiding a column is an ordinary
        // thing to do (point 65), and a configuration that turned into an error
        // when the reader hid a column would be unusable.
        let data_type = field.map(|field| field.data_type);
        let mut column = ColumnPresentation {
            width: entry.width,
            mono: entry.mono.unwrap_or(false),
            emphasis: entry.emphasis.unwrap_or(false),
            muted: entry.muted.unwrap_or(false),
            ..ColumnPresentation::default()
        };

        if let Some(token) = &entry.align {
            match Align::parse(token) {
                Some(align) => column.align = Some(align),
                None => problems.push(PresentationProblem::new(
                    name,
                    format!("{token} is not an alignment"),
                )),
            }
        }

        if let Some(token) = &entry.aggregate {
            match aggregate_from(token) {
                Some(aggregate)
                    if data_type
                        .is_none_or(|data_type| aggregates_for(data_type).contains(&aggregate)) =>
                {
                    column.aggregate = Some(aggregate);
                }
                Some(_) => problems.push(PresentationProblem::new(
                    name,
                    format!("{token} is not an aggregate for this type"),
                )),
                None => problems.push(PresentationProblem::new(
                    name,
                    format!("{token} is not an aggregate"),
                )),
            }
        }

        if let Some(token) = &entry.facet {
            match FacetKind::parse(token) {
                Some(facet) if data_type.is_none_or(|data_type| facet.fits(data_type)) => {
                    column.facet = Some(facet)
                }
                Some(_) => problems.push(PresentationProblem::new(
                    name,
                    format!("{token} is not a facet for this type"),
                )),
                None => problems.push(PresentationProblem::new(
                    name,
                    format!("{token} is not a facet"),
                )),
            }
        }

        out.push((name.clone(), column));
    }

    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

/// The presentation of every column of a host, already checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnStyles {
    by_name: Vec<(String, ColumnPresentation)>,
}

impl ColumnStyles {
    pub fn new(entries: Vec<(String, ColumnPresentation)>) -> Self {
        Self { by_name: entries }
    }

    /// What a page said about this column, if anything.
    pub fn get(&self, name: &str) -> Option<&ColumnPresentation> {
        self.by_name
            .iter()
            .find(|(column, _)| column == name)
            .map(|(_, presentation)| presentation)
    }

    /// The `data-` markers a cell of this column carries, as `(name, value)`.
    ///
    /// Markers rather than inline styles, so the shadow stylesheet decides what
    /// "muted" looks like and a page can still reach the cell through
    /// `::part(cell)`.
    pub fn markers(&self, name: &str, data_type: DataType) -> Vec<(&'static str, String)> {
        let presentation = self.get(name);
        let align = presentation
            .and_then(|column| column.align)
            .unwrap_or_else(|| Align::of(data_type));
        let mut markers = vec![("data-align", align.as_str().to_owned())];
        if let Some(column) = presentation {
            if column.mono {
                markers.push(("data-mono", "true".to_owned()));
            }
            if column.emphasis {
                markers.push(("data-emphasis", "true".to_owned()));
            }
            if column.muted {
                markers.push(("data-muted", "true".to_owned()));
            }
        }
        markers
    }

    /// The starting width a page gave this column, if any.
    /// The facets a page configured, in its order (point 66).
    pub fn facets(&self) -> Vec<(String, FacetKind)> {
        self.by_name
            .iter()
            .filter_map(|(name, column)| column.facet.map(|kind| (name.clone(), kind)))
            .collect()
    }

    pub fn width(&self, name: &str) -> Option<u32> {
        self.get(name).and_then(|column| column.width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{Field, FieldName};

    fn schema() -> Schema {
        Schema::new(vec![
            Field::new(FieldName::new("id").unwrap(), DataType::Int64),
            Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("amount").unwrap(), DataType::Float64),
            Field::new(FieldName::new("ordered_on").unwrap(), DataType::Date),
        ])
    }

    fn raw(entry: RawColumn) -> Vec<(String, RawColumn)> {
        vec![("amount".to_owned(), entry)]
    }

    /// The whole point: what the schema forbids, the configuration cannot buy.
    #[test]
    fn a_sum_over_text_is_refused() {
        let problems = validate(
            &[(
                "customer".to_owned(),
                RawColumn {
                    aggregate: Some("sum".to_owned()),
                    ..RawColumn::default()
                },
            )],
            &schema(),
            &[],
        )
        .expect_err("is refused");
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].column, "customer");
        assert!(problems[0].reason.contains("sum"));
    }

    /// `count` is a question about rows, not about the type, so it is allowed
    /// everywhere — including on text.
    #[test]
    fn counting_is_allowed_on_every_type() {
        for column in ["id", "customer", "amount", "ordered_on"] {
            let ok = validate(
                &[(
                    column.to_owned(),
                    RawColumn {
                        aggregate: Some("count".to_owned()),
                        ..RawColumn::default()
                    },
                )],
                &schema(),
                &[],
            );
            assert!(ok.is_ok(), "count refused on {column}");
        }
    }

    /// A range over text, a list over a number: both are type errors dressed as
    /// preferences.
    #[test]
    fn a_facet_that_does_not_fit_the_type_is_refused() {
        for (column, facet) in [("customer", "range"), ("amount", "list"), ("id", "period")] {
            let problems = validate(
                &[(
                    column.to_owned(),
                    RawColumn {
                        facet: Some(facet.to_owned()),
                        ..RawColumn::default()
                    },
                )],
                &schema(),
                &[],
            )
            .expect_err("is refused");
            assert!(
                problems[0].reason.contains(facet),
                "{column}/{facet}: {problems:?}"
            );
        }
    }

    /// A typo in a column name is the mistake this catches most often, and the
    /// one silence hides best.
    #[test]
    fn an_unknown_column_is_named() {
        let problems = validate(
            &[("amuont".to_owned(), RawColumn::default())],
            &schema(),
            &[],
        )
        .expect_err("is refused");
        assert_eq!(problems[0].column, "amuont");
    }

    /// Every problem at once: finding them one release apart is the failure
    /// mode this avoids.
    #[test]
    fn all_the_problems_are_reported_together() {
        let problems = validate(
            &[
                ("nope".to_owned(), RawColumn::default()),
                (
                    "customer".to_owned(),
                    RawColumn {
                        aggregate: Some("sum".to_owned()),
                        align: Some("sideways".to_owned()),
                        ..RawColumn::default()
                    },
                ),
            ],
            &schema(),
            &[],
        )
        .expect_err("is refused");
        assert_eq!(problems.len(), 3);
    }

    /// Alignment follows the type unless a page says otherwise — it is taste,
    /// not meaning.
    #[test]
    fn numbers_line_up_right_and_text_left() {
        let styles = ColumnStyles::new(
            validate(&raw(RawColumn::default()), &schema(), &[]).expect("is accepted"),
        );
        assert_eq!(
            styles.markers("amount", DataType::Float64)[0],
            ("data-align", "end".to_owned())
        );
        assert_eq!(
            styles.markers("customer", DataType::Utf8)[0],
            ("data-align", "start".to_owned())
        );

        let overridden = ColumnStyles::new(
            validate(
                &raw(RawColumn {
                    align: Some("start".to_owned()),
                    ..RawColumn::default()
                }),
                &schema(),
                &[],
            )
            .expect("is accepted"),
        );
        assert_eq!(
            overridden.markers("amount", DataType::Float64)[0],
            ("data-align", "start".to_owned())
        );
    }

    /// The three that exist because no schema says them.
    #[test]
    fn mono_emphasis_and_muted_are_markers_not_styles() {
        let styles = ColumnStyles::new(
            validate(
                &raw(RawColumn {
                    mono: Some(true),
                    emphasis: Some(true),
                    muted: Some(true),
                    ..RawColumn::default()
                }),
                &schema(),
                &[],
            )
            .expect("is accepted"),
        );
        let markers = styles.markers("amount", DataType::Float64);
        for name in ["data-mono", "data-emphasis", "data-muted"] {
            assert!(
                markers.iter().any(|(marker, _)| *marker == name),
                "{name} is missing"
            );
        }
    }

    /// A declared column that the result does not carry is hidden, not
    /// unknown. Its configuration survives — otherwise hiding a column would
    /// break the page's configuration of it.
    #[test]
    fn a_hidden_column_keeps_its_configuration() {
        let hidden_from_result = Schema::new(vec![Field::new(
            FieldName::new("id").unwrap(),
            DataType::Int64,
        )]);
        let accepted = validate(
            &[(
                "amount".to_owned(),
                RawColumn {
                    aggregate: Some("sum".to_owned()),
                    width: Some(150),
                    ..RawColumn::default()
                },
            )],
            &hidden_from_result,
            &["id".to_owned(), "amount".to_owned()],
        )
        .expect("a hidden column is not an error");
        assert_eq!(accepted[0].1.width, Some(150));

        // And a name that is in neither is still named.
        let problems = validate(
            &[("nope".to_owned(), RawColumn::default())],
            &hidden_from_result,
            &["id".to_owned()],
        )
        .expect_err("is refused");
        assert_eq!(problems[0].column, "nope");
    }

    /// A column nobody configured still gets its alignment, and nothing else.
    #[test]
    fn an_unconfigured_column_carries_only_its_alignment() {
        let styles = ColumnStyles::default();
        assert_eq!(styles.markers("amount", DataType::Float64).len(), 1);
        assert_eq!(styles.width("amount"), None);
    }
}

#[cfg(target_arch = "wasm32")]
pub use host::{raw, set_columns_for, store, styles};

/// Per-host storage of the column presentation, mirroring the formats seam.
#[cfg(target_arch = "wasm32")]
mod host {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::HtmlElement;

    use super::{ColumnStyles, RawColumn};

    /// What a page asked for, per column, before it met the schema.
    type Raw = Rc<Vec<(String, RawColumn)>>;

    thread_local! {
        static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
        static STYLES: RefCell<HashMap<u32, Rc<ColumnStyles>>> = RefCell::new(HashMap::new());
        /// What the page last asked for, unchecked.
        ///
        /// Kept because `set_columns` can be called before the first result, so
        /// before the real schema exists: the check then runs again when the
        /// schema arrives, rather than being skipped.
        static RAW: RefCell<HashMap<u32, Raw>> = RefCell::new(HashMap::new());
    }

    fn id_symbol() -> js_sys::Symbol {
        js_sys::Symbol::for_("opengrid.columns_id")
    }

    fn id_of(host: &HtmlElement) -> u32 {
        if let Some(id) = js_sys::Reflect::get(host.as_ref(), id_symbol().as_ref())
            .ok()
            .and_then(|value| value.as_f64())
        {
            return id as u32;
        }
        let id = NEXT_ID.with(|next| {
            let mut next = next.borrow_mut();
            let id = *next;
            *next += 1;
            id
        });
        let _ = js_sys::Reflect::set(
            host.as_ref(),
            id_symbol().as_ref(),
            &JsValue::from_f64(id as f64),
        );
        id
    }

    /// The checked presentation of a host; empty when nobody configured one.
    pub fn styles(host: &HtmlElement) -> Rc<ColumnStyles> {
        let id = id_of(host);
        STYLES.with(|map| {
            map.borrow()
                .get(&id)
                .cloned()
                .unwrap_or_else(|| Rc::new(ColumnStyles::default()))
        })
    }

    /// What the page asked for, unchecked, so it can be checked again once the
    /// schema is known.
    pub fn raw(host: &HtmlElement) -> Rc<Vec<(String, RawColumn)>> {
        let id = id_of(host);
        RAW.with(|map| map.borrow().get(&id).cloned().unwrap_or_default())
    }

    /// Stores the checked presentation.
    pub fn store(host: &HtmlElement, checked: ColumnStyles) {
        let id = id_of(host);
        STYLES.with(|map| map.borrow_mut().insert(id, Rc::new(checked)));
    }

    /// Reads a `set_columns` object into the unchecked form and remembers it.
    pub fn set_columns_for(host: &HtmlElement, value: &JsValue) -> Rc<Vec<(String, RawColumn)>> {
        let mut out: Vec<(String, RawColumn)> = Vec::new();
        if let Some(object) = value.dyn_ref::<js_sys::Object>() {
            for key in js_sys::Object::keys(object).iter() {
                let Some(name) = key.as_string() else {
                    continue;
                };
                let Ok(entry) = js_sys::Reflect::get(object, &key) else {
                    continue;
                };
                let text = |field: &str| {
                    js_sys::Reflect::get(&entry, &JsValue::from_str(field))
                        .ok()
                        .and_then(|value| value.as_string())
                };
                let flag = |field: &str| {
                    js_sys::Reflect::get(&entry, &JsValue::from_str(field))
                        .ok()
                        .and_then(|value| value.as_bool())
                };
                let width = js_sys::Reflect::get(&entry, &JsValue::from_str("width"))
                    .ok()
                    .and_then(|value| value.as_f64())
                    .filter(|width| *width > 0.0)
                    .map(|width| width as u32);
                out.push((
                    name,
                    RawColumn {
                        width,
                        align: text("align"),
                        mono: flag("mono"),
                        emphasis: flag("emphasis"),
                        muted: flag("muted"),
                        aggregate: text("aggregate"),
                        facet: text("facet"),
                    },
                ));
            }
        }
        let id = id_of(host);
        let stored = Rc::new(out);
        RAW.with(|map| map.borrow_mut().insert(id, stored.clone()));
        stored
    }
}
