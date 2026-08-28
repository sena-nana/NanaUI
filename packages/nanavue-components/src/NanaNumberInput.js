/**
 * NanaNumberInput — semantic peer of Runtime `NumberInput` (`nana-number-input`).
 * HTML `<input type="number">` stays a text field; use this wrapper for the stepper.
 */
import { h } from "@vue/runtime-core";

export const NanaNumberInput = {
  name: "NanaNumberInput",
  props: {
    modelValue: { type: Number, default: 0 },
    min: { type: Number, default: 0 },
    max: { type: Number, default: 100 },
    step: { type: Number, default: 1 },
    precision: { type: Number, default: undefined },
    label: { type: String, default: "" },
    placeholder: { type: String, default: "" },
    size: { type: String, default: "medium" },
    disabled: { type: Boolean, default: false },
    readOnly: { type: Boolean, default: false },
    invalid: { type: Boolean, default: false },
  },
  emits: ["update:modelValue", "change", "input"],
  setup(props, { emit, attrs }) {
    function emitValue(ev) {
      const value = typeof ev?.value === "number" ? ev.value : Number(ev?.value ?? ev);
      if (!Number.isFinite(value)) return;
      emit("update:modelValue", value);
      emit("change", value, ev);
      emit("input", value, ev);
    }
    return () =>
      h("nana-number-input", {
        ...attrs,
        class: ["nana-number-input", attrs.class].flat().filter(Boolean).join(" "),
        label: props.label,
        value: props.modelValue,
        min: props.min,
        max: props.max,
        step: props.step,
        precision: props.precision,
        placeholder: props.placeholder,
        size: props.size,
        disabled: props.disabled,
        "read-only": props.readOnly,
        invalid: props.invalid,
        "data-agent-id": attrs["data-agent-id"] || "nana.number-input",
        onChange: emitValue,
        onInput: emitValue,
      });
  },
};

export default NanaNumberInput;
