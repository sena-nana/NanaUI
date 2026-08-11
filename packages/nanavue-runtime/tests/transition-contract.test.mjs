import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  NANA_TRANSITION_COMPUTED_DEFAULTS,
  transitionInfoLooksImmediate,
} from "../src/transitionContract.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const shim = readFileSync(
  join(root, "../../crates/nana-ui-web-api/src/shim.js"),
  "utf8",
);
const renderer = readFileSync(join(root, "src/createNanaRenderer.js"), "utf8");
const vueHost = readFileSync(
  join(root, "../../crates/nana-ui-vue/src/lib.rs"),
  "utf8",
);

test("transition defaults are immediate (0s)", () => {
  assert.equal(NANA_TRANSITION_COMPUTED_DEFAULTS.transitionDuration, "0s");
  assert.ok(transitionInfoLooksImmediate(NANA_TRANSITION_COMPUTED_DEFAULTS));
  assert.ok(!transitionInfoLooksImmediate({ transitionDuration: "0.14s" }));
});

test("shim getComputedStyle exposes camelCase transition keys", () => {
  assert.match(shim, /transitionDuration:\s*"0s"/);
  assert.match(shim, /animationDuration:\s*"0s"/);
  assert.match(shim, /no CSS transition\/animation engine/i);
});

test("document.body uses wrapHostNode for Teleport stability", () => {
  assert.match(shim, /wrapHostNode\(id,\s*"body"\)/);
  assert.match(shim, /Stable `document\.body` for Vue Teleport/);
  assert.match(shim, /hostNodeCache/);
});

test("hostOps querySelector tags body/html for Teleport", () => {
  assert.match(renderer, /lower === "body" \|\| lower === "html"/);
  assert.match(renderer, /Teleport `to="body"`/);
});

test("shim document.querySelector tags body for Teleport identity", () => {
  assert.match(shim, /teleportTargetTag/);
});

test("VueHost pump_frame drains nested rAF for Transition nextFrame", () => {
  assert.match(vueHost, /MAX_TIMER_PASSES/);
  assert.match(vueHost, /double-rAF|nextFrame/);
  assert.match(vueHost, /after-leave|Dialog\/Drawer/);
});

/**
 * Mirror Vue runtime-dom leave: nextFrame(double rAF) → getTransitionInfo →
 * resolve when type is null. Nested rAF drain must complete in one host pump.
 */
test("simulated Transition after-leave completes with nested rAF drain", async () => {
  const pending = [];
  const raf = (cb) => {
    pending.push(cb);
    return pending.length;
  };
  const nextFrame = (cb) => raf(() => raf(cb));
  const getTransitionInfo = (styles) =>
    transitionInfoLooksImmediate(styles) ? { type: null } : { type: "transition" };

  let afterLeave = false;
  const leave = () =>
    new Promise((resolve) => {
      nextFrame(() => {
        const info = getTransitionInfo(NANA_TRANSITION_COMPUTED_DEFAULTS);
        assert.equal(info.type, null);
        afterLeave = true;
        resolve();
      });
    });

  const done = leave();
  // One-shot drain would leave the second rAF pending (pre-fix hang).
  assert.equal(pending.length, 1);
  const first = pending.shift();
  first();
  assert.equal(pending.length, 1);
  // Nested drain (VueHost::pump_frame loop):
  while (pending.length) pending.shift()();
  await done;
  assert.equal(afterLeave, true);
});
