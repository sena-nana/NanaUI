/**
 * NanaDropdown — Lilia `@lilia/ui/search` Dropdown stand-in for Nana host.
 * Maps to iced `Select` (`nana-select`) — not CSS fixed Teleport menus.
 */
import { h } from "@vue/runtime-core";
import { NanaSelect } from "./NanaSelect.js";

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
    /** Accepted for API parity; iced Select owns placement. */
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
    return () => {
      const size =
        props.size === "large" ? "large" : props.size === "medium" ? "medium" : "small";
      const value = Array.isArray(props.modelValue)
        ? props.modelValue[0] ?? ""
        : props.modelValue;
      return h(NanaSelect, {
        ...attrs,
        class: ["nana-dropdown", props.buttonClass, attrs.class]
          .filter(Boolean)
          .join(" "),
        modelValue: value,
        options: props.options,
        placeholder: props.displayLabel || props.placeholder || props.menuLabel || "",
        disabled: props.disabled,
        loading: props.loading,
        invalid: props.invalid,
        size,
        "data-agent-id": props.agentId || attrs["data-agent-id"] || "nana.dropdown",
        "onUpdate:modelValue": (v) => emit("update:modelValue", v),
        onSelect: (v, ev) => {
          emit("select", v, ev);
          emit("change", v, ev);
        },
        onChange: (v, ev) => emit("change", v, ev),
      });
    };
  },
};

export default NanaDropdown;
