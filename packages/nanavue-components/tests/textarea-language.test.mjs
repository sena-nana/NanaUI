/**
 * Behavior: `<NanaTextarea language="rust">` hands the Rust host a `language`
 * attribute through the real renderer host ops, in the `createElement` seed
 * and as a `patchProp`. crates/nana-ui-vue's
 * `vue_textarea_language_reaches_the_runtime_as_a_highlighted_editor` takes
 * those calls to a highlighted code editor. No Vue is installed here, so
 * `@vue/runtime-core` resolves to a stand-in whose `h` returns the vnode.
 */
import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { test } from "node:test";

const vue = "export const h = (type, props) => ({ type, props }); export const createRenderer = () => ({});";
registerHooks({
  resolve: (specifier, context, nextResolve) =>
    specifier === "@vue/runtime-core"
      ? { url: `data:text/javascript,${encodeURIComponent(vue)}`, shortCircuit: true }
      : nextResolve(specifier, context),
});

const calls = [];
globalThis.__nanaHost = {
  call(name, args) {
    calls.push([name, args]);
    return name === "createElement" ? 1 : null;
  },
};
const { hostOps } = await import("../../nanavue-runtime/src/createNanaRenderer.js");
const { NanaTextarea } = await import("../src/NanaTextarea.js");

/**
 * Mount one render as Vue's `mountElement` does (create with the props, then
 * patch each) and return what the host got for `key`: the seed's, then the patch's.
 */
function sent(key, props, attrs = {}) {
  calls.length = 0;
  const vnode = NanaTextarea.setup(props, { emit() {}, attrs })();
  const el = hostOps.createElement(vnode.type, undefined, undefined, vnode.props);
  for (const [name, value] of Object.entries(vnode.props)) hostOps.patchProp(el, name, null, value);
  return calls.flatMap(([name, args]) => {
    if (name === "createElement") return [args[3][key]];
    return name === "patchProp" && args[1] === key ? [args[2]] : [];
  });
}

test("language reaches the host in the createElement seed and as a patch", () => {
  assert.deepEqual(sent("language", { language: "rust" }), ["rust", "rust"]);
});

test("an empty language reaches the host as null, which clears it", () => {
  assert.deepEqual(sent("language", { language: "" }), [null, null]);
});

test("syntax falls through to the host as an attribute", () => {
  assert.deepEqual(sent("syntax", {}, { syntax: "rs" }), ["rs", "rs"]);
});
