/**
 * NanaWorkspaceShell — sidebar + primary workspace chrome.
 * Mirrors `nana_ui_core` RegionPlacement Start/Primary layout contract
 * (not full WorkspaceController).
 */
import { computed, h } from "@vue/runtime-core";

export const NanaWorkspaceShell = {
  name: "NanaWorkspaceShell",
  props: {
    collapsed: { type: Boolean, default: false },
    title: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    const className = computed(() =>
      [
        "nana-workspace-shell",
        props.collapsed ? "is-collapsed" : "",
        attrs.class,
      ]
        .filter(Boolean)
        .join(" "),
    );

    return () =>
      h(
        "div",
        {
          ...attrs,
          class: className.value,
          "data-agent-id": attrs["data-agent-id"] || "nana.workspace",
          "data-collapsed": props.collapsed ? "true" : "false",
        },
        [
          props.title
            ? h(
                "header",
                {
                  class: "nana-workspace-shell__titlebar",
                  "data-agent-id": "nana.workspace.titlebar",
                },
                props.title,
              )
            : null,
          h("div", { class: "nana-workspace-shell__body" }, [
            h(
              "aside",
              {
                class: [
                  "nana-workspace-shell__sidebar",
                  props.collapsed ? "is-collapsed" : "",
                ]
                  .filter(Boolean)
                  .join(" "),
                "data-region": "global-navigation",
                "data-agent-id": "nana.workspace.sidebar",
              },
              slots.sidebar?.(),
            ),
            h(
              "main",
              {
                class: "nana-workspace-shell__primary",
                "data-region": "primary",
                "data-agent-id": "nana.workspace.primary",
              },
              slots.default?.() || slots.primary?.(),
            ),
          ]),
        ],
      );
  },
};

export default NanaWorkspaceShell;
