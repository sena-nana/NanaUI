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

  // Bare V8 has no TextEncoder/TextDecoder -- they are Web APIs, not
  // ECMAScript -- so these are the real implementations for every Nana page,
  // not a rarely-taken fallback. Both work in batches: the obvious
  // byte-at-a-time versions build one rope node (decode) or one JS array slot
  // (encode) per character, which measured ~1.8 s for a 16 MiB `response.text()`
  // and dominated everything else on that path by two orders of magnitude.
  const TEXT_BATCH = 8192;
  if (typeof globalThis.TextEncoder === "undefined") {
    globalThis.TextEncoder = function TextEncoder() {
      this.encode = function (str) {
        const s = String(str ?? "");
        // UTF-8 never needs more than 3 bytes per UTF-16 code unit (a surrogate
        // pair is 2 units for 4 bytes), so one allocation is always enough.
        const out = new Uint8Array(s.length * 3);
        let at = 0;
        for (let i = 0; i < s.length; i++) {
          const code = s.charCodeAt(i);
          if (code < 0x80) {
            out[at++] = code;
          } else if (code < 0x800) {
            out[at++] = 0xc0 | (code >> 6);
            out[at++] = 0x80 | (code & 0x3f);
          } else if (code >= 0xd800 && code <= 0xdbff && i + 1 < s.length) {
            const next = s.charCodeAt(++i);
            const cp = 0x10000 + ((code - 0xd800) << 10) + (next - 0xdc00);
            out[at++] = 0xf0 | (cp >> 18);
            out[at++] = 0x80 | ((cp >> 12) & 0x3f);
            out[at++] = 0x80 | ((cp >> 6) & 0x3f);
            out[at++] = 0x80 | (cp & 0x3f);
          } else {
            out[at++] = 0xe0 | (code >> 12);
            out[at++] = 0x80 | ((code >> 6) & 0x3f);
            out[at++] = 0x80 | (code & 0x3f);
          }
        }
        return out.slice(0, at);
      };
    };
  }
  if (typeof globalThis.TextDecoder === "undefined") {
    globalThis.TextDecoder = function TextDecoder() {
      this.decode = function (input) {
        const bytes = input instanceof Uint8Array ? input : new Uint8Array(input || []);
        const length = bytes.length;
        if (length === 0) return "";
        let out = "";
        let ascii = true;
        for (let i = 0; i < length; i++) {
          if (bytes[i] >= 0x80) { ascii = false; break; }
        }
        if (ascii) {
          // The common case for JSON and text bodies: hand whole slices of the
          // byte view straight to fromCharCode.
          for (let i = 0; i < length; i += TEXT_BATCH) {
            const end = i + TEXT_BATCH < length ? i + TEXT_BATCH : length;
            out += String.fromCharCode.apply(null, bytes.subarray(i, end));
          }
          return out;
        }
        const units = [];
        let i = 0;
        while (i < length) {
          const b = bytes[i++];
          if (b < 0x80) {
            units.push(b);
          } else if (b < 0xe0) {
            const b2 = bytes[i++];
            units.push(((b & 0x1f) << 6) | (b2 & 0x3f));
          } else if (b < 0xf0) {
            const b2 = bytes[i++];
            const b3 = bytes[i++];
            units.push(((b & 0x0f) << 12) | ((b2 & 0x3f) << 6) | (b3 & 0x3f));
          } else {
            const b2 = bytes[i++];
            const b3 = bytes[i++];
            const b4 = bytes[i++];
            let cp = ((b & 0x07) << 18) | ((b2 & 0x3f) << 12) | ((b3 & 0x3f) << 6) | (b4 & 0x3f);
            cp -= 0x10000;
            units.push(0xd800 + (cp >> 10), 0xdc00 + (cp & 0x3ff));
          }
          if (units.length >= TEXT_BATCH) {
            out += String.fromCharCode.apply(null, units);
            units.length = 0;
          }
        }
        if (units.length) out += String.fromCharCode.apply(null, units);
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
