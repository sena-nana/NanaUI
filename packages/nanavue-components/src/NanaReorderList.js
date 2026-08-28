/**
 * NanaReorderList — semantic peer of Runtime `ReorderList` (`nana-reorder-list`).
 */
import { h } from "@vue/runtime-core";

export const NanaReorderList = {
  name: "NanaReorderList",
  props: {
    options: { type: Array, default: () => [] },
    items: { type: Array, default: undefined },
    label: { type: String, default: "" },
    size: { type: String, default: "medium" },
    spacing: { type: Number, default: undefined },
    treeDrop: { type: Boolean, default: false },
  },
  emits: ["reorder"],
  setup(props, { emit, attrs }) {
    return () =>
      h("nana-reorder-list", {
        ...attrs,
        class: ["nana-reorder-list", attrs.class].flat().filter(Boolean).join(" "),
        label: props.label,
        options: props.items || props.options,
        items: props.items || props.options,
        size: props.size,
        spacing: props.spacing,
        "tree-drop": props.treeDrop,
        "data-agent-id": attrs["data-agent-id"] || "nana.reorder-list",
        onReorder: (ev) => emit("reorder", ev),
      });
  },
};

export default NanaReorderList;
