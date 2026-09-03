  const win = new WindowShim();
  const windowContexts = new Map();
  windowContexts.set(0, { window: win, document: win.document });

  function scopedObject(target, windowId) {
    if (!target || typeof target !== "object" || typeof Proxy === "undefined") return target;
    return new Proxy(target, {
      get: function (inner, key, receiver) {
        const value = Reflect.get(inner, key, receiver);
        if (typeof value === "function") {
          return function () {
            const args = arguments;
            return withWindowContext(windowId, function () {
              return value.apply(inner, args);
            });
          };
        }
        if (value && typeof value === "object" && (key === "dataset" || key === "style")) {
          return scopedObject(value, windowId);
        }
        return value;
      },
      set: function (inner, key, value, receiver) {
        return withWindowContext(windowId, function () {
          return Reflect.set(inner, key, value, receiver);
        });
      },
    });
  }

  globalThis.__nanaWithWindowContext = withWindowContext;
  globalThis.__nanaCreateWindowContext = function __nanaCreateWindowContext(
    windowId,
    width,
    height,
    scaleFactor,
  ) {
    const id = Number(windowId || 0);
    if (!id) return windowContexts.get(0);
    const existing = windowContexts.get(id);
    if (existing) return existing;
    const rawWindow = new WindowShim();
    const logicalWidth = Math.max(1, Number(width) || 800);
    const logicalHeight = Math.max(1, Number(height) || 600);
    const dpr = Math.max(0.01, Number(scaleFactor) || 1);
    rawWindow.innerWidth = logicalWidth;
    rawWindow.outerWidth = logicalWidth;
    rawWindow.innerHeight = logicalHeight;
    rawWindow.outerHeight = logicalHeight;
    rawWindow.devicePixelRatio = dpr;
    rawWindow.visualViewport.width = logicalWidth;
    rawWindow.visualViewport.height = logicalHeight;
    const rawDocument = rawWindow.document;
    const documentElement = scopedObject(rawDocument.documentElement, id);
    rawDocument.documentElement = documentElement;
    const document = scopedObject(rawDocument, id);
    rawWindow.document = document;
    const window = scopedObject(rawWindow, id);
    const context = { window: window, document: document };
    windowContexts.set(id, context);
    return context;
  };
  globalThis.__nanaGetWindowContext = function __nanaGetWindowContext(windowId) {
    return windowContexts.get(Number(windowId || 0)) || null;
  };
  globalThis.__nanaDestroyWindowContext = function __nanaDestroyWindowContext(windowId) {
    const id = Number(windowId || 0);
    if (!id) return false;
    const context = windowContexts.get(id);
    withWindowContext(id, function () {
      for (const [timerId, entry] of rafCallbacks) {
        if (entry.windowId === id) cancelAnimationFrame(timerId);
      }
      for (const [timerId, entry] of timeoutCallbacks) {
        if (entry.windowId === id) clearTimeoutShim(timerId);
      }
      for (const [timerId, entry] of intervalCallbacks) {
        if (entry.windowId === id) clearIntervalShim(timerId);
      }
      for (const pending of pendingFetches.values()) {
        if (pending.windowId === id) pending.abort();
      }
      for (const socket of pendingSockets.values()) {
        if (socket._wsWindowId === id && socket.readyState < SOCKET_CLOSING) {
          try { socket.close(); } catch (_err) {}
        }
      }
      if (typeof globalThis.__nanaReleaseWindowObservers === "function") {
        globalThis.__nanaReleaseWindowObservers(id);
      }
      if (context) {
        for (const target of [context.window, context.document, context.window.visualViewport]) {
          if (target && typeof target.__nanaClearListeners === "function") {
            target.__nanaClearListeners();
          }
        }
        if (context.window && context.window.__nanaMediaQueries) {
          for (const query of context.window.__nanaMediaQueries) {
            if (query && typeof query.__nanaClearListeners === "function") {
              query.__nanaClearListeners();
            }
          }
          context.window.__nanaMediaQueries.clear();
        }
      }
    });
    return windowContexts.delete(id);
  };
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
  globalThis.Headers = HeadersShim;
  globalThis.Request = RequestShim;
  globalThis.Response = ResponseShim;
  globalThis.AbortSignal = AbortSignalShim;
  globalThis.AbortController = AbortControllerShim;
  globalThis.fetch = fetchShim;
  globalThis.WebSocket = WebSocketShim;
  globalThis.ImageData = ImageDataShim;
  globalThis.CanvasGradient = CanvasGradientShim;
  globalThis.CanvasPattern = CanvasPatternShim;
  globalThis.CanvasRenderingContext2D = CanvasRenderingContext2DShim;
  HTMLCanvasElementShim.prototype = Object.create(globalThis.HTMLElement.prototype);
  HTMLCanvasElementShim.prototype.constructor = HTMLCanvasElementShim;
  globalThis.HTMLCanvasElement = HTMLCanvasElementShim;
  HTMLMediaElementShim.prototype = Object.create(globalThis.HTMLElement.prototype);
  HTMLMediaElementShim.prototype.constructor = HTMLMediaElementShim;
  globalThis.HTMLMediaElement = HTMLMediaElementShim;
  HTMLVideoElementShim.prototype = Object.create(HTMLMediaElementShim.prototype);
  HTMLVideoElementShim.prototype.constructor = HTMLVideoElementShim;
  globalThis.HTMLVideoElement = HTMLVideoElementShim;
  HTMLAudioElementShim.prototype = Object.create(HTMLMediaElementShim.prototype);
  HTMLAudioElementShim.prototype.constructor = HTMLAudioElementShim;
  globalThis.HTMLAudioElement = HTMLAudioElementShim;
  globalThis.Blob = BlobShim;
  globalThis.Image = ImageShim;
  globalThis.ImageBitmap = ImageBitmapShim;
  globalThis.createImageBitmap = createImageBitmapShim;
  globalThis.GPU = GPUShim;
  globalThis.GPUAdapter = GPUAdapterShim;
  globalThis.GPUDevice = GPUDeviceShim;
  globalThis.GPUValidationError = GPUValidationErrorShim;
  globalThis.GPUOutOfMemoryError = GPUOutOfMemoryErrorShim;
  globalThis.GPUInternalError = GPUInternalErrorShim;
  globalThis.GPUQueue = GPUQueueShim;
  globalThis.GPUBuffer = GPUBufferShim;
  globalThis.GPUTexture = GPUTextureShim;
  globalThis.GPUTextureView = GPUTextureViewShim;
  globalThis.GPUSampler = GPUSamplerShim;
  globalThis.GPUShaderModule = GPUShaderModuleShim;
  globalThis.GPUBindGroup = GPUBindGroupShim;
  globalThis.GPUBindGroupLayout = GPUBindGroupLayoutShim;
  globalThis.GPUPipelineLayout = GPUPipelineLayoutShim;
  globalThis.GPURenderPipeline = GPURenderPipelineShim;
  globalThis.GPUComputePipeline = GPUComputePipelineShim;
  globalThis.GPUCommandEncoder = GPUCommandEncoderShim;
  globalThis.GPUCommandBuffer = GPUCommandBufferShim;
  globalThis.GPURenderPassEncoder = GPURenderPassEncoderShim;
  globalThis.GPUComputePassEncoder = GPUComputePassEncoderShim;
  globalThis.GPUCanvasContext = GPUCanvasContextShim;
  globalThis.GPUBufferUsage = Object.freeze({ MAP_READ: 1, MAP_WRITE: 2, COPY_SRC: 4, COPY_DST: 8, INDEX: 16, VERTEX: 32, UNIFORM: 64, STORAGE: 128, INDIRECT: 256, QUERY_RESOLVE: 512 });
  globalThis.GPUTextureUsage = Object.freeze({ COPY_SRC: 1, COPY_DST: 2, TEXTURE_BINDING: 4, STORAGE_BINDING: 8, RENDER_ATTACHMENT: 16 });
  globalThis.GPUShaderStage = Object.freeze({ VERTEX: 1, FRAGMENT: 2, COMPUTE: 4 });
  globalThis.GPUMapMode = Object.freeze({ READ: 1, WRITE: 2 });
  globalThis.GPUColorWrite = Object.freeze({ RED: 1, GREEN: 2, BLUE: 4, ALPHA: 8, ALL: 15 });
  if (typeof globalThis.DOMException === "undefined") {
    globalThis.DOMException = function DOMException(message, name) {
      this.name = name || "Error";
      this.message = String(message || "");
    };
    globalThis.DOMException.prototype = Object.create(Error.prototype);
  }
  if (globalThis.URL) {
    globalThis.URL.createObjectURL = function (resource) {
      const id = resourceId(resource);
      if (id == null) throw new TypeError("createObjectURL requires a Nana resource");
      return hostCall("objectUrlCreate", [id]);
    };
    globalThis.URL.revokeObjectURL = function (url) {
      hostCall("objectUrlRevoke", [String(url || "")]);
    };
  }
  win.Headers = HeadersShim;
  win.Request = RequestShim;
  win.Response = ResponseShim;
  win.AbortSignal = AbortSignalShim;
  win.AbortController = AbortControllerShim;
  win.fetch = fetchShim;
  win.WebSocket = WebSocketShim;
  win.ImageData = ImageDataShim;
  win.CanvasRenderingContext2D = CanvasRenderingContext2DShim;
  win.HTMLCanvasElement = HTMLCanvasElementShim;
  win.HTMLMediaElement = HTMLMediaElementShim;
  win.HTMLVideoElement = HTMLVideoElementShim;
  win.HTMLAudioElement = HTMLAudioElementShim;
  win.Blob = BlobShim;
  win.Image = ImageShim;
  win.ImageBitmap = ImageBitmapShim;
  win.createImageBitmap = createImageBitmapShim;
  win.navigator.gpu = new GPUShim();

  /**
   * Host → JS lifecycle surface for focus refresh / layout listeners.
   * Payload: { type: "resize"|"focus"|"blur"|"visibilitychange", width?, height?, hidden? }
   * Dispatches on shim EventTarget (`window` or `document`).
   */
  function pumpLifecycle(win, doc, payload) {
    if (!win || !payload || typeof payload !== "object") return false;
    const type = String(payload.type || "");
    if (type === "resize") {
      const w = Math.max(0, Number(payload.width));
      const h = Math.max(0, Number(payload.height));
      const dpr = Math.max(0.01, Number(payload.scaleFactor || win.devicePixelRatio || 1));
      win.devicePixelRatio = dpr;
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
      if (win.visualViewport) win.visualViewport.dispatchEvent(new CustomEventShim("resize"));
      refreshMediaQueries(win);
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
  }

  globalThis.__nanaPumpLifecycle = function __nanaPumpLifecycle(payload) {
    return pumpLifecycle(globalThis.window, globalThis.document, payload);
  };

  globalThis.__nanaPumpWindowLifecycle = function __nanaPumpWindowLifecycle(windowId, payload) {
    const context = windowContexts.get(Number(windowId || 0));
    if (!context) return false;
    return withWindowContext(windowId, function () {
      return pumpLifecycle(context.window, context.document, payload);
    });
  };

  globalThis.__nanaWebApi = {
    version: "webview-source-subset-1",
    installed: true,
    EventTarget: EventTargetShim,
    CustomEvent: CustomEventShim,
    pumpLifecycle: globalThis.__nanaPumpLifecycle,
  };
})();
