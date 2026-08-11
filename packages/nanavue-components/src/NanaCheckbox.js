/**
 * NanaCheckbox — semantic peer of Rust `Checkbox`.
 */
import { h } from "@vue/runtime-core";

export const NanaCheckbox = {
  name: "NanaCheckbox",
  props: {
    modelValue: { type: Boolean, default: false },
    label: { type: String, default: "" },
    disabled: { type: Boolean, default: false },
    invalid: { type: Boolean, default: false },
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
      const label =
        props.label ||
        (typeof slots.default === "function"
          ? slots
              .default()
              .map((vnode) => (typeof vnode.children === "string" ? vnode.children : ""))
              .join("")
          : "");
      return h("nana-checkbox", {
        ...attrs,
        label,
        disabled: props.disabled,
        invalid: props.invalid,
        toggled: props.modelValue,
        checked: props.modelValue,
        "model-value": props.modelValue,
        "data-agent-id": attrs["data-agent-id"] || "nana.checkbox",
        onChange,
        onClick: onChange,
      });
    };
  },
};

export default NanaCheckbox;
