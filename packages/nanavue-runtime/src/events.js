/** One listener table per renderer. Node identity is supplied by its node store. */
export function createEventDispatcher(wrapById) {
const listeners = new Map();
function listenerKey(nid, event) {
  return `${nid}:${String(event).toLowerCase()}`;
}

const EVENT_OPTIONS_RE = /(Once|Passive|Capture)$/;

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

/** Parse Vue `onClickCapture` / `onClickOnce` → `{ name, options }` (runtime-dom subset). */
function parseEventName(rawName) {
  let name = String(rawName);
  let options;
  let m;
  while ((m = name.match(EVENT_OPTIONS_RE)) && !/^on:?(?:Once|Passive|Capture)$/.test(name)) {
    name = name.slice(0, name.length - m[1].length);
    if (!options) options = {};
    options[m[1].toLowerCase()] = true;
  }
  let event;
  if (name.startsWith("on:") || name.startsWith("on")) {
    const body = name.startsWith("on:") ? name.slice(3) : name.slice(2);
    event = body.replace(/^[A-Z]/, (c) => c.toLowerCase()).toLowerCase();
  } else {
    event = name.toLowerCase();
  }
  return { name: event, options };
}

function normalizeHandler(next) {
  if (typeof next === "function") return next;
  if (Array.isArray(next)) {
    const fns = next.filter((fn) => typeof fn === "function");
    if (!fns.length) return null;
    return (evt) => {
      for (const fn of fns) {
        if (evt && (evt._stopped || evt._immediateStopped)) break;
        fn(evt);
      }
    };
  }
  return null;
}

function isListenerObject(listener) {
  return (
    typeof listener === "function" ||
    (listener != null && typeof listener.handleEvent === "function")
  );
}

function addNanaListener(nid, type, listener, options) {
  if (!isListenerObject(listener)) return;
  const opts = normalizeListenerOptions(options);
  const key = listenerKey(nid, type);
  let list = listeners.get(key);
  if (!list) {
    list = [];
    listeners.set(key, list);
  }
  for (const entry of list) {
    if (entry.listener === listener && entry.capture === opts.capture) return;
  }
  list.push({
    listener,
    capture: opts.capture,
    once: opts.once,
    passive: opts.passive,
  });
}

function removeNanaListener(nid, type, listener, options) {
  const capture = normalizeListenerOptions(options).capture;
  const key = listenerKey(nid, type);
  const list = listeners.get(key);
  if (!list) return;
  const next = list.filter((entry) => !(entry.listener === listener && entry.capture === capture));
  if (next.length) listeners.set(key, next);
  else listeners.delete(key);
}

function invokeNanaListenerPhase(nid, type, event, capture) {
  const list = listeners.get(listenerKey(nid, type));
  if (!list || !list.length) return;
  const snapshot = list.slice();
  for (const entry of snapshot) {
    if (entry.capture !== !!capture) continue;
    if (event && event._immediateStopped) break;
    try {
      if (event) {
        const currentTarget = wrapById(nid) || event.target;
        event.currentTarget = currentTarget;
        event.eventPhase = currentTarget === event.target ? 2 : capture ? 1 : 3;
      }
      if (typeof entry.listener === "function") {
        entry.listener.call(event && event.currentTarget, event);
      } else {
        entry.listener.handleEvent(event);
      }
    } catch (_err) {}
    if (entry.once) removeNanaListener(nid, type, entry.listener, entry.capture);
  }
}

function invokeGlobalPhase(target, type, event, capture) {
  if (!target) return;
  if (typeof target.__nanaInvokePhase === "function") {
    target.__nanaInvokePhase(type, event, capture);
    return;
  }
  // Fallback: plain EventTarget-like with _listeners (tests / partial shims).
  const bag = target._listeners;
  if (!bag) return;
  const list = bag[String(type)];
  if (!list || !list.length) return;
  const snapshot = list.slice();
  for (const entry of snapshot) {
    const isCapture = typeof entry === "object" && entry != null ? !!entry.capture : false;
    if (isCapture !== !!capture) continue;
    if (event && event._immediateStopped) break;
    try {
      if (event) {
        event.currentTarget = target;
        event.eventPhase = capture ? 1 : 3;
      }
      const listener = typeof entry === "function" ? entry : entry.listener;
      if (typeof listener === "function") listener.call(target, event);
      else if (listener && typeof listener.handleEvent === "function") listener.handleEvent(event);
    } catch (_err) {}
  }
}

function createFileList(files) {
  const list = Array.isArray(files) ? files.slice() : [];
  Object.defineProperty(list, "item", {
    configurable: true,
    enumerable: false,
    value(index) {
      return this[Number(index)] || null;
    },
  });
  return list;
}

function createDataTransfer(files) {
  const items = files.map((file) => ({
    kind: "file",
    type: String(file && file.type ? file.type : ""),
    getAsFile() {
      return file || null;
    },
  }));
  Object.defineProperty(items, "item", {
    configurable: true,
    enumerable: false,
    value(index) {
      return this[Number(index)] || null;
    },
  });
  return {
    files,
    items,
    types: files.length ? ["Files"] : [],
    dropEffect: "copy",
    effectAllowed: "copy",
  };
}

function createEventPayload(type, target, detail) {
  const source = detail && typeof detail === "object" ? detail : {};
  const files = createFileList(source.files);
  const payload = {
    type,
    target,
    currentTarget: target,
    detail: source,
    key: source.key,
    code: source.code,
    data: source.data,
    value: source.value,
    checked: source.checked,
    inputType: source.inputType,
    isComposing: !!source.isComposing,
    repeat: !!source.repeat,
    location: Number(source.location || 0),
    clientX: Number(source.clientX || 0),
    clientY: Number(source.clientY || 0),
    x: Number(source.x ?? source.clientX ?? 0),
    y: Number(source.y ?? source.clientY ?? 0),
    offsetX: Number(source.offsetX ?? source.clientX ?? 0),
    offsetY: Number(source.offsetY ?? source.clientY ?? 0),
    screenX: Number(source.screenX || 0),
    screenY: Number(source.screenY || 0),
    button: Number(source.button ?? 0),
    buttons: Number(source.buttons ?? 0),
    pressure: Number(source.pressure || 0),
    tangentialPressure: Number(source.tangentialPressure || 0),
    tiltX: Number(source.tiltX || 0),
    tiltY: Number(source.tiltY || 0),
    twist: Number(source.twist || 0),
    pointerId: Number(source.pointerId || 0),
    pointerType: source.pointerType || "",
    isPrimary: !!source.isPrimary,
    relatedTarget:
      source.relatedTarget == null ? null : wrapById(Number(source.relatedTarget)),
    altKey: !!source.altKey,
    ctrlKey: !!source.ctrlKey,
    metaKey: !!source.metaKey,
    shiftKey: !!source.shiftKey,
    deltaX: Number(source.deltaX || 0),
    deltaY: Number(source.deltaY || 0),
    deltaMode: Number(source.deltaMode || 0),
    files,
    dataTransfer: source.dataTransfer || createDataTransfer(files),
    bubbles: !/^(pointerenter|pointerleave|mouseenter|mouseleave|focus|blur)$/.test(type),
    cancelable: true,
    defaultPrevented: false,
    eventPhase: 0,
    _stopped: false,
    _immediateStopped: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    stopPropagation() {
      this._stopped = true;
    },
    stopImmediatePropagation() {
      this._stopped = true;
      this._immediateStopped = true;
    },
  };
  for (const key of Object.keys(source)) {
    if (!Object.prototype.hasOwnProperty.call(payload, key)) {
      payload[key] = source[key];
    }
  }
  return payload;
}

/** window capture → document capture → target → document bubble → window bubble. */
function fanOutDocumentWindow(payload, type) {
  const win = globalThis.window;
  const doc = globalThis.document;
  invokeGlobalPhase(win, type, payload, true);
  if (payload._stopped) return;
  invokeGlobalPhase(doc, type, payload, true);
  if (payload._stopped) return;
  invokeGlobalPhase(doc, type, payload, false);
  if (payload._stopped) return;
  invokeGlobalPhase(win, type, payload, false);
}


function releaseNodeListeners(nid) {
  for (const key of [...listeners.keys()]) {
    if (key.startsWith(`${nid}:`)) listeners.delete(key);
  }
}

return { listenerKey, normalizeListenerOptions, parseEventName, normalizeHandler, isListenerObject, addNanaListener, removeNanaListener, invokeNanaListenerPhase, invokeGlobalPhase, createFileList, createDataTransfer, createEventPayload, fanOutDocumentWindow, releaseNodeListeners };
}
