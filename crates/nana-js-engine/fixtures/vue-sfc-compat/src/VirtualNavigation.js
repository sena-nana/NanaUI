import { h, ref } from "@vue/runtime-core";
import { createApp } from "../../../../../packages/nanavue-runtime/src/createNanaRenderer.js";
import { NanaVirtualList } from "../../../../../packages/nanavue-components/src/NanaVirtualList.js";

createApp({
  setup() {
    const list = ref(null);
    const selected = ref(-1);
    const retained = ref(["row-2"]);
    return () => h("div", {style: {width: "440px", height: "300px", display: "flex", flexDirection: "column"}}, [
      h("button", {"data-agent-id": "jump", onClick: () => list.value.scrollToIndex(500000, "start")}, selected.value < 0 ? "Jump to row 500000" : `Selected ${selected.value}`),
      h("button", {"data-agent-id": "release", onClick: () => {retained.value = [];}}, "Release retained row"),
      h(NanaVirtualList, {
        ref: list, count: 1000000, itemExtent: 32, overscan: 0,
        keyAt: index => `row-${index}`, indexOfKey: key => Number(key.slice(4)),
        retainedKeys: retained.value,
        style: {height: "160px", width: "400px", flexShrink: 0},
      }, {default: ({index, key}) => h("button", {"data-agent-id": key, onClick: () => {selected.value = index;}, style: {height: "32px", width: "180px"}}, `Row ${index}`)}),
    ]);
  },
}).mount();
