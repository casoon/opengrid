/**
 * Solid components for `@casoon/opengrid` — Solid 2 only.
 *
 * The same thin shell as the other adapters: the component renders the custom
 * element with its attributes, and hands everything else — provider, texts,
 * formats, presentation, choices, actions, validators, the view and the event
 * callbacks — to one `connect`. The rules of the module functions live in
 * `connect`, not here.
 *
 * What only a Solid shell has to know:
 *
 * - **No JSX in the adapter, so nothing to compile.** The element is rendered
 *   with `Dynamic` from `@solidjs/web`, which writes it the same way on the
 *   server (`renderToString`) and on the client, and claims the server's
 *   element when the page hydrates. The package ships plain JavaScript; the
 *   page's own JSX needs Solid's compiler, the adapter does not.
 * - **Attributes are rendered, not set later,** so they are in the server HTML
 *   too. A boolean attribute (`selection`, `toolbar`, …) is present or absent:
 *   `selection={false}` leaves it out. Solid writes `true` as an empty
 *   attribute and `false` not at all, on the server and in the browser alike.
 * - **Props are read where Solid tracks them,** never in the component body:
 *   one effect reads every option, and runs again when one of them changes.
 *   There is no render to run again, so nothing is passed that did not
 *   change — a view the page holds is not written back over the reader's
 *   because a text changed.
 * - **An option is passed when it changed**, and one that goes away is passed
 *   as `undefined`, which `connect` resets; one that was never there is left
 *   alone.
 * - **The view is controlled** like in the other adapters: the reader's change
 *   goes to `onViewChange`; the page takes it into its signal, and Solid
 *   passes it back, which `connect` knows is no change. A view the page does
 *   not take back is written again once the page's updates have flushed.
 * - **The ref is the element,** and the element behind it carries the
 *   connection's `setCellState` and `setValues` — `ref={grid}` with
 *   `let grid` is the usual Solid ref, a function the component calls.
 * - **Effects, and `connect`, run in the browser only.** On the server the
 *   element is rendered with its attributes and nothing is connected.
 * - **A provider created inline in JSX is created on every read.** Solid
 *   makes `provider={createRestProvider(…)}` a getter, and each read is a new
 *   provider that asks the source again. Create it once, in the component or
 *   outside it.
 */

import { createComponent, createEffect, merge, omit, onFlush, onSettled } from "solid-js";
import { Dynamic } from "@solidjs/web";
import { connect } from "../loader.js";

/** The options of `connect` that are values; the page's props of that name. */
const VALUES = [
  "provider",
  "texts",
  "formats",
  "presentation",
  "choices",
  "view",
  "defaultView",
  "actions",
  "cellActions",
  "toolbarItems",
  "presets",
  "presetOptions",
  "validators",
  "cellTemplates",
  "detailRenderer",
];

/** The options of `connect` that are callbacks. */
const CALLBACKS = [
  "onViewChange",
  "onSelectionChange",
  "onCellChange",
  "onRowAdd",
  "onPaste",
  "onBulkChange",
  "onRowAction",
  "onCellAction",
  "onToolbarAction",
  "onDrillDown",
  "onSourceReplaced",
];

/**
 * Builds one component.
 *
 * @param {string} tag the custom element.
 * @param {string} displayName the component's name in the Solid tools.
 * @param {Record<string, string>} attributes prop name → attribute name.
 * @param {string[]} booleans the props that are boolean attributes.
 */
function component(tag, displayName, attributes, booleans) {
  const owned = [...VALUES, ...CALLBACKS, ...Object.keys(attributes), "ref"];

  const Component = (props) => {
    let element = null;
    let connection = null;
    /** The options of the last run, to tell a removed prop from an absent one. */
    let previous = {};

    // The callbacks read the prop when the event comes, so a changed handler
    // needs no update, and a reader's event is not a read of the component.
    const callbacks = {};
    for (const name of CALLBACKS) {
      callbacks[name] = (detail) => props[name]?.(detail);
    }
    callbacks.onViewChange = (view) => {
      props.onViewChange?.(view);
      // Controlled: once the page's own updates have flushed, the view it
      // holds is written again — a view it took back is no change, one it
      // refused puts the grid back.
      onFlush(() => {
        const held = props.view;
        if (held != null) {
          connection?.update({ view: held });
        }
      });
    };

    createEffect(
      () => {
        const options = {};
        for (const name of VALUES) {
          options[name] = props[name];
        }
        return options;
      },
      (options) => {
        if (!connection) {
          if (options.view != null && options.defaultView != null) {
            console.warn(
              `[opengrid] <${displayName}> has both view and defaultView; the view is controlled and leads`,
            );
          }
          // Only what is set: `texts={undefined}` is no texts, not a reset.
          const set = Object.entries(options).filter(([, value]) => value !== undefined);
          connection = connect(element, { ...Object.fromEntries(set), ...callbacks });
        } else {
          const changed = {};
          for (const name of VALUES) {
            if (options[name] !== previous[name]) {
              changed[name] = options[name];
            }
          }
          connection.update(changed);
        }
        previous = options;
      },
    );

    // Unmount: the listeners go; the element keeps its state for as long as it
    // lives (docs/api.md §Connecting).
    onSettled(() => () => {
      connection?.disconnect();
      connection = null;
    });

    const rendered = {};
    for (const [prop, attribute] of Object.entries(attributes)) {
      Object.defineProperty(rendered, attribute, {
        enumerable: true,
        get: booleans.includes(prop)
          ? () => (props[prop] ? true : undefined)
          : () => {
              const value = props[prop];
              return value == null ? undefined : String(value);
            },
      });
    }

    return createComponent(
      Dynamic,
      merge(rendered, omit(props, ...owned), {
        component: tag,
        ref: (host) => {
          element = host;
          Object.assign(host, {
            setCellState: (...args) => connection?.setCellState(...args),
            setValues: (values) => connection?.setValues(values),
          });
          const user = props.ref;
          if (Array.isArray(user)) {
            user.flat(Infinity).forEach((ref) => ref?.(host));
          } else if (typeof user === "function") {
            user(host);
          }
        },
      }),
    );
  };
  Object.defineProperty(Component, "name", { value: displayName });
  return Component;
}

export const OpengridGrid = component(
  "opengrid-grid",
  "OpengridGrid",
  {
    label: "label",
    datasource: "datasource",
    columns: "columns",
    windowSize: "window-size",
    pageSize: "page-size",
    mode: "mode",
    groupBy: "group-by",
    tree: "tree",
    treeKey: "tree-key",
    rowKey: "row-key",
    rowMuted: "row-muted",
    filter: "filter",
    collation: "collation",
    addRow: "add-row",
    editLocked: "edit-locked",
    editToggle: "edit-toggle",
    enterMoves: "enter-moves",
    search: "search",
    facets: "facets",
    toolbar: "toolbar",
    columnMenu: "column-menu",
    filterBuilder: "filter-builder",
    selection: "selection",
    density: "density",
    theme: "theme",
  },
  ["search", "facets", "toolbar", "columnMenu", "filterBuilder", "selection", "addRow", "editLocked", "editToggle"],
);

export const OpengridTable = component(
  "opengrid-table",
  "OpengridTable",
  { label: "label", datasource: "datasource", columns: "columns", theme: "theme" },
  [],
);

export const OpengridPivot = component(
  "opengrid-pivot",
  "OpengridPivot",
  {
    label: "label",
    datasource: "datasource",
    rows: "rows",
    columns: "columns",
    values: "values",
    sort: "sort",
    collapsed: "collapsed",
    toolbar: "toolbar",
    fields: "fields",
    measures: "measures",
    filter: "filter",
    drillDown: "drill-down",
    theme: "theme",
  },
  ["toolbar", "drillDown"],
);
