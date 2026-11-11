// What the specs share, as far as their copies were byte for byte the same.
//
// `shadow` runs a function against the shadow root of the page's first
// `<opengrid-grid>` and hands it one argument; `status` reads the status line.
// The specs that differ — another element, a selector, another return — keep
// their own: a helper that took every variant's options would be harder to read
// than the copies it replaced (#361).

export const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

export const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);
