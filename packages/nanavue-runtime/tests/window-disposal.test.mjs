/**
 * Behavior: the disposal guard drops document work aimed at a dying window,
 * but never a release. Host handles (sockets, requests, decoded resources) are
 * process-wide, so a swallowed release leaks for the life of the process.
 */
import assert from "node:assert/strict";
import vm from "node:vm";
import { test } from "node:test";
import { loadShimSource } from "./load-runtime.mjs";
import {
  hostCall,
  isNanaHostCallSuppressed,
  withNanaWindowDisposal,
} from "../src/layoutMetrics.js";

test("a disposing window drops document operations and still releases handles", () => {
  const previous = globalThis.__nanaHost;
  const calls = [];
  globalThis.__nanaHost = { call(name, args) { calls.push([name, args]); return null; } };
  try {
    withNanaWindowDisposal(9, () => {
      hostCall("setText", [1, "gone"]);
      hostCall("resourceRelease", [3]);
      hostCall("windowCall", [9, "mediaRelease", [4]]);
    });
    assert.deepEqual(calls.map(([, args]) => args[1]), ["resourceRelease", "mediaRelease"]);
  } finally { globalThis.__nanaHost = previous; }
});

test("the web-api shim routes its host calls through the same predicate", () => {
  const calls = [];
  const sandbox = { console, queueMicrotask, setTimeout, clearTimeout, Promise };
  sandbox.globalThis = sandbox;
  sandbox.__nanaActiveWindowId = 9;
  sandbox.__nanaIsHostCallSuppressed = isNanaHostCallSuppressed;
  sandbox.__nanaHost = {
    call(name, args) { calls.push([name, args]); return args?.[1] === "wsOpen" ? 7 : null; },
  };
  vm.runInNewContext(loadShimSource(), sandbox);

  const socket = new sandbox.WebSocket("ws://127.0.0.1:1/echo");
  socket.readyState = sandbox.WebSocket.OPEN;
  calls.length = 0;
  withNanaWindowDisposal(9, () => {
    socket.send("dropped with the document");
    socket.close();
  });
  assert.deepEqual(calls.map(([, args]) => args[1]), ["wsClose"]);
});
