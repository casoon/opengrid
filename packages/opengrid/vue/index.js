/**
 * Vue components for `@casoon/opengrid` (plan point 78, E32).
 *
 * The same thin shell as the React adapter: the component renders the custom
 * element with its attributes, and hands everything else — provider, texts,
 * formats, presentation, choices, actions, validators, the view and the
 * events — to one `connect`.
 *
 * What only a Vue shell has to know:
 *
 * - **The element is rendered with `h()`,** so Vue never tries to resolve
 *   `opengrid-grid` as a component — no `compilerOptions.isCustomElement` for
 *   the page. (A page that writes the bare element in a template needs it.)
 *   `class`, `style`, `id` and `aria-*` fall through to it, as Vue does for a
 *   component's root.
 * - **Events in camelCase** (`selectionChange`, `cellChange`), which a
 *   template writes as `@selection-change` — Vue normalizes the two, and only
 *   the camelCase form gives typed `onSelectionChange` props.
 * - **`onRowAdd` is a prop, not an event:** its answer is the new row's
 *   state, and an emitted event has none. A template still writes it as
 *   `@row-add`, which Vue passes as the prop `onRowAdd`.
 * - **The template ref** exposes `setCellState` and `setValues`; `$el` is
 *   the element, as before.
 * - **`v-model:view`:** the view comes in as a prop and goes out as
 *   `update:view`, which is the reader's change `connect` reports. Writing it
 *   back is no change, so the loop ends there.
 * - **`<KeepAlive>` detaches, it does not unmount:** the connection stays, and
 *   the element brings back its own view and selection when it returns
 *   (docs/api.md §Connecting, plan point 74). Only an unmount disconnects. A
 *   view that changed while the grid was away waited in `connect` (there is no
 *   view to set on a detached element); reactivation hands it over.
 * - **Watchers run after the render** (`flush: "post"`), so the element has
 *   its new attributes before an option follows: a new `datasource` and a new
 *   `provider` together ask once, a view naming a new column finds it.
 * - **The view is controlled like in React:** a view the page does not take
 *   back from `update:view` is written again — on the next tick, once Vue has
 *   had its chance to pass the new one down.
 */

import { defineComponent, h, nextTick, onActivated, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { connect } from "../loader.js";

/** The options of `connect` that are props; deep ones compare by value. */
const SHALLOW = ["provider"];
const DEEP = ["texts", "formats", "presentation", "choices", "actions", "validators", "view"];

/** The events, by `connect`'s callback. */
const EVENTS = {
  onSelectionChange: "selectionChange",
  onCellChange: "cellChange",
  onPaste: "paste",
  onBulkChange: "bulkChange",
  onRowAction: "rowAction",
  onSourceReplaced: "sourceReplaced",
};

/**
 * Builds one component.
 *
 * @param {string} tag the custom element.
 * @param {string} name the component's name in the Vue tools.
 * @param {Record<string, string>} attributes prop name → attribute name.
 * @param {string[]} booleans the props that are boolean attributes.
 */
function component(tag, name, attributes, booleans) {
  const props = {};
  for (const prop of Object.keys(attributes)) {
    props[prop] = booleans.includes(prop) ? { type: Boolean, default: false } : null;
  }
  for (const option of [...SHALLOW, ...DEEP, "defaultView"]) {
    props[option] = { type: null, default: undefined };
  }
  props.onRowAdd = { type: Function, default: undefined };

  return defineComponent({
    name,
    props,
    emits: ["update:view", ...Object.values(EVENTS)],
    setup(props, { emit, expose }) {
      const element = ref(null);
      let connection = null;

      onMounted(() => {
        if (props.view != null && props.defaultView != null) {
          console.warn(`[opengrid] <${name}> has both view and defaultView; the view is controlled and leads`);
        }
        const options = {
          onViewChange: (view) => {
            emit("update:view", view);
            nextTick(() => {
              if (props.view != null) connection?.update({ view: props.view });
            });
          },
          onRowAdd: (detail) => props.onRowAdd?.(detail),
        };
        for (const [callback, event] of Object.entries(EVENTS)) {
          options[callback] = (detail) => emit(event, detail);
        }
        for (const option of [...SHALLOW, ...DEEP, "defaultView"]) {
          if (props[option] !== undefined) {
            options[option] = props[option];
          }
        }
        connection = connect(element.value, options);
      });

      // A changed option is passed on, and one that went away is passed as
      // `undefined`, which resets it (`connect`).
      for (const option of SHALLOW) {
        watch(
          () => props[option],
          (value) => connection?.update({ [option]: value }),
          { flush: "post" },
        );
      }
      for (const option of DEEP) {
        watch(
          () => props[option],
          (value) => connection?.update({ [option]: value }),
          { deep: true, flush: "post" },
        );
      }
      onActivated(() => connection?.update({}));

      expose({
        setCellState: (...args) => connection?.setCellState(...args),
        setValues: (values) => connection?.setValues(values),
      });

      onBeforeUnmount(() => {
        connection?.disconnect();
        connection = null;
      });

      return () => {
        const rendered = { ref: element };
        for (const [prop, attribute] of Object.entries(attributes)) {
          const value = props[prop];
          if (booleans.includes(prop)) {
            rendered[attribute] = value ? "" : undefined;
          } else {
            rendered[attribute] = value == null ? undefined : String(value);
          }
        }
        return h(tag, rendered);
      };
    },
  });
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
    addRow: "add-row",
    search: "search",
    facets: "facets",
    toolbar: "toolbar",
    columnMenu: "column-menu",
    selection: "selection",
    density: "density",
    theme: "theme",
  },
  ["search", "facets", "toolbar", "columnMenu", "selection", "addRow"],
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
    theme: "theme",
  },
  ["toolbar"],
);
