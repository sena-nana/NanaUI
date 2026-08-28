/**
 * NanaRangeField — semantic peer of Rust `RangeField`.
 * Host node `nana-range-field` draws as Runtime RangeField (not DOM track paint).
 */
import { h } from "@vue/runtime-core";

export const NanaRangeField = {
  name: "NanaRangeField",
  props: {
    modelValue: { type: Number, default: 0 },
    min: { type: Number, default: 0 },
    max: { type: Number, default: 100 },
    step: { type: Number, default: 1 },
    unit: { type: String, default: "" },
    label: { type: String, default: "" },
    disabled: { type: Boolean, default: false },
    agentId: { type: String, default: "" },
  },
  emits: ["update:modelValue", "change"],
  setup(props, { emit, attrs }) {
    return () => {
      const agentId =
        props.agentId || attrs["data-agent-id"] || attrs["agent-id"] || "nana.range";
      return h("nana-range-field", {
        ...attrs,
        class: ["nana-range-field", props.disabled ? "is-disabled" : "", attrs.class]
          .filter(Boolean)
          .join(" "),
        role: "slider",
        label: props.label,
        value: props.modelValue,
        "model-value": props.modelValue,
        min: props.min,
        max: props.max,
        step: props.step,
        unit: props.unit,
        disabled: props.disabled,
        "aria-valuemin": props.min,
        "aria-valuemax": props.max,
        "aria-valuenow": props.modelValue,
        "data-agent-id": agentId,
        onChange: (ev) => {
          const value = typeof ev?.value === "number" ? ev.value : Number(ev);
          if (Number.isFinite(value)) {
            emit("update:modelValue", value);
            emit("change", value, ev);
          }
        },
      });
    };
  },
};

export default NanaRangeField;
