import { h, ref } from "@vue/runtime-core";
import { createApp } from "../../../../../packages/nanavue-runtime/src/createNanaRenderer.js";
import { NanaVirtualTable } from "../../../../../packages/nanavue-components/src/NanaVirtualTable.js";
createApp({setup() {
  const table = ref(null), selection = ref("Jump"), retained = ref(["r-2"]);
  return () => h("div", {style: {width: "480px", height: "320px", display: "flex", flexDirection: "column"}}, [
    h("button", {"data-agent-id": "jump", onClick: () => table.value.scrollToCell(500000, 8000, "start")}, selection.value),
    h("button", {"data-agent-id": "release", onClick: () => {retained.value = [];}}, "Release"),
    h(NanaVirtualTable, {ref: table, rowCount: 1000000, columnCount: 10000,
      rowExtent: 32, columnExtent: 80, frozenRows: 1, frozenColumns: 1, overscan: 0,
      rowKeyAt: row => `r-${row}`, columnKeyAt: col => `c-${col}`,
      rowIndexOfKey: key => Number(key.slice(2)), columnIndexOfKey: key => Number(key.slice(2)),
      retainedRowKeys: retained.value, retainedColumnKeys: retained.value.length ? ["c-2"] : [],
      style: {height: "160px", width: "320px", flexShrink: 0},
    }, {default: ({row, column}) => h("button", {"data-agent-id": `cell-${row}-${column}`,
      onClick: () => {selection.value = `${row}/${column}`;}, style: {height: "32px", width: "80px"}}, `${row}/${column}`)}),
  ]);
}}).mount();
