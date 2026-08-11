/**
 * NanaSwitch — semantic peer of Rust `Switch` / Lilia `UiSwitch` (no DOM paint).
 */
import { h } from "@vue/runtime-core";

export const NanaSwitch = {
  name: "NanaSwitch",
  props: {
    modelValue: { type: Boolean, default: false },
    label: { type: String, default: "" },
    hint: { type: String, default: "" },
    disabled: { type: Boolean, default: false },
    invalid: { type: Boolean, default: false },
    controlPosition: { type: String, default: "end" },
    agentId: { type: String, default: "" },
  },
  emits: ["update:modelValue", "change"],
  setup(props, { slots, emit, attrs }) {
    function onChange(ev) {
      if (props.disabled) {
        ev?.preventDefault?.();
        return;
      }
      const next =
        typeof ev?.value === "boolean"
          ? ev.value
          : typeof ev?.checked === "boolean"
            ? ev.checked
            : !props.modelValue;
      emit("update:modelValue", next);
      emit("change", next, ev);
    }

    return () => {
      const agentId =
        props.agentId || attrs["data-agent-id"] || attrs["agent-id"] || "nana.switch";
      const label =
        props.label ||
        (typeof slots.default === "function"
          ? slots
              .default()
              .map((vnode) => (typeof vnode.children === "string" ? vnode.children : ""))
              .join("")
          : "");
      return h("nana-switch", {
        ...attrs,
        label,
        hint: props.hint,
        disabled: props.disabled,
        invalid: props.invalid,
        toggled: props.modelValue,
        "model-value": props.modelValue,
        "control-position": props.controlPosition,
        "data-agent-id": agentId,
        onChange,
        onClick: onChange,
      });
    };
  },
};

export default NanaSwitch;
