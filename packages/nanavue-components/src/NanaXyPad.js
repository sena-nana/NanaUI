/**
 * NanaXyPad — two-axis pad.
 * Semantic peer of Runtime `XYPad` (`nana-xy-pad`).
 *
 * `x`/`y` (or `modelValue.x` / `modelValue.y`) plus `min`/`max` or per-axis
 * ranges. Invalid / disabled / loading map to Runtime field state.
 */
import { h } from "@vue/runtime-core";

function axisValue(value, fallback) {
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? n : fallback;
}

export const NanaXyPad = {
  name: "NanaXyPad",
  props: {
    modelValue: { type: Object, default: undefined },
    x: { type: Number, default: undefined },
    y: { type: Number, default: undefined },
    min: { type: Number, default: 0 },
    max: { type: Number, default: 1 },
    xMin: { type: Number, default: undefined },
    xMax: { type: Number, default: undefined },
    yMin: { type: Number, default: undefined },
    yMax: { type: Number, default: undefined },
    step: { type: Number, default: 0 },
    size: { type: String, default: "medium" },
    label: { type: String, default: "" },
    disabled: { type: Boolean, default: false },
    loading: { type: Boolean, default: false },
    invalid: { type: Boolean, default: false },
  },
  emits: ["update:modelValue", "input", "change"],
  setup(props, { emit, attrs }) {
    return () => {
      const x = axisValue(props.x, axisValue(props.modelValue?.x, 0));
      const y = axisValue(props.y, axisValue(props.modelValue?.y, 0));
      return h("nana-xy-pad", {
        ...attrs,
        class: ["nana-xy-pad", attrs.class].filter(Boolean).join(" "),
        role: attrs.role || "slider",
        label: props.label,
        x,
        y,
        number: x,
        min: props.min,
        max: props.max,
        "x-min": props.xMin,
        "x-max": props.xMax,
        "y-min": props.yMin,
        "y-max": props.yMax,
        step: props.step,
        size: props.size,
        disabled: props.disabled,
        loading: props.loading,
        invalid: props.invalid,
        "data-agent-id": attrs["data-agent-id"] || "nana.xy-pad",
        onInput: (ev) => {
          const next = ev?.value ?? ev;
          emit("input", next, ev);
          emit("update:modelValue", next);
        },
        onChange: (ev) => {
          const next = ev?.value ?? ev;
          emit("change", next, ev);
          emit("update:modelValue", next);
        },
      });
    };
  },
};

export default NanaXyPad;
