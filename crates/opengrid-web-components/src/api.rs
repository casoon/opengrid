//! The public surface, frozen (plan point 39).
//!
//! The API of this crate is not its Rust items — every module is `pub(crate)`.
//! It is the **DOM**: three custom elements, their attributes, the events they
//! fire, the parts a page may style, the custom properties it may set, and the
//! keys it may translate. Plus the ten module functions and the loader's exports.
//!
//! This module writes that surface down as data and a test compares it against
//! a list that a human maintains. A name that changes shows up in the diff of
//! that list, where it can be argued about, instead of quietly in a release.
//!
//! **Adding to the list is a decision, not a formality.** Everything in it is a
//! promise to somebody who does not have this repository.

use crate::{grid, pivot, table};

/// What `packages/opengrid/loader.js` exports.
const LOADER_EXPORTS: [&str; 8] = [
    "loadOpengrid",
    "connect",
    "createLocalProvider",
    "createWorkerProvider",
    "createRestProvider",
    "createPivotProvider",
    "createHybridProvider",
    "exportRows",
];

/// The methods of the provider `createRestProvider` answers — `export` is
/// the server's export (issue #2), which `exportRows` uses when it is there.
const REST_PROVIDER_METHODS: [&str; 3] = ["describe", "execute", "export"];

/// The fields on the `Error` a server provider or `exportRows` rejects with
/// (issue #16): a plain `Error`, no class of its own, the message unchanged.
const ERROR_FIELDS: [&str; 3] = ["status", "code", "path"];

/// The `code`s of the server's error form — `ErrorCode` in
/// `opengrid_datasource::wire`, a closed list; a test below holds the two
/// together. The REST and pivot providers hand them on as they came.
const SERVER_ERROR_CODES: [&str; 7] = [
    "validation",
    "unknown_source",
    "limit_exceeded",
    "busy",
    "unauthorized",
    "backend",
    "malformed",
];

/// The `code`s of `exportRows`' own errors, the `coded(…)` calls of
/// loader.js; a test below reads them from there.
const EXPORT_ERROR_CODES: [&str; 3] = ["too_many_rows", "source_changed", "module_not_loaded"];

/// Every name a page can rely on, in one string.
fn surface() -> String {
    let mut out = String::new();

    out.push_str("elements\n");
    for tag in [table::TABLE_TAG, grid::GRID_TAG, pivot::PIVOT_TAG] {
        out.push_str(&format!("  {tag}\n"));
    }

    out.push_str("\nattributes\n");
    for (tag, observed) in [
        (table::TABLE_TAG, table::OBSERVED),
        (grid::GRID_TAG, grid::OBSERVED),
        (pivot::PIVOT_TAG, pivot::OBSERVED),
    ] {
        let mut names: Vec<&str> = observed.to_vec();
        names.sort_unstable();
        out.push_str(&format!("  {tag}: {}\n", names.join(" ")));
    }

    out.push_str("\nevents\n");
    for event in [
        crate::grid_element_events::SELECTION_EVENT,
        crate::grid_element_events::CELL_EVENT,
        crate::grid_element_events::VIEW_EVENT,
    ] {
        out.push_str(&format!("  {event}\n"));
    }

    out.push_str("\nfunctions\n");
    for name in [
        "register",
        "set_provider",
        "set_texts",
        "set_formats",
        "set_choices",
        "get_view",
        "set_view",
        "set_columns",
        "get_query",
        "get_pivot",
    ] {
        out.push_str(&format!("  {name}\n"));
    }

    // The loader's own exports (point 76): the page imports these by name, so
    // they are as much a promise as the module functions.
    out.push_str("\nloader exports\n");
    for name in LOADER_EXPORTS {
        out.push_str(&format!("  {name}\n"));
    }

    out.push_str("\nprovider methods\n");
    out.push_str(&format!(
        "  createRestProvider: {}\n",
        REST_PROVIDER_METHODS.join(" ")
    ));

    // What a page branches on when a call fails (issue #16).
    out.push_str("\nerror fields\n");
    out.push_str(&format!("  {}\n", ERROR_FIELDS.join(" ")));
    out.push_str("\nerror codes (server)\n");
    out.push_str(&format!("  {}\n", SERVER_ERROR_CODES.join(" ")));
    out.push_str("\nerror codes (exportRows)\n");
    out.push_str(&format!("  {}\n", EXPORT_ERROR_CODES.join(" ")));

    // Two groups, because they are two promises: a page *sets* the first and
    // may override the second, which the grid otherwise computes for it.
    out.push_str("\ncustom properties (set)\n");
    out.push_str(&format!("  {}\n", grid::SET_TOKENS.join(" ")));
    out.push_str("\ncustom properties (computed)\n");
    out.push_str(&format!("  {}\n", grid::COMPUTED_TOKENS.join(" ")));

    out.push_str("\nparts\n");
    out.push_str(&format!("  {}\n", parts().join(" ")));

    out.push_str("\ntext keys\n");
    let mut keys: Vec<&str> = crate::texts::KEYS.to_vec();
    keys.sort_unstable();
    out.push_str(&format!("  {}\n", keys.join(" ")));

    out
}

/// Every `part` the three elements write, gathered by building them.
///
/// Computed rather than listed: a part that exists only in the renderer is one
/// a page can already style, and a hand-kept list would not know about it.
fn parts() -> Vec<String> {
    use opengrid_types::{DataType, Field, FieldName, Schema};
    use opengrid_web_core::patch::{NodeAllocator, Patch, PatchBuffer};

    let schema = Schema::new(vec![
        Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
        Field::new(FieldName::new("qty").unwrap(), DataType::Int64),
    ]);
    let texts = crate::texts::GridTexts::default();
    let declared = vec![("customer".to_owned(), true), ("qty".to_owned(), false)];

    let mut buffer = PatchBuffer::new();
    let mut nodes = NodeAllocator::new();
    grid::build_grid(
        &mut buffer,
        &mut nodes,
        &grid::GridSkeleton {
            label: Some("x"),
            schema: &schema,
            pool: 2,
            texts: &texts,
            declared: &declared,
            presentation: &Default::default(),
            // Built **with** the selection column (point 61): its parts belong
            // to the public surface even though the column is opt-in, and a
            // freeze that only saw the default would not know them.
            selection: true,
            column_menu: true,
            toolbar: true,
            facets: true,
            search: true,
        },
    );
    table::build_table(&mut buffer, &mut nodes, Some("x"), None, None);
    pivot::build_pivot(
        &mut buffer,
        &mut nodes,
        Some("x"),
        None,
        "",
        "ready",
        &texts,
    );

    let mut parts: Vec<String> = buffer
        .patches()
        .iter()
        .filter_map(|patch| match patch {
            Patch::SetAttribute { name, value, .. } if name == "part" => Some(value.clone()),
            _ => None,
        })
        // A part attribute may name more than one (`row total-row`).
        .flat_map(|value| {
            value
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    // Written at render time, not in the skeleton: the editor of point 37 and
    // the total row of a pivot with data.
    parts.push("editor".to_owned());
    parts.push("total-row".to_owned());
    // The column menu of point 64 is built when it opens, not in the skeleton.
    parts.push("column-menu".to_owned());
    parts.push("menu-label".to_owned());
    // The chips of point 65 are drawn from the view by the element.
    parts.push("chip".to_owned());
    parts.push("chip-remove".to_owned());
    parts.push("chips-clear".to_owned());
    // The facet sidebar's contents are drawn from the configuration (point 66),
    // and the toolbar's facet switch exists only once facets are configured.
    for part in [
        "facets-toggle",
        "facets-head",
        "facet-cost",
        "facet",
        "facet-value",
        "facet-count",
        "facet-pills",
        "facet-pill",
        "facet-bounds",
    ] {
        parts.push(part.to_owned());
    }
    parts.sort_unstable();
    parts.dedup();
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The freeze.** Every name a page outside this repository can rely on.
    ///
    /// A failure here is not a bug — it is the diff of a promise. Change the
    /// list when the change is intended, and say why in the commit.
    #[test]
    fn the_public_surface_is_what_it_says_it_is() {
        let expected = "\
elements
  opengrid-table
  opengrid-grid
  opengrid-pivot

attributes
  opengrid-table: columns datasource label
  opengrid-grid: column-menu columns datasource density facets group-by label mode page-size \
search selection toolbar window-size
  opengrid-pivot: columns datasource label rows values

events
  opengrid-selection-change
  opengrid-cell-change
  opengrid-view-change

functions
  register
  set_provider
  set_texts
  set_formats
  set_choices
  get_view
  set_view
  set_columns
  get_query
  get_pivot

loader exports
  loadOpengrid
  connect
  createLocalProvider
  createWorkerProvider
  createRestProvider
  createPivotProvider
  createHybridProvider
  exportRows

provider methods
  createRestProvider: describe execute export

error fields
  status code path

error codes (server)
  validation unknown_source limit_exceeded busy unauthorized backend malformed

error codes (exportRows)
  too_many_rows source_changed module_not_loaded

custom properties (set)
  --og-font --og-font-mono --og-font-size --og-surface --og-surface-2 --og-ink --og-ink-muted \
--og-line --og-line-strong --og-accent --og-on-accent --og-radius --og-pad --og-focus-width \
--og-row-height --og-header-height --og-filter-height --og-status-height

custom properties (computed)
  --og-accent-soft --og-accent-ink --og-selected --og-hover

parts
  body cell chip chip-remove chips chips-clear column-menu column-menu-button column-toggle \
columns columns-toggle density editor empty empty-reset empty-text facet facet-bounds \
facet-cost facet-count facet-pill facet-pills facet-value facets facets-head facets-toggle \
filter filter-clear filter-operator filter-row-toggle filter-value header layout menu-label \
page-first page-label page-last page-next page-previous pager row search search-hint \
search-input search-list select select-all select-mark sort-direction sort-index status toolbar \
total-row viewport

text keys
  aggregateAvg aggregateCell aggregateCount aggregateGroup aggregateMax aggregateMin \
aggregateNone aggregateRange aggregateSum cellRequired chipRemove chipsClear chipsGroup clear columnAtEdge \
columnHidden columnMenu columnMoved columnShown columnWidth columnsGroup densityComfortable \
densityCompact densityGroup densityNormal empty emptyFiltered emptyReset emptySource emptyValue \
error errorUnknown facetChipValues facetFrom facetQueries facetTo facetsGroup facetsReset \
facetsToggle filterColumn filterGroup filterInvalid filterRemoved filterRowToggle \
filtersCleared groupByColumn groupChip groupCollapsed groupExpanded groupInvalid groupRow \
groupSecondLevel hideColumn lang loading matchesOne matchesOther noValue operatorLabel \
operators pageFirst pageLast pageNext pageOf pagePrevious queryAnd queryMissingValue \
queryUnknownColumn queryWrongOperator rowsOne rowsOther searchChip searchHint searchLabel \
searchPlaceholder searchSuggestions selectAll selectedAll selectionCleared sortAscending \
sortDescending subtotal toolbarGroup total totalRow typeBool typeDate typeInteger typeNumber \
typeText typeTime ungroupColumn valueLabel
";
        assert_eq!(surface(), expected);
    }

    /// Every frozen name, without the section headings and element prefixes.
    fn frozen_names() -> Vec<String> {
        surface()
            .lines()
            .filter(|line| line.starts_with("  "))
            .flat_map(|line| {
                let line = line.trim();
                // `opengrid-grid: columns datasource …` — the tag is its own entry.
                let names = line.split_once(": ").map_or(line, |(_, names)| names);
                names
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The freeze and the documentation are one promise (point 70). Phase E
    /// kept its documentation in the gitignored plan and shipped none; a name
    /// that is frozen but not in `docs/api.md` is that failure again, one name
    /// at a time.
    #[test]
    fn every_frozen_name_is_documented() {
        let docs = include_str!("../../../docs/api.md");
        let missing: Vec<String> = frozen_names()
            .into_iter()
            // A function is documented with its signature, an element as a tag.
            .filter(|name| {
                ![
                    format!("`{name}`"),
                    format!("`{name}("),
                    format!("`<{name}>`"),
                ]
                .iter()
                .any(|form| docs.contains(form.as_str()))
            })
            .collect();
        assert!(
            missing.is_empty(),
            "frozen but not in docs/api.md: {missing:?}"
        );
    }

    /// The frozen names of some sections of the surface, without the headings
    /// and element prefixes.
    fn frozen_names_in(sections: &[&str]) -> Vec<String> {
        let mut section = "";
        let mut names = Vec::new();
        for line in surface().lines() {
            if !line.starts_with("  ") {
                section = line;
                continue;
            }
            if !sections.contains(&section) {
                continue;
            }
            let line = line.trim();
            if let Some((tag, rest)) = line.split_once(": ") {
                names.push(tag.to_owned());
                names.extend(rest.split_whitespace().map(str::to_owned));
            } else {
                names.extend(line.split_whitespace().map(str::to_owned));
            }
        }
        names
    }

    /// The types are the same promise again (point 75). A page in TypeScript
    /// reads `loader.d.ts`, not `docs/api.md`; a frozen name missing there is a
    /// name that page cannot use without casting. Parts and custom properties
    /// are CSS and have no type to be in.
    #[test]
    fn every_frozen_name_is_typed() {
        let types = include_str!("../../../packages/opengrid/loader.d.ts");
        // A tag, event or text key is a string literal, a function a method.
        let typed = |name: &str| {
            [
                format!("\"{name}\""),
                format!("{name}?:"),
                format!("{name}("),
            ]
            .iter()
            .any(|form| types.contains(form.as_str()))
        };
        let mut missing: Vec<String> = frozen_names_in(&[
            "elements",
            "events",
            "functions",
            "loader exports",
            "provider methods",
            "error fields",
            "error codes (server)",
            "error codes (exportRows)",
            "text keys",
        ])
        .into_iter()
        .filter(|name| !typed(name))
        .collect();

        // An attribute is a property of **its element's** interface — `mode`
        // on the grid, not somewhere in the file (the hybrid provider has a
        // `mode` too).
        for line in surface()
            .split("\nattributes\n")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .expect("the surface lists attributes")
            .lines()
        {
            let (tag, names) = line.trim().split_once(": ").expect("tag: names");
            let interface = match tag {
                "opengrid-grid" => "OpengridGridAttributes",
                "opengrid-table" => "OpengridTableAttributes",
                "opengrid-pivot" => "OpengridPivotAttributes",
                other => panic!("an element without an attributes interface: {other}"),
            };
            let block = types
                .split(&format!("export interface {interface} {{"))
                .nth(1)
                .and_then(|rest| rest.split("\n}").next())
                .unwrap_or_else(|| panic!("loader.d.ts declares {interface}"));
            for name in names.split_whitespace() {
                let property = [format!("\n  {name}?:"), format!("\n  \"{name}\"?:")];
                if !property.iter().any(|form| block.contains(form.as_str())) {
                    missing.push(format!("{tag}[{name}]"));
                }
            }
        }

        assert!(
            missing.is_empty(),
            "frozen but not in packages/opengrid/loader.d.ts: {missing:?}"
        );
    }

    /// `exportRows` (loader.js) splits the CSV options off its own with a list
    /// of its own; `export.rs` reads them with `CSV_OPTION_KEYS`. One key more
    /// in the loader and it would hand on a key the module refuses, one fewer
    /// and a CSV option would be an "unknown option". Compared as text: the
    /// module is wasm32-only, so no host test can name the constant.
    #[test]
    fn the_loader_and_the_module_agree_on_the_csv_options() {
        fn keys<'a>(source: &'a str, declaration: &str) -> Vec<&'a str> {
            let line = source
                .lines()
                .find(|line| line.starts_with(declaration))
                .unwrap_or_else(|| panic!("declared: {declaration}"));
            let list = line.split_once('[').map_or("", |(_, rest)| rest);
            let list = list.rsplit_once('[').map_or(list, |(_, rest)| rest);
            list.split('"').skip(1).step_by(2).collect()
        }
        let loader = keys(
            include_str!("../../../packages/opengrid/loader.js"),
            "const CSV_OPTIONS = [",
        );
        let module = keys(include_str!("export.rs"), "const CSV_OPTION_KEYS:");
        assert_eq!(loader.len(), 4, "{loader:?}");
        assert_eq!(loader, module);
    }

    /// The server takes the CSV options as parameters of `POST /export`, under
    /// the names `exportRows` takes them (issue #2): `exportRows` hands them to
    /// `createRestProvider(...).export` as they are, and a name the server does
    /// not know is its `400`. Compared as text, as above: the server is a
    /// crate this one does not depend on.
    #[test]
    fn the_loader_and_the_server_agree_on_the_csv_options() {
        let line = |source: &'static str, declaration: &str| -> Vec<&'static str> {
            let line = source
                .lines()
                .find(|line| line.starts_with(declaration))
                .unwrap_or_else(|| panic!("declared: {declaration}"));
            let list = line.rsplit_once('[').map_or("", |(_, rest)| rest);
            list.split('"').skip(1).step_by(2).collect()
        };
        let loader = line(
            include_str!("../../../packages/opengrid/loader.js"),
            "const CSV_OPTIONS = [",
        );
        let server = line(
            include_str!("../../opengrid-server/src/export.rs"),
            "const CSV_PARAMETERS:",
        );
        assert_eq!(server.len(), 4, "{server:?}");
        assert_eq!(loader, server);
    }

    /// `createRestProvider` answers the methods the freeze lists — no more,
    /// no fewer. A method is a promise like an export is.
    #[test]
    fn the_rest_provider_has_the_frozen_methods() {
        let loader = include_str!("../../../packages/opengrid/loader.js");
        let body = loader
            .split("export function createRestProvider(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("loader.js defines createRestProvider");
        let mut methods: Vec<&str> = body
            .lines()
            // A method is a line at the object's indent that is a name and `(`;
            // `if (`, `return fetch(` and deeper lines are not.
            .filter_map(|line| {
                let line = line.strip_prefix("    ")?;
                let line = line.strip_prefix("async ").unwrap_or(line);
                let (name, _) = line.split_once('(')?;
                (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()))
                    .then_some(name)
            })
            .collect();
        methods.sort_unstable();
        assert_eq!(methods, REST_PROVIDER_METHODS);
    }

    /// The loader exports what the freeze lists — no more, no fewer. An export
    /// is a promise (point 39), and one added without the list is a promise
    /// nobody decided to make.
    #[test]
    fn the_loader_exports_are_the_frozen_ones() {
        let loader = include_str!("../../../packages/opengrid/loader.js");
        // `exportRows` is `async`: a function all the same.
        fn function(line: &str) -> Option<&str> {
            line.strip_prefix("export function ")
                .or_else(|| line.strip_prefix("export async function "))
        }
        let mut exported: Vec<&str> = loader
            .lines()
            .filter_map(function)
            .filter_map(|rest| rest.split('(').next())
            .collect();
        exported.sort_unstable();
        let mut frozen = LOADER_EXPORTS.to_vec();
        frozen.sort_unstable();
        assert_eq!(exported, frozen);
        assert!(
            !loader
                .lines()
                .any(|line| line.starts_with("export ") && function(line).is_none()),
            "loader.js exports something other than a function"
        );
    }

    /// The parts list of the documentation is the frozen one — no more, no
    /// fewer. A documented part the element does not write is a promise it
    /// breaks the first time a page styles it.
    #[test]
    fn the_documented_parts_are_the_frozen_parts() {
        let docs = include_str!("../../../docs/api.md");
        let list = docs
            .split("**Parts:**")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .expect("docs/api.md has a **Parts:** paragraph");
        let mut documented: Vec<String> = list
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        documented.sort_unstable();
        assert_eq!(documented, parts());
    }

    /// The server's codes in the freeze are the wire's `ErrorCode`, the same
    /// set in the same order. The variants are read from the enum's source —
    /// no host code can enumerate them — so a variant added to the wire
    /// without its place in the list fails here; and every listed name has to
    /// read as a variant and serialize back to itself, so a misspelt one fails
    /// too. The list is closed, and growing it is a wire-format change
    /// (issue #16 added `busy`).
    #[test]
    fn the_server_codes_are_the_wire_codes() {
        use opengrid_datasource::wire::ErrorCode;
        let wire = include_str!("../../opengrid-datasource/src/wire.rs");
        let body = wire
            .split("pub enum ErrorCode {")
            .nth(1)
            .and_then(|rest| rest.split("\n}").next())
            .expect("wire.rs declares ErrorCode");
        // `UnknownSource,` → `unknown_source`, as `rename_all = "snake_case"`.
        let variants: Vec<String> = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("//") && !line.starts_with('#'))
            .filter_map(|line| line.strip_suffix(','))
            .map(|name| {
                let mut snake = String::new();
                for (at, c) in name.chars().enumerate() {
                    if c.is_ascii_uppercase() && at > 0 {
                        snake.push('_');
                    }
                    snake.push(c.to_ascii_lowercase());
                }
                snake
            })
            .collect();
        assert_eq!(
            variants, SERVER_ERROR_CODES,
            "the wire's variants, in order"
        );
        for name in SERVER_ERROR_CODES {
            let code: ErrorCode = serde_json::from_str(&format!("\"{name}\""))
                .unwrap_or_else(|_| panic!("not a wire code: {name}"));
            assert_eq!(serde_json::to_string(&code).unwrap(), format!("\"{name}\""));
        }
    }

    /// `exportRows`' codes in the freeze are the ones loader.js gives — no
    /// more, no fewer — and none of them is a server's, so a page can tell
    /// where a failure came from by its code alone.
    #[test]
    fn the_export_codes_are_the_loaders() {
        let loader = include_str!("../../../packages/opengrid/loader.js");
        let mut given: Vec<&str> = loader
            .split("coded(")
            .skip(1)
            .filter_map(|rest| {
                // A call, `coded(\n      "too_many_rows",`, not the definition.
                let rest = rest.trim_start().strip_prefix('"')?;
                rest.split_once('"').map(|(code, _)| code)
            })
            .collect();
        given.sort_unstable();
        given.dedup();
        let mut frozen = EXPORT_ERROR_CODES.to_vec();
        frozen.sort_unstable();
        assert_eq!(given, frozen);
        for code in EXPORT_ERROR_CODES {
            assert!(!SERVER_ERROR_CODES.contains(&code), "{code} is a server's");
        }
    }

    /// `ErrorCode` in loader.d.ts is the whole list, both halves, in their
    /// order: a page in TypeScript switches over that union, and a code
    /// missing from it would be one its `switch` cannot name.
    #[test]
    fn the_typed_codes_are_the_frozen_codes() {
        let types = include_str!("../../../packages/opengrid/loader.d.ts");
        let union = types
            .split("export type ErrorCode =")
            .nth(1)
            .and_then(|rest| rest.split(';').next())
            .expect("loader.d.ts declares ErrorCode");
        let typed: Vec<&str> = union.split('"').skip(1).step_by(2).collect();
        let frozen: Vec<&str> = SERVER_ERROR_CODES
            .iter()
            .chain(EXPORT_ERROR_CODES.iter())
            .copied()
            .collect();
        assert_eq!(typed, frozen);
    }
}
