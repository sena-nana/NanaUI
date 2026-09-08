// Hover-cost fixture: one flat column of rows, built three ways.
//
// Three modes rather than one number, because "Vue hover is slow" would hide
// which of three separable things is slow:
//
//   bare      no pointer handler anywhere. Whatever this costs is what the tier
//             spends dispatching events onto a path nobody listens on.
//   listeners a handler on every row that touches nothing reactive. Adds the
//             cost of actually calling into JS, without a re-render.
//   reactive  a handler that writes a ref the rows render. Adds Vue's patch of
//             the whole column, because one render function owns every row --
//             the shape a naive hover handler over a long list really has.
//
// Rows are fixed-height and the pointer never leaves the column, so nothing
// reflows: the scenario's `layout_nodes == 0` invariant has to hold on this
// tree, not merely on the Rust one.
import { h, ref } from "@vue/runtime-core";
import { createApp } from "../../../../../packages/nanavue-runtime/src/createNanaRenderer.js";

const ROWS = __HOVER_BENCH_ROWS__;
const MODE = __HOVER_BENCH_MODE__;
const ROW_HEIGHT = 24;

// Written by the `listeners` handler and never read by the render function, so
// the handler does real work without invalidating anything. Deliberately not a
// `ref`: a reactive write here would quietly turn this mode into `reactive`.
let touched = 0;
globalThis.__hoverBenchTouched = () => touched;

createApp({
  setup() {
    const active = ref(-1);
    return () =>
      h(
        "div",
        {
          "data-agent-id": "column",
          style: { display: "flex", flexDirection: "column", width: "320px" },
        },
        Array.from({ length: ROWS }, (_, index) => {
          const props = {
            key: index,
            "data-agent-id": `row-${index}`,
            style: {
              height: `${ROW_HEIGHT}px`,
              width: "320px",
              flexShrink: 0,
              color: MODE === "reactive" && active.value === index ? "#ffffff" : "#a0a0a0",
            },
          };
          if (MODE === "listeners") {
            props.onPointerenter = () => {
              touched += 1;
            };
          } else if (MODE === "reactive") {
            props.onPointerenter = () => {
              active.value = index;
            };
          }
          return h("div", props, `Row ${index}`);
        }),
      );
  },
}).mount();
