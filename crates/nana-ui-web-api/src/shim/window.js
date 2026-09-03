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
