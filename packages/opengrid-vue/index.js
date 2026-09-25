/**
 * Vue components for `@casoon/opengrid` (plan point 78, E32).
 *
 * The same thin shell as the React adapter: the component renders the custom
 * element with its attributes, and hands everything else — provider, texts,
 * formats, presentation, choices, the view and the events — to one `connect`.
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
 * - **`v-model:view`:** the view comes in as a prop and goes out as
 *   `update:view`, which is the reader's change `connect` reports. Writing it
 *   back is no change, so the loop ends there.
 * - **`<KeepAlive>` detaches, it does not unmount:** the connection stays, and
 *   the element brings back its own view and selection when it returns
 *   (docs/api.md §Connecting, plan point 74). Only an unmount disconnects.
 */

import { defineComponent, h, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { connect } from "@casoon/opengrid";

/** The options of `connect` that are props; deep ones compare by value. */
const SHALLOW = ["provider"];
const DEEP = ["texts", "formats", "presentation", "choices", "view"];

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

  return defineComponent({
    name,
    props,
    emits: ["update:view", "selectionChange", "cellChange"],
    setup(props, { emit }) {
      const element = ref(null);
      let connection = null;

      onMounted(() => {
        const options = {
          onViewChange: (view) => emit("update:view", view),
          onSelectionChange: (detail) => emit("selectionChange", detail),
          onCellChange: (detail) => emit("cellChange", detail),
        };
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
        );
      }
      for (const option of DEEP) {
        watch(
          () => props[option],
          (value) => connection?.update({ [option]: value }),
          { deep: true },
        );
      }

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
