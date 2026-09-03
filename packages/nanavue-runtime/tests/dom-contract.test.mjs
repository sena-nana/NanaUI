import { loadShimSource, loadRenderer } from "./load-runtime.mjs";
/**
 * Track B DOM contract: wrapNode identity cache + live tree navigation.
 * Mirrors LiliaUI useAnchoredOverlay.containsTarget (instanceof Node + contains).
 */
import assert from "node:assert/strict";
import vm from "node:vm";
import { describe, test, beforeEach, afterEach } from "node:test";

const shimSrc = loadShimSource();

/** In-memory host tree for wrapNode getters. */
function makeTreeHost() {
  let next = 1;
  const nodes = new Map();

  function alloc(kind, tag) {
    const id = next++;
    nodes.set(id, { kind, tag: tag || null, parent: null, children: [] });
    return id;
  }

  const html = alloc("element", "html");
  const body = alloc("element", "body");
  nodes.get(html).children.push(body);
  nodes.get(body).parent = html;
  const calls = [];

  return {
    html,
    body,
    alloc,
    calls,
    insert(child, parent, anchor) {
      const c = nodes.get(child);
      const p = nodes.get(parent);
      if (!c || !p) return;
      if (c.parent != null) {
        const prev = nodes.get(c.parent);
        if (prev) prev.children = prev.children.filter((x) => x !== child);
      }
      c.parent = parent;
      const at = anchor != null ? p.children.indexOf(anchor) : -1;
      if (at >= 0) p.children.splice(at, 0, child);
      else if (!p.children.includes(child)) p.children.push(child);
    },
    contains(a, b) {
      if (!nodes.has(a) || !nodes.has(b)) return false;
      let cur = b;
      while (cur != null) {
        if (cur === a) return true;
        cur = nodes.get(cur)?.parent ?? null;
      }
      return false;
    },
    call(name, args) {
      calls.push([name, args]);
      const a0 = args?.[0];
      const a1 = args?.[1];
      switch (name) {
        case "parentNode":
          return nodes.get(Number(a0))?.parent ?? null;
        case "firstChild": {
          const kids = nodes.get(Number(a0))?.children || [];
          return kids.length ? kids[0] : null;
        }
        case "childNodes":
          return [...(nodes.get(Number(a0))?.children || [])];
        case "nodeKind":
          return nodes.get(Number(a0))?.kind ?? "other";
        case "elementTag":
          return nodes.get(Number(a0))?.tag ?? null;
        case "contains":
          return this.contains(Number(a0), Number(a1));
        case "querySelector":
          if (String(a0) === "html") return html;
          if (String(a0) === "body") return body;
          return null;
        case "layoutBox":
          return { x: 0, y: 0, width: 0, height: 0 };
        case "insert":
          this.insert(Number(a0), Number(a1), a1 == null ? null : Number(args?.[2]));
          return null;
        case "remove": {
          const child = Number(a0);
          const c = nodes.get(child);
          if (!c) return null;
          if (c.parent != null) {
            const p = nodes.get(c.parent);
            if (p) p.children = p.children.filter((x) => x !== child);
          }
          c.parent = null;
          return null;
        }
        case "createElement":
          return alloc("element", String(a0 || "div"));
        case "createText":
          return alloc("text", null);
        case "mountRoot":
          return body;
        default:
          return null;
      }
    },
  };
}

async function loadRuntime(host) {
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
    Boolean,
    TypeError,
    Error,
    JSON,
    Date,
    Proxy,
    Reflect,
  };
  sandbox.globalThis = sandbox;
  sandbox.__nanaHost = host;

  // Install Node/HTMLElement constructors first.
  vm.runInNewContext(shimSrc, sandbox, { filename: "shim.js" });

  sandbox.__exports = await loadRenderer(sandbox);
  assert.equal(typeof sandbox.__exports.wrapNode, "function");
  return {
    wrapNode: sandbox.__exports.wrapNode,
    nodeId: sandbox.__exports.nodeId,
    Node: sandbox.Node,
    HTMLElement: sandbox.HTMLElement,
    sandbox,
  };
}

describe("shim Node instanceof for click-outside", () => {
  test("Node is a constructor; HTMLElement instances are instanceof Node", () => {
    const sandbox = {
      console,
      Object,
      Array,
      Number,
      String,
      Math,
      TypeError,
      Map,
      Set,
      Promise,
      JSON,
      Date,
    };
    sandbox.globalThis = sandbox;
    sandbox.__nanaHost = { call() { return null; } };
    vm.runInNewContext(shimSrc, sandbox, { filename: "shim.js" });
    assert.equal(typeof sandbox.Node, "function");
    assert.equal(sandbox.Node.ELEMENT_NODE, 1);
    const el = Object.create(sandbox.HTMLElement.prototype);
    el.__nid = 1;
    assert.ok(el instanceof sandbox.Node);
    assert.ok(el instanceof sandbox.HTMLElement);
  });
});

test("cache preserves identity before and after host lookup", async () => {
  const host = makeTreeHost();
  const { wrapNode } = await loadRuntime(host);
  const body = wrapNode(host.body, "element", "body");
  assert.equal(body.parentElement.firstChild, body);
  assert.equal(wrapNode(host.body, "element", null), body);
  assert.equal(body.isConnected, true);
});

describe("wrapNode DOM contract", () => {
  let host;
  let wrapNode;
  let Node;

  beforeEach(async () => {
    host = makeTreeHost();
    ({ wrapNode, Node } = await loadRuntime(host));
  });

  test("same nid returns the same object", () => {
    const a = wrapNode(host.body, "element", "body");
    const b = wrapNode(host.body, "element", null);
    assert.equal(a, b);
    assert.equal(a.__nid, host.body);
  });

  test("firstChild / childNodes / parentElement follow host tree", () => {
    const outer = host.alloc("element", "div");
    host.insert(outer, host.body, null);
    const text = host.alloc("text", null);
    host.insert(text, outer, null);
    const inner = host.alloc("element", "span");
    host.insert(inner, outer, null);

    const outerNode = wrapNode(outer, "element", "div");
    const textNode = wrapNode(text, "text", null);
    const innerNode = wrapNode(inner, "element", "span");

    assert.equal(outerNode.firstChild, textNode);
    assert.deepEqual(
      outerNode.childNodes.map((n) => n.__nid),
      [text, inner],
    );
    assert.equal(innerNode.parentElement, outerNode);
    assert.equal(textNode.parentElement, outerNode);
    assert.equal(outerNode.parentElement, wrapNode(host.body, "element", "body"));
    assert.ok(outerNode.contains(innerNode));
    assert.ok(outerNode.contains(textNode));
    assert.equal(outerNode.contains(wrapNode(host.body, "element", "body")), false);
  });

  test("useAnchoredOverlay containsTarget pattern works", () => {
    const overlay = host.alloc("element", "div");
    const anchor = host.alloc("element", "button");
    const inside = host.alloc("element", "span");
    const outside = host.alloc("element", "div");
    host.insert(overlay, host.body, null);
    host.insert(anchor, host.body, null);
    host.insert(inside, overlay, null);
    host.insert(outside, host.body, null);

    const overlayEl = wrapNode(overlay, "element", "div");
    const anchorEl = wrapNode(anchor, "element", "button");
    const insideEl = wrapNode(inside, "element", "span");
    const outsideEl = wrapNode(outside, "element", "div");

    function containsTarget(target) {
      const node = target instanceof Node ? target : null;
      if (!node) return false;
      return Boolean(overlayEl.contains(node) || anchorEl.contains(node));
    }

    assert.ok(insideEl instanceof Node);
    assert.equal(containsTarget(insideEl), true);
    assert.equal(containsTarget(overlayEl), true);
    assert.equal(containsTarget(anchorEl), true);
    assert.equal(containsTarget(outsideEl), false);
    assert.equal(containsTarget(wrapNode(inside, "element", null)), true);
    assert.equal(wrapNode(inside, "element", null), insideEl);
  });

  test("isConnected walks cached parent chain to html without querySelector", () => {
    const outer = host.alloc("element", "div");
    host.insert(outer, host.body, null);
    const inner = host.alloc("element", "span");
    host.insert(inner, outer, null);
    const stray = host.alloc("element", "div");

    const callsBefore = host.calls ? host.calls.length : 0;
    const innerEl = wrapNode(inner, "element", "span");
    const strayEl = wrapNode(stray, "element", "div");
    assert.equal(innerEl.isConnected, true);
    assert.equal(wrapNode(host.body, "element", "body").isConnected, true);
    assert.equal(wrapNode(host.html, "element", "html").isConnected, true);
    assert.equal(strayEl.isConnected, false);

    const names = (host.calls || []).slice(callsBefore).map((c) => c[0]);
    assert.equal(names.includes("querySelector"), false);
  });
});
