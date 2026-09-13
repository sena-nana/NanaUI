/**
 * NanaVirtualTable — two-axis visible window over Runtime `ScrollView`.
 * Geometry matches `VirtualTableLayout`.
 */
import { cloneVNode, computed, h, inject, provide } from "@vue/runtime-core";
import { createWindowIndex, frozenWindowGeometryEqual } from "./virtual-window.js";
import { useScrollWindow, useStableVirtualWindow, retainedWindowChildren, retainedKeyIndices, useVirtualActivity } from "./NanaVirtualList.js";

const SCROLL_X = "nanaVirtualScrollX";
const SCROLL_Y = "nanaVirtualScrollY";

const VirtualTableRowPinned = {
  name: "NanaVirtualTableRowPinned",
  props: {
    width: { type: Number, required: true },
    height: { type: Number, required: true },
    content: { type: Object, required: true },
  },
  setup(props) {
    const y = inject(SCROLL_Y, { value: 0 });
    return () => cloneVNode(props.content, {
      style: {
        ...props.content.props?.style,
        width: `${props.width}px`,
        height: `${props.height}px`,
        transform: `translateY(${y.value}px)`,
      },
    });
  },
};

const VirtualTableCellPinned = {
  name: "NanaVirtualTableCellPinned",
  props: {
    width: { type: Number, required: true },
    height: { type: Number, required: true },
    content: { type: Object, required: true },
  },
  setup(props) {
    const x = inject(SCROLL_X, { value: 0 });
    return () => cloneVNode(props.content, {
      style: {
        ...props.content.props?.style,
        width: `${props.width}px`,
        height: `${props.height}px`,
        transform: `translateX(${x.value}px)`,
      },
    });
  },
};

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

    provide(SCROLL_X, x);
    provide(SCROLL_Y, y);
    const activity = useVirtualActivity(
      () => ({ count: rowSizes.value.length, keyAt: props.rowKeyAt, indexOfKey: props.rowIndexOfKey }),
      () => ({ count: columnSizes.value.length, keyAt: props.columnKeyAt, indexOfKey: props.columnIndexOfKey }),
    );
    const rowPane = useStableVirtualWindow(
      () => rowSizes.value.frozenWindow(y.value, height.value, props.overscan, props.frozenRows),
      frozenWindowGeometryEqual,
    );
    const columnPane = useStableVirtualWindow(
      () => columnSizes.value.frozenWindow(x.value, width.value, props.overscan, props.frozenColumns),
      frozenWindowGeometryEqual,
    );
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
        retainedWindowChildren(rowSizes.value, rowPane.value.body, retainedRows(), "nana-virtual-table", "y", (row) => {
          const rowKey = props.rowKeyAt ? props.rowKeyAt(row) : row;
          const frozenRow = row < rowPane.value.count;
          const rowHeight = rowSizes.value.prefixAt(row + 1) - rowSizes.value.prefixAt(row);
          const cells = retainedWindowChildren(columnSizes.value, columnPane.value.body, retainedColumns(), "nana-virtual-table", "x", (column) => {
            const columnKey = props.columnKeyAt ? props.columnKeyAt(column) : column;
            const frozenColumn = column < columnPane.value.count;
            const cellWidth = columnSizes.value.prefixAt(column + 1) - columnSizes.value.prefixAt(column);
            const content = h(
              "div",
              {
                key: columnKey,
                class: "nana-virtual-table__cell",
                style: {
                  width: `${cellWidth}px`,
                  height: `${rowHeight}px`,
                  flexShrink: 0,
                  position: "relative",
                  zIndex: frozenColumn ? 1 : 0,
                  background: frozenRow || frozenColumn ? "var(--bg-elev, #f3f4f6)" : undefined,
                },
                ...activity.handlers(row, column),
              },
              () => slots.default?.({ row, column, rowKey, columnKey, frozenRow, frozenColumn }) || [],
            );
            return frozenColumn ? h(
              VirtualTableCellPinned,
              { key: columnKey, width: cellWidth, height: rowHeight, content },
            ) : content;
          });
          const content = h(
            "div",
            {
              key: rowKey,
              class: "nana-virtual-table__row",
              style: {
                display: "flex",
                flexDirection: "row",
                flexShrink: 0,
                width: `${columnPane.value.body.total}px`,
                height: `${rowHeight}px`,
                position: "relative",
                zIndex: frozenRow ? 2 : 0,
              },
            },
            cells,
          );
          return frozenRow ? h(VirtualTableRowPinned, {
            key: rowKey,
            width: columnPane.value.body.total,
            height: rowHeight,
            content,
          }) : content;
        }),
      );
  },
};

export default NanaVirtualTable;
