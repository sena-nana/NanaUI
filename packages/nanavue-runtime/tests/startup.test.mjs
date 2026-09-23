/**
 * Behavior: `Nana.startup` is a projection of the host's one startup record.
 * It calls host-global APIs (never through a window), reads the state on
 * demand, and relays the host's `startup` event to listeners.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import { createTestRuntime } from "./load-runtime.mjs";

test("Nana.startup reads and requests through host-global calls", async () => {
  const { sandbox, calls } = await createTestRuntime();
  sandbox.__nanaActiveWindowId = 3;
  const host = sandbox.__nanaHost.call;
  sandbox.__nanaHost.call = (name, args) => {
    if (name === "startupStatus") return { phase: "ui-ready", splash: "animated", ticket: 1, timeline: {} };
    if (name === "startupDeferTakeover") return true;
    return host(name, args);
  };
  calls.length = 0;
  assert.equal(sandbox.Nana.startup.state.phase, "ui-ready");
  assert.equal(sandbox.Nana.startup.deferTakeover(), true);
  sandbox.Nana.startup.takeOver();
  sandbox.Nana.startup.takeOver(1);
  sandbox.Nana.startup.cancelTakeover(1);
  // Arrays built in the sandbox realm: compare their shape, not prototypes.
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    ["startupTakeOver", []],
    ["startupTakeOver", [1]],
    ["startupCancelTakeover", [1]],
  ]);
});

test("onChange listeners can be removed", async () => {
  const { sandbox } = await createTestRuntime();
  assert.throws(() => sandbox.Nana.startup.onChange(null), { name: "TypeError" });
  const remove = sandbox.Nana.startup.onChange(() => {});
  assert.equal(remove(), true);
  assert.equal(remove(), false);
});
