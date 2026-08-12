import { createNanaApp, mountRootHandle } from "@nanaui/nanavue-runtime";
import CompatFixture from "./CompatFixture.vue";

const { createApp } = createNanaApp();
const app = createApp(CompatFixture);
app.mount(mountRootHandle());
const applicationValue = globalThis.__nanaHost.call("fixtureApplicationApi", []);

(globalThis as typeof globalThis & { __nanaSfcFixture?: unknown }).__nanaSfcFixture = {
  app,
  mounted: true,
  probe() {
    return {
      applicationValue,
      hasTauri:
        "__TAURI_INTERNALS__" in globalThis ||
        "__TAURI__" in globalThis ||
        "__TAURI_INTERNALS__" in globalThis.window ||
        "__TAURI__" in globalThis.window,
    };
  },
};
