/**
 * React components for `@casoon/opengrid` (plan point 77, E32).
 *
 * A thin shell around `connect`: the component renders the custom element with
 * its attributes, and hands everything else — provider, texts, formats,
 * presentation, choices, the view and the three event callbacks — to one
 * connection. The rules of the module functions live in `connect`, not here.
 *
 * Three things only a React shell has to know:
 *
 * - **Attributes are rendered, not set later.** So they are in the server
 *   HTML too. A boolean attribute (`selection`, `toolbar`, …) is present or
 *   absent: React 18 would write `selection="false"` for `false`, and for a
 *   boolean attribute presence is what counts, so `true` becomes `""` and
 *   `false` leaves it out — the same in React 18 and 19.
 * - **StrictMode mounts twice in development**, and that costs nothing:
 *   `connect` applies only once the module has loaded, a microtask later at
 *   the earliest, so the first connection is disconnected before it has
 *   written anything, and only the second one asks the source.
 * - **An option is passed when it is set, or was set last time** — so a prop
 *   that goes away is reset (`connect`'s `undefined`), and one that was never
 *   there is not reset on every render.
 */

import { createElement, forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { connect } from "@casoon/opengrid";

/** What `connect` takes, by prop name. */
const OPTIONS = [
  "provider",
  "texts",
  "formats",
  "presentation",
  "choices",
  "view",
  "defaultView",
  "onViewChange",
  "onSelectionChange",
  "onCellChange",
];

/**
 * Builds one component.
 *
 * @param {string} tag the custom element.
 * @param {string} displayName the component's name in the React tools.
 * @param {Record<string, string>} attributes prop name → attribute name.
 * @param {string[]} booleans the props that are boolean attributes.
 */
function component(tag, displayName, attributes, booleans) {
  const Component = forwardRef(function OpengridComponent(props, ref) {
    const element = useRef(null);
    const connection = useRef(null);
    /** The options of the last render, to tell a removed prop from an absent one. */
    const previous = useRef({});
    useImperativeHandle(ref, () => element.current, []);

    const rendered = { ref: element };
    const options = {};
    for (const [name, value] of Object.entries(props)) {
      if (OPTIONS.includes(name)) {
        options[name] = value;
      } else if (name in attributes) {
        if (booleans.includes(name)) {
          rendered[attributes[name]] = value ? "" : undefined;
        } else {
          rendered[attributes[name]] = value == null ? undefined : String(value);
        }
      } else if (name === "className") {
        // `class` on a custom element: React 18 would write `classname`.
        rendered.class = value;
      } else {
        rendered[name] = value;
      }
    }

    // Unmount: the listeners go; the element keeps its state for as long as
    // it lives (docs/api.md §Connecting).
    useEffect(
      () => () => {
        connection.current?.disconnect();
        connection.current = null;
      },
      [],
    );

    // Every render: connect once, then update — `connect` writes only what
    // changed, so passing everything each time costs nothing.
    useEffect(() => {
      const last = previous.current;
      previous.current = options;
      if (connection.current) {
        const changed = {};
        for (const name of OPTIONS) {
          if (options[name] !== undefined || last[name] !== undefined) {
            changed[name] = options[name];
          }
        }
        connection.current.update(changed);
      } else {
        if (props.view != null && props.defaultView != null) {
          console.warn(
            `[opengrid] <${displayName}> has both view and defaultView; the view is controlled and leads`,
          );
        }
        connection.current = connect(element.current, options);
      }
    });

    return createElement(tag, rendered);
  });
  Component.displayName = displayName;
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
    search: "search",
    facets: "facets",
    toolbar: "toolbar",
    columnMenu: "column-menu",
    selection: "selection",
    density: "density",
  },
  ["search", "facets", "toolbar", "columnMenu", "selection"],
);

export const OpengridTable = component(
  "opengrid-table",
  "OpengridTable",
  { label: "label", datasource: "datasource", columns: "columns" },
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
  },
  [],
);
