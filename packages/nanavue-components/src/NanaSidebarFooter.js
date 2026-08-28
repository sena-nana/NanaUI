/**
 * NanaSidebarFooter — semantic peer of Runtime `SidebarFooter`.
 */
import { h } from "@vue/runtime-core";

export const NanaSidebarFooter = {
  name: "NanaSidebarFooter",
  setup(_props, { slots, attrs }) {
    return () =>
      h(
        "nana-sidebar-footer",
        {
          ...attrs,
          class: ["nana-sidebar-footer", attrs.class].flat().filter(Boolean).join(" "),
          "data-slot": attrs["data-slot"] || "footer",
          "data-agent-id": attrs["data-agent-id"] || "nana.sidebar-footer",
        },
        slots.default?.(),
      );
  },
};

export default NanaSidebarFooter;
