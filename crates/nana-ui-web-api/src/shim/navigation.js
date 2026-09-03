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
