  function layoutBoxSizeFromNid(nid) {
    if (nid == null || !Number.isFinite(Number(nid))) {
      return { width: 0, height: 0 };
    }
    try {
      const box = hostCall("layoutBox", [Number(nid)]);
      if (!box || typeof box !== "object") return { width: 0, height: 0 };
      return {
        width: Math.max(0, Math.round(Number(box.width) || 0)),
        height: Math.max(0, Math.round(Number(box.height) || 0)),
      };
    } catch (_err) {
      return { width: 0, height: 0 };
    }
  }

  function readLayoutBoxFromNid(nid) {
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

  function installElementLayoutMetrics(proto) {
    if (!proto || proto.__nanaLayoutMetricsInstalled) return;
    function sizeMetric(kind, axis) {
      return {
        configurable: true,
        enumerable: true,
        get: function () {
          const s = layoutBoxSizeFromNid(this.__nid);
          const border = axis === "w" ? s.width : s.height;
          if (kind === "offset") return border;
          const box = readLayoutBoxFromNid(this.__nid);
          if (!box) return border;
          if (kind === "client") {
            return metricPx(axis === "w" ? box.clientWidth : box.clientHeight, border);
          }
          return metricPx(axis === "w" ? box.scrollWidth : box.scrollHeight, border);
        },
      };
    }
    Object.defineProperty(proto, "offsetWidth", sizeMetric("offset", "w"));
    Object.defineProperty(proto, "offsetHeight", sizeMetric("offset", "h"));
    Object.defineProperty(proto, "clientWidth", sizeMetric("client", "w"));
    Object.defineProperty(proto, "clientHeight", sizeMetric("client", "h"));
    Object.defineProperty(proto, "scrollWidth", sizeMetric("scroll", "w"));
    Object.defineProperty(proto, "scrollHeight", sizeMetric("scroll", "h"));
    Object.defineProperty(proto, "offsetLeft", {
      configurable: true,
      enumerable: true,
      get: function () {
        const box = readLayoutBoxFromNid(this.__nid);
        return box ? offsetPx(box.offsetLeft) : 0;
      },
    });
    Object.defineProperty(proto, "offsetTop", {
      configurable: true,
      enumerable: true,
      get: function () {
        const box = readLayoutBoxFromNid(this.__nid);
        return box ? offsetPx(box.offsetTop) : 0;
      },
    });
    Object.defineProperty(proto, "clientLeft", {
      configurable: true,
      enumerable: true,
      get: function () {
        const box = readLayoutBoxFromNid(this.__nid);
        return box ? offsetPx(box.clientLeft ?? box.borderWidth) : 0;
      },
    });
    Object.defineProperty(proto, "clientTop", {
      configurable: true,
      enumerable: true,
      get: function () {
        const box = readLayoutBoxFromNid(this.__nid);
        return box ? offsetPx(box.clientTop ?? box.borderWidth) : 0;
      },
    });
    Object.defineProperty(proto, "offsetParent", {
      configurable: true,
      enumerable: true,
      get: function () {
        // offsetLeft/Top are the real subset; offsetParent may be null without a node cache.
        const box = readLayoutBoxFromNid(this.__nid);
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
    Object.defineProperty(proto, "scrollTop", {
      configurable: true,
      enumerable: true,
      get: function () {
        const nid = this.__nid;
        if (nid == null || !Number.isFinite(Number(nid))) {
          const v = this.__nanaScrollTop;
          return Number.isFinite(v) ? v : 0;
        }
        try {
          const off = hostCall("getScrollOffset", [Number(nid)]);
          const n = Number(off && (off.scrollTop != null ? off.scrollTop : off.y));
          return Number.isFinite(n) ? n : 0;
        } catch (_err) {
          const v = this.__nanaScrollTop;
          return Number.isFinite(v) ? v : 0;
        }
      },
      set: function (next) {
        const n = Number(next);
        const value = Number.isFinite(n) ? Math.max(0, n) : 0;
        this.__nanaScrollTop = value;
        const nid = this.__nid;
        if (nid == null || !Number.isFinite(Number(nid))) return;
        try {
          const off = hostCall("getScrollOffset", [Number(nid)]) || {};
          const x = Number(off.scrollLeft != null ? off.scrollLeft : off.x) || 0;
          hostCall("setScrollOffset", [Number(nid), x, value]);
        } catch (_err) {}
      },
    });
    Object.defineProperty(proto, "scrollLeft", {
      configurable: true,
      enumerable: true,
      get: function () {
        const nid = this.__nid;
        if (nid == null || !Number.isFinite(Number(nid))) {
          const v = this.__nanaScrollLeft;
          return Number.isFinite(v) ? v : 0;
        }
        try {
          const off = hostCall("getScrollOffset", [Number(nid)]);
          const n = Number(off && (off.scrollLeft != null ? off.scrollLeft : off.x));
          return Number.isFinite(n) ? n : 0;
        } catch (_err) {
          const v = this.__nanaScrollLeft;
          return Number.isFinite(v) ? v : 0;
        }
      },
      set: function (next) {
        const n = Number(next);
        const value = Number.isFinite(n) ? Math.max(0, n) : 0;
        this.__nanaScrollLeft = value;
        const nid = this.__nid;
        if (nid == null || !Number.isFinite(Number(nid))) return;
        try {
          const off = hostCall("getScrollOffset", [Number(nid)]) || {};
          const y = Number(off.scrollTop != null ? off.scrollTop : off.y) || 0;
          hostCall("setScrollOffset", [Number(nid), value, y]);
        } catch (_err) {}
      },
    });
    // Also install scrollIntoView on Element.prototype for wrapNode / query results.
    proto.scrollIntoView = function (arg) {
      const nid = this.__nid;
      if (nid == null || !Number.isFinite(Number(nid))) return;
      let opts = { block: "start", inline: "nearest" };
      if (arg === false) opts = { block: "end", inline: "nearest" };
      else if (arg && typeof arg === "object") {
        opts = {
          block: arg.block != null ? String(arg.block) : "start",
          inline: arg.inline != null ? String(arg.inline) : "nearest",
        };
      }
      try {
        hostCall("scrollIntoView", [Number(nid), opts]);
      } catch (_err) {}
    };
    proto.__nanaLayoutMetricsInstalled = true;
  }
  installElementLayoutMetrics(globalThis.Element.prototype);
  if (typeof globalThis.ResizeObserver === "undefined") {
    const activeResizeObservers = [];

    function readObservedBox(target) {
      if (!target || typeof target !== "object") return null;
      const nid = target.__nid;
      if (typeof nid === "number" && Number.isFinite(nid)) {
        try {
          const box = hostCall("layoutBox", [nid]);
          if (box && typeof box === "object") {
            return {
              x: Number(box.x) || 0,
              y: Number(box.y) || 0,
              width: Number(box.width) || 0,
              height: Number(box.height) || 0,
            };
          }
        } catch (_err) {}
      }
      if (typeof target.getBoundingClientRect === "function") {
        try {
          const r = target.getBoundingClientRect();
          if (r && typeof r === "object") {
            return {
              x: Number(r.x != null ? r.x : r.left) || 0,
              y: Number(r.y != null ? r.y : r.top) || 0,
              width: Number(r.width) || 0,
              height: Number(r.height) || 0,
            };
          }
        } catch (_err) {}
      }
      return null;
    }

    function resizeEntry(target, box) {
      const w = box.width;
      const h = box.height;
      return {
        target: target,
        contentRect: {
          x: 0,
          y: 0,
          width: w,
          height: h,
          top: 0,
          left: 0,
          bottom: h,
          right: w,
        },
        borderBoxSize: [{ inlineSize: w, blockSize: h }],
        contentBoxSize: [{ inlineSize: w, blockSize: h }],
        devicePixelContentBoxSize: [{ inlineSize: w, blockSize: h }],
      };
    }

    function sizeKey(box) {
      return String(box.width) + "x" + String(box.height);
    }

    function deliverResizeObserver(obs) {
      if (!obs || typeof obs._cb !== "function" || !obs._targets || !obs._targets.length) {
        return;
      }
      const entries = [];
      for (let i = 0; i < obs._targets.length; i++) {
        const target = obs._targets[i];
        const box = readObservedBox(target);
        if (!box) continue;
        const key = sizeKey(box);
        if (obs._lastKeys.get(target) === key) continue;
        obs._lastKeys.set(target, key);
        entries.push(resizeEntry(target, box));
      }
      if (!entries.length) return;
      try {
        obs._cb(entries, obs);
      } catch (_err) {}
    }

    function scheduleResizeObserver(obs) {
      queueMicrotask(function () {
        deliverResizeObserver(obs);
      });
      try {
        requestAnimationFrame(function () {
          deliverResizeObserver(obs);
        });
      } catch (_err) {
        deliverResizeObserver(obs);
      }
    }

    globalThis.ResizeObserver = function ResizeObserver(cb) {
      this._cb = cb;
      this._targets = [];
      this._lastKeys = new Map();
      this._windowId = Number(globalThis.__nanaActiveWindowId || 0);
      this.observe = function (target) {
        if (!target || typeof target !== "object") return;
        if (this._targets.indexOf(target) >= 0) return;
        this._targets.push(target);
        if (activeResizeObservers.indexOf(this) < 0) {
          activeResizeObservers.push(this);
        }
        scheduleResizeObserver(this);
      };
      this.unobserve = function (target) {
        const idx = this._targets.indexOf(target);
        if (idx >= 0) this._targets.splice(idx, 1);
        this._lastKeys.delete(target);
        if (!this._targets.length) {
          const oi = activeResizeObservers.indexOf(this);
          if (oi >= 0) activeResizeObservers.splice(oi, 1);
        }
      };
      this.disconnect = function () {
        this._targets = [];
        this._lastKeys = new Map();
        const oi = activeResizeObservers.indexOf(this);
        if (oi >= 0) activeResizeObservers.splice(oi, 1);
      };
    };

    // Host pump calls this after resolve_layout so observers see fresh layoutBox.
    globalThis.__nanaNotifyLayout = function __nanaNotifyLayout() {
      for (let i = 0; i < activeResizeObservers.length; i++) {
        deliverResizeObserver(activeResizeObservers[i]);
      }
      return activeResizeObservers.length;
    };
    globalThis.__nanaReleaseWindowObservers = function __nanaReleaseWindowObservers(windowId) {
      const id = Number(windowId || 0);
      let released = 0;
      for (let i = activeResizeObservers.length - 1; i >= 0; i--) {
        const observer = activeResizeObservers[i];
        if (observer && observer._windowId === id) {
          observer.disconnect();
          released++;
        }
      }
      return released;
    };
  }
  if (typeof globalThis.MutationObserver === "undefined") {
    globalThis.MutationObserver = function MutationObserver() {
      throw new DOMException("MutationObserver is not implemented by the Nana DOM subset", "NotSupportedError");
    };
  }
  if (typeof globalThis.IntersectionObserver === "undefined") {
    globalThis.IntersectionObserver = function IntersectionObserver() {
      throw new DOMException("IntersectionObserver is not implemented by the Nana layout subset", "NotSupportedError");
    };
  }
  if (typeof globalThis.Intl === "undefined") {
    globalThis.Intl = {};
  }
  if (typeof globalThis.Intl.NumberFormat !== "function") {
    globalThis.Intl.NumberFormat = function NumberFormat(locale, options) {
      const loc = String(locale || "en");
      const opts = options || {};
      this.format = function (value) {
        const n = Number(value);
        if (!Number.isFinite(n)) return String(value);
        if (opts.notation === "compact") {
          const abs = Math.abs(n);
          const zh = /^zh\b/i.test(loc);
          if (zh) {
            if (abs >= 1e8) return trim1(n / 1e8) + "亿";
            if (abs >= 1e4) return trim1(n / 1e4) + "万";
          } else {
            if (abs >= 1e9) return trim1(n / 1e9) + "B";
            if (abs >= 1e6) return trim1(n / 1e6) + "M";
            if (abs >= 1e3) return trim1(n / 1e3) + "K";
          }
          return String(Math.round(n));
        }
        if (opts.style === "percent") {
          return String(Math.round(n * 100)) + "%";
        }
        if (typeof opts.maximumFractionDigits === "number") {
          return n.toFixed(opts.maximumFractionDigits);
        }
        return String(Math.round(n));
      };
      function trim1(v) {
        return v.toFixed(1).replace(/\.0$/, "");
      };
    };
  }
  if (typeof globalThis.Intl.DateTimeFormat !== "function") {
    globalThis.Intl.DateTimeFormat = function DateTimeFormat(locale, options) {
      const loc = String(locale || "en");
      const opts = options || {};
      this.format = function (date) {
        const d = date instanceof Date ? date : new Date(date);
        if (Number.isNaN(d.getTime())) return String(date);
        const y = d.getFullYear();
        const m = String(d.getMonth() + 1).padStart(2, "0");
        const day = String(d.getDate()).padStart(2, "0");
        if (/^zh\b/i.test(loc)) {
          if (opts.dateStyle === "medium" || opts.year) return y + "年" + m + "月" + day + "日";
          return y + "/" + m + "/" + day;
        }
        return y + "/" + m + "/" + day;
      };
    };
  }
