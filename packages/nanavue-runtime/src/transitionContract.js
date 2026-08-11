/**
 * Vue Transition + Teleport contract notes for Nana custom renderer.
 *
 * Implemented (honest minimal):
 * - Teleport `to="body"` / `html` → stable mount-root wrapNode
 *   (`document.body` === `querySelector("body")` === `mountRoot` nid/proxy)
 * - L2 Overlay (Dialog/Drawer/Popover/ContextMenu) may live under that mount-root
 *   alongside anonymous CSS `position:fixed` layers — Overlay is not a second DOM portal
 * - Insert/remove under mount-root must not leave MessageBridge widgets behind
 * - CSS Transition end detection reports 0s durations via getComputedStyle
 * - VueHost::pump_frame drains nested rAF (Transition `nextFrame` is double-rAF)
 *   so `@after-leave` fires for LiliaUI Dialog / Drawer / Dropdown
 *
 * Deferred (no CSS transition/animation engine on Nana):
 * - Real timed `transition-duration` / `animation-duration` from stylesheets
 * - `transitionend` / `animationend` event fidelity
 * - TransitionGroup move transitions
 * - appear/enter visual class timing beyond immediate completion
 * - Full DOM / ARIA Teleport portal (product floats use Nana Overlay)
 */

/** Keys Vue runtime-dom `getTransitionInfo` reads on getComputedStyle(el). */
export const NANA_TRANSITION_COMPUTED_DEFAULTS = Object.freeze({
  transitionDelay: "0s",
  transitionDuration: "0s",
  transitionProperty: "none",
  animationDelay: "0s",
  animationDuration: "0s",
  animationName: "none",
});

/**
 * Mirror Vue `getTimeout` + empty style → immediate resolve (type null).
 * Used by unit tests; host shim installs the same defaults on window.getComputedStyle.
 */
export function transitionInfoLooksImmediate(styles) {
  const dur = String((styles && styles.transitionDuration) || "0s");
  const adelay = String((styles && styles.animationDuration) || "0s");
  const toMs = (s) => {
    if (s === "auto" || !s) return 0;
    const n = Number(String(s).slice(0, -1).replace(",", "."));
    return Number.isFinite(n) ? n * 1e3 : 0;
  };
  return Math.max(toMs(dur), toMs(adelay)) <= 0;
}

/**
 * Teleport target selectors Nana maps to the stable document mount-root / html root.
 * Anything else is a normal querySelector — not a fake `document.body` portal.
 */
export const NANA_TELEPORT_MOUNT_SELECTORS = Object.freeze(["body", "html"]);

export function isNanaTeleportMountSelector(sel) {
  const lower = String(sel ?? "")
    .trim()
    .toLowerCase();
  return NANA_TELEPORT_MOUNT_SELECTORS.includes(lower);
}
