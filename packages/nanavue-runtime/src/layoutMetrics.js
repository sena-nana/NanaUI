/**
 * Project host `layoutBox` onto Element layout readables.
 * Shared by wrapNode (nanavue-runtime) — not a full CSSOM.
 */

export function hostCall(name, args) {
  const host = globalThis.__nanaHost;
  if (!host || typeof host.call !== "function") {
    throw new Error("__nanaHost.call is not registered");
  }
  return host.call(name, args);
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
 * Install offsetWidth/clientWidth/scroll* from iced/measure layoutBox.
 * No border/padding split — those metrics share the same box.
 * scrollTop/scrollLeft round-trip through host scroll contract.
 */
export function defineLayoutMetrics(node, nid) {
  const dim = (axis) => ({
    configurable: true,
    enumerable: true,
    get() {
      const s = layoutSizePx(nid);
      return axis === "w" ? s.width : s.height;
    },
  });
  Object.defineProperty(node, "offsetWidth", dim("w"));
  Object.defineProperty(node, "offsetHeight", dim("h"));
  Object.defineProperty(node, "clientWidth", dim("w"));
  Object.defineProperty(node, "clientHeight", dim("h"));
  Object.defineProperty(node, "scrollWidth", dim("w"));
  Object.defineProperty(node, "scrollHeight", dim("h"));
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
