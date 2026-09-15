import { createApp } from "@nanaui/nanavue-runtime";
import { h } from "vue";
import WebSocketProbe from "./WebSocketProbe.vue";
import ChromeProbe from "./ChromeProbe.vue";
import CompatFixture from "./CompatFixture.vue";
import HostedAcceptance from "./HostedAcceptance.vue";
let acceptanceMode = "";
try {
  acceptanceMode = String(globalThis.__nanaHost.call("acceptanceMode", []));
} catch {}

if (acceptanceMode === "websocket-probe") {
  createApp(WebSocketProbe).mount();
} else if (acceptanceMode && (globalThis as any).Nana.windows.current()?.isolation === "isolated") {
  // This bundle runs again inside an isolated window's own JavaScript realm.
  localStorage.setItem("nana.acceptance.sawMain", String(localStorage.getItem("nana.acceptance.realm")));
  localStorage.setItem("nana.acceptance.realm", "isolated");
  setTimeout(() => localStorage.setItem("nana.acceptance.timer", "fired"), 0);
  createApp({
    render() {
      return h("main", {
        style: "display:flex;flex-direction:column;gap:12px;padding:24px;background:rgba(15,23,42,.86);color:white",
      }, [
        h("h2", null, "Vue isolated window"),
        h("p", null, "Same V8 and GPU, its own JavaScript realm."),
        h("button", { onClick: () => (globalThis as any).Nana.windows.current()?.close() }, "Close window"),
      ]);
    },
  }).mount();
} else if (acceptanceMode) {
  (globalThis as any).__nanaHostedAcceptance = {
    mount() {
      const app =
        acceptanceMode === "chrome-probe"
          ? createApp(ChromeProbe)
          : createApp(HostedAcceptance, {
              hybrid: acceptanceMode.startsWith("hybrid"),
              autoWindows: acceptanceMode.endsWith("-windows"),
            });
      app.mount();
      return { mounted: true, mode: acceptanceMode };
    },
  };
} else {
  const app = createApp(CompatFixture);
  app.mount();
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
