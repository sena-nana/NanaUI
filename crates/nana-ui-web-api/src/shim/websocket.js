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
