import { createTestRuntime } from "./load-runtime.mjs";
/**
 * X7 / D-03 Teleport contract: stable mount-root target + Overlay coexistence notes.
 * Transition stays immediate (0s) — do not invent CSS transition durations.
 */
import assert from "node:assert/strict";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { describe, test, beforeEach, afterEach } from "node:test";
import { register } from "node:module";
import {
  NANA_TRANSITION_COMPUTED_DEFAULTS,
  isNanaTeleportMountSelector,
  transitionInfoLooksImmediate,
} from "../src/transitionContract.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const stubUrl = pathToFileURL(join(root, "tests/vue-runtime-core-stub.mjs")).href;
const hooksUrl = pathToFileURL(join(root, "tests/vue-stub-hooks.mjs")).href;
register(hooksUrl, import.meta.url, { data: { stub: stubUrl } });
const modUrl = pathToFileURL(join(root, "src/createNanaRenderer.js")).href;

test("transition defaults stay immediate (no fake CSS duration)", () => {
  assert.equal(NANA_TRANSITION_COMPUTED_DEFAULTS.transitionDuration, "0s");
  assert.ok(transitionInfoLooksImmediate(NANA_TRANSITION_COMPUTED_DEFAULTS));
  assert.equal(isNanaTeleportMountSelector("body"), true);
  assert.equal(isNanaTeleportMountSelector(" html "), true);
  assert.equal(isNanaTeleportMountSelector("#app"), false);
});

test("early document target resolves to the same object after renderer install", async () => {
  const { sandbox, api } = await createTestRuntime();
  const body = sandbox.document.body;
  assert.equal(sandbox.document.querySelector("body"), body);
  assert.equal(api.hostOps.querySelector("body"), body);
  assert.equal(api.wrapNode(body.__nid, "element", "body"), body);
});

describe("Teleport target identity", () => {
  const calls = [];
  let bodyId = 2;
  let htmlId = 1;

  beforeEach(async () => {
    calls.length = 0;
    bodyId = 2;
    htmlId = 1;
    globalThis.__nanaHost = {
      call(name, args) {
        calls.push([name, args]);
        if (name === "querySelector") {
          const sel = String(args?.[0] ?? "")
            .trim()
            .toLowerCase();
          if (sel === "body") return bodyId;
          if (sel === "html") return htmlId;
          return null;
        }
        if (name === "querySelectorAll") {
          const sel = String(args?.[0] ?? "")
            .trim()
            .toLowerCase();
          if (sel === "body") return [bodyId];
          if (sel === "html") return [htmlId];
          return [];
        }
        if (name === "mountRoot") return bodyId;
        if (name === "nodeKind") return "element";
        if (name === "elementTag") {
          const id = Number(args?.[0]);
          if (id === bodyId) return "body";
          if (id === htmlId) return "html";
          return "div";
        }
        if (name === "parentNode" || name === "nextSibling" || name === "firstChild") {
          return null;
        }
        if (name === "childNodes") return [];
        if (name === "contains") return false;
        return null;
      },
    };
  });

  afterEach(() => {
    delete globalThis.__nanaHost;
    delete globalThis.__nanaWrapNode;
  });

  test("querySelector(body) is the stable mount-root wrapNode", async () => {
    const mod = await import(`${modUrl}?teleport=${Date.now()}`);
    const { hostOps, wrapNode } = mod;
    const viaQs = hostOps.querySelector("body");
    const viaWrap = wrapNode(bodyId, "element", "body");
    assert.equal(viaQs, viaWrap);
    assert.equal(viaQs.tag, "body");
    assert.equal(viaQs.__nid, bodyId);
  });

  test("repeated Teleport target lookups keep object identity", async () => {
    const mod = await import(`${modUrl}?teleport-id=${Date.now()}`);
    const { hostOps } = mod;
    const a = hostOps.querySelector("body");
    const b = hostOps.querySelector("body");
    const c = hostOps.querySelectorAll("body")[0];
    assert.equal(a, b);
    assert.equal(a, c);
  });
});
