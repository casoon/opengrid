<!-- `<opengrid-table>` for Svelte 5 (plan point 79). See Opengrid.svelte. -->
<script>
  import Opengrid from "./Opengrid.svelte";

  let { label, datasource, columns, theme, view = $bindable(), element = $bindable(), ...rest } = $props();
</script>

<Opengrid tag="opengrid-table" attributes={{ label, datasource, columns, theme }} bind:view bind:element {...rest} />
