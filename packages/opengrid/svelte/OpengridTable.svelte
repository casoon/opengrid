<!-- `<opengrid-table>` for Svelte 5 (plan point 79). See Opengrid.svelte. -->
<script>
  import Opengrid from "./Opengrid.svelte";

  let { label, datasource, columns, view = $bindable(), element = $bindable(), ...rest } = $props();
</script>

<Opengrid tag="opengrid-table" attributes={{ label, datasource, columns }} bind:view bind:element {...rest} />
