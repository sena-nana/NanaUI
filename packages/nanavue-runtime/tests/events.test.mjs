/**
 * Track C: multi-listener + capture/options subset + document/window fan-out
 * (Lilia useDismissableLayer / ContextMenu pointerdown + Escape).
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { describe, test, beforeEach } from "node:test";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const shimSrc = readFileSync(
  join(root, "../../crates/nana-ui-web-api/src/shim.js"),
  "utf8",
);
const rendererSrc = readFileSync(join(root, "src/createNanaRenderer.js"), "utf8");
const layoutMetricsSrc = readFileSync(join(root, "src/layoutMetrics.js"), "utf8");

function stripEsm(src) {
  return src
    .replace(/^import\s+[^;]+;?\s*$/gm, "")
    .replace(/^export\s+\{[^}]+\}\s+from\s+[^;]+;?\s*$/gm, "")
    .replace(/^export\s+\{[^}]+\};?\s*$/gm, "")
    .replace(/^export\s+(default\s+)?/gm, "");
}

function loadRuntime() {
  const sandbox = {
    console,
    queueMicrotask: (fn) => Promise.resolve().then(fn),
    Promise,
    Map,
    Set,
    WeakMap,
    Object,
    Array,
    Number,
    String,
    Math,
    TypeError,
    JSON,
    Date,
    Symbol,
    Boolean,
    RegExp,
  };
  sandbox.globalThis = sandbox;
  sandbox.__nanaHost = {
    call() {
      return null;
    },
  };
  sandbox.createRenderer = () => ({
    createApp() {
      return {};
    },
    render() {},
  });

  vm.runInNewContext(shimSrc, sandbox, { filename: "shim.js" });
  vm.runInNewContext(
    `
    ${stripEsm(layoutMetricsSrc)}
    globalThis.hostCall = hostCall;
    globalThis.layoutRect = layoutRect;
    globalThis.defineLayoutMetrics = defineLayoutMetrics;
    `,
    sandbox,
    { filename: "layoutMetrics.js" },
  );
  const renderer = `
    (function () {
      const createRenderer = globalThis.createRenderer;
      const defineLayoutMetrics = globalThis.defineLayoutMetrics;
      const hostCall = globalThis.hostCall;
      const layoutRect = globalThis.layoutRect;
      ${stripEsm(rendererSrc)}
      globalThis.wrapNode = wrapNode;
      globalThis.hostOps = hostOps;
    })();
  `;
  vm.runInNewContext(renderer, sandbox, { filename: "createNanaRenderer.js" });
  return sandbox;
}

describe("EventTargetShim multi-listener + capture", () => {
  test("same event invokes all listeners; capture before bubble", () => {
    const sandbox = loadRuntime();
    const order = [];
    const a = () => order.push("a-bubble");
    const b = () => order.push("b-bubble");
    const c = () => order.push("c-capture");
    sandbox.document.addEventListener("pointerdown", a);
    sandbox.document.addEventListener("pointerdown", b);
    sandbox.document.addEventListener("pointerdown", c, true);
    sandbox.document.dispatchEvent({ type: "pointerdown" });
    assert.deepEqual(order, ["c-capture", "a-bubble", "b-bubble"]);
  });

  test("removeEventListener matches capture flag", () => {
    const sandbox = loadRuntime();
    let n = 0;
    const fn = () => {
      n += 1;
    };
    sandbox.window.addEventListener("pointerdown", fn, true);
    sandbox.window.removeEventListener("pointerdown", fn); // bubble — no match
    sandbox.window.dispatchEvent({ type: "pointerdown" });
    assert.equal(n, 1);
    sandbox.window.removeEventListener("pointerdown", fn, true);
    sandbox.window.dispatchEvent({ type: "pointerdown" });
    assert.equal(n, 1);
  });

  test("once option removes after first invoke", () => {
    const sandbox = loadRuntime();
    let n = 0;
    sandbox.document.addEventListener(
      "keydown",
      () => {
        n += 1;
      },
      { once: true },
    );
    sandbox.document.dispatchEvent({ type: "keydown", key: "Escape" });
    sandbox.document.dispatchEvent({ type: "keydown", key: "Escape" });
    assert.equal(n, 1);
  });
});

describe("Lilia dismiss / ContextMenu fan-out smoke", () => {
  let sandbox;

  beforeEach(() => {
    sandbox = loadRuntime();
  });

  test("document capture pointerdown closes like useDismissableLayer", () => {
    let open = true;
    const onDocPointer = (event) => {
      if (!open) return;
      const nid = event.target && event.target.__nid;
      if (nid === 2) open = false;
    };
    sandbox.document.addEventListener("pointerdown", onDocPointer, true);
    sandbox.__nanaFireEvent(2, "pointerdown", {});
    assert.equal(open, false);
  });

  test("window capture pointerdown closes like ContextMenu", () => {
    let open = true;
    sandbox.window.addEventListener(
      "pointerdown",
      () => {
        open = false;
      },
      true,
    );
    sandbox.__nanaFireEvent(5, "pointerdown", {});
    assert.equal(open, false);
  });

  test("Escape on keydown fans out to document and window", () => {
    const hits = [];
    sandbox.document.addEventListener("keydown", (e) => {
      if (e.key === "Escape") hits.push("doc");
    });
    sandbox.window.addEventListener("keydown", (e) => {
      if (e.key === "Escape") hits.push("win");
    });
    sandbox.__nanaFireEvent(2, "keydown", { key: "Escape", code: "Escape" });
    assert.deepEqual(hits, ["doc", "win"]);
  });

  test("multi listener on wrapNode + Vue onClickCapture coexist", () => {
    const order = [];
    const el = sandbox.wrapNode(9, "element", "div");
    el.addEventListener("click", () => order.push("add"), true);
    sandbox.hostOps.patchProp(el, "onClick", null, () => order.push("vue"));
    sandbox.hostOps.patchProp(el, "onClickCapture", null, () => order.push("vue-cap"));
    sandbox.__nanaFireEvent(9, "click", {});
    assert.ok(order.includes("add"));
    assert.ok(order.includes("vue"));
    assert.ok(order.includes("vue-cap"));
    const addIdx = order.indexOf("add");
    const vueCapIdx = order.indexOf("vue-cap");
    const vueIdx = order.indexOf("vue");
    assert.ok(vueCapIdx < vueIdx);
    assert.ok(addIdx < vueIdx);
  });

  test("stopPropagation on capture prevents later document bubble peers", () => {
    const hits = [];
    sandbox.window.addEventListener(
      "pointerdown",
      (e) => {
        hits.push("win-cap");
        e.stopPropagation();
      },
      true,
    );
    sandbox.document.addEventListener(
      "pointerdown",
      () => {
        hits.push("doc-cap");
      },
      true,
    );
    sandbox.__nanaFireEvent(3, "pointerdown", {});
    assert.deepEqual(hits, ["win-cap"]);
  });

  test("stopImmediatePropagation skips remaining same-phase peers", () => {
    const hits = [];
    sandbox.document.addEventListener(
      "keydown",
      (e) => {
        hits.push("first");
        e.stopImmediatePropagation();
      },
      true,
    );
    sandbox.document.addEventListener(
      "keydown",
      () => {
        hits.push("second");
      },
      true,
    );
    sandbox.__nanaFireEvent(1, "keydown", { key: "Escape" });
    assert.deepEqual(hits, ["first"]);
  });

  test("press and click aliases both reach Vue onClick", () => {
    let hits = 0;
    const el = sandbox.wrapNode(11, "element", "button");
    sandbox.hostOps.patchProp(el, "onClick", null, () => {
      hits += 1;
    });
    sandbox.__nanaFireEvent(11, "press", {});
    sandbox.__nanaFireEvent(11, "click", {});
    assert.equal(hits, 2);
  });

  test("fan-out order is window-cap → doc-cap → target → doc-bubble → win-bubble", () => {
    const order = [];
    const el = sandbox.wrapNode(7, "element", "div");
    sandbox.window.addEventListener("click", () => order.push("win-cap"), true);
    sandbox.document.addEventListener("click", () => order.push("doc-cap"), true);
    el.addEventListener("click", () => order.push("tgt-cap"), true);
    el.addEventListener("click", () => order.push("tgt-bub"));
    sandbox.document.addEventListener("click", () => order.push("doc-bub"));
    sandbox.window.addEventListener("click", () => order.push("win-bub"));
    sandbox.__nanaFireEvent(7, "click", {});
    assert.deepEqual(order, [
      "win-cap",
      "doc-cap",
      "tgt-cap",
      "tgt-bub",
      "doc-bub",
      "win-bub",
    ]);
  });

  test("does not bubble through intermediate parent nodes (honest subset)", () => {
    const hits = [];
    const parent = sandbox.wrapNode(20, "element", "div");
    const child = sandbox.wrapNode(21, "element", "button");
    parent.addEventListener("click", () => hits.push("parent"));
    child.addEventListener("click", () => hits.push("child"));
    sandbox.__nanaFireEvent(21, "click", {});
    assert.deepEqual(hits, ["child"]);
  });
});
