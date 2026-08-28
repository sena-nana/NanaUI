/**
 * NanaList — semantic peer of Runtime `List` (`nana-list`).
 * Children are typically `NanaListItem`. HTML `ul`/`ol` stay layout boxes.
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
        "nana-list",
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
