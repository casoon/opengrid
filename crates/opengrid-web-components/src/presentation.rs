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

use opengrid_grid::layout::{PageColumns, Pin};
use opengrid_query::AggregateFn;
use std::collections::HashMap;
use std::rc::Rc;

use opengrid_types::{DataType, Schema, Value};

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

pub use opengrid_grid::facets::FacetKind;

/// Whether a facet kind makes sense for a column of this type.
///
/// A lookup column (issue #326) lists its keys by their labels, whatever the
/// keys' type; a span of keys is not a span of labels, so it takes no range.
fn facet_fits(kind: FacetKind, data_type: DataType, lookup: bool) -> bool {
    match kind {
        FacetKind::List | FacetKind::Pills => {
            lookup || matches!(data_type, DataType::Utf8 | DataType::Bool)
        }
        FacetKind::Range => is_numeric(data_type) && !lookup,
        FacetKind::Period => matches!(data_type, DataType::Date | DataType::Timestamp),
    }
}

/// True for the three types a sum or an average means something for.
pub fn is_numeric(data_type: DataType) -> bool {
    matches!(
        data_type,
        DataType::Int64 | DataType::Float64 | DataType::Decimal { .. }
    )
}

pub use opengrid_grid::summary::{Summary, aggregate_from};

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

/// [`aggregates_for`], for a column that may be a lookup (issue #326): a key
/// is counted and nothing else — it is not an amount, and the smallest key is
/// not the first label.
pub fn aggregates_of(data_type: DataType, lookup: bool) -> Vec<Summary> {
    if lookup {
        vec![Summary::Fn(AggregateFn::Count)]
    } else {
        aggregates_for(data_type)
    }
}

/// The labels of a column that stores keys (issue #326): what the page gave
/// `set_columns` as `lookup`, key → label.
///
/// The label is display. The key stays the value: in the query, the sort, the
/// groups, the view and every export. A key is held as the text the value is
/// written with ([`crate::formats::plain_text`]), which is how a JavaScript
/// object names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lookup {
    /// In the order the page's map lists them: the order of the options.
    entries: Vec<(String, String)>,
    by_key: HashMap<String, usize>,
}

impl Lookup {
    /// The first label stands where the page named a key twice (`7` and `07`
    /// of a number column).
    fn new(pairs: Vec<(String, String)>) -> Self {
        let mut lookup = Self::default();
        for (key, label) in pairs {
            if !lookup.by_key.contains_key(&key) {
                lookup.by_key.insert(key.clone(), lookup.entries.len());
                lookup.entries.push((key, label));
            }
        }
        lookup
    }

    /// The label of `key`, if the map names it.
    pub fn label(&self, key: &str) -> Option<&str> {
        self.by_key.get(key).map(|at| self.entries[*at].1.as_str())
    }

    /// What a cell holding `value` shows: its label; the key itself where the
    /// map does not name it; nothing for NULL.
    pub fn text(&self, value: &Value) -> String {
        let key = crate::formats::plain_text(value);
        match self.label(&key) {
            Some(label) if !matches!(value, Value::Null) => label.to_owned(),
            _ => key,
        }
    }

    /// `(key, label)`, in the page's order.
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }
}

/// Everything a page may say about one column.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnPresentation {
    /// What the column is called where a reader reads it (issue #66). The
    /// field name stays the identifier: query, view, expressions, export.
    pub title: Option<String>,
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
    /// What the page said about editing this column: `Some(true)` it fills
    /// the column (`set_values`, issue #153), `Some(false)` it opens the
    /// `row-key` column for editing (issue #523), `None` nothing — the key
    /// column is then read-only and every other column editable.
    pub readonly: Option<bool>,
    /// The page means this column to be edited (issue #523): it is drawn so.
    /// What can be edited is still everything not read-only; this says where
    /// the editing is.
    pub editable: bool,
    /// Where the column starts pinned (issue #316). The reader's choice leads
    /// after that, like a resize over `width`.
    pub pin: Option<Pin>,
    /// Where the column is pinned while the grid is narrow (issue #525):
    /// `Some(None)` keeps it from being pinned there (`pinNarrow: "none"`).
    pub pin_narrow: Option<Option<Pin>>,
    /// The labels of the keys this column stores (issue #326).
    pub lookup: Option<Rc<Lookup>>,
    /// The places of an `int64` column that stores its value × 10^scale
    /// (issue #537); never `Some(0)`, which is no scale.
    pub scale: Option<u8>,
    /// The band the column stands under (issue #319): the header it shares
    /// with the other columns that name the same one.
    pub band: Option<String>,
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

/// What a derived column cannot be (issue #528): a year or a month of a date has
/// nothing to write back to, so a column the page marks `editable` and that
/// turns out derived — the source's schema says so, after the call — is
/// reported like any other contradiction of the call.
pub fn derived_problems(
    columns: &[(String, ColumnPresentation)],
    derived: &std::collections::BTreeSet<String>,
) -> Vec<PresentationProblem> {
    columns
        .iter()
        .filter(|(name, column)| column.editable && derived.contains(name))
        .map(|(name, _)| PresentationProblem::new(name, "is derived and editable at once"))
        .collect()
}

/// One entry of a `set_columns` call, before it has met the schema.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawColumn {
    pub title: Option<String>,
    pub width: Option<u32>,
    pub align: Option<String>,
    pub mono: Option<bool>,
    pub emphasis: Option<bool>,
    pub muted: Option<bool>,
    pub aggregate: Option<String>,
    pub facet: Option<String>,
    pub readonly: Option<bool>,
    pub editable: Option<bool>,
    pub pin: Option<String>,
    pub pin_narrow: Option<String>,
    /// The `lookup` as `(key, label)` pairs; a label that is no string is
    /// empty here. `Some(None)`: something was given that is no plain object.
    pub lookup: Option<Option<Vec<(String, String)>>>,
    /// `Some(None)`: something was given that is no whole number of places.
    pub scale: Option<Option<u8>>,
    pub band: Option<String>,
}

/// Where a configuration pins and which columns it puts under a band, read
/// from what the page asked for.
///
/// Before the check, and on purpose: a pin and a band decide the **order** of
/// the columns, so they have to be known when the first query is built — and
/// the check waits for the schema that query brings. Neither asks anything of
/// the type, so there is nothing the schema could refuse about them; a token
/// that is no pin is none here and a problem in [`validate`], and so is a band
/// without a name.
pub fn page_columns_of(raw: &[(String, RawColumn)]) -> PageColumns {
    PageColumns {
        pins: raw
            .iter()
            .filter_map(|(name, column)| {
                let pin = Pin::parse(column.pin.as_deref()?)?;
                Some((name.clone(), pin))
            })
            .collect(),
        narrow_pins: raw
            .iter()
            .filter_map(|(name, column)| {
                let token = column.pin_narrow.as_deref()?;
                let pin = narrow_pin(token)?;
                Some((name.clone(), pin))
            })
            .collect(),
        narrow: false,
        bands: raw
            .iter()
            .filter_map(|(name, column)| {
                let band = column.band.as_deref()?.trim();
                (!band.is_empty()).then(|| (name.clone(), band.to_owned()))
            })
            .collect(),
    }
}

/// A `pinNarrow` token (issue #525): an end, or `none`.
fn narrow_pin(token: &str) -> Option<Option<Pin>> {
    match token {
        "none" => Some(None),
        token => Pin::parse(token).map(Some),
    }
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
            title: None,
            width: entry.width,
            mono: entry.mono.unwrap_or(false),
            emphasis: entry.emphasis.unwrap_or(false),
            muted: entry.muted.unwrap_or(false),
            readonly: entry.readonly,
            editable: entry.editable.unwrap_or(false),
            ..ColumnPresentation::default()
        };

        // Edited and filled by the page at once: the two cannot both be meant
        // (issue #523).
        if column.editable && column.readonly == Some(true) {
            problems.push(PresentationProblem::new(
                name,
                "is readonly and editable at once",
            ));
        }

        if let Some(title) = &entry.title {
            if title.trim().is_empty() {
                problems.push(PresentationProblem::new(name, "has an empty title"));
            } else {
                column.title = Some(title.trim().to_owned());
            }
        }

        if let Some(token) = &entry.align {
            match Align::parse(token) {
                Some(align) => column.align = Some(align),
                None => problems.push(PresentationProblem::new(
                    name,
                    format!("{token} is not an alignment"),
                )),
            }
        }

        if let Some(token) = &entry.pin {
            match Pin::parse(token) {
                Some(pin) => column.pin = Some(pin),
                None => problems.push(PresentationProblem::new(
                    name,
                    format!("{token} is not a pin"),
                )),
            }
        }
        if let Some(token) = &entry.pin_narrow {
            match narrow_pin(token) {
                Some(pin) => column.pin_narrow = Some(pin),
                None => problems.push(PresentationProblem::new(
                    name,
                    format!("pinNarrow: {token} is not a pin"),
                )),
            }
        }

        let lookup = entry.lookup.is_some();
        match &entry.lookup {
            None => {}
            Some(None) => problems.push(PresentationProblem::new(
                name,
                "lookup is not a map of keys to labels",
            )),
            Some(Some(_))
                if data_type.is_some_and(|data_type| {
                    !matches!(data_type, DataType::Utf8 | DataType::Int64)
                }) =>
            {
                problems.push(PresentationProblem::new(
                    name,
                    "a lookup needs a utf8 or int64 column",
                ));
            }
            Some(Some(pairs)) => {
                let before = problems.len();
                let mut checked = Vec::with_capacity(pairs.len());
                for (given, label) in pairs {
                    // The key as the column writes it: `07` names the number 7.
                    let key = match data_type {
                        Some(DataType::Int64) => given
                            .trim()
                            .parse::<i64>()
                            .ok()
                            .map(|number| number.to_string()),
                        _ => Some(given.clone()),
                    };
                    match key {
                        None => problems.push(PresentationProblem::new(
                            name,
                            format!("lookup: {given} is not a value of this column"),
                        )),
                        Some(key) if key.is_empty() => {
                            problems.push(PresentationProblem::new(name, "lookup has an empty key"))
                        }
                        Some(key) if label.trim().is_empty() => problems.push(
                            PresentationProblem::new(name, format!("lookup: {key} has no label")),
                        ),
                        Some(key) => checked.push((key, label.clone())),
                    }
                }
                if problems.len() == before {
                    column.lookup = Some(Rc::new(Lookup::new(checked)));
                }
            }
        }

        // A scale makes the stored integer a number with places (issue #537):
        // it needs an integer to store it in, and a lookup's keys are not
        // amounts.
        match entry.scale {
            None => {}
            Some(None) => problems.push(PresentationProblem::new(
                name,
                format!(
                    "scale is not a whole number from 0 to {}",
                    opengrid_grid::scaled::MAX_SCALE
                ),
            )),
            Some(Some(_)) if data_type.is_some_and(|data_type| data_type != DataType::Int64) => {
                problems.push(PresentationProblem::new(
                    name,
                    "a scale needs an int64 column",
                ));
            }
            Some(Some(_)) if lookup => problems.push(PresentationProblem::new(
                name,
                "a lookup column takes no scale",
            )),
            Some(Some(places)) => column.scale = Some(places).filter(|places| *places > 0),
        }

        if let Some(band) = &entry.band {
            if band.trim().is_empty() {
                problems.push(PresentationProblem::new(name, "has an empty band"));
            } else {
                column.band = Some(band.trim().to_owned());
            }
        }

        if let Some(token) = &entry.aggregate {
            match aggregate_from(token) {
                Some(aggregate)
                    if (!lookup || aggregate == Summary::Fn(AggregateFn::Count))
                        && data_type.is_none_or(|data_type| {
                            aggregates_for(data_type).contains(&aggregate)
                        }) =>
                {
                    column.aggregate = Some(aggregate);
                }
                Some(_) if lookup => problems.push(PresentationProblem::new(
                    name,
                    format!("{token} is not an aggregate for a lookup column"),
                )),
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
                Some(facet)
                    if data_type.is_none_or(|data_type| facet_fits(facet, data_type, lookup)) =>
                {
                    column.facet = Some(facet)
                }
                Some(FacetKind::Range) if lookup => problems.push(PresentationProblem::new(
                    name,
                    format!("{token} is not a facet for a lookup column"),
                )),
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

    // A band is pinned as a whole (issue #319): two of its columns pinned at
    // different ends ask for a band in two pieces.
    for (index, (name, column)) in out.iter().enumerate() {
        let (Some(band), Some(pin)) = (&column.band, column.pin) else {
            continue;
        };
        let first = out[..index]
            .iter()
            .find(|(_, other)| other.band.as_ref() == Some(band) && other.pin.is_some());
        if let Some((other, earlier)) = first
            && earlier.pin != Some(pin)
        {
            problems.push(PresentationProblem::new(
                name,
                format!("is pinned at another end than {other}, of the same band"),
            ));
        }
    }

    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

/// [`validate`] for `<opengrid-table>` (issue #29): the same checks, and the
/// grid's own options — `width`, `aggregate`, `facet`, `pin`, `band` —
/// refused. The table has no resize, no groups, no facets, nothing that
/// scrolls sideways and one header row, and an option that silently did nothing
/// would hide the mistake the same way an unknown column would.
pub fn validate_for_table(
    raw: &[(String, RawColumn)],
    schema: &Schema,
    declared: &[String],
) -> Result<Vec<(String, ColumnPresentation)>, Vec<PresentationProblem>> {
    let mut problems = Vec::new();
    for (name, entry) in raw {
        for (given, option) in [
            (entry.width.is_some(), "width"),
            (entry.aggregate.is_some(), "aggregate"),
            (entry.facet.is_some(), "facet"),
            (entry.readonly.is_some(), "readonly"),
            (entry.editable.is_some(), "editable"),
            (entry.pin.is_some(), "pin"),
            (entry.pin_narrow.is_some(), "pinNarrow"),
            (entry.lookup.is_some(), "lookup"),
            (entry.scale.is_some(), "scale"),
            (entry.band.is_some(), "band"),
        ] {
            if given {
                problems.push(PresentationProblem::new(
                    name,
                    format!("{option} belongs to the grid, not to a table"),
                ));
            }
        }
    }
    match validate(raw, schema, declared) {
        Ok(checked) if problems.is_empty() => Ok(checked),
        Ok(_) => Err(problems),
        Err(more) => {
            problems.extend(more);
            Err(problems)
        }
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
        // A lookup column shows labels, which are text whatever the keys are
        // (issue #326): it lines up like text unless the page says otherwise.
        let align = presentation
            .and_then(|column| {
                column
                    .align
                    .or_else(|| column.lookup.as_ref().map(|_| Align::Start))
            })
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

    /// The titles the page gave, by field name (issue #66).
    pub fn titles(&self) -> Vec<(String, String)> {
        self.by_name
            .iter()
            .filter_map(|(name, column)| column.title.clone().map(|title| (name.clone(), title)))
            .collect()
    }

    /// Whether this column never opens an editor (issues #153, #523): the page
    /// fills it (`readonly: true`), or it is the `row-key` column, whose value
    /// names the record — unless the page opened it with `readonly: false`.
    pub fn readonly(&self, name: &str, row_key: Option<&str>) -> bool {
        match self.get(name).and_then(|column| column.readonly) {
            Some(readonly) => readonly,
            None => row_key == Some(name),
        }
    }

    /// Whether the page means this column to be edited (issue #523).
    pub fn editable(&self, name: &str) -> bool {
        self.get(name).is_some_and(|column| column.editable)
    }

    /// Whether the page left this column out of the ones it marked for editing
    /// (issue #527): as soon as one column has `editable: true`, only the marked
    /// columns edit — and a column the page opened with `readonly: false` is
    /// as good as marked. A grid in which no column is marked edits every
    /// column, as it always did.
    pub fn unmarked(&self, name: &str) -> bool {
        self.by_name.iter().any(|(_, column)| column.editable)
            && !self.editable(name)
            && self.get(name).and_then(|column| column.readonly) != Some(false)
    }

    /// The labels of a column that stores keys (issue #326), if it has any.
    pub fn lookup(&self, name: &str) -> Option<&Rc<Lookup>> {
        self.get(name).and_then(|column| column.lookup.as_ref())
    }

    /// The places of a column that stores its value × 10^scale (issue #537).
    pub fn scale(&self, name: &str) -> Option<u8> {
        self.get(name).and_then(|column| column.scale)
    }

    pub fn width(&self, name: &str) -> Option<u32> {
        self.get(name).and_then(|column| column.width)
    }

    /// Where the page pins (issue #316) and its bands (issue #319), by field
    /// name.
    pub fn page_columns(&self) -> PageColumns {
        PageColumns {
            pins: self
                .by_name
                .iter()
                .filter_map(|(name, column)| column.pin.map(|pin| (name.clone(), pin)))
                .collect(),
            narrow_pins: self
                .by_name
                .iter()
                .filter_map(|(name, column)| column.pin_narrow.map(|pin| (name.clone(), pin)))
                .collect(),
            narrow: false,
            bands: self
                .by_name
                .iter()
                .filter_map(|(name, column)| column.band.clone().map(|band| (name.clone(), band)))
                .collect(),
        }
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

    /// `pinNarrow` (issue #525) is an end or `none`, kept for the page's
    /// narrow pins; anything else is named.
    #[test]
    fn a_narrow_pin_is_an_end_or_none() {
        let narrow = |token: &str| RawColumn {
            pin_narrow: Some(token.to_owned()),
            ..Default::default()
        };
        let checked = validate(&raw(narrow("end")), &schema(), &[]).expect("an end");
        assert_eq!(checked[0].1.pin_narrow, Some(Some(Pin::End)));
        assert_eq!(
            ColumnStyles::new(checked).page_columns().narrow_pins,
            [("amount".to_owned(), Some(Pin::End))]
        );
        assert_eq!(
            page_columns_of(&raw(narrow("none"))).narrow_pins,
            [("amount".to_owned(), None)]
        );
        let problems = validate(&raw(narrow("left")), &schema(), &[]).expect_err("is refused");
        assert_eq!(problems[0].reason, "pinNarrow: left is not a pin");
    }

    /// `scale` (issue #537) is a whole number of places of an `int64` column;
    /// 0 is no scale, and every other mistake is named, none of the call applied.
    #[test]
    fn a_scale_is_a_whole_number_of_places_of_an_integer_column() {
        let scaled = |name: &str, places: Option<u8>| {
            vec![(
                name.to_owned(),
                RawColumn {
                    scale: Some(places),
                    ..RawColumn::default()
                },
            )]
        };
        let refused = |raw: Vec<(String, RawColumn)>| {
            let problems = validate(&raw, &schema(), &[]).expect_err("is refused");
            assert_eq!(problems.len(), 1);
            problems[0].reason.clone()
        };

        let checked = validate(&scaled("id", Some(2)), &schema(), &[]).expect("an integer");
        assert_eq!(checked[0].1.scale, Some(2));
        let styles = ColumnStyles::new(checked);
        assert_eq!(styles.scale("id"), Some(2));
        assert_eq!(styles.scale("customer"), None);
        // No places is no scale, as a page may compute it (a currency without
        // minor units).
        let checked = validate(&scaled("id", Some(0)), &schema(), &[]).expect("no scale");
        assert_eq!(checked[0].1.scale, None);

        for column in ["amount", "customer", "ordered_on"] {
            assert_eq!(
                refused(scaled(column, Some(2))),
                "a scale needs an int64 column",
                "{column}"
            );
        }
        assert_eq!(
            refused(scaled("id", None)),
            "scale is not a whole number from 0 to 18"
        );
        let lookup = RawColumn {
            scale: Some(Some(2)),
            lookup: Some(Some(vec![("1".to_owned(), "One".to_owned())])),
            ..RawColumn::default()
        };
        assert_eq!(
            refused(vec![("id".to_owned(), lookup)]),
            "a lookup column takes no scale"
        );

        // A hidden column is not in the schema: its type is checked when it is
        // shown again.
        let hidden = validate(
            &scaled("hidden", Some(2)),
            &schema(),
            &["hidden".to_owned()],
        )
        .expect("a declared column");
        assert_eq!(hidden[0].1.scale, Some(2));

        // A table has no scale: refused by name, not ignored.
        let problems = validate_for_table(&scaled("id", Some(2)), &schema(), &[])
            .expect_err("a table refuses it");
        assert_eq!(
            problems[0].reason,
            "scale belongs to the grid, not to a table"
        );
    }

    /// A table takes the look (title, alignment, faces) and refuses the grid's
    /// own options by name, together with any ordinary mistake.
    #[test]
    fn a_table_refuses_the_grid_options() {
        let accepted = validate_for_table(
            &raw(RawColumn {
                title: Some("Betrag".to_owned()),
                align: Some("center".to_owned()),
                muted: Some(true),
                ..RawColumn::default()
            }),
            &schema(),
            &[],
        )
        .unwrap();
        assert_eq!(accepted[0].1.title.as_deref(), Some("Betrag"));
        assert!(accepted[0].1.muted);

        for (entry, option) in [
            (
                RawColumn {
                    width: Some(90),
                    ..RawColumn::default()
                },
                "width",
            ),
            (
                RawColumn {
                    aggregate: Some("sum".to_owned()),
                    ..RawColumn::default()
                },
                "aggregate",
            ),
            (
                RawColumn {
                    facet: Some("range".to_owned()),
                    ..RawColumn::default()
                },
                "facet",
            ),
        ] {
            let problems = validate_for_table(&raw(entry), &schema(), &[]).unwrap_err();
            assert_eq!(problems.len(), 1, "{option}");
            assert!(problems[0].reason.starts_with(option), "{problems:?}");
        }

        let problems = validate_for_table(
            &raw(RawColumn {
                width: Some(90),
                align: Some("sideways".to_owned()),
                ..RawColumn::default()
            }),
            &schema(),
            &[],
        )
        .unwrap_err();
        assert_eq!(problems.len(), 2, "both at once: {problems:?}");
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

    /// A pin is `start` or `end` (issue #316): anything else is named, a table
    /// refuses the option, and the order of the columns reads it before the
    /// schema is there to check the rest.
    #[test]
    fn a_pin_is_an_end_of_the_row() {
        let pinned = |token: &str| RawColumn {
            pin: Some(token.to_owned()),
            ..RawColumn::default()
        };
        let checked = validate(&raw(pinned("end")), &schema(), &[]).unwrap();
        assert_eq!(checked[0].1.pin, Some(Pin::End));
        let styles = ColumnStyles::new(checked);
        assert_eq!(
            styles.page_columns().pins,
            [("amount".to_owned(), Pin::End)]
        );
        assert_eq!(
            page_columns_of(&raw(pinned("start"))).pins,
            [("amount".to_owned(), Pin::Start)]
        );

        let problems = validate(&raw(pinned("left")), &schema(), &[]).expect_err("is refused");
        assert_eq!(problems[0].reason, "left is not a pin");
        assert!(page_columns_of(&raw(pinned("left"))).pins.is_empty());

        let problems =
            validate_for_table(&raw(pinned("start")), &schema(), &[]).expect_err("is refused");
        assert_eq!(
            problems[0].reason,
            "pin belongs to the grid, not to a table"
        );
    }

    /// A band is a name (issue #319): read before the schema is there, since
    /// it decides the order of the columns; an empty one is named, a table
    /// refuses the option, and two columns of one band pinned at different
    /// ends are refused — a band is pinned as a whole.
    #[test]
    fn a_band_is_a_name_and_pinned_as_a_whole() {
        let banded = |band: &str, pin: Option<&str>| RawColumn {
            band: Some(band.to_owned()),
            pin: pin.map(str::to_owned),
            ..RawColumn::default()
        };
        let two = |first: RawColumn, second: RawColumn| {
            vec![
                ("customer".to_owned(), first),
                ("amount".to_owned(), second),
            ]
        };
        let asked = two(banded(" Order ", Some("end")), banded("Order", None));
        let bands = [
            ("customer".to_owned(), "Order".to_owned()),
            ("amount".to_owned(), "Order".to_owned()),
        ];
        assert_eq!(page_columns_of(&asked).bands, bands);
        let checked = validate(&asked, &schema(), &[]).expect("is accepted");
        assert_eq!(checked[0].1.band.as_deref(), Some("Order"));
        let page = ColumnStyles::new(checked).page_columns();
        assert_eq!(page.bands, bands);
        assert_eq!(page.pins, [("customer".to_owned(), Pin::End)]);

        let problems = validate(&raw(banded("  ", None)), &schema(), &[]).expect_err("is refused");
        assert_eq!(problems[0].reason, "has an empty band");
        assert!(page_columns_of(&raw(banded("  ", None))).bands.is_empty());

        let apart = two(banded("Order", Some("start")), banded("Order", Some("end")));
        let problems = validate(&apart, &schema(), &[]).expect_err("is refused");
        assert_eq!(
            (problems[0].column.as_str(), problems[0].reason.as_str()),
            (
                "amount",
                "is pinned at another end than customer, of the same band"
            )
        );
        let alike = two(banded("Order", Some("end")), banded("Order", Some("end")));
        assert!(validate(&alike, &schema(), &[]).is_ok());

        let problems = validate_for_table(&raw(banded("Order", None)), &schema(), &[])
            .expect_err("is refused");
        assert_eq!(
            problems[0].reason,
            "band belongs to the grid, not to a table"
        );
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

    /// A column the page fills is read-only, and a table, which edits nothing,
    /// refuses the option (issue #153).
    #[test]
    fn a_readonly_column_is_read_only_and_a_table_refuses_it() {
        let entry = RawColumn {
            readonly: Some(true),
            ..RawColumn::default()
        };
        let styles = ColumnStyles::new(validate(&raw(entry.clone()), &schema(), &[]).unwrap());
        assert!(styles.readonly("amount", None));
        assert!(!styles.readonly("id", None));
        let problems = validate_for_table(&raw(entry), &schema(), &[]).expect_err("is refused");
        assert_eq!(
            problems[0].reason,
            "readonly belongs to the grid, not to a table"
        );
    }

    /// The key column names the record, so it is read-only unless the page
    /// opens it (issue #523).
    #[test]
    fn the_key_column_is_read_only() {
        let fields = schema();
        let none = ColumnStyles::default();
        assert!(none.readonly("id", Some("id")));
        assert!(!none.readonly("id", Some("customer")));
        assert!(!none.readonly("id", None));
        let opened = ColumnStyles::new(
            validate(
                &[(
                    "id".to_owned(),
                    RawColumn {
                        readonly: Some(false),
                        ..RawColumn::default()
                    },
                )],
                &fields,
                &[],
            )
            .unwrap(),
        );
        assert!(!opened.readonly("id", Some("id")));
    }

    /// Issue #528: what the source's schema says is derived cannot be `editable`; the
    /// page's other columns are not touched.
    #[test]
    fn a_derived_editable_column_is_reported() {
        let marked = RawColumn {
            editable: Some(true),
            ..RawColumn::default()
        };
        let columns = validate(&raw(marked), &schema(), &[]).expect("is accepted");
        let derived = std::collections::BTreeSet::from(["amount".to_owned()]);
        let problems = derived_problems(&columns, &derived);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].column, "amount");
        assert_eq!(problems[0].reason, "is derived and editable at once");
        assert!(derived_problems(&columns, &std::collections::BTreeSet::new()).is_empty());
        let plain = validate(&raw(RawColumn::default()), &schema(), &[]).expect("is accepted");
        assert!(derived_problems(&plain, &derived).is_empty());
    }

    /// `editable` draws a column as the one to edit, and contradicting it is
    /// reported; a table refuses it (issue #523).
    #[test]
    fn an_editable_column_is_checked() {
        let editable = RawColumn {
            editable: Some(true),
            ..RawColumn::default()
        };
        let styles = ColumnStyles::new(validate(&raw(editable.clone()), &schema(), &[]).unwrap());
        assert!(styles.editable("amount"));
        assert!(!styles.editable("customer"));
        let both = RawColumn {
            readonly: Some(true),
            ..editable.clone()
        };
        let problems = validate(&raw(both), &schema(), &[]).expect_err("contradicts");
        assert_eq!(problems[0].reason, "is readonly and editable at once");
        let problems = validate_for_table(&raw(editable), &schema(), &[]).expect_err("is refused");
        assert_eq!(
            problems[0].reason,
            "editable belongs to the grid, not to a table"
        );
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

    fn lookup_of(column: &str, pairs: &[(&str, &str)]) -> Vec<(String, RawColumn)> {
        vec![(
            column.to_owned(),
            RawColumn {
                lookup: Some(Some(
                    pairs
                        .iter()
                        .map(|(key, label)| ((*key).to_owned(), (*label).to_owned()))
                        .collect(),
                )),
                ..RawColumn::default()
            },
        )]
    }

    /// The contract of issue #326 in one place: a key shows its label, a key
    /// the map does not name shows itself, NULL shows nothing — and the
    /// options keep the page's order.
    #[test]
    fn a_lookup_shows_the_label_of_a_key_and_the_key_where_it_has_none() {
        let styles = ColumnStyles::new(
            validate(
                &lookup_of("id", &[("2", "Beta"), ("07", "Seven"), ("1", "Alpha")]),
                &schema(),
                &[],
            )
            .expect("is accepted"),
        );
        let lookup = styles.lookup("id").expect("the column has a lookup");
        assert_eq!(lookup.text(&Value::Int64(1)), "Alpha");
        // `07` names the number 7, as the column writes it.
        assert_eq!(lookup.text(&Value::Int64(7)), "Seven");
        assert_eq!(lookup.text(&Value::Int64(3)), "3", "no label: the key");
        assert_eq!(lookup.text(&Value::Null), "", "NULL is not a key");
        assert_eq!(lookup.label("2"), Some("Beta"));
        assert_eq!(lookup.label("3"), None);
        let keys: Vec<&str> = lookup
            .entries()
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(keys, ["2", "7", "1"], "the page's order, not the keys'");
        assert!(styles.lookup("customer").is_none());
        // Labels are text: a number column of keys lines up like text, unless
        // the page says otherwise.
        assert_eq!(
            styles.markers("id", DataType::Int64)[0],
            ("data-align", "start".to_owned())
        );
        let mut ended = lookup_of("id", &[("1", "Alpha")]);
        ended[0].1.align = Some("end".to_owned());
        assert_eq!(
            ColumnStyles::new(validate(&ended, &schema(), &[]).unwrap())
                .markers("id", DataType::Int64)[0],
            ("data-align", "end".to_owned())
        );

        // Text keys are taken as they are, and NULL stays empty even where
        // the map names the empty text of another column's notation.
        let text = ColumnStyles::new(
            validate(&lookup_of("customer", &[("DE", "Germany")]), &schema(), &[]).unwrap(),
        );
        let lookup = text.lookup("customer").unwrap();
        assert_eq!(lookup.text(&Value::Utf8("DE".to_owned())), "Germany");
        assert_eq!(lookup.text(&Value::Utf8("de".to_owned())), "de");
    }

    /// The configuration narrows: a lookup that cannot be one is named, every
    /// mistake of it at once.
    #[test]
    fn a_lookup_that_does_not_fit_its_column_is_refused() {
        let reasons = |raw: &[(String, RawColumn)]| -> Vec<String> {
            validate(raw, &schema(), &[])
                .expect_err("is refused")
                .into_iter()
                .map(|problem| problem.reason)
                .collect()
        };
        assert_eq!(
            reasons(&lookup_of("id", &[("abc", "A"), ("1", ""), ("2", "Two")])),
            [
                "lookup: abc is not a value of this column",
                "lookup: 1 has no label"
            ]
        );
        assert_eq!(
            reasons(&lookup_of("customer", &[("", "Nobody")])),
            ["lookup has an empty key"]
        );
        assert_eq!(
            reasons(&lookup_of("amount", &[("1", "One")])),
            ["a lookup needs a utf8 or int64 column"]
        );
        assert_eq!(
            reasons(&lookup_of("ordered_on", &[("2026-01-01", "New year")])),
            ["a lookup needs a utf8 or int64 column"]
        );
        let not_a_map = vec![(
            "id".to_owned(),
            RawColumn {
                lookup: Some(None),
                ..RawColumn::default()
            },
        )];
        assert_eq!(
            reasons(&not_a_map),
            ["lookup is not a map of keys to labels"]
        );
        // A table shows no labels: the option is the grid's.
        let problems = validate_for_table(&lookup_of("id", &[("1", "One")]), &schema(), &[])
            .expect_err("is refused");
        assert_eq!(
            problems[0].reason,
            "lookup belongs to the grid, not to a table"
        );
    }

    /// A key is counted and listed, not added up or spanned: a number column
    /// with a lookup takes the facets of a text column and `count` alone.
    #[test]
    fn a_lookup_column_is_counted_and_listed_whatever_its_keys_are() {
        let with = |aggregate: Option<&str>, facet: Option<&str>| {
            let mut raw = lookup_of("id", &[("1", "One")]);
            raw[0].1.aggregate = aggregate.map(str::to_owned);
            raw[0].1.facet = facet.map(str::to_owned);
            validate(&raw, &schema(), &[])
        };
        for facet in ["list", "pills"] {
            let accepted = with(Some("count"), Some(facet)).expect("is accepted");
            assert_eq!(accepted[0].1.facet.map(|kind| kind.as_str()), Some(facet));
        }
        assert_eq!(
            with(None, Some("range")).unwrap_err()[0].reason,
            "range is not a facet for a lookup column"
        );
        for aggregate in ["sum", "avg", "min", "max", "range"] {
            assert_eq!(
                with(Some(aggregate), None).unwrap_err()[0].reason,
                format!("{aggregate} is not an aggregate for a lookup column")
            );
        }
        // Without a lookup the same column is a number like any other.
        assert!(
            validate(
                &[(
                    "id".to_owned(),
                    RawColumn {
                        aggregate: Some("sum".to_owned()),
                        facet: Some("range".to_owned()),
                        ..RawColumn::default()
                    }
                )],
                &schema(),
                &[]
            )
            .is_ok()
        );
        assert_eq!(
            aggregates_of(DataType::Int64, true),
            [Summary::Fn(AggregateFn::Count)]
        );
        assert_eq!(
            aggregates_of(DataType::Int64, false),
            aggregates_for(DataType::Int64)
        );
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
pub use host::{
    is_narrow, is_refused, page_columns, raw, refuse, set_columns_for, set_narrow, store, styles,
};

/// Per-host storage of the column presentation, mirroring the formats seam.
#[cfg(target_arch = "wasm32")]
mod host {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use opengrid_web_core::host::{id as host_id, on_release};
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::HtmlElement;

    use super::{ColumnStyles, PageColumns, RawColumn};

    /// What a page asked for, per column, before it met the schema.
    type Raw = Rc<Vec<(String, RawColumn)>>;

    thread_local! {
        static STYLES: RefCell<HashMap<u32, Rc<ColumnStyles>>> = RefCell::new(HashMap::new());
        /// What the page last asked for, unchecked.
        ///
        /// Kept because `set_columns` can be called before the first result, so
        /// before the real schema exists: the check then runs again when the
        /// schema arrives, rather than being skipped.
        static RAW: RefCell<HashMap<u32, Raw>> = RefCell::new(HashMap::new());
        /// The hosts whose last configuration was refused against the real
        /// schema: none of it is applied, its pins included.
        static REFUSED: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
        /// The hosts that are narrow now (issue #525).
        static NARROW: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    }

    fn release(id: u32) {
        STYLES.with(|map| map.borrow_mut().remove(&id));
        RAW.with(|map| map.borrow_mut().remove(&id));
        REFUSED.with(|hosts| hosts.borrow_mut().retain(|host| *host != id));
        NARROW.with(|hosts| hosts.borrow_mut().retain(|host| *host != id));
    }

    /// Where the page pins (issue #316) and its bands (issue #319): what it
    /// asked for ([`super::page_columns_of`]), or — once that was refused —
    /// what the last accepted configuration says.
    ///
    /// On a narrow grid (issue #525) the narrow pins hold, and the `row-key`
    /// column is pinned at the start when it is shown and the page pins
    /// nothing that holds on narrow grids itself — no `pinNarrow`, and no
    /// `pin`, which holds at every width. `pinNarrow: "none"` on it says no.
    pub fn page_columns(host: &HtmlElement) -> PageColumns {
        let mut page = if is_refused(host) {
            styles(host).page_columns()
        } else {
            super::page_columns_of(&raw(host))
        };
        page.narrow = is_narrow(host);
        if let Some(key) = host
            .get_attribute("row-key")
            .map(|key| key.trim().to_owned())
            && !key.is_empty()
            && host
                .get_attribute("columns")
                .is_some_and(|columns| columns.split(',').any(|name| name.trim() == key))
            && page.pins.is_empty()
            && page.narrow_pins.iter().all(|(_, pin)| pin.is_none())
            && !page.narrow_pins.iter().any(|(name, _)| *name == key)
        {
            page.narrow_pins.push((key, Some(super::Pin::Start)));
        }
        page
    }

    /// Whether the grid is narrow now (issue #525), as it last measured.
    pub fn is_narrow(host: &HtmlElement) -> bool {
        let id = id_of(host);
        NARROW.with(|hosts| hosts.borrow().contains(&id))
    }

    /// Remembers whether the grid is narrow; answers whether that changed.
    pub fn set_narrow(host: &HtmlElement, narrow: bool) -> bool {
        let id = id_of(host);
        NARROW.with(|hosts| {
            let mut hosts = hosts.borrow_mut();
            let was = hosts.contains(&id);
            hosts.retain(|host| *host != id);
            if narrow {
                hosts.push(id);
            }
            was != narrow
        })
    }

    /// Whether the configuration the page asked for was refused.
    pub fn is_refused(host: &HtmlElement) -> bool {
        let id = id_of(host);
        REFUSED.with(|hosts| hosts.borrow().contains(&id))
    }

    /// Remembers whether the configuration the page asked for was refused.
    pub fn refuse(host: &HtmlElement, refused: bool) {
        let id = id_of(host);
        REFUSED.with(|hosts| {
            let mut hosts = hosts.borrow_mut();
            hosts.retain(|host| *host != id);
            if refused {
                hosts.push(id);
            }
        });
    }

    /// The host's id, with this store's release registered.
    fn id_of(host: &HtmlElement) -> u32 {
        on_release(release);
        host_id(host)
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
        // The titles go with the texts (issue #66): every sentence that names
        // a column reads them there.
        let titles = checked.titles();
        STYLES.with(|map| map.borrow_mut().insert(id, Rc::new(checked)));
        let current = crate::texts::texts(host);
        if current.titles != titles {
            let mut texts = (*current).clone();
            texts.titles = titles;
            crate::texts::store(host, Rc::new(texts));
        }
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
                // A plain object, key → label. Anything else that was given
                // — an array, a `Map`, a string — is kept as "not a map", so
                // the check can say so.
                let lookup = js_sys::Reflect::get(&entry, &JsValue::from_str("lookup"))
                    .ok()
                    .filter(|value| !value.is_undefined())
                    .map(|value| {
                        let object = value.dyn_ref::<js_sys::Object>().filter(|_| {
                            !js_sys::Array::is_array(&value)
                                && !value.is_instance_of::<js_sys::Map>()
                                && !value.is_function()
                        })?;
                        Some(
                            js_sys::Object::entries(object)
                                .iter()
                                .filter_map(|pair| {
                                    let pair: js_sys::Array = pair.unchecked_into();
                                    Some((
                                        pair.get(0).as_string()?,
                                        pair.get(1).as_string().unwrap_or_default(),
                                    ))
                                })
                                .collect(),
                        )
                    });
                // A whole number of places; anything else that was given — a
                // string, `2.5`, `-1` — is kept as "not a scale", so the check
                // can say so.
                let scale = js_sys::Reflect::get(&entry, &JsValue::from_str("scale"))
                    .ok()
                    .filter(|value| !value.is_undefined())
                    .map(|value| {
                        value
                            .as_f64()
                            .filter(|places| {
                                places.fract() == 0.0
                                    && (0.0..=f64::from(opengrid_grid::scaled::MAX_SCALE))
                                        .contains(places)
                            })
                            .map(|places| places as u8)
                    });
                out.push((
                    name,
                    RawColumn {
                        title: text("title"),
                        width,
                        align: text("align"),
                        mono: flag("mono"),
                        emphasis: flag("emphasis"),
                        muted: flag("muted"),
                        aggregate: text("aggregate"),
                        facet: text("facet"),
                        readonly: flag("readonly"),
                        editable: flag("editable"),
                        pin: text("pin"),
                        pin_narrow: text("pinNarrow"),
                        lookup,
                        scale,
                        band: text("band"),
                    },
                ));
            }
        }
        let id = id_of(host);
        let stored = Rc::new(out);
        // A new configuration is unchecked, not refused.
        REFUSED.with(|hosts| hosts.borrow_mut().retain(|host| *host != id));
        RAW.with(|map| map.borrow_mut().insert(id, stored.clone()));
        stored
    }
}
