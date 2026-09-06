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

test("million measured items preserve the visible anchor and bounded window", () => {
  const index = createWindowIndex({count: 1_000_000, itemExtent: 20});
  const viewport = {offset: [0, 10_000_003], extent: [0, 400], overscan: [0, 80]};
  const anchor = index.anchor(viewport.offset[1]);
  assert.equal(anchor.index, 500_000);
  assert.equal(index.measure(1, 40, viewport), true);
  assert.equal(viewport.offset[1], 10_000_023);
  assert.deepEqual(index.anchor(viewport.offset[1]), anchor);
  const window = index.windowFor(viewport);
  assert.ok(window.end - window.start <= 30);
});

test("public index queries sanitize fractional positions and nonfinite counts", () => {
  const index = createWindowIndex({extents: [20, 20, 20, 20]});
  assert.equal(index.prefixAt(2.5), 40);
  assert.equal(index.prefixAt(-1), 0);
  assert.equal(index.prefixAt(Number.NaN), 0);
  assert.equal(index.restoreAnchor({index: 2.7, inset: 3}), 43);
  assert.deepEqual(createWindowIndex({count: Infinity, itemExtent: 20}).window(0, 400, 80),
    {start: 0, end: 0, leading: 0, trailing: 0, total: 0});
  const large = uniformWindow(2 ** 32, 1, 100, 20, 0);
  assert.equal(large.start, 100);
  assert.equal(large.end, 120);
});


test("navigation matches Rust nearest-edge and alignment semantics", () => {
  const sizes = createWindowIndex({ extents: [20, 200, 30, 40] });
  assert.equal(sizes.offsetForIndex(1, 50, 60), 50);
  assert.equal(sizes.offsetForIndex(2, 0, 60), 190);
  assert.equal(sizes.offsetForIndex(0, 190, 60), 0);
  assert.equal(sizes.offsetForIndex(3, 0, 60, "start"), 230);
  assert.equal(sizes.offsetForIndex(1, 0, 60, "center"), 90);
  assert.equal(sizes.offsetForIndex(4, 10, 60, "end"), null);
  assert.throws(() => sizes.offsetForIndex(0, 0, 60, "bad"), RangeError);
});

test("a retained editor adds one range without materializing intervening data", () => {
  const sizes = createWindowIndex({count: 1_000_000, itemExtent: 20});
  const win = sizes.window(10_000_000, 100, 0);
  assert.deepEqual(sizes.retainedRanges(win, [2, 2, 500001, 999999, -1, Infinity]), [
    {start: 2, end: 3}, {start: 500000, end: 500005}, {start: 999999, end: 1000000},
  ]);
  assert.deepEqual(sizes.retainedRanges(win), [{start: 500000, end: 500005}]);
});


test("frozen prefix reserves viewport space and remains bounded", () => {
  const index = createWindowIndex({count: 1000000, itemExtent: 20});
  assert.equal(index.offsetForFrozenIndex(500000, 0, 100, 1, "start"), 9999980);
  const pane = index.frozenWindow(9999980, 100, 0, 1);
  assert.deepEqual(pane.frozen, [0]);
  assert.equal(pane.body.start, 500000);
  assert.equal(pane.body.end, 500004);
  assert.equal(index.offsetForFrozenIndex(0, 9999980, 100, 1), 9999980);
  const covered = index.frozenWindow(0, 100, 0, 1000000);
  assert.equal(covered.frozen.length, 5);
  assert.equal(index.offsetForFrozenIndex(999999, 0, 100, 1000000), null);
  assert.equal(covered.body.start, covered.body.end);
  assert.equal(index.offsetForFrozenIndex(10, 0, 100, 8), null);
  const overlap = index.frozenWindow(0, 100, 200, 2);
  assert.equal(overlap.body.start, 2);
});
