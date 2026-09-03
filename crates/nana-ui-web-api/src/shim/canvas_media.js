  function resourceId(value) {
    if (value == null) return null;
    if (typeof value === "bigint" || typeof value === "number") return value;
    if (value.__nanaResource && value.id != null) return value.id;
    if (value.__nanaCanvasResource && value.__nanaCanvasResource.id != null) {
      return value.__nanaCanvasResource.id;
    }
    return null;
  }

  function asUint8Array(value) {
    if (ArrayBuffer.isView && ArrayBuffer.isView(value)) {
      return new Uint8Array(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
    }
    if (value instanceof ArrayBuffer) return new Uint8Array(value.slice(0));
    return Uint8Array.from(value || []);
  }

  function ImageDataShim(dataOrWidth, widthOrHeight, maybeHeight) {
    if (typeof dataOrWidth === "number") {
      this.width = Math.max(1, Math.trunc(Number(dataOrWidth) || 1));
      this.height = Math.max(1, Math.trunc(Number(widthOrHeight) || 1));
      this.data = new Uint8ClampedArray(this.width * this.height * 4);
    } else {
      this.data = new Uint8ClampedArray(asUint8Array(dataOrWidth));
      this.width = Math.max(1, Math.trunc(Number(widthOrHeight) || 1));
      this.height = maybeHeight == null
        ? Math.max(1, Math.trunc(this.data.length / (this.width * 4)))
        : Math.max(1, Math.trunc(Number(maybeHeight) || 1));
      if (this.data.length !== this.width * this.height * 4) {
        throw new DOMException("ImageData byte length does not match dimensions", "IndexSizeError");
      }
    }
    this.colorSpace = "srgb";
  }

  function CanvasGradientShim(kind, args) {
    this.kind = kind;
    this.args = Array.from(args, Number);
    this.stops = [];
  }
  CanvasGradientShim.prototype.addColorStop = function (offset, color) {
    const value = Number(offset);
    if (!Number.isFinite(value) || value < 0 || value > 1) {
      throw new DOMException("Color stop offset must be between 0 and 1", "IndexSizeError");
    }
    this.stops.push([value, String(color)]);
    this.stops.sort(function (a, b) { return a[0] - b[0]; });
  };
  EventTargetShim.prototype.__nanaClearListeners = function () {
    this._listeners = Object.create(null);
  };

  function CanvasPatternShim(source, repetition) {
    const id = resourceId(source);
    if (id == null) throw new TypeError("Canvas pattern source has no Nana image resource");
    this.kind = "pattern";
    this.sourceId = id;
    this.repetition = repetition == null || repetition === "" ? "repeat" : String(repetition);
    this.transform = [1, 0, 0, 1, 0, 0];
  }
  CanvasPatternShim.prototype.setTransform = function (matrix) {
    this.transform = [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f].map(Number);
  };

  function CanvasRenderingContext2DShim(canvas) {
    this.canvas = canvas;
    this._id = canvas.__nanaCanvasResource.id;
    this._lineDash = [];
    this._fillStyle = "#000000";
    this._strokeStyle = "#000000";
    this._lineWidth = 1;
    this._lineCap = "butt";
    this._lineJoin = "miter";
    this._lineDashOffset = 0;
    this._globalAlpha = 1;
    this._globalCompositeOperation = "source-over";
    this._font = "10px sans-serif";
  }
  function HTMLCanvasElementShim() {}

  function gpuResourceId(value) {
    if (value == null) return null;
    if (typeof value === "bigint" || typeof value === "number") return value;
    const resource = value.__nanaGpuResource || value;
    return resource && resource.id != null ? resource.id : null;
  }

  function GPUObjectBase(resource) {
    this.__nanaGpuResource = resource;
    this.id = resource.id;
    this.label = resource.label || "";
    this.generation = resource.generation;
  }
  GPUObjectBase.prototype.destroy = function () {
    if (this.__nanaGpuResource) hostCall("webgpuResourceRelease", [this.id]);
    this.__nanaGpuResource = null;
  };

  function GPUBufferShim(resource, descriptor, device) {
    GPUObjectBase.call(this, resource);
    this.size = Number(resource.size || descriptor.size || 0);
    this.usage = Number(descriptor.usage || 0);
    this.mapState = descriptor.mappedAtCreation ? "mapped" : "unmapped";
    this._device = device;
    this._mapped = descriptor.mappedAtCreation ? new ArrayBuffer(this.size) : null;
  }
  GPUBufferShim.prototype = Object.create(GPUObjectBase.prototype);
  GPUBufferShim.prototype.constructor = GPUBufferShim;
  GPUBufferShim.prototype.getMappedRange = function (offset, size) {
    if (this.mapState !== "mapped" || !this._mapped) throw new DOMException("Buffer is not mapped", "OperationError");
    const begin = Number(offset || 0);
    const length = size == null ? this._mapped.byteLength - begin : Number(size);
    if (begin !== 0 || length !== this._mapped.byteLength) {
      throw new DOMException("Nana write mapping currently exposes the complete mapped range", "NotSupportedError");
    }
    return this._mapped;
  };
  GPUBufferShim.prototype.mapAsync = function (mode, offset, size) {
    return Promise.reject(new DOMException("GPUBuffer.mapAsync is not implemented by the Nana WebGPU subset", "NotSupportedError"));
  };
  GPUBufferShim.prototype.unmap = function () {
    if (this._mapped) hostCall("webgpuBufferUnmapInitial", [this.id, new Uint8Array(this._mapped)]);
    this._mapped = null;
    this.mapState = "unmapped";
  };

  function GPUTextureViewShim(resource) { GPUObjectBase.call(this, resource); }
  GPUTextureViewShim.prototype = Object.create(GPUObjectBase.prototype);
  GPUTextureViewShim.prototype.constructor = GPUTextureViewShim;
  function GPUTextureShim(resource, device) {
    GPUObjectBase.call(this, resource);
    this.width = Number(resource.width || 1);
    this.height = Number(resource.height || 1);
    this.depthOrArrayLayers = Number(resource.depthOrArrayLayers || 1);
    this.format = resource.format || "rgba8unorm";
    this._device = device;
  }
  GPUTextureShim.prototype = Object.create(GPUObjectBase.prototype);
  GPUTextureShim.prototype.constructor = GPUTextureShim;
  GPUTextureShim.prototype.createView = function (descriptor) {
    return new GPUTextureViewShim(hostCall("webgpuTextureCreateView", [this.id, descriptor || {}]));
  };
  GPUTextureShim.prototype.destroy = function () { hostCall("webgpuTextureDestroy", [this.id]); this.__nanaGpuResource = null; };

  function simpleGpuType(name) {
    const ctor = function (resource) { GPUObjectBase.call(this, resource); };
    Object.defineProperty(ctor, "name", { value: name });
    ctor.prototype = Object.create(GPUObjectBase.prototype);
    ctor.prototype.constructor = ctor;
    return ctor;
  }
  const GPUSamplerShim = simpleGpuType("GPUSampler");
  const GPUShaderModuleShim = simpleGpuType("GPUShaderModule");
  GPUShaderModuleShim.prototype.getCompilationInfo = function () {
    return Promise.reject(new DOMException("Shader compilation info is not exposed by the Nana WebGPU subset", "NotSupportedError"));
  };
  const GPUBindGroupLayoutShim = simpleGpuType("GPUBindGroupLayout");
  const GPUPipelineLayoutShim = simpleGpuType("GPUPipelineLayout");
  const GPUBindGroupShim = simpleGpuType("GPUBindGroup");
  const GPURenderPipelineShim = simpleGpuType("GPURenderPipeline");
  const GPUComputePipelineShim = simpleGpuType("GPUComputePipeline");
  const GPUCommandBufferShim = simpleGpuType("GPUCommandBuffer");

  function normalizeGpuDescriptor(value) {
    if (value == null || typeof value !== "object") return value;
    const id = gpuResourceId(value);
    if (id != null) return {
      id: id,
      generation: Number(value.generation || (value.__nanaGpuResource && value.__nanaGpuResource.generation) || 0),
      kind: value.__nanaGpuResource && value.__nanaGpuResource.kind || value.kind || "",
    };
    if (Array.isArray(value)) return value.map(normalizeGpuDescriptor);
    if (ArrayBuffer.isView && ArrayBuffer.isView(value)) return Array.from(value);
    const result = {};
    for (const key of Object.keys(value)) result[key] = normalizeGpuDescriptor(value[key]);
    return result;
  }

  function GPURenderPassEncoderShim(resource) { GPUObjectBase.call(this, resource); this._ended = false; }
  GPURenderPassEncoderShim.prototype._command = function (name, args) {
    if (this._ended) throw new DOMException("Render pass has ended", "InvalidStateError");
    hostCall("webgpuPassCommand", [this.id, name, Array.from(args, normalizeGpuDescriptor)]);
  };
  [
    "setPipeline", "setBindGroup", "setVertexBuffer", "setIndexBuffer",
    "setViewport", "setScissorRect", "setBlendConstant", "setStencilReference",
    "draw", "drawIndexed"
  ].forEach(function (name) {
    GPURenderPassEncoderShim.prototype[name] = function () { this._command(name, arguments); };
  });
  GPURenderPassEncoderShim.prototype.end = function () { if (!this._ended) hostCall("webgpuEndPass", [this.id]); this._ended = true; };

  function GPUComputePassEncoderShim(resource) { GPUObjectBase.call(this, resource); this._ended = false; }
  GPUComputePassEncoderShim.prototype._command = GPURenderPassEncoderShim.prototype._command;
  ["setPipeline", "setBindGroup", "dispatchWorkgroups"].forEach(function (name) {
    GPUComputePassEncoderShim.prototype[name] = function () { this._command(name, arguments); };
  });
  GPUComputePassEncoderShim.prototype.end = GPURenderPassEncoderShim.prototype.end;

  function GPUCommandEncoderShim(resource) { GPUObjectBase.call(this, resource); this._finished = false; }
  GPUCommandEncoderShim.prototype.beginRenderPass = function (descriptor) {
    return new GPURenderPassEncoderShim(hostCall("webgpuBeginPass", [this.id, "render", normalizeGpuDescriptor(descriptor || {})]));
  };
  GPUCommandEncoderShim.prototype.beginComputePass = function (descriptor) {
    return new GPUComputePassEncoderShim(hostCall("webgpuBeginPass", [this.id, "compute", normalizeGpuDescriptor(descriptor || {})]));
  };
  GPUCommandEncoderShim.prototype.copyBufferToBuffer = function (source, sourceOffset, destination, destinationOffset, size) {
    hostCall("webgpuEncoderCopyBuffer", [this.id, gpuResourceId(source), sourceOffset, gpuResourceId(destination), destinationOffset, size]);
  };
  GPUCommandEncoderShim.prototype.finish = function () {
    if (this._finished) throw new DOMException("Command encoder is already finished", "InvalidStateError");
    this._finished = true;
    return new GPUCommandBufferShim(hostCall("webgpuFinishEncoder", [this.id]));
  };

  function GPUQueueShim(device) { this._device = device; this.label = ""; }
  GPUQueueShim.prototype.writeBuffer = function (buffer, bufferOffset, data, dataOffset, size) {
    let bytes = asUint8Array(data);
    const start = Number(dataOffset || 0);
    const end = size == null ? bytes.byteLength : start + Number(size);
    bytes = bytes.slice(start, end);
    hostCall("webgpuQueueWriteBuffer", [gpuResourceId(buffer), Number(bufferOffset || 0), bytes]);
  };
  GPUQueueShim.prototype.writeTexture = function (destination, data, dataLayout, size) {
    hostCall("webgpuQueueWriteTexture", [normalizeGpuDescriptor(destination), asUint8Array(data), dataLayout || {}, size]);
  };
  GPUQueueShim.prototype.submit = function (commandBuffers) {
    hostCall("webgpuQueueSubmit", [Array.from(commandBuffers || [], gpuResourceId)]);
  };
  GPUQueueShim.prototype.onSubmittedWorkDone = function () {
    if (!globalThis.Nana || !globalThis.Nana.host || typeof globalThis.Nana.host.invoke !== "function") {
      return Promise.reject(new DOMException("Nana async host bridge is unavailable", "InvalidStateError"));
    }
    return globalThis.Nana.host.invoke("webgpuQueueSubmittedWorkDone", []);
  };

  function GPUErrorShim(record, fallbackName) {
    this.name = String(record && record.name || fallbackName || "GPUError");
    this.message = String(record && record.message || "WebGPU operation failed");
    this.code = String(record && record.code || "");
    if (Error.captureStackTrace) Error.captureStackTrace(this, GPUErrorShim);
  }
  GPUErrorShim.prototype = Object.create(Error.prototype);
  GPUErrorShim.prototype.constructor = GPUErrorShim;
  function GPUValidationErrorShim(message) { GPUErrorShim.call(this, { name: "GPUValidationError", message }, "GPUValidationError"); }
  GPUValidationErrorShim.prototype = Object.create(GPUErrorShim.prototype);
  GPUValidationErrorShim.prototype.constructor = GPUValidationErrorShim;
  function GPUOutOfMemoryErrorShim(message) { GPUErrorShim.call(this, { name: "GPUOutOfMemoryError", message }, "GPUOutOfMemoryError"); }
  GPUOutOfMemoryErrorShim.prototype = Object.create(GPUErrorShim.prototype);
  GPUOutOfMemoryErrorShim.prototype.constructor = GPUOutOfMemoryErrorShim;
  function GPUInternalErrorShim(message) { GPUErrorShim.call(this, { name: "GPUInternalError", message }, "GPUInternalError"); }
  GPUInternalErrorShim.prototype = Object.create(GPUErrorShim.prototype);
  GPUInternalErrorShim.prototype.constructor = GPUInternalErrorShim;

  function gpuErrorFromRecord(record) {
    if (!record) return null;
    const message = String(record.message || "WebGPU operation failed");
    const error = record.name === "GPUOutOfMemoryError"
      ? new GPUOutOfMemoryErrorShim(message)
      : record.name === "GPUInternalError"
        ? new GPUInternalErrorShim(message)
        : new GPUValidationErrorShim(message);
    error.code = String(record.code || error.code || "");
    return error;
  }

  function GPUDeviceShim(info) {
    this.label = "Nana host device";
    this.features = new Set();
    this.limits = {};
    this.adapterInfo = info;
    this.queue = new GPUQueueShim(this);
    let resolveLost;
    this.lost = new Promise(function (resolve) { resolveLost = resolve; });
    this.__resolveLost = resolveLost;
  }
  GPUDeviceShim.prototype.createBuffer = function (descriptor) { return new GPUBufferShim(hostCall("webgpuCreateBuffer", [normalizeGpuDescriptor(descriptor)]), descriptor, this); };
  GPUDeviceShim.prototype.createTexture = function (descriptor) { return new GPUTextureShim(hostCall("webgpuCreateTexture", [normalizeGpuDescriptor(descriptor)]), this); };
  GPUDeviceShim.prototype.createSampler = function (descriptor) { return new GPUSamplerShim(hostCall("webgpuCreateSampler", [descriptor || {}])); };
  GPUDeviceShim.prototype.createShaderModule = function (descriptor) { return new GPUShaderModuleShim(hostCall("webgpuCreateShaderModule", [descriptor || {}])); };
  GPUDeviceShim.prototype.createBindGroupLayout = function (descriptor) { return new GPUBindGroupLayoutShim(hostCall("webgpuCreateBindGroupLayout", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createPipelineLayout = function (descriptor) { return new GPUPipelineLayoutShim(hostCall("webgpuCreatePipelineLayout", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createBindGroup = function (descriptor) { return new GPUBindGroupShim(hostCall("webgpuCreateBindGroup", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createRenderPipeline = function (descriptor) { return new GPURenderPipelineShim(hostCall("webgpuCreateRenderPipeline", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createRenderPipelineAsync = function () {
    return Promise.reject(new DOMException("Asynchronous render pipeline compilation is not implemented by the Nana WebGPU subset", "NotSupportedError"));
  };
  GPUDeviceShim.prototype.createComputePipeline = function (descriptor) { return new GPUComputePipelineShim(hostCall("webgpuCreateComputePipeline", [normalizeGpuDescriptor(descriptor)])); };
  GPUDeviceShim.prototype.createComputePipelineAsync = function () {
    return Promise.reject(new DOMException("Asynchronous compute pipeline compilation is not implemented by the Nana WebGPU subset", "NotSupportedError"));
  };
  GPUDeviceShim.prototype.createCommandEncoder = function () { return new GPUCommandEncoderShim(hostCall("webgpuCreateCommandEncoder", [])); };
  GPUDeviceShim.prototype.pushErrorScope = function (filter) {
    hostCall("webgpuPushErrorScope", [String(filter)]);
  };
  GPUDeviceShim.prototype.popErrorScope = function () {
    if (!globalThis.Nana || !globalThis.Nana.host || typeof globalThis.Nana.host.invoke !== "function") {
      return Promise.reject(new DOMException("Nana async host bridge is unavailable", "InvalidStateError"));
    }
    return globalThis.Nana.host.invoke("webgpuPopErrorScope", []).then(gpuErrorFromRecord);
  };
  GPUDeviceShim.prototype.destroy = function () { if (this.__resolveLost) this.__resolveLost({ reason: "destroyed", message: "GPUDevice destroyed" }); };

  function GPUAdapterShim(info) {
    this.info = info;
    this.features = new Set();
    this.limits = {};
    this.isFallbackAdapter = false;
  }
  GPUAdapterShim.prototype.requestDevice = function (descriptor) {
    const requested = descriptor && typeof descriptor === "object" ? descriptor : {};
    if (Array.isArray(requested.requiredFeatures) && requested.requiredFeatures.length) {
      return Promise.reject(new DOMException("Required WebGPU features are not available in the Nana subset", "NotSupportedError"));
    }
    if (requested.requiredLimits && Object.keys(requested.requiredLimits).length) {
      return Promise.reject(new DOMException("Required WebGPU limits are not available in the Nana subset", "NotSupportedError"));
    }
    if (!globalThis.__nanaGpuDevice) globalThis.__nanaGpuDevice = new GPUDeviceShim(this.info);
    return Promise.resolve(globalThis.__nanaGpuDevice);
  };
  GPUAdapterShim.prototype.requestAdapterInfo = function () { return Promise.resolve(this.info); };

  function GPUShim() {}
  GPUShim.prototype.requestAdapter = function () {
    try { return Promise.resolve(new GPUAdapterShim(hostCall("webgpuAdapterInfo", []))); }
    catch (_error) { return Promise.resolve(null); }
  };
  GPUShim.prototype.getPreferredCanvasFormat = function () { return "rgba8unorm"; };

  function GPUCanvasContextShim(canvas) { this.canvas = canvas; this._device = null; this._texture = null; }
  GPUCanvasContextShim.prototype.configure = function (descriptor) {
    if (!descriptor || !(descriptor.device instanceof GPUDeviceShim)) throw new TypeError("GPUCanvasContext.configure requires a Nana GPUDevice");
    this._device = descriptor.device;
    const request = {
      format: descriptor.format || "rgba8unorm",
      usage: descriptor.usage == null ? GPUTextureUsage.RENDER_ATTACHMENT : descriptor.usage,
      alphaMode: descriptor.alphaMode || "premultiplied",
      width: this.canvas.width,
      height: this.canvas.height,
    };
    this._texture = new GPUTextureShim(hostCall("webgpuCanvasConfigure", [this.canvas.__nanaResource.id, request]), descriptor.device);
    const slot = this._texture.__nanaGpuResource.slot;
    if (slot && typeof this.canvas.setAttribute === "function") this.canvas.setAttribute("data-nana-gpu", slot);
  };
  GPUCanvasContextShim.prototype.getCurrentTexture = function () {
    if (!this._device) throw new DOMException("GPUCanvasContext is not configured", "InvalidStateError");
    this._texture = new GPUTextureShim(hostCall("webgpuCanvasCurrentTexture", [this.canvas.__nanaResource.id]), this._device);
    return this._texture;
  };
  GPUCanvasContextShim.prototype.unconfigure = function () { if (this._texture) this._texture.destroy(); this._texture = this._device = null; };

  globalThis.__nanaWebGpuDeviceLost = function (message) {
    const device = globalThis.__nanaGpuDevice;
    if (device && device.__resolveLost) device.__resolveLost({ reason: "unknown", message: String(message || "GPU device was replaced") });
    globalThis.__nanaGpuDevice = null;
  };
  CanvasRenderingContext2DShim.prototype._call = function (name, args) {
    return hostCall("canvasCommand", [this._id, name, Array.from(args || [])]);
  };
  function canvasMethod(name) {
    CanvasRenderingContext2DShim.prototype[name] = function () { return this._call(name, arguments); };
  }
  [
    "save", "restore", "beginPath", "closePath", "moveTo", "lineTo",
    "quadraticCurveTo", "bezierCurveTo", "rect", "arc", "ellipse", "fill", "stroke",
    "clip", "clearRect", "fillRect", "strokeRect", "translate", "rotate", "scale",
    "transform", "setTransform", "resetTransform", "fillText", "strokeText",
  ].forEach(canvasMethod);
  CanvasRenderingContext2DShim.prototype.drawImage = function (source) {
    const id = resourceId(source);
    if (id == null) throw new TypeError("drawImage source has no Nana image resource");
    const args = [id].concat(Array.prototype.slice.call(arguments, 1));
    return this._call("drawImage", args);
  };
  CanvasRenderingContext2DShim.prototype.measureText = function (text) {
    return this._call("measureText", [String(text)]);
  };
  CanvasRenderingContext2DShim.prototype.createLinearGradient = function () {
    return new CanvasGradientShim("linear", arguments);
  };
  CanvasRenderingContext2DShim.prototype.createRadialGradient = function () {
    return new CanvasGradientShim("radial", arguments);
  };
  CanvasRenderingContext2DShim.prototype.createPattern = function (source, repetition) {
    return new CanvasPatternShim(source, repetition);
  };
  CanvasRenderingContext2DShim.prototype.createImageData = function (a, b) {
    if (a instanceof ImageDataShim) return new ImageDataShim(a.width, a.height);
    return new ImageDataShim(a, b);
  };
  CanvasRenderingContext2DShim.prototype.getImageData = function (x, y, width, height) {
    const result = hostCall("canvasGetImageData", [this._id, x, y, width, height]);
    return new ImageDataShim(new Uint8ClampedArray(result.data), result.width, result.height);
  };
  CanvasRenderingContext2DShim.prototype.putImageData = function (imageData, dx, dy) {
    if (!(imageData instanceof ImageDataShim)) throw new TypeError("putImageData requires ImageData");
    hostCall("canvasPutImageData", [this._id, imageData.data, imageData.width, imageData.height, dx, dy]);
  };
  CanvasRenderingContext2DShim.prototype.setLineDash = function (segments) {
    const values = Array.from(segments || [], Number);
    if (values.some(function (value) { return !Number.isFinite(value) || value < 0; })) {
      throw new DOMException("Line dash values must be finite and non-negative", "IndexSizeError");
    }
    this._lineDash = values.length % 2 ? values.concat(values) : values;
    hostCall("canvasSetState", [this._id, "lineDash", this._lineDash]);
  };
  CanvasRenderingContext2DShim.prototype.getLineDash = function () { return this._lineDash.slice(); };
  function canvasState(name, initial, validate) {
    const privateName = "_" + name;
    Object.defineProperty(CanvasRenderingContext2DShim.prototype, name, {
      configurable: true,
      get: function () { return this[privateName] == null ? initial : this[privateName]; },
      set: function (value) {
        const next = validate ? validate(value, this[privateName] == null ? initial : this[privateName]) : value;
        this[privateName] = next;
        hostCall("canvasSetState", [this._id, name, next]);
      },
    });
  }
  canvasState("fillStyle", "#000000");
  canvasState("strokeStyle", "#000000");
  canvasState("lineWidth", 1, function (v, old) { v = Number(v); return v > 0 && Number.isFinite(v) ? v : old; });
  canvasState("lineCap", "butt", function (v, old) { v = String(v); return ["butt", "round", "square"].includes(v) ? v : old; });
  canvasState("lineJoin", "miter", function (v, old) { v = String(v); return ["miter", "round", "bevel"].includes(v) ? v : old; });
  canvasState("lineDashOffset", 0, function (v) { return Number(v) || 0; });
  canvasState("globalAlpha", 1, function (v, old) { v = Number(v); return v >= 0 && v <= 1 ? v : old; });
  canvasState("globalCompositeOperation", "source-over", function (v) { return String(v); });
  canvasState("font", "10px sans-serif", function (v) { return String(v); });

  function enhanceCanvasElement(element) {
    if (!element || element.__nanaCanvasResource) return element;
    let width = 300;
    let height = 150;
    const resource = hostCall("canvasCreate", [width, height]);
    element.__nanaCanvasResource = resource;
    element.__nanaResource = resource;
    element.__nanaOwnsCanvasResource = true;
    let context2d = null;
    Object.defineProperty(element, "width", {
      configurable: true,
      get: function () { return width; },
      set: function (value) {
        width = Math.max(1, Math.trunc(Number(value) || 300));
        hostCall("canvasResize", [resource.id, width, height]);
      },
    });
    Object.defineProperty(element, "height", {
      configurable: true,
      get: function () { return height; },
      set: function (value) {
        height = Math.max(1, Math.trunc(Number(value) || 150));
        hostCall("canvasResize", [resource.id, width, height]);
      },
    });
    let contextGpu = null;
    let contextKind = null;
    element.getContext = function (kind) {
      const type = String(kind).toLowerCase();
      if (type !== "2d" && type !== "webgpu") return null;
      if (contextKind && contextKind !== type) return null;
      contextKind = type;
      if (type === "2d") return context2d || (context2d = new CanvasRenderingContext2DShim(element));
      if (type === "webgpu") return contextGpu || (contextGpu = new GPUCanvasContextShim(element));
      return null;
    };
    element.toDataURL = function (type, quality) {
      const mime = type || "image/png";
      const bytes = hostCall("canvasEncode", [resource.id, mime, quality]);
      return hostCall("dataUrlFromBytes", [bytes, mime]);
    };
    element.toBlob = function (callback, type, quality) {
      const mime = type || "image/png";
      const bytes = hostCall("canvasEncode", [resource.id, mime, quality]);
      const blob = new BlobShim([bytes], { type: mime });
      queueMicrotask(function () { callback(blob); });
    };
    if (typeof element.setAttribute === "function") {
      try { element.setAttribute("data-nana-canvas", String(resource.id)); } catch (_err) {}
    }
    if (globalThis.HTMLCanvasElement && globalThis.HTMLCanvasElement.prototype) {
      try { Object.setPrototypeOf(element, globalThis.HTMLCanvasElement.prototype); } catch (_err) {}
    }
    return element;
  }
  globalThis.__nanaEnhanceCanvas = enhanceCanvasElement;

  function enhanceMediaElement(element, kind) {
    if (!element || element.__nanaMediaResource) return element;
    const type = kind === "audio" ? "audio" : "video";
    const resource = hostCall("mediaCreate", [type]);
    element.__nanaMediaResource = resource;
    element.__nanaOwnsMediaResource = true;
    if (resource && resource.id != null && typeof element.setAttribute === "function") {
      try { element.setAttribute("data-nana-media", String(resource.id)); } catch (_err) {}
    }
    element.paused = true;
    element.ended = false;
    element.muted = false;
    element.volume = 1;
    element.currentTime = 0;
    element.duration = 0;
    element.readyState = 0;
    element.videoWidth = 0;
    element.videoHeight = 0;
    let src = "";
    function applyDescriptor(next) {
      if (!next || typeof next !== "object") return;
      element.__nanaMediaResource = next;
      element.paused = !!next.paused;
      element.duration = Number(next.duration || 0);
      element.currentTime = Number(next.currentTime || 0);
      element.readyState = Number(next.readyState || 0);
      element.videoWidth = Number(next.width || 0);
      element.videoHeight = Number(next.height || 0);
      if (typeof element.setAttribute === "function") {
        try {
          if (next.id != null) {
            element.setAttribute("data-nana-media", String(next.id));
          }
          if (type === "video") {
            if (next.hasVideoFrame && next.id != null) {
              element.setAttribute("data-nana-video", String(next.id));
            } else {
              element.setAttribute("data-nana-video", "");
            }
          }
        } catch (_err) {}
      }
    }
    Object.defineProperty(element, "src", {
      configurable: true,
      get: function () { return src; },
      set: function (value) {
        src = String(value || "");
        applyDescriptor(hostCall("mediaSetSrc", [resource.id, src]));
      },
    });
    Object.defineProperty(element, "srcObject", {
      configurable: true,
      get: function () { return element.__nanaSrcObject || null; },
      set: function (stream) {
        element.__nanaSrcObject = stream || null;
        const streamId = stream && stream.id != null ? stream.id : 0;
        applyDescriptor(hostCall("mediaSetSrcObject", [resource.id, streamId]));
      },
    });
    Object.defineProperty(element, "currentTime", {
      configurable: true,
      get: function () { return Number(element.__nanaMediaResource && element.__nanaMediaResource.currentTime || 0); },
      set: function (value) {
        applyDescriptor(hostCall("mediaSetCurrentTime", [resource.id, Number(value) || 0]));
      },
    });
    element.play = function () {
      applyDescriptor(hostCall("mediaPlay", [resource.id]));
      return Promise.resolve();
    };
    element.pause = function () {
      applyDescriptor(hostCall("mediaPause", [resource.id]));
    };
    if (globalThis.HTMLMediaElement && globalThis.HTMLMediaElement.prototype) {
      try { Object.setPrototypeOf(element, type === "audio" ? globalThis.HTMLAudioElement.prototype : globalThis.HTMLVideoElement.prototype); } catch (_err) {}
    }
    return element;
  }
  globalThis.__nanaEnhanceMedia = enhanceMediaElement;

  function HTMLMediaElementShim() {}
  function HTMLVideoElementShim() {}
  function HTMLAudioElementShim() {}

  function BlobShim(parts, options) {
    const chunks = [];
    let length = 0;
    for (const part of parts || []) {
      const bytes = typeof part === "string" ? new TextEncoder().encode(part) : asUint8Array(part);
      chunks.push(bytes);
      length += bytes.byteLength;
    }
    const joined = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) { joined.set(chunk, offset); offset += chunk.byteLength; }
    this.type = String(options && options.type || "").toLowerCase();
    this.size = joined.byteLength;
    this.__nanaResource = hostCall("blobCreate", [joined, this.type]);
  }
  BlobShim.prototype.arrayBuffer = function () {
    if (!this.__nanaResource) return Promise.reject(new DOMException("Blob has been released", "InvalidStateError"));
    const bytes = hostCall("resourceBytes", [this.__nanaResource.id]);
    return Promise.resolve(asUint8Array(bytes).buffer);
  };
  BlobShim.prototype.text = function () { return this.arrayBuffer().then(function (bytes) { return new TextDecoder().decode(bytes); }); };
  BlobShim.prototype.slice = function (start, end, type) {
    if (!this.__nanaResource) throw new DOMException("Blob has been released", "InvalidStateError");
    const bytes = asUint8Array(hostCall("resourceBytes", [this.__nanaResource.id])).slice(start || 0, end == null ? this.size : end);
    return new BlobShim([bytes], { type: type || "" });
  };
  BlobShim.prototype.close = function () {
    if (this.__nanaResource) hostCall("resourceRelease", [this.__nanaResource.id]);
    this.__nanaResource = null;
  };

  function ImageBitmapShim(resource) {
    this.__nanaResource = resource;
    this.width = resource.width;
    this.height = resource.height;
  }
  ImageBitmapShim.prototype.close = function () {
    if (this.__nanaResource) hostCall("resourceRelease", [this.__nanaResource.id]);
    this.__nanaResource = null;
  };

  function ImageShim() {
    EventTargetShim.call(this);
    this.complete = false;
    this.naturalWidth = 0;
    this.naturalHeight = 0;
    this.width = 0;
    this.height = 0;
    this._src = "";
    this.__nanaResource = null;
    this._loadGeneration = 0;
    this._loadController = null;
    this._decodePromise = Promise.resolve(this);
  }
  ImageShim.prototype = Object.create(EventTargetShim.prototype);
  ImageShim.prototype.constructor = ImageShim;
  Object.defineProperty(ImageShim.prototype, "src", {
    get: function () { return this._src; },
    set: function (value) {
      const self = this;
      const generation = ++this._loadGeneration;
      if (this._loadController) this._loadController.abort();
      this._loadController = new AbortControllerShim();
      if (this.__nanaResource) hostCall("resourceRelease", [this.__nanaResource.id]);
      this.__nanaResource = null;
      this._src = String(value || "");
      this.complete = false;
      let load;
      if (/^data:/i.test(this._src)) {
        const comma = this._src.indexOf(",");
        const head = this._src.slice(0, comma);
        const body = this._src.slice(comma + 1);
        if (/;base64/i.test(head)) {
          load = Promise.resolve(decodeBase64(body));
        } else load = Promise.resolve(new TextEncoder().encode(decodeURIComponent(body)));
      } else if (/^blob:nana\//.test(this._src)) {
        load = Promise.resolve(hostCall("objectUrlBytes", [this._src]));
      } else {
        load = fetch(this._src, { signal: this._loadController.signal }).then(function (response) {
          if (!response.ok) throw new Error("HTTP " + response.status);
          return response.arrayBuffer();
        });
      }
      this._decodePromise = load.then(function (bytes) {
        if (self._loadGeneration !== generation) throw abortError();
        const resource = hostCall("imageDecode", [asUint8Array(bytes)]);
        if (self._loadGeneration !== generation) {
          hostCall("resourceRelease", [resource.id]);
          throw abortError();
        }
        self.__nanaResource = resource;
        self.naturalWidth = self.width = resource.width;
        self.naturalHeight = self.height = resource.height;
        self.complete = true;
        self.dispatchEvent(new CustomEventShim("load"));
        return self;
      }, function (error) {
        if (self._loadGeneration === generation && !(error && error.name === "AbortError")) {
          self.dispatchEvent(new CustomEventShim("error"));
        }
        throw error;
      });
      this._decodePromise.catch(function () {});
    },
  });
  ImageShim.prototype.decode = function () { return this._decodePromise; };
  ImageShim.prototype.close = function () {
    ++this._loadGeneration;
    if (this._loadController) this._loadController.abort();
    this._loadController = null;
    if (this.__nanaResource) hostCall("resourceRelease", [this.__nanaResource.id]);
    this.__nanaResource = null;
    this.complete = false;
  };

  function createImageBitmapShim(source) {
    const id = resourceId(source);
    if (id == null) return Promise.reject(new TypeError("createImageBitmap source is unsupported"));
    const args = Array.prototype.slice.call(arguments, 1);
    let request = {};
    if (args.length >= 4 && args.slice(0, 4).every(function (value) { return Number.isFinite(Number(value)); })) {
      request = {
        sx: Number(args[0]), sy: Number(args[1]),
        sw: Number(args[2]), sh: Number(args[3]),
        ...(args[4] && typeof args[4] === "object" ? args[4] : {}),
      };
    } else if (args[0] && typeof args[0] === "object") {
      request = { ...args[0] };
    }
    return Promise.resolve(new ImageBitmapShim(hostCall("imageBitmapCreate", [id, request])));
  }

  DocumentShim.prototype.createElement = function (tag) {
    const t = String(tag).toLowerCase();
    if (t === "template") {
      const tmpl = Object.create(globalThis.HTMLElement.prototype);
      tmpl.tagName = "TEMPLATE";
      tmpl.nodeName = "TEMPLATE";
      tmpl.nodeType = 1;
      tmpl.content = createTemplateContent();
      Object.defineProperty(tmpl, "innerHTML", {
        get: function () {
          return serializeHtmlNodes(tmpl.content.childNodes);
        },
        set: function (v) {
          var kids = parseHtmlFragment(String(v ?? ""));
          for (var i = 0; i < kids.length; i++) kids[i].parentNode = tmpl.content;
          tmpl.content.childNodes = kids;
        },
        configurable: true,
      });
      return tmpl;
    }
    // Detached element stub (not in host tree) — prefer Vue hostOps createElement.
    const el = Object.create(globalThis.HTMLElement.prototype);
    el.tagName = t.toUpperCase();
    el.nodeName = el.tagName;
    el.nodeType = 1;
    el.style = {};
    el.dataset = {};
    el.className = "";
    el.classList = {
      add: function () {},
      remove: function () {},
      contains: function () {
        return false;
      },
      toggle: function () {
        return false;
      },
    };
    el.setAttribute = function () {};
    el.getAttribute = function () {
      return null;
    };
    el.removeAttribute = function () {};
    el.appendChild = function () {};
    el.removeChild = function () {};
    el.addEventListener = function () {};
    el.removeEventListener = function () {};
    el.textContent = "";
    el.innerHTML = "";
    return t === "canvas" ? enhanceCanvasElement(el)
      : t === "video" ? enhanceMediaElement(el, "video")
      : t === "audio" ? enhanceMediaElement(el, "audio")
      : el;
  };
  DocumentShim.prototype.createElementNS = function (_ns, tag) {
    return this.createElement(tag);
  };
  DocumentShim.prototype.createTextNode = function (text) {
    return { nodeType: 3, textContent: String(text ?? "") };
  };
  DocumentShim.prototype.getElementById = function (id) {
    try {
      const found = hostCall("querySelector", ["#" + String(id ?? "")]);
      return found == null ? null : wrapHostNode(found, null);
    } catch (_err) {
      return null;
    }
  };
  DocumentShim.prototype.querySelector = function (sel) {
    try {
      const raw = String(sel ?? "");
      const id = hostCall("querySelector", [raw]);
      if (id == null) return null;
      // Same body/html tag hint as hostOps.querySelector (Teleport target stability).
      return wrapHostNode(id, teleportTargetTag(raw));
    } catch (_err) {
      return null;
    }
  };
  DocumentShim.prototype.querySelectorAll = function (sel) {
    try {
      const raw = String(sel ?? "");
      const ids = hostCall("querySelectorAll", [raw]) || [];
      const tag = teleportTargetTag(raw);
      return Array.from(ids, function (id) {
        return wrapHostNode(id, tag);
      });
    } catch (_err) {
      return [];
    }
  };
  DocumentShim.prototype.hasFocus = function () {
    const win = globalThis.window;
    if (!win) return false;
    // Default true until host pumps blur (matches initial focused window).
    return win.__nanaFocused !== false;
  };
