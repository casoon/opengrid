// Bundles the view (#140): its code with the wasm2js elements and the MCP
// Apps client, minified into one module, inlined into one HTML file — what
// the `ui://opengrid/grid` resource serves. A host's view may load nothing
// from elsewhere (E40). Needs `just mcp-elements` first.
//
// Also bundles the elements alone (the CSP fixture of part 1) and, for the
// tests only, a small MCP Apps host.
import { build } from "esbuild";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const common = {
  absWorkingDir: root,
  bundle: true,
  format: "esm",
  minify: true,
  legalComments: "none",
  target: "es2022",
  logLevel: "warning",
};

await build({ ...common, entryPoints: ["view/elements.js"], outfile: "dist/elements.js" });
await build({ ...common, entryPoints: ["test/host.js"], outfile: "dist/test-host.js" });

const view = await build({ ...common, entryPoints: ["view/main.js"], write: false });
// Inline: a `</script` inside the code would end the element early.
const code = view.outputFiles[0].text.replaceAll("</script", "<\\/script");
const shell = readFileSync(`${root}view/index.html`, "utf8");
if (!shell.includes("/*VIEW*/")) throw new Error("build-view: view/index.html has no /*VIEW*/ slot");
writeFileSync(`${root}dist/view.html`, shell.replace("/*VIEW*/", () => code));
