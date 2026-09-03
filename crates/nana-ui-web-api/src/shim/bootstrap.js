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
