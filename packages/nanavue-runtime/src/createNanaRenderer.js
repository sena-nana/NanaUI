/**
 * Nana custom renderer hostOps — **L1 + L2** entry into the Nana Style Model.
 *
 * - L1: `createElement` + class/inline style → Rust `css_map` / `widget_map`
 * - L2: HTML 1:1 tags (`button`, `input`, …) seed WidgetProps via createElement;
 *   colliding / Nana-only controls still use `createWidget` / `nana-*`
 * Both share one forest; draw path is Runtime/UiScene via the Scene host
 * (`scene-view`; no Iced widget tree, no WebView paint).
 *
 * Also enhances host nodes with Element-like stubs so @lilia/ui template refs
 * (getBoundingClientRect, style, classList, dataset, …) do not throw.
 */
import { createRenderer } from "@vue/runtime-core";
import { defineLayoutMetrics, hostCall, layoutRect, nanaWindowIdFromNode, scrollNodeIntoView, withNanaWindowContext } from "./layoutMetrics.js";
import { appearEnterPhaseAfter, armMotionEndFromStyles, cancelArmedMotionEnd, createMotionEndEvent, isPaintOnlyStyleKey, isVueTransitionClass, preserveMotionClasses, resolveTransitionComputedStyles, vueTransitionClassKind } from "./transitionContract.js";

export { hostCall } from "./layoutMetrics.js";
export { applyFlipPaintTransform, clearFlipPaintTransform, readFlipBox } from "./transitionContract.js";



import { isOn, isModelListener, shouldSetAsDomProp, isSvgElement, isSvgAttrKey, serializePatchValue, seedHostProps, syncClassList } from "./props.js";
import { flushPendingStyles, queueStyleFlush, flushHostFrame, installFlushHooks, parseCssText, hostStyleStore, paintTransformCssValue, syncPaintTransform, createStyleProxy } from "./styles.js";
import { contextForWindow } from "./windowContext.js";
import { createEventDispatcher } from "./events.js";
import { createNodeStore } from "./nodes.js";

const events = createEventDispatcher((id) => nodes.wrapById(id));
const nodes = createNodeStore(events, releaseNodeResources);
const { listenerKey, normalizeListenerOptions, parseEventName, normalizeHandler, isListenerObject, addNanaListener, removeNanaListener, invokeNanaListenerPhase, invokeGlobalPhase, createFileList, createDataTransfer, createEventPayload, fanOutDocumentWindow, releaseNodeListeners } = events;
const { parentCacheFresh, childrenCacheFresh, refillParent, refillChildren, parentIdOf, childIdsOf, nodeTagName, markCreatedNode, clearChildrenCache, invalidateChildrenCache, unlinkChild, linkChild, siblingNode, isConnectedNode, wrapById, wrapNode, camelToDataAttr, createDatasetProxy, createClassList, readElementMotionStyles, armElementMotionEnd, nodeId } = nodes;
export { wrapNode, nodeId, flushHostFrame };
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
  for (const [nid, node] of nodes.cachedNodes()) {
    if (nanaWindowIdFromNode(nid) !== id) continue;
    releaseNodeResources(node);
    nodes.forgetNode(nid);
    releaseNodeListeners(nid);
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
  if (node.__nanaOwnsMediaResource && node.__nanaMediaResource) {
    hostCall("mediaRelease", [node.__nanaMediaResource.id]);
    node.__nanaOwnsMediaResource = false;
    node.__nanaMediaResource = null;
    const nid = nodeId(node);
    if (nid != null) {
      hostCall("patchProp", [nid, "data-nana-media", ""]);
      hostCall("patchProp", [nid, "data-nana-video", ""]);
    }
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

function bindMediaProp(el, nid, key, next) {
  if (typeof globalThis.__nanaEnhanceMedia === "function" && !el.__nanaMediaResource) {
    const tag = String(el && (el.tag || el.tagName || "")).toLowerCase();
    globalThis.__nanaEnhanceMedia(el, tag === "audio" ? "audio" : "video");
  }
  const resource = el.__nanaMediaResource;
  if (!resource || resource.id == null) {
    hostCall("patchProp", [nid, key, next == null ? null : next]);
    return;
  }
  if (key === "srcObject") {
    el.srcObject = next;
    return;
  }
  if (key === "currentTime") {
    el.currentTime = Number(next) || 0;
    hostCall("patchProp", [nid, "currentTime", String(el.currentTime)]);
    return;
  }
  el.src = next == null ? "" : String(next);
  el.attributes = el.attributes || {};
  el.attributes.src = el.src;
  hostCall("patchProp", [nid, "src", el.src]);
  const tag = String(el && (el.tag || el.tagName || "")).toLowerCase();
  if (el.__nanaMediaResource && el.__nanaMediaResource.id != null) {
    hostCall("patchProp", [nid, "data-nana-media", String(el.__nanaMediaResource.id)]);
  }
  if (tag === "video" && el.__nanaMediaResource) {
    const id = el.__nanaMediaResource.id;
    const slot = el.__nanaMediaResource.hasVideoFrame && id != null ? String(id) : "";
    hostCall("patchProp", [nid, "data-nana-video", slot]);
  }
}

/**
 * Create a semantic Nana widget (button / switch / text / …) without DOM paint.
 * Returns a host node whose id is tracked by Rust `MessageBridge`.
 */
const RETIRED_HTML_ALIAS_TAGS = new Map([
  ["nana-button", "button"],
  ["nana-text-input", "input"],
  ["nana-input", "input"],
  ["nana-textarea", "textarea"],
  ["nana-select", "select"],
  ["nana-progress", "progress"],
  ["nana-divider", "hr"],
  ["nana-dialog", "dialog"],
  ["nana-checkbox", "input"],
  ["nana-range-field", "input"],
  ["nana-range", "input"],
  ["nana-table", "table"],
  ["nana-table-row", "tr"],
  ["nana-table-cell", "td"],
  ["nana-search", "search-dropdown"],
  ["nana-list", "ul"],
  ["nana-list-item", "li"],
  ["nana-number-input", "input"],
  ["nana-level-meter", "meter"],
  ["nana-settings-collapsible-card", "details"],
]);

export function createWidget(kind, props) {
  const raw = String(kind);
  const normalized = raw.replace(/^nana-/i, "");
  const tag =
    RETIRED_HTML_ALIAS_TAGS.get(`nana-${normalized}`) ?? htmlTagForKind(normalized);
  const id = hostCall("createWidget", [raw, props && typeof props === "object" ? { ...props } : {}]);
  return markCreatedNode(wrapNode(id, "element", tag));
}

function htmlTagForKind(kind) {
  switch (String(kind).toLowerCase()) {
    case "button":
      return "button";
    case "text-input":
    case "input":
    case "checkbox":
    case "radio":
    case "range-field":
    case "range":
    case "number-input":
      return "input";
    case "textarea":
      return "textarea";
    case "select":
      return "select";
    case "progress":
      return "progress";
    case "divider":
      return "hr";
    case "dialog":
      return "dialog";
    case "table":
      return "table";
    case "table-row":
    case "tr":
      return "tr";
    case "table-cell":
    case "td":
      return "td";
    case "th":
      return "th";
    case "search-dropdown":
      return "search-dropdown";
    case "list":
      return "ul";
    case "list-item":
      return "li";
    case "level-meter":
      return "meter";
    case "settings-collapsible-card":
      return "details";
    default:
      return `nana-${kind}`;
  }
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
    if (
      ["video", "audio"].includes(String(el && (el.tag || el.tagName || "")).toLowerCase()) &&
      (key === "src" || key === "srcObject" || key === "currentTime")
    ) {
      bindMediaProp(el, nid, key, next);
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
        let sawPaintTransform = false;
        for (const [k, v] of Object.entries(next)) {
          if (isPaintOnlyStyleKey(k)) {
            if (el && el.style) el.style[k] = v == null ? "" : v;
            sawPaintTransform = true;
            continue;
          }
          if (v != null && v !== "") cleaned[k] = Array.isArray(v) ? v[v.length - 1] : v;
        }
        if (sawPaintTransform && el && el.style) {
          try {
            hostCall("setPaintTransform", [nid, paintTransformCssValue(el.style)]);
          } catch (_err) {}
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
        invalidateChildrenCache(el);
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
    cancelArmedMotionEnd(nid);
    releaseNodeListeners(nid);
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
    // Retired HTML aliases (`nana-button`, …) error; use the native tag.
    if (RETIRED_HTML_ALIAS_TAGS.has(lower)) {
      throw new Error(
        `retired tag \`${lower}\`; use \`${RETIRED_HTML_ALIAS_TAGS.get(lower)}\``,
      );
    }
    if (
      lower.startsWith("nana-") &&
      lower !== "nana-gpu"
    ) {
      const kind = lower.slice("nana-".length);
      const id = hostCall("createWidget", [kind, seed || {}]);
      const node = markCreatedNode(wrapNode(id, "element", tagName));
      node.__isSVG = false;
      return node;
    }
    const id = hostCall("createElement", [tagName, ns, is, seed]);
    const node = markCreatedNode(wrapNode(id, "element", tagName));
    node.__isSVG = isSvgElement({ tag: lower }, ns);
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
    if (
      (lower === "video" || lower === "audio") &&
      typeof globalThis.__nanaEnhanceMedia === "function"
    ) {
      globalThis.__nanaEnhanceMedia(node, lower);
      const resource = node.__nanaMediaResource;
      if (resource && resource.id != null) {
        hostCall("patchProp", [id, "data-nana-media", String(resource.id)]);
      }
    }
    if (lower === "img" && vnodeProps && typeof vnodeProps === "object" && vnodeProps.src != null) {
      bindImageSource(node, id, vnodeProps.src);
    }
    if (
      (lower === "video" || lower === "audio") &&
      vnodeProps &&
      typeof vnodeProps === "object"
    ) {
      if (vnodeProps.src != null) bindMediaProp(node, id, "src", vnodeProps.src);
      if (vnodeProps.srcObject != null) bindMediaProp(node, id, "srcObject", vnodeProps.srcObject);
    }
    return node;
  },
  createText(text) {
    return markCreatedNode(wrapNode(hostCall("createText", [String(text)]), "text", null));
  },
  createComment(text) {
    return markCreatedNode(wrapNode(hostCall("createComment", [String(text ?? "")]), "comment", null));
  },
  setText(node, text) {
    hostCall("setText", [nodeId(node), String(text)]);
  },
  setElementText(el, text) {
    const n = el && typeof el === "object" ? el : wrapById(nodeId(el));
    for (const child of Array.from((n && n.childNodes) || [])) releaseNodeResources(child);
    invalidateChildrenCache(n);
    hostCall("setElementText", [nodeId(el), String(text)]);
  },
  parentNode(node) {
    const n = node && typeof node === "object" ? node : wrapById(nodeId(node));
    return n ? n.parentNode : null;
  },
  nextSibling(node) {
    const n = node && typeof node === "object" ? node : wrapById(nodeId(node));
    return n ? siblingNode(n, 1) : null;
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
    invalidateChildrenCache(parent && typeof parent === "object" ? parent : wrapById(nodeId(parent)));
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

function createRendererForWindow(windowId = 0) {
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
    const origMount = app && typeof app.mount === "function" ? app.mount.bind(app) : null;
    if (origMount) {
      app.mount = function (container) {
        if (container == null || container === "") {
          container = defaultMountContainer(windowId);
        }
        return origMount(container);
      };
    }
    return app;
  };
  return { createApp: createAppWithDiagnostics, render, hostOps: scopedHostOps, windowId: Number(windowId || 0) };
}

function defaultMountContainer(windowId = 0) {
  return withNanaWindowContext(windowId, () =>
    wrapNode(hostCall("mountRoot", []), "element", "body"),
  );
}

export function createApp(rootComponent, rootProps) {
  return createRendererForWindow(0).createApp(rootComponent, rootProps);
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

  /**
   * Host-callable motion completion. Dispatches `transitionend` / `animationend`
   * on the wrapNode — not WAAPI, not Element.animate.
   */
  globalThis.__nanaMotionCancel = function __nanaMotionCancel(nid) {
    cancelArmedMotionEnd(nid);
  };

  globalThis.__nanaMotionComplete = function __nanaMotionComplete(nid, detail) {
    const extra = detail && typeof detail === "object" ? detail : {};
    const type = extra.type || extra.event || "transitionend";
    cancelArmedMotionEnd(nid);
    const target = wrapById(nid);
    if (!target) return false;
    const event = createMotionEndEvent(type, target, extra);
    return target.dispatchEvent(event);
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
installFlushHooks();

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
  const renderer = createRendererForWindow(id);
  const root = defaultMountContainer(id);
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
    setIcon(icon) {
      hostCall("windowSetIcon", [id, icon ?? null]);
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
  setApplicationIcon(icon) {
    hostCall("windowSetApplicationIcon", [icon ?? null]);
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
