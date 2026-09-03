/** Property classification and host serialization; no node or listener state. */
export function isOn(key) {
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
export function isModelListener(key) {
  return typeof key === "string" && key.startsWith("onUpdate:");
}

/** Keys Vue sets as DOM properties (not attributes) on a real Element. */
export function shouldSetAsDomProp(key) {
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

export function isSvgElement(el, namespace) {
  if (namespace === "svg") return true;
  if (el && el.__isSVG) return true;
  const tag = String((el && (el.tag || el.tagName)) || "").toLowerCase();
  return SVG_TAGS.has(tag);
}

export function isSvgAttrKey(key) {
  return (
    key.startsWith("xlink:") ||
    key.startsWith("xml:") ||
    COMMON_SVG_ATTRS.has(key) ||
    COMMON_SVG_ATTRS.has(key.toLowerCase())
  );
}

export function serializePatchValue(next) {
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
export function seedHostProps(vnodeProps) {
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

export function syncClassList(el, classValue) {
  if (!el || !el.classList || typeof el.classList.__replace !== "function") return;
  el.classList.__replace(classValue == null ? "" : String(classValue));
}
