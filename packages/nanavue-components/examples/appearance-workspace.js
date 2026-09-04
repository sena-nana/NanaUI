/**
 * Vue example — Theme / Appearance / Workspace components (no WebView).
 *
 * Used by Lilia shell fixture and as a copy-paste reference:
 *
 *   import {
 *     NanaAppearancePanel,
 *     NanaWorkspaceShell,
 *     NanaSidebarNav,
 *     NanaButton,
 *   } from "@nanaui/nanavue-components";
 *   import "@nanaui/nanavue-components/controls.css";
 */
import { h, ref } from "@vue/runtime-core";
import { NanaAppearancePanel } from "../src/NanaAppearancePanel.js";
import { NanaWorkspaceShell } from "../src/NanaWorkspaceShell.js";
import { NanaSidebarNav } from "../src/NanaSidebarNav.js";
import { NanaButton } from "../src/NanaButton.js";

export function createAppearanceWorkspaceDemo(options = {}) {
  const active = ref(options.activeKey || "appearance");
  const navItems = [
    { key: "home", label: "Home", agentId: "demo.nav.home" },
    { key: "appearance", label: "Appearance", agentId: "demo.nav.appearance" },
    { key: "workspace", label: "Workspace", agentId: "demo.nav.workspace" },
  ];

  return {
    name: "NanaAppearanceWorkspaceDemo",
    setup() {
      return () =>
        h(
          NanaWorkspaceShell,
          {
            title: options.title || "NanaVue Components",
            collapsed: false,
            "data-agent-id": "demo.workspace",
          },
          {
            sidebar: () =>
              h(NanaSidebarNav, {
                items: navItems,
                activeKey: active.value,
                onSelect: (item) => {
                  active.value = item.key;
                },
              }),
            default: () => h("div", { style: "width:100%;padding:20px 24px" }, [(() => {
              if (active.value === "appearance") {
                return h(NanaAppearancePanel, {
                  title: "外观",
                  description: "nanavue-components Appearance demo（Segmented / Switch / Range）。",
                });
              }
              if (active.value === "workspace") {
                return h("section", { class: "nana-demo-panel" }, [
                  h("h2", null, "Workspace"),
                  h("p", null, "Region Start / Primary shell is mounted."),
                  h(NanaButton, {
                    kind: "primary",
                    label: "Primary action",
                    "data-agent-id": "demo.workspace.primary",
                  }),
                ]);
              }
              return h("section", { class: "nana-demo-panel" }, [
                h("h2", null, "Home"),
                h("p", null, "Select Appearance or Workspace in the sidebar."),
              ]);
            })()]),
          },
        );
    },
  };
}

export default createAppearanceWorkspaceDemo;
