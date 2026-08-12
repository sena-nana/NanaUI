/**
 * Nana custom renderer hostOps — **L1 + L2** entry into the Nana Style Model.
 *
 * - L1: `createElement` + class/inline style → Rust `css_map` / `widget_map`
 * - L2: `createWidget` / `nana-*` semantic props → same MessageBridge (skip CSS)
 * Both share one forest; draw path is Nana iced-view only (no WebView paint).
 *
 * Also enhances host nodes with Element-like stubs so @lilia/ui template refs
 * (getBoundingClientRect, style, classList, dataset, …) do not throw.
 */
import { createRenderer } from "@vue/runtime-core";
import { defineLayoutMetrics, hostCall, layoutRect, scrollNodeIntoView } from "./layoutMetrics.js";

export { hostCall } from "./layoutMetrics.js";

/** nid:event → [{ listener, capture, once, passive }] — multi-listener + options subset. */
const listeners = new Map();
/** Stable host-node identity so parentNode/nextSibling/=== comparisons stay useful. */
const nodeCache = new Map();

function listenerKey(nid, event) {
  return `${nid}:${String(event).toLowerCase()}`;
}

function isOn(key) {
  // Match @vue/shared: on* where the 3rd char is NOT a-z (onClick yes, onclick no).
  return (
    typeof key === "string" &&
    key.charCodeAt(0) === 111 &&
    key.charCodeAt(1) === 110 &&
    key.length > 2 &&
    (key.charCodeAt(2) > 122 || key.charCodeAt(2) < 97)
  );
}

/** Vue v-model listeners (`onUpdate:xxx`) are props, not DOM events. */
function isModelListener(key) {
  return typeof key === "string" && key.startsWith("onUpdate:");
}

const EVENT_OPTIONS_RE = /(Once|Passive|Capture)$/;

function normalizeListenerOptions(options) {
  if (options === true) return { capture: true, once: false, passive: false };
  if (options && typeof options === "object") {
    return {
      capture: !!options.capture,
      once: !!options.once,
      passive: !!options.passive,
    };
  }
  return { capture: false, once: false, passive: false };
}

/** Parse Vue `onClickCapture` / `onClickOnce` → `{ name, options }` (runtime-dom subset). */
function parseEventName(rawName) {
  let name = String(rawName);
  let options;
  let m;
  while ((m = name.match(EVENT_OPTIONS_RE)) && !/^on:?(?:Once|Passive|Capture)$/.test(name)) {
    name = name.slice(0, name.length - m[1].length);
    if (!options) options = {};
    options[m[1].toLowerCase()] = true;
  }
  let event;
  if (name.startsWith("on:") || name.startsWith("on")) {
    const body = name.startsWith("on:") ? name.slice(3) : name.slice(2);
    event = body.replace(/^[A-Z]/, (c) => c.toLowerCase()).toLowerCase();
  } else {
    event = name.toLowerCase();
  }
  return { name: event, options };
}

function normalizeHandler(next) {
  if (typeof next === "function") return next;
  if (Array.isArray(next)) {
    const fns = next.filter((fn) => typeof fn === "function");
    if (!fns.length) return null;
    return (evt) => {
      for (const fn of fns) {
        if (evt && (evt._stopped || evt._immediateStopped)) break;
        fn(evt);
      }
    };
  }
  return null;
}

function isListenerObject(listener) {
  return (
    typeof listener === "function" ||
    (listener != null && typeof listener.handleEvent === "function")
  );
}

function addNanaListener(nid, type, listener, options) {
  if (!isListenerObject(listener)) return;
  const opts = normalizeListenerOptions(options);
  const key = listenerKey(nid, type);
  let list = listeners.get(key);
  if (!list) {
    list = [];
    listeners.set(key, list);
  }
  for (const entry of list) {
    if (entry.listener === listener && entry.capture === opts.capture) return;
  }
  list.push({
    listener,
    capture: opts.capture,
    once: opts.once,
    passive: opts.passive,
  });
}

function removeNanaListener(nid, type, listener, options) {
  const capture = normalizeListenerOptions(options).capture;
  const key = listenerKey(nid, type);
  const list = listeners.get(key);
  if (!list) return;
  const next = list.filter((entry) => !(entry.listener === listener && entry.capture === capture));
  if (next.length) listeners.set(key, next);
  else listeners.delete(key);
}

function invokeNanaListenerPhase(nid, type, event, capture) {
  const list = listeners.get(listenerKey(nid, type));
  if (!list || !list.length) return;
  const snapshot = list.slice();
  for (const entry of snapshot) {
    if (entry.capture !== !!capture) continue;
    if (event && event._immediateStopped) break;
    try {
      if (event) {
        event.currentTarget = event.target;
        event.eventPhase = capture ? 1 : 2;
      }
      if (typeof entry.listener === "function") {
        entry.listener.call(event && event.target, event);
      } else {
        entry.listener.handleEvent(event);
      }
    } catch (_err) {}
    if (entry.once) removeNanaListener(nid, type, entry.listener, entry.capture);
  }
}

function invokeGlobalPhase(target, type, event, capture) {
  if (!target) return;
  if (typeof target.__nanaInvokePhase === "function") {
    target.__nanaInvokePhase(type, event, capture);
    return;
  }
  // Fallback: plain EventTarget-like with _listeners (tests / partial shims).
  const bag = target._listeners;
  if (!bag) return;
  const list = bag[String(type)];
  if (!list || !list.length) return;
  const snapshot = list.slice();
  for (const entry of snapshot) {
    const isCapture = typeof entry === "object" && entry != null ? !!entry.capture : false;
    if (isCapture !== !!capture) continue;
    if (event && event._immediateStopped) break;
    try {
      if (event) {
        event.currentTarget = target;
        event.eventPhase = capture ? 1 : 3;
      }
      const listener = typeof entry === "function" ? entry : entry.listener;
      if (typeof listener === "function") listener.call(target, event);
      else if (listener && typeof listener.handleEvent === "function") listener.handleEvent(event);
    } catch (_err) {}
  }
}

function createEventPayload(type, target, detail) {
  return {
    type,
    target,
    currentTarget: target,
    key: detail && detail.key,
    code: detail && detail.code,
    data: detail && detail.data,
    value: detail && detail.value,
    checked: detail && detail.checked,
    deltaX: detail && detail.deltaX,
    deltaY: detail && detail.deltaY,
    bubbles: true,
    cancelable: true,
    defaultPrevented: false,
    eventPhase: 0,
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

/** window capture → document capture → target → document bubble → window bubble. */
function fanOutDocumentWindow(payload, type) {
  const win = globalThis.window;
  const doc = globalThis.document;
  invokeGlobalPhase(win, type, payload, true);
  if (payload._stopped) return;
  invokeGlobalPhase(doc, type, payload, true);
  if (payload._stopped) return;
  invokeGlobalPhase(doc, type, payload, false);
  if (payload._stopped) return;
  invokeGlobalPhase(win, type, payload, false);
}

/** Keys Vue sets as DOM properties (not attributes) on a real Element. */
function shouldSetAsDomProp(key) {
  return (
    key === "innerHTML" ||
    key === "textContent" ||
    key === "value" ||
    key === "checked" ||
    key === "selected" ||
    key === "muted" ||
    key === "multiple" ||
    key === "defaultValue" ||
    key === "indeterminate"
  );
}

/** Subset of SVG tags Lilia/Lucide commonly create. */
const SVG_TAGS = new Set([
  "svg",
  "path",
  "g",
  "circle",
  "rect",
  "line",
  "polyline",
  "polygon",
  "ellipse",
  "text",
  "tspan",
  "defs",
  "clippath",
  "mask",
  "use",
  "symbol",
  "lineargradient",
  "radialgradient",
  "stop",
  "pattern",
  "image",
  "foreignobject",
]);

/** Common SVG presentation / geometry attrs (Vue sets these as attributes). */
const COMMON_SVG_ATTRS = new Set([
  "viewBox",
  "viewbox",
  "fill",
  "stroke",
  "d",
  "cx",
  "cy",
  "r",
  "rx",
  "ry",
  "x",
  "y",
  "x1",
  "x2",
  "y1",
  "y2",
  "points",
  "transform",
  "opacity",
  "stroke-width",
  "strokewidth",
  "stroke-linecap",
  "stroke-linejoin",
  "stroke-dasharray",
  "stroke-dashoffset",
  "fill-opacity",
  "stroke-opacity",
  "fill-rule",
  "clip-path",
  "href",
  "preserveAspectRatio",
  "xmlns",
  "pathLength",
  "width",
  "height",
]);

function isSvgElement(el, namespace) {
  if (namespace === "svg") return true;
  if (el && el.__isSVG) return true;
  const tag = String((el && (el.tag || el.tagName)) || "").toLowerCase();
  return SVG_TAGS.has(tag);
}

function isSvgAttrKey(key) {
  return (
    key.startsWith("xlink:") ||
    key.startsWith("xml:") ||
    COMMON_SVG_ATTRS.has(key) ||
    COMMON_SVG_ATTRS.has(key.toLowerCase())
  );
}

function serializePatchValue(next) {
  if (typeof next === "boolean") return next;
  if (Array.isArray(next)) {
    return next.map((item) =>
      item != null && typeof item === "object" && !Array.isArray(item) ? { ...item } : item,
    );
  }
  if (next != null && typeof next === "object") return { ...next };
  if (next == null) return null;
  return String(next);
}

/**
 * Seed props for host createElement / createWidget.
 * `__nanaHost.call` JSON-encodes args — never pass Vue vnode props / reactive
 * graphs / wrapNodes or circular style objects through raw.
 */
function seedHostProps(vnodeProps) {
  if (!vnodeProps || typeof vnodeProps !== "object") return null;
  const out = {};
  let any = false;
  for (const [rawKey, value] of Object.entries(vnodeProps)) {
    if (rawKey === "key" || rawKey === "ref" || isOn(rawKey) || isModelListener(rawKey)) {
      continue;
    }
    let key = rawKey;
    if (key[0] === "." || key[0] === "^") key = key.slice(1);
    if (value == null) {
      out[key] = null;
      any = true;
      continue;
    }
    const t = typeof value;
    if (t === "string" || t === "number" || t === "boolean") {
      out[key] = value;
      any = true;
      continue;
    }
    if (t !== "object") continue;
    // Skip host nodes / DOM-likes.
    if (typeof value.__nid === "number" || typeof value.nodeType === "number") continue;
    if (Array.isArray(value)) {
      const items = [];
      let ok = true;
      for (const item of value) {
        const it = typeof item;
        if (item == null || it === "string" || it === "number" || it === "boolean") {
          items.push(item);
        } else {
          ok = false;
          break;
        }
      }
      if (ok) {
        out[key] = items;
        any = true;
      }
      continue;
    }
    // Shallow plain object (class object / style bag) — primitives only.
    try {
      const plain = {};
      let plainAny = false;
      for (const [sk, sv] of Object.entries(value)) {
        const st = typeof sv;
        if (sv == null || st === "string" || st === "number" || st === "boolean") {
          plain[sk] = sv;
          plainAny = true;
        }
      }
      if (plainAny) {
        out[key] = plain;
        any = true;
      }
    } catch (_err) {}
  }
  return any ? out : null;
}

function syncClassList(el, classValue) {
  if (!el || !el.classList || typeof el.classList.__replace !== "function") return;
  el.classList.__replace(classValue == null ? "" : String(classValue));
}

/** Resolve kind/tag from host when wrapping an id without local metadata. */
function wrapById(id) {
  const nid = Number(id);
  const cached = nodeCache.get(nid);
  if (cached) return cached;
  let kind = "element";
  let tag = null;
  try {
    const k = hostCall("nodeKind", [nid]);
    if (k === "text" || k === "comment") kind = String(k);
    else if (k === "element" || k == null) {
      try {
        tag = hostCall("elementTag", [nid]);
      } catch (_err) {}
    }
  } catch (_err) {}
  return wrapNode(nid, kind, tag);
}

export function wrapNode(id, kind, tag) {
  const nid = Number(id);
  if (!Number.isFinite(nid)) return null;
  const cached = nodeCache.get(nid);
  if (cached) {
    if (kind && cached.__kind === "element" && kind !== "element") {
      // keep first classification
    } else if (kind) {
      cached.__kind = kind;
      cached.nodeType = kind === "text" ? 3 : kind === "comment" ? 8 : 1;
    }
    if (tag && !cached.tag) {
      cached.tag = tag;
      cached.tagName = String(tag).toUpperCase();
      cached.nodeName = cached.tagName;
    }
    return cached;
  }
  const resolvedKind = kind || "element";
  const node = {
    __nid: nid,
    __kind: resolvedKind,
    tagName: tag ? String(tag).toUpperCase() : null,
    nodeName: tag
      ? String(tag).toUpperCase()
      : resolvedKind === "text"
        ? "#text"
        : resolvedKind === "comment"
          ? "#comment"
          : "#node",
    tag: tag || null,
    nodeType: resolvedKind === "text" ? 3 : resolvedKind === "comment" ? 8 : 1,
    ownerDocument: globalThis.document || null,
    style: createStyleProxy(nid),
    dataset: createDatasetProxy(nid),
    attributes: {},
    className: "",
    __isSVG: false,
    // classList assigned after object create so sync can touch attributes.
    getBoundingClientRect() {
      return layoutRect(nid);
    },
    getClientRects() {
      const r = layoutRect(nid);
      return r.width > 0 || r.height > 0 ? [r] : [];
    },
    focus() {
      try {
        hostCall("setFocus", [nid]);
      } catch (_err) {}
      const payload = createEventPayload("focus", this, null);
      invokeNanaListenerPhase(nid, "focus", payload, true);
      if (!payload._immediateStopped) invokeNanaListenerPhase(nid, "focus", payload, false);
    },
    blur() {
      try {
        hostCall("clearFocus", []);
      } catch (_err) {}
    },
    get value() {
      try {
        const v = hostCall("getAttribute", [nid, "value"]);
        return v == null ? "" : String(v);
      } catch (_err) {
        return this.attributes.value || "";
      }
    },
    set value(next) {
      const s = next == null ? "" : String(next);
      this.attributes.value = s;
      try {
        hostCall("patchProp", [nid, "value", s]);
      } catch (_err) {}
    },
    click() {
      if (typeof globalThis.__nanaFireEvent === "function") {
        globalThis.__nanaFireEvent(nid, "click", {});
        return;
      }
      const payload = createEventPayload("click", this, null);
      invokeNanaListenerPhase(nid, "click", payload, true);
      if (!payload._immediateStopped) invokeNanaListenerPhase(nid, "click", payload, false);
    },
    addEventListener(type, listener, options) {
      addNanaListener(nid, type, listener, options);
      try {
        hostCall("patchProp", [nid, "on" + String(type), true]);
      } catch (_err) {}
    },
    removeEventListener(type, listener, options) {
      removeNanaListener(nid, type, listener, options);
    },
    dispatchEvent(event) {
      if (!event || event.type == null) return true;
      const type = String(event.type);
      if (typeof event.stopPropagation !== "function") {
        event._stopped = false;
        event._immediateStopped = false;
        event.stopPropagation = function () {
          this._stopped = true;
        };
        event.stopImmediatePropagation = function () {
          this._stopped = true;
          this._immediateStopped = true;
        };
      }
      if (event.target == null) event.target = this;
      invokeNanaListenerPhase(nid, type, event, true);
      if (!event._immediateStopped) invokeNanaListenerPhase(nid, type, event, false);
      return !event.defaultPrevented;
    },
    setAttribute(name, value) {
      this.attributes[name] = String(value);
      try {
        hostCall("patchProp", [nid, String(name), String(value)]);
      } catch (_err) {}
    },
    getAttribute(name) {
      if (name in this.attributes) return this.attributes[name];
      try {
        const v = hostCall("getAttribute", [nid, String(name)]);
        return v == null ? null : String(v);
      } catch (_err) {
        return null;
      }
    },
    removeAttribute(name) {
      delete this.attributes[name];
      try {
        hostCall("patchProp", [nid, String(name), null]);
      } catch (_err) {}
    },
    hasAttribute(name) {
      return this.getAttribute(name) != null;
    },
    appendChild(child) {
      try {
        hostCall("insert", [nodeId(child), nid, null]);
      } catch (_err) {}
      return child;
    },
    removeChild(child) {
      try {
        hostCall("remove", [nodeId(child)]);
      } catch (_err) {}
      return child;
    },
    insertBefore(child, anchor) {
      try {
        hostCall("insert", [nodeId(child), nid, nodeId(anchor)]);
      } catch (_err) {}
      return child;
    },
    querySelector(sel) {
      try {
        const id = hostCall("querySelector", [String(sel ?? "")]);
        return id == null ? null : wrapNode(id, "element", null);
      } catch (_err) {
        return null;
      }
    },
    querySelectorAll(sel) {
      try {
        const ids = hostCall("querySelectorAll", [String(sel ?? "")]) || [];
        return Array.from(ids, (id) => wrapNode(id, "element", null));
      } catch (_err) {
        const one = this.querySelector(sel);
        return one ? [one] : [];
      }
    },
    closest(sel) {
      try {
        const id = hostCall("closest", [nid, String(sel ?? "")]);
        return id == null ? null : wrapNode(id, "element", null);
      } catch (_err) {
        return null;
      }
    },
    matches(sel) {
      try {
        const id = hostCall("closest", [nid, String(sel ?? "")]);
        return id != null && Number(id) === nid;
      } catch (_err) {
        return false;
      }
    },
    contains(other) {
      // DOM Node.contains — required by LiliaUI overlays (click-outside).
      const otherId = nodeId(other);
      if (otherId == null) return false;
      if (otherId === nid) return true;
      try {
        return hostCall("contains", [nid, otherId]) === true;
      } catch (_err) {
        return false;
      }
    },
    scrollIntoView(arg) {
      scrollNodeIntoView(nid, arg);
    },
    get rootNode() {
      return this;
    },
  };
  node.classList = createClassList(nid, node);
  const proto =
    (globalThis.HTMLElement && globalThis.HTMLElement.prototype) ||
    (globalThis.Element && globalThis.Element.prototype) ||
    (globalThis.Node && globalThis.Node.prototype) ||
    null;
  if (proto) {
    Object.setPrototypeOf(node, proto);
  }
  // Live tree navigation — always reflect Rust NanaTreeDocument (not a JS shadow).
  Object.defineProperty(node, "parentNode", {
    configurable: true,
    enumerable: true,
    get() {
      try {
        const pid = hostCall("parentNode", [nid]);
        return pid == null ? null : wrapById(pid);
      } catch (_err) {
        return null;
      }
    },
  });
  Object.defineProperty(node, "parentElement", {
    configurable: true,
    enumerable: true,
    get() {
      const p = this.parentNode;
      return p && p.nodeType === 1 ? p : null;
    },
  });
  Object.defineProperty(node, "childNodes", {
    configurable: true,
    enumerable: true,
    get() {
      try {
        const ids = hostCall("childNodes", [nid]) || [];
        return Array.from(ids, (cid) => wrapById(cid));
      } catch (_err) {
        return [];
      }
    },
  });
  Object.defineProperty(node, "children", {
    configurable: true,
    enumerable: true,
    get() {
      return this.childNodes.filter((c) => c && c.nodeType === 1);
    },
  });
  Object.defineProperty(node, "firstChild", {
    configurable: true,
    enumerable: true,
    get() {
      try {
        const cid = hostCall("firstChild", [nid]);
        return cid == null ? null : wrapById(cid);
      } catch (_err) {
        return null;
      }
    },
  });
  Object.defineProperty(node, "lastChild", {
    configurable: true,
    enumerable: true,
    get() {
      const kids = this.childNodes;
      return kids.length ? kids[kids.length - 1] : null;
    },
  });
  Object.defineProperty(node, "isConnected", {
    configurable: true,
    enumerable: true,
    get() {
      try {
        const html = hostCall("querySelector", ["html"]);
        if (html == null) return false;
        return hostCall("contains", [html, nid]) === true;
      } catch (_err) {
        return false;
      }
    },
  });
  // Vue patchProp(domProps) + Lucide/templates may set these directly.
  Object.defineProperty(node, "innerHTML", {
    get() {
      return this.attributes.innerHTML || "";
    },
    set(v) {
      const s = v == null ? "" : String(v);
      this.attributes.innerHTML = s;
      try {
        hostCall("patchProp", [nid, "innerHTML", s]);
      } catch (_err) {
        try {
          hostCall("setElementText", [nid, s]);
        } catch (_err2) {}
      }
    },
    configurable: true,
  });
  Object.defineProperty(node, "textContent", {
    get() {
      return this.attributes.textContent || "";
    },
    set(v) {
      const s = v == null ? "" : String(v);
      this.attributes.textContent = s;
      try {
        if (node.__kind === "text") hostCall("setText", [nid, s]);
        else hostCall("patchProp", [nid, "textContent", s]);
      } catch (_err) {
        try {
          if (node.__kind === "text") hostCall("setText", [nid, s]);
          else hostCall("setElementText", [nid, s]);
        } catch (_err2) {}
      }
    },
    configurable: true,
  });
  Object.defineProperty(node, "className", {
    get() {
      return this.attributes.class || this.classList.value || "";
    },
    set(v) {
      const s = v == null ? "" : String(v);
      this.attributes.class = s;
      syncClassList(this, s);
      try {
        hostCall("patchProp", [nid, "class", s]);
      } catch (_err) {}
    },
    configurable: true,
  });
  // Keep the object-literal `value` / `checked` accessors above — they read
  // through host `getAttribute` so Iced→bridge→tree patches stay visible.
  // Do not redefine them with attributes-only stubs.
  defineLayoutMetrics(node, nid);
  nodeCache.set(nid, node);
  return node;
}

function linkChild(_parent, _child, _anchor) {
  // Tree links live in Rust NanaTreeDocument; wrapNode getters read via host.
}

function unlinkChild(_child) {
  // Detach is host `remove`; keep wrapNode cache for DOM identity.
}

function parseCssText(cssText) {
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

function createStyleProxy(nid) {
  const store = Object.create(null);
  const flush = () => {
    try {
      hostCall("patchProp", [nid, "style", { ...store }]);
    } catch (_err) {}
  };
  return new Proxy(store, {
    get(target, prop) {
      if (prop === "setProperty") {
        return (name, value) => {
          target[name] = value;
          flush();
        };
      }
      if (prop === "removeProperty") {
        return (name) => {
          delete target[name];
          flush();
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
        flush();
        return true;
      }
      target[prop] = value;
      flush();
      return true;
    },
  });
}

function camelToDataAttr(prop) {
  return (
    "data-" +
    String(prop)
      .replace(/[A-Z]/g, (m) => "-" + m.toLowerCase())
      .replace(/^-/, "")
  );
}

function createDatasetProxy(nid) {
  const store = Object.create(null);
  return new Proxy(store, {
    set(target, prop, value) {
      if (typeof prop === "symbol") return false;
      target[prop] = String(value);
      try {
        hostCall("patchProp", [nid, camelToDataAttr(prop), String(value)]);
      } catch (_err) {}
      return true;
    },
    get(target, prop) {
      if (typeof prop === "symbol") return target[prop];
      if (prop in target) return target[prop];
      try {
        const v = hostCall("getAttribute", [nid, camelToDataAttr(prop)]);
        if (v != null) {
          target[prop] = String(v);
          return target[prop];
        }
      } catch (_err) {}
      return undefined;
    },
    has(target, prop) {
      if (prop in target) return true;
      try {
        return hostCall("getAttribute", [nid, camelToDataAttr(prop)]) != null;
      } catch (_err) {
        return false;
      }
    },
  });
}

function createClassList(nid, el) {
  const set = new Set();
  const writeLocal = () => {
    const joined = [...set].join(" ");
    if (el) {
      el.attributes = el.attributes || {};
      if (joined) el.attributes.class = joined;
      else delete el.attributes.class;
    }
    return joined;
  };
  const sync = () => {
    const joined = writeLocal();
    try {
      hostCall("patchProp", [nid, "class", joined || null]);
    } catch (_err) {}
  };
  return {
    add(...tokens) {
      tokens.forEach((t) => {
        const s = String(t).trim();
        if (s) set.add(s);
      });
      sync();
    },
    remove(...tokens) {
      tokens.forEach((t) => set.delete(String(t)));
      sync();
    },
    toggle(token, force) {
      const t = String(token);
      if (force === true) set.add(t);
      else if (force === false) set.delete(t);
      else if (set.has(t)) set.delete(t);
      else set.add(t);
      sync();
      return set.has(t);
    },
    contains(token) {
      return set.has(String(token));
    },
    /** Replace tokens from a Vue `class` patch without re-entering patchProp loops. */
    __replace(classValue) {
      set.clear();
      String(classValue || "")
        .split(/\s+/)
        .filter(Boolean)
        .forEach((t) => set.add(t));
      writeLocal();
    },
    get value() {
      return [...set].join(" ");
    },
    get length() {
      return set.size;
    },
  };
}

export function nodeId(node) {
  if (node == null) return null;
  if (typeof node === "number") return node;
  if (typeof node.__nid === "number") return node.__nid;
  return null;
}

/**
 * Create a semantic Nana widget (button / switch / text / …) without DOM paint.
 * Returns a host node whose id is tracked by Rust `MessageBridge`.
 */
export function createWidget(kind, props) {
  const id = hostCall("createWidget", [String(kind), props && typeof props === "object" ? { ...props } : {}]);
  return wrapNode(id, "element", `nana-${String(kind).replace(/^nana-/i, "")}`);
}

export const hostOps = {
  /**
   * Mirrors `@vue/runtime-dom` patchProp branching:
   * class → style → on* (skip onUpdate:) → .prop/^attr → domProps → attrs.
   * SVG: prefer attributes (incl. xlink: / viewBox); force `.` still uses prop path.
   */
  patchProp(el, key, prev, next, namespace, _parentComponent) {
    const nid = nodeId(el);
    if (nid == null || typeof key !== "string") return;

    if (key === "class" || key === "className") {
      const value = next == null ? null : String(next);
      if (el) {
        el.attributes.class = value == null ? undefined : value;
        if (value == null) delete el.attributes.class;
        syncClassList(el, value);
      }
      hostCall("patchProp", [nid, "class", value]);
      return;
    }

    if (key === "style") {
      if (next == null) {
        hostCall("patchProp", [nid, "style", null]);
        return;
      }
      if (typeof next === "string") {
        hostCall("patchProp", [nid, "style", next]);
        return;
      }
      if (typeof next === "object") {
        const cleaned = {};
        for (const [k, v] of Object.entries(next)) {
          if (v != null && v !== "") cleaned[k] = Array.isArray(v) ? v[v.length - 1] : v;
        }
        hostCall("patchProp", [nid, "style", cleaned]);
      }
      return;
    }

    if (isOn(key)) {
      if (isModelListener(key)) {
        // v-model update listeners are component props, not host events.
        return;
      }
      const { name: event, options } = parseEventName(key);
      const invokers = el.__nanaVei || (el.__nanaVei = Object.create(null));
      const existing = invokers[key];
      if (next == null || next === false) {
        if (existing) {
          removeNanaListener(nid, event, existing, options);
          if (event === "press") removeNanaListener(nid, "click", existing, options);
          if (event === "click") removeNanaListener(nid, "press", existing, options);
          invokers[key] = undefined;
        }
        hostCall("patchProp", [nid, key, null]);
        return;
      }
      const handler = normalizeHandler(next);
      if (!handler) return;
      if (existing) {
        // Vue patchEvent: update invoker.value; keep addEventListener peers.
        existing.value = handler;
      } else {
        const invoker = function (evt) {
          const fn = invoker.value;
          if (typeof fn === "function") fn(evt);
        };
        invoker.value = handler;
        invokers[key] = invoker;
        addNanaListener(nid, event, invoker, options);
        // Alias press ↔ click for NanaButton / Iced bridge.
        if (event === "press") addNanaListener(nid, "click", invoker, options);
        if (event === "click") addNanaListener(nid, "press", invoker, options);
      }
      hostCall("patchProp", [nid, key, true]);
      return;
    }

    let propKey = key;
    let forceProp;
    if (propKey[0] === ".") {
      propKey = propKey.slice(1);
      forceProp = true;
    } else if (propKey[0] === "^") {
      propKey = propKey.slice(1);
      forceProp = false;
    }

    const isSVG = isSvgElement(el, namespace);
    // Vue shouldSetAsProp: SVG → false except innerHTML/textContent.
    const asDomProp =
      forceProp === true ||
      (forceProp !== false &&
        (propKey === "innerHTML" ||
          propKey === "textContent" ||
          (!isSVG && !isSvgAttrKey(propKey) && shouldSetAsDomProp(propKey))));

    if (asDomProp) {
      if (propKey === "innerHTML" || propKey === "textContent") {
        const text = next == null ? "" : String(next);
        if (el) el.attributes[propKey] = text;
        hostCall("patchProp", [nid, propKey, text]);
        return;
      }
      const value = serializePatchValue(next);
      if (el) {
        if (propKey === "value") {
          el.attributes.value = value == null ? "" : String(value);
        } else if (propKey === "checked" || propKey === "selected") {
          if (value === true || value === "") el.attributes[propKey] = "";
          else if (value == null || value === false) delete el.attributes[propKey];
          else el.attributes[propKey] = String(value);
        }
      }
      hostCall("patchProp", [nid, propKey, value]);
      return;
    }

    // Attribute path (incl. `^attr`, SVG attrs, boolean attrs).
    const value = serializePatchValue(next);
    if (el) {
      el.attributes = el.attributes || {};
      if (value == null || value === false) {
        delete el.attributes[propKey];
      } else if (value === true) {
        el.attributes[propKey] = "";
      } else {
        el.attributes[propKey] = String(value);
      }
    }
    hostCall("patchProp", [nid, propKey, value]);
  },
  insert(child, parent, anchor) {
    const c = child && typeof child === "object" ? child : child != null ? wrapNode(child) : null;
    const p =
      parent && typeof parent === "object" ? parent : parent != null ? wrapNode(parent) : null;
    const a =
      anchor && typeof anchor === "object" ? anchor : anchor != null ? wrapNode(anchor) : null;
    const cid = nodeId(c);
    const pid = nodeId(p);
    // Never call host insert with a null parent — Rust used to detach-then-fail
    // and orphan sidebar footer slots during remount / Teleport.
    if (cid == null || pid == null) return;
    hostCall("insert", [cid, pid, nodeId(a)]);
    linkChild(p, c, a);
  },
  remove(child) {
    const nid = nodeId(child);
    unlinkChild(child);
    for (const key of [...listeners.keys()]) {
      if (key.startsWith(`${nid}:`)) listeners.delete(key);
    }
    hostCall("remove", [nid]);
    // Keep nodeCache entry so detached refs retain === identity (DOM contract).
  },
  createElement(tag, namespace, isCustomizedBuiltIn, vnodeProps) {
    const tagName = String(tag);
    const lower = tagName.toLowerCase();
    const ns =
      namespace == null || namespace === false
        ? null
        : namespace === true
          ? "svg"
          : String(namespace);
    const is =
      isCustomizedBuiltIn == null || isCustomizedBuiltIn === false
        ? null
        : String(isCustomizedBuiltIn);
    const seed = seedHostProps(vnodeProps);
    // Prefer createWidget for nana-* semantic controls so props seed the bridge.
    if (lower.startsWith("nana-") && lower !== "nana-gpu") {
      const kind = lower.slice("nana-".length);
      const id = hostCall("createWidget", [kind, seed || {}]);
      const node = wrapNode(id, "element", tagName);
      node.__isSVG = false;
      return node;
    }
    const id = hostCall("createElement", [tagName, ns, is, seed]);
    const node = wrapNode(id, "element", tagName);
    node.__isSVG = ns === "svg" || SVG_TAGS.has(lower);
    if (ns) node.__namespace = ns;
    if (is) {
      try {
        node.setAttribute("is", is);
      } catch (_err) {}
    }
    if (lower === "nana-gpu") {
      let slot = "default";
      if (vnodeProps && typeof vnodeProps === "object") {
        const raw =
          vnodeProps["data-slot"] ??
          vnodeProps.dataSlot ??
          (vnodeProps.dataset && vnodeProps.dataset.slot);
        if (raw != null && String(raw).length) {
          slot = String(raw);
        }
      }
      hostCall("setGpuSlot", [id, slot]);
    }
    return node;
  },
  createText(text) {
    return wrapNode(hostCall("createText", [String(text)]), "text", null);
  },
  createComment(text) {
    return wrapNode(hostCall("createComment", [String(text ?? "")]), "comment", null);
  },
  setText(node, text) {
    hostCall("setText", [nodeId(node), String(text)]);
  },
  setElementText(el, text) {
    hostCall("setElementText", [nodeId(el), String(text)]);
  },
  parentNode(node) {
    const id = hostCall("parentNode", [nodeId(node)]);
    return id == null ? null : wrapById(id);
  },
  nextSibling(node) {
    const id = hostCall("nextSibling", [nodeId(node)]);
    return id == null ? null : wrapById(id);
  },
  querySelector(sel) {
    const raw = String(sel ?? "");
    const id = hostCall("querySelector", [raw]);
    if (id == null) return null;
    // Teleport `to="body"` / `html` — keep stable wrapNode + tag metadata.
    const lower = raw.trim().toLowerCase();
    const tag = lower === "body" || lower === "html" ? lower : null;
    return wrapNode(id, "element", tag);
  },
  querySelectorAll(sel) {
    const raw = String(sel ?? "");
    const ids = hostCall("querySelectorAll", [raw]) || [];
    const lower = raw.trim().toLowerCase();
    const tag = lower === "body" || lower === "html" ? lower : null;
    return Array.from(ids, (id) => wrapNode(id, "element", tag));
  },
  cloneNode(node) {
    const id = hostCall("cloneNode", [nodeId(node), true]);
    return wrapNode(id, node?.__kind || "element", node?.tag || null);
  },
  insertStaticContent(content, parent, anchor, namespace, start, end) {
    let ns = namespace;
    if (namespace === true) ns = "svg";
    else if (namespace === false || namespace == null || namespace === "") ns = null;
    else ns = String(namespace);
    const pair = hostCall("insertStaticContent", [
      String(content ?? ""),
      nodeId(parent),
      nodeId(anchor),
      ns,
      nodeId(start),
      nodeId(end),
    ]);
    const first = wrapNode(pair[0], "element", null);
    const last = wrapNode(pair[1], "element", null);
    return [first, last];
  },
  setScopeId(el, id) {
    hostCall("setScopeId", [nodeId(el), String(id)]);
  },
};

function scheduleJob(job) {
  if (typeof queueMicrotask === "function") {
    queueMicrotask(job);
  } else {
    Promise.resolve().then(job);
  }
}

export function createNanaApp() {
  const { createApp, render } = createRenderer({
    ...hostOps,
    scheduleJob,
  });
  return { createApp, render, hostOps };
}

export function mountRootHandle() {
  return wrapNode(hostCall("mountRoot", []), "element", "body");
}

export function installEventBridge() {
  globalThis.__nanaWrapNode = wrapNode;

  globalThis.__nanaFireEvent = function __nanaFireEvent(nid, event, detail) {
    const type = String(event);
    const target = wrapNode(nid, "element", null);
    const payload = createEventPayload(type, target, detail);
    const win = globalThis.window;
    const doc = globalThis.document;

    // Capture: window → document → target (+ legacy nid 0)
    invokeGlobalPhase(win, type, payload, true);
    if (!payload._stopped) invokeGlobalPhase(doc, type, payload, true);
    if (!payload._stopped) {
      invokeNanaListenerPhase(nid, type, payload, true);
      if (!payload._immediateStopped) invokeNanaListenerPhase(nid, type, payload, false);
      invokeNanaListenerPhase(0, type, payload, true);
      if (!payload._immediateStopped) invokeNanaListenerPhase(0, type, payload, false);
    }
    // Bubble: document → window (Lilia useDismissableLayer / ContextMenu)
    if (!payload._stopped) invokeGlobalPhase(doc, type, payload, false);
    if (!payload._stopped) invokeGlobalPhase(win, type, payload, false);

    return !payload.defaultPrevented;
  };

  /** Rust → Vue unidirectional theme inject (`VueHost::inject_theme`). */
  globalThis.__nanaApplyTheme = function __nanaApplyTheme(mode) {
    const theme = String(mode || "light").toLowerCase() === "dark" ? "dark" : "light";
    try {
      hostCall("setDocumentTheme", [theme]);
    } catch (_err) {}
    try {
      const el = globalThis.document && globalThis.document.documentElement;
      if (el) {
        if (el.dataset) el.dataset.theme = theme;
        if (el.setAttribute) el.setAttribute("data-theme", theme);
      }
    } catch (_err) {}
    try {
      if (typeof globalThis.__nanaOnTheme === "function") {
        globalThis.__nanaOnTheme(theme);
      }
    } catch (_err) {}
    return theme;
  };
}

installEventBridge();
