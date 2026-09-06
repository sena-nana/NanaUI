import { h, ref } from "@vue/runtime-core";
import { createApp } from "../../../../../packages/nanavue-runtime/src/createNanaRenderer.js";
import { NanaVirtualList } from "../../../../../packages/nanavue-components/src/NanaVirtualList.js";

createApp({
  setup() {
    const list = ref(null);
    let editor = null;
    return () => h("div", {style: {width: "440px", height: "300px", display: "flex", flexDirection: "column"}}, [
      h("button", {"data-agent-id": "jump", onClick: async () => {
        editor.focus();
        await list.value.scrollToIndex(500000, "start");
      }}, "Continue editing while scrolling"),
      h("button", {"data-agent-id": "release", onClick: () => editor?.blur()}, "Finish editing"),
      h(NanaVirtualList, {
        ref: list, count: 1000000, itemExtent: 32, overscan: 0,
        keyAt: index => `row-${index}`, indexOfKey: key => Number(key.slice(4)),
        style: {height: "160px", width: "400px", flexShrink: 0},
      }, {default: ({index, key}) => h("input", {
        ref: index === 2 ? el => { if (el) editor = el; } : undefined,
        "data-agent-id": key, value: `Draft ${index}`,
        style: {height: "32px", width: "240px"},
      })}),
    ]);
  },
}).mount();
