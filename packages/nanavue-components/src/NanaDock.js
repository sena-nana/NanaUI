/**
 * NanaDock — host tag for Runtime `Dock` (`nana-dock`).
 * Children become dock items (`id` / `title` / `data-dock-id`).
 * Optional `layout` / `root` is a host dock tree; not DesktopShell chrome.
 */
import { h } from "@vue/runtime-core";

export const NanaDock = {
  name: "NanaDock",
  props: {
    layout: { type: Object, default: undefined },
    root: { type: Object, default: undefined },
  },
  setup(props, { slots, attrs }) {
    return () =>
      h(
        "nana-dock",
        {
          ...attrs,
          class: ["nana-dock", attrs.class].flat().filter(Boolean).join(" "),
          layout: props.layout,
          root: props.root,
          "data-agent-id": attrs["data-agent-id"] || "nana.dock",
        },
        slots.default?.(),
      );
  },
};

export default NanaDock;
