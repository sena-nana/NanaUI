/**
 * NanaDropdown — single or multiple Runtime Dropdown field.
 * Host tag `nana-dropdown` projects Runtime `Dropdown`, not Select.
 */
import { h } from "@vue/runtime-core";

function normalizeOptions(options) {
  return (options || []).map((option) => ({
    value: option.value ?? option.key ?? option.label,
    label: option.label ?? String(option.value ?? option.key ?? ""),
    disabled: !!option.disabled,
  }));
}

export const NanaDropdown = {
  name: "NanaDropdown",
  props: {
    modelValue: { type: [String, Number, Array], default: "" },
    options: { type: Array, default: () => [] },
    placeholder: { type: String, default: "" },
    displayLabel: { type: String, default: "" },
    disabled: { type: Boolean, default: false },
    loading: { type: Boolean, default: false },
    invalid: { type: Boolean, default: false },
    size: { type: String, default: "small" },
    placement: { type: String, default: "bottom" },
    multiple: { type: Boolean, default: false },
    block: { type: Boolean, default: false },
    buttonClass: { type: String, default: "" },
    hideButtonLabel: { type: Boolean, default: false },
    agentId: { type: String, default: "" },
    menuWidth: { type: String, default: "" },
    menuLabel: { type: String, default: "" },
    icon: { default: undefined },
  },
  emits: ["update:modelValue", "select", "change"],
  setup(props, { emit, attrs }) {
    function onSelect(ev) {
      const value = ev?.value ?? ev;
      emit("update:modelValue", value);
      emit("select", value, ev);
      emit("change", value, ev);
    }

    return () => {
      const size =
        props.size === "large" ? "large" : props.size === "medium" ? "medium" : "small";
      const value = Array.isArray(props.modelValue)
        ? props.modelValue.join(",")
        : props.modelValue;
      return h("nana-dropdown", {
        ...attrs,
        class: ["nana-dropdown", props.buttonClass, attrs.class].filter(Boolean).join(" "),
        value,
        "model-value": value,
        options: normalizeOptions(props.options),
        placeholder: props.displayLabel || props.placeholder || props.menuLabel || "",
        disabled: props.disabled,
        loading: props.loading,
        invalid: props.invalid,
        size,
        multiple: props.multiple ? "" : undefined,
        "data-agent-id": props.agentId || attrs["data-agent-id"] || "nana.dropdown",
        onSelect,
        onChange: onSelect,
      });
    };
  },
};

export default NanaDropdown;
