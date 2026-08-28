/**
 * NanaFormField — semantic peer of Runtime `FormField` (`nana-form-field`).
 * Default slot is the control.
 */
import { h } from "@vue/runtime-core";

export const NanaFormField = {
  name: "NanaFormField",
  props: {
    label: { type: String, default: "" },
    hint: { type: String, default: "" },
    invalid: { type: Boolean, default: false },
    size: { type: String, default: "medium" },
  },
  setup(props, { slots, attrs }) {
    return () =>
      h(
        "nana-form-field",
        {
          ...attrs,
          class: ["nana-form-field", attrs.class].flat().filter(Boolean).join(" "),
          label: props.label,
          hint: props.hint,
          invalid: props.invalid,
          size: props.size,
          "data-agent-id": attrs["data-agent-id"] || "nana.form-field",
        },
        slots.default
          ? [h("div", { "data-slot": "control" }, slots.default())]
          : [],
      );
  },
};

export default NanaFormField;
