# @casoon/opengrid-react

The accessible grid, table and pivot of [`@casoon/opengrid`](https://www.npmjs.com/package/@casoon/opengrid)
as components for React 18 and 19: `OpengridGrid`, `OpengridTable` and `OpengridPivot`.

```sh
npm install @casoon/opengrid @casoon/opengrid-react
```

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
    <OpengridGrid label="Orders" datasource="orders" columns="id,customer,amount"
                  provider={provider} view={view} onViewChange={setView} />
  );
}
```

The element's attributes are props; the provider, texts, formats, presentation and the view
are options — the view bound the way React binds anything. Create the provider once (`useMemo`, or outside the component): a new provider on every render asks the source again. `ref` is the element; the component is marked `"use client"`.

For data in the browser instead of a server, pass `createLocalProvider(engine)` — see the
[`@casoon/opengrid` README](https://github.com/casoon/opengrid#data-in-the-browser). Everything
the adapters share, and what is tested, is in
[Frameworks](https://github.com/casoon/opengrid/blob/main/docs/guides/frameworks.md).

Licensed under MIT or Apache-2.0, at your option.
