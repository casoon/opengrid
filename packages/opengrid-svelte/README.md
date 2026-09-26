# @casoon/opengrid-svelte

The accessible grid, table and pivot of [`@casoon/opengrid`](https://www.npmjs.com/package/@casoon/opengrid)
as components for Svelte 5: `OpengridGrid`, `OpengridTable` and `OpengridPivot`.

```sh
npm install @casoon/opengrid @casoon/opengrid-svelte
```

```svelte
<script>
  import { createRestProvider } from "@casoon/opengrid";
  import { OpengridGrid } from "@casoon/opengrid-svelte";

  const provider = createRestProvider({ url: "https://example.org", source: "orders", token: "…" });
  let view = $state(null);
</script>

<OpengridGrid label="Orders" datasource="orders" columns="id,customer,amount"
              {provider} bind:view />
```

The element's attributes are props; the provider, texts, formats, presentation and the view
are options — the view bound the way Svelte binds anything. The package ships `.svelte` sources; your bundler's Svelte plugin compiles them.

For data in the browser instead of a server, pass `createLocalProvider(engine)` — see the
[`@casoon/opengrid` README](https://github.com/casoon/opengrid#data-in-the-browser). Everything
the adapters share, and what is tested, is in
[Frameworks](https://github.com/casoon/opengrid/blob/main/docs/guides/frameworks.md).

Licensed under MIT or Apache-2.0, at your option.
