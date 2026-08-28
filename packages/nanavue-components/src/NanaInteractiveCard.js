/**
 * NanaInteractiveCard — semantic peer of Runtime `InteractiveCard`.
 */
import { h } from "@vue/runtime-core";

export const NanaInteractiveCard = {
  name: "NanaInteractiveCard",
  props: {
    active: { type: Boolean, default: false },
    selected: { type: Boolean, default: false },
    disabled: { type: Boolean, default: false },
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
      const selected = props.selected || props.active;
      return h(
        "nana-interactive-card",
        {
          ...attrs,
          class: ["nana-interactive-card", attrs.class].flat().filter(Boolean).join(" "),
          active: selected,
          selected,
          disabled: props.disabled,
          "data-agent-id": attrs["data-agent-id"] || "nana.interactive-card",
          onSelect,
          onClick: onSelect,
        },
        slots.default?.(),
      );
    };
  },
};

export default NanaInteractiveCard;
