/**
 * NanaSearch — Runtime SearchDropdown field.
 * Host tag `search-dropdown` (not HTML `<search>`) keeps query IME on the same retained TextInput state.
 */
import { h } from "@vue/runtime-core";

function normalizeOptions(options) {
  return (options || []).map((option) => ({
    value: option.value ?? option.key ?? option.label,
    label: option.label ?? String(option.value ?? option.key ?? ""),
    disabled: !!option.disabled,
  }));
}

export const NanaSearch = {
  name: "NanaSearch",
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
      h("search-dropdown", {
        ...attrs,
        class: ["nana-search", attrs.class].filter(Boolean).join(" "),
        value: props.modelValue,
        "model-value": props.modelValue,
        options: normalizeOptions(props.options),
        placeholder: props.placeholder,
        disabled: props.disabled,
        loading: props.loading,
        invalid: props.invalid,
        size: props.size,
        "data-agent-id": attrs["data-agent-id"] || "nana.search-dropdown",
        onSelect,
        onChange: onSelect,
      });
  },
};

export default NanaSearch;
