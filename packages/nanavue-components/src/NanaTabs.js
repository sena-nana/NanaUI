/**
 * NanaTabs — horizontal tab list.
 * Semantic peer of Rust `Tabs` (`nana-tabs` host node → Runtime / Scene host).
 */
import { h } from "@vue/runtime-core";

export const NanaTabs = {
  name: "NanaTabs",
  props: {
    modelValue: { type: [String, Number], default: "" },
    options: { type: Array, default: () => [] },
    fill: { type: Boolean, default: false },
  },
  emits: ["update:modelValue", "select"],
  setup(props, { emit, attrs }) {
    return () =>
      h("nana-tabs", {
        ...attrs,
        class: ["nana-tabs", props.fill ? "nana-tabs--fill" : "", attrs.class]
          .filter(Boolean)
          .join(" "),
        role: "tablist",
        fill: props.fill,
        value: props.modelValue,
        "model-value": props.modelValue,
        options: props.options.map((option) => ({
          value: option.value ?? option.key ?? option.label,
          label: option.label ?? String(option.value ?? option.key ?? ""),
          disabled: !!option.disabled,
        })),
        "data-agent-id": attrs["data-agent-id"] || "nana.tabs",
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

export default NanaTabs;
