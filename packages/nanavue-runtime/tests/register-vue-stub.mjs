import { register } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const stub = pathToFileURL(join(here, "vue-runtime-core-stub.mjs")).href;

register(pathToFileURL(join(here, "vue-stub-hooks.mjs")).href, import.meta.url, {
  data: { stub },
});
