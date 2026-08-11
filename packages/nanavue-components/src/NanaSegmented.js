/**
 * NanaSegmented — compact mutually-exclusive control.
 * Semantic peer of Rust `SegmentedControl` (`nana-segmented` → Iced).
 */
import { h } from "@vue/runtime-core";

export const NanaSegmented = {
  name: "NanaSegmented",
  props: {
    modelValue: { type: [String, Number], default: "" },
    options: { type: Array, default: () => [] },
  },
  emits: ["update:modelValue", "select"],
  setup(props, { emit, attrs }) {
    return () =>
      h("nana-segmented", {
        ...attrs,
        class: ["nana-segmented", attrs.class].filter(Boolean).join(" "),
        role: "group",
        value: props.modelValue,
        "model-value": props.modelValue,
        options: props.options.map((option) => ({
          value: option.value ?? option.key ?? option.label,
          label: option.label ?? String(option.value ?? option.key ?? ""),
          disabled: !!option.disabled,
        })),
        "data-agent-id": attrs["data-agent-id"] || "nana.segmented",
        onSelect: (ev) => {
          const value = ev?.value ?? ev;
          emit("update:modelValue", value);
          emit("select", value, ev);
        },
        onChange: (ev) => {
          const value = ev?.value ?? ev;
          emit("update:modelValue", value);
          emit("select", value, ev);
        },
      });
  },
};

export default NanaSegmented;
