/**
 * NanaWorkspace — host tag for Runtime `Workspace` (`nana-workspace`).
 * Children with `region` / `data-region` become WorkspaceRegionSlots.
 * Not DesktopShell / NanaWorkspaceShell.
 */
import { h } from "@vue/runtime-core";

export const NanaWorkspace = {
  name: "NanaWorkspace",
  setup(_props, { slots, attrs }) {
    return () =>
      h(
        "nana-workspace",
        {
          ...attrs,
          class: ["nana-workspace", attrs.class].flat().filter(Boolean).join(" "),
          "data-agent-id": attrs["data-agent-id"] || "nana.workspace",
        },
        slots.default?.(),
      );
  },
};

export default NanaWorkspace;
