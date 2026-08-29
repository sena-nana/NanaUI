/**
 * NanaList — semantic peer of Runtime `List` (`ul` / `ol`).
 * Children are typically `NanaListItem` / `<li>`.
 */
import { h } from "@vue/runtime-core";

export const NanaList = {
  name: "NanaList",
  props: {
    label: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    return () =>
      h(
        "ul",
        {
          ...attrs,
          class: ["nana-list", attrs.class].flat().filter(Boolean).join(" "),
          label: props.label,
          "data-agent-id": attrs["data-agent-id"] || "nana.list",
        },
        slots.default?.(),
      );
  },
};

export default NanaList;
