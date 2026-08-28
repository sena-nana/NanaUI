/**
 * NanaDesktopShell — host tag for Runtime `DesktopShell` (`nana-desktop-shell`).
 * Named slots: `titleBar`, `primary`, `navigation`, `navigationFooter`,
 * `inspector`, `bottom`, `overlay`.
 */
import { h } from "@vue/runtime-core";

function flatten(nodes) {
  return (Array.isArray(nodes) ? nodes : nodes == null ? [] : [nodes])
    .flat(Infinity)
    .filter(Boolean);
}

function wrapSlot(name, nodes) {
  const children = flatten(nodes);
  if (!children.length) return null;
  return h("div", { "data-slot": name }, children);
}

export const NanaDesktopShell = {
  name: "NanaDesktopShell",
  props: {
    title: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const children = [
        wrapSlot("title-bar", slots.titleBar?.() || slots["title-bar"]?.()),
        wrapSlot("primary", slots.primary?.() || slots.default?.()),
        wrapSlot("navigation", slots.navigation?.()),
        wrapSlot(
          "navigation-footer",
          slots.navigationFooter?.() || slots["navigation-footer"]?.(),
        ),
        wrapSlot("inspector", slots.inspector?.()),
        wrapSlot("bottom", slots.bottom?.()),
        wrapSlot("overlay", slots.overlay?.()),
      ].filter(Boolean);
      return h(
        "nana-desktop-shell",
        {
          ...attrs,
          class: ["nana-desktop-shell", attrs.class].flat().filter(Boolean).join(" "),
          title: props.title,
          label: props.title,
          "data-agent-id": attrs["data-agent-id"] || "nana.desktop-shell",
        },
        children,
      );
    };
  },
};

export default NanaDesktopShell;
