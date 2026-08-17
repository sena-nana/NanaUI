/**
 * NanaTooltip — compact label-only hover card.
 * Semantic peer of Runtime `Tooltip` (`nana-tooltip`).
 *
 * `label` (or `hint`) is the tooltip copy. Hover delay / placement stay on
 * the Runtime overlay host.
 */
import { h } from "@vue/runtime-core";

export const NanaTooltip = {
  name: "NanaTooltip",
  props: {
    label: { type: String, default: "" },
    hint: { type: String, default: "" },
  },
  setup(props, { attrs }) {
    return () => {
      const label = props.label || props.hint;
      return h("nana-tooltip", {
        ...attrs,
        class: ["nana-tooltip", attrs.class].filter(Boolean).join(" "),
        role: attrs.role || "tooltip",
        label,
        hint: props.hint || label,
        "data-agent-id": attrs["data-agent-id"] || "nana.tooltip",
      });
    };
  },
};

export default NanaTooltip;
