/**
 * NanaIconButton — semantic peer of Runtime `IconButton` (`nana-icon-button`).
 */
import { h } from "@vue/runtime-core";

export const NanaIconButton = {
  name: "NanaIconButton",
  props: {
    icon: { type: String, default: "" },
    label: { type: String, default: "" },
    kind: { type: String, default: "ghost" },
    size: { type: String, default: "medium" },
    selected: { type: Boolean, default: false },
    disabled: { type: Boolean, default: false },
    tooltip: { type: String, default: "" },
    hint: { type: String, default: "" },
  },
  emits: ["press"],
  setup(props, { emit, attrs }) {
    function onPress(ev) {
      if (props.disabled) {
        ev?.preventDefault?.();
        return;
      }
      emit("press", ev);
    }
    return () =>
      h("nana-icon-button", {
        ...attrs,
        class: ["nana-icon-button", attrs.class].flat().filter(Boolean).join(" "),
        icon: props.icon,
        "icon-name": props.icon,
        label: props.label || props.icon,
        kind: props.kind,
        size: props.size,
        selected: props.selected,
        active: props.selected,
        disabled: props.disabled,
        hint: props.tooltip || props.hint,
        tooltip: props.tooltip || props.hint,
        "data-agent-id": attrs["data-agent-id"] || "nana.icon-button",
        onPress,
        onClick: onPress,
      });
  },
};

export default NanaIconButton;
