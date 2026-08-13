import { createNanaApp, mountRootHandle } from "@nanaui/nanavue-runtime";
import CompatFixture from "./CompatFixture.vue";
import HostedAcceptance from "./HostedAcceptance.vue";

const { createApp } = createNanaApp();
let acceptanceMode = "";
try {
  acceptanceMode = String(globalThis.__nanaHost.call("acceptanceMode", []));
} catch {}

if (acceptanceMode) {
  (globalThis as any).__nanaHostedAcceptance = {
    mount() {
      const app = createApp(HostedAcceptance, {
        hybrid: acceptanceMode.startsWith("hybrid"),
        autoWindows: acceptanceMode.endsWith("-windows"),
      });
      app.mount(mountRootHandle());
      return { mounted: true, mode: acceptanceMode };
    },
  };
} else {
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
}
