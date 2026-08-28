/**
 * NanaSidebarSection — semantic peer of Runtime `SidebarSection`.
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

export const NanaSidebarSection = {
  name: "NanaSidebarSection",
  props: {
    label: { type: String, default: "" },
    title: { type: String, default: "" },
    hint: { type: String, default: "" },
    emptyText: { type: String, default: "" },
    size: { type: String, default: "medium" },
    collapsible: { type: Boolean, default: false },
    expanded: { type: Boolean, default: true },
    disabled: { type: Boolean, default: false },
    count: { type: Number, default: undefined },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const children = [
        wrapSlot("header", slots.header?.()),
        wrapSlot("tools", slots.tools?.()),
        wrapSlot("body", slots.body?.() || slots.default?.()),
      ].filter(Boolean);
      return h(
        "nana-sidebar-section",
        {
          ...attrs,
          class: ["nana-sidebar-section", attrs.class].flat().filter(Boolean).join(" "),
          label: props.label || props.title,
          hint: props.emptyText || props.hint,
          size: props.size,
          collapsible: props.collapsible,
          expanded: props.expanded,
          disabled: props.disabled,
          count: props.count,
          "data-agent-id": attrs["data-agent-id"] || "nana.sidebar-section",
        },
        children,
      );
    };
  },
};

export default NanaSidebarSection;
