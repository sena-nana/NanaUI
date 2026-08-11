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
    return host.call(name, args);
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

  // Vite-style import.meta.env for IIFE / eval hosts (engines may not parse import.meta).
  if (typeof globalThis.__nanaImportMeta === "undefined") {
    globalThis.__nanaImportMeta = {
      env: {
        DEV: true,
        PROD: false,
        MODE: "development",
        SSR: false,
        BASE_URL: "/",
        VITE_LILIA_AGENT_DEBUG: "0",
        VITE_LILIA_GITHUB_AGENT_DEBUG: "0",
        VITE_LILIA_GITHUB_AGENT_DEBUG_MOCK_WORKSPACE: "1",
      },
      url: "nana://app/main.js",
    };
  }

  if (typeof globalThis.console === "undefined") {
    globalThis.console = {
      log() {},
      warn() {},
      error() {},
      info() {},
      debug() {},
      trace() {},
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

  // Prefer JSON clone — engine native structuredClone may be incomplete under Nana.
  globalThis.structuredClone = function structuredClone(value) {
    if (value === undefined) return undefined;
    if (typeof value === "function") return value;
    return JSON.parse(JSON.stringify(value));
  };

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
    rafCallbacks.set(id, cb);
    hostCall("rafSchedule", [id]);
    return id;
  }
  function cancelAnimationFrame(id) {
    rafCallbacks.delete(Number(id));
    hostCall("rafCancel", [Number(id)]);
  }
  function setTimeoutShim(cb, delay) {
    const id = nextTimeoutId++;
    timeoutCallbacks.set(id, typeof cb === "function" ? cb : function () {});
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
      const cb = rafCallbacks.get(id);
      rafCallbacks.delete(id);
      if (typeof cb === "function") {
        try {
          cb(now);
        } catch (_err) {}
      }
    }
    const timeoutIds = (payload && payload.timeouts) || [];
    for (let j = 0; j < timeoutIds.length; j++) {
      const id = Number(timeoutIds[j]);
      const cb = timeoutCallbacks.get(id);
      timeoutCallbacks.delete(id);
      if (typeof cb === "function") {
        try {
          cb();
        } catch (_err) {}
      }
    }
    const intervalIds = (payload && payload.intervals) || [];
    for (let k = 0; k < intervalIds.length; k++) {
      const id = Number(intervalIds[k]);
      const entry = intervalCallbacks.get(id);
      if (entry && typeof entry.cb === "function") {
        try {
          entry.cb();
        } catch (_err) {}
        hostCall("intervalSchedule", [id, entry.delay]);
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
    this.title = "LiliaGithub";
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
    return el;
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
  LocationShim.prototype.reload = function () {};

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
      userAgent: "NanaUI/Phase4 (LiliaGithub fixture)",
      language: "zh-CN",
      languages: ["zh-CN", "en"],
      platform: "MacIntel",
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
    };
    this.location = new LocationShim();
    this.history = new HistoryShim(this.location);
    this.performance = {
      now: function () {
        return Date.now();
      },
      timeOrigin: Date.now(),
    };
    this.visualViewport = { width: 960, height: 640, offsetLeft: 0, offsetTop: 0, scale: 1 };
    this.matchMedia = function (query) {
      const q = String(query || "");
      const dark = /prefers-color-scheme:\s*dark/i.test(q);
      return {
        matches: dark ? false : true,
        media: q,
        addEventListener: function () {},
        removeEventListener: function () {},
        addListener: function () {},
        removeListener: function () {},
        onchange: null,
      };
    };
    /**
     * Vue runtime-dom Transition reads camelCase keys on the returned object
     * (`transitionDuration`, …), not only getPropertyValue.
     *
     * Nana has no CSS transition/animation engine — report 0s/none so
     * `whenTransitionEnds` resolves immediately after nextFrame (honest
     * minimal completion). Real timed CSS Transition: defer.
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
      const computed = {
        getPropertyValue: function (name) {
          const key = String(name || "").toLowerCase();
          if (key === "transition-delay" || key === "transitiondelay")
            return read("transition-delay", "transitionDelay", "0s");
          if (key === "transition-duration" || key === "transitionduration")
            return read("transition-duration", "transitionDuration", "0s");
          if (key === "transition-property" || key === "transitionproperty")
            return read("transition-property", "transitionProperty", "none");
          if (key === "animation-delay" || key === "animationdelay")
            return read("animation-delay", "animationDelay", "0s");
          if (key === "animation-duration" || key === "animationduration")
            return read("animation-duration", "animationDuration", "0s");
          if (key === "animation-name" || key === "animationname")
            return read("animation-name", "animationName", "none");
          if (style.getPropertyValue) return style.getPropertyValue(name) || "";
          return style[name] || "";
        },
        transitionDelay: "0s",
        transitionDuration: "0s",
        transitionProperty: "none",
        animationDelay: "0s",
        animationDuration: "0s",
        animationName: "none",
      };
      // Prefer explicit inline style when present (still usually 0s on Nana).
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
      return true;
    };
    this.prompt = function (_msg, def) {
      return def == null ? "" : String(def);
    };
    this.alert = function () {};
    this.__TAURI_INTERNALS__ = undefined;
    this.__TAURI__ = undefined;
    // Host pumps FocusChanged → true/false; seed as focused like a newly shown window.
    this.__nanaFocused = true;
  }
  WindowShim.prototype = Object.create(EventTargetShim.prototype);
  WindowShim.prototype.constructor = WindowShim;

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
    // (required by LiliaUI useAnchoredOverlay.containsTarget).
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
   * offsetWidth/Height, clientWidth/Height, scrollWidth/Height share one box
   * (no border/padding split).
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

  function installElementLayoutMetrics(proto) {
    if (!proto || proto.__nanaLayoutMetricsInstalled) return;
    function dim(axis) {
      return {
        configurable: true,
        enumerable: true,
        get: function () {
          const s = layoutBoxSizeFromNid(this.__nid);
          return axis === "w" ? s.width : s.height;
        },
      };
    }
    Object.defineProperty(proto, "offsetWidth", dim("w"));
    Object.defineProperty(proto, "offsetHeight", dim("h"));
    Object.defineProperty(proto, "clientWidth", dim("w"));
    Object.defineProperty(proto, "clientHeight", dim("h"));
    Object.defineProperty(proto, "scrollWidth", dim("w"));
    Object.defineProperty(proto, "scrollHeight", dim("h"));
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
  }
  if (typeof globalThis.MutationObserver === "undefined") {
    globalThis.MutationObserver = function MutationObserver() {
      this.observe = function () {};
      this.disconnect = function () {};
      this.takeRecords = function () {
        return [];
      };
    };
  }
  if (typeof globalThis.IntersectionObserver === "undefined") {
    globalThis.IntersectionObserver = function IntersectionObserver() {
      this.observe = function () {};
      this.unobserve = function () {};
      this.disconnect = function () {};
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

  // Probe helpers for mock workspace transport selection.
  globalThis.__NANA_FORCE_MOCK_WORKSPACE__ = true;

  /**
   * Host → JS lifecycle surface for Lilia focus refresh / layout listeners.
   * Payload: { type: "resize"|"focus"|"blur"|"visibilitychange", width?, height?, hidden? }
   * Dispatches on shim EventTarget (`window` or `document`).
   */
  globalThis.__nanaPumpLifecycle = function __nanaPumpLifecycle(payload) {
    const win = globalThis.window;
    const doc = globalThis.document;
    if (!win || !payload || typeof payload !== "object") return false;
    const type = String(payload.type || "");
    if (type === "resize") {
      const w = Math.max(0, Number(payload.width));
      const h = Math.max(0, Number(payload.height));
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
  };

  globalThis.__nanaWebApi = {
    version: "phase4-p0",
    installed: true,
    EventTarget: EventTargetShim,
    CustomEvent: CustomEventShim,
    pumpLifecycle: globalThis.__nanaPumpLifecycle,
  };
})();
