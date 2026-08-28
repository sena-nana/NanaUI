/**
 * NanaLabeledValue — semantic peer of Runtime `LabeledValue` (`nana-labeled-value`).
 */
import { h } from "@vue/runtime-core";

export const NanaLabeledValue = {
  name: "NanaLabeledValue",
  props: {
    label: { type: String, default: "" },
    value: { type: [String, Number], default: "" },
    muted: { type: Boolean, default: false },
    compact: { type: Boolean, default: false },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const children = [];
      if (slots.action) {
        children.push(h("div", { "data-slot": "action" }, slots.action()));
      }
      return h(
        "nana-labeled-value",
        {
          ...attrs,
          class: ["nana-labeled-value", attrs.class].flat().filter(Boolean).join(" "),
          label: props.label,
          value: props.value,
          muted: props.muted,
          compact: props.compact,
          "data-agent-id": attrs["data-agent-id"] || "nana.labeled-value",
        },
        children,
      );
    };
  },
};

export default NanaLabeledValue;
