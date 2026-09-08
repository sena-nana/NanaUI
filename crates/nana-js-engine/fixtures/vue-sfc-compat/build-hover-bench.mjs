// Builds the two HoverBench bundles the hover benchmark compares.
//
// Output lands under `target/`, not `dist/`: these are measurement inputs
// regenerated on demand, not a committed fixture whose bytes CI diffs.
//
//   node build-hover-bench.mjs <bare|listeners|reactive|reactive-components>[-scroll] [rows]
import { build } from "vite";
import { fileURLToPath } from "node:url";

const path = (relative) => fileURLToPath(new URL(relative, import.meta.url));
const runtimeCore = path(
  "./node_modules/@vue/runtime-core/dist/runtime-core.esm-bundler.js",
);

const MODES = ["bare", "listeners", "reactive", "reactive-components"];
const requested = process.argv[2] ?? "bare";
const scroll = requested.endsWith("-scroll");
const mode = scroll ? requested.slice(0, -"-scroll".length) : requested;
if (!MODES.includes(mode)) {
  throw new Error(
    `mode must be one of ${MODES.join(", ")} (optionally -scroll), got ${requested}`,
  );
}
const rows = Number(process.argv[3] ?? 2000);
if (!Number.isInteger(rows) || rows <= 0) {
  throw new Error(`rows must be a positive integer, got ${process.argv[3]}`);
}

await build({
  configFile: false,
  resolve: { alias: { "@vue/runtime-core": runtimeCore } },
  define: {
    __HOVER_BENCH_ROWS__: String(rows),
    __HOVER_BENCH_MODE__: JSON.stringify(mode),
    __HOVER_BENCH_SCROLL__: String(scroll),
  },
  build: {
    target: "es2020",
    outDir: path(`../../../../target/hover-bench/${requested}-${rows}`),
    emptyOutDir: false,
    lib: {
      entry: path("./src/HoverBench.js"),
      name: "HoverBench",
      formats: ["iife"],
      fileName: () => "app.js",
    },
  },
});
