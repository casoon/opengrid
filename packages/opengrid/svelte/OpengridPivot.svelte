<!-- `<opengrid-pivot>` for Svelte 5 (plan point 79). See Opengrid.svelte. -->
<script>
  import Opengrid from "./Opengrid.svelte";

  let { label, datasource, rows, columns, values, sort, collapsed, toolbar = false, fields, measures, filter, theme, view = $bindable(), element = $bindable(), ...rest } = $props();

  /** A boolean attribute is present or absent; `false` is absent. */
  const flag = (on) => (on ? "" : undefined);
</script>

<Opengrid
  tag="opengrid-pivot"
  attributes={{ label, datasource, rows, columns, values, sort, collapsed, toolbar: flag(toolbar), fields, measures, filter, theme }}
  bind:view
  bind:element
  {...rest}
/>
