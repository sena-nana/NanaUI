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
