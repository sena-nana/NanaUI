/** Shared style queue; flushing never invalidates node identity or hierarchy. */
import { hostCall } from "./layoutMetrics.js";
import { isPaintOnlyStyleKey } from "./transitionContract.js";

/**
 * Per node, what each writer has declared and what the host was last given.
 *
 * Two writers own one style attribute: Vue's `patchProp` and the `el.style`
 * proxy. The host op replaces the whole attribute, so whoever writes last used
 * to erase the other -- a `TransitionGroup` FLIP writing `transitionDuration`
 * through the proxy would drop every declaration Vue had patched. Keeping the
 * two layers apart and sending their merge is what makes both survive, and it
 * is also what makes `sent` a truthful answer to "does the host already have
 * this?", which is the question that lets an unchanged repatch stay home.
 *
 * `vue` is the cleaned object from `patchProp`, or the raw CSS string Vue handed
 * over. A string wins outright rather than merging: parsing it back into
 * declarations would have to get `url(a:b)` and quoted semicolons right, for no
 * gain over what that path did before.
 */
const styleLayers = new Map();
const pendingStyleFlush = new Set();
let styleFlushScheduled = false;

function layersFor(nid) {
  let layers = styleLayers.get(nid);
  if (!layers) {
    layers = { vue: null, proxy: null, sent: undefined };
    styleLayers.set(nid, layers);
  }
  return layers;
}

/** The declarations the host should hold for `nid`, or `null` to clear. */
function mergedStyle(layers) {
  const vue = layers.vue;
  if (typeof vue === "string") return vue;
  let proxy = layers.proxy ? hostStyleStore(layers.proxy) : null;
  // A registered but empty proxy layer is the same as no proxy layer. Letting
  // `{}` count as present would turn "Vue cleared the style" into "Vue set an
  // empty style", and the host clears the attribute only for `null`.
  if (proxy && Object.keys(proxy).length === 0) proxy = null;
  if (!vue && !proxy) return null;
  // The proxy wins per key: it is an imperative write applied after render, the
  // same precedence the DOM gives `el.style.foo = v` over the attribute.
  return { ...(vue || {}), ...(proxy || {}) };
}

/** Whether the host already holds exactly `next`. */
function alreadySent(sent, next) {
  if (sent === next) return true;
  if (!sent || !next || typeof sent !== "object" || typeof next !== "object") return false;
  const keys = Object.keys(next);
  if (keys.length !== Object.keys(sent).length) return false;
  for (const key of keys) {
    if (sent[key] !== next[key]) return false;
  }
  return true;
}

/** Send the merged style unless the host already has exactly it. */
function sendStyle(nid) {
  const layers = layersFor(nid);
  const next = mergedStyle(layers);
  if (layers.sent !== undefined && alreadySent(layers.sent, next)) return false;
  layers.sent = next;
  try {
    hostCall("patchProp", [nid, "style", next]);
  } catch (_err) {}
  return true;
}

export function flushPendingStyles() {
  if (!pendingStyleFlush.size) return;
  const batch = [...pendingStyleFlush];
  pendingStyleFlush.clear();
  for (const nid of batch) sendStyle(nid);
}

/**
 * Record what Vue's `patchProp` declared and send it if the host lacks it.
 *
 * `cleaned` is an object, or a raw CSS string, or `null` to clear. Returns
 * whether anything crossed into the host.
 */
export function setVueStyle(nid, cleaned) {
  layersFor(nid).vue = cleaned ?? null;
  return sendStyle(nid);
}

/** Drop every layer for a node. Called wherever the node's host state dies. */
export function forgetStyle(nid) {
  styleLayers.delete(nid);
  pendingStyleFlush.delete(nid);
}

export function queueStyleFlush(nid, store) {
  layersFor(nid).proxy = store;
  pendingStyleFlush.add(nid);
  if (styleFlushScheduled) return;
  styleFlushScheduled = true;
  const run = () => {
    styleFlushScheduled = false;
    flushPendingStyles();
  };
  if (typeof queueMicrotask === "function") queueMicrotask(run);
  else Promise.resolve().then(run);
}

/** Commit batched style patches. Does not invalidate wrapNode parent/child cache. */
export function flushHostFrame() {
  flushPendingStyles();
  styleFlushScheduled = false;
}

export function installFlushHooks() {
  globalThis.__nanaFlushHostFrame = flushHostFrame;
  const prevNotify = globalThis.__nanaNotifyLayout;
  globalThis.__nanaNotifyLayout = function nanaNotifyLayoutAndFlush() {
    flushHostFrame();
    if (typeof prevNotify === "function") return prevNotify.apply(this, arguments);
  };
}

export function parseCssText(cssText) {
  const store = Object.create(null);
  for (const decl of String(cssText || "").split(";")) {
    const idx = decl.indexOf(":");
    if (idx < 0) continue;
    const name = decl.slice(0, idx).trim();
    const value = decl.slice(idx + 1).trim();
    if (name) store[name] = value;
  }
  return store;
}

export function hostStyleStore(store) {
  const out = {};
  for (const [key, value] of Object.entries(store)) {
    if (isPaintOnlyStyleKey(key)) continue;
    out[key] = value;
  }
  return out;
}

export function paintTransformCssValue(store) {
  const value =
    store.transform ?? store.webkitTransform ?? store.MozTransform ?? store.msTransform;
  return value == null ? "" : String(value);
}

export function syncPaintTransform(nid, store) {
  try {
    hostCall("setPaintTransform", [nid, paintTransformCssValue(store)]);
  } catch (_err) {}
}

export function createStyleProxy(nid) {
  const store = Object.create(null);
  // Registered up front so a Vue patch merges with an empty proxy layer rather
  // than with one that only appears after the first imperative write.
  layersFor(nid).proxy = store;
  const markDirty = () => queueStyleFlush(nid, store);
  return new Proxy(store, {
    get(target, prop) {
      if (prop === "setProperty") {
        return (name, value) => {
          target[name] = value;
          if (isPaintOnlyStyleKey(name)) syncPaintTransform(nid, target);
          else markDirty();
        };
      }
      if (prop === "removeProperty") {
        return (name) => {
          delete target[name];
          if (isPaintOnlyStyleKey(name)) syncPaintTransform(nid, target);
          else markDirty();
        };
      }
      if (prop === "cssText") {
        return Object.entries(target)
          .map(([k, v]) => `${k}: ${v}`)
          .join("; ");
      }
      return target[prop];
    },
    set(target, prop, value) {
      if (prop === "cssText") {
        for (const k of Object.keys(target)) delete target[k];
        Object.assign(target, parseCssText(value));
        markDirty();
        syncPaintTransform(nid, target);
        return true;
      }
      target[prop] = value;
      // FLIP / Vue TransitionGroup translate is paint-only: Scene transform,
      // not LayoutBox, not a style recascade.
      if (isPaintOnlyStyleKey(prop)) syncPaintTransform(nid, target);
      else markDirty();
      return true;
    },
  });
}
