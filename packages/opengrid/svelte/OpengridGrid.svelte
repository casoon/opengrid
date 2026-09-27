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
    search = false,
    facets = false,
    toolbar = false,
    columnMenu = false,
    selection = false,
    density,
    view = $bindable(),
    element = $bindable(),
    ...rest
  } = $props();

  /** A boolean attribute is present or absent; `false` is absent. */
  const flag = (on) => (on ? "" : undefined);
  const text = (value) => (value == null ? undefined : String(value));
</script>

<Opengrid
  tag="opengrid-grid"
  attributes={{
    label,
    datasource,
    columns,
    "window-size": text(windowSize),
    "page-size": text(pageSize),
    mode,
    "group-by": groupBy,
    search: flag(search),
    facets: flag(facets),
    toolbar: flag(toolbar),
    "column-menu": flag(columnMenu),
    selection: flag(selection),
    density,
  }}
  bind:view
  bind:element
  {...rest}
/>
