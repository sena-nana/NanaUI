/**
 * NanaTable — semantic peer of Runtime `Table` (`nana-table`).
 * HTML `<table>` stays a layout box; use this host tag for the Runtime table.
 */
import { h } from "@vue/runtime-core";

export const NanaTable = {
  name: "NanaTable",
  props: {
    label: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    return () =>
      h(
        "nana-table",
        {
          ...attrs,
          class: ["nana-table", attrs.class].flat().filter(Boolean).join(" "),
          label: props.label,
          "data-agent-id": attrs["data-agent-id"] || "nana.table",
        },
        slots.default?.(),
      );
  },
};

export default NanaTable;
