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
