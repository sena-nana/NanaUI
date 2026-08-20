/**
 * Nana custom renderer hostOps — **L1 + L2** entry into the Nana Style Model.
 *
 * - L1: `createElement` + class/inline style → Rust `css_map` / `widget_map`
 * - L2: `createWidget` / `nana-*` semantic props → same MessageBridge (skip CSS)
 * Both share one forest; draw path is Runtime/UiScene via the Scene host
 * (`scene-view`; no Iced widget tree, no WebView paint).
 *
 * Also enhances host nodes with Element-like stubs so @lilia/ui template refs
 * (getBoundingClientRect, style, classList, dataset, …) do not throw.
 */
import { createRenderer } from "@vue/runtime-core";
import { defineLayoutMetrics, hostCall, layoutRect, nanaWindowIdFromNode, scrollNodeIntoView, withNanaWindowContext } from "./layoutMetrics.js";

export { hostCall } from "./layoutMetrics.js";

/** nid:event → [{ listener, capture, once, passive }] — multi-listener + options subset. */
const listeners = new Map();
/** Stable host-node identity so parentNode/nextSibling/=== comparisons stay useful. */
const nodeCache = new Map();

function contextForWindow(windowId) {
  const id = Number(windowId || 0);
  if (id && typeof globalThis.__nanaGetWindowContext === "function") {
    const context = globalThis.__nanaGetWindowContext(id);
    if (context) return context;
  }
  return { window: globalThis.window, document: globalThis.document };
}

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
        const currentTarget = wrapById(nid) || event.target;
        event.currentTarget = currentTarget;
        event.eventPhase = currentTarget === event.target ? 2 : capture ? 1 : 3;
      }
      if (typeof entry.listener === "function") {
        entry.listener.call(event && event.currentTarget, event);
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

function createFileList(files) {
  const list = Array.isArray(files) ? files.slice() : [];
  Object.defineProperty(list, "item", {
    configurable: true,
    enumerable: false,
    value(index) {
      return this[Number(index)] || null;
    },
  });
  return list;
}

function createDataTransfer(files) {
  const items = files.map((file) => ({
    kind: "file",
    type: String(file && file.type ? file.type : ""),
    getAsFile() {
      return file || null;
    },
  }));
  Object.defineProperty(items, "item", {
    configurable: true,
    enumerable: false,
    value(index) {
      return this[Number(index)] || null;
    },
  });
  return {
    files,
    items,
    types: files.length ? ["Files"] : [],
    dropEffect: "copy",
    effectAllowed: "copy",
  };
}

function createEventPayload(type, target, detail) {
  const source = detail && typeof detail === "object" ? detail : {};
  const files = createFileList(source.files);
  const payload = {
    type,
    target,
    currentTarget: target,
    detail: source,
    key: source.key,
    code: source.code,
    data: source.data,
    value: source.value,
    checked: source.checked,
    inputType: source.inputType,
    isComposing: !!source.isComposing,
    repeat: !!source.repeat,
    location: Number(source.location || 0),
    clientX: Number(source.clientX || 0),
    clientY: Number(source.clientY || 0),
    x: Number(source.x ?? source.clientX ?? 0),
    y: Number(source.y ?? source.clientY ?? 0),
    offsetX: Number(source.offsetX ?? source.clientX ?? 0),
    offsetY: Number(source.offsetY ?? source.clientY ?? 0),
    screenX: Number(source.screenX || 0),
    screenY: Number(source.screenY || 0),
    button: Number(source.button ?? 0),
    buttons: Number(source.buttons ?? 0),
    pressure: Number(source.pressure || 0),
    tangentialPressure: Number(source.tangentialPressure || 0),
    tiltX: Number(source.tiltX || 0),
    tiltY: Number(source.tiltY || 0),
    twist: Number(source.twist || 0),
    pointerId: Number(source.pointerId || 0),
    pointerType: source.pointerType || "",
    isPrimary: !!source.isPrimary,
    relatedTarget:
      source.relatedTarget == null ? null : wrapById(Number(source.relatedTarget)),
    altKey: !!source.altKey,
    ctrlKey: !!source.ctrlKey,
    metaKey: !!source.metaKey,
    shiftKey: !!source.shiftKey,
    deltaX: Number(source.deltaX || 0),
    deltaY: Number(source.deltaY || 0),
    deltaMode: Number(source.deltaMode || 0),
    files,
    dataTransfer: source.dataTransfer || createDataTransfer(files),
    bubbles: !/^(pointerenter|pointerleave|mouseenter|mouseleave|focus|blur)$/.test(type),
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
  for (const key of Object.keys(source)) {
    if (!Object.prototype.hasOwnProperty.call(payload, key)) {
      payload[key] = source[key];
    }
  }
  return payload;
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
  const primitiveType = typeof next;
  // Custom-renderer props are structured values, not HTML attribute text.
  // Keep numeric/boolean/bigint identity for registered Rust components; the
  // local Element facade still stringifies values when mirroring attributes.
  if (
    primitiveType === "string" ||
    primitiveType === "number" ||
    primitiveType === "boolean" ||
    primitiveType === "bigint"
  ) return next;
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
  const windowId = nanaWindowIdFromNode(nid);
  const windowContext = contextForWindow(windowId);
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
    ownerDocument: windowContext.document || globalThis.document || null,
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
        withNanaWindowContext(windowId, () => hostCall("clearFocus", []));
      } catch (_err) {}
    },
    setPointerCapture(pointerId) {
      hostCall("setPointerCapture", [nid, Number(pointerId)]);
    },
    releasePointerCapture(pointerId) {
      return !!hostCall("releasePointerCapture", [nid, Number(pointerId)]);
    },
    hasPointerCapture(pointerId) {
      return !!hostCall("hasPointerCapture", [nid, Number(pointerId)]);
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
        const id = withNanaWindowContext(windowId, () =>
          hostCall("querySelector", [String(sel ?? "")]),
        );
        return id == null ? null : wrapNode(id, "element", null);
      } catch (_err) {
        return null;
      }
    },
    querySelectorAll(sel) {
      try {
        const ids =
          withNanaWindowContext(windowId, () =>
            hostCall("querySelectorAll", [String(sel ?? "")]),
          ) || [];
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
        const html = withNanaWindowContext(windowId, () => hostCall("querySelector", ["html"]));
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
      for (const child of Array.from(this.childNodes || [])) releaseNodeResources(child);
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
      for (const child of Array.from(this.childNodes || [])) releaseNodeResources(child);
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
  // through host `getAttribute` so Runtime/bridge→tree patches stay visible.
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

function isNanaGpuElement(node) {
  return String(node && (node.tag || node.tagName || "")).toLowerCase() === "nana-gpu";
}

/** Resolve a NanaTextureHandle or an explicit slot without serializing it. */
function gpuSlotFromSource(source, fallback = "default") {
  if (source == null || source === false) return fallback;
  if (typeof source === "string" || typeof source === "number" || typeof source === "bigint") {
    const slot = String(source);
    return slot.length ? slot : fallback;
  }
  if (typeof source === "object") {
    const explicit = source.slot ?? source["data-slot"] ?? source.dataSlot;
    if (explicit != null && String(explicit).length) return String(explicit);
    if (source.id != null) {
      return `texture:${String(source.id)}:${Number(source.generation || 0)}`;
    }
  }
  return fallback;
}

function setGpuSource(node, source) {
  const nid = nodeId(node);
  if (nid == null) return;
  const slot = gpuSlotFromSource(source);
  if (node) {
    node.__nanaGpuSource = source;
    node.attributes = node.attributes || {};
    node.attributes["data-nana-gpu"] = slot;
  }
  hostCall("setGpuSlot", [nid, slot]);
}

function imageResourceId(source) {
  if (source == null) return null;
  const resource = source.__nanaResource || source.__nanaCanvasResource || source;
  return resource && resource.id != null ? resource.id : null;
}

function bindDecodedImage(el, nid, source) {
  const id = imageResourceId(source);
  if (id == null) return false;
  el.__nanaImageSource = source;
  el.attributes = el.attributes || {};
  el.attributes["data-nana-image"] = String(id);
  hostCall("patchProp", [nid, "data-nana-image", String(id)]);
  return true;
}

function releaseWindowNodeHandles(windowId) {
  const id = Number(windowId || 0);
  if (!id) return;
  for (const [nid, node] of [...nodeCache.entries()]) {
    if (nanaWindowIdFromNode(nid) !== id) continue;
    releaseNodeResources(node);
    nodeCache.delete(nid);
    for (const key of [...listeners.keys()]) {
      if (key.startsWith(`${nid}:`)) listeners.delete(key);
    }
  }
}

globalThis.__nanaReleaseWindowNodes = releaseWindowNodeHandles;

function releaseNodeResources(node) {
  if (!node || typeof node !== "object") return;
  const children = Array.from(node.childNodes || []);
  for (const child of children) releaseNodeResources(child);
  if (node.__nanaOwnedImage && typeof node.__nanaOwnedImage.close === "function") {
    node.__nanaOwnedImage.close();
    node.__nanaOwnedImage = null;
  }
  if (node.__nanaOwnsCanvasResource && node.__nanaCanvasResource) {
    hostCall("resourceRelease", [node.__nanaCanvasResource.id]);
    node.__nanaOwnsCanvasResource = false;
    node.__nanaCanvasResource = null;
    node.__nanaResource = null;
  }
}

function bindImageSource(el, nid, source) {
  if (el.__nanaOwnedImage && typeof el.__nanaOwnedImage.close === "function") {
    el.__nanaOwnedImage.close();
  }
  el.__nanaOwnedImage = null;
  el.__nanaImageGeneration = (el.__nanaImageGeneration || 0) + 1;
  const generation = el.__nanaImageGeneration;
  if (source && typeof source === "object" && bindDecodedImage(el, nid, source)) {
    return;
  }
  const href = source == null ? "" : String(source);
  el.attributes = el.attributes || {};
  el.attributes.src = href;
  hostCall("patchProp", [nid, "src", href]);
  if (!href) {
    hostCall("patchProp", [nid, "data-nana-image", ""]);
    return;
  }
  const ImageCtor = globalThis.Image;
  if (typeof ImageCtor !== "function") return;
  const image = new ImageCtor();
  el.__nanaOwnedImage = image;
  el.__nanaPendingImage = image;
  image.onload = function () {
    if (el.__nanaImageGeneration !== generation) return;
    bindDecodedImage(el, nid, image);
    if (typeof el.dispatchEvent === "function") {
      el.dispatchEvent({ type: "load" });
    }
  };
  image.onerror = function () {
    if (el.__nanaImageGeneration !== generation) return;
    if (typeof el.dispatchEvent === "function") {
      el.dispatchEvent({ type: "error" });
    }
  };
  image.src = href;
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

    if (
      isNanaGpuElement(el) &&
      (key === "source" || key === "data-slot" || key === "dataSlot")
    ) {
      setGpuSource(el, next);
      return;
    }
    if (String(el && (el.tag || el.tagName || "")).toLowerCase() === "img" && key === "src") {
      bindImageSource(el, nid, next);
      return;
    }

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
        // Alias press ↔ click for NanaButton / MessageBridge.
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
        for (const child of Array.from((el && el.childNodes) || [])) releaseNodeResources(child);
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
    releaseNodeResources(child);
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
      let source = "default";
      if (vnodeProps && typeof vnodeProps === "object") {
        source =
          vnodeProps.source ??
          vnodeProps["data-slot"] ??
          vnodeProps.dataSlot ??
          (vnodeProps.dataset && vnodeProps.dataset.slot) ??
          source;
      }
      setGpuSource(node, source);
    }
    if (lower === "canvas" && typeof globalThis.__nanaEnhanceCanvas === "function") {
      globalThis.__nanaEnhanceCanvas(node);
    }
    if (lower === "img" && vnodeProps && typeof vnodeProps === "object" && vnodeProps.src != null) {
      bindImageSource(node, id, vnodeProps.src);
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

function ancestorNodeIds(target) {
  const out = [];
  const seen = new Set();
  let current = target && target.parentNode;
  while (current && Number.isFinite(Number(current.__nid)) && !seen.has(Number(current.__nid))) {
    const id = Number(current.__nid);
    seen.add(id);
    out.push(id);
    current = current.parentNode;
  }
  return out;
}

function hostOpsForWindow(windowId) {
  const id = Number(windowId || 0);
  if (!id) return hostOps;
  const scoped = {};
  for (const [name, operation] of Object.entries(hostOps)) {
    scoped[name] =
      typeof operation === "function"
        ? function () {
            const args = arguments;
            return withNanaWindowContext(id, () => operation.apply(hostOps, args));
          }
        : operation;
  }
  return scoped;
}

export function createNanaApp(windowId = 0) {
  const scopedHostOps = hostOpsForWindow(windowId);
  const { createApp, render } = createRenderer({
    ...scopedHostOps,
    scheduleJob,
  });
  const createAppWithDiagnostics = function () {
    const app = createApp.apply(null, arguments);
    if (app && app.config) {
      const priorWarn = app.config.warnHandler;
      const priorError = app.config.errorHandler;
      app.config.warnHandler = function (message, instance, trace) {
        try {
          hostCall("diagnosticReport", [{
            source: "vue.warn",
            level: "warning",
            message: String(message || "Vue warning"),
            stack: trace == null ? null : String(trace),
          }]);
        } catch (_error) {}
        if (typeof priorWarn === "function") priorWarn(message, instance, trace);
      };
      app.config.errorHandler = function (error, instance, info) {
        try {
          hostCall("diagnosticReport", [{
            source: "vue.error",
            level: "error",
            message: String(error && error.message ? error.message : error || "Vue error"),
            stack: error && error.stack ? String(error.stack) : String(info || ""),
          }]);
        } catch (_error) {}
        if (typeof priorError === "function") priorError(error, instance, info);
      };
    }
    return app;
  };
  return { createApp: createAppWithDiagnostics, render, hostOps: scopedHostOps, windowId: Number(windowId || 0) };
}

export function mountRootHandle(windowId = 0) {
  return withNanaWindowContext(windowId, () =>
    wrapNode(hostCall("mountRoot", []), "element", "body"),
  );
}

export function installEventBridge() {
  globalThis.__nanaWrapNode = wrapNode;

  function fireWindowEvent(windowId, nid, event, detail) {
    const type = String(event);
    const context = contextForWindow(windowId);
    const target = withNanaWindowContext(windowId, () => wrapNode(nid, "element", null));
    const payload = createEventPayload(type, target, detail);
    const win = context.window;
    const doc = context.document;
    const ancestors = ancestorNodeIds(target);

    // Capture: window → document → outer ancestors → target.
    invokeGlobalPhase(win, type, payload, true);
    if (!payload._stopped) invokeGlobalPhase(doc, type, payload, true);
    if (!payload._stopped) {
      for (let i = ancestors.length - 1; i >= 0; i--) {
        invokeNanaListenerPhase(ancestors[i], type, payload, true);
        if (payload._stopped) break;
      }
    }
    if (!payload._stopped) {
      invokeNanaListenerPhase(nid, type, payload, true);
      if (!payload._immediateStopped) invokeNanaListenerPhase(nid, type, payload, false);
      invokeNanaListenerPhase(0, type, payload, true);
      if (!payload._immediateStopped) invokeNanaListenerPhase(0, type, payload, false);
    }
    // Bubble: inner ancestors → document → window.
    if (payload.bubbles && !payload._stopped) {
      for (let i = 0; i < ancestors.length; i++) {
        invokeNanaListenerPhase(ancestors[i], type, payload, false);
        if (payload._stopped) break;
      }
    }
    if (payload.bubbles && !payload._stopped) invokeGlobalPhase(doc, type, payload, false);
    if (payload.bubbles && !payload._stopped) invokeGlobalPhase(win, type, payload, false);

    return !payload.defaultPrevented;
  }

  globalThis.__nanaFireEvent = function __nanaFireEvent(nid, event, detail) {
    return fireWindowEvent(0, nid, event, detail);
  };

  globalThis.__nanaFireWindowEvent = function __nanaFireWindowEvent(
    windowId,
    nid,
    event,
    detail,
  ) {
    return fireWindowEvent(Number(windowId || 0), nid, event, detail);
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

  globalThis.__nanaApplyWindowTheme = function __nanaApplyWindowTheme(windowId, mode) {
    const theme = String(mode || "light").toLowerCase() === "dark" ? "dark" : "light";
    const context = contextForWindow(windowId);
    return withNanaWindowContext(windowId, () => {
      try {
        hostCall("setDocumentTheme", [theme]);
      } catch (_err) {}
      try {
        const el = context.document && context.document.documentElement;
        if (el) {
          if (el.dataset) el.dataset.theme = theme;
          if (el.setAttribute) el.setAttribute("data-theme", theme);
        }
      } catch (_err) {}
      return theme;
    });
  };
}

installEventBridge();

const nanaWindowHandles = new Map();

function createWindowHandle(descriptor) {
  const id = Number(descriptor && descriptor.id);
  if (!Number.isFinite(id) || id < 0) throw new TypeError("invalid Nana window descriptor");
  const cached = nanaWindowHandles.get(id);
  if (cached) return cached;
  const width = Number(descriptor.width) || 800;
  const height = Number(descriptor.height) || 600;
  const context =
    typeof globalThis.__nanaCreateWindowContext === "function"
      ? globalThis.__nanaCreateWindowContext(id, width, height, 1)
      : contextForWindow(id);
  const renderer = createNanaApp(id);
  const root = mountRootHandle(id);
  let app = null;
  let resolveReady;
  let rejectReady;
  let resolveClosed;
  let settled = !!descriptor.ready;
  let closedSettled = false;
  const ready = settled
    ? Promise.resolve()
    : new Promise((resolve, reject) => {
        resolveReady = resolve;
        rejectReady = reject;
      });
  const closed = new Promise((resolve) => {
    resolveClosed = resolve;
  });
  const handle = {
    id,
    window: context.window,
    document: context.document,
    root,
    ready,
    closed,
    __resolveReady() {
      if (settled) return;
      settled = true;
      resolveReady();
    },
    __rejectReady(error) {
      if (settled) return;
      settled = true;
      rejectReady(error);
    },
    __resolveClosed(detail) {
      if (closedSettled) return;
      closedSettled = true;
      resolveClosed(detail || { reason: "closed" });
    },
    mount(component, props) {
      if (app && typeof app.unmount === "function") app.unmount();
      app = renderer.createApp(component, props || null);
      return withNanaWindowContext(id, () => app.mount(root));
    },
    render(vnode) {
      return withNanaWindowContext(id, () => renderer.render(vnode, root));
    },
    unmount() {
      if (app && typeof app.unmount === "function") app.unmount();
      else withNanaWindowContext(id, () => renderer.render(null, root));
      app = null;
    },
    close() {
      hostCall("windowClose", [id]);
    },
    focus() {
      hostCall("windowFocus", [id]);
    },
    moveTo(x, y) {
      hostCall("windowMove", [id, Number(x) || 0, Number(y) || 0]);
    },
    setTitle(title) {
      hostCall("windowSetTitle", [id, String(title ?? "")]);
    },
    setBounds(x, y, width, height) {
      hostCall("windowSetBounds", [id, Number(x) || 0, Number(y) || 0, Number(width) || 1, Number(height) || 1]);
    },
    setFullscreen(fullscreen) {
      hostCall("windowSetFullscreen", [id, !!fullscreen]);
    },
    setMinimized(minimized) {
      hostCall("windowSetMinimized", [id, !!minimized]);
    },
    setMaximized(maximized) {
      hostCall("windowSetMaximized", [id, !!maximized]);
    },
    setAlwaysOnTop(alwaysOnTop) {
      hostCall("windowSetAlwaysOnTop", [id, !!alwaysOnTop]);
    },
    geometry() {
      return hostCall("windowGeometry", [id]) || {};
    },
    get scaleFactor() {
      const geometry = hostCall("windowGeometry", [id]) || {};
      return Number(geometry.scaleFactor) || 1;
    },
  };
  nanaWindowHandles.set(id, handle);
  return handle;
}

globalThis.Nana = globalThis.Nana || {};
globalThis.Nana.resources = {
  release(resource) {
    if (resource && typeof resource.close === "function") {
      resource.close();
      return true;
    }
    const handle = resource && resource.__nanaResource ? resource.__nanaResource : resource;
    if (!handle || handle.id == null) return false;
    return Boolean(hostCall("resourceRelease", [handle.id]));
  },
};

const nanaNativeComponentErrorListeners = new Set();
globalThis.Nana.components = {
  list() {
    return hostCall("componentList", []);
  },
  call(element, command, args) {
    const id = nodeId(element);
    if (id == null) {
      return Promise.reject(new TypeError("Nana.components.call requires a mounted Nana element"));
    }
    if (!globalThis.Nana.host || typeof globalThis.Nana.host.invoke !== "function") {
      return Promise.reject(new Error("Nana.host.invoke is required for native component commands"));
    }
    return globalThis.Nana.host.invoke("componentCall", [id, String(command), args ?? null]);
  },
  onError(listener) {
    if (typeof listener !== "function") throw new TypeError("Nana.components.onError requires a function");
    nanaNativeComponentErrorListeners.add(listener);
    return () => nanaNativeComponentErrorListeners.delete(listener);
  },
};

let nanaDialogProvider = null;
globalThis.Nana.dialogs = {
  install(provider) {
    if (!provider || typeof provider !== "object") {
      throw new TypeError("Nana.dialogs.install requires a Vue dialog provider");
    }
    nanaDialogProvider = provider;
    return () => {
      if (nanaDialogProvider === provider) nanaDialogProvider = null;
    };
  },
  alert(message, options) {
    return invokeDialogProvider("alert", [message, options || {}]);
  },
  confirm(message, options) {
    return invokeDialogProvider("confirm", [message, options || {}]);
  },
  prompt(message, defaultValue, options) {
    return invokeDialogProvider("prompt", [message, defaultValue ?? "", options || {}]);
  },
};

function invokeDialogProvider(method, args) {
  if (!nanaDialogProvider || typeof nanaDialogProvider[method] !== "function") {
    const error = new Error(`No Vue dialog provider installed for Nana.dialogs.${method}`);
    error.name = "NotSupportedError";
    return Promise.reject(error);
  }
  try {
    return Promise.resolve(nanaDialogProvider[method](...args));
  } catch (error) {
    return Promise.reject(error);
  }
}
globalThis.Nana.windows = {
  async create(options) {
    if (!globalThis.Nana.host || typeof globalThis.Nana.host.invoke !== "function") {
      throw new Error("Nana.host.invoke is required for Nana.windows.create");
    }
    const descriptor = await globalThis.Nana.host.invoke("windowCreate", [options || {}]);
    const handle = createWindowHandle(descriptor);
    await handle.ready;
    return handle;
  },
  get(id) {
    return nanaWindowHandles.get(Number(id)) || null;
  },
  list() {
    const descriptors = hostCall("windowList", []) || [];
    return Array.from(descriptors, createWindowHandle);
  },
};

if (globalThis.Nana.host && typeof globalThis.Nana.host.on === "function") {
  globalThis.Nana.host.on("native-component-error", (payload) => {
    const raw = payload && payload.error && typeof payload.error === "object" ? payload.error : {};
    const error = new Error(String(raw.message || "Native component rendering failed"));
    error.name = String(raw.name || "NativeComponentRenderError");
    if (raw.code != null) error.code = String(raw.code);
    if (raw.stack != null) error.stack = String(raw.stack);
    if (raw.details !== undefined) error.details = raw.details;
    error.component = String((payload && payload.component) || "");
    error.elementId = Number(payload && payload.id);
    error.windowId = Number(payload && payload.windowId);
    if (Number.isFinite(error.elementId) && typeof globalThis.__nanaFireWindowEvent === "function") {
      globalThis.__nanaFireWindowEvent(error.windowId, error.elementId, "error", {
        error,
        component: error.component,
      });
    }
    for (const listener of [...nanaNativeComponentErrorListeners]) listener(error);
  });
  globalThis.Nana.host.on("window-ready", (payload) => {
    const handle = nanaWindowHandles.get(Number(payload && payload.id));
    if (handle) handle.__resolveReady();
  });
  globalThis.Nana.host.on("window-open-failed", (payload) => {
    const id = Number(payload && payload.id);
    const handle = nanaWindowHandles.get(id);
    if (handle) {
      const error = new Error(String((payload && payload.message) || "native window creation failed"));
      error.name = "WindowOpenError";
      handle.__rejectReady(error);
      handle.unmount();
      handle.__resolveClosed({ reason: "open-failed", error });
    }
    nanaWindowHandles.delete(id);
    if (typeof globalThis.__nanaReleaseWindowNodes === "function") {
      globalThis.__nanaReleaseWindowNodes(id);
    }
    if (typeof globalThis.__nanaDestroyWindowContext === "function") {
      globalThis.__nanaDestroyWindowContext(id);
    }
  });
  globalThis.Nana.host.on("window-geometry", (payload) => {
    const handle = nanaWindowHandles.get(Number(payload && payload.id));
    if (handle && handle.window) {
      const scale = Number(payload.scaleFactor || handle.window.devicePixelRatio || 1);
      handle.window.innerWidth = Number(payload.width) || handle.window.innerWidth;
      handle.window.innerHeight = Number(payload.height) || handle.window.innerHeight;
      handle.window.devicePixelRatio = scale;
    }
  });
  globalThis.Nana.host.on("window-closed", (payload) => {
    const id = Number(payload && payload.id);
    const handle = nanaWindowHandles.get(id);
    if (handle) {
      const error = new Error("window closed before becoming ready");
      error.name = "AbortError";
      handle.__rejectReady(error);
      handle.unmount();
      handle.__resolveClosed({ reason: "closed" });
    }
    nanaWindowHandles.delete(id);
    if (typeof globalThis.__nanaReleaseWindowNodes === "function") {
      globalThis.__nanaReleaseWindowNodes(id);
    }
    if (typeof globalThis.__nanaDestroyWindowContext === "function") {
      globalThis.__nanaDestroyWindowContext(id);
    }
  });
}
