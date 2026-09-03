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
