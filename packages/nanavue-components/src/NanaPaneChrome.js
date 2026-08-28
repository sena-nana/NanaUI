/**
 * NanaPaneChrome — semantic peer of Runtime `PaneChrome` (`nana-pane-chrome`).
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

export const NanaPaneChrome = {
  name: "NanaPaneChrome",
  props: {
    active: { type: Boolean, default: true },
    disabled: { type: Boolean, default: false },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const children = [
        wrapSlot("header", slots.header?.()),
        wrapSlot("tabs", slots.tabs?.()),
        wrapSlot("body", slots.body?.() || slots.default?.()),
      ].filter(Boolean);
      return h(
        "nana-pane-chrome",
        {
          ...attrs,
          class: ["nana-pane-chrome", attrs.class].flat().filter(Boolean).join(" "),
          active: props.active && !props.disabled,
          disabled: props.disabled,
          "data-agent-id": attrs["data-agent-id"] || "nana.pane-chrome",
        },
        children,
      );
    };
  },
};

export default NanaPaneChrome;
