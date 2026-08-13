/**
 * Behavior: createNanaRenderer hostOps align with Vue RendererOptions + patchProp.
 */
import assert from "node:assert/strict";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { describe, test, beforeEach, afterEach } from "node:test";
import { register } from "node:module";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const stubUrl = pathToFileURL(join(root, "tests/vue-runtime-core-stub.mjs")).href;
const hooksUrl = pathToFileURL(join(root, "tests/vue-stub-hooks.mjs")).href;
register(hooksUrl, import.meta.url, { data: { stub: stubUrl } });

const modUrl = pathToFileURL(join(root, "src/createNanaRenderer.js")).href;

const calls = [];
const parents = new Map();
const children = new Map();
const attrs = new Map();
let nextId = 1;

function installMockHost() {
  calls.length = 0;
  parents.clear();
  children.clear();
  attrs.clear();
  nextId = 1;
  globalThis.__nanaHost = {
    call(name, args) {
      calls.push([name, args]);
      if (name === "createElement" || name === "createText" || name === "createComment") {
        const id = nextId++;
        children.set(id, []);
        return id;
      }
      if (name === "createWidget") {
        const id = nextId++;
        children.set(id, []);
        return id;
      }
      if (name === "insert") {
        const [child, parent, anchor] = args;
        // detach
        for (const [, kids] of children) {
          const i = kids.indexOf(child);
          if (i >= 0) kids.splice(i, 1);
        }
        parents.set(child, parent);
        const kids = children.get(parent) || [];
        const at = anchor != null ? kids.indexOf(anchor) : -1;
        if (at >= 0) kids.splice(at, 0, child);
        else kids.push(child);
        children.set(parent, kids);
        return null;
      }
      if (name === "remove") {
        const [child] = args;
        const p = parents.get(child);
        if (p != null) {
          const kids = children.get(p) || [];
          const i = kids.indexOf(child);
          if (i >= 0) kids.splice(i, 1);
        }
        parents.delete(child);
        return null;
      }
      if (name === "parentNode") return parents.get(args[0]) ?? null;
      if (name === "childNodes") return [...(children.get(args[0]) || [])];
      if (name === "nextSibling") {
        const nid = args[0];
        const p = parents.get(nid);
        if (p == null) return null;
        const kids = children.get(p) || [];
        const i = kids.indexOf(nid);
        return i >= 0 ? kids[i + 1] ?? null : null;
      }
      if (name === "contains") {
        const [a, b] = args;
        let cur = b;
        while (cur != null) {
          if (cur === a) return true;
          cur = parents.get(cur);
        }
        return false;
      }
      if (name === "patchProp") {
        const [nid, key, value] = args;
        const map = attrs.get(nid) || {};
        if (value == null) delete map[key];
        else map[key] = value;
        attrs.set(nid, map);
        return null;
      }
      if (name === "setElementText" || name === "setText") {
        const [nid, text] = args;
        const map = attrs.get(nid) || {};
        map.__text = text;
        attrs.set(nid, map);
        return null;
      }
      if (name === "setScopeId") return null;
      if (name === "cloneNode") {
        const id = nextId++;
        children.set(id, []);
        return id;
      }
      if (name === "insertStaticContent") {
        const start = nextId++;
        children.set(start, []);
        return [start, start];
      }
      if (name === "querySelector" || name === "querySelectorAll" || name === "closest") {
        return name === "querySelectorAll" ? [] : null;
      }
      if (name === "layoutBox") {
        return { x: 0, y: 0, width: 0, height: 0 };
      }
      return null;
    },
  };
}

describe("hostOps Vue RendererOptions contract", () => {
  let prevHost;
  let hostOps;
  let wrapNode;
  let nodeId;
  let createNanaApp;

  beforeEach(async () => {
    prevHost = globalThis.__nanaHost;
    installMockHost();
    // Fresh module instance so nodeCache/listeners reset.
    const mod = await import(`${modUrl}?t=${Date.now()}-${Math.random()}`);
    hostOps = mod.hostOps;
    wrapNode = mod.wrapNode;
    nodeId = mod.nodeId;
    createNanaApp = mod.createNanaApp;
  });

  afterEach(() => {
    globalThis.__nanaHost = prevHost;
  });

  test("required nodeOps exist", () => {
    for (const name of [
      "patchProp",
      "insert",
      "remove",
      "createElement",
      "createText",
      "createComment",
      "setText",
      "setElementText",
      "parentNode",
      "nextSibling",
      "querySelector",
      "setScopeId",
      "cloneNode",
      "insertStaticContent",
    ]) {
      assert.equal(typeof hostOps[name], "function", name);
    }
  });

  test("insert/parentNode/nextSibling/remove keep stable node identity", () => {
    const parent = hostOps.createElement("div");
    const a = hostOps.createElement("span");
    const b = hostOps.createElement("span");
    hostOps.insert(a, parent, null);
    hostOps.insert(b, parent, null);
    assert.equal(hostOps.parentNode(a), parent);
    assert.equal(hostOps.nextSibling(a), b);
    assert.equal(hostOps.parentNode(a), a.parentNode);
    assert.equal(parent.contains(a), true);
    assert.equal(a.contains(b), false);
    hostOps.remove(a);
    assert.equal(hostOps.parentNode(a), null);
  });

  test("removing a subtree releases renderer-owned image and Canvas resources", () => {
    const parent = hostOps.createElement("div");
    const canvas = hostOps.createElement("canvas");
    const image = hostOps.createElement("img");
    let imageClosed = 0;
    canvas.__nanaOwnsCanvasResource = true;
    canvas.__nanaCanvasResource = { id: 41n };
    canvas.__nanaResource = canvas.__nanaCanvasResource;
    image.__nanaOwnedImage = { close() { imageClosed += 1; } };
    hostOps.insert(canvas, parent, null);
    hostOps.insert(image, canvas, null);

    hostOps.remove(canvas);

    assert.equal(imageClosed, 1);
    assert.deepEqual(
      calls.filter(([name]) => name === "resourceRelease"),
      [["resourceRelease", [41n]]],
    );
    assert.equal(canvas.__nanaCanvasResource, null);
  });

  test("patchProp class syncs classList", () => {
    const el = hostOps.createElement("div");
    hostOps.patchProp(el, "class", null, "foo bar");
    assert.equal(el.classList.contains("foo"), true);
    assert.equal(el.classList.contains("bar"), true);
    hostOps.patchProp(el, "class", "foo bar", null);
    assert.equal(el.classList.contains("foo"), false);
  });

  test("patchProp style accepts object and string; null clears", () => {
    const el = hostOps.createElement("div");
    hostOps.patchProp(el, "style", null, { display: "flex", gap: "8px" });
    assert.deepEqual(attrs.get(nodeId(el)).style, { display: "flex", gap: "8px" });
    hostOps.patchProp(el, "style", null, "color:red");
    assert.equal(attrs.get(nodeId(el)).style, "color:red");
    hostOps.patchProp(el, "style", "color:red", null);
    assert.equal(attrs.get(nodeId(el)).style, undefined);
  });

  test("Vue warning and error handlers report structured diagnostics", () => {
    const app = createNanaApp().createApp({});
    app.config.warnHandler("bad prop", null, "component trace");
    app.config.errorHandler(new Error("render failed"), null, "render");
    const reports = calls.filter(([name]) => name === "diagnosticReport");
    assert.equal(reports.length, 2);
    assert.equal(reports[0][1][0].source, "vue.warn");
    assert.equal(reports[0][1][0].level, "warning");
    assert.equal(reports[1][1][0].source, "vue.error");
    assert.match(reports[1][1][0].stack, /render failed/);
  });

  test("nana-gpu source routes texture handles through a stable slot", () => {
    const source = {
      __nanaTexture: true,
      slot: "live2d:main",
      id: 9007199254740993n,
      generation: 4,
      version: 12,
      width: 1024,
      height: 1024,
      alphaMode: "premultiplied",
    };
    const el = hostOps.createElement("nana-gpu", undefined, undefined, { source });
    assert.deepEqual(
      calls.findLast(([name]) => name === "setGpuSlot"),
      ["setGpuSlot", [nodeId(el), "live2d:main"]],
    );
    assert.equal(el.__nanaGpuSource, source);

    hostOps.patchProp(el, "source", source, { id: 7n, generation: 2 });
    assert.deepEqual(calls.at(-1), ["setGpuSlot", [nodeId(el), "texture:7:2"]]);
  });

  test("patchProp events skip onUpdate and accept handler arrays", () => {
    const el = hostOps.createElement("button");
    let hits = 0;
    hostOps.patchProp(el, "onUpdate:modelValue", null, () => {
      hits += 1;
    });
    assert.equal(
      calls.some(([n, a]) => n === "patchProp" && a[1] === "onUpdate:modelValue"),
      false,
    );
    hostOps.patchProp(el, "onClick", null, [
      () => {
        hits += 1;
      },
      () => {
        hits += 10;
      },
    ]);
    globalThis.__nanaFireEvent(nodeId(el), "click", {});
    assert.equal(hits, 11);
  });

  test("patchProp innerHTML/textContent route as domProps", () => {
    const el = hostOps.createElement("article");
    hostOps.patchProp(el, "innerHTML", null, "<b>x</b>");
    assert.equal(attrs.get(nodeId(el)).innerHTML, "<b>x</b>");
    hostOps.patchProp(el, "textContent", null, "plain");
    assert.equal(attrs.get(nodeId(el)).textContent, "plain");
  });

  test("insertStaticContent accepts Vue namespace arity", () => {
    const parent = hostOps.createElement("div");
    const [start, end] = hostOps.insertStaticContent("hi", parent, null, undefined);
    assert.ok(start);
    assert.ok(end);
    assert.equal(typeof nodeId(start), "number");
  });

  test("wrapNode caches by nid", () => {
    const a = wrapNode(42, "element", "div");
    const b = wrapNode(42, "element", "div");
    assert.equal(a, b);
  });

  test("registered native components use semantic tags and promise commands", async () => {
    const el = hostOps.createElement("nana-live2d-view", undefined, undefined, {
      modelId: "m1",
      paused: false,
    });
    assert.deepEqual(calls.find(([name]) => name === "createWidget"), [
      "createWidget",
      ["live2d-view", { modelId: "m1", paused: false }],
    ]);

    const previousHost = globalThis.Nana.host;
    globalThis.Nana.host = {
      invoke(name, args) {
        return Promise.resolve({ name, args });
      },
    };
    try {
      assert.deepEqual(await globalThis.Nana.components.call(el, "play-motion", { group: "tap" }), {
        name: "componentCall",
        args: [nodeId(el), "play-motion", { group: "tap" }],
      });
    } finally {
      globalThis.Nana.host = previousHost;
    }
  });
});
