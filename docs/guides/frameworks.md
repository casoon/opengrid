---
title: Frameworks
description: React, Vue and Svelte components, and the same elements in Angular, Astro and server-rendered pages without one.
order: 4
---

The elements are Web Components, so they work in any page. What a framework adds is a way to
hand them their provider, texts and view the way the framework hands anything to anything —
props, `v-model`, `bind:`. Three adapters do that; for everything else there is `connect`, the
function they are all built on ([The public API → Connecting](../../api/#connecting)).

| | Package | View |
|---|---|---|
| React 18 and 19 | `@casoon/opengrid-react` | `view` + `onViewChange`, or `defaultView` |
| Vue 3.3 and later | `@casoon/opengrid-vue` | `v-model:view`, or `defaultView` |
| Svelte 5 | `@casoon/opengrid-svelte` | `bind:view`, or `defaultView` |
| Angular 22 | a directive over `connect` (below) | `[(view)]` |
| Lit, Astro, plain pages | `@casoon/opengrid` | `connect(host, { view, onViewChange })` |

None of them is published yet; like the element package, they build from the repository.

## What every adapter does the same way

- **Attributes are props** in camelCase — `label`, `datasource`, `columns`, `windowSize`,
  `groupBy`, `columnMenu`, … — and they are **rendered**, so they are in the server's HTML. A
  boolean attribute is present or absent; `selection={false}` leaves it out.
- **Everything else goes to `connect`:** `provider`, `texts`, `formats`, `presentation` (what
  `set_columns` takes — named apart from the `columns` attribute), `choices`, `view`,
  `defaultView`, and the three events.
- **A prop that goes away is reset** — texts back to English, no formats — and one that was never
  given is left alone.
- **The source is asked once** for the first result, in React's StrictMode too.
- **A grid taken out is collected**; one that is only moved, or parked by Vue's `<KeepAlive>`,
  comes back as it was, without asking again.

Four rules to know, because they come from the grid and not from the framework:

1. **`provider` and format functions compare by identity.** A provider created inline —
   `provider={createRestProvider(…)}` in a React render — is a new provider on every render,
   and each one asks the source again. Create it once: outside the component, or in `useMemo`,
   a module-level `const`, a Svelte `<script module>`.
2. **`view` is controlled.** A view the page does not take back from the change event is written
   back — after the next render in React, after the tick in Vue and Svelte. With `defaultView`
   the reader leads after the first application.
3. **`density` and `groupBy` belong to the view as well.** The reader changes them from the
   toolbar and the column menu, so as props they set the start. With a controlled `view`, leave
   them out and let the view carry them.
4. **Bundlers and the WebAssembly module.** The element module is loaded from `pkg/` next to
   `loader.js` through `new URL("./pkg/…", import.meta.url)`. Vite's development server handles
   that without a setting: it pre-bundles `@casoon/opengrid` and rewrites the URL to the
   installed `pkg/` (checked by hand with Vite 8.3 and the packed package). A production build
   is another matter — Vite copies the glue module into its assets but not the `.wasm` beside
   it, so the element would fall back to the plain table. For a production build, and for any
   bundler: serve the `pkg/` directory yourself and call `loadOpengrid({ moduleUrl })` once
   before the first component mounts — every later load reuses that call. The examples do
   exactly that.

   The engine's worker follows the same rule. `createWorkerProvider()` finds `worker.js` and
   the engine under `engine/` next to `loader.js`, and without a bundler and under Vite's
   development server those defaults work. A production build breaks both: Vite inlines
   `worker.js` as a `data:` worker, which cannot import the engine, and copies the engine's glue
   without its `.wasm` (checked with Vite 8.3 and the packed package; an explicit `moduleUrl`
   alone does not help). Serve `engine/` and `worker.js` yourself and pass both:
   `createWorkerProvider({ moduleUrl, workerUrl })`, `moduleUrl` as a string. The engine on
   the main thread, imported from `@casoon/opengrid/engine/opengrid_wasm.js`, is bundled like
   any module and needs nothing.

## React

```jsx
import { useMemo, useState } from "react";
import { createRestProvider } from "@casoon/opengrid";
import { OpengridGrid } from "@casoon/opengrid-react";

export function Orders() {
  const provider = useMemo(
    () => createRestProvider({ url: "https://example.org", source: "orders", token: "…" }),
    [],
  );
  const [view, setView] = useState(null);
  return (
    <OpengridGrid
      label="Orders"
      datasource="orders"
      columns="id,customer,amount"
      selection
      toolbar
      provider={provider}
      view={view}
      onViewChange={setView}
      onSelectionChange={(detail) => console.log(detail.rows)}
    />
  );
}
```

- `ref` is the element. `id`, `className`, `style`, `aria-*` go to it; `className` is written as
  `class`, which React 18 would not do for a custom element.
- The module says `"use client"`, so a Server Component can import it as a client boundary.
- `hidden`, `inert` and `autoFocus` follow the present-or-absent rule too: React 18 would write
  `hidden="false"` — hidden — and React 19 sets them as properties.
- **Exporting:** with `const grid = useRef(null)` and `ref={grid}`, a handler exports the view
  as `exportRows(provider, loader.module.get_query(grid.current))` — see
  [Exporting](../export/#in-the-browser).

## Vue

```vue
<script setup>
import { ref } from "vue";
import { createRestProvider } from "@casoon/opengrid";
import { OpengridGrid } from "@casoon/opengrid-vue";

const provider = createRestProvider({ url: "https://example.org", source: "orders", token: "…" });
const view = ref(null);
</script>

<template>
  <OpengridGrid
    v-model:view="view"
    label="Orders"
    datasource="orders"
    columns="id,customer,amount"
    selection
    :provider="provider"
    @selection-change="(detail) => console.log(detail.rows)"
  />
</template>
```

- The element is rendered by the adapter, so **no** `compilerOptions.isCustomElement` is needed.
  A page that writes `<opengrid-grid>` into a template itself needs it.
- The events are `update:view`, `selectionChange` and `cellChange` — `@selection-change` and
  `@cell-change` in a template.
- Under `<KeepAlive>` the connection stays; the grid comes back with its view and selection.
- A provider held in `ref()` is a proxy. That is fine for the providers `@casoon/opengrid`
  creates; a provider class of your own with `#private` fields belongs in `markRaw` or
  `shallowRef`.
- **Exporting:** a template ref is the component, and its `$el` the element — with
  `const grid = ref(null)` and `ref="grid"`, a handler exports the view as
  `exportRows(provider, loader.module.get_query(grid.value.$el))` — see
  [Exporting](../export/#in-the-browser).

## Svelte

```svelte
<script>
  import { createRestProvider } from "@casoon/opengrid";
  import { OpengridGrid } from "@casoon/opengrid-svelte";

  const provider = createRestProvider({ url: "https://example.org", source: "orders", token: "…" });
  let view = $state(null);
  let grid = $state();
</script>

<OpengridGrid
  label="Orders"
  datasource="orders"
  columns="id,customer,amount"
  selection
  {provider}
  bind:view
  bind:element={grid}
  onselectionchange={(detail) => console.log(detail.rows)}
/>
```

- The package ships `.svelte` sources; the page's bundler compiles them with its Svelte plugin,
  as for any Svelte library.
- The callbacks are spelled the Svelte way: `onviewchange`, `onselectionchange`, `oncellchange`.
- Controlled is `bind:view`. A `view={…}` **without** `bind:` follows Svelte's rule for bindable
  props: after the reader's first change the component holds its own value, as an
  `<input value>` without `bind:` does.
- A change **inside** a `$state` object — `texts.loading = "…"` — reaches the grid as much as a
  new object does.
- **Exporting:** `bind:element={grid}` is the element, so a handler exports the view as
  `exportRows(provider, loader.module.get_query(grid))` — see
  [Exporting](../export/#in-the-browser).

## Without an adapter

### Angular

Angular binds custom elements natively once the component declares `CUSTOM_ELEMENTS_SCHEMA`.
What it cannot do by itself is call `connect`; a directive does that. This one runs in
`examples/angular` — Angular 22, AOT, zoneless — and the tests check that the file there is
this text. It needs Angular 19 or later (standalone by default):

```ts
import {
  Directive,
  ElementRef,
  EventEmitter,
  Input,
  OnChanges,
  OnDestroy,
  Output,
  SimpleChanges,
  afterNextRender,
  inject,
} from "@angular/core";
import { connect, type Connection, type ConnectOptions, type View } from "@casoon/opengrid";

@Directive({ selector: "[opengrid]" })
export class OpengridDirective implements OnChanges, OnDestroy {
  @Input() provider?: ConnectOptions["provider"];
  @Input() texts?: ConnectOptions["texts"];
  @Input() view?: ConnectOptions["view"];
  @Output() viewChange = new EventEmitter<View>();

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private connection?: Connection;

  constructor() {
    // In the browser only, once rendered — never during server rendering.
    afterNextRender(() => {
      this.connection = connect(this.host.nativeElement, {
        provider: this.provider,
        texts: this.texts,
        view: this.view,
        onViewChange: (view) => this.viewChange.emit(view),
      });
    });
  }

  // Only the inputs that changed: an unchanged `view` passed along with new
  // texts would put back a view the reader has changed since.
  ngOnChanges(changes: SimpleChanges): void {
    const changed: ConnectOptions = {};
    if ("provider" in changes) changed.provider = this.provider;
    if ("texts" in changes) changed.texts = this.texts;
    if ("view" in changes) changed.view = this.view;
    this.connection?.update(changed);
  }

  ngOnDestroy(): void {
    this.connection?.disconnect();
  }
}
```

```html
<opengrid-grid opengrid label="Orders" datasource="orders" columns="id,customer,amount"
               [provider]="provider" [(view)]="view"
               (opengrid-selection-change)="onSelection($event)"></opengrid-grid>
```

- The element's own events are bound in the template as they are:
  `(opengrid-selection-change)`, `(opengrid-cell-change)`. With `strictTemplates` the handler
  takes an `Event` and reads `(event as CustomEvent<SelectionChangeDetail>).detail`.
- `ngOnChanges` passes on only the inputs that changed. Passing `view` along with every other
  change would put back a view the reader had changed since.
- `afterNextRender` connects in the browser only, after the first render — `ngAfterViewInit`
  would also run during Angular's server rendering, where there is no element to supply.
- The view is controlled through `[(view)]`. With `[view]` alone the page sets the view when
  its value changes, and in between the reader's change stays — the directive does not write
  the page's view back.
- Add inputs for `formats`, `presentation`, `choices` and `defaultView` the same way when a page
  needs them.
- **Exporting:** a template variable on the element is the element — `#grid` on
  `<opengrid-grid>` and `(click)="exportView(grid)"` — and the component's
  `exportView(grid: HTMLElement)` exports it as `exportRows(this.provider, query)` when
  `const query = (await loadOpengrid()).module?.get_query(grid)` gives one — see
  [Exporting](../export/#in-the-browser).

### Astro

An element in the markup and `connect` in a `<script>` — no island, no framework runtime:

```astro
<opengrid-grid label="Orders" datasource="orders" columns="id,customer,amount"></opengrid-grid>

<script>
  import { connect, createRestProvider } from "@casoon/opengrid";

  connect(document.querySelector("opengrid-grid"), {
    provider: createRestProvider({ url: "https://example.org", source: "orders" }),
  });
</script>
```

Using a React, Vue or Svelte adapter inside an Astro island works as it does in that framework,
with one catch: an island receives its props **serialized**, and a provider or a format function
does not survive that. Create them inside the island's own component instead.

Exporting is the same script's: `exportRows(provider, loader.module.get_query(grid))` with
`grid = document.querySelector("opengrid-grid")` — the whole button is in
[Exporting](../export/#in-the-browser).

### Server-rendered pages

ASP.NET Razor, Django, Laravel, plain HTML: the server writes the element with its attributes,
and one module script supplies it.

```html
<opengrid-grid label="Orders" datasource="orders" columns="id,customer,amount"></opengrid-grid>

<script type="module">
  import { connect, createRestProvider } from "/assets/opengrid/loader.js";

  connect(document.querySelector("opengrid-grid"), {
    provider: createRestProvider({ url: "/api", source: "orders" }),
    texts: { lang: "de", loading: "Wird geladen …" },
  });
</script>
```

An export button is the same module script's, as in [Exporting](../export/#in-the-browser):
`exportRows(provider, loader.module.get_query(grid))`, the provider kept in a variable rather
than created inline.

## What is tested, and what is not

Each adapter has an example in `examples/` that the end-to-end suite runs in Chromium — React
in both 18 and 19 — with StrictMode or a development build, axe-core, and a check that the grid
is collected once the framework takes it out. Each adapter also renders on the server, and its
packed tarball is put into a scratch project's `node_modules` — beside the framework and
nothing else from the repository — and rendered and type-checked there. The Angular directive
runs the same browser checks in an Angular 22 app built by the Angular CLI, plus a one-way
`[view]` that keeps the reader's change.

**Hydration** is tested in all three (`tests/e2e/hydration.spec.js`): the HTML the example's
`ssr.mjs` renders is put into a page before the element module has loaded, and the framework
hydrates it with the same props. React 18 and 19 and Vue say nothing, keep the server's element,
and the grid fills with one query and answers the reader; a changed attribute makes React and
Vue warn, and the test fail. Svelte 5 does not compare attributes when it hydrates — by design —
so there the test holds only that the element is kept and connected.

Not tested: React 18 rendering on the server (in the repository's Node, `react-dom@18` resolves
React 19; the React 18 test hydrates React 19's HTML, which is the same markup), React 19.2's
`<Activity>`, the types against `@types/react` 18, SvelteKit, Nuxt, Next.js and Astro as real
applications.
