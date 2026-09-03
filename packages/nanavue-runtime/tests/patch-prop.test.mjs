/** Behavior at the public renderer boundary; independent of implementation files. */
import assert from "node:assert/strict";
import { test } from "node:test";
import { createTestRuntime } from "./load-runtime.mjs";

test("prop and attr modifiers preserve values and clear false attributes", async () => {
  const { api: { hostOps }, calls } = await createTestRuntime();
  const node = hostOps.createElement("input");
  hostOps.patchProp(node, ".value", null, "typed");
  hostOps.patchProp(node, "^disabled", null, true);
  hostOps.patchProp(node, "^disabled", true, false);
  assert.equal(node.value, "typed");
  assert.equal(node.attributes.disabled, undefined);
  assert.ok(calls.some(([name, args]) => name === "patchProp" && args[1] === "value" && args[2] === "typed"));
});
test("class patches and classList changes share one visible value", async () => {
  const { api: { hostOps } } = await createTestRuntime();
  const node = hostOps.createElement("div");
  hostOps.patchProp(node, "class", null, "base");
  assert.ok(node.classList.contains("base"));
  node.classList.add("selected");
  assert.equal(node.attributes.class, "base selected");
  node.classList.remove("base");
  assert.equal(node.className, "selected");
});
test("SVG presentation and namespace attributes reach the host unchanged", async () => {
  const { api: { hostOps }, calls } = await createTestRuntime();
  const node = hostOps.createElement("svg", "svg");
  hostOps.patchProp(node, "viewBox", null, "0 0 40 20", "svg");
  hostOps.patchProp(node, "xlink:href", null, "#shape", "svg");
  assert.equal(node.__isSVG, true);
  assert.equal(node.attributes.viewBox, "0 0 40 20");
  assert.ok(calls.some(([name, args]) => name === "patchProp" && args[1] === "xlink:href" && args[2] === "#shape"));
});
