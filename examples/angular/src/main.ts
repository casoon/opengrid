import { provideBrowserGlobalErrorListeners } from "@angular/core";
import { bootstrapApplication } from "@angular/platform-browser";
import { loadOpengrid } from "@casoon/opengrid";
import { App } from "./app/app";
import { MODULE_URL, ORDERS, ROOMS, ordersProvider, roomsProvider } from "./app/provider";

await loadOpengrid({ moduleUrl: MODULE_URL });
const provider = await ordersProvider();
const rooms = await roomsProvider();
bootstrapApplication(App, {
  providers: [
    provideBrowserGlobalErrorListeners(),
    { provide: ORDERS, useValue: provider },
    { provide: ROOMS, useValue: rooms },
  ],
}).catch((error) => console.error(error));
