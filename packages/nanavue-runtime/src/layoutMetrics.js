/**
 * Project host `layoutBox` onto Element layout readables.
 * Shared by wrapNode (nanavue-runtime) — not a full CSSOM.
 */

const NANA_NODE_DOCUMENT_STRIDE = 4294967296;

export function nanaWindowIdFromNode(value) {
  const node = Number(value);
  if (!Number.isSafeInteger(node) || node < NANA_NODE_DOCUMENT_STRIDE) return 0;
  return Math.floor(node / NANA_NODE_DOCUMENT_STRIDE);
}

export function withNanaWindowContext(windowId, action) {
  const previous = Number(globalThis.__nanaActiveWindowId || 0);
  globalThis.__nanaActiveWindowId = Number(windowId || 0);
  try {
    return action();
  } finally {
    globalThis.__nanaActiveWindowId = previous;
  }
}

export function hostCall(name, args) {
  const host = globalThis.__nanaHost;
  if (!host || typeof host.call !== "function") {
    throw new Error("__nanaHost.call is not registered");
  }
  const values = Array.isArray(args) ? args : [];
  let windowId = Number(globalThis.__nanaActiveWindowId || 0);
  if (!windowId && values.length) windowId = nanaWindowIdFromNode(values[0]);
  if (windowId && name !== "windowCall") {
    return host.call("windowCall", [windowId, String(name), values]);
  }
  return host.call(name, values);
}

function emptyRect() {
  return {
    x: 0,
    y: 0,
    width: 0,
    height: 0,
    top: 0,
    left: 0,
    bottom: 0,
    right: 0,
    toJSON() {
      return this;
    },
  };
}

function readHostLayoutBox(nid) {
  if (nid == null || !Number.isFinite(Number(nid))) return null;
  try {
    const box = hostCall("layoutBox", [Number(nid)]);
    if (!box || typeof box !== "object") return null;
    return box;
  } catch (_err) {
    return null;
  }
}

function metricPx(value, fallback) {
  const n = Number(value);
  if (Number.isFinite(n)) return Math.max(0, Math.round(n));
  return fallback;
}

function offsetPx(value) {
  const n = Number(value);
  return Number.isFinite(n) ? Math.round(n) : 0;
}

export function layoutRect(nid) {
  try {
    const box = hostCall("layoutBox", [nid]);
    if (!box || typeof box !== "object") return emptyRect();
    const x = Number(box.x) || 0;
    const y = Number(box.y) || 0;
    const width = Number(box.width) || 0;
    const height = Number(box.height) || 0;
    return {
      x,
      y,
      width,
      height,
      top: y,
      left: x,
      bottom: y + height,
      right: x + width,
      toJSON() {
        return this;
      },
    };
  } catch (_err) {
    return emptyRect();
  }
}

/** Integer CSS-px size from layoutBox (0 when missing / NaN). */
export function layoutSizePx(nid) {
  const r = layoutRect(nid);
  return {
    width: Math.max(0, Math.round(Number(r.width) || 0)),
    height: Math.max(0, Math.round(Number(r.height) || 0)),
  };
}

/** Normalize Element.scrollIntoView argument → host options object. */
export function normalizeScrollIntoViewArg(arg) {
  if (arg == null || arg === true) {
    return { block: "start", inline: "nearest" };
  }
  if (arg === false) {
    return { block: "end", inline: "nearest" };
  }
  if (typeof arg === "object") {
    return {
      block: arg.block != null ? String(arg.block) : "start",
      inline: arg.inline != null ? String(arg.inline) : "nearest",
    };
  }
  return { block: "start", inline: "nearest" };
}

/** Host `scrollIntoView` based on layoutBox + scrollable ancestors. */
export function scrollNodeIntoView(nid, arg) {
  if (nid == null || !Number.isFinite(Number(nid))) return;
  try {
    hostCall("scrollIntoView", [Number(nid), normalizeScrollIntoViewArg(arg)]);
  } catch (_err) {}
}

function readHostScroll(nid, axis) {
  if (nid == null || !Number.isFinite(Number(nid))) return 0;
  try {
    const off = hostCall("getScrollOffset", [Number(nid)]);
    if (!off || typeof off !== "object") return 0;
    const v = axis === "x" ? off.scrollLeft ?? off.x : off.scrollTop ?? off.y;
    const n = Number(v);
    return Number.isFinite(n) ? n : 0;
  } catch (_err) {
    return 0;
  }
}

function writeHostScroll(nid, axis, next) {
  if (nid == null || !Number.isFinite(Number(nid))) return;
  const n = Number(next);
  const value = Number.isFinite(n) ? Math.max(0, n) : 0;
  try {
    const cur = hostCall("getScrollOffset", [Number(nid)]) || {};
    const x = axis === "x" ? value : Number(cur.scrollLeft ?? cur.x) || 0;
    const y = axis === "y" ? value : Number(cur.scrollTop ?? cur.y) || 0;
    hostCall("setScrollOffset", [Number(nid), x, y]);
  } catch (_err) {}
}

/**
 * Install offset/client/scroll metrics from host layoutBox.
 * offset* is the border box; client* is the padding box when the host
 * sends clientWidth; scroll* uses scrollWidth when present.
 * offsetLeft/Top are the real subset; offsetParent may be null without a node cache.
 * scrollTop/scrollLeft round-trip through host scroll contract.
 */
export function defineLayoutMetrics(node, nid) {
  const sizeMetric = (kind, axis) => ({
    configurable: true,
    enumerable: true,
    get() {
      const s = layoutSizePx(nid);
      const border = axis === "w" ? s.width : s.height;
      if (kind === "offset") return border;
      const box = readHostLayoutBox(nid);
      if (!box) return border;
      if (kind === "client") {
        return metricPx(axis === "w" ? box.clientWidth : box.clientHeight, border);
      }
      return metricPx(axis === "w" ? box.scrollWidth : box.scrollHeight, border);
    },
  });
  Object.defineProperty(node, "offsetWidth", sizeMetric("offset", "w"));
  Object.defineProperty(node, "offsetHeight", sizeMetric("offset", "h"));
  Object.defineProperty(node, "clientWidth", sizeMetric("client", "w"));
  Object.defineProperty(node, "clientHeight", sizeMetric("client", "h"));
  Object.defineProperty(node, "scrollWidth", sizeMetric("scroll", "w"));
  Object.defineProperty(node, "scrollHeight", sizeMetric("scroll", "h"));
  Object.defineProperty(node, "offsetLeft", {
    configurable: true,
    enumerable: true,
    get() {
      const box = readHostLayoutBox(nid);
      return box ? offsetPx(box.offsetLeft) : 0;
    },
  });
  Object.defineProperty(node, "offsetTop", {
    configurable: true,
    enumerable: true,
    get() {
      const box = readHostLayoutBox(nid);
      return box ? offsetPx(box.offsetTop) : 0;
    },
  });
  Object.defineProperty(node, "clientLeft", {
    configurable: true,
    enumerable: true,
    get() {
      const box = readHostLayoutBox(nid);
      return box ? offsetPx(box.clientLeft ?? box.borderWidth) : 0;
    },
  });
  Object.defineProperty(node, "clientTop", {
    configurable: true,
    enumerable: true,
    get() {
      const box = readHostLayoutBox(nid);
      return box ? offsetPx(box.clientTop ?? box.borderWidth) : 0;
    },
  });
  Object.defineProperty(node, "offsetParent", {
    configurable: true,
    enumerable: true,
    get() {
      // offsetLeft/Top are the real subset; offsetParent may be null without a node cache.
      const box = readHostLayoutBox(nid);
      const id = Number(box && box.offsetParent) || 0;
      if (!id) return null;
      const cache = globalThis.__nanaNodeCache;
      if (cache && typeof cache.get === "function") {
        const found = cache.get(id);
        return found == null ? null : found;
      }
      return null;
    },
  });
  Object.defineProperty(node, "scrollTop", {
    configurable: true,
    enumerable: true,
    get() {
      return readHostScroll(nid, "y");
    },
    set(next) {
      writeHostScroll(nid, "y", next);
    },
  });
  Object.defineProperty(node, "scrollLeft", {
    configurable: true,
    enumerable: true,
    get() {
      return readHostScroll(nid, "x");
    },
    set(next) {
      writeHostScroll(nid, "x", next);
    },
  });
}
