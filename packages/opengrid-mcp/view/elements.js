// The element module as the MCP view uses it (#140): the same exports as
// `@casoon/opengrid`'s module, from the wasm2js build — no WebAssembly, which
// the hosts' CSP does not allow.
export {
  register,
  set_provider,
  set_texts,
  set_formats,
  set_columns,
  get_view,
  set_view,
  get_query,
} from "../build/elements/opengrid_web_components.js";
