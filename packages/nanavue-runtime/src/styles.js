/** Shared style queue; flushing never invalidates node identity or hierarchy. */
import { hostCall } from "./layoutMetrics.js";
import { isPaintOnlyStyleKey } from "./transitionContract.js";
const pendingStyleStores = new Map();
let styleFlushScheduled = false;
export function flushPendingStyles() {
  if (!pendingStyleStores.size) return;
  const batch = [...pendingStyleStores.entries()];
  pendingStyleStores.clear();
  for (const [nid, store] of batch) {
    try {
      hostCall("patchProp", [nid, "style", { ...store }]);
    } catch (_err) {}
  }
}

export function queueStyleFlush(nid, store) {
  pendingStyleStores.set(nid, hostStyleStore(store));
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
