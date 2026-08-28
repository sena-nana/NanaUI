/**
 * NanaTableCell — semantic peer of Runtime `TableCell` (`nana-table-cell`).
 * `header` marks a column header (`th` is not a separate Runtime type).
 */
import { h } from "@vue/runtime-core";

export const NanaTableCell = {
  name: "NanaTableCell",
  props: {
    label: { type: String, default: "" },
    header: { type: Boolean, default: false },
    selected: { type: Boolean, default: false },
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
      return h(
        "nana-table-cell",
        {
          ...attrs,
          class: ["nana-table-cell", attrs.class].flat().filter(Boolean).join(" "),
          label,
          header: props.header,
          "column-header": props.header,
          selected: props.selected,
          active: props.selected,
          "data-agent-id": attrs["data-agent-id"] || "nana.table-cell",
        },
        slots.default?.(),
      );
    };
  },
};

export default NanaTableCell;
