// Hover-cost fixture: one column of rows, built three ways, optionally scrolled.
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
//
// SCROLL wraps the column in a fixed-height scrollport at a non-zero offset.
// That is not cosmetic: with no scroll container the paint-box store's view
// overlays stay empty for the life of the run, and every gate keyed on that
// store looks like it works. A scrolled tree is the case those gates have to
// survive, so it needs its own build.
import { h, ref } from "@vue/runtime-core";
import { createApp } from "../../../../../packages/nanavue-runtime/src/createNanaRenderer.js";

const ROWS = __HOVER_BENCH_ROWS__;
const MODE = __HOVER_BENCH_MODE__;
const SCROLL = __HOVER_BENCH_SCROLL__;
const ROW_HEIGHT = 24;
const PORT_HEIGHT = 480;
// Deep enough that rows above and below the port stay clipped, so the overlays
// the store keeps are the interesting kind rather than a no-op translation.
const SCROLL_TOP = 240;

// Written by the `listeners` handler and never read by the render function, so
// the handler does real work without invalidating anything. Deliberately not a
// `ref`: a reactive write here would quietly turn this mode into `reactive`.
let touched = 0;
globalThis.__hoverBenchTouched = () => touched;

createApp({
  setup() {
    const active = ref(-1);
    const column = () =>
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
    if (!SCROLL) return column;
    return () =>
      h(
        "nana-scroll-view",
        {
          "data-agent-id": "port",
          ref: (el) => {
            if (el && el.scrollTop !== SCROLL_TOP) el.scrollTop = SCROLL_TOP;
          },
          style: {
            width: "320px",
            height: `${PORT_HEIGHT}px`,
            overflowY: "auto",
          },
        },
        [column()],
      );
  },
}).mount();
