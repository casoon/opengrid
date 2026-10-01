// A small MCP Apps host for the e2e tests of the view (#140): it puts the
// view's HTML into a sandboxed iframe under the CSP every host gives by
// default, and passes its tool calls to the test, which runs them on a real
// opengrid-mcp server in Node. Built into dist/test-host.js; never shipped.
import { AppBridge, PostMessageTransport } from "@modelcontextprotocol/ext-apps/app-bridge";

// The MCP Apps default: the view's own inline code, nothing from elsewhere,
// no WebAssembly (E40).
export const DEFAULT_CSP =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:";

/**
 * Starts a view: `html` its resource, `context` the host's (theme, locale).
 * `window.__callTool(name, args)` must answer a tool call (the test exposes
 * it); messages and context updates land in `window.__host`.
 */
window.__startHost = async ({ html, context = {} }) => {
  const record = (window.__host = { messages: [], contexts: [], initialized: false });
  const frame = document.createElement("iframe");
  frame.title = "opengrid";
  frame.setAttribute("sandbox", "allow-scripts");
  frame.style.cssText = "width: 100%; height: 560px; border: 0;";
  (document.querySelector("main") ?? document.body).append(frame);

  const bridge = new AppBridge(
    null,
    { name: "opengrid test host", version: "0" },
    { serverTools: {}, updateModelContext: { text: {} }, message: { text: {} } },
    { hostContext: { theme: "light", locale: "en-US", ...context } },
  );
  bridge.oncalltool = (params) => window.__callTool(params.name, params.arguments ?? {});
  bridge.onmessage = async (params) => {
    record.messages.push(params);
    return {};
  };
  bridge.onupdatemodelcontext = async (params) => {
    record.contexts.push(params);
    return {};
  };
  const ready = new Promise((resolve) => {
    bridge.oninitialized = () => {
      record.initialized = true;
      resolve();
    };
  });
  // Listening before the view exists: its `ui/initialize` is its first word.
  await bridge.connect(new PostMessageTransport(frame.contentWindow, frame.contentWindow));
  frame.srcdoc = html.replace(
    "<head>",
    `<head><meta http-equiv="Content-Security-Policy" content="${DEFAULT_CSP}">`,
  );
  await ready;
  window.__bridge = bridge;
  return true;
};

/** The model opened a grid: the view gets the tool's result. */
window.__deliver = async (result) => {
  await window.__bridge.sendToolResult(result);
};
