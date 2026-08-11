/**
 * Phase 3 Vue Custom Renderer — hostOps return Rust DOM node handles.
 * Shared by Counter / Todo / upgraded probe.
 */
import { createRenderer, ref, h } from "@vue/runtime-core";

function hostCall(name, args) {
  const host = globalThis.__nanaHost;
  if (!host || typeof host.call !== "function") {
    throw new Error("__nanaHost.call is not registered");
  }
  return host.call(name, args);
}

/** JS-side event listener table (functions don't survive JSON host bridge). */
const listeners = new Map(); // `${nid}:${event}` → fn

function listenerKey(nid, event) {
  return `${nid}:${String(event).toLowerCase()}`;
}

function wrapNode(id, kind, tag) {
  return { __nid: Number(id), __kind: kind || "element", tag: tag || null };
}

function nodeId(node) {
  if (node == null) return null;
  if (typeof node === "number") return node;
  if (typeof node.__nid === "number") return node.__nid;
  return null;
}

const hostOps = {
  patchProp(el, key, _prev, next) {
    const nid = nodeId(el);
    if (typeof key === "string" && key.startsWith("on") && typeof next === "function") {
      const event = key.slice(2).toLowerCase();
      listeners.set(listenerKey(nid, event), next);
      hostCall("patchProp", [nid, key, true]);
      return;
    }
    if (typeof key === "string" && key.startsWith("on") && (next == null || next === false)) {
      const event = key.slice(2).toLowerCase();
      listeners.delete(listenerKey(nid, event));
      hostCall("patchProp", [nid, key, null]);
      return;
    }
    let value = next;
    if (value != null && typeof value === "object" && !Array.isArray(value)) {
      // style object — pass through as plain object (JSON)
      value = { ...value };
    } else if (value == null) {
      value = null;
    } else {
      value = String(value);
    }
    hostCall("patchProp", [nid, String(key), value]);
  },
  insert(child, parent, anchor) {
    hostCall("insert", [nodeId(child), nodeId(parent), nodeId(anchor)]);
  },
  remove(child) {
    const nid = nodeId(child);
    for (const key of [...listeners.keys()]) {
      if (key.startsWith(`${nid}:`)) listeners.delete(key);
    }
    hostCall("remove", [nid]);
  },
  createElement(tag) {
    const id = hostCall("createElement", [String(tag)]);
    return wrapNode(id, "element", String(tag));
  },
  createText(text) {
    const id = hostCall("createText", [String(text)]);
    return wrapNode(id, "text", null);
  },
  createComment(text) {
    const id = hostCall("createComment", [String(text ?? "")]);
    return wrapNode(id, "comment", null);
  },
  setText(node, text) {
    hostCall("setText", [nodeId(node), String(text)]);
  },
  setElementText(el, text) {
    hostCall("setElementText", [nodeId(el), String(text)]);
  },
  parentNode(node) {
    const id = hostCall("parentNode", [nodeId(node)]);
    return id == null ? null : wrapNode(id, "element", null);
  },
  nextSibling(node) {
    const id = hostCall("nextSibling", [nodeId(node)]);
    return id == null ? null : wrapNode(id, "element", null);
  },
  querySelector(sel) {
    const id = hostCall("querySelector", [String(sel)]);
    return id == null ? null : wrapNode(id, "element", null);
  },
};

const { createApp } = createRenderer({
  ...hostOps,
  scheduleJob(job) {
    if (typeof queueMicrotask === "function") {
      queueMicrotask(job);
    } else {
      Promise.resolve().then(job);
    }
  },
  cloneNode(node) {
    const id = hostCall("cloneNode", [nodeId(node), true]);
    return wrapNode(id, node?.__kind || "element", node?.tag || null);
  },
  insertStaticContent(content, parent, anchor) {
    const pair = hostCall("insertStaticContent", [
      String(content ?? ""),
      nodeId(parent),
      nodeId(anchor),
    ]);
    return [wrapNode(pair[0], "element", "span"), wrapNode(pair[1], "element", "span")];
  },
  setScopeId(el, id) {
    hostCall("setScopeId", [nodeId(el), String(id)]);
  },
});

globalThis.__nanaFireEvent = function __nanaFireEvent(nid, event) {
  const fn = listeners.get(listenerKey(nid, event));
  if (typeof fn === "function") {
    fn({ type: String(event), target: Number(nid) });
  }
  return true;
};

function mountRootHandle() {
  const id = hostCall("mountRoot", []);
  return wrapNode(id, "element", "body");
}

function CounterApp() {
  const count = ref(0);
  return {
    setup() {
      return () =>
        h(
          "div",
          {
            class: "counter",
            style: {
              display: "flex",
              "flex-direction": "column",
              gap: "12px",
              padding: "16px",
              width: "280px",
            },
          },
          [
            h(
              "div",
              {
                class: "count",
                style: {
                  "font-size": "28px",
                  "font-weight": "600",
                  padding: "8px",
                },
              },
              String(count.value),
            ),
            h(
              "button",
              {
                class: "inc",
                style: {
                  width: "120px",
                  height: "40px",
                  background: "#3867ff",
                  color: "#ffffff",
                  border: "none",
                },
                onClick: () => {
                  count.value += 1;
                },
              },
              "inc",
            ),
          ],
        );
    },
  };
}

function TodoApp() {
  const text = ref("");
  const items = ref([
    { id: 1, title: "one", done: false },
    { id: 2, title: "two", done: true },
  ]);
  let nextId = 3;
  return {
    setup() {
      return () =>
        h(
          "div",
          {
            class: "todo",
            style: {
              display: "flex",
              "flex-direction": "column",
              gap: "10px",
              padding: "16px",
              width: "320px",
            },
          },
          [
            h("div", { style: { "font-size": "20px", "font-weight": "600" } }, "Todo"),
            h(
              "div",
              { style: { display: "flex", gap: "8px" } },
              [
                h("div", {
                  class: "todo-input",
                  style: {
                    flex: "1",
                    height: "36px",
                    background: "#ffffff",
                    border: "1px solid #ccd",
                    padding: "8px",
                  },
                }, text.value || " "),
                h(
                  "button",
                  {
                    class: "todo-add",
                    style: {
                      width: "72px",
                      height: "36px",
                      background: "#3867ff",
                      color: "#fff",
                    },
                    onClick: () => {
                      const title = (text.value || "item").trim() || "item";
                      items.value = items.value.concat([
                        { id: nextId++, title, done: false },
                      ]);
                      text.value = "";
                    },
                  },
                  "add",
                ),
              ],
            ),
            h(
              "ul",
              {
                class: "todo-list",
                style: {
                  display: "flex",
                  "flex-direction": "column",
                  gap: "6px",
                  margin: "0",
                  padding: "0",
                  "list-style": "none",
                },
              },
              items.value.map((item) =>
                h(
                  "li",
                  {
                    key: item.id,
                    class: item.done ? "todo-item done" : "todo-item",
                    style: {
                      display: "flex",
                      "align-items": "center",
                      gap: "8px",
                      padding: "8px",
                      background: item.done ? "#e8f5e9" : "#ffffff",
                      height: "36px",
                    },
                    onClick: () => {
                      items.value = items.value.map((it) =>
                        it.id === item.id ? { ...it, done: !it.done } : it,
                      );
                    },
                  },
                  [
                    h("span", null, item.done ? "[x]" : "[ ]"),
                    h("span", null, item.title),
                  ],
                ),
              ),
            ),
          ],
        );
    },
  };
}

function runCounter() {
  const root = mountRootHandle();
  const app = createApp(CounterApp());
  app.mount(root);
  hostCall("resolveLayout", []);
  return {
    ok: true,
    app: "counter",
    mountRoot: nodeId(root),
    boxes: hostCall("layoutSnapshot", []).boxes,
  };
}

function runTodo() {
  const root = mountRootHandle();
  const app = createApp(TodoApp());
  app.mount(root);
  hostCall("resolveLayout", []);
  return {
    ok: true,
    app: "todo",
    mountRoot: nodeId(root),
    boxes: hostCall("layoutSnapshot", []).boxes,
  };
}

/** Phase 2 probe parity: reactive counter + host DOM ops. */
function runProbe() {
  const root = mountRootHandle();
  const count = ref(0);
  const App = {
    setup() {
      count.value = 1;
      return () =>
        h("div", { class: "probe" }, [
          h("span", null, String(count.value)),
          h(
            "button",
            {
              onClick: () => {
                count.value += 1;
              },
            },
            "inc",
          ),
        ]);
    },
  };
  const app = createApp(App);
  app.mount(root);
  count.value = 2;
  hostCall("resolveLayout", []);
  return {
    ok: true,
    phase: "phase3",
    vue: true,
    count: count.value,
    mountRoot: nodeId(root),
    boxes: hostCall("layoutSnapshot", []).boxes,
  };
}

globalThis.__nanaVue = {
  runCounter,
  runTodo,
  runProbe,
  createApp,
  hostOps,
  version: "phase3-custom-renderer",
};

// Back-compat for Phase 2 example entrypoints.
globalThis.__nanaProbe = {
  run: runProbe,
  version: "phase3-vue-runtime-core",
};

export { createApp, hostOps, runCounter, runTodo, runProbe };
