<!-- `<opengrid-grid>` for Svelte 5 (plan point 79). See Opengrid.svelte. -->
<script>
  import Opengrid from "./Opengrid.svelte";

  let {
    label,
    datasource,
    columns,
    windowSize,
    pageSize,
    mode,
    groupBy,
    tree,
    treeKey,
    rowKey,
    addRow = false,
    search = false,
    facets = false,
    toolbar = false,
    columnMenu = false,
    selection = false,
    density,
    theme,
    view = $bindable(),
    element = $bindable(),
    ...rest
  } = $props();

  /** A boolean attribute is present or absent; `false` is absent. */
  const flag = (on) => (on ? "" : undefined);
  const text = (value) => (value == null ? undefined : String(value));

  let inner;

  export function setCellState(key, column, state, message) {
    inner.setCellState(key, column, state, message);
  }

  export function setValues(values) {
    inner.setValues(values);
  }
</script>

<Opengrid
  bind:this={inner}
  tag="opengrid-grid"
  attributes={{
    label,
    datasource,
    columns,
    "window-size": text(windowSize),
    "page-size": text(pageSize),
    mode,
    "group-by": groupBy,
    tree,
    "tree-key": treeKey,
    "row-key": rowKey,
    "add-row": flag(addRow),
    search: flag(search),
    facets: flag(facets),
    toolbar: flag(toolbar),
    "column-menu": flag(columnMenu),
    selection: flag(selection),
    density,
    theme,
  }}
  bind:view
  bind:element
  {...rest}
/>
