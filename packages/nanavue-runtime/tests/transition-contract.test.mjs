import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  NANA_TRANSITION_COMPUTED_DEFAULTS,
  appearEnterPhaseAfter,
  applyFlipPaintTransform,
  armMotionEndFromStyles,
  cancelArmedMotionEnd,
  motionEndFallbackWaitMs,
  createMotionEndEvent,
  cssTimeToMs,
  flipDelta,
  isPaintOnlyStyleKey,
  isVueTransitionClass,
  preserveMotionClasses,
  resolveTransitionComputedStyles,
  transitionInfoLooksImmediate,
  vueTransitionClassKind,
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

test("cascade motion resolves non-zero transition duration", () => {
  const styles = resolveTransitionComputedStyles({
    transitionDuration: "0.2s",
    transitionProperty: "opacity",
  });
  assert.equal(styles.transitionDuration, "0.2s");
  assert.ok(!transitionInfoLooksImmediate(styles));
});

test("shim getComputedStyle exposes camelCase transition keys", () => {
  assert.match(shim, /transitionDuration/);
  assert.match(shim, /animationDuration/);
  assert.match(shim, /computedStyle/);
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

test("css time parser reads seconds and milliseconds", () => {
  assert.equal(cssTimeToMs("0s"), 0);
  assert.equal(cssTimeToMs("0.24s"), 240);
  assert.equal(cssTimeToMs("160ms"), 160);
  assert.equal(cssTimeToMs("0.1s, 0.2s"), 200);
});

test("appear/enter class kinds and vnode class replace keep motion tokens", () => {
  assert.equal(vueTransitionClassKind("fade-enter-from"), "enter-from");
  assert.equal(vueTransitionClassKind("v-appear-active"), "appear-active");
  assert.equal(vueTransitionClassKind("list-move"), "move");
  assert.equal(isVueTransitionClass("card"), false);
  const kept = preserveMotionClasses("card open", [
    "fade-enter-from",
    "fade-enter-active",
  ]);
  assert.ok(kept.includes("card"));
  assert.ok(kept.includes("fade-enter-from"));
  assert.ok(kept.includes("fade-enter-active"));
  assert.equal(
    appearEnterPhaseAfter(["fade-enter-from", "fade-enter-active"]),
    "enter-from-active",
  );
  assert.equal(
    appearEnterPhaseAfter(["fade-appear-from", "fade-appear-active", "fade-appear-to"]),
    "appear-to",
  );
});

test("FLIP delta is layout-box inverse translate, not LayoutBox writeback", () => {
  const prev = { left: 40, top: 10, width: 20, height: 16 };
  const next = { left: 10, top: 10, width: 20, height: 16 };
  assert.deepEqual(flipDelta(prev, next), { dx: 30, dy: 0 });
  const el = { style: {} };
  const result = applyFlipPaintTransform(el, prev, next);
  assert.equal(result.applied, true);
  assert.equal(el.style.transform, "translate(30px, 0px)");
  assert.equal(el.style.transitionDuration, "0s");
  assert.equal(isPaintOnlyStyleKey("transform"), true);
  assert.equal(isPaintOnlyStyleKey("width"), false);
});

test("motion end event is a host-dispatchable Event, not WAAPI", () => {
  const target = { id: 3 };
  const event = createMotionEndEvent("transitionend", target, {
    propertyName: "opacity",
    elapsedTime: 0.16,
  });
  assert.equal(event.type, "transitionend");
  assert.equal(event.target, target);
  assert.equal(event.propertyName, "opacity");
  assert.equal(event.elapsedTime, 0.16);
  assert.equal(typeof event.preventDefault, "function");
  assert.equal(renderer.includes("element.animate"), false);
  assert.match(renderer, /__nanaMotionComplete/);
  assert.match(renderer, /isPaintOnlyStyleKey/);
  assert.match(renderer, /setPaintTransform/);
});

test("armed motion end timeout dispatches once through the host callback", async () => {
  const hits = [];
  const styles = { transitionDuration: "10ms", transitionProperty: "opacity" };
  const wait = armMotionEndFromStyles(9, styles, (detail) => hits.push(detail));
  assert.equal(wait, motionEndFallbackWaitMs(styles));
  assert.ok(wait >= 10 + 32, "fallback is duration + 2 frames, not duration+1ms");
  await new Promise((resolve) => setTimeout(resolve, wait + 20));
  assert.equal(hits.length, 1);
  assert.equal(hits[0].type, "transitionend");
  assert.equal(hits[0].propertyName, "opacity");
});

test("host complete cancels the class-arm fallback so transitionend fires once", async () => {
  const hits = [];
  const styles = { transitionDuration: "10ms", transitionProperty: "opacity" };
  const wait = armMotionEndFromStyles(5, styles, () => hits.push("timeout"));
  cancelArmedMotionEnd(5);
  hits.push("complete");
  await new Promise((resolve) => setTimeout(resolve, wait + 20));
  assert.deepEqual(hits, ["complete"]);
  assert.match(renderer, /__nanaMotionCancel/);
});
