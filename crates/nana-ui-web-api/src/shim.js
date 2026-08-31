/**
 * NanaUI progressive Web API shim (not a WebView).
 * Requires globalThis.__nanaHost.call for storage / timers / documentElement sync.
 */
(function installNanaWebApiShim() {
  "use strict";

  if (globalThis.__nanaWebApi && globalThis.__nanaWebApi.installed) {
    return;
  }

  function hostCall(name, args) {
    const host = globalThis.__nanaHost;
    if (!host || typeof host.call !== "function") {
      throw new Error("__nanaHost.call missing for web-api `" + name + "`");
    }
    const values = Array.isArray(args) ? args : [];
    let windowId = Number(globalThis.__nanaActiveWindowId || 0);
    if (!windowId && values.length) {
      const first = Number(values[0]);
      if (Number.isSafeInteger(first) && first >= 4294967296) {
        windowId = Math.floor(first / 4294967296);
      }
    }
    if (windowId && name !== "windowCall") {
      return host.call("windowCall", [windowId, String(name), values]);
    }
    return host.call(name, values);
  }

  function withWindowContext(windowId, action) {
    const previous = Number(globalThis.__nanaActiveWindowId || 0);
    globalThis.__nanaActiveWindowId = Number(windowId || 0);
    try {
      return action();
    } finally {
      globalThis.__nanaActiveWindowId = previous;
    }
  }

  if (typeof globalThis.queueMicrotask !== "function") {
    globalThis.queueMicrotask = function queueMicrotask(fn) {
      Promise.resolve().then(fn);
    };
  }

  if (typeof globalThis.process === "undefined") {
    globalThis.process = { env: { NODE_ENV: "production" } };
  } else if (!globalThis.process.env) {
    globalThis.process.env = { NODE_ENV: "production" };
  }
  if (typeof globalThis.process.env.DEV === "undefined") {
    globalThis.process.env.DEV = "true";
  }
  if (typeof globalThis.process.env.MODE === "undefined") {
    globalThis.process.env.MODE = "development";
  }

  // Generic Vite-style import.meta.env for IIFE / eval hosts.
  if (typeof globalThis.__nanaImportMeta === "undefined") {
    globalThis.__nanaImportMeta = {
      env: {
        DEV: true,
        PROD: false,
        MODE: "development",
        SSR: false,
        BASE_URL: "/",
      },
      url: "nana://app/main.js",
    };
  }

  globalThis.__nanaConsoleErrors = globalThis.__nanaConsoleErrors || [];
  function captureConsole(level, args) {
    try {
      const parts = [];
      for (let i = 0; i < args.length; i++) {
        const a = args[i];
        if (a == null) parts.push(String(a));
        else if (typeof a === "string") parts.push(a);
        else if (a && typeof a.message === "string") {
          parts.push(a.message + (a.stack ? "\n" + a.stack : ""));
        } else {
          try {
            parts.push(JSON.stringify(a));
          } catch (_e) {
            parts.push(String(a));
          }
        }
      }
      const line = "[" + level + "] " + parts.join(" ");
      globalThis.__nanaConsoleErrors.push(line);
      // Keep a short ring so remount spam stays inspectable.
      if (globalThis.__nanaConsoleErrors.length > 40) {
        globalThis.__nanaConsoleErrors.splice(0, globalThis.__nanaConsoleErrors.length - 40);
      }
    } catch (_err) {}
  }
  globalThis.__nanaDumpConsoleErrors = function __nanaDumpConsoleErrors() {
    const list = globalThis.__nanaConsoleErrors || [];
    return list.slice(-20).join("\n---\n");
  };
  if (!globalThis.__nanaConsoleCaptureInstalled) {
    globalThis.__nanaConsoleCaptureInstalled = true;
    const prev = typeof globalThis.console !== "undefined" ? globalThis.console : null;
    function bindPrev(name) {
      return prev && typeof prev[name] === "function" ? prev[name].bind(prev) : function () {};
    }
    globalThis.console = {
      log: bindPrev("log"),
      info: bindPrev("info"),
      debug: bindPrev("debug"),
      trace: bindPrev("trace"),
      warn: function () {
        captureConsole("warn", arguments);
        if (prev && typeof prev.warn === "function") prev.warn.apply(prev, arguments);
      },
      error: function () {
        captureConsole("error", arguments);
        if (prev && typeof prev.error === "function") prev.error.apply(prev, arguments);
      },
    };
  }

  if (typeof globalThis.TextEncoder === "undefined") {
    globalThis.TextEncoder = function TextEncoder() {
      this.encode = function (str) {
        const s = String(str ?? "");
        const bytes = [];
        for (let i = 0; i < s.length; i++) {
          let code = s.charCodeAt(i);
          if (code < 0x80) bytes.push(code);
          else if (code < 0x800) {
            bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
          } else if (code >= 0xd800 && code <= 0xdbff && i + 1 < s.length) {
            const next = s.charCodeAt(++i);
            const cp = 0x10000 + ((code - 0xd800) << 10) + (next - 0xdc00);
            bytes.push(
              0xf0 | (cp >> 18),
              0x80 | ((cp >> 12) & 0x3f),
              0x80 | ((cp >> 6) & 0x3f),
              0x80 | (cp & 0x3f),
            );
          } else {
            bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
          }
        }
        return Uint8Array.from(bytes);
      };
    };
  }
  if (typeof globalThis.TextDecoder === "undefined") {
    globalThis.TextDecoder = function TextDecoder() {
      this.decode = function (input) {
        const bytes = input instanceof Uint8Array ? input : new Uint8Array(input || []);
        let out = "";
        for (let i = 0; i < bytes.length; ) {
          const b = bytes[i++];
          if (b < 0x80) out += String.fromCharCode(b);
          else if (b < 0xe0) {
            const b2 = bytes[i++];
            out += String.fromCharCode(((b & 0x1f) << 6) | (b2 & 0x3f));
          } else if (b < 0xf0) {
            const b2 = bytes[i++];
            const b3 = bytes[i++];
            out += String.fromCharCode(((b & 0x0f) << 12) | ((b2 & 0x3f) << 6) | (b3 & 0x3f));
          } else {
            const b2 = bytes[i++];
            const b3 = bytes[i++];
            const b4 = bytes[i++];
            let cp = ((b & 0x07) << 18) | ((b2 & 0x3f) << 12) | ((b3 & 0x3f) << 6) | (b4 & 0x3f);
            cp -= 0x10000;
            out += String.fromCharCode(0xd800 + (cp >> 10), 0xdc00 + (cp & 0x3ff));
          }
        }
        return out;
      };
    };
  }
  if (typeof globalThis.URL === "undefined") {
    globalThis.URL = function URL(path, base) {
      this.href = String(path || "");
      this.pathname = String(path || "/");
      this.search = "";
      this.hash = "";
      this.origin = "nana://app";
      if (base) this.href = String(base).replace(/\/$/, "") + "/" + String(path || "").replace(/^\//, "");
    };
    globalThis.URL.createObjectURL = function () {
      return "nana://blob";
    };
    globalThis.URL.revokeObjectURL = function () {};
  }

  if (typeof globalThis.structuredClone !== "function") {
    globalThis.structuredClone = function structuredClone() {
      throw new DOMException("structuredClone is not implemented by this Nana runtime", "NotSupportedError");
    };
  }

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

  function isListenerObject(listener) {
    return (
      typeof listener === "function" ||
      (listener != null && typeof listener.handleEvent === "function")
    );
  }

  function invokeListenerEntry(entry, event, currentTarget) {
    if (typeof entry.listener === "function") {
      entry.listener.call(currentTarget, event);
    } else {
      entry.listener.handleEvent(event);
    }
  }

  function EventTargetShim() {
    this._listeners = Object.create(null);
  }
  EventTargetShim.prototype.addEventListener = function (type, listener, options) {
    if (!isListenerObject(listener)) return;
    const opts = normalizeListenerOptions(options);
    const key = String(type);
    if (!this._listeners[key]) this._listeners[key] = [];
    const list = this._listeners[key];
    for (let i = 0; i < list.length; i++) {
      if (list[i].listener === listener && list[i].capture === opts.capture) return;
    }
    list.push({
      listener: listener,
      capture: opts.capture,
      once: opts.once,
      passive: opts.passive,
    });
  };
  EventTargetShim.prototype.removeEventListener = function (type, listener, options) {
    const capture = normalizeListenerOptions(options).capture;
    const key = String(type);
    const list = this._listeners[key];
    if (!list) return;
    this._listeners[key] = list.filter(function (entry) {
      return !(entry.listener === listener && entry.capture === capture);
    });
  };
  /** Phase-only invoke used by Nana fan-out (window ↔ document ↔ target). */
  EventTargetShim.prototype.__nanaInvokePhase = function (type, event, capture) {
    const list = this._listeners[String(type)];
    if (!list || !list.length) return;
    const snapshot = list.slice();
    for (let i = 0; i < snapshot.length; i++) {
      const entry = snapshot[i];
      if (entry.capture !== !!capture) continue;
      if (event && event._immediateStopped) break;
      try {
        if (event) {
          event.currentTarget = this;
          event.eventPhase = capture ? 1 : 3;
        }
        invokeListenerEntry(entry, event, this);
      } catch (_err) {}
      if (entry.once) {
        this.removeEventListener(type, entry.listener, entry.capture);
      }
    }
  };
  EventTargetShim.prototype.dispatchEvent = function (event) {
    const type = event && event.type != null ? String(event.type) : "";
    if (event) {
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
      if (typeof event.preventDefault !== "function") {
        event.defaultPrevented = !!event.defaultPrevented;
        event.preventDefault = function () {
          this.defaultPrevented = true;
        };
      }
    }
    this.__nanaInvokePhase(type, event, true);
    if (!(event && event._immediateStopped)) {
      const handler = this["on" + type];
      if (typeof handler === "function") {
        if (event) {
          event.target = event.target || this;
          event.currentTarget = this;
          event.eventPhase = 2;
        }
        try { handler.call(this, event); } catch (_err) {}
      }
    }
    if (!(event && event._stopped)) {
      this.__nanaInvokePhase(type, event, false);
    }
    return !(event && event.defaultPrevented);
  };

  function CustomEventShim(type, init) {
    this.type = String(type);
    this.detail = init && "detail" in init ? init.detail : null;
    this.bubbles = !!(init && init.bubbles);
    this.cancelable = !!(init && init.cancelable);
    this.defaultPrevented = false;
  }
  CustomEventShim.prototype.preventDefault = function () {
    this.defaultPrevented = true;
  };
  CustomEventShim.prototype.stopPropagation = function () {};

  function StorageShim(bucket) {
    this._bucket = String(bucket || "local");
  }
  StorageShim.prototype.getItem = function (key) {
    const v = hostCall("storageGet", [this._bucket, String(key)]);
    return v == null ? null : String(v);
  };
  StorageShim.prototype.setItem = function (key, value) {
    hostCall("storageSet", [this._bucket, String(key), String(value)]);
  };
  StorageShim.prototype.removeItem = function (key) {
    hostCall("storageRemove", [this._bucket, String(key)]);
  };
  StorageShim.prototype.clear = function () {
    hostCall("storageClear", [this._bucket]);
  };
  StorageShim.prototype.key = function (index) {
    const keys = hostCall("storageKeys", [this._bucket]) || [];
    return keys[index] != null ? String(keys[index]) : null;
  };
  Object.defineProperty(StorageShim.prototype, "length", {
    get: function () {
      return (hostCall("storageKeys", [this._bucket]) || []).length;
    },
  });

  const rafCallbacks = new Map();
  let nextRafId = 1;
  const timeoutCallbacks = new Map();
  let nextTimeoutId = 1;
  const intervalCallbacks = new Map();
  let nextIntervalId = 1;

  function requestAnimationFrame(cb) {
    if (typeof cb !== "function") throw new TypeError("rAF callback required");
    const id = nextRafId++;
    rafCallbacks.set(id, { cb: cb, windowId: Number(globalThis.__nanaActiveWindowId || 0) });
    hostCall("rafSchedule", [id]);
    return id;
  }
  function cancelAnimationFrame(id) {
    rafCallbacks.delete(Number(id));
    hostCall("rafCancel", [Number(id)]);
  }
  function setTimeoutShim(cb, delay) {
    const id = nextTimeoutId++;
    timeoutCallbacks.set(id, {
      cb: typeof cb === "function" ? cb : function () {},
      windowId: Number(globalThis.__nanaActiveWindowId || 0),
    });
    hostCall("timeoutSchedule", [id, Math.max(0, Number(delay) || 0)]);
    return id;
  }
  function clearTimeoutShim(id) {
    timeoutCallbacks.delete(Number(id));
    hostCall("timeoutCancel", [Number(id)]);
  }
  function setIntervalShim(cb, delay) {
    const id = nextIntervalId++;
    intervalCallbacks.set(id, {
      cb: typeof cb === "function" ? cb : function () {},
      delay: Math.max(0, Number(delay) || 0),
      windowId: Number(globalThis.__nanaActiveWindowId || 0),
    });
    hostCall("intervalSchedule", [id, Math.max(0, Number(delay) || 0)]);
    return id;
  }
  function clearIntervalShim(id) {
    intervalCallbacks.delete(Number(id));
    hostCall("intervalCancel", [Number(id)]);
  }

  globalThis.__nanaDrainTimers = function __nanaDrainTimers(payload) {
    const now = (payload && payload.now) || Date.now();
    const rafIds = (payload && payload.raf) || [];
    for (let i = 0; i < rafIds.length; i++) {
      const id = Number(rafIds[i]);
      const entry = rafCallbacks.get(id);
      rafCallbacks.delete(id);
      if (entry && typeof entry.cb === "function") {
        try {
          withWindowContext(entry.windowId, function () { entry.cb(now); });
        } catch (_err) {}
      }
    }
    const timeoutIds = (payload && payload.timeouts) || [];
    for (let j = 0; j < timeoutIds.length; j++) {
      const id = Number(timeoutIds[j]);
      const entry = timeoutCallbacks.get(id);
      timeoutCallbacks.delete(id);
      if (entry && typeof entry.cb === "function") {
        try {
          withWindowContext(entry.windowId, function () { entry.cb(); });
        } catch (_err) {}
      }
    }
    const intervalIds = (payload && payload.intervals) || [];
    for (let k = 0; k < intervalIds.length; k++) {
      const id = Number(intervalIds[k]);
      const entry = intervalCallbacks.get(id);
      if (entry && typeof entry.cb === "function") {
        try {
          withWindowContext(entry.windowId, function () { entry.cb(); });
        } catch (_err) {}
        withWindowContext(entry.windowId, function () {
          hostCall("intervalSchedule", [id, entry.delay]);
        });
      }
    }
    return true;
  };

  function DatasetProxy(target) {
    // Avoid Proxy — keep QuickJS/V8 surface identical for dataset writes.
    return {
      get theme() {
        return target._dataset.theme;
      },
      set theme(v) {
        target._dataset.theme = String(v);
        hostCall("documentElementSet", ["dataset", "theme", String(v)]);
      },
      get backdrop() {
        return target._dataset.backdrop;
      },
      set backdrop(v) {
        target._dataset.backdrop = String(v);
        hostCall("documentElementSet", ["dataset", "backdrop", String(v)]);
      },
      get backdropTarget() {
        return target._dataset.backdropTarget;
      },
      set backdropTarget(v) {
        target._dataset.backdropTarget = String(v);
        hostCall("documentElementSet", ["dataset", "backdropTarget", String(v)]);
      },
      get titlebarFollowsSidebar() {
        return target._dataset.titlebarFollowsSidebar;
      },
      set titlebarFollowsSidebar(v) {
        target._dataset.titlebarFollowsSidebar = String(v);
        hostCall("documentElementSet", [
          "dataset",
          "titlebarFollowsSidebar",
          String(v),
        ]);
      },
      get corners() {
        return target._dataset.corners;
      },
      set corners(v) {
        target._dataset.corners = String(v);
        hostCall("documentElementSet", ["dataset", "corners", String(v)]);
      },
      get platform() {
        return target._dataset.platform;
      },
      set platform(v) {
        target._dataset.platform = String(v);
        hostCall("documentElementSet", ["dataset", "platform", String(v)]);
      },
    };
  }

  function StyleProxy(target) {
    return {
      setProperty: function (name, value) {
        target._style[String(name)] = String(value);
        hostCall("documentElementSet", ["style", String(name), String(value)]);
      },
      getPropertyValue: function (name) {
        return target._style[String(name)] || "";
      },
      removeProperty: function (name) {
        delete target._style[String(name)];
        hostCall("documentElementSet", ["style", String(name), null]);
      },
    };
  }

  function DocumentElement() {
    this.tagName = "HTML";
    this._dataset = Object.create(null);
    this._style = Object.create(null);
    this.dataset = DatasetProxy(this);
    this.style = StyleProxy(this);
    this.classList = {
      _set: new Set(),
      add: function () {
        for (let i = 0; i < arguments.length; i++) this._set.add(String(arguments[i]));
      },
      remove: function () {
        for (let i = 0; i < arguments.length; i++) this._set.delete(String(arguments[i]));
      },
      contains: function (c) {
        return this._set.has(String(c));
      },
      toggle: function (c, force) {
        const key = String(c);
        if (force === true || (!this._set.has(key) && force !== false)) {
          this._set.add(key);
          return true;
        }
        this._set.delete(key);
        return false;
      },
    };
  }

  /**
   * Stable `document.body` for Vue Teleport `to="body"`.
   * Must return the same wrapNode identity as hostOps.querySelector("body")
   * and mountRoot — Nana mount-root, not a fake DOM portal.
   */
  function mountBodyNode() {
    try {
      const id = hostCall("querySelector", ["body"]);
      if (id == null) return null;
      return wrapHostNode(id, "body");
    } catch (_err) {
      return null;
    }
  }

  /** Teleport target tag hint so body/html keep stable metadata across accessors. */
  function teleportTargetTag(sel) {
    const lower = String(sel ?? "")
      .trim()
      .toLowerCase();
    return lower === "body" || lower === "html" ? lower : null;
  }

  function DocumentShim() {
    EventTargetShim.call(this);
    this.documentElement = new DocumentElement();
    this._bodyFallback = {
      tagName: "BODY",
      style: {},
      classList: {
        add() {},
        remove() {},
        contains() {
          return false;
        },
      },
    };
    this.head = { tagName: "HEAD", appendChild() {}, removeChild() {} };
    this.readyState = "complete";
    this.visibilityState = "visible";
    this.hidden = false;
    this.title = "";
    this.scrollingElement = { scrollTop: 0, scrollLeft: 0, clientWidth: 800, clientHeight: 600 };
  }
  DocumentShim.prototype = Object.create(EventTargetShim.prototype);
  DocumentShim.prototype.constructor = DocumentShim;
  Object.defineProperty(DocumentShim.prototype, "body", {
    configurable: true,
    enumerable: true,
    get: function () {
      return mountBodyNode() || this._bodyFallback;
    },
  });
  // Fallback identity cache before `__nanaWrapNode` is installed (Teleport may
  // resolve `to="body"` early). Once wrapNode is live, prefer its nodeCache.
  var hostNodeCache = new Map();
  function wrapHostNode(id, tagHint) {
    if (typeof globalThis.__nanaWrapNode === "function") {
      return globalThis.__nanaWrapNode(id, "element", tagHint || null);
    }
    const nid = Number(id);
    if (!Number.isFinite(nid)) return null;
    const cached = hostNodeCache.get(nid);
    if (cached) {
      if (tagHint && !cached.tag) {
        cached.tag = tagHint;
        cached.tagName = String(tagHint).toUpperCase();
        cached.nodeName = cached.tagName;
      }
      return cached;
    }
    const node = Object.create(globalThis.HTMLElement.prototype);
    node.__nid = nid;
    node.tag = tagHint || null;
    node.tagName = tagHint ? String(tagHint).toUpperCase() : "DIV";
    node.nodeName = node.tagName;
    node.nodeType = 1;
    node.style = {};
    node.dataset = {};
    node.attributes = {};
    node.getBoundingClientRect = function () {
      return { x: 0, y: 0, width: 0, height: 0, top: 0, left: 0, bottom: 0, right: 0 };
    };
    node.querySelector = function (sel) {
      try {
        const found = hostCall("querySelector", [String(sel ?? "")]);
        return found == null ? null : wrapHostNode(found, teleportTargetTag(sel));
      } catch (_err) {
        return null;
      }
    };
    node.querySelectorAll = function (sel) {
      try {
        const ids = hostCall("querySelectorAll", [String(sel ?? "")]) || [];
        const tag = teleportTargetTag(sel);
        return Array.from(ids, function (id) {
          return wrapHostNode(id, tag);
        });
      } catch (_err) {
        return [];
      }
    };
    node.closest = function (sel) {
      try {
        const found = hostCall("closest", [nid, String(sel ?? "")]);
        return found == null ? null : wrapHostNode(found, teleportTargetTag(sel));
      } catch (_err) {
        return null;
      }
    };
    node.getAttribute = function (name) {
      try {
        const v = hostCall("getAttribute", [nid, String(name)]);
        return v == null ? null : String(v);
      } catch (_err) {
        return null;
      }
    };
    node.scrollIntoView = function (arg) {
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
    hostNodeCache.set(nid, node);
    return node;
  }

  // Minimal HTML *fragment* parser for template.innerHTML (Markdown sanitize path).
  // Honest subset: common tags + text + attrs; comments skipped; NOT a full HTML5 parser.
  var HTML_VOID_TAGS = {
    area: 1,
    base: 1,
    br: 1,
    col: 1,
    embed: 1,
    hr: 1,
    img: 1,
    input: 1,
    link: 1,
    meta: 1,
    param: 1,
    source: 1,
    track: 1,
    wbr: 1,
  };

  function decodeHtmlEntities(text) {
    return String(text)
      .replace(/&lt;/gi, "<")
      .replace(/&gt;/gi, ">")
      .replace(/&quot;/gi, '"')
      .replace(/&#39;|&apos;/gi, "'")
      .replace(/&nbsp;/gi, "\u00A0")
      .replace(/&#x([0-9a-f]+);/gi, function (_m, hex) {
        return String.fromCharCode(parseInt(hex, 16));
      })
      .replace(/&#(\d+);/g, function (_m, dec) {
        return String.fromCharCode(parseInt(dec, 10));
      })
      .replace(/&amp;/gi, "&");
  }

  function escapeHtmlText(text) {
    return String(text)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  function escapeHtmlAttr(text) {
    return escapeHtmlText(text).replace(/"/g, "&quot;");
  }

  function collectNodeText(node) {
    if (!node) return "";
    if (node.nodeType === 3) return String(node.textContent ?? node.data ?? "");
    var out = "";
    var kids = node.childNodes || [];
    for (var i = 0; i < kids.length; i++) out += collectNodeText(kids[i]);
    return out;
  }

  function serializeHtmlNodes(nodes) {
    var out = "";
    for (var i = 0; i < (nodes || []).length; i++) out += serializeHtmlNode(nodes[i]);
    return out;
  }

  function serializeHtmlNode(node) {
    if (!node) return "";
    if (node.nodeType === 3) return escapeHtmlText(node.textContent ?? node.data ?? "");
    if (node.nodeType !== 1) return "";
    var tag = String(node.tagName || "").toLowerCase();
    var open = "<" + tag;
    var attrs = node._attrs || node.attributes || [];
    for (var i = 0; i < attrs.length; i++) {
      var a = attrs[i];
      if (!a || a.name == null) continue;
      open += " " + a.name + '="' + escapeHtmlAttr(a.value == null ? "" : a.value) + '"';
    }
    if (HTML_VOID_TAGS[tag]) return open + ">";
    return open + ">" + serializeHtmlNodes(node.childNodes) + "</" + tag + ">";
  }

  function parseHtmlAttributes(attrStr) {
    var attrs = [];
    var re = /([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
    var m;
    while ((m = re.exec(String(attrStr || "")))) {
      var name = m[1];
      if (!name || name === "/") continue;
      var value = m[2] != null ? m[2] : m[3] != null ? m[3] : m[4] != null ? m[4] : "";
      attrs.push({ name: name, value: decodeHtmlEntities(value) });
    }
    return attrs;
  }

  function createFragmentTextNode(text, parent) {
    return {
      nodeType: 3,
      nodeName: "#text",
      textContent: String(text ?? ""),
      data: String(text ?? ""),
      parentNode: parent || null,
      childNodes: [],
    };
  }

  function createFragmentElement(tagName, attrs, parent) {
    var el = Object.create(
      (globalThis.HTMLElement && globalThis.HTMLElement.prototype) || null,
    );
    el.tagName = String(tagName).toUpperCase();
    el.nodeName = el.tagName;
    el.nodeType = 1;
    el.parentNode = parent || null;
    el.childNodes = [];
    el._attrs = [];
    Object.defineProperty(el, "attributes", {
      configurable: true,
      enumerable: true,
      get: function () {
        return this._attrs;
      },
    });
    el.getAttribute = function (name) {
      var key = String(name).toLowerCase();
      for (var i = 0; i < this._attrs.length; i++) {
        if (String(this._attrs[i].name).toLowerCase() === key) {
          return this._attrs[i].value;
        }
      }
      return null;
    };
    el.setAttribute = function (name, value) {
      var n = String(name);
      var v = String(value);
      var key = n.toLowerCase();
      for (var i = 0; i < this._attrs.length; i++) {
        if (String(this._attrs[i].name).toLowerCase() === key) {
          this._attrs[i].name = n;
          this._attrs[i].value = v;
          return;
        }
      }
      this._attrs.push({ name: n, value: v });
    };
    el.removeAttribute = function (name) {
      var key = String(name).toLowerCase();
      this._attrs = this._attrs.filter(function (a) {
        return String(a.name).toLowerCase() !== key;
      });
    };
    el.remove = function () {
      var p = this.parentNode;
      if (!p || !p.childNodes) return;
      var idx = p.childNodes.indexOf(this);
      if (idx >= 0) p.childNodes.splice(idx, 1);
      this.parentNode = null;
    };
    el.replaceWith = function () {
      var p = this.parentNode;
      if (!p || !p.childNodes) return;
      var idx = p.childNodes.indexOf(this);
      if (idx < 0) return;
      var incoming = Array.prototype.slice.call(arguments);
      for (var j = 0; j < incoming.length; j++) {
        if (incoming[j]) incoming[j].parentNode = p;
      }
      var args = [idx, 1].concat(incoming);
      Array.prototype.splice.apply(p.childNodes, args);
      this.parentNode = null;
    };
    Object.defineProperty(el, "textContent", {
      configurable: true,
      enumerable: true,
      get: function () {
        return collectNodeText(this);
      },
      set: function (v) {
        this.childNodes = [];
        var tn = createFragmentTextNode(String(v ?? ""), this);
        this.childNodes.push(tn);
      },
    });
    Object.defineProperty(el, "innerHTML", {
      configurable: true,
      enumerable: true,
      get: function () {
        return serializeHtmlNodes(this.childNodes);
      },
      set: function (v) {
        var kids = parseHtmlFragment(String(v ?? ""));
        for (var i = 0; i < kids.length; i++) kids[i].parentNode = this;
        this.childNodes = kids;
      },
    });
    var list = attrs || [];
    for (var a = 0; a < list.length; a++) {
      el.setAttribute(list[a].name, list[a].value);
    }
    return el;
  }

  function parseHtmlFragment(html) {
    var root = { childNodes: [], parentNode: null, nodeType: 11 };
    var stack = [root];
    var s = String(html ?? "");
    var i = 0;
    while (i < s.length) {
      if (s.charCodeAt(i) === 60) {
        if (s.slice(i, i + 4) === "<!--") {
          var cend = s.indexOf("-->", i + 4);
          i = cend < 0 ? s.length : cend + 3;
          continue;
        }
        if (s.slice(i, i + 2) === "<!" || s.slice(i, i + 2) === "<?") {
          var bang = s.indexOf(">", i + 2);
          i = bang < 0 ? s.length : bang + 1;
          continue;
        }
        var close = /^<\/([A-Za-z][\w:-]*)\s*>/.exec(s.slice(i));
        if (close) {
          var closeName = close[1].toLowerCase();
          for (var d = stack.length - 1; d > 0; d--) {
            if (
              stack[d].tagName &&
              String(stack[d].tagName).toLowerCase() === closeName
            ) {
              stack.length = d;
              break;
            }
          }
          i += close[0].length;
          continue;
        }
        var open = /^<([A-Za-z][\w:-]*)((?:\s[^>]*)?)\s*(\/?)\s*>/.exec(s.slice(i));
        if (open) {
          var name = open[1].toLowerCase();
          var attrs = parseHtmlAttributes(open[2] || "");
          var selfClosing = open[3] === "/" || !!HTML_VOID_TAGS[name];
          var parent = stack[stack.length - 1];
          var el = createFragmentElement(name, attrs, parent);
          parent.childNodes.push(el);
          i += open[0].length;
          if (!selfClosing) stack.push(el);
          continue;
        }
        // Malformed `<` — keep as text so we never spin.
        var badParent = stack[stack.length - 1];
        badParent.childNodes.push(createFragmentTextNode("<", badParent));
        i += 1;
        continue;
      }
      var next = s.indexOf("<", i);
      var end = next < 0 ? s.length : next;
      var raw = s.slice(i, end);
      if (raw) {
        var textParent = stack[stack.length - 1];
        textParent.childNodes.push(
          createFragmentTextNode(decodeHtmlEntities(raw), textParent),
        );
      }
      i = end;
      if (next < 0) break;
    }
    return root.childNodes;
  }

  function createTemplateContent() {
    var content = {
      nodeType: 11,
      nodeName: "#document-fragment",
      childNodes: [],
      parentNode: null,
    };
    Object.defineProperty(content, "textContent", {
      configurable: true,
      enumerable: true,
      get: function () {
        return collectNodeText(this);
      },
    });
    return content;
  }

  function resourceId(value) {
    if (value == null) return null;
    if (typeof value === "bigint" || typeof value === "number") return value;
    if (value.__nanaResource && value.id != null) return value.id;
    if (value.__nanaCanvasResource && value.__nanaCanvasResource.id != null) {
      return value.__nanaCanvasResource.id;
    }
    return null;
  }

  function asUint8Array(value) {
    if (ArrayBuffer.isView && ArrayBuffer.isView(value)) {
      return new Uint8Array(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
    }
    if (value instanceof ArrayBuffer) return new Uint8Array(value.slice(0));
    return Uint8Array.from(value || []);
  }

  function ImageDataShim(dataOrWidth, widthOrHeight, maybeHeight) {
    if (typeof dataOrWidth === "number") {
      this.width = Math.max(1, Math.trunc(Number(dataOrWidth) || 1));
      this.height = Math.max(1, Math.trunc(Number(widthOrHeight) || 1));
      this.data = new Uint8ClampedArray(this.width * this.height * 4);
    } else {
      this.data = new Uint8ClampedArray(asUint8Array(dataOrWidth));
      this.width = Math.max(1, Math.trunc(Number(widthOrHeight) || 1));
      this.height = maybeHeight == null
        ? Math.max(1, Math.trunc(this.data.length / (this.width * 4)))
        : Math.max(1, Math.trunc(Number(maybeHeight) || 1));
      if (this.data.length !== this.width * this.height * 4) {
        throw new DOMException("ImageData byte length does not match dimensions", "IndexSizeError");
      }
    }
    this.colorSpace = "srgb";
  }

  function CanvasGradientShim(kind, args) {
    this.kind = kind;
    this.args = Array.from(args, Number);
    this.stops = [];
  }
  CanvasGradientShim.prototype.addColorStop = function (offset, color) {
    const value = Number(offset);
    if (!Number.isFinite(value) || value < 0 || value > 1) {
      throw new DOMException("Color stop offset must be between 0 and 1", "IndexSizeError");
    }
    this.stops.push([value, String(color)]);
    this.stops.sort(function (a, b) { return a[0] - b[0]; });
  };
  EventTargetShim.prototype.__nanaClearListeners = function () {
    this._listeners = Object.create(null);
  };

  function CanvasPatternShim(source, repetition) {
    const id = resourceId(source);
    if (id == null) throw new TypeError("Canvas pattern source has no Nana image resource");
    this.kind = "pattern";
    this.sourceId = id;
    this.repetition = repetition == null || repetition === "" ? "repeat" : String(repetition);
    this.transform = [1, 0, 0, 1, 0, 0];
  }
  CanvasPatternShim.prototype.setTransform = function (matrix) {
    this.transform = [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f].map(Number);
  };

  function CanvasRenderingContext2DShim(canvas) {
    this.canvas = canvas;
    this._id = canvas.__nanaCanvasResource.id;
    this._lineDash = [];
    this._fillStyle = "#000000";
    this._strokeStyle = "#000000";
    this._lineWidth = 1;
    this._lineCap = "butt";
    this._lineJoin = "miter";
    this._lineDashOffset = 0;
    this._globalAlpha = 1;
    this._globalCompositeOperation = "source-over";
    this._font = "10px sans-serif";
  }
  function HTMLCanvasElementShim() {}

  function gpuResourceId(value) {
    if (value == null) return null;
    if (typeof value === "bigint" || typeof value === "number") return value;
    const resource = value.__nanaGpuResource || value;
    return resource && resource.id != null ? resource.id : null;
  }

  function GPUObjectBase(resource) {
    this.__nanaGpuResource = resource;
    this.id = resource.id;
    this.label = resource.label || "";
    this.generation = resource.generation;
  }
  GPUObjectBase.prototype.destroy = function () {
    if (this.__nanaGpuResource) hostCall("webgpuResourceRelease", [this.id]);
    this.__nanaGpuResource = null;
  };

  function GPUBufferShim(resource, descriptor, device) {
    GPUObjectBase.call(this, resource);
    this.size = Number(resource.size || descriptor.size || 0);
    this.usage = Number(descriptor.usage || 0);
    this.mapState = descriptor.mappedAtCreation ? "mapped" : "unmapped";
    this._device = device;
    this._mapped = descriptor.mappedAtCreation ? new ArrayBuffer(this.size) : null;
  }
  GPUBufferShim.prototype = Object.create(GPUObjectBase.prototype);
  GPUBufferShim.prototype.constructor = GPUBufferShim;
  GPUBufferShim.prototype.getMappedRange = function (offset, size) {
    if (this.mapState !== "mapped" || !this._mapped) throw new DOMException("Buffer is not mapped", "OperationError");
    const begin = Number(offset || 0);
    const length = size == null ? this._mapped.byteLength - begin : Number(size);
    if (begin !== 0 || length !== this._mapped.byteLength) {
      throw new DOMException("Nana write mapping currently exposes the complete mapped range", "NotSupportedError");
    }
    return this._mapped;
  };
  GPUBufferShim.prototype.mapAsync = function (mode, offset, size) {
    return Promise.reject(new DOMException("GPUBuffer.mapAsync is not implemented by the Nana WebGPU subset", "NotSupportedError"));
  };
  GPUBufferShim.prototype.unmap = function () {
    if (this._mapped) hostCall("webgpuBufferUnmapInitial", [this.id, new Uint8Array(this._mapped)]);
    this._mapped = null;
    this.mapState = "unmapped";
  };

  function GPUTextureViewShim(resource) { GPUObjectBase.call(this, resource); }
  GPUTextureViewShim.prototype = Object.create(GPUObjectBase.prototype);
  GPUTextureViewShim.prototype.constructor = GPUTextureViewShim;
  function GPUTextureShim(resource, device) {
    GPUObjectBase.call(this, resource);
    this.width = Number(resource.width || 1);
    this.height = Number(resource.height || 1);
    this.depthOrArrayLayers = Number(resource.depthOrArrayLayers || 1);
    this.format = resource.format || "rgba8unorm";
    this._device = device;
  }
  GPUTextureShim.prototype = Object.create(GPUObjectBase.prototype);
  GPUTextureShim.prototype.constructor = GPUTextureShim;
  GPUTextureShim.prototype.createView = function (descriptor) {
    return new GPUTextureViewShim(hostCall("webgpuTextureCreateView", [this.id, descriptor || {}]));
  };
  GPUTextureShim.prototype.destroy = function () { hostCall("webgpuTextureDestroy", [this.id]); this.__nanaGpuResource = null; };

  function simpleGpuType(name) {
    const ctor = function (resource) { GPUObjectBase.call(this, resource); };
    Object.defineProperty(ctor, "name", { value: name });
    ctor.prototype = Object.create(GPUObjectBase.prototype);
    ctor.prototype.constructor = ctor;
    return ctor;
  }
  const GPUSamplerShim = simpleGpuType("GPUSampler");
  const GPUShaderModuleShim = simpleGpuType("GPUShaderModule");
  GPUShaderModuleShim.prototype.getCompilationInfo = function () {
    return Promise.reject(new DOMException("Shader compilation info is not exposed by the Nana WebGPU subset", "NotSupportedError"));
  };
  const GPUBindGroupLayoutShim = simpleGpuType("GPUBindGroupLayout");
  const GPUPipelineLayoutShim = simpleGpuType("GPUPipelineLayout");
  const GPUBindGroupShim = simpleGpuType("GPUBindGroup");
  const GPURenderPipelineShim = simpleGpuType("GPURenderPipeline");
  const GPUComputePipelineShim = simpleGpuType("GPUComputePipeline");
  const GPUCommandBufferShim = simpleGpuType("GPUCommandBuffer");

  function normalizeGpuDescriptor(value) {
    if (value == null || typeof value !== "object") return value;
    const id = gpuResourceId(value);
    if (id != null) return {
      id: id,
      generation: Number(value.generation || (value.__nanaGpuResource && value.__nanaGpuResource.generation) || 0),
      kind: value.__nanaGpuResource && value.__nanaGpuResource.kind || value.kind || "",
    };
    if (Array.isArray(value)) return value.map(normalizeGpuDescriptor);
    if (ArrayBuffer.isView && ArrayBuffer.isView(value)) return Array.from(value);
    const result = {};
    for (const key of Object.keys(value)) result[key] = normalizeGpuDescriptor(value[key]);
    return result;
  }

  function GPURenderPassEncoderShim(resource) { GPUObjectBase.call(this, resource); this._ended = false; }
  GPURenderPassEncoderShim.prototype._command = function (name, args) {
    if (this._ended) throw new DOMException("Render pass has ended", "InvalidStateError");
    hostCall("webgpuPassCommand", [this.id, name, Array.from(args, normalizeGpuDescriptor)]);
  };
  [
    "setPipeline", "setBindGroup", "setVertexBuffer", "setIndexBuffer",
    "setViewport", "setScissorRect", "setBlendConstant", "setStencilReference",
    "draw", "drawIndexed"
  ].forEach(function (name) {
    GPURenderPassEncoderShim.prototype[name] = function () { this._command(name, arguments); };
  });
  GPURenderPassEncoderShim.prototype.end = function () { if (!this._ended) hostCall("webgpuEndPass", [this.id]); this._ended = true; };

  function GPUComputePassEncoderShim(resource) { GPUObjectBase.call(this, resource); this._ended = false; }
  GPUComputePassEncoderShim.prototype._command = GPURenderPassEncoderShim.prototype._command;
  ["setPipeline", "setBindGroup", "dispatchWorkgroups"].forEach(function (name) {
    GPUComputePassEncoderShim.prototype[name] = function () { this._command(name, arguments); };
  });
  GPUComputePassEncoderShim.prototype.end = GPURenderPassEncoderShim.prototype.end;

  function GPUCommandEncoderShim(resource) { GPUObjectBase.call(this, resource); this._finished = false; }
  GPUCommandEncoderShim.prototype.beginRenderPass = function (descriptor) {
    return new GPURenderPassEncoderShim(hostCall("webgpuBeginPass", [this.id, "render", normalizeGpuDescriptor(descriptor || {})]));
  };
  GPUCommandEncoderShim.prototype.beginComputePass = function (descriptor) {
    return new GPUComputePassEncoderShim(hostCall("webgpuBeginPass", [this.id, "compute", normalizeGpuDescriptor(descriptor || {})]));
  };
  GPUCommandEncoderShim.prototype.copyBufferToBuffer = function (source, sourceOffset, destination, destinationOffset, size) {
    hostCall("webgpuEncoderCopyBuffer", [this.id, gpuResourceId(source), sourceOffset, gpuResourceId(destination), destinationOffset, size]);
  };
  GPUCommandEncoderShim.prototype.finish = function () {
    if (this._finished) throw new DOMException("Command encoder is already finished", "InvalidStateError");
    this._finished = true;
    return new GPUCommandBufferShim(hostCall("webgpuFinishEncoder", [this.id]));
  };

  function GPUQueueShim(device) { this._device = device; this.label = ""; }
  GPUQueueShim.prototype.writeBuffer = function (buffer, bufferOffset, data, dataOffset, size) {
    let bytes = asUint8Array(data);
    const start = Number(dataOffset || 0);
    const end = size == null ? bytes.byteLength : start + Number(size);
    bytes = bytes.slice(start, end);
    hostCall("webgpuQueueWriteBuffer", [gpuResourceId(buffer), Number(bufferOffset || 0), bytes]);
  };
  GPUQueueShim.prototype.writeTexture = function (destination, data, dataLayout, size) {
    hostCall("webgpuQueueWriteTexture", [normalizeGpuDescriptor(destination), asUint8Array(data), dataLayout || {}, size]);
  };
  GPUQueueShim.prototype.submit = function (commandBuffers) {
    hostCall("webgpuQueueSubmit", [Array.from(commandBuffers || [], gpuResourceId)]);
  };
  GPUQueueShim.prototype.onSubmittedWorkDone = function () {
    if (!globalThis.Nana || !globalThis.Nana.host || typeof globalThis.Nana.host.invoke !== "function") {
      return Promise.reject(new DOMException("Nana async host bridge is unavailable", "InvalidStateError"));
    }
    return globalThis.Nana.host.invoke("webgpuQueueSubmittedWorkDone", []);
  };

  function GPUErrorShim(record, fallbackName) {
    this.name = String(record && record.name || fallbackName || "GPUError");
    this.message = String(record && record.message || "WebGPU operation failed");
    this.code = String(record && record.code || "");
    if (Error.captureStackTrace) Error.captureStackTrace(this, GPUErrorShim);
  }
  GPUErrorShim.prototype = Object.create(Error.prototype);
  GPUErrorShim.prototype.constructor = GPUErrorShim;
  function GPUValidationErrorShim(message) { GPUErrorShim.call(this, { name: "GPUValidationError", message }, "GPUValidationError"); }
  GPUValidationErrorShim.prototype = Object.create(GPUErrorShim.prototype);
  GPUValidationErrorShim.prototype.constructor = GPUValidationErrorShim;
  function GPUOutOfMemoryErrorShim(message) { GPUErrorShim.call(this, { name: "GPUOutOfMemoryError", message }, "GPUOutOfMemoryError"); }
  GPUOutOfMemoryErrorShim.prototype = Object.create(GPUErrorShim.prototype);
  GPUOutOfMemoryErrorShim.prototype.constructor = GPUOutOfMemoryErrorShim;
  function GPUInternalErrorShim(message) { GPUErrorShim.call(this, { name: "GPUInternalError", message }, "GPUInternalError"); }
  GPUInternalErrorShim.prototype = Object.create(GPUErrorShim.prototype);
  GPUInternalErrorShim.prototype.constructor = GPUInternalErrorShim;

  function gpuErrorFromRecord(record) {
    if (!record) return null;
    const message = String(record.message || "WebGPU operation failed");
    const error = record.name === "GPUOutOfMemoryError"
      ? new GPUOutOfMemoryErrorShim(message)
      : record.name === "GPUInternalError"
        ? new GPUInternalErrorShim(message)
        : new GPUValidationErrorShim(message);
    error.code = String(record.code || error.code || "");
    return error;
  }

  function GPUDeviceShim(info) {
    this.label = "Nana host device";
    this.features = new Set();
    this.limits = {};
    this.adapterInfo = info;
    this.queue = new GPUQueueShim(this);
    let resolveLost;
    this.lost = new Promise(function (resolve) { resolveLost = resolve; });
    this.__resolveLost = resolveLost;
  }
  GPUDeviceShim.prototype.createBuffer = function (descriptor) { return new GPUBufferShim(hostCall("webgpuCreateBuffer", [normalizeGpuDescriptor(descriptor)]), descriptor, this); };
  GPUDeviceShim.prototype.createTexture = function (descriptor) { return new GPUTextureShim(hostCall("webgpuCreateTexture", [normalizeGpuDescriptor(descriptor)]), this); };
  GPUDeviceShim.prototype.createSampler = function (descriptor) { return new GPUSamplerShim(hostCall("webgpuCreateSampler", [descriptor || {}])); };
  GPUDeviceShim.prototype.createShaderModule = function (descriptor) { return new GPUShaderModuleShim(hostCall("webgpuCreateShaderModule", [descriptor || {}])); };
  GPUDeviceShim.prototype.createBindGroupLayout = function (descriptor) { return new GPUBindGroupLayoutShim(hostCall("webgpuCreateBindGroupLayout", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createPipelineLayout = function (descriptor) { return new GPUPipelineLayoutShim(hostCall("webgpuCreatePipelineLayout", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createBindGroup = function (descriptor) { return new GPUBindGroupShim(hostCall("webgpuCreateBindGroup", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createRenderPipeline = function (descriptor) { return new GPURenderPipelineShim(hostCall("webgpuCreateRenderPipeline", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createRenderPipelineAsync = function () {
    return Promise.reject(new DOMException("Asynchronous render pipeline compilation is not implemented by the Nana WebGPU subset", "NotSupportedError"));
  };
  GPUDeviceShim.prototype.createComputePipeline = function (descriptor) { return new GPUComputePipelineShim(hostCall("webgpuCreateComputePipeline", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createComputePipelineAsync = function () {
    return Promise.reject(new DOMException("Asynchronous compute pipeline compilation is not implemented by the Nana WebGPU subset", "NotSupportedError"));
  };
  GPUDeviceShim.prototype.createCommandEncoder = function () { return new GPUCommandEncoderShim(hostCall("webgpuCreateCommandEncoder", [])); };
  GPUDeviceShim.prototype.pushErrorScope = function (filter) {
    hostCall("webgpuPushErrorScope", [String(filter)]);
  };
  GPUDeviceShim.prototype.popErrorScope = function () {
    if (!globalThis.Nana || !globalThis.Nana.host || typeof globalThis.Nana.host.invoke !== "function") {
      return Promise.reject(new DOMException("Nana async host bridge is unavailable", "InvalidStateError"));
    }
    return globalThis.Nana.host.invoke("webgpuPopErrorScope", []).then(gpuErrorFromRecord);
  };
  GPUDeviceShim.prototype.destroy = function () { if (this.__resolveLost) this.__resolveLost({ reason: "destroyed", message: "GPUDevice destroyed" }); };

  function GPUAdapterShim(info) {
    this.info = info;
    this.features = new Set();
    this.limits = {};
    this.isFallbackAdapter = false;
  }
  GPUAdapterShim.prototype.requestDevice = function (descriptor) {
    const requested = descriptor && typeof descriptor === "object" ? descriptor : {};
    if (Array.isArray(requested.requiredFeatures) && requested.requiredFeatures.length) {
      return Promise.reject(new DOMException("Required WebGPU features are not available in the Nana subset", "NotSupportedError"));
    }
    if (requested.requiredLimits && Object.keys(requested.requiredLimits).length) {
      return Promise.reject(new DOMException("Required WebGPU limits are not available in the Nana subset", "NotSupportedError"));
    }
    if (!globalThis.__nanaGpuDevice) globalThis.__nanaGpuDevice = new GPUDeviceShim(this.info);
    return Promise.resolve(globalThis.__nanaGpuDevice);
  };
  GPUAdapterShim.prototype.requestAdapterInfo = function () { return Promise.resolve(this.info); };

  function GPUShim() {}
  GPUShim.prototype.requestAdapter = function () {
    try { return Promise.resolve(new GPUAdapterShim(hostCall("webgpuAdapterInfo", []))); }
    catch (_error) { return Promise.resolve(null); }
  };
  GPUShim.prototype.getPreferredCanvasFormat = function () { return "rgba8unorm"; };

  function GPUCanvasContextShim(canvas) { this.canvas = canvas; this._device = null; this._texture = null; }
  GPUCanvasContextShim.prototype.configure = function (descriptor) {
    if (!descriptor || !(descriptor.device instanceof GPUDeviceShim)) throw new TypeError("GPUCanvasContext.configure requires a Nana GPUDevice");
    this._device = descriptor.device;
    const request = {
      format: descriptor.format || "rgba8unorm",
      usage: descriptor.usage == null ? GPUTextureUsage.RENDER_ATTACHMENT : descriptor.usage,
      alphaMode: descriptor.alphaMode || "premultiplied",
      width: this.canvas.width,
      height: this.canvas.height,
    };
    this._texture = new GPUTextureShim(hostCall("webgpuCanvasConfigure", [this.canvas.__nanaResource.id, request]), descriptor.device);
    const slot = this._texture.__nanaGpuResource.slot;
    if (slot && typeof this.canvas.setAttribute === "function") this.canvas.setAttribute("data-nana-gpu", slot);
  };
  GPUCanvasContextShim.prototype.getCurrentTexture = function () {
    if (!this._device) throw new DOMException("GPUCanvasContext is not configured", "InvalidStateError");
    this._texture = new GPUTextureShim(hostCall("webgpuCanvasCurrentTexture", [this.canvas.__nanaResource.id]), this._device);
    return this._texture;
  };
  GPUCanvasContextShim.prototype.unconfigure = function () { if (this._texture) this._texture.destroy(); this._texture = this._device = null; };

  globalThis.__nanaWebGpuDeviceLost = function (message) {
    const device = globalThis.__nanaGpuDevice;
    if (device && device.__resolveLost) device.__resolveLost({ reason: "unknown", message: String(message || "GPU device was replaced") });
    globalThis.__nanaGpuDevice = null;
  };
  CanvasRenderingContext2DShim.prototype._call = function (name, args) {
    return hostCall("canvasCommand", [this._id, name, Array.from(args || [])]);
  };
  function canvasMethod(name) {
    CanvasRenderingContext2DShim.prototype[name] = function () { return this._call(name, arguments); };
  }
  [
    "save", "restore", "beginPath", "closePath", "moveTo", "lineTo",
    "quadraticCurveTo", "bezierCurveTo", "rect", "arc", "ellipse", "fill", "stroke",
    "clip", "clearRect", "fillRect", "strokeRect", "translate", "rotate", "scale",
    "transform", "setTransform", "resetTransform", "fillText", "strokeText",
  ].forEach(canvasMethod);
  CanvasRenderingContext2DShim.prototype.drawImage = function (source) {
    const id = resourceId(source);
    if (id == null) throw new TypeError("drawImage source has no Nana image resource");
    const args = [id].concat(Array.prototype.slice.call(arguments, 1));
    return this._call("drawImage", args);
  };
  CanvasRenderingContext2DShim.prototype.measureText = function (text) {
    return this._call("measureText", [String(text)]);
  };
  CanvasRenderingContext2DShim.prototype.createLinearGradient = function () {
    return new CanvasGradientShim("linear", arguments);
  };
  CanvasRenderingContext2DShim.prototype.createRadialGradient = function () {
    return new CanvasGradientShim("radial", arguments);
  };
  CanvasRenderingContext2DShim.prototype.createPattern = function (source, repetition) {
    return new CanvasPatternShim(source, repetition);
  };
  CanvasRenderingContext2DShim.prototype.createImageData = function (a, b) {
    if (a instanceof ImageDataShim) return new ImageDataShim(a.width, a.height);
    return new ImageDataShim(a, b);
  };
  CanvasRenderingContext2DShim.prototype.getImageData = function (x, y, width, height) {
    const result = hostCall("canvasGetImageData", [this._id, x, y, width, height]);
    return new ImageDataShim(new Uint8ClampedArray(result.data), result.width, result.height);
  };
  CanvasRenderingContext2DShim.prototype.putImageData = function (imageData, dx, dy) {
    if (!(imageData instanceof ImageDataShim)) throw new TypeError("putImageData requires ImageData");
    hostCall("canvasPutImageData", [this._id, imageData.data, imageData.width, imageData.height, dx, dy]);
  };
  CanvasRenderingContext2DShim.prototype.setLineDash = function (segments) {
    const values = Array.from(segments || [], Number);
    if (values.some(function (value) { return !Number.isFinite(value) || value < 0; })) {
      throw new DOMException("Line dash values must be finite and non-negative", "IndexSizeError");
    }
    this._lineDash = values.length % 2 ? values.concat(values) : values;
    hostCall("canvasSetState", [this._id, "lineDash", this._lineDash]);
  };
  CanvasRenderingContext2DShim.prototype.getLineDash = function () { return this._lineDash.slice(); };
  function canvasState(name, initial, validate) {
    const privateName = "_" + name;
    Object.defineProperty(CanvasRenderingContext2DShim.prototype, name, {
      configurable: true,
      get: function () { return this[privateName] == null ? initial : this[privateName]; },
      set: function (value) {
        const next = validate ? validate(value, this[privateName] == null ? initial : this[privateName]) : value;
        this[privateName] = next;
        hostCall("canvasSetState", [this._id, name, next]);
      },
    });
  }
  canvasState("fillStyle", "#000000");
  canvasState("strokeStyle", "#000000");
  canvasState("lineWidth", 1, function (v, old) { v = Number(v); return v > 0 && Number.isFinite(v) ? v : old; });
  canvasState("lineCap", "butt", function (v, old) { v = String(v); return ["butt", "round", "square"].includes(v) ? v : old; });
  canvasState("lineJoin", "miter", function (v, old) { v = String(v); return ["miter", "round", "bevel"].includes(v) ? v : old; });
  canvasState("lineDashOffset", 0, function (v) { return Number(v) || 0; });
  canvasState("globalAlpha", 1, function (v, old) { v = Number(v); return v >= 0 && v <= 1 ? v : old; });
  canvasState("globalCompositeOperation", "source-over", function (v) { return String(v); });
  canvasState("font", "10px sans-serif", function (v) { return String(v); });

  function enhanceCanvasElement(element) {
    if (!element || element.__nanaCanvasResource) return element;
    let width = 300;
    let height = 150;
    const resource = hostCall("canvasCreate", [width, height]);
    element.__nanaCanvasResource = resource;
    element.__nanaResource = resource;
    element.__nanaOwnsCanvasResource = true;
    let context2d = null;
    Object.defineProperty(element, "width", {
      configurable: true,
      get: function () { return width; },
      set: function (value) {
        width = Math.max(1, Math.trunc(Number(value) || 300));
        hostCall("canvasResize", [resource.id, width, height]);
      },
    });
    Object.defineProperty(element, "height", {
      configurable: true,
      get: function () { return height; },
      set: function (value) {
        height = Math.max(1, Math.trunc(Number(value) || 150));
        hostCall("canvasResize", [resource.id, width, height]);
      },
    });
    let contextGpu = null;
    let contextKind = null;
    element.getContext = function (kind) {
      const type = String(kind).toLowerCase();
      if (type !== "2d" && type !== "webgpu") return null;
      if (contextKind && contextKind !== type) return null;
      contextKind = type;
      if (type === "2d") return context2d || (context2d = new CanvasRenderingContext2DShim(element));
      if (type === "webgpu") return contextGpu || (contextGpu = new GPUCanvasContextShim(element));
      return null;
    };
    element.toDataURL = function (type, quality) {
      const mime = type || "image/png";
      const bytes = hostCall("canvasEncode", [resource.id, mime, quality]);
      return hostCall("dataUrlFromBytes", [bytes, mime]);
    };
    element.toBlob = function (callback, type, quality) {
      const mime = type || "image/png";
      const bytes = hostCall("canvasEncode", [resource.id, mime, quality]);
      const blob = new BlobShim([bytes], { type: mime });
      queueMicrotask(function () { callback(blob); });
    };
    if (typeof element.setAttribute === "function") {
      try { element.setAttribute("data-nana-canvas", String(resource.id)); } catch (_err) {}
    }
    if (globalThis.HTMLCanvasElement && globalThis.HTMLCanvasElement.prototype) {
      try { Object.setPrototypeOf(element, globalThis.HTMLCanvasElement.prototype); } catch (_err) {}
    }
    return element;
  }
  globalThis.__nanaEnhanceCanvas = enhanceCanvasElement;

  function enhanceMediaElement(element, kind) {
    if (!element || element.__nanaMediaResource) return element;
    const type = kind === "audio" ? "audio" : "video";
    const resource = hostCall("mediaCreate", [type]);
    element.__nanaMediaResource = resource;
    element.__nanaOwnsMediaResource = true;
    if (resource && resource.id != null && typeof element.setAttribute === "function") {
      try { element.setAttribute("data-nana-media", String(resource.id)); } catch (_err) {}
    }
    element.paused = true;
    element.ended = false;
    element.muted = false;
    element.volume = 1;
    element.currentTime = 0;
    element.duration = 0;
    element.readyState = 0;
    element.videoWidth = 0;
    element.videoHeight = 0;
    let src = "";
    function applyDescriptor(next) {
      if (!next || typeof next !== "object") return;
      element.__nanaMediaResource = next;
      element.paused = !!next.paused;
      element.duration = Number(next.duration || 0);
      element.currentTime = Number(next.currentTime || 0);
      element.readyState = Number(next.readyState || 0);
      element.videoWidth = Number(next.width || 0);
      element.videoHeight = Number(next.height || 0);
      if (typeof element.setAttribute === "function") {
        try {
          if (next.id != null) {
            element.setAttribute("data-nana-media", String(next.id));
          }
          if (type === "video") {
            if (next.hasVideoFrame && next.id != null) {
              element.setAttribute("data-nana-video", String(next.id));
            } else {
              element.setAttribute("data-nana-video", "");
            }
          }
        } catch (_err) {}
      }
    }
    Object.defineProperty(element, "src", {
      configurable: true,
      get: function () { return src; },
      set: function (value) {
        src = String(value || "");
        applyDescriptor(hostCall("mediaSetSrc", [resource.id, src]));
      },
    });
    Object.defineProperty(element, "srcObject", {
      configurable: true,
      get: function () { return element.__nanaSrcObject || null; },
      set: function (stream) {
        element.__nanaSrcObject = stream || null;
        const streamId = stream && stream.id != null ? stream.id : 0;
        applyDescriptor(hostCall("mediaSetSrcObject", [resource.id, streamId]));
      },
    });
    Object.defineProperty(element, "currentTime", {
      configurable: true,
      get: function () { return Number(element.__nanaMediaResource && element.__nanaMediaResource.currentTime || 0); },
      set: function (value) {
        applyDescriptor(hostCall("mediaSetCurrentTime", [resource.id, Number(value) || 0]));
      },
    });
    element.play = function () {
      applyDescriptor(hostCall("mediaPlay", [resource.id]));
      return Promise.resolve();
    };
    element.pause = function () {
      applyDescriptor(hostCall("mediaPause", [resource.id]));
    };
    if (globalThis.HTMLMediaElement && globalThis.HTMLMediaElement.prototype) {
      try { Object.setPrototypeOf(element, type === "audio" ? globalThis.HTMLAudioElement.prototype : globalThis.HTMLVideoElement.prototype); } catch (_err) {}
    }
    return element;
  }
  globalThis.__nanaEnhanceMedia = enhanceMediaElement;

  function HTMLMediaElementShim() {}
  function HTMLVideoElementShim() {}
  function HTMLAudioElementShim() {}

  function BlobShim(parts, options) {
    const chunks = [];
    let length = 0;
    for (const part of parts || []) {
      const bytes = typeof part === "string" ? new TextEncoder().encode(part) : asUint8Array(part);
      chunks.push(bytes);
      length += bytes.byteLength;
    }
    const joined = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) { joined.set(chunk, offset); offset += chunk.byteLength; }
    this.type = String(options && options.type || "").toLowerCase();
    this.size = joined.byteLength;
    this.__nanaResource = hostCall("blobCreate", [joined, this.type]);
  }
  BlobShim.prototype.arrayBuffer = function () {
    if (!this.__nanaResource) return Promise.reject(new DOMException("Blob has been released", "InvalidStateError"));
    const bytes = hostCall("resourceBytes", [this.__nanaResource.id]);
    return Promise.resolve(asUint8Array(bytes).buffer);
  };
  BlobShim.prototype.text = function () { return this.arrayBuffer().then(function (bytes) { return new TextDecoder().decode(bytes); }); };
  BlobShim.prototype.slice = function (start, end, type) {
    if (!this.__nanaResource) throw new DOMException("Blob has been released", "InvalidStateError");
    const bytes = asUint8Array(hostCall("resourceBytes", [this.__nanaResource.id])).slice(start || 0, end == null ? this.size : end);
    return new BlobShim([bytes], { type: type || "" });
  };
  BlobShim.prototype.close = function () {
    if (this.__nanaResource) hostCall("resourceRelease", [this.__nanaResource.id]);
    this.__nanaResource = null;
  };

  function ImageBitmapShim(resource) {
    this.__nanaResource = resource;
    this.width = resource.width;
    this.height = resource.height;
  }
  ImageBitmapShim.prototype.close = function () {
    if (this.__nanaResource) hostCall("resourceRelease", [this.__nanaResource.id]);
    this.__nanaResource = null;
  };

  function ImageShim() {
    EventTargetShim.call(this);
    this.complete = false;
    this.naturalWidth = 0;
    this.naturalHeight = 0;
    this.width = 0;
    this.height = 0;
    this._src = "";
    this.__nanaResource = null;
    this._loadGeneration = 0;
    this._loadController = null;
    this._decodePromise = Promise.resolve(this);
  }
  ImageShim.prototype = Object.create(EventTargetShim.prototype);
  ImageShim.prototype.constructor = ImageShim;
  Object.defineProperty(ImageShim.prototype, "src", {
    get: function () { return this._src; },
    set: function (value) {
      const self = this;
      const generation = ++this._loadGeneration;
      if (this._loadController) this._loadController.abort();
      this._loadController = new AbortControllerShim();
      if (this.__nanaResource) hostCall("resourceRelease", [this.__nanaResource.id]);
      this.__nanaResource = null;
      this._src = String(value || "");
      this.complete = false;
      let load;
      if (/^data:/i.test(this._src)) {
        const comma = this._src.indexOf(",");
        const head = this._src.slice(0, comma);
        const body = this._src.slice(comma + 1);
        if (/;base64/i.test(head)) {
          load = Promise.resolve(decodeBase64(body));
        } else load = Promise.resolve(new TextEncoder().encode(decodeURIComponent(body)));
      } else if (/^blob:nana\//.test(this._src)) {
        load = Promise.resolve(hostCall("objectUrlBytes", [this._src]));
      } else {
        load = fetch(this._src, { signal: this._loadController.signal }).then(function (response) {
          if (!response.ok) throw new Error("HTTP " + response.status);
          return response.arrayBuffer();
        });
      }
      this._decodePromise = load.then(function (bytes) {
        if (self._loadGeneration !== generation) throw abortError();
        const resource = hostCall("imageDecode", [asUint8Array(bytes)]);
        if (self._loadGeneration !== generation) {
          hostCall("resourceRelease", [resource.id]);
          throw abortError();
        }
        self.__nanaResource = resource;
        self.naturalWidth = self.width = resource.width;
        self.naturalHeight = self.height = resource.height;
        self.complete = true;
        self.dispatchEvent(new CustomEventShim("load"));
        return self;
      }, function (error) {
        if (self._loadGeneration === generation && !(error && error.name === "AbortError")) {
          self.dispatchEvent(new CustomEventShim("error"));
        }
        throw error;
      });
      this._decodePromise.catch(function () {});
    },
  });
  ImageShim.prototype.decode = function () { return this._decodePromise; };
  ImageShim.prototype.close = function () {
    ++this._loadGeneration;
    if (this._loadController) this._loadController.abort();
    this._loadController = null;
    if (this.__nanaResource) hostCall("resourceRelease", [this.__nanaResource.id]);
    this.__nanaResource = null;
    this.complete = false;
  };

  function createImageBitmapShim(source) {
    const id = resourceId(source);
    if (id == null) return Promise.reject(new TypeError("createImageBitmap source is unsupported"));
    const args = Array.prototype.slice.call(arguments, 1);
    let request = {};
    if (args.length >= 4 && args.slice(0, 4).every(function (value) { return Number.isFinite(Number(value)); })) {
      request = {
        sx: Number(args[0]), sy: Number(args[1]),
        sw: Number(args[2]), sh: Number(args[3]),
        ...(args[4] && typeof args[4] === "object" ? args[4] : {}),
      };
    } else if (args[0] && typeof args[0] === "object") {
      request = { ...args[0] };
    }
    return Promise.resolve(new ImageBitmapShim(hostCall("imageBitmapCreate", [id, request])));
  }

  DocumentShim.prototype.createElement = function (tag) {
    const t = String(tag).toLowerCase();
    if (t === "template") {
      const tmpl = Object.create(globalThis.HTMLElement.prototype);
      tmpl.tagName = "TEMPLATE";
      tmpl.nodeName = "TEMPLATE";
      tmpl.nodeType = 1;
      tmpl.content = createTemplateContent();
      Object.defineProperty(tmpl, "innerHTML", {
        get: function () {
          return serializeHtmlNodes(tmpl.content.childNodes);
        },
        set: function (v) {
          var kids = parseHtmlFragment(String(v ?? ""));
          for (var i = 0; i < kids.length; i++) kids[i].parentNode = tmpl.content;
          tmpl.content.childNodes = kids;
        },
        configurable: true,
      });
      return tmpl;
    }
    // Detached element stub (not in host tree) — prefer Vue hostOps createElement.
    const el = Object.create(globalThis.HTMLElement.prototype);
    el.tagName = t.toUpperCase();
    el.nodeName = el.tagName;
    el.nodeType = 1;
    el.style = {};
    el.dataset = {};
    el.className = "";
    el.classList = {
      add: function () {},
      remove: function () {},
      contains: function () {
        return false;
      },
      toggle: function () {
        return false;
      },
    };
    el.setAttribute = function () {};
    el.getAttribute = function () {
      return null;
    };
    el.removeAttribute = function () {};
    el.appendChild = function () {};
    el.removeChild = function () {};
    el.addEventListener = function () {};
    el.removeEventListener = function () {};
    el.textContent = "";
    el.innerHTML = "";
    return t === "canvas" ? enhanceCanvasElement(el)
      : t === "video" ? enhanceMediaElement(el, "video")
      : t === "audio" ? enhanceMediaElement(el, "audio")
      : el;
  };
  DocumentShim.prototype.createElementNS = function (_ns, tag) {
    return this.createElement(tag);
  };
  DocumentShim.prototype.createTextNode = function (text) {
    return { nodeType: 3, textContent: String(text ?? "") };
  };
  DocumentShim.prototype.getElementById = function (id) {
    try {
      const found = hostCall("querySelector", ["#" + String(id ?? "")]);
      return found == null ? null : wrapHostNode(found, null);
    } catch (_err) {
      return null;
    }
  };
  DocumentShim.prototype.querySelector = function (sel) {
    try {
      const raw = String(sel ?? "");
      const id = hostCall("querySelector", [raw]);
      if (id == null) return null;
      // Same body/html tag hint as hostOps.querySelector (Teleport target stability).
      return wrapHostNode(id, teleportTargetTag(raw));
    } catch (_err) {
      return null;
    }
  };
  DocumentShim.prototype.querySelectorAll = function (sel) {
    try {
      const raw = String(sel ?? "");
      const ids = hostCall("querySelectorAll", [raw]) || [];
      const tag = teleportTargetTag(raw);
      return Array.from(ids, function (id) {
        return wrapHostNode(id, tag);
      });
    } catch (_err) {
      return [];
    }
  };
  DocumentShim.prototype.hasFocus = function () {
    const win = globalThis.window;
    if (!win) return false;
    // Default true until host pumps blur (matches initial focused window).
    return win.__nanaFocused !== false;
  };

  function LocationShim() {
    this._href = "nana://app/";
    this._path = "/";
    this._search = "";
    this._hash = "";
  }
  Object.defineProperty(LocationShim.prototype, "href", {
    get: function () {
      return this._href;
    },
    set: function (v) {
      this.assign(String(v));
    },
  });
  Object.defineProperty(LocationShim.prototype, "pathname", {
    get: function () {
      return this._path;
    },
    set: function (v) {
      this._path = String(v || "/");
      this._href = "nana://app" + this._path + this._search + this._hash;
      hostCall("locationSet", [this._path, this._search, this._hash]);
    },
  });
  Object.defineProperty(LocationShim.prototype, "search", {
    get: function () {
      return this._search;
    },
    set: function (v) {
      const s = String(v || "");
      this._search = s && s.charAt(0) !== "?" ? "?" + s : s;
      this._href = "nana://app" + this._path + this._search + this._hash;
      hostCall("locationSet", [this._path, this._search, this._hash]);
    },
  });
  Object.defineProperty(LocationShim.prototype, "hash", {
    get: function () {
      return this._hash;
    },
    set: function (v) {
      const h = String(v || "");
      this._hash = h && h.charAt(0) !== "#" ? "#" + h : h;
      this._href = "nana://app" + this._path + this._search + this._hash;
    },
  });
  LocationShim.prototype.assign = function (url) {
    const raw = String(url || "/");
    let path = raw;
    let search = "";
    let hash = "";
    const hashIdx = path.indexOf("#");
    if (hashIdx >= 0) {
      hash = path.slice(hashIdx);
      path = path.slice(0, hashIdx);
    }
    const qIdx = path.indexOf("?");
    if (qIdx >= 0) {
      search = path.slice(qIdx);
      path = path.slice(0, qIdx);
    }
    if (path.indexOf("://") >= 0) {
      const after = path.replace(/^[^:]+:\/\/[^/]*/, "");
      path = after || "/";
    }
    if (!path) path = "/";
    this._path = path.charAt(0) === "/" ? path : "/" + path;
    this._search = search;
    this._hash = hash;
    this._href = "nana://app" + this._path + this._search + this._hash;
    hostCall("locationSet", [this._path, this._search, this._hash]);
    if (globalThis.window) {
      globalThis.window.dispatchEvent({ type: "popstate", state: globalThis.history && globalThis.history.state });
    }
  };
  LocationShim.prototype.replace = function (url) {
    this.assign(url);
  };
  LocationShim.prototype.reload = function () {
    throw new DOMException("location.reload is owned by the Nana application host", "NotSupportedError");
  };

  function HistoryShim(location) {
    this.state = null;
    this._stack = [{ path: "/", search: "", hash: "", state: null }];
    this._index = 0;
    this._location = location;
  }
  HistoryShim.prototype.pushState = function (state, _title, url) {
    if (url != null) this._location.assign(url);
    this.state = state;
    this._stack = this._stack.slice(0, this._index + 1);
    this._stack.push({
      path: this._location.pathname,
      search: this._location.search,
      hash: this._location.hash,
      state: state,
    });
    this._index = this._stack.length - 1;
  };
  HistoryShim.prototype.replaceState = function (state, _title, url) {
    if (url != null) this._location.assign(url);
    this.state = state;
    this._stack[this._index] = {
      path: this._location.pathname,
      search: this._location.search,
      hash: this._location.hash,
      state: state,
    };
  };
  HistoryShim.prototype.go = function (delta) {
    const next = this._index + (Number(delta) || 0);
    if (next < 0 || next >= this._stack.length) return;
    this._index = next;
    const entry = this._stack[this._index];
    this.state = entry.state;
    this._location._path = entry.path;
    this._location._search = entry.search;
    this._location._hash = entry.hash;
    this._location._href = "nana://app" + entry.path + entry.search + entry.hash;
    hostCall("locationSet", [entry.path, entry.search, entry.hash]);
    if (globalThis.window) {
      globalThis.window.dispatchEvent({ type: "popstate", state: this.state });
    }
  };
  HistoryShim.prototype.back = function () {
    this.go(-1);
  };
  HistoryShim.prototype.forward = function () {
    this.go(1);
  };

  function normalizeHeaderName(name) {
    const normalized = String(name).toLowerCase();
    if (!/^[!#$%&'*+.^_`|~0-9a-z-]+$/.test(normalized)) {
      throw new TypeError("Invalid header name: " + name);
    }
    return normalized;
  }

  function normalizeHeaderValue(value) {
    return String(value).replace(/^\s+|\s+$/g, "");
  }

  function HeadersShim(init) {
    this._list = [];
    if (init instanceof HeadersShim) init = init._list;
    if (Array.isArray(init)) {
      for (let i = 0; i < init.length; i++) {
        if (!Array.isArray(init[i]) || init[i].length !== 2) {
          throw new TypeError("Header entry must be a [name, value] pair");
        }
        this.append(init[i][0], init[i][1]);
      }
    } else if (init && typeof init === "object") {
      const keys = Object.keys(init);
      for (let i = 0; i < keys.length; i++) this.append(keys[i], init[keys[i]]);
    }
  }
  HeadersShim.prototype.append = function (name, value) {
    const key = normalizeHeaderName(name);
    this._list.push([key, normalizeHeaderValue(value)]);
  };
  HeadersShim.prototype.set = function (name, value) {
    const key = normalizeHeaderName(name);
    this.delete(key);
    this.append(key, value);
  };
  HeadersShim.prototype.get = function (name) {
    const key = normalizeHeaderName(name);
    const values = this._list.filter(function (pair) { return pair[0] === key; });
    return values.length ? values.map(function (pair) { return pair[1]; }).join(", ") : null;
  };
  HeadersShim.prototype.has = function (name) {
    const key = normalizeHeaderName(name);
    return this._list.some(function (pair) { return pair[0] === key; });
  };
  HeadersShim.prototype.delete = function (name) {
    const key = normalizeHeaderName(name);
    this._list = this._list.filter(function (pair) { return pair[0] !== key; });
  };
  HeadersShim.prototype.entries = function () {
    return this._list.slice()[Symbol.iterator]();
  };
  HeadersShim.prototype.keys = function () {
    return this._list.map(function (pair) { return pair[0]; })[Symbol.iterator]();
  };
  HeadersShim.prototype.values = function () {
    return this._list.map(function (pair) { return pair[1]; })[Symbol.iterator]();
  };
  HeadersShim.prototype.forEach = function (callback, thisArg) {
    for (let i = 0; i < this._list.length; i++) {
      callback.call(thisArg, this._list[i][1], this._list[i][0], this);
    }
  };
  HeadersShim.prototype[Symbol.iterator] = HeadersShim.prototype.entries;

  function bodyBytes(body) {
    if (body == null) return new Uint8Array(0);
    if (typeof body === "string") return new TextEncoder().encode(body);
    if (body instanceof ArrayBuffer) return new Uint8Array(body.slice(0));
    if (ArrayBuffer.isView && ArrayBuffer.isView(body)) {
      return new Uint8Array(body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength));
    }
    if (body instanceof BlobShim && body.__nanaResource) {
      return asUint8Array(hostCall("resourceBytes", [body.__nanaResource.id]));
    }
    const name = body && body.constructor && body.constructor.name;
    if (name === "Blob" || name === "FormData" || name === "URLSearchParams") {
      throw new TypeError(name + " request bodies are not supported by Nana fetch");
    }
    throw new TypeError("Nana fetch only supports string, ArrayBuffer, or typed-array bodies");
  }

  function rejectUnsupportedRequestOptions(init) {
    const unsupported = [
      "mode", "credentials", "cache", "integrity", "keepalive", "referrer",
      "referrerPolicy", "priority", "duplex",
    ];
    for (let i = 0; i < unsupported.length; i++) {
      if (Object.prototype.hasOwnProperty.call(init, unsupported[i])) {
        throw new TypeError("Request option `" + unsupported[i] + "` is not supported by Nana fetch");
      }
    }
    if (init.redirect != null && init.redirect !== "follow") {
      throw new TypeError("Only redirect: \"follow\" is supported by Nana fetch");
    }
  }

  function AbortSignalShim() {
    EventTargetShim.call(this);
    this.aborted = false;
    this.reason = undefined;
  }
  AbortSignalShim.prototype = Object.create(EventTargetShim.prototype);
  AbortSignalShim.prototype.constructor = AbortSignalShim;
  AbortSignalShim.prototype.throwIfAborted = function () {
    if (this.aborted) throw this.reason || abortError();
  };
  AbortSignalShim.abort = function (reason) {
    const controller = new AbortControllerShim();
    controller.abort(reason);
    return controller.signal;
  };

  function AbortControllerShim() {
    this.signal = new AbortSignalShim();
  }
  AbortControllerShim.prototype.abort = function (reason) {
    if (this.signal.aborted) return;
    this.signal.aborted = true;
    this.signal.reason = reason === undefined ? abortError() : reason;
    this.signal.dispatchEvent(new CustomEventShim("abort"));
  };

  function abortError() {
    const error = new Error("The operation was aborted");
    error.name = "AbortError";
    return error;
  }

  function RequestShim(input, init) {
    init = init || {};
    rejectUnsupportedRequestOptions(init);
    const source = input instanceof RequestShim ? input : null;
    if (source && source.bodyUsed) throw new TypeError("Request body has already been consumed");
    this.url = String(source ? source.url : input);
    this.method = String(init.method || (source && source.method) || "GET").toUpperCase();
    this.headers = new HeadersShim(init.headers || (source && source.headers));
    if (this.headers.has("cookie") || this.headers.has("set-cookie")) {
      throw new TypeError("Cookie headers are not supported by Nana fetch");
    }
    this.signal = init.signal || (source && source.signal) || new AbortSignalShim();
    this.redirect = init.redirect || (source && source.redirect) || "follow";
    this._body = Object.prototype.hasOwnProperty.call(init, "body")
      ? bodyBytes(init.body)
      : source ? new Uint8Array(source._body) : new Uint8Array(0);
    this.bodyUsed = false;
    if ((this.method === "GET" || this.method === "HEAD") && this._body.length) {
      throw new TypeError("GET/HEAD requests cannot have a body");
    }
  }
  RequestShim.prototype.clone = function () {
    if (this.bodyUsed) throw new TypeError("Request body has already been consumed");
    return new RequestShim(this);
  };
  RequestShim.prototype.text = function () { return consumeBody(this, "text"); };
  RequestShim.prototype.json = function () { return consumeBody(this, "json"); };
  RequestShim.prototype.arrayBuffer = function () { return consumeBody(this, "arrayBuffer"); };

  function consumeBody(owner, kind) {
    if (owner.bodyUsed) return Promise.reject(new TypeError("Body has already been consumed"));
    owner.bodyUsed = true;
    const copy = new Uint8Array(owner._body);
    if (kind === "arrayBuffer") return Promise.resolve(copy.buffer);
    const text = new TextDecoder().decode(copy);
    if (kind === "json") {
      return Promise.resolve().then(function () { return JSON.parse(text); });
    }
    return Promise.resolve(text);
  }

  function ResponseShim(body, init) {
    init = init || {};
    this._body = body instanceof Uint8Array ? new Uint8Array(body) : bodyBytes(body);
    this.status = Number(init.status == null ? 200 : init.status);
    this.statusText = String(init.statusText || "");
    this.headers = new HeadersShim(init.headers);
    this.url = String(init.url || "");
    this.redirected = !!init.redirected;
    this.type = "basic";
    this.bodyUsed = false;
  }
  Object.defineProperty(ResponseShim.prototype, "ok", {
    get: function () { return this.status >= 200 && this.status <= 299; },
  });
  ResponseShim.prototype.text = function () { return consumeBody(this, "text"); };
  ResponseShim.prototype.json = function () { return consumeBody(this, "json"); };
  ResponseShim.prototype.arrayBuffer = function () { return consumeBody(this, "arrayBuffer"); };
  ResponseShim.prototype.blob = function () {
    if (this.bodyUsed) return Promise.reject(new TypeError("Body has already been consumed"));
    this.bodyUsed = true;
    return Promise.resolve(new BlobShim([this._body], {
      type: this.headers.get("content-type") || "",
    }));
  };
  ResponseShim.prototype.clone = function () {
    if (this.bodyUsed) throw new TypeError("Response body has already been consumed");
    return new ResponseShim(this._body, {
      status: this.status,
      statusText: this.statusText,
      headers: this.headers,
      url: this.url,
      redirected: this.redirected,
    });
  };

  const pendingFetches = new Map();
  function fetchShim(input, init) {
    return Promise.resolve().then(function () {
      const request = new RequestShim(input, init);
      if (request.signal && request.signal.aborted) throw request.signal.reason || abortError();
      const id = hostCall("fetchStart", [{
        url: request.url,
        method: request.method,
        headers: request.headers._list,
        body: request._body,
      }]);
      return new Promise(function (resolve, reject) {
        const abort = function () {
          if (!pendingFetches.has(id)) return;
          pendingFetches.delete(id);
          try { hostCall("fetchCancel", [id]); } catch (_err) {}
          reject(request.signal.reason || abortError());
        };
        pendingFetches.set(id, {
          resolve: resolve,
          reject: reject,
          abort: abort,
          signal: request.signal,
          windowId: Number(globalThis.__nanaActiveWindowId || 0),
        });
        if (request.signal && typeof request.signal.addEventListener === "function") {
          request.signal.addEventListener("abort", abort, { once: true });
        }
      });
    });
  }

  globalThis.__nanaDrainFetch = function __nanaDrainFetch(completions) {
    const list = Array.isArray(completions) ? completions : [];
    for (let i = 0; i < list.length; i++) {
      const completion = list[i] || {};
      const pending = pendingFetches.get(completion.id);
      if (!pending) continue;
      pendingFetches.delete(completion.id);
      if (pending.signal && typeof pending.signal.removeEventListener === "function") {
        pending.signal.removeEventListener("abort", pending.abort);
      }
      if (!completion.ok) {
        withWindowContext(pending.windowId, function () {
          pending.reject(new TypeError((completion.error && completion.error.message) || "Fetch failed"));
        });
        continue;
      }
      const raw = completion.response || {};
      withWindowContext(pending.windowId, function () {
        pending.resolve(new ResponseShim(asUint8Array(raw.body), {
          status: raw.status,
          statusText: raw.statusText,
          headers: raw.headers,
          url: raw.url,
          redirected: raw.redirected,
        }));
      });
    }
    return list.length;
  };

  const SOCKET_CONNECTING = 0;
  const SOCKET_OPEN = 1;
  const SOCKET_CLOSING = 2;
  const SOCKET_CLOSED = 3;
  const pendingSockets = new Map();

  function normalizeWsProtocols(protocols) {
    if (protocols == null) return [];
    const list = Array.isArray(protocols) ? protocols : [protocols];
    const names = [];
    const seen = new Set();
    for (let i = 0; i < list.length; i++) {
      const name = String(list[i]);
      if (!name) throw new SyntaxError("WebSocket subprotocol must be a non-empty string");
      if (seen.has(name)) throw new SyntaxError("Duplicate WebSocket subprotocol: " + name);
      seen.add(name);
      names.push(name);
    }
    return names;
  }

  function socketPayloadBytes(data) {
    if (data instanceof ArrayBuffer) return new Uint8Array(data.slice(0));
    if (ArrayBuffer.isView && ArrayBuffer.isView(data)) {
      return new Uint8Array(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength));
    }
    throw new TypeError("Nana WebSocket only supports string, ArrayBuffer, or typed-array payloads");
  }

  /**
   * Buffered WebSocket surface over the reserved `wsOpen` / `wsSend` / `wsClose`
   * host ops. The transport is application-owned: without a host-injected
   * socket backend the constructor throws and nothing connects.
   */
  function WebSocketShim(url, protocols) {
    EventTargetShim.call(this);
    const raw = String(url == null ? "" : url);
    if (!/^wss?:\/\//i.test(raw)) {
      throw new SyntaxError("Nana WebSocket only supports ws:// and wss:// URLs");
    }
    const id = Number(hostCall("wsOpen", [{
      url: raw,
      protocols: normalizeWsProtocols(protocols),
    }]));
    this.url = raw;
    this.readyState = SOCKET_CONNECTING;
    this._wsId = id;
    this._wsWindowId = Number(globalThis.__nanaActiveWindowId || 0);
    pendingSockets.set(id, this);
  }
  WebSocketShim.CONNECTING = SOCKET_CONNECTING;
  WebSocketShim.OPEN = SOCKET_OPEN;
  WebSocketShim.CLOSING = SOCKET_CLOSING;
  WebSocketShim.CLOSED = SOCKET_CLOSED;
  WebSocketShim.prototype.CONNECTING = SOCKET_CONNECTING;
  WebSocketShim.prototype.OPEN = SOCKET_OPEN;
  WebSocketShim.prototype.CLOSING = SOCKET_CLOSING;
  WebSocketShim.prototype.CLOSED = SOCKET_CLOSED;
  WebSocketShim.prototype.send = function (data) {
    if (this.readyState !== SOCKET_OPEN) {
      throw new TypeError("WebSocket send is only allowed while the socket is open");
    }
    const kind = typeof data === "string" ? "text" : "binary";
    const payload = kind === "text" ? data : socketPayloadBytes(data);
    hostCall("wsSend", [{ id: this._wsId, kind: kind, data: payload }]);
  };
  WebSocketShim.prototype.close = function (code, reason) {
    if (this.readyState === SOCKET_CLOSING || this.readyState === SOCKET_CLOSED) return;
    const closeCode = code == null ? 1000 : Number(code);
    if (!(closeCode === 1000 || (closeCode >= 3000 && closeCode <= 4999))) {
      throw new TypeError("Invalid WebSocket close code");
    }
    hostCall("wsClose", [{
      id: this._wsId,
      code: closeCode,
      reason: reason == null ? "" : String(reason),
    }]);
    this.readyState = SOCKET_CLOSING;
  };

  function SocketEventShim(type, init) {
    this.type = type;
    this.data = init ? init.data : undefined;
    this.code = init ? init.code : undefined;
    this.reason = init ? init.reason : "";
    this.wasClean = init ? !!init.wasClean : false;
    this.origin = "";
  }

  globalThis.__nanaDrainWs = function __nanaDrainWs(events) {
    const list = Array.isArray(events) ? events : [];
    for (let i = 0; i < list.length; i++) {
      const item = list[i] || {};
      const socket = pendingSockets.get(item.id);
      if (!socket) continue;
      if (item.kind === "open") {
        socket.readyState = SOCKET_OPEN;
        withWindowContext(socket._wsWindowId, function () {
          socket.dispatchEvent(new SocketEventShim("open"));
        });
      } else if (item.kind === "message") {
        withWindowContext(socket._wsWindowId, function () {
          const data = item.bytes ? asUint8Array(item.bytes) : item.data;
          socket.dispatchEvent(new SocketEventShim("message", { data: data }));
        });
      } else if (item.kind === "error") {
        withWindowContext(socket._wsWindowId, function () {
          socket.dispatchEvent(new SocketEventShim("error", { message: item.message }));
        });
      } else if (item.kind === "close") {
        socket.readyState = SOCKET_CLOSED;
        pendingSockets.delete(item.id);
        withWindowContext(socket._wsWindowId, function () {
          socket.dispatchEvent(new SocketEventShim("close", {
            code: Number(item.code || 1005),
            reason: String(item.reason || ""),
            wasClean: !!item.wasClean,
          }));
        });
      }
    }
    return list.length;
  };

  function WindowShim() {
    EventTargetShim.call(this);
    this.document = new DocumentShim();
    this.localStorage = new StorageShim("local");
    this.sessionStorage = new StorageShim("session");
    this.innerWidth = 960;
    this.innerHeight = 640;
    this.outerWidth = 960;
    this.outerHeight = 640;
    this.devicePixelRatio = 1;
    this.navigator = {
      language: "en-US",
      languages: ["en-US"],
      clipboard: {
        writeText: function (text) {
          return Promise.resolve().then(function () {
            hostCall("clipboardWriteText", [String(text == null ? "" : text)]);
          });
        },
        readText: function () {
          return Promise.resolve().then(function () {
            return hostCall("clipboardReadText", []);
          });
        },
      },
      mediaDevices: {
        getUserMedia: function (constraints) {
          return Promise.resolve().then(function () {
            return hostCall("mediaDevicesGetUserMedia", [constraints && typeof constraints === "object" ? constraints : { video: true }]);
          });
        },
      },
    };
    this.location = new LocationShim();
    this.history = new HistoryShim(this.location);
    this.performance = {
      now: function () {
        return Date.now();
      },
      timeOrigin: Date.now(),
    };
    this.visualViewport = new EventTargetShim();
    this.visualViewport.width = 960;
    this.visualViewport.height = 640;
    this.visualViewport.offsetLeft = 0;
    this.visualViewport.offsetTop = 0;
    this.visualViewport.scale = 1;
    this.__nanaMediaQueries = new Set();
    this.matchMedia = function (query) {
      const q = String(query || "");
      const owner = this;
      const result = new EventTargetShim();
      result.media = q;
      result.onchange = null;
      result.matches = evaluateMediaQuery(owner, q);
      result.addListener = function (listener) { result.addEventListener("change", listener); };
      result.removeListener = function (listener) { result.removeEventListener("change", listener); };
      owner.__nanaMediaQueries.add(result);
      return result;
    };
    /**
     * Vue runtime-dom Transition reads camelCase keys on the returned object
     * (`transitionDuration`, …), not only getPropertyValue.
     *
     * When the host exposes cascade-resolved motion (`computedStyle` host op),
     * prefer that over inline defaults so real CSS transitions are honored.
     */
    this.getComputedStyle = function (el) {
      const style = (el && el.style) || {};
      const read = function (name, camel, fallback) {
        if (style.getPropertyValue) {
          const v = style.getPropertyValue(name);
          if (v) return String(v);
        }
        if (style[name] != null && style[name] !== "") return String(style[name]);
        if (camel && style[camel] != null && style[camel] !== "") return String(style[camel]);
        return fallback;
      };
      let hostMotion = null;
      const nid = el && el.__nid;
      if (nid != null && Number.isFinite(Number(nid))) {
        try {
          hostMotion = hostCall("computedStyle", [Number(nid)]);
        } catch (_err) {
          hostMotion = null;
        }
      }
      const fromHost = function (key, camel, fallback) {
        if (hostMotion && hostMotion[key] != null && hostMotion[key] !== "") {
          return String(hostMotion[key]);
        }
        if (hostMotion && camel && hostMotion[camel] != null && hostMotion[camel] !== "") {
          return String(hostMotion[camel]);
        }
        return fallback;
      };
      const computed = {
        getPropertyValue: function (name) {
          const key = String(name || "").toLowerCase();
          if (key === "transition-delay" || key === "transitiondelay")
            return fromHost("transitionDelay", "transitionDelay", read("transition-delay", "transitionDelay", "0s"));
          if (key === "transition-duration" || key === "transitionduration")
            return fromHost("transitionDuration", "transitionDuration", read("transition-duration", "transitionDuration", "0s"));
          if (key === "transition-property" || key === "transitionproperty")
            return fromHost("transitionProperty", "transitionProperty", read("transition-property", "transitionProperty", "none"));
          if (key === "animation-delay" || key === "animationdelay")
            return fromHost("animationDelay", "animationDelay", read("animation-delay", "animationDelay", "0s"));
          if (key === "animation-duration" || key === "animationduration")
            return fromHost("animationDuration", "animationDuration", read("animation-duration", "animationDuration", "0s"));
          if (key === "animation-name" || key === "animationname")
            return fromHost("animationName", "animationName", read("animation-name", "animationName", "none"));
          if (key === "transition-timing-function" || key === "transitiontimingfunction")
            return fromHost("transitionTimingFunction", "transitionTimingFunction", read("transition-timing-function", "transitionTimingFunction", "ease"));
          if (key === "width") return fromHost("width", "width", read("width", "width", "0px"));
          if (key === "height") return fromHost("height", "height", read("height", "height", "0px"));
          if (key === "opacity") return fromHost("opacity", "opacity", read("opacity", "opacity", "1"));
          if (key === "color") return fromHost("color", "color", read("color", "color", ""));
          if (key === "transform") return fromHost("transform", "transform", read("transform", "transform", "none"));
          if (key === "background-color" || key === "backgroundcolor")
            return fromHost("backgroundColor", "background-color", read("background-color", "backgroundColor", "rgba(0, 0, 0, 0)"));
          if (key === "font-size" || key === "fontsize")
            return fromHost("fontSize", "font-size", read("font-size", "fontSize", ""));
          if (key === "font-family" || key === "fontfamily")
            return fromHost("fontFamily", "font-family", read("font-family", "fontFamily", ""));
          if (key === "font-weight" || key === "fontweight")
            return fromHost("fontWeight", "font-weight", read("font-weight", "fontWeight", ""));
          if (style.getPropertyValue) return style.getPropertyValue(name) || "";
          return style[name] || "";
        },
        transitionDelay: fromHost("transitionDelay", "transitionDelay", "0s"),
        transitionDuration: fromHost("transitionDuration", "transitionDuration", "0s"),
        transitionProperty: fromHost("transitionProperty", "transitionProperty", "none"),
        animationDelay: fromHost("animationDelay", "animationDelay", "0s"),
        animationDuration: fromHost("animationDuration", "animationDuration", "0s"),
        animationName: fromHost("animationName", "animationName", "none"),
        transitionTimingFunction: fromHost("transitionTimingFunction", "transitionTimingFunction", "ease"),
        width: fromHost("width", "width", "0px"),
        height: fromHost("height", "height", "0px"),
        opacity: fromHost("opacity", "opacity", "1"),
        color: fromHost("color", "color", ""),
        transform: fromHost("transform", "transform", "none"),
        backgroundColor: fromHost("backgroundColor", "background-color", "rgba(0, 0, 0, 0)"),
        fontSize: fromHost("fontSize", "font-size", ""),
        fontFamily: fromHost("fontFamily", "font-family", ""),
        fontWeight: fromHost("fontWeight", "font-weight", ""),
      };
      // Prefer explicit inline style when present.
      const td = read("transition-duration", "transitionDuration", null);
      if (td != null) computed.transitionDuration = td;
      const tdelay = read("transition-delay", "transitionDelay", null);
      if (tdelay != null) computed.transitionDelay = tdelay;
      const tp = read("transition-property", "transitionProperty", null);
      if (tp != null) computed.transitionProperty = tp;
      const ad = read("animation-duration", "animationDuration", null);
      if (ad != null) computed.animationDuration = ad;
      const adelay = read("animation-delay", "animationDelay", null);
      if (adelay != null) computed.animationDelay = adelay;
      const an = read("animation-name", "animationName", null);
      if (an != null) computed.animationName = an;
      return computed;
    };
    this.requestAnimationFrame = requestAnimationFrame;
    this.cancelAnimationFrame = cancelAnimationFrame;
    this.setTimeout = setTimeoutShim;
    this.clearTimeout = clearTimeoutShim;
    this.setInterval = setIntervalShim;
    this.clearInterval = clearIntervalShim;
    this.queueMicrotask = globalThis.queueMicrotask;
    this.confirm = function () {
      throw new DOMException("Synchronous confirm is unavailable; use Nana.dialogs.confirm", "NotSupportedError");
    };
    this.prompt = function () {
      throw new DOMException("Synchronous prompt is unavailable; use Nana.dialogs.prompt", "NotSupportedError");
    };
    this.alert = function () {
      throw new DOMException("Synchronous alert is unavailable; use Nana.dialogs.alert", "NotSupportedError");
    };
    this.open = function (url, name, features) {
      const href = url == null || url === "" ? "" : String(url);
      if (href && href !== "about:blank") {
        throw new DOMException(
          "window.open(url) is unavailable; use Nana.windows.create",
          "NotSupportedError",
        );
      }
      if (!globalThis.Nana || !globalThis.Nana.windows || typeof globalThis.Nana.windows.create !== "function") {
        throw new DOMException("Nana.windows.create is required for window.open", "NotSupportedError");
      }
      const options = parseWindowOpenFeatures(features);
      if (name != null && name !== "" && name !== "_blank") {
        options.title = String(name);
      }
      return globalThis.Nana.windows.create(options);
    };
    // Host pumps FocusChanged → true/false; seed as focused like a newly shown window.
    this.__nanaFocused = true;
  }
  WindowShim.prototype = Object.create(EventTargetShim.prototype);
  WindowShim.prototype.constructor = WindowShim;

  function parseWindowOpenFeatures(features) {
    const options = {};
    if (features == null || features === "") return options;
    if (typeof features !== "string") {
      throw new DOMException("window.open features must be a string", "NotSupportedError");
    }
    const allowed = { width: "width", height: "height", left: "x", top: "y" };
    for (const part of features.split(",")) {
      const split = part.split("=");
      const key = String(split[0] || "").trim().toLowerCase();
      if (!key) continue;
      const mapped = allowed[key];
      if (!mapped) {
        throw new DOMException(`window.open feature "${key}" is not supported`, "NotSupportedError");
      }
      const n = Number(split[1]);
      if (!Number.isFinite(n)) {
        throw new DOMException(`window.open feature "${key}" must be a number`, "NotSupportedError");
      }
      options[mapped] = n;
    }
    return options;
  }

  function evaluateMediaQuery(win, query) {
    try {
      const hosted = hostCall("evaluateMediaQuery", [String(query || "")]);
      if (typeof hosted === "boolean") return hosted;
    } catch (_err) {}
    return evaluateMediaQueryLocal(win, query);
  }

  function splitMediaList(query) {
    const s = String(query || "");
    const out = [];
    let start = 0;
    let depth = 0;
    for (let i = 0; i < s.length; i++) {
      const c = s.charAt(i);
      if (c === "(") depth += 1;
      else if (c === ")") depth -= 1;
      else if (c === "," && depth === 0) {
        const part = s.slice(start, i).trim();
        if (part) out.push(part);
        start = i + 1;
      }
    }
    const part = s.slice(start).trim();
    if (part) out.push(part);
    return out;
  }

  function splitMediaAnd(query) {
    const s = String(query || "");
    const out = [];
    let start = 0;
    let depth = 0;
    const lower = s.toLowerCase();
    let i = 0;
    while (i < s.length) {
      const c = s.charAt(i);
      if (c === "(") depth += 1;
      else if (c === ")") depth -= 1;
      if (depth === 0 && lower.slice(i, i + 5) === " and ") {
        const part = s.slice(start, i).trim();
        if (part) out.push(part);
        i += 5;
        start = i;
        continue;
      }
      i += 1;
    }
    const part = s.slice(start).trim();
    if (part) out.push(part);
    return out;
  }

  function evaluateMediaFeature(win, raw) {
    let q = String(raw || "").trim().toLowerCase();
    if (q.charAt(0) === "(" && q.charAt(q.length - 1) === ")" && q.length >= 2) {
      q = q.slice(1, -1).trim();
    }
    q = q.replace(/\s+/g, " ");
    let match = /^min-width\s*:\s*([\d.]+)px$/.exec(q);
    if (match) return Number(win.innerWidth) >= Number(match[1]);
    match = /^max-width\s*:\s*([\d.]+)px$/.exec(q);
    if (match) return Number(win.innerWidth) <= Number(match[1]);
    match = /^width\s*:\s*([\d.]+)px$/.exec(q);
    if (match) return Math.abs(Number(win.innerWidth) - Number(match[1])) < 0.5;
    match = /^min-height\s*:\s*([\d.]+)px$/.exec(q);
    if (match) return Number(win.innerHeight) >= Number(match[1]);
    match = /^max-height\s*:\s*([\d.]+)px$/.exec(q);
    if (match) return Number(win.innerHeight) <= Number(match[1]);
    match = /^height\s*:\s*([\d.]+)px$/.exec(q);
    if (match) return Math.abs(Number(win.innerHeight) - Number(match[1])) < 0.5;
    const compact = q.replace(/\s/g, "");
    if (compact === "orientation:landscape") return Number(win.innerWidth) >= Number(win.innerHeight);
    if (compact === "orientation:portrait") return Number(win.innerHeight) > Number(win.innerWidth);
    if (compact === "prefers-color-scheme:dark") {
      return !!(win.document && win.document.documentElement && win.document.documentElement.dataset.theme === "dark");
    }
    if (compact === "prefers-color-scheme:light") {
      return !(win.document && win.document.documentElement && win.document.documentElement.dataset.theme === "dark");
    }
    return false;
  }

  function evaluateOneMediaQuery(win, raw) {
    let rest = String(raw || "").trim().toLowerCase();
    if (!rest) return true;
    let negated = false;
    if (/^not\b/.test(rest)) {
      negated = true;
      rest = rest.replace(/^not\s+/, "");
    } else if (/^only\b/.test(rest)) {
      rest = rest.replace(/^only\s+/, "");
    }
    let typeOk = true;
    let featureText = rest;
    if (rest.charAt(0) !== "(") {
      const m = /^(all|screen|print|[a-z][\w-]*)\b/.exec(rest);
      if (!m) return false;
      const ty = m[1];
      typeOk = ty === "all" || ty === "screen";
      rest = rest.slice(m[0].length).trim();
      if (rest) {
        if (!/^and\b/.test(rest)) return negated;
        featureText = rest.replace(/^and\s+/, "");
      } else {
        featureText = "";
      }
    }
    const featuresOk = !featureText || splitMediaAnd(featureText).every(function (part) {
      return evaluateMediaFeature(win, part);
    });
    const matched = typeOk && featuresOk;
    return negated ? !matched : matched;
  }

  // Local fallback when the Vue host op is unavailable. Same subset as Rust
  // evaluate_media_query: screen/all true, print/unknown false.
  function evaluateMediaQueryLocal(win, query) {
    const parts = splitMediaList(String(query || "").toLowerCase());
    if (!parts.length) return true;
    return parts.some(function (part) {
      return evaluateOneMediaQuery(win, part);
    });
  }

  function refreshMediaQueries(win) {
    if (!win || !win.__nanaMediaQueries) return;
    for (const query of Array.from(win.__nanaMediaQueries)) {
      const matches = evaluateMediaQuery(win, query.media);
      if (matches === query.matches) continue;
      query.matches = matches;
      const event = new CustomEventShim("change", { detail: { matches: matches, media: query.media } });
      event.matches = matches;
      event.media = query.media;
      query.dispatchEvent(event);
      if (typeof query.onchange === "function") query.onchange(event);
    }
  }

  if (typeof globalThis.EventTarget === "undefined") {
    globalThis.EventTarget = EventTargetShim;
  }
  if (typeof globalThis.CustomEvent === "undefined") {
    globalThis.CustomEvent = CustomEventShim;
  }
  if (typeof globalThis.Event === "undefined") {
    globalThis.Event = CustomEventShim;
  }
  if (typeof globalThis.Node === "undefined") {
    function NodeCtor() {}
    NodeCtor.ELEMENT_NODE = 1;
    NodeCtor.TEXT_NODE = 3;
    NodeCtor.COMMENT_NODE = 8;
    NodeCtor.DOCUMENT_NODE = 9;
    NodeCtor.DOCUMENT_FRAGMENT_NODE = 11;
    NodeCtor.prototype = {
      ELEMENT_NODE: 1,
      TEXT_NODE: 3,
      COMMENT_NODE: 8,
      DOCUMENT_NODE: 9,
      DOCUMENT_FRAGMENT_NODE: 11,
      nodeType: 0,
      contains: function () {
        return false;
      },
    };
    NodeCtor.prototype.constructor = NodeCtor;
    globalThis.Node = NodeCtor;
  } else if (typeof globalThis.Node === "object" && globalThis.Node) {
    // Upgrade constants bag → callable constructor so `instanceof Node` works
    // (required by anchored-overlay outside-click detection).
    const constants = globalThis.Node;
    function NodeCtor() {}
    NodeCtor.ELEMENT_NODE = constants.ELEMENT_NODE || 1;
    NodeCtor.TEXT_NODE = constants.TEXT_NODE || 3;
    NodeCtor.COMMENT_NODE = constants.COMMENT_NODE || 8;
    NodeCtor.DOCUMENT_NODE = constants.DOCUMENT_NODE || 9;
    NodeCtor.DOCUMENT_FRAGMENT_NODE = constants.DOCUMENT_FRAGMENT_NODE || 11;
    NodeCtor.prototype = {
      ELEMENT_NODE: NodeCtor.ELEMENT_NODE,
      TEXT_NODE: NodeCtor.TEXT_NODE,
      COMMENT_NODE: NodeCtor.COMMENT_NODE,
      DOCUMENT_NODE: NodeCtor.DOCUMENT_NODE,
      DOCUMENT_FRAGMENT_NODE: NodeCtor.DOCUMENT_FRAGMENT_NODE,
    };
    NodeCtor.prototype.constructor = NodeCtor;
    globalThis.Node = NodeCtor;
  }
  if (typeof globalThis.Element === "undefined") {
    function ElementCtor() {}
    ElementCtor.prototype = Object.create(globalThis.Node.prototype);
    ElementCtor.prototype.constructor = ElementCtor;
    globalThis.Element = ElementCtor;
  } else if (
    globalThis.Node &&
    globalThis.Node.prototype &&
    !(Object.prototype.isPrototypeOf.call(globalThis.Node.prototype, globalThis.Element.prototype) ||
      globalThis.Element.prototype instanceof globalThis.Node)
  ) {
    // Re-chain existing Element under Node when Node was upgraded late.
    const prev = globalThis.Element.prototype;
    const chained = Object.create(globalThis.Node.prototype);
    Object.assign(chained, prev);
    chained.constructor = globalThis.Element;
    globalThis.Element.prototype = chained;
  }
  if (typeof globalThis.HTMLElement === "undefined") {
    function HTMLElementCtor() {}
    HTMLElementCtor.prototype = Object.create(globalThis.Element.prototype);
    HTMLElementCtor.prototype.constructor = HTMLElementCtor;
    globalThis.HTMLElement = HTMLElementCtor;
  }

  /**
   * Project host `layoutBox` onto Element layout readables.
   * wrapNode / wrapHostNode set `__nid`; detached stubs without it read as 0.
   * offset* is the border box; client* is the padding box when the host
   * sends clientWidth; scroll* uses scrollWidth when present.
   * offsetLeft/Top are the real subset; offsetParent may be null without a node cache.
   */
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

  const win = new WindowShim();
  const windowContexts = new Map();
  windowContexts.set(0, { window: win, document: win.document });

  function scopedObject(target, windowId) {
    if (!target || typeof target !== "object" || typeof Proxy === "undefined") return target;
    return new Proxy(target, {
      get: function (inner, key, receiver) {
        const value = Reflect.get(inner, key, receiver);
        if (typeof value === "function") {
          return function () {
            const args = arguments;
            return withWindowContext(windowId, function () {
              return value.apply(inner, args);
            });
          };
        }
        if (value && typeof value === "object" && (key === "dataset" || key === "style")) {
          return scopedObject(value, windowId);
        }
        return value;
      },
      set: function (inner, key, value, receiver) {
        return withWindowContext(windowId, function () {
          return Reflect.set(inner, key, value, receiver);
        });
      },
    });
  }

  globalThis.__nanaWithWindowContext = withWindowContext;
  globalThis.__nanaCreateWindowContext = function __nanaCreateWindowContext(
    windowId,
    width,
    height,
    scaleFactor,
  ) {
    const id = Number(windowId || 0);
    if (!id) return windowContexts.get(0);
    const existing = windowContexts.get(id);
    if (existing) return existing;
    const rawWindow = new WindowShim();
    const logicalWidth = Math.max(1, Number(width) || 800);
    const logicalHeight = Math.max(1, Number(height) || 600);
    const dpr = Math.max(0.01, Number(scaleFactor) || 1);
    rawWindow.innerWidth = logicalWidth;
    rawWindow.outerWidth = logicalWidth;
    rawWindow.innerHeight = logicalHeight;
    rawWindow.outerHeight = logicalHeight;
    rawWindow.devicePixelRatio = dpr;
    rawWindow.visualViewport.width = logicalWidth;
    rawWindow.visualViewport.height = logicalHeight;
    const rawDocument = rawWindow.document;
    const documentElement = scopedObject(rawDocument.documentElement, id);
    rawDocument.documentElement = documentElement;
    const document = scopedObject(rawDocument, id);
    rawWindow.document = document;
    const window = scopedObject(rawWindow, id);
    const context = { window: window, document: document };
    windowContexts.set(id, context);
    return context;
  };
  globalThis.__nanaGetWindowContext = function __nanaGetWindowContext(windowId) {
    return windowContexts.get(Number(windowId || 0)) || null;
  };
  globalThis.__nanaDestroyWindowContext = function __nanaDestroyWindowContext(windowId) {
    const id = Number(windowId || 0);
    if (!id) return false;
    const context = windowContexts.get(id);
    withWindowContext(id, function () {
      for (const [timerId, entry] of rafCallbacks) {
        if (entry.windowId === id) cancelAnimationFrame(timerId);
      }
      for (const [timerId, entry] of timeoutCallbacks) {
        if (entry.windowId === id) clearTimeoutShim(timerId);
      }
      for (const [timerId, entry] of intervalCallbacks) {
        if (entry.windowId === id) clearIntervalShim(timerId);
      }
      for (const pending of pendingFetches.values()) {
        if (pending.windowId === id) pending.abort();
      }
      for (const socket of pendingSockets.values()) {
        if (socket._wsWindowId === id && socket.readyState < SOCKET_CLOSING) {
          try { socket.close(); } catch (_err) {}
        }
      }
      if (typeof globalThis.__nanaReleaseWindowObservers === "function") {
        globalThis.__nanaReleaseWindowObservers(id);
      }
      if (context) {
        for (const target of [context.window, context.document, context.window.visualViewport]) {
          if (target && typeof target.__nanaClearListeners === "function") {
            target.__nanaClearListeners();
          }
        }
        if (context.window && context.window.__nanaMediaQueries) {
          for (const query of context.window.__nanaMediaQueries) {
            if (query && typeof query.__nanaClearListeners === "function") {
              query.__nanaClearListeners();
            }
          }
          context.window.__nanaMediaQueries.clear();
        }
      }
    });
    return windowContexts.delete(id);
  };
  globalThis.window = win;
  globalThis.self = win;
  globalThis.document = win.document;
  globalThis.localStorage = win.localStorage;
  globalThis.sessionStorage = win.sessionStorage;
  globalThis.navigator = win.navigator;
  globalThis.location = win.location;
  globalThis.history = win.history;
  globalThis.performance = win.performance;
  globalThis.requestAnimationFrame = requestAnimationFrame;
  globalThis.cancelAnimationFrame = cancelAnimationFrame;
  globalThis.setTimeout = setTimeoutShim;
  globalThis.clearTimeout = clearTimeoutShim;
  globalThis.setInterval = setIntervalShim;
  globalThis.clearInterval = clearIntervalShim;
  globalThis.Headers = HeadersShim;
  globalThis.Request = RequestShim;
  globalThis.Response = ResponseShim;
  globalThis.AbortSignal = AbortSignalShim;
  globalThis.AbortController = AbortControllerShim;
  globalThis.fetch = fetchShim;
  globalThis.WebSocket = WebSocketShim;
  globalThis.ImageData = ImageDataShim;
  globalThis.CanvasGradient = CanvasGradientShim;
  globalThis.CanvasPattern = CanvasPatternShim;
  globalThis.CanvasRenderingContext2D = CanvasRenderingContext2DShim;
  HTMLCanvasElementShim.prototype = Object.create(globalThis.HTMLElement.prototype);
  HTMLCanvasElementShim.prototype.constructor = HTMLCanvasElementShim;
  globalThis.HTMLCanvasElement = HTMLCanvasElementShim;
  HTMLMediaElementShim.prototype = Object.create(globalThis.HTMLElement.prototype);
  HTMLMediaElementShim.prototype.constructor = HTMLMediaElementShim;
  globalThis.HTMLMediaElement = HTMLMediaElementShim;
  HTMLVideoElementShim.prototype = Object.create(HTMLMediaElementShim.prototype);
  HTMLVideoElementShim.prototype.constructor = HTMLVideoElementShim;
  globalThis.HTMLVideoElement = HTMLVideoElementShim;
  HTMLAudioElementShim.prototype = Object.create(HTMLMediaElementShim.prototype);
  HTMLAudioElementShim.prototype.constructor = HTMLAudioElementShim;
  globalThis.HTMLAudioElement = HTMLAudioElementShim;
  globalThis.Blob = BlobShim;
  globalThis.Image = ImageShim;
  globalThis.ImageBitmap = ImageBitmapShim;
  globalThis.createImageBitmap = createImageBitmapShim;
  globalThis.GPU = GPUShim;
  globalThis.GPUAdapter = GPUAdapterShim;
  globalThis.GPUDevice = GPUDeviceShim;
  globalThis.GPUValidationError = GPUValidationErrorShim;
  globalThis.GPUOutOfMemoryError = GPUOutOfMemoryErrorShim;
  globalThis.GPUInternalError = GPUInternalErrorShim;
  globalThis.GPUQueue = GPUQueueShim;
  globalThis.GPUBuffer = GPUBufferShim;
  globalThis.GPUTexture = GPUTextureShim;
  globalThis.GPUTextureView = GPUTextureViewShim;
  globalThis.GPUSampler = GPUSamplerShim;
  globalThis.GPUShaderModule = GPUShaderModuleShim;
  globalThis.GPUBindGroup = GPUBindGroupShim;
  globalThis.GPUBindGroupLayout = GPUBindGroupLayoutShim;
  globalThis.GPUPipelineLayout = GPUPipelineLayoutShim;
  globalThis.GPURenderPipeline = GPURenderPipelineShim;
  globalThis.GPUComputePipeline = GPUComputePipelineShim;
  globalThis.GPUCommandEncoder = GPUCommandEncoderShim;
  globalThis.GPUCommandBuffer = GPUCommandBufferShim;
  globalThis.GPURenderPassEncoder = GPURenderPassEncoderShim;
  globalThis.GPUComputePassEncoder = GPUComputePassEncoderShim;
  globalThis.GPUCanvasContext = GPUCanvasContextShim;
  globalThis.GPUBufferUsage = Object.freeze({ MAP_READ: 1, MAP_WRITE: 2, COPY_SRC: 4, COPY_DST: 8, INDEX: 16, VERTEX: 32, UNIFORM: 64, STORAGE: 128, INDIRECT: 256, QUERY_RESOLVE: 512 });
  globalThis.GPUTextureUsage = Object.freeze({ COPY_SRC: 1, COPY_DST: 2, TEXTURE_BINDING: 4, STORAGE_BINDING: 8, RENDER_ATTACHMENT: 16 });
  globalThis.GPUShaderStage = Object.freeze({ VERTEX: 1, FRAGMENT: 2, COMPUTE: 4 });
  globalThis.GPUMapMode = Object.freeze({ READ: 1, WRITE: 2 });
  globalThis.GPUColorWrite = Object.freeze({ RED: 1, GREEN: 2, BLUE: 4, ALPHA: 8, ALL: 15 });
  if (typeof globalThis.DOMException === "undefined") {
    globalThis.DOMException = function DOMException(message, name) {
      this.name = name || "Error";
      this.message = String(message || "");
    };
    globalThis.DOMException.prototype = Object.create(Error.prototype);
  }
  if (globalThis.URL) {
    globalThis.URL.createObjectURL = function (resource) {
      const id = resourceId(resource);
      if (id == null) throw new TypeError("createObjectURL requires a Nana resource");
      return hostCall("objectUrlCreate", [id]);
    };
    globalThis.URL.revokeObjectURL = function (url) {
      hostCall("objectUrlRevoke", [String(url || "")]);
    };
  }
  win.Headers = HeadersShim;
  win.Request = RequestShim;
  win.Response = ResponseShim;
  win.AbortSignal = AbortSignalShim;
  win.AbortController = AbortControllerShim;
  win.fetch = fetchShim;
  win.WebSocket = WebSocketShim;
  win.ImageData = ImageDataShim;
  win.CanvasRenderingContext2D = CanvasRenderingContext2DShim;
  win.HTMLCanvasElement = HTMLCanvasElementShim;
  win.HTMLMediaElement = HTMLMediaElementShim;
  win.HTMLVideoElement = HTMLVideoElementShim;
  win.HTMLAudioElement = HTMLAudioElementShim;
  win.Blob = BlobShim;
  win.Image = ImageShim;
  win.ImageBitmap = ImageBitmapShim;
  win.createImageBitmap = createImageBitmapShim;
  win.navigator.gpu = new GPUShim();

  /**
   * Host → JS lifecycle surface for focus refresh / layout listeners.
   * Payload: { type: "resize"|"focus"|"blur"|"visibilitychange", width?, height?, hidden? }
   * Dispatches on shim EventTarget (`window` or `document`).
   */
  function pumpLifecycle(win, doc, payload) {
    if (!win || !payload || typeof payload !== "object") return false;
    const type = String(payload.type || "");
    if (type === "resize") {
      const w = Math.max(0, Number(payload.width));
      const h = Math.max(0, Number(payload.height));
      const dpr = Math.max(0.01, Number(payload.scaleFactor || win.devicePixelRatio || 1));
      win.devicePixelRatio = dpr;
      if (Number.isFinite(w) && w > 0) {
        win.innerWidth = w;
        win.outerWidth = w;
        if (win.visualViewport) win.visualViewport.width = w;
        if (doc && doc.scrollingElement) doc.scrollingElement.clientWidth = w;
      }
      if (Number.isFinite(h) && h > 0) {
        win.innerHeight = h;
        win.outerHeight = h;
        if (win.visualViewport) win.visualViewport.height = h;
        if (doc && doc.scrollingElement) doc.scrollingElement.clientHeight = h;
      }
      if (win.visualViewport) win.visualViewport.dispatchEvent(new CustomEventShim("resize"));
      refreshMediaQueries(win);
      win.dispatchEvent(new CustomEventShim("resize"));
      return true;
    }
    if (type === "focus") {
      win.__nanaFocused = true;
      win.dispatchEvent(new CustomEventShim("focus"));
      return true;
    }
    if (type === "blur") {
      win.__nanaFocused = false;
      win.dispatchEvent(new CustomEventShim("blur"));
      return true;
    }
    if (type === "visibilitychange") {
      if (!doc) return false;
      const hidden = !!payload.hidden;
      doc.hidden = hidden;
      doc.visibilityState = hidden ? "hidden" : "visible";
      doc.dispatchEvent(new CustomEventShim("visibilitychange"));
      return true;
    }
    return false;
  }

  globalThis.__nanaPumpLifecycle = function __nanaPumpLifecycle(payload) {
    return pumpLifecycle(globalThis.window, globalThis.document, payload);
  };

  globalThis.__nanaPumpWindowLifecycle = function __nanaPumpWindowLifecycle(windowId, payload) {
    const context = windowContexts.get(Number(windowId || 0));
    if (!context) return false;
    return withWindowContext(windowId, function () {
      return pumpLifecycle(context.window, context.document, payload);
    });
  };

  globalThis.__nanaWebApi = {
    version: "webview-source-subset-1",
    installed: true,
    EventTarget: EventTargetShim,
    CustomEvent: CustomEventShim,
    pumpLifecycle: globalThis.__nanaPumpLifecycle,
  };
})();
