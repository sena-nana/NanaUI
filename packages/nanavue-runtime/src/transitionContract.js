/**
 * Vue Transition + Teleport contract notes for Nana custom renderer.
 *
 * Implemented (honest minimal):
 * - Teleport `to="body"` / `html` → stable mount-root wrapNode
 *   (`document.body` === `querySelector("body")` === `mountRoot` nid/proxy)
 * - L2 Overlay (Dialog/Drawer/Popover/ContextMenu) may live under that mount-root
 *   alongside anonymous CSS `position:fixed` layers — Overlay is not a second DOM portal
 * - Insert/remove under mount-root must not leave MessageBridge widgets behind
 * - CSS Transition end detection reads cascade-resolved durations via getComputedStyle
 * - VueHost::pump_frame drains nested rAF (Transition `nextFrame` is double-rAF)
 *   so `@after-leave` fires for LiliaUI Dialog / Drawer / Dropdown
 * - `transitionend` / `animationend` via Rust `__nanaMotionComplete` (primary)
 *   plus a class-armed fallback of duration+2 frames if no Runtime timeline
 *   starts (not WAAPI / Element.animate; not a parallel dispatcher)
 * - Transition appear/enter class tokens survive vnode `class` replace
 * - TransitionGroup FLIP uses getBoundingClientRect/layoutBox paint overlay;
 *   inline transform is not written back as Runtime LayoutBox
 * - FLIP `style.transform` is a paint-only host op (`setPaintTransform`) onto
 *   `LayoutStyle.transform` → UiScene node affine → SceneWgpuPainter
 * - Layout long-hand CSS transitions (width/height/margin/padding) lerp through
 *   bridge.rs → LayoutStyle → incremental layout
 *
 * Vue runtime constructs that reuse the same host ops:
 * - KeepAlive moves via insert into an unparented storage node (not a second tree)
 * - Suspense fallback/default are ordinary vnodes
 * - v-html parses a fragment into live host children (not markup-as-text)
 *
 * Deferred (no WAAPI / browser portal):
 * - Full WAAPI / Element.animate
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

/** Inline keys TransitionGroup FLIP writes; paint overlay only. */
export const NANA_PAINT_ONLY_STYLE_KEYS = Object.freeze([
  "transform",
  "webkitTransform",
  "MozTransform",
  "msTransform",
]);

const MOTION_CLASS_RE = /(?:^|-)(?:enter|leave|appear)(?:-from|-active|-to)?$|(?:^|-)move$/;
const MOTION_END_TYPES = new Set(["transitionend", "animationend"]);

/**
 * Mirror Vue `getTimeout` + empty style → immediate resolve (type null).
 * Used by unit tests; host shim installs cascade-resolved defaults on
 * window.getComputedStyle when MessageBridge motion rules apply.
 */
export function transitionInfoLooksImmediate(styles) {
  const dur = String((styles && styles.transitionDuration) || "0s");
  const adelay = String((styles && styles.animationDuration) || "0s");
  return Math.max(cssTimeToMs(dur), cssTimeToMs(adelay)) <= 0;
}

export function cssTimeToMs(value) {
  const s = String(value || "0s").trim();
  if (!s || s === "auto") return 0;
  const parts = s.split(",");
  let max = 0;
  for (const part of parts) {
    const raw = part.trim().replace(",", ".");
    if (!raw) continue;
    let n;
    let ms = 0;
    if (raw.endsWith("ms")) {
      n = Number(raw.slice(0, -2));
      ms = n;
    } else if (raw.endsWith("s")) {
      n = Number(raw.slice(0, -1));
      ms = n * 1e3;
    } else {
      n = Number(raw);
      ms = n;
    }
    if (!Number.isFinite(n)) continue;
    if (ms > max) max = ms;
  }
  return max;
}

/**
 * Resolve transition timing from host cascade motion when present.
 * Falls back to immediate defaults when no stylesheet motion applies.
 */
export function resolveTransitionComputedStyles(hostMotion) {
  if (!hostMotion || typeof hostMotion !== "object") {
    return { ...NANA_TRANSITION_COMPUTED_DEFAULTS };
  }
  return {
    transitionDelay: String(hostMotion.transitionDelay || "0s"),
    transitionDuration: String(hostMotion.transitionDuration || "0s"),
    transitionProperty: String(hostMotion.transitionProperty || "none"),
    animationDelay: String(hostMotion.animationDelay || "0s"),
    animationDuration: String(hostMotion.animationDuration || "0s"),
    animationName: String(hostMotion.animationName || "none"),
  };
}

export function isPaintOnlyStyleKey(key) {
  return NANA_PAINT_ONLY_STYLE_KEYS.includes(String(key));
}

export function isVueTransitionClass(token) {
  return MOTION_CLASS_RE.test(String(token || "").trim());
}

export function vueTransitionClassKind(token) {
  const t = String(token || "").trim();
  const m = t.match(
    /(?:^|-)(enter|leave|appear)(-from|-active|-to)?$|(?:^|-)(move)$/,
  );
  if (!m) return null;
  if (m[3] === "move") return "move";
  return `${m[1]}${m[2] || ""}`;
}

/**
 * Keep Vue Transition/TransitionGroup tokens when a vnode `class` patch
 * replaces the string so appear/enter-from/active survive the next frame.
 */
export function preserveMotionClasses(nextClassValue, previousTokens) {
  const next = new Set(
    String(nextClassValue || "")
      .split(/\s+/)
      .filter(Boolean),
  );
  for (const token of previousTokens || []) {
    if (isVueTransitionClass(token)) next.add(token);
  }
  return [...next];
}

/**
 * Appear/enter visual class order Vue runtime-dom uses:
 * first frame `*-from` + `*-active`, next frame drop `-from` and add `-to`.
 */
export function appearEnterPhaseAfter(tokens) {
  const kinds = new Set(
    [...tokens].map(vueTransitionClassKind).filter(Boolean),
  );
  const appear = kinds.has("appear-from") || kinds.has("appear-active");
  const prefix = appear ? "appear" : "enter";
  if (kinds.has(`${prefix}-from`) && kinds.has(`${prefix}-active`)) {
    if (kinds.has(`${prefix}-to`)) return `${prefix}-to`;
    return `${prefix}-from-active`;
  }
  if (kinds.has(`${prefix}-active`) && kinds.has(`${prefix}-to`)) {
    return `${prefix}-to`;
  }
  return null;
}

export function createMotionEndEvent(type, target, extra) {
  const kind = MOTION_END_TYPES.has(String(type)) ? String(type) : "transitionend";
  const source = extra && typeof extra === "object" ? extra : {};
  const propertyName =
    source.propertyName != null
      ? String(source.propertyName)
      : kind === "animationend"
        ? String(source.animationName || "none")
        : String(source.transitionProperty || "all");
  return {
    type: kind,
    target,
    currentTarget: target,
    bubbles: true,
    cancelable: false,
    defaultPrevented: false,
    eventPhase: 0,
    timeStamp: Number(source.timeStamp || 0),
    elapsedTime: Number(source.elapsedTime || 0),
    propertyName,
    animationName: String(source.animationName || ""),
    pseudoElement: String(source.pseudoElement || ""),
    _stopped: false,
    _immediateStopped: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    stopPropagation() {
      this._stopped = true;
    },
    stopImmediatePropagation() {
      this._stopped = true;
      this._immediateStopped = true;
    },
  };
}

export function motionEndTimeoutMs(styles) {
  const resolved = resolveTransitionComputedStyles(styles);
  const t =
    cssTimeToMs(resolved.transitionDuration) + cssTimeToMs(resolved.transitionDelay);
  const a =
    cssTimeToMs(resolved.animationDuration) + cssTimeToMs(resolved.animationDelay);
  return Math.max(t, a);
}

export function motionEndTypeFromStyles(styles) {
  const resolved = resolveTransitionComputedStyles(styles);
  const t = cssTimeToMs(resolved.transitionDuration);
  const a = cssTimeToMs(resolved.animationDuration);
  if (a > t) return "animationend";
  return "transitionend";
}

const armedMotionEnds = new Map();

/** One Runtime / host frame. Fallback slack is two of these, not +1ms. */
export const MOTION_END_FRAME_MS = 16;
export const MOTION_END_FALLBACK_FRAMES = 2;

export function motionEndFallbackWaitMs(styles) {
  const timeout = motionEndTimeoutMs(styles);
  if (timeout <= 0) return 0;
  return timeout + MOTION_END_FALLBACK_FRAMES * MOTION_END_FRAME_MS;
}

export function cancelArmedMotionEnd(nid) {
  const key = Number(nid);
  const handle = armedMotionEnds.get(key);
  if (handle == null) return;
  armedMotionEnds.delete(key);
  if (typeof clearTimeout === "function") clearTimeout(handle);
}

/**
 * Arm a fallback timeout only. Rust `__nanaMotionComplete` is the dispatcher
 * once a Runtime timeline is running. Slack is duration+2 frames so a hosted
 * wake that applies samples at T_end can complete before this fires.
 * Not WAAPI: no `element.animate`, no Animation timeline.
 */
export function armMotionEndFromStyles(nid, styles, dispatch) {
  const id = Number(nid);
  if (!Number.isFinite(id) || typeof dispatch !== "function") return 0;
  cancelArmedMotionEnd(id);
  const timeout = motionEndTimeoutMs(styles);
  if (timeout <= 0) return 0;
  const type = motionEndTypeFromStyles(styles);
  const resolved = resolveTransitionComputedStyles(styles);
  const wait = motionEndFallbackWaitMs(styles);
  if (typeof setTimeout !== "function") return wait;
  const handle = setTimeout(() => {
    armedMotionEnds.delete(id);
    dispatch({
      type,
      elapsedTime: timeout / 1000,
      propertyName:
        type === "animationend"
          ? resolved.animationName
          : resolved.transitionProperty,
      animationName: resolved.animationName,
      transitionProperty: resolved.transitionProperty,
    });
  }, wait);
  armedMotionEnds.set(id, handle);
  return wait;
}

function boxFromRect(rect) {
  const x = Number(rect && (rect.x ?? rect.left)) || 0;
  const y = Number(rect && (rect.y ?? rect.top)) || 0;
  const width = Number(rect && rect.width) || 0;
  const height = Number(rect && rect.height) || 0;
  return {
    x,
    y,
    width,
    height,
    top: y,
    left: x,
    bottom: y + height,
    right: x + width,
  };
}

/** Record a layout-projection box for TransitionGroup FLIP (not Runtime LayoutBox). */
export function readFlipBox(el) {
  if (!el) return boxFromRect(null);
  if (typeof el.getBoundingClientRect === "function") {
    return boxFromRect(el.getBoundingClientRect());
  }
  return boxFromRect(el);
}

export function flipDelta(prevBox, nextBox) {
  const prev = boxFromRect(prevBox);
  const next = boxFromRect(nextBox);
  return {
    dx: prev.left - next.left,
    dy: prev.top - next.top,
  };
}

/**
 * Apply the inverse translate as a paint overlay. Callers must not write
 * Runtime LayoutBox; the renderer sends paint-only keys through
 * `setPaintTransform` (not `patchProp` style / recascade).
 */
export function applyFlipPaintTransform(el, prevBox, nextBox) {
  if (!el || !el.style) return { dx: 0, dy: 0, applied: false };
  const { dx, dy } = flipDelta(prevBox, nextBox);
  if (!dx && !dy) {
    el.style.transform = "";
    el.style.webkitTransform = "";
    return { dx: 0, dy: 0, applied: false };
  }
  const value = `translate(${dx}px, ${dy}px)`;
  el.style.transitionDuration = "0s";
  el.style.transform = value;
  el.style.webkitTransform = value;
  return { dx, dy, applied: true };
}

export function clearFlipPaintTransform(el) {
  if (!el || !el.style) return;
  el.style.transform = "";
  el.style.webkitTransform = "";
  el.style.transitionDuration = "";
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
