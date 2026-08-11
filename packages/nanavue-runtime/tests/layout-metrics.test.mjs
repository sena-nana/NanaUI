/**
 * Behavior: layout metrics project host layoutBox (wrapNode helper + Element shim).
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { describe, test, beforeEach, afterEach } from "node:test";
import {
  defineLayoutMetrics,
  layoutSizePx,
  normalizeScrollIntoViewArg,
  scrollNodeIntoView,
} from "../src/layoutMetrics.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const shimSrc = readFileSync(
  join(root, "../../crates/nana-ui-web-api/src/shim.js"),
  "utf8",
);
const rendererSrc = readFileSync(join(root, "src/createNanaRenderer.js"), "utf8");

const boxes = new Map();
const scrolls = new Map();
const scrollIntoViewCalls = [];

function installMockHost() {
  globalThis.__nanaHost = {
    call(name, args) {
      if (name === "layoutBox") {
        const nid = Number(args?.[0]);
        const b = boxes.get(nid);
        if (!b) {
          return {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            top: 0,
            left: 0,
            bottom: 0,
            right: 0,
          };
        }
        return {
          x: b.x,
          y: b.y,
          width: b.width,
          height: b.height,
          top: b.y,
          left: b.x,
          bottom: b.y + b.height,
          right: b.x + b.width,
        };
      }
      if (name === "getScrollOffset") {
        const nid = Number(args?.[0]);
        const off = scrolls.get(nid) || { x: 0, y: 0 };
        return {
          x: off.x,
          y: off.y,
          scrollLeft: off.x,
          scrollTop: off.y,
        };
      }
      if (name === "setScrollOffset") {
        const nid = Number(args?.[0]);
        const x = Number(args?.[1]) || 0;
        const y = Number(args?.[2]) || 0;
        scrolls.set(nid, { x, y });
        return { x, y, scrollLeft: x, scrollTop: y };
      }
      if (name === "scrollIntoView") {
        scrollIntoViewCalls.push({ nid: Number(args?.[0]), opts: args?.[1] });
        return { scrolled: [] };
      }
      return null;
    },
  };
}

describe("layoutMetrics from layoutBox", () => {
  let prevHost;

  beforeEach(() => {
    prevHost = globalThis.__nanaHost;
    boxes.clear();
    scrolls.clear();
    scrollIntoViewCalls.length = 0;
    installMockHost();
  });

  afterEach(() => {
    globalThis.__nanaHost = prevHost;
  });

  test("layoutSizePx rounds box; missing → 0", () => {
    boxes.set(7, { x: 10.4, y: 20.6, width: 120.4, height: 48.6 });
    assert.deepEqual(layoutSizePx(7), { width: 120, height: 49 });
    assert.deepEqual(layoutSizePx(99), { width: 0, height: 0 });
  });

  test("defineLayoutMetrics installs finite readables + host scroll*", () => {
    boxes.set(7, { x: 0, y: 0, width: 120.4, height: 48.6 });
    const node = { __nid: 7 };
    defineLayoutMetrics(node, 7);
    assert.equal(node.offsetWidth, 120);
    assert.equal(node.offsetHeight, 49);
    assert.equal(node.clientWidth, 120);
    assert.equal(node.clientHeight, 49);
    assert.equal(node.scrollWidth, 120);
    assert.equal(node.scrollHeight, 49);
    assert.equal(typeof node.offsetWidth, "number");
    assert.ok(!Number.isNaN(node.offsetWidth));
    assert.equal(node.scrollTop, 0);
    node.scrollTop = 40;
    node.scrollLeft = "12";
    assert.equal(node.scrollTop, 40);
    assert.equal(node.scrollLeft, 12);
    assert.deepEqual(scrolls.get(7), { x: 12, y: 40 });
    node.scrollTop = "nope";
    assert.equal(node.scrollTop, 0);

    const empty = { __nid: 99 };
    defineLayoutMetrics(empty, 99);
    assert.equal(empty.offsetWidth, 0);
    assert.equal(empty.clientHeight, 0);
  });

  test("scrollNodeIntoView forwards host options", () => {
    scrollNodeIntoView(11, { block: "center", inline: "nearest" });
    assert.equal(scrollIntoViewCalls.length, 1);
    assert.equal(scrollIntoViewCalls[0].nid, 11);
    assert.equal(scrollIntoViewCalls[0].opts.block, "center");
    assert.deepEqual(normalizeScrollIntoViewArg(false), {
      block: "end",
      inline: "nearest",
    });
  });

  test("wrapNode wires defineLayoutMetrics + scrollIntoView", () => {
    assert.match(rendererSrc, /defineLayoutMetrics\(node,\s*nid\)/);
    assert.match(rendererSrc, /from\s+["']\.\/layoutMetrics\.js["']/);
    assert.match(rendererSrc, /scrollNodeIntoView\(nid,\s*arg\)/);
  });
});

describe("shim Element.prototype layout metrics", () => {
  test("Element with __nid reads layoutBox; detached → 0", () => {
    boxes.clear();
    scrolls.clear();
    boxes.set(3, { x: 0, y: 0, width: 220, height: 80 });

    const sandbox = {
      console,
      queueMicrotask: (fn) => Promise.resolve().then(fn),
      Promise,
      Map,
      Set,
      Object,
      Array,
      Number,
      String,
      Math,
      JSON,
      Error,
      TypeError,
    };
    sandbox.globalThis = sandbox;
    sandbox.window = sandbox;
    sandbox.self = sandbox;
    sandbox.__nanaHost = {
      call(name, args) {
        if (name === "layoutBox") {
          const nid = Number(args?.[0]);
          const b = boxes.get(nid);
          if (!b) {
            return { x: 0, y: 0, width: 0, height: 0 };
          }
          return {
            x: b.x,
            y: b.y,
            width: b.width,
            height: b.height,
          };
        }
        if (name === "getScrollOffset") {
          const nid = Number(args?.[0]);
          const off = scrolls.get(nid) || { x: 0, y: 0 };
          return {
            x: off.x,
            y: off.y,
            scrollLeft: off.x,
            scrollTop: off.y,
          };
        }
        if (name === "setScrollOffset") {
          const nid = Number(args?.[0]);
          const x = Number(args?.[1]) || 0;
          const y = Number(args?.[2]) || 0;
          scrolls.set(nid, { x, y });
          return { x, y, scrollLeft: x, scrollTop: y };
        }
        if (name === "scrollIntoView") {
          return { scrolled: [{ id: 1, scrollTop: 10 }] };
        }
        return null;
      },
    };

    vm.runInNewContext(shimSrc, sandbox);
    const El = sandbox.Element;
    assert.ok(El && El.prototype);

    const withNid = Object.create(El.prototype);
    withNid.__nid = 3;
    assert.equal(withNid.offsetWidth, 220);
    assert.equal(withNid.clientHeight, 80);
    withNid.scrollTop = 55;
    assert.equal(withNid.scrollTop, 55);
    assert.equal(typeof withNid.scrollIntoView, "function");
    withNid.scrollIntoView({ block: "start" });

    const detached = Object.create(El.prototype);
    assert.equal(detached.offsetWidth, 0);
    assert.equal(detached.clientHeight, 0);
  });
});
