<!--
  The element and its connection (plan point 79) — internal; the page uses
  OpengridGrid, OpengridTable and OpengridPivot.

  The element is rendered with its attributes, so they are in the server HTML
  too; everything else goes to one `connect` from `@casoon/opengrid`:

  - One effect reads every option — objects through `$state.snapshot`, so a
    change inside a `$state` object counts (for `formats` and `validators`,
    the functions in them keep their identity) — and passes on what is set now or was set last time:
    a prop taken away is reset, one never given is left alone (the same rule
    as the React adapter).
  - `bind:view`: the view the reader changed is written into the bound prop;
    the effect then hands it back, and `connect` knows that is no change. A
    binding that refuses it (`bind:view={get, set}` whose setter keeps the old
    view) gets its own view written back after the tick — controlled, as in
    React and Vue. A `view={…}` without `bind:` follows Svelte's rule for
    bindable props: the reader's change becomes the component's own value,
    as a typed-in `value` does on an `<input>` without `bind:`.
  - `bind:element` gives the page the element, as `ref` does in React;
    `bind:this` the component, with the connection's `setCellState` and
    `setValues`.
  - `onrowadd` answers with the new row's state, as `onRowAdd` does in
    `connect`.
  - The connection is made once the element exists and closed when the
    component is destroyed. The element keeps its own state for as long as it
    lives (docs/api.md §Connecting).
-->
<script>
  import { tick, untrack } from "svelte";
  import { connect } from "../loader.js";

  let {
    tag,
    attributes,
    view = $bindable(),
    element = $bindable(),
    defaultView,
    provider,
    texts,
    formats,
    presentation,
    choices,
    actions,
    validators,
    onviewchange,
    onselectionchange,
    oncellchange,
    onrowadd,
    onpaste,
    onbulkchange,
    onrowaction,
    onsourcereplaced,
    ...rest
  } = $props();

  let connection = null;
  let previous = {};

  $effect(() => {
    const options = {
      provider,
      texts: $state.snapshot(texts),
      formats: formats && snapshotKeepingFunctions(formats),
      presentation: $state.snapshot(presentation),
      choices: $state.snapshot(choices),
      actions: $state.snapshot(actions),
      validators: validators && snapshotKeepingFunctions(validators),
      view: $state.snapshot(view),
      defaultView: $state.snapshot(defaultView),
    };
    untrack(() => {
      const callbacks = {
        onViewChange: (next) => {
          view = next;
          onviewchange?.(next);
          tick().then(() => {
            if (view != null) connection?.update({ view: $state.snapshot(view) });
          });
        },
        onSelectionChange: (detail) => onselectionchange?.(detail),
        onCellChange: (detail) => oncellchange?.(detail),
        onRowAdd: (detail) => onrowadd?.(detail),
        onPaste: (detail) => onpaste?.(detail),
        onBulkChange: (detail) => onbulkchange?.(detail),
        onRowAction: (detail) => onrowaction?.(detail),
        onSourceReplaced: (detail) => onsourcereplaced?.(detail),
      };
      if (!connection) {
        if (options.view != null && options.defaultView != null) {
          console.warn(`[opengrid] <${tag}> has both view and defaultView; the view is controlled and leads`);
        }
        const set = Object.entries(options).filter(([, value]) => value !== undefined);
        connection = connect(element, { ...Object.fromEntries(set), ...callbacks });
      } else {
        const changed = { ...callbacks };
        for (const [name, value] of Object.entries(options)) {
          if (value !== undefined || previous[name] !== undefined) {
            changed[name] = value;
          }
        }
        connection.update(changed);
      }
      previous = options;
    });
  });

  /** Plain values all the way down, the functions kept as they are. */
  function snapshotKeepingFunctions(value) {
    if (typeof value === "function" || value === null || typeof value !== "object") {
      return value;
    }
    if (Array.isArray(value)) {
      return value.map(snapshotKeepingFunctions);
    }
    return Object.fromEntries(
      Object.entries(value).map(([name, inner]) => [name, snapshotKeepingFunctions(inner)]),
    );
  }

  export function setCellState(key, column, state, message) {
    connection?.setCellState(key, column, state, message);
  }

  export function setValues(values) {
    connection?.setValues(values);
  }

  $effect(() => () => {
    connection?.disconnect();
    connection = null;
  });
</script>

<svelte:element this={tag} bind:this={element} {...attributes} {...rest}></svelte:element>
