import { build } from "vite";
import { fileURLToPath } from "node:url";
const path = relative => fileURLToPath(new URL(relative, import.meta.url));
const runtimeCore = path("./node_modules/@vue/runtime-core/dist/runtime-core.esm-bundler.js");
const table = process.argv[2] === "table";
const activity = process.argv[2] === "activity";
await build({
  configFile: false,
  resolve: {alias: {"@vue/runtime-core": runtimeCore}},
  build: {
    target: "es2020", outDir: path(activity ? "../../../../target/virtual-activity" : table ? "../../../../target/virtual-table" : "../../../../target/virtual-navigation"), emptyOutDir: false,
    lib: {entry: path(activity ? "./src/VirtualActivity.js" : table ? "./src/VirtualFrozenTable.js" : "./src/VirtualNavigation.js"), name: "VirtualNavigation", formats: ["iife"], fileName: () => "app.js"},
  },
});
