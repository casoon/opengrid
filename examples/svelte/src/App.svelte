<script>
  // The view bound to Svelte state, a saved view to restore, a lock that
  // refuses the reader's changes, texts switched and changed in place, and a
  // grid that is taken out and put back, the way a route change does.
  import { OpengridGrid } from "@casoon/opengrid-svelte";

  // eslint-disable-next-line no-undef -- defined by vite.config.js
  const VERSION = __SVELTE_VERSION__;

  let { provider } = $props();

  const SAVED = { sort: [{ field: "customer", direction: "asc" }] };
  // One `$state` object, changed in place by "Say rows" — a change inside it
  // has to reach the grid as much as a new object does.
  const german_texts = $state({ lang: "de", matchesOne: "{count} Treffer", matchesOther: "{count} Treffer" });

  let view = $state({ sort: [{ field: "id", direction: "asc" }] });
  let selected = $state(0);
  let shown = $state(true);
  let german = $state(false);
  let locked = $state(false);
  let element = $state();

  // For the tests: what Svelte holds, and the element behind `bind:element`.
  $effect(() => {
    window.__view = $state.snapshot(view);
  });
  $effect(() => {
    window.__element = element;
  });
</script>

<main>
  <h1>opengrid in Svelte {VERSION}</h1>
  <p>
    <button type="button" onclick={() => (shown = !shown)}>
      {shown ? "Hide the grid" : "Show the grid"}
    </button>
    <button type="button" onclick={() => (view = SAVED)}>Restore the saved view</button>
    <button type="button" aria-pressed={german} onclick={() => (german = !german)}>German</button>
    <button type="button" onclick={() => (german_texts.matchesOther = "{count} Zeilen")}>Say rows</button>
    <button type="button" aria-pressed={locked} onclick={() => (locked = !locked)}>Lock the view</button>
  </p>
  <p id="selected">{selected} rows selected</p>
  {#if shown}
    <OpengridGrid
      label="Orders"
      datasource="orders"
      columns="id,customer,country,amount,qty"
      windowSize={40}
      selection
      toolbar
      class="orders"
      {provider}
      texts={german ? german_texts : undefined}
      bind:view={() => view, (next) => {
        if (!locked) view = next;
      }}
      bind:element
      onselectionchange={(detail) => (selected = detail.count)}
    />
  {/if}
</main>
