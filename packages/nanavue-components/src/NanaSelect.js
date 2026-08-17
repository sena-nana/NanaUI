/**
 * NanaSelect — single-value pick list.
 * Semantic peer of Rust `Select` (`nana-select` → Runtime / Scene host).
 */
import { h } from "@vue/runtime-core";

function normalizeOptions(options) {
  return (options || []).map((option) => ({
    value: option.value ?? option.key ?? option.label,
    label: option.label ?? String(option.value ?? option.key ?? ""),
    disabled: !!option.disabled,
  }));
}

export const NanaSelect = {
  name: "NanaSelect",
  props: {
    modelValue: { type: [String, Number], default: "" },
    options: { type: Array, default: () => [] },
    placeholder: { type: String, default: "" },
    disabled: { type: Boolean, default: false },
    loading: { type: Boolean, default: false },
    invalid: { type: Boolean, default: false },
    size: { type: String, default: "medium" },
  },
  emits: ["update:modelValue", "select", "change"],
  setup(props, { emit, attrs }) {
    function onSelect(ev) {
      const value = ev?.value ?? ev;
      emit("update:modelValue", value);
      emit("select", value, ev);
      emit("change", value, ev);
    }

    return () =>
      h("nana-select", {
        ...attrs,
        class: ["nana-select", attrs.class].filter(Boolean).join(" "),
        value: props.modelValue,
        "model-value": props.modelValue,
        options: normalizeOptions(props.options),
        placeholder: props.placeholder,
        disabled: props.disabled,
        loading: props.loading,
        invalid: props.invalid,
        size: props.size,
        "data-agent-id": attrs["data-agent-id"] || "nana.select",
        onSelect,
        onChange: onSelect,
      });
  },
};

export default NanaSelect;
