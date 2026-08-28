/**
 * NanaIcon — semantic peer of Runtime `IconGlyph` (`nana-icon`).
 */
import { h } from "@vue/runtime-core";

export const NanaIcon = {
  name: "NanaIcon",
  props: {
    icon: { type: String, default: "" },
    name: { type: String, default: "" },
    label: { type: String, default: "" },
    size: { type: [String, Number], default: undefined },
  },
  setup(props, { attrs }) {
    return () => {
      const icon = props.icon || props.name || props.label;
      return h("nana-icon", {
        ...attrs,
        class: ["nana-icon", attrs.class].flat().filter(Boolean).join(" "),
        icon,
        "icon-name": icon,
        label: props.label || icon,
        size: props.size,
        "data-agent-id": attrs["data-agent-id"] || "nana.icon",
      });
    };
  },
};

export default NanaIcon;
