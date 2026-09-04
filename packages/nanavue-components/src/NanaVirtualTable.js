/**
 * NanaVirtualTable — two-axis visible window over Runtime `ScrollView`.
 * Geometry matches `VirtualTableLayout`.
 */
import { computed, h } from "@vue/runtime-core";
import { createWindowIndex } from "./virtual-window.js";
import { useScrollWindow, windowChildren } from "./NanaVirtualList.js";

export const NanaVirtualTable = {
  name: "NanaVirtualTable",
  props: {
    rowCount: { type: Number, default: 0 },
    columnCount: { type: Number, default: 0 },
    rowExtent: { type: Number, default: 32 },
    columnExtent: { type: Number, default: 96 },
    rowExtents: { type: Array, default: undefined },
    columnExtents: { type: Array, default: undefined },
    overscan: { type: Number, default: 64 },
    scrollbars: { type: String, default: "auto" },
    rowKeyAt: { type: Function, default: undefined },
    columnKeyAt: { type: Function, default: undefined },
  },
  setup(props, { slots, attrs }) {
    const { x, y, width, height, bindHost, onScroll } = useScrollWindow();
    const rowSizes = computed(() =>
      createWindowIndex({
        count: props.rowCount,
        itemExtent: props.rowExtent,
        extents: props.rowExtents,
      }),
    );
    const columnSizes = computed(() =>
      createWindowIndex({
        count: props.columnCount,
        itemExtent: props.columnExtent,
        extents: props.columnExtents,
      }),
    );

    const rows = computed(() => rowSizes.value.window(y.value, height.value, props.overscan));
    const columns = computed(() => columnSizes.value.window(x.value, width.value, props.overscan));

    return () =>
      h(
        "nana-scroll-view",
        {
          ...attrs,
          ref: bindHost,
          class: ["nana-virtual-table", attrs.class].flat().filter(Boolean).join(" "),
          axes: "both",
          scrollbars: props.scrollbars,
          "data-agent-id": attrs["data-agent-id"] || "nana.virtual-table",
          onScroll,
        },
        windowChildren(rows.value, "nana-virtual-table", "y", (row) => {
          const rowKey = props.rowKeyAt ? props.rowKeyAt(row) : row;
          return h(
            "div",
            { key: rowKey, class: "nana-virtual-table__row" },
            windowChildren(columns.value, "nana-virtual-table", "x", (column) => {
              const columnKey = props.columnKeyAt ? props.columnKeyAt(column) : column;
              return h(
                "div",
                { key: `${rowKey}:${columnKey}`, class: "nana-virtual-table__cell" },
                slots.default?.({ row, column, rowKey, columnKey }) || [],
              );
            }),
          );
        }),
      );
  },
};

export default NanaVirtualTable;
