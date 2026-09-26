import { provideBrowserGlobalErrorListeners } from "@angular/core";
import { bootstrapApplication } from "@angular/platform-browser";
import { loadOpengrid } from "@casoon/opengrid";
import { App } from "./app/app";
import { MODULE_URL, ORDERS, ordersProvider } from "./app/provider";

await loadOpengrid({ moduleUrl: MODULE_URL });
const provider = await ordersProvider();
bootstrapApplication(App, {
  providers: [provideBrowserGlobalErrorListeners(), { provide: ORDERS, useValue: provider }],
}).catch((error) => console.error(error));
