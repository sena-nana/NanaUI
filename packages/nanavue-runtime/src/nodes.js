/** The renderer's sole JS identity cache. Retained tree authority stays in Runtime. */
import { defineLayoutMetrics, hostCall, layoutRect, nanaWindowIdFromNode, scrollNodeIntoView, withNanaWindowContext } from "./layoutMetrics.js";
import { appearEnterPhaseAfter, armMotionEndFromStyles, cancelArmedMotionEnd, createMotionEndEvent, isVueTransitionClass, preserveMotionClasses, resolveTransitionComputedStyles, vueTransitionClassKind } from "./transitionContract.js";
import { contextForWindow } from "./windowContext.js";
import { createStyleProxy } from "./styles.js";
import { syncClassList, isSvgElement } from "./props.js";
export function createNodeStore(events, releaseNodeResources) {
const { createEventPayload, invokeNanaListenerPhase, addNanaListener, removeNanaListener } = events;
const nodeCache = new Map();
function parentCacheFresh(node) {
  return !!node && node.__parentId !== undefined;
}

function childrenCacheFresh(node) {
  return !!node && Array.isArray(node.__childIds);
}

function refillParent(node) {
  let pid = null;
  try {
    const raw = hostCall("parentNode", [node.__nid]);
    pid = raw == null ? null : Number(raw);
  } catch (_err) {
    pid = null;
  }
  node.__parentId = pid;
  return pid;
}

function refillChildren(node) {
  let ids = [];
  try {
    ids = hostCall("childNodes", [node.__nid]) || [];
  } catch (_err) {
    ids = [];
  }
  node.__childIds = Array.from(ids, (id) => Number(id));
  for (const cid of node.__childIds) {
    const child = nodeCache.get(cid);
    if (!child) continue;
    child.__parentId = node.__nid;
  }
  return node.__childIds;
}

function parentIdOf(node) {
  if (parentCacheFresh(node)) return node.__parentId;
  return refillParent(node);
}

function childIdsOf(node) {
  if (childrenCacheFresh(node)) return node.__childIds;
  return refillChildren(node);
}

function nodeTagName(node) {
  return String((node && (node.tag || node.tagName)) || "").toLowerCase();
}

function markCreatedNode(node) {
  if (!node) return node;
  node.__parentId = null;
  node.__childIds = [];
  return node;
}

function clearChildrenCache(node) {
  if (!node || typeof node !== "object") return;
  node.__childIds = [];
}

function invalidateChildrenCache(node) {
  if (!node || typeof node !== "object") return;
  node.__childIds = undefined;
}

function unlinkChild(child) {
  if (!child || typeof child !== "object") return;
  if (parentCacheFresh(child) && child.__parentId != null) {
    const prev = nodeCache.get(child.__parentId);
    if (prev && childrenCacheFresh(prev)) {
      const i = prev.__childIds.indexOf(child.__nid);
      if (i >= 0) prev.__childIds.splice(i, 1);
    }
  }
  child.__parentId = null;
}

function linkChild(parent, child, anchor) {
  if (!parent || !child) return;
  unlinkChild(child);
  child.__parentId = parent.__nid;
  if (!childrenCacheFresh(parent)) return;
  const kids = parent.__childIds;
  const cid = child.__nid;
  const existing = kids.indexOf(cid);
  if (existing >= 0) kids.splice(existing, 1);
  const aid = nodeId(anchor);
  const at = aid != null ? kids.indexOf(aid) : -1;
  if (at >= 0) kids.splice(at, 0, cid);
  else kids.push(cid);
}

function siblingNode(node, delta) {
  const nid = nodeId(node);
  if (nid == null) return null;
  const parentId = parentIdOf(node);
  if (parentId == null) return null;
  const parent = wrapById(parentId);
  if (!parent) return null;
  const kids = childIdsOf(parent);
  const i = kids.indexOf(nid);
  if (i < 0) return null;
  const sid = kids[i + delta];
  return sid == null ? null : wrapById(sid);
}

function isConnectedNode(node) {
  let cur = node;
  const seen = new Set();
  while (cur && Number.isFinite(Number(cur.__nid))) {
    const nid = Number(cur.__nid);
    if (seen.has(nid)) return false;
    seen.add(nid);
    const tag = nodeTagName(cur);
    if (tag === "html" || tag === "#document") return true;
    const pid = parentIdOf(cur);
    if (pid == null) return tag === "body";
    cur = wrapById(pid);
  }
  return false;
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

function wrapNode(id, kind, tag) {
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
      if (type === "transitionend" || type === "animationend") {
        cancelArmedMotionEnd(nid);
      }
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
      const c = child && typeof child === "object" ? child : wrapById(nodeId(child));
      try {
        hostCall("insert", [nodeId(c), nid, null]);
      } catch (_err) {}
      linkChild(this, c, null);
      return child;
    },
    removeChild(child) {
      const c = child && typeof child === "object" ? child : wrapById(nodeId(child));
      unlinkChild(c);
      try {
        hostCall("remove", [nodeId(c)]);
      } catch (_err) {}
      return child;
    },
    insertBefore(child, anchor) {
      const c = child && typeof child === "object" ? child : wrapById(nodeId(child));
      const a =
        anchor && typeof anchor === "object"
          ? anchor
          : anchor != null
            ? wrapById(nodeId(anchor))
            : null;
      try {
        hostCall("insert", [nodeId(c), nid, nodeId(a)]);
      } catch (_err) {}
      linkChild(this, c, a);
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
      // DOM Node.contains — walk cached parent chain (hostCall only on miss).
      const otherId = nodeId(other);
      if (otherId == null) return false;
      if (otherId === nid) return true;
      let cur = wrapById(otherId);
      const seen = new Set();
      while (cur && Number.isFinite(Number(cur.__nid))) {
        const pid = parentIdOf(cur);
        if (pid == null) return false;
        if (pid === nid) return true;
        if (seen.has(pid)) return false;
        seen.add(pid);
        cur = wrapById(pid);
      }
      return false;
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
  // Live tree navigation — JS parent/child cache from insert/remove; hostCall on miss
  // or after hierarchy invalidation (insertStaticContent), never on style flush.
  Object.defineProperty(node, "parentNode", {
    configurable: true,
    enumerable: true,
    get() {
      const pid = parentIdOf(this);
      return pid == null ? null : wrapById(pid);
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
      return childIdsOf(this).map((cid) => wrapById(cid));
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
      const kids = childIdsOf(this);
      return kids.length ? wrapById(kids[0]) : null;
    },
  });
  Object.defineProperty(node, "lastChild", {
    configurable: true,
    enumerable: true,
    get() {
      const kids = childIdsOf(this);
      return kids.length ? wrapById(kids[kids.length - 1]) : null;
    },
  });
  Object.defineProperty(node, "nextSibling", {
    configurable: true,
    enumerable: true,
    get() {
      return siblingNode(this, 1);
    },
  });
  Object.defineProperty(node, "previousSibling", {
    configurable: true,
    enumerable: true,
    get() {
      return siblingNode(this, -1);
    },
  });
  Object.defineProperty(node, "isConnected", {
    configurable: true,
    enumerable: true,
    get() {
      return isConnectedNode(this);
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
      invalidateChildrenCache(this);
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
      if (node.__kind !== "text") invalidateChildrenCache(this);
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
  const armFromClasses = () => {
    if (!el) return;
    const phase = appearEnterPhaseAfter(set);
    el.__nanaTransitionPhase = phase;
    if (!phase) {
      const hasMove = [...set].some((token) => vueTransitionClassKind(token) === "move");
      if (!hasMove && ![...set].some(isVueTransitionClass)) {
        cancelArmedMotionEnd(nid);
        return;
      }
    }
    armElementMotionEnd(el, nid);
  };
  return {
    add(...tokens) {
      tokens.forEach((t) => {
        const s = String(t).trim();
        if (s) set.add(s);
      });
      sync();
      armFromClasses();
    },
    remove(...tokens) {
      tokens.forEach((t) => set.delete(String(t)));
      sync();
      armFromClasses();
    },
    toggle(token, force) {
      const t = String(token);
      if (force === true) set.add(t);
      else if (force === false) set.delete(t);
      else if (set.has(t)) set.delete(t);
      else set.add(t);
      sync();
      armFromClasses();
      return set.has(t);
    },
    contains(token) {
      return set.has(String(token));
    },
    /** Replace tokens from a Vue `class` patch without re-entering patchProp loops. */
    __replace(classValue) {
      const kept = preserveMotionClasses(classValue, [...set]);
      set.clear();
      kept.forEach((t) => set.add(t));
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

function readElementMotionStyles(el) {
  try {
    const view =
      (el && el.ownerDocument && el.ownerDocument.defaultView) || globalThis;
    if (view && typeof view.getComputedStyle === "function" && el) {
      return view.getComputedStyle(el);
    }
  } catch (_err) {}
  return resolveTransitionComputedStyles(null);
}

function armElementMotionEnd(el, nid) {
  const styles = readElementMotionStyles(el);
  armMotionEndFromStyles(nid, styles, (detail) => {
    const node = wrapById(nid) || el;
    if (!node || typeof node.dispatchEvent !== "function") return;
    node.dispatchEvent(createMotionEndEvent(detail.type, node, detail));
  });
}

function nodeId(node) {
  if (node == null) return null;
  if (typeof node === "number") return node;
  if (typeof node.__nid === "number") return node.__nid;
  return null;
}


return { parentCacheFresh, childrenCacheFresh, refillParent, refillChildren, parentIdOf, childIdsOf, nodeTagName, markCreatedNode, clearChildrenCache, invalidateChildrenCache, unlinkChild, linkChild, siblingNode, isConnectedNode, wrapById, wrapNode, camelToDataAttr, createDatasetProxy, createClassList, readElementMotionStyles, armElementMotionEnd, nodeId, cachedNodes: () => [...nodeCache.entries()], forgetNode: (id) => nodeCache.delete(id) };
}
