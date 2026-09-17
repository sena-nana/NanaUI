/**
 * Behavior: the Web Audio shim keeps the host mixer fed and forwards the
 * mixer-owned `loop` flag. Both live in the shim, so the shim runs for real in
 * a sandbox against a recording host and a driven clock.
 */
import assert from "node:assert/strict";
import vm from "node:vm";
import { test } from "node:test";
import { loadShimSource } from "./load-runtime.mjs";

const SAMPLE_RATE = 48_000;

function audioSandbox() {
  const calls = [];
  const due = new Map();
  let now = 0;
  let nextId = 10;
  const sandbox = { console, queueMicrotask, Promise, setTimeout, clearTimeout };
  sandbox.globalThis = sandbox;
  sandbox.__nanaHost = {
    call(name, args) {
      calls.push([name, ...(args || [])]);
      if (name === "intervalSchedule") due.set(Number(args[0]), now + Number(args[1]));
      if (name === "intervalCancel") due.delete(Number(args[0]));
      if (name === "audioContextCreate") {
        return {
          id: 1,
          sampleRate: SAMPLE_RATE,
          state: "running",
          destination: { id: 2, context: 1 },
        };
      }
      if (name === "audioContextCurrentTime") return now / 1000;
      if (name.startsWith("audio") && name.endsWith("Create")) {
        return { id: nextId++, context: 1 };
      }
      return null;
    },
  };
  sandbox.__nanaTestNow = () => now;
  const context = vm.createContext(sandbox);
  vm.runInContext(loadShimSource(), context);
  // The pump paces itself off the wall clock, so the test owns that clock.
  vm.runInContext("Date.now = globalThis.__nanaTestNow;", context);
  return {
    sandbox,
    calls,
    /** Drives the host's interval wakeups until `ms` of clock has passed. */
    advance(ms) {
      const until = now + ms;
      for (;;) {
        let next = null;
        for (const [id, at] of due) if (next === null || at < next.at) next = { id, at };
        if (next === null || next.at > until) {
          now = until;
          return;
        }
        now = next.at;
        due.delete(next.id);
        sandbox.__nanaDrainTimers({ intervals: [next.id], now });
      }
    },
  };
}

test("assigning loop tells the mixer, which is the only place it is honoured", () => {
  const { sandbox, calls } = audioSandbox();
  const context = new sandbox.AudioContext();
  const source = context.createBufferSource();
  calls.length = 0;

  source.loop = true;
  assert.equal(source.loop, true);
  assert.deepEqual(calls, [["audioBufferSourceSetLoop", source.id, true]]);

  source.loop = false;
  assert.deepEqual(calls[1], ["audioBufferSourceSetLoop", source.id, false]);
});

test("the script processor pump submits a second of audio per second of clock", () => {
  const { sandbox, calls, advance } = audioSandbox();
  const context = new sandbox.AudioContext();
  const processor = context.createScriptProcessor(256, 1, 1);
  processor.onaudioprocess = (event) => {
    event.outputBuffer.getChannelData(0).fill(0.5);
  };
  calls.length = 0;

  advance(1000);

  const submitted = calls
    .filter(([name]) => name === "audioScriptProcessorSubmit")
    .reduce((total, [, , pcm]) => total + pcm.length, 0);
  // A short lead is expected; falling behind is what starves the mixer.
  assert.ok(
    submitted >= SAMPLE_RATE && submitted <= SAMPLE_RATE + 256 * 4,
    `one second of wall clock must produce about ${SAMPLE_RATE} frames, got ${submitted}`,
  );
});
