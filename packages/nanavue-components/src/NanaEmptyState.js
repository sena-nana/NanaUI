/**
 * NanaEmptyState — semantic peer of Runtime `EmptyState` (`nana-empty-state`).
 */
import { h } from "@vue/runtime-core";

export const NanaEmptyState = {
  name: "NanaEmptyState",
  props: {
    title: { type: String, default: "" },
    hint: { type: String, default: "" },
    icon: { type: String, default: "" },
    compact: { type: Boolean, default: false },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const children = [];
      if (slots.default) children.push(...(slots.default() || []));
      if (slots.action) {
        children.push(h("div", { "data-slot": "action" }, slots.action()));
      }
      return h(
        "nana-empty-state",
        {
          ...attrs,
          class: ["nana-empty-state", attrs.class].flat().filter(Boolean).join(" "),
          label: props.title,
          hint: props.hint,
          icon: props.icon,
          compact: props.compact,
          "data-agent-id": attrs["data-agent-id"] || "nana.empty-state",
        },
        children,
      );
    };
  },
};

export default NanaEmptyState;
