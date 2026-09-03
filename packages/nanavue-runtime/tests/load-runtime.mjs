/** Execute the actual module graph; each sandbox gets exactly one identity cache. */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export function loadShimSource() {
  const directory = resolve(root, "../../crates/nana-ui-web-api/src/shim");
  return readFileSync(resolve(directory, "manifest.txt"), "utf8").trim().split(/\r?\n/).map(name => readFileSync(resolve(directory, name), "utf8")).join("\n");
}
export async function loadRenderer(sandbox) {
  const context = vm.isContext(sandbox) ? sandbox : vm.createContext(sandbox);
  const modules = new Map();
  function moduleAt(path) {
    if (modules.has(path)) return modules.get(path);
    const module = path === "@vue/runtime-core"
      ? new vm.SyntheticModule(["createRenderer"], function () {
          this.setExport("createRenderer", sandbox.createRenderer || (() => ({ createApp() { return {}; }, render() {} })));
        }, { context })
      : new vm.SourceTextModule(readFileSync(path, "utf8"), { context, identifier: path });
    modules.set(path, module);
    return module;
  }
  const entry = moduleAt(resolve(root, "src/createNanaRenderer.js"));
  await entry.link((specifier, referrer) => moduleAt(specifier === "@vue/runtime-core" ? specifier : resolve(dirname(referrer.identifier), specifier)));
  await entry.evaluate();
  return entry.namespace;
}
export async function createTestRuntime() {
  let next = 10;
  const calls = [];
  const attrs = new Map();
  const sandbox = { console, queueMicrotask, setTimeout, clearTimeout, Promise };
  sandbox.globalThis = sandbox;
  sandbox.__nanaHost = { call(name, args = []) {
    calls.push([name, args]);
    if (name === "mountRoot") return 2;
    if (name === "querySelector") return args[0] === "html" ? 1 : args[0] === "body" ? 2 : null;
    if (name === "querySelectorAll") return args[0] === "body" ? [2] : [];
    if (name === "createElement" || name === "createWidget") return next++;
    if (name === "nodeKind") return "element";
    if (name === "elementTag") return args[0] === 2 ? "body" : "div";
    if (name === "getAttribute") return attrs.get(`${args[0]}:${args[1]}`) ?? null;
    if (name === "patchProp") attrs.set(`${args[0]}:${args[1]}`, args[2]);
    if (name === "childNodes") return [];
    if (name === "computedStyle") return { transitionDuration: "0.2s", transitionProperty: "opacity", animationDuration: "0s" };
    if (name === "layoutBox") return { x: 10, y: 20, width: 80, height: 40, clientWidth: 78, clientHeight: 38, scrollWidth: 80, scrollHeight: 40 };
    return null;
  }};
  vm.runInNewContext(loadShimSource(), sandbox);
  const api = await loadRenderer(sandbox);
  return { sandbox, calls, api };
}
