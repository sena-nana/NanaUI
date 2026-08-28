/**
 * NanaListItem — semantic peer of Runtime `ListItem` (`nana-list-item`).
 */
import { h } from "@vue/runtime-core";

export const NanaListItem = {
  name: "NanaListItem",
  props: {
    label: { type: String, default: "" },
    selected: { type: Boolean, default: false },
    disabled: { type: Boolean, default: false },
    size: { type: String, default: "medium" },
    autoHeight: { type: Boolean, default: false },
  },
  emits: ["select"],
  setup(props, { slots, emit, attrs }) {
    function onSelect(ev) {
      if (props.disabled) {
        ev?.preventDefault?.();
        return;
      }
      emit("select", ev);
    }
    return () => {
      const children = [];
      if (slots.leading) {
        children.push(h("div", { "data-slot": "leading" }, slots.leading()));
      }
      if (slots.default || props.label) {
        children.push(
          h("div", { "data-slot": "content" }, slots.default?.() || props.label),
        );
      }
      if (slots.trailing) {
        children.push(h("div", { "data-slot": "trailing" }, slots.trailing()));
      }
      return h(
        "nana-list-item",
        {
          ...attrs,
          class: ["nana-list-item", attrs.class].flat().filter(Boolean).join(" "),
          label: props.label,
          selected: props.selected,
          active: props.selected,
          disabled: props.disabled,
          size: props.size,
          "auto-height": props.autoHeight,
          "data-agent-id": attrs["data-agent-id"] || "nana.list-item",
          onSelect,
          onClick: onSelect,
        },
        children,
      );
    };
  },
};

export default NanaListItem;
