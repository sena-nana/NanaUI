import assert from "node:assert/strict";
import { describe, test } from "node:test";
import {
  uniformWindow,
  variableWindow,
  virtualWindow,
  createWindowIndex,
} from "../src/virtual-window.js";

describe("virtual window geometry", () => {
  test("scrolling and resizing query cached variable sizes without reading the rows", () => {
    let reads = 0;
    const extents = new Proxy(Array(100_000).fill(32), {
      get(target, key, receiver) {
        if (/^\d+$/.test(String(key))) reads += 1;
        return Reflect.get(target, key, receiver);
      },
    });
    const index = createWindowIndex({ extents });
    reads = 0;
    for (const scroll of [0, 32, 32_000, 3_000_000]) {
      for (const viewport of [600, 900]) {
        assert.deepEqual(index.window(scroll, viewport, 64),
          uniformWindow(100_000, 32, scroll, viewport, 64));
      }
    }
    assert.equal(reads, 0);
  });

  test("rebuilding after size edits and insertion updates spacers and total", () => {
    const extents = [10, 20, 30];
    const before = createWindowIndex({ extents });
    extents[0] = 40;
    extents.push(50);
    const after = createWindowIndex({ extents });
    assert.equal(before.window(0, 10, 0).total, 60);
    assert.deepEqual(after.window(45, 25, 0), {
      start: 1, end: 3, leading: 40, trailing: 50, total: 140,
    });
    assert.deepEqual(createWindowIndex({ count: 100, itemExtent: 20 }).window(50, 80, 10),
      uniformWindow(100, 20, 50, 80, 10));
  });

  test("matches VirtualListLayout window with overscan and spacers", () => {
    const window = variableWindow([10, 20, 30, 40, 50], 35, 35, 10);
    assert.deepEqual(window, {
      start: 1,
      end: 4,
      leading: 10,
      trailing: 50,
      total: 150,
    });
  });

  test("clamps invalid extents and keeps one item visible", () => {
    const window = variableWindow([Number.NaN, -5, 24], Number.POSITIVE_INFINITY, 0, 0);
    assert.equal(window.total, 24);
    assert.equal(window.start, 2);
    assert.equal(window.end, 3);
  });

  test("uniform window is O(1) and matches prefix geometry", () => {
    const window = uniformWindow(10_000, 20, 0, 100, 20);
    assert.equal(window.start, 0);
    assert.equal(window.end, 6);
    assert.equal(window.leading, 0);
    assert.equal(window.total, 200_000);
    assert.equal(window.trailing, 200_000 - 120);
  });

  test("virtualWindow prefers extents over uniform count", () => {
    const window = virtualWindow({
      count: 100,
      itemExtent: 10,
      extents: [10, 20, 30, 40, 50],
      scroll: 35,
      viewport: 35,
      overscan: 10,
    });
    assert.equal(window.start, 1);
    assert.equal(window.end, 4);
  });
});
