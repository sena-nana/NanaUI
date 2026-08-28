/**
 * NanaSpinner — semantic peer of Runtime `Spinner` (`nana-spinner`).
 */
import { h } from "@vue/runtime-core";

export const NanaSpinner = {
  name: "NanaSpinner",
  props: {
    label: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const label =
        props.label ||
        (typeof slots.default === "function"
          ? slots
              .default()
              .map((vnode) => (typeof vnode.children === "string" ? vnode.children : ""))
              .join("")
          : "");
      return h("nana-spinner", {
        ...attrs,
        class: ["nana-spinner", attrs.class].flat().filter(Boolean).join(" "),
        label,
        "data-agent-id": attrs["data-agent-id"] || "nana.spinner",
      });
    };
  },
};

export default NanaSpinner;
