// Copies the built opengrid modules and the conformance dataset into
// public/opengrid/, so the demo page loads them from the site itself.
// Build them first, from the repository root:
//   just wasm-build-components && just wasm-build
import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const out = resolve(repo, 'site/public/opengrid');

const files = {
  'packages/opengrid/loader.js': 'loader.js',
  'packages/opengrid/pkg': 'pkg',
  'examples/engine-demo/pkg/opengrid_wasm.js': 'engine/opengrid_wasm.js',
  'examples/engine-demo/pkg/opengrid_wasm_bg.wasm': 'engine/opengrid_wasm_bg.wasm',
  'crates/opengrid-conformance/data/orders.csv': 'data/orders.csv',
  'crates/opengrid-conformance/data/orders.schema.json': 'data/orders.schema.json',
};

const missing = Object.keys(files).filter((from) => !existsSync(resolve(repo, from)));
if (missing.length > 0) {
  console.error(`demo-assets: missing ${missing.join(', ')}`);
  console.error('Build the modules first: just wasm-build-components && just wasm-build');
  process.exit(1);
}

rmSync(out, { recursive: true, force: true });
for (const [from, to] of Object.entries(files)) {
  const target = resolve(out, to);
  mkdirSync(dirname(target), { recursive: true });
  cpSync(resolve(repo, from), target, { recursive: true, filter: (src) => !src.endsWith('.d.ts') });
}
console.log(`demo-assets: copied ${Object.keys(files).length} entries to public/opengrid/`);
