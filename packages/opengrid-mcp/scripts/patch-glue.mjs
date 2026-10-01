// The wasm-bindgen glue caches views of the module's memory and renews them
// when the old buffer is *detached* — what a real WebAssembly memory does when
// it grows. wasm2js's memory replaces its buffer without detaching the old one,
// so the cache would keep reading the old, smaller buffer: strings come back
// empty (`createElement('')`, found in the spike of #140). Renew whenever the
// buffer is another one.
//
// The patterns are wasm-bindgen 0.2.126's (pinned, E4); a glue that does not
// carry them fails here rather than in a host.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const file = join(process.argv[2], "opengrid_web_components_bg.js");
let source = readFileSync(file, "utf8");
const checks = [
  [
    "cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0",
    "cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.buffer !== wasm.memory.buffer",
  ],
  [
    "cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)",
    "cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer !== wasm.memory.buffer",
  ],
];
for (const [from, to] of checks) {
  if (!source.includes(from)) {
    throw new Error(`patch-glue: the memory check was not found in ${file}:\n  ${from}`);
  }
  source = source.replaceAll(from, to);
}
// The bundler target imports the module as `.wasm`; here it is JavaScript.
const glue = join(process.argv[2], "opengrid_web_components.js");
const entry = readFileSync(glue, "utf8");
const wasmImport = '"./opengrid_web_components_bg.wasm"';
if (!entry.includes(wasmImport)) {
  throw new Error(`patch-glue: no import of the module in ${glue}`);
}
writeFileSync(glue, entry.replace(wasmImport, '"./opengrid_web_components_bg.wasm.js"'));
writeFileSync(file, source);
