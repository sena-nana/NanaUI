/**
 * NanaInput — single-line text field.
 * Semantic peer of Rust `Input` (`nana-input` → Runtime / Scene host).
 */
import { h } from "@vue/runtime-core";

export const NanaInput = {
  name: "NanaInput",
  props: {
    modelValue: { type: String, default: "" },
    placeholder: { type: String, default: "" },
    disabled: { type: Boolean, default: false },
    invalid: { type: Boolean, default: false },
    size: { type: String, default: "medium" },
  },
  emits: ["update:modelValue", "input"],
  setup(props, { emit, attrs }) {
    return () =>
      h("nana-input", {
        ...attrs,
        value: props.modelValue,
        "model-value": props.modelValue,
        placeholder: props.placeholder,
        disabled: props.disabled,
        invalid: props.invalid,
        size: props.size,
        "data-agent-id": attrs["data-agent-id"] || "nana.input",
        onInput: (ev) => {
          const value = ev?.value ?? ev ?? "";
          emit("update:modelValue", String(value));
          emit("input", String(value), ev);
        },
      });
  },
};

export default NanaInput;
