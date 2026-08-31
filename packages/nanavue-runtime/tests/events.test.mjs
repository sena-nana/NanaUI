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
const transitionContractSrc = readFileSync(
  join(root, "src/transitionContract.js"),
  "utf8",
);

function stripEsm(src) {
  return src
    .replace(/^import\s+[^;]+;?\s*$/gm, "")
    .replace(/^export\s+\{[^}]+\}\s+from\s+[^;]+;?\s*$/gm, "")
    .replace(/^export\s+\{[^}]+\};?\s*$/gm, "")
    .replace(/^export\s+(default\s+)?/gm, "");
}

function loadRuntime() {
  const hostListeners = new Map();
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
    Proxy,
    setTimeout,
    clearTimeout,
  };
  sandbox.globalThis = sandbox;
  sandbox.__hostListeners = hostListeners;
  sandbox.Nana = {
    host: {
      on(name, listener) {
        hostListeners.set(name, listener);
        return () => hostListeners.delete(name);
      },
    },
  };
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
      ${stripEsm(transitionContractSrc)}
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

  test("native component render failures reach local and global Vue listeners", () => {
    const local = [];
    const global = [];
    const el = sandbox.wrapNode(19, "element", "nana-live2d-view");
    el.addEventListener("error", (event) => local.push(event.error));
    sandbox.Nana.components.onError((error) => global.push(error));

    sandbox.__hostListeners.get("native-component-error")({
      windowId: 0,
      id: 19,
      component: "live2d-view",
      error: {
        name: "NativeComponentRenderError",
        code: "render_failed",
        message: "draw failed",
        details: { frame: 7 },
      },
    });

    assert.equal(local.length, 1);
    assert.equal(global.length, 1);
    assert.equal(local[0], global[0]);
    assert.equal(global[0].component, "live2d-view");
    assert.equal(global[0].code, "render_failed");
    assert.equal(global[0].details.frame, 7);
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

  test("bubbles through intermediate parent nodes", () => {
    const hits = [];
    const parent = sandbox.wrapNode(20, "element", "div");
    const child = sandbox.wrapNode(21, "element", "button");
    sandbox.__nanaHost.call = (name, args) => {
      if (name === "parentNode" && args[0] === 21) return 20;
      return null;
    };
    parent.addEventListener("click", () => hits.push("parent"));
    child.addEventListener("click", () => hits.push("child"));
    sandbox.__nanaFireEvent(21, "click", {});
    assert.deepEqual(hits, ["child", "parent"]);
  });

  test("ancestor capture and bubble order matches DOM propagation", () => {
    const order = [];
    const parent = sandbox.wrapNode(50, "element", "div");
    const child = sandbox.wrapNode(51, "element", "button");
    sandbox.__nanaHost.call = (name, args) => {
      if (name === "parentNode" && args[0] === 51) return 50;
      return null;
    };
    sandbox.window.addEventListener("pointerdown", () => order.push("window-capture"), true);
    sandbox.document.addEventListener("pointerdown", () => order.push("document-capture"), true);
    parent.addEventListener("pointerdown", () => order.push("parent-capture"), true);
    child.addEventListener("pointerdown", () => order.push("target"));
    parent.addEventListener("pointerdown", () => order.push("parent-bubble"));
    sandbox.document.addEventListener("pointerdown", () => order.push("document-bubble"));
    sandbox.window.addEventListener("pointerdown", () => order.push("window-bubble"));
    sandbox.__nanaFireEvent(51, "pointerdown", {});
    assert.deepEqual(order, [
      "window-capture",
      "document-capture",
      "parent-capture",
      "target",
      "parent-bubble",
      "document-bubble",
      "window-bubble",
    ]);
  });

  test("pointer, keyboard, composition and wheel fields reach listeners", () => {
    const el = sandbox.wrapNode(30, "element", "input");
    const received = [];
    for (const type of ["pointermove", "keydown", "compositionupdate", "wheel"]) {
      el.addEventListener(type, (event) => received.push(event));
    }

    sandbox.__nanaFireEvent(30, "pointermove", {
      clientX: 12.5,
      clientY: 24.5,
      screenX: 112,
      screenY: 224,
      button: -1,
      buttons: 3,
      pressure: 0.75,
      tangentialPressure: -0.2,
      tiltX: -35,
      tiltY: 20,
      twist: 180,
      pointerId: 7,
      pointerType: "pen",
      isPrimary: true,
      altKey: true,
      shiftKey: true,
    });
    sandbox.__nanaFireEvent(30, "keydown", {
      key: "A",
      code: "KeyA",
      repeat: true,
      location: 2,
      isComposing: true,
      ctrlKey: true,
    });
    sandbox.__nanaFireEvent(30, "compositionupdate", {
      data: "拼",
      isComposing: true,
    });
    sandbox.__nanaFireEvent(30, "wheel", {
      deltaX: -4,
      deltaY: 18,
      deltaMode: 1,
      metaKey: true,
    });

    assert.equal(received.length, 4);
    assert.equal(received[0].clientX, 12.5);
    assert.equal(received[0].screenY, 224);
    assert.equal(received[0].button, -1);
    assert.equal(received[0].buttons, 3);
    assert.equal(received[0].pressure, 0.75);
    assert.equal(received[0].tangentialPressure, -0.2);
    assert.equal(received[0].tiltX, -35);
    assert.equal(received[0].tiltY, 20);
    assert.equal(received[0].twist, 180);
    assert.equal(received[0].pointerId, 7);
    assert.equal(received[0].pointerType, "pen");
    assert.equal(received[0].isPrimary, true);
    assert.equal(received[0].altKey, true);
    assert.equal(received[0].shiftKey, true);
    assert.equal(received[1].key, "A");
    assert.equal(received[1].code, "KeyA");
    assert.equal(received[1].repeat, true);
    assert.equal(received[1].location, 2);
    assert.equal(received[1].isComposing, true);
    assert.equal(received[1].ctrlKey, true);
    assert.equal(received[2].data, "拼");
    assert.equal(received[2].isComposing, true);
    assert.equal(received[3].deltaX, -4);
    assert.equal(received[3].deltaY, 18);
    assert.equal(received[3].deltaMode, 1);
    assert.equal(received[3].metaKey, true);
  });

  test("native file drops expose FileList-like dataTransfer descriptors", () => {
    const el = sandbox.wrapNode(31, "element", "div");
    let received;
    el.addEventListener("drop", (event) => {
      received = event;
    });

    sandbox.__nanaFireEvent(31, "drop", {
      clientX: 18,
      clientY: 27,
      files: [
        {
          name: "avatar.png",
          path: "C:/drop/avatar.png",
          size: 2048,
          type: "image/png",
          lastModified: 1234,
        },
        {
          name: "background.jpg",
          path: "C:/drop/background.jpg",
          size: 4096,
          type: "image/jpeg",
          lastModified: 5678,
        },
      ],
    });

    assert.equal(received.clientX, 18);
    assert.equal(received.dataTransfer.types[0], "Files");
    assert.equal(received.dataTransfer.dropEffect, "copy");
    assert.equal(received.dataTransfer.files.length, 2);
    assert.equal(received.dataTransfer.files.item(0).path, "C:/drop/avatar.png");
    assert.equal(received.dataTransfer.files.item(1).path, "C:/drop/background.jpg");
    assert.equal(received.dataTransfer.items.item(0).kind, "file");
    assert.equal(received.dataTransfer.items.item(0).getAsFile().name, "avatar.png");
    assert.equal(received.dataTransfer.items.item(1).getAsFile().name, "background.jpg");
  });

  test("host nodes expose pointer capture through Nana host operations", () => {
    const calls = [];
    sandbox.__nanaHost.call = (name, args) => {
      calls.push([name, Array.from(args)]);
      return name !== "setPointerCapture";
    };
    const el = sandbox.wrapNode(41, "element", "div");

    assert.equal(el.setPointerCapture(9), undefined);
    assert.equal(el.hasPointerCapture(9), true);
    assert.equal(el.releasePointerCapture(9), true);
    assert.deepEqual(calls, [
      ["setPointerCapture", [41, 9]],
      ["hasPointerCapture", [41, 9]],
      ["releasePointerCapture", [41, 9]],
    ]);
  });

  test("Nana.windows creates a scoped document and routes its node operations", async () => {
    const calls = [];
    const rootId = 4294967296 + 2;
    sandbox.__nanaHost.call = (name, args) => {
      calls.push([name, Array.from(args || [])]);
      if (name === "windowCall") {
        const [, operation] = args;
        if (operation === "mountRoot" || operation === "querySelector") return rootId;
        if (operation === "nodeKind") return "element";
        if (operation === "elementTag") return "body";
      }
      return null;
    };
    sandbox.Nana.host = {
      invoke(name, args) {
        assert.equal(name, "windowCreate");
        assert.equal(args[0].title, "工具");
        return Promise.resolve({ id: 1, mountRoot: rootId, width: 420, height: 300, ready: true });
      },
    };

    const handle = await sandbox.Nana.windows.create({ title: "工具" });
    assert.equal(handle.id, 1);
    assert.equal(handle.root.__nid, rootId);
    assert.notEqual(handle.document, sandbox.document);
    assert.equal(handle.root.ownerDocument, handle.document);

    handle.root.setAttribute("data-ready", "true");
    assert.ok(
      calls.some(
        ([name, args]) =>
          name === "windowCall" &&
          args[0] === 1 &&
          args[1] === "patchProp" &&
          args[2][0] === rootId,
      ),
    );
    assert.equal(handle.document.querySelector("body").__nid, rootId);
    assert.ok(
      calls.some(
        ([name, args]) =>
          name === "windowCall" && args[0] === 1 && args[1] === "querySelector",
      ),
    );

    const closed = handle.closed;
    sandbox.__hostListeners.get("window-closed")({ id: 1 });
    assert.equal((await closed).reason, "closed");
    assert.equal(sandbox.Nana.windows.get(1), null);
  });

  test("window.open without url maps onto Nana.windows.create", async () => {
    const sandbox = loadRuntime();
    const rootId = 4294967296 + 2;
    sandbox.__nanaHost.call = (name, args) => {
      if (name === "windowCall") {
        const [, operation] = args;
        if (operation === "mountRoot" || operation === "querySelector") return rootId;
        if (operation === "nodeKind") return "element";
        if (operation === "elementTag") return "body";
      }
      return null;
    };
    sandbox.Nana.host.invoke = async (name, args) => {
      assert.equal(name, "windowCreate");
      assert.equal(args[0].title, "工具");
      assert.equal(args[0].width, 420);
      assert.equal(args[0].height, 300);
      return { id: 1, mountRoot: rootId, width: 420, height: 300, ready: true };
    };
    const handle = await sandbox.window.open(null, "工具", "width=420,height=300");
    assert.equal(handle.id, 1);
    assert.throws(
      () => sandbox.window.open("https://example.com"),
      /window.open\(url\)/,
    );
  });

  test("closing a window clears scoped observers and window listeners", () => {
    const context = sandbox.__nanaCreateWindowContext(2, 320, 240, 1);
    let eventCount = 0;
    context.window.addEventListener("resize", () => eventCount++);
    sandbox.__nanaWithWindowContext(2, () => {
      const observer = new sandbox.ResizeObserver(() => {});
      observer.observe({ getBoundingClientRect: () => ({ width: 10, height: 10 }) });
    });
    assert.equal(sandbox.__nanaNotifyLayout(), 1);

    assert.equal(sandbox.__nanaDestroyWindowContext(2), true);
    assert.equal(sandbox.__nanaNotifyLayout(), 0);
    context.window.dispatchEvent(new sandbox.Event("resize"));
    assert.equal(eventCount, 0);
  });
});
