/**
 * NanaDivider — semantic peer of Runtime `Divider` (`nana-divider`).
 */
import { h } from "@vue/runtime-core";

export const NanaDivider = {
  name: "NanaDivider",
  props: {
    orientation: { type: String, default: "horizontal" },
    thickness: { type: [Number, String], default: undefined },
    inset: { type: [Number, String], default: undefined },
  },
  setup(props, { attrs }) {
    return () =>
      h("nana-divider", {
        ...attrs,
        class: ["nana-divider", attrs.class].flat().filter(Boolean).join(" "),
        orientation: props.orientation,
        thickness: props.thickness,
        inset: props.inset,
        "data-agent-id": attrs["data-agent-id"] || "nana.divider",
      });
  },
};

export default NanaDivider;
