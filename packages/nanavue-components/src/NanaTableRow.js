/**
 * NanaTableRow — semantic peer of Runtime `TableRow` (`<tr>`).
 */
import { h } from "@vue/runtime-core";

export const NanaTableRow = {
  name: "NanaTableRow",
  props: {
    selected: { type: Boolean, default: false },
  },
  emits: ["select"],
  setup(props, { slots, emit, attrs }) {
    function onSelect(ev) {
      emit("select", ev);
    }
    return () =>
      h(
        "tr",
        {
          ...attrs,
          class: ["nana-table-row", attrs.class].flat().filter(Boolean).join(" "),
          selected: props.selected,
          active: props.selected,
          "data-agent-id": attrs["data-agent-id"] || "nana.table-row",
          onSelect,
          onClick: onSelect,
        },
        slots.default?.(),
      );
  },
};

export default NanaTableRow;
