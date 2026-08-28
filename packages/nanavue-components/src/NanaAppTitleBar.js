/**
 * NanaAppTitleBar — semantic peer of Runtime `AppTitleBar` (`nana-app-title-bar`).
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

export const NanaAppTitleBar = {
  name: "NanaAppTitleBar",
  props: {
    title: { type: String, default: "" },
    label: { type: String, default: "" },
    maximized: { type: Boolean, default: false },
    windowControls: { type: Boolean, default: undefined },
    centerWidth: { type: Number, default: undefined },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const children = [
        wrapSlot("leading", slots.leading?.()),
        wrapSlot("center", slots.center?.() || slots.default?.()),
        wrapSlot("trailing", slots.trailing?.()),
        wrapSlot("controls", slots.controls?.()),
      ].filter(Boolean);
      return h(
        "nana-app-title-bar",
        {
          ...attrs,
          class: ["nana-app-title-bar", attrs.class].flat().filter(Boolean).join(" "),
          title: props.title || props.label,
          label: props.title || props.label,
          maximized: props.maximized,
          "window-controls": props.windowControls,
          "center-width": props.centerWidth,
          "data-slot": attrs["data-slot"] || "title-bar",
          "data-agent-id": attrs["data-agent-id"] || "nana.app-title-bar",
        },
        children,
      );
    };
  },
};

export default NanaAppTitleBar;
