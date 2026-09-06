/**
 * NanaVirtualTable — two-axis visible window over Runtime `ScrollView`.
 * Geometry matches `VirtualTableLayout`.
 */
import { computed, h } from "@vue/runtime-core";
import { createWindowIndex } from "./virtual-window.js";
import { useScrollWindow, retainedWindowChildren, retainedKeyIndices, useVirtualActivity } from "./NanaVirtualList.js";

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
    frozenRows: { type: Number, default: 0 },
    frozenColumns: { type: Number, default: 0 },
    retainedRowKeys: { type: Array, default: () => [] },
    retainedColumnKeys: { type: Array, default: () => [] },
    rowIndexOfKey: { type: Function, default: undefined },
    columnIndexOfKey: { type: Function, default: undefined },
    rowKeyAt: { type: Function, default: undefined },
    columnKeyAt: { type: Function, default: undefined },
  },
  setup(props, { slots, attrs, expose }) {
    const { x, y, width, height, bindHost, onScroll, scrollTo } = useScrollWindow();
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

    const activity = useVirtualActivity(
      () => ({ count: rowSizes.value.length, keyAt: props.rowKeyAt, indexOfKey: props.rowIndexOfKey }),
      () => ({ count: columnSizes.value.length, keyAt: props.columnKeyAt, indexOfKey: props.columnIndexOfKey }),
    );
    const rowPane = computed(() => rowSizes.value.frozenWindow(y.value, height.value, props.overscan, props.frozenRows));
    const rows = computed(() => rowPane.value.body);
    const columnPane = computed(() => columnSizes.value.frozenWindow(x.value, width.value, props.overscan, props.frozenColumns));
    const columns = computed(() => columnPane.value.body);
    const retainedRows = () => [...rowPane.value.frozen, ...activity.indices(0), ...retainedKeyIndices({
      retainedKeys: props.retainedRowKeys, keyAt: props.rowKeyAt, indexOfKey: props.rowIndexOfKey,
    }, rowSizes.value.length)];
    const retainedColumns = () => [...columnPane.value.frozen, ...activity.indices(1), ...retainedKeyIndices({
      retainedKeys: props.retainedColumnKeys, keyAt: props.columnKeyAt, indexOfKey: props.columnIndexOfKey,
    }, columnSizes.value.length)];

    expose({
      async scrollToCell(row, column, alignment = "nearest") {
        const nextY = rowSizes.value.offsetForFrozenIndex(row, y.value, height.value, props.frozenRows, alignment);
        const nextX = columnSizes.value.offsetForFrozenIndex(column, x.value, width.value, props.frozenColumns, alignment);
        return nextX !== null && nextY !== null && await scrollTo(nextX, nextY);
      },
    });

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
        retainedWindowChildren(rowSizes.value, rows.value, retainedRows(), "nana-virtual-table", "y", (row) => {
          const rowKey = props.rowKeyAt ? props.rowKeyAt(row) : row;
          return h(
            "div",
            { key: rowKey, class: "nana-virtual-table__row", style: {
              display: "flex", flexDirection: "row", flexShrink: 0,
              width: `${columns.value.total}px`,
              height: `${rowSizes.value.prefixAt(row + 1) - rowSizes.value.prefixAt(row)}px`,
              position: "relative", zIndex: row < rowPane.value.count ? 2 : 0,
              transform: row < rowPane.value.count ? `translateY(${y.value}px)` : undefined,
            } },
            retainedWindowChildren(columnSizes.value, columns.value, retainedColumns(), "nana-virtual-table", "x", (column) => {
              const columnKey = props.columnKeyAt ? props.columnKeyAt(column) : column;
              return h(
                "div",
                { key: columnKey, ...activity.handlers(row, column), class: "nana-virtual-table__cell", style: {
                  width: `${columnSizes.value.prefixAt(column + 1) - columnSizes.value.prefixAt(column)}px`,
                  height: `${rowSizes.value.prefixAt(row + 1) - rowSizes.value.prefixAt(row)}px`, flexShrink: 0,
                  position: "relative", zIndex: column < columnPane.value.count ? 1 : 0,
                  transform: column < columnPane.value.count ? `translateX(${x.value}px)` : undefined,
                  background: row < rowPane.value.count || column < columnPane.value.count ? "var(--bg-elev, #f3f4f6)" : undefined,
                } },
                slots.default?.({ row, column, rowKey, columnKey, frozenRow: row < rowPane.value.count, frozenColumn: column < columnPane.value.count }) || [],
              );
            }),
          );
        }),
      );
  },
};

export default NanaVirtualTable;
