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
        if (key === "innerHTML" || key === "textContent") {
          children.set(nid, []);
          const html = String(value ?? "");
          if (html.includes("<")) {
            const child = nextId++;
            children.set(child, []);
            parents.set(child, nid);
            children.set(nid, [child]);
          }
        }
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
        const parent = args[1];
        const anchor = args[2];
        if (parent != null) {
          parents.set(start, parent);
          const kids = children.get(parent) || [];
          const at = anchor != null ? kids.indexOf(anchor) : -1;
          if (at >= 0) kids.splice(at, 0, start);
          else kids.push(start);
          children.set(parent, kids);
        }
        return [start, start];
      }
      if (name === "querySelector" || name === "querySelectorAll" || name === "closest") {
        return name === "querySelectorAll" ? [] : null;
      }
      if (name === "mountRoot") {
        return 1;
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
  let createApp;
  let flushHostFrame;

  beforeEach(async () => {
    prevHost = globalThis.__nanaHost;
    installMockHost();
    // Fresh module instance so nodeCache/listeners reset.
    const mod = await import(`${modUrl}?t=${Date.now()}-${Math.random()}`);
    hostOps = mod.hostOps;
    wrapNode = mod.wrapNode;
    nodeId = mod.nodeId;
    createApp = mod.createApp;
    flushHostFrame = mod.flushHostFrame;
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

  test("wrapNode parent/child cache survives flushHostFrame", () => {
    const TREE_READS = new Set([
      "parentNode",
      "childNodes",
      "firstChild",
      "lastChild",
      "nextSibling",
      "querySelector",
      "contains",
    ]);
    const treeReads = () => calls.filter(([name]) => TREE_READS.has(name));

    const parent = hostOps.createElement("div");
    const a = hostOps.createElement("span");
    const b = hostOps.createElement("span");
    hostOps.insert(a, parent, null);
    hostOps.insert(b, parent, a);
    const afterInsert = treeReads().length;

    assert.equal(a.parentNode, parent);
    assert.equal(b.parentNode, parent);
    assert.equal(parent.firstChild, b);
    assert.equal(parent.lastChild, a);
    assert.deepEqual(
      parent.childNodes.map((n) => n.__nid),
      [nodeId(b), nodeId(a)],
    );
    assert.equal(b.nextSibling, a);
    assert.equal(a.previousSibling, b);
    assert.equal(parent.contains(a), true);
    assert.equal(a.isConnected, false);
    assert.equal(treeReads().length, afterInsert, "insert cache must serve tree getters");

    flushHostFrame();
    assert.equal(a.parentNode, parent);
    assert.equal(parent.firstChild, b);
    assert.equal(parent.childNodes.length, 2);
    assert.equal(b.nextSibling, a);
    assert.equal(
      treeReads().length,
      afterInsert,
      "style flush must not force parentNode/childNodes hostCalls",
    );
  });

  test("insertStaticContent invalidates children cache and refills from host", () => {
    const TREE_READS = new Set(["parentNode", "childNodes", "firstChild", "lastChild"]);
    const treeReads = () => calls.filter(([name]) => TREE_READS.has(name));

    const parent = hostOps.createElement("div");
    const a = hostOps.createElement("span");
    hostOps.insert(a, parent, null);
    assert.equal(parent.childNodes.length, 1);
    const afterInsert = treeReads().length;

    const [start] = hostOps.insertStaticContent("hi", parent, null);
    assert.ok(start);
    const afterStatic = treeReads().length;
    assert.equal(afterStatic, afterInsert, "static insert must not read tree; it invalidates");

    assert.equal(parent.childNodes.length, 2);
    assert.equal(
      treeReads().some(([name]) => name === "childNodes"),
      true,
      "invalidated children cache must refill from host",
    );
    const afterRefill = treeReads().length;
    assert.equal(parent.childNodes.length, 2);
    assert.equal(parent.firstChild, a);
    assert.equal(treeReads().length, afterRefill, "refilled cache must serve subsequent getters");
  });

  test("innerHTML invalidates children cache and refills from host", () => {
    const parent = hostOps.createElement("div");
    const stale = hostOps.createElement("span");
    hostOps.insert(stale, parent, null);
    assert.equal(parent.childNodes.length, 1);

    parent.innerHTML = "<span></span>";
    assert.equal(parent.childNodes.length, 1);
    assert.notEqual(parent.firstChild, stale);
  });

  test("style setProperty batches until flushHostFrame", async () => {
    const el = hostOps.createElement("div");
    const stylePatches = () =>
      calls.filter(([name, args]) => name === "patchProp" && args[1] === "style");
    const before = stylePatches().length;
    el.style.setProperty("color", "red");
    el.style.setProperty("gap", "8px");
    el.style.display = "flex";
    assert.equal(stylePatches().length, before, "setProperty must not hostCall per property");
    assert.equal(el.style.color, "red");
    assert.equal(el.style.gap, "8px");

    flushHostFrame();
    assert.equal(stylePatches().length, before + 1);
    assert.deepEqual(attrs.get(nodeId(el)).style, {
      color: "red",
      gap: "8px",
      display: "flex",
    });

    el.style.removeProperty("gap");
    el.style.setProperty("color", "blue");
    assert.equal(stylePatches().length, before + 1);
    await Promise.resolve();
    assert.equal(stylePatches().length, before + 2);
    assert.equal(attrs.get(nodeId(el)).style.color, "blue");
    assert.equal(attrs.get(nodeId(el)).style.gap, undefined);
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
    const app = createApp({});
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

  test("inline transform is a paint overlay and does not patchProp style", async () => {
    const el = hostOps.createElement("li");
    calls.length = 0;
    el.style.transform = "translate(12px, 4px)";
    if (typeof flushHostFrame === "function") flushHostFrame();
    await Promise.resolve();
    assert.equal(el.style.transform, "translate(12px, 4px)");
    assert.equal(
      calls.some(
        ([name, args]) =>
          name === "setPaintTransform" && args[1] === "translate(12px, 4px)",
      ),
      true,
    );
    assert.equal(
      calls.some(
        ([name, args]) =>
          name === "patchProp" &&
          args[1] === "style" &&
          args[2] &&
          typeof args[2] === "object" &&
          args[2].transform,
      ),
      false,
    );
    calls.length = 0;
    el.style.transform = "";
    assert.equal(
      calls.some(([name, args]) => name === "setPaintTransform" && args[1] === ""),
      true,
    );
  });

  test("host __nanaMotionComplete dispatches transitionend on wrapNode", () => {
    const el = hostOps.createElement("div");
    const seen = [];
    el.addEventListener("transitionend", (event) => {
      seen.push(event.propertyName);
    });
    assert.equal(typeof globalThis.__nanaMotionComplete, "function");
    globalThis.__nanaMotionComplete(nodeId(el), {
      type: "transitionend",
      propertyName: "opacity",
      elapsedTime: 0.24,
    });
    assert.deepEqual(seen, ["opacity"]);
  });

  test("class-arm fallback plus host complete fires transitionend once", async () => {
    const el = hostOps.createElement("div");
    const seen = [];
    el.addEventListener("transitionend", () => seen.push("end"));
    const view =
      (el.ownerDocument && el.ownerDocument.defaultView) || globalThis;
    const previous = view.getComputedStyle;
    view.getComputedStyle = () => ({
      transitionDelay: "0s",
      transitionDuration: "10ms",
      transitionProperty: "opacity",
      animationDelay: "0s",
      animationDuration: "0s",
      animationName: "none",
    });
    try {
      el.classList.add("fade-enter-active", "fade-enter-to");
      assert.equal(typeof globalThis.__nanaMotionCancel, "function");
      globalThis.__nanaMotionComplete(nodeId(el), {
        type: "transitionend",
        propertyName: "opacity",
      });
      await new Promise((resolve) => setTimeout(resolve, 80));
      assert.deepEqual(seen, ["end"]);
    } finally {
      if (previous) view.getComputedStyle = previous;
      else delete view.getComputedStyle;
    }
  });

  test("class replace keeps appear/enter tokens for Vue Transition timing", () => {
    const el = hostOps.createElement("div");
    el.classList.add("fade-appear-from", "fade-appear-active");
    el.classList.__replace("panel");
    assert.ok(el.classList.contains("panel"));
    assert.ok(el.classList.contains("fade-appear-from"));
    assert.ok(el.classList.contains("fade-appear-active"));
    assert.equal(el.__nanaTransitionPhase, "appear-from-active");
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

  test("HTML button seeds createElement rather than createWidget", () => {
    const el = hostOps.createElement("button", undefined, undefined, {
      label: "Save",
      disabled: true,
    });
    assert.equal(el.tag, "button");
    assert.deepEqual(
      calls.find(([name]) => name === "createElement"),
      ["createElement", ["button", null, null, { label: "Save", disabled: true }]],
    );
    assert.equal(
      calls.some(([name]) => name === "createWidget"),
      false,
    );
  });

  test("retired nana-button alias throws instead of creating a layout box", () => {
    assert.throws(
      () => hostOps.createElement("nana-button", undefined, undefined, { label: "Save" }),
      /retired tag `nana-button`; use `button`/,
    );
    assert.equal(calls.length, 0);
  });

  test("HTML table uses createElement; retired nana-table throws", () => {
    const table = hostOps.createElement("table");
    assert.equal(table.tag, "table");
    assert.equal(
      calls.some(([name]) => name === "createWidget"),
      false,
    );
    calls.length = 0;
    assert.throws(
      () => hostOps.createElement("nana-table"),
      /retired tag `nana-table`; use `table`/,
    );
    assert.equal(calls.length, 0);
  });

  test("search-dropdown uses createElement; HTML search is not a widget tag", () => {
    const field = hostOps.createElement("search-dropdown");
    assert.equal(field.tag, "search-dropdown");
    assert.equal(
      calls.some(([name]) => name === "createWidget"),
      false,
    );
  });

  test("nana-drawer still uses createWidget", () => {
    hostOps.createElement("nana-drawer", undefined, undefined, { open: true });
    assert.deepEqual(calls.find(([name]) => name === "createWidget"), [
      "createWidget",
      ["drawer", { open: true }],
    ]);
  });

  test("createApp is the package entry and mount defaults to the body root", () => {
    assert.equal(typeof createApp, "function");
    assert.equal(createApp.length, 2);
    const app = createApp({});
    app.mount();
    assert.ok(calls.some(([name]) => name === "mountRoot"));
  });
});
