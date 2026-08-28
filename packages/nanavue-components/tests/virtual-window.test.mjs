import assert from "node:assert/strict";
import { describe, test } from "node:test";
import {
  uniformWindow,
  variableWindow,
  virtualWindow,
} from "../src/virtual-window.js";

describe("virtual window geometry", () => {
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
