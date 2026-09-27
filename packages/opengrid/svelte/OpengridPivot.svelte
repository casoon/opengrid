<!-- `<opengrid-pivot>` for Svelte 5 (plan point 79). See Opengrid.svelte. -->
<script>
  import Opengrid from "./Opengrid.svelte";

  let { label, datasource, rows, columns, values, theme, view = $bindable(), element = $bindable(), ...rest } = $props();
</script>

<Opengrid
  tag="opengrid-pivot"
  attributes={{ label, datasource, rows, columns, values, theme }}
  bind:view
  bind:element
  {...rest}
/>
