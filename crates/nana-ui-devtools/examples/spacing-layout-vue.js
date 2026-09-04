// Bundle with the vue-runtime-probe toolchain; pass the output path to spacing-layout.
import { h, ref } from "@vue/runtime-core";
import { createApp } from "../../../packages/nanavue-runtime/src/createNanaRenderer.js";
import { NanaSettingsPage } from "../../../packages/nanavue-components/src/NanaSettingsPage.js";
import { NanaCard } from "../../../packages/nanavue-components/src/NanaCard.js";

const padding = ref(undefined);
const settings = {
  defaultTab: "appearance", hideHeader: true,
  tabs: [{ key: "appearance", id: "appearance", label: "Appearance" }],
};
createApp({
  setup() {
    return () => h(NanaSettingsPage, { settings, contentPadding: padding.value, "data-agent-id": "page" }, {
      default: () => h("div", { style: "display:flex;flex-direction:column;width:100%;gap:16px;align-items:stretch" },
        Array.from({ length: 5 }, (_, i) => h(NanaCard, { style: "width:100%;height:80px", "data-agent-id": `card-${i}` }, {
          default: () => h("div", { style: "display:flex;flex-direction:column;width:100%;gap:8px;align-items:stretch" }, [
            h("span", { style: "width:100%;font-family:sans-serif;white-space:nowrap", "data-agent-id": `title-${i}` }, `Section ${i + 1}`),
          ]),
        })),
      ),
    });
  },
}).mount();
globalThis.spacingScroll = (id) => globalThis.__nanaHost.call("setScrollOffset", [id, 0, 10000]);
globalThis.spacingPadding = (value) => { padding.value = value; };
