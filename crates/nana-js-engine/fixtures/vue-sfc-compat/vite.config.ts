import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath } from "node:url";

const runtimeCore = fileURLToPath(
  new URL("./node_modules/@vue/runtime-core/dist/runtime-core.esm-bundler.js", import.meta.url),
);

export default defineConfig({
  plugins: [vue()],
  resolve: {
    // SFC compiler helpers are runtime-core exports. Nana supplies createApp via
    // createRenderer, so runtime-dom is intentionally excluded from the bundle.
    alias: {
      vue: runtimeCore,
      "@vue/runtime-core": runtimeCore,
    },
  },
  build: {
    target: "es2020",
    cssCodeSplit: false,
    emptyOutDir: true,
    lib: {
      entry: "src/main.ts",
      name: "NanaVueSfcCompatFixture",
      formats: ["iife"],
      fileName: () => "vue-sfc-compat.iife.js",
    },
  },
});
