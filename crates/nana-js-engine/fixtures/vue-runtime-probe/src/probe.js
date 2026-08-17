/**
 * Dual-engine Vue `@vue/runtime-core` probe with stub host renderer ops.
 *
 * Proves both JS engines can load runtime-core and share one HostApiRegistry
 * surface. Product UI is retained in UiWorld/UiScene; this artifact does not paint.
 */
import { createRenderer, ref, h } from "@vue/runtime-core";

function hostCall(name, args) {
  const host = globalThis.__nanaHost;
  if (!host || typeof host.call !== "function") {
    throw new Error("__nanaHost.call is not registered");
  }
  return host.call(name, args);
}

const hostOps = {
  patchProp(el, key, _prev, next) {
    el.props = el.props || {};
    el.props[key] = next;
    hostCall("patchProp", [String(key), next == null ? null : String(next)]);
  },
  insert(child, parent, _anchor) {
    child.parent = parent;
    parent.children = parent.children || [];
    parent.children.push(child);
    hostCall("insert", [child.tag || child.type || "node"]);
  },
  remove(child) {
    const parent = child.parent;
    if (parent && parent.children) {
      parent.children = parent.children.filter((c) => c !== child);
    }
    hostCall("remove", [child.tag || child.type || "node"]);
  },
  createElement(tag) {
    hostCall("createElement", [String(tag)]);
    return { type: "element", tag: String(tag), children: [], props: {} };
  },
  createText(text) {
    hostCall("createText", [String(text)]);
    return { type: "text", text: String(text) };
  },
  createComment(text) {
    return { type: "comment", text: String(text) };
  },
  setText(node, text) {
    node.text = String(text);
    hostCall("setText", [String(text)]);
  },
  setElementText(el, text) {
    el.text = String(text);
    hostCall("setElementText", [String(text)]);
  },
  parentNode(node) {
    return node.parent || null;
  },
  nextSibling(_node) {
    return null;
  },
};

// Run reactive jobs synchronously so QuickJS/V8 probes need no browser event loop.
const { createApp } = createRenderer({
  ...hostOps,
  scheduleJob(job) {
    job();
  },
});

function runProbe() {
  const root = { type: "root", children: [] };
  const count = ref(0);

  const App = {
    setup() {
      count.value = 1;
      hostCall("reactiveSet", [count.value]);
      return () =>
        h("div", { class: "probe" }, [
          h("span", null, String(count.value)),
          h(
            "button",
            {
              onClick: () => {
                count.value += 1;
                hostCall("increment", [count.value]);
              },
            },
            "inc",
          ),
        ]);
    },
  };

  const app = createApp(App);
  app.mount(root);

  // Simulate a host-driven reactive update after mount.
  count.value = 2;
  hostCall("increment", [count.value]);

  const snapshot = hostCall("snapshot", []);
  return {
    ok: true,
    phase: "phase2",
    vue: true,
    count: count.value,
    host: snapshot,
  };
}

globalThis.__nanaProbe = {
  run: runProbe,
  version: "phase2-vue-runtime-core",
};
