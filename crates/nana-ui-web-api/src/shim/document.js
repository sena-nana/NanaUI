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
