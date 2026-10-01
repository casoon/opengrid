// Bundles the view's code into one minified module (#140): what the MCP
// resource inlines into its HTML. Needs `just mcp-elements` first.
import { build } from "esbuild";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
await build({
  absWorkingDir: root,
  entryPoints: ["view/elements.js"],
  outfile: "dist/elements.js",
  bundle: true,
  format: "esm",
  minify: true,
  legalComments: "none",
  target: "es2022",
  logLevel: "warning",
});
