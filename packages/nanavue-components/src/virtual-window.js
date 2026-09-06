/**
 * Visible-window geometry matching `nana_ui_core::VirtualListLayout::window`.
 * Uniform lists stay O(1); variable extents use prefix sums of sanitized sizes.
 */

export function sanitizeExtent(extent) {
  const value = Number(extent);
  return Number.isFinite(value) && value > 0 ? value : 0;
}

function emptyWindow(total = 0) {
  return { start: 0, end: 0, leading: 0, trailing: 0, total };
}

function itemCount(count) {
  const value = Number(count);
  return Number.isFinite(value) ? Math.max(0, Math.min(Number.MAX_SAFE_INTEGER, Math.floor(value))) : 0;
}

function prefixPartitionPoint(prefixAt, len, offset, inclusive) {
  const before = (value) => (inclusive ? value <= offset : value < offset);
  if (!before(0)) return 0;
  let index = 0;
  let step = 1;
  while (step <= Math.floor(len / 2)) step *= 2;
  while (step > 0) {
    const next = index + step;
    if (next <= len && before(prefixAt(next))) index = next;
    step = Math.floor(step / 2);
  }
  return index + 1;
}

function windowFromPrefix({
  len,
  total,
  prefixAt,
  scrollOffset,
  viewportExtent,
  overscanExtent,
  partitionPoint = (offset, inclusive) => prefixPartitionPoint(prefixAt, len, offset, inclusive),
}) {
  if (len === 0) return emptyWindow(total);
  const scroll = Math.min(sanitizeExtent(scrollOffset), total);
  const viewport = sanitizeExtent(viewportExtent);
  const overscan = sanitizeExtent(overscanExtent);
  const startOffset = Math.max(0, scroll - overscan);
  const endOffset = Math.min(total, scroll + viewport + overscan);
  const start = Math.max(
    0,
    Math.min(len - 1, partitionPoint(startOffset, true) - 1),
  );
  const end = Math.min(
    len,
    Math.max(start + 1, partitionPoint(endOffset, false)),
  );
  const leading = prefixAt(start);
  return {
    start,
    end,
    leading,
    trailing: total - prefixAt(end),
    total,
  };
}

export function uniformWindow(count, itemExtent, scrollOffset, viewportExtent, overscanExtent) {
  const n = itemCount(count);
  const extent = sanitizeExtent(itemExtent);
  const total = extent * n;
  if (n === 0 || extent === 0) return emptyWindow(total);
  return windowFromPrefix({
    len: n,
    total,
    prefixAt: (end) => extent * Math.min(end, n),
    scrollOffset,
    viewportExtent,
    overscanExtent,
  });
}

export function variableWindow(extents, scrollOffset, viewportExtent, overscanExtent) {
  return createWindowIndex({ extents }).window(scrollOffset, viewportExtent, overscanExtent);
}

/**
 * Build once per size change, query on every scroll. Vue callers keep this in
 * a computed that reads sizes only, so in-place reactive edits also invalidate
 * the prefix sums without making scroll/viewport changes rebuild them.
 */
export function createWindowIndex({ count = 0, itemExtent = 0, extents } = {}) {
  const variable = Array.isArray(extents) && extents.length > 0;
  const len = variable ? extents.length : itemCount(count);
  let sizes = variable ? Float64Array.from(extents, sanitizeExtent) : null;
  let tree = null;
  const uniform = sanitizeExtent(itemExtent);
  function build() {
    tree = new Float64Array(len + 1);
    for (let i = 1; i <= len; i += 1) {
      tree[i] += sizes[i - 1];
      const parent = i + (i & -i);
      if (parent <= len) tree[parent] += tree[i];
    }
  }
  if (sizes) build();
  function prefixAt(end) {
    end = Math.max(0, Math.min(len, Math.floor(Number(end) || 0)));
    if (!tree) return Math.min(end, len) * uniform;
    let sum = 0;
    for (let i = Math.min(end, len); i > 0; i -= i & -i) sum += tree[i];
    return sum;
  }
  function partitionPoint(offset, inclusive) {
    if (!tree) return prefixPartitionPoint(prefixAt, len, offset, inclusive);
    const before = value => inclusive ? value <= offset : value < offset;
    if (!before(0)) return 0;
    let index = 0, prefix = 0, step = 1;
    while (step <= Math.floor(len / 2)) step *= 2;
    for (; step > 0; step = Math.floor(step / 2)) {
      const next = index + step;
      if (next <= len && before(prefix + tree[next])) { index = next; prefix += tree[next]; }
    }
    return index + 1;
  }
  const window = (scrollOffset, viewportExtent, overscanExtent) => {
    if (!tree && uniform === 0) return emptyWindow(0);
    return windowFromPrefix({len, total: prefixAt(len), prefixAt, partitionPoint, scrollOffset, viewportExtent, overscanExtent});
  };
  const anchor = offset => {
    if (len === 0) return null;
    offset = Math.min(sanitizeExtent(offset), prefixAt(len));
    const index = Math.max(0, Math.min(len - 1, partitionPoint(offset, true) - 1));
    return {index, inset: offset - prefixAt(index)};
  };
  const restoreAnchor = (anchor, viewport = 0) => {
    if (!anchor || len === 0) return 0;
    const index = Math.max(0, Math.min(len - 1, Math.floor(Number(anchor.index) || 0)));
    return Math.min(prefixAt(index) + Math.min(sanitizeExtent(anchor.inset), sizes ? sizes[index] : uniform), Math.max(0, prefixAt(len) - sanitizeExtent(viewport)));
  };
  function measure(index, extent, viewport) {
    if (!Number.isInteger(index) || index < 0 || index >= len) return false;
    extent = sanitizeExtent(extent);
    const previous = sizes ? sizes[index] : uniform;
    if (previous === extent) return false;
    const retained = viewport ? anchor(viewport.offset[1]) : null;
    if (!sizes) { sizes = new Float64Array(len).fill(uniform); build(); }
    sizes[index] = extent;
    for (let i = index + 1; i <= len; i += i & -i) tree[i] += extent - previous;
    if (viewport) viewport.offset[1] = restoreAnchor(retained, viewport.extent[1]);
    return true;
  }
  function offsetForIndex(index, offset, viewport, alignment = "nearest") {
    if (!Number.isInteger(index) || index < 0 || index >= len) return null;
    if (!["nearest", "start", "center", "end"].includes(alignment)) {
      throw new RangeError(`Unknown virtual alignment: ${alignment}`);
    }
    viewport = sanitizeExtent(viewport);
    const maxOffset = Math.max(0, prefixAt(len) - viewport);
    offset = Math.min(sanitizeExtent(offset), maxOffset);
    const start = prefixAt(index), end = prefixAt(index + 1);
    let next = offset;
    if (alignment === "start") next = start;
    else if (alignment === "center") next = (start + end - viewport) / 2;
    else if (alignment === "end") next = end - viewport;
    else if (!((start >= offset && end <= offset + viewport)
      || (start <= offset && end >= offset + viewport))) {
      next = Math.abs(start - offset) <= Math.abs(end - viewport - offset)
        ? start : end - viewport;
    }
    return Math.max(0, Math.min(maxOffset, next));
  }
  function retainedRanges(win, retainedIndices = []) {
    const ranges = retainedIndices.filter(index => Number.isInteger(index) && index >= 0 && index < len)
      .map(index => ({ start: index, end: index + 1 }));
    const start = Math.max(0, Math.min(len, win.start));
    const end = Math.max(0, Math.min(len, win.end));
    if (start < end) ranges.push({ start, end });
    ranges.sort((a, b) => a.start - b.start);
    const merged = [];
    for (const range of ranges) {
      const previous = merged.at(-1);
      if (previous && range.start <= previous.end) previous.end = Math.max(previous.end, range.end);
      else merged.push(range);
    }
    return merged;
  }
  function frozenWindow(offset, extent, overscan, count = 0) {
    count = Math.min(len, itemCount(count));
    const frozenExtent = prefixAt(count);
    const available = Math.max(0, sanitizeExtent(extent) - frozenExtent);
    const body = available > 0 ? window(sanitizeExtent(offset) + frozenExtent, available, overscan)
      : {start: count, end: count, leading: frozenExtent, trailing: prefixAt(len) - frozenExtent, total: prefixAt(len)};
    body.start = Math.max(count, body.start);
    body.end = Math.max(body.start, body.end);
    body.leading = prefixAt(body.start);
    body.trailing = body.total - prefixAt(body.end);
    const visible = count && extent > 0 ? window(0, Math.min(extent, frozenExtent), 0) : emptyWindow();
    const frozen = [];
    for (let i = visible.start; i < Math.min(count, visible.end); i++) frozen.push(i);
    return {body, frozen, count, frozenExtent};
  }
  function offsetForFrozenIndex(index, offset, extent, count, alignment = "nearest") {
    if (!Number.isInteger(index) || index < 0 || index >= len) return null;
    count = Math.min(len, itemCount(count));
    if (index < count) {
      if (prefixAt(index) >= sanitizeExtent(extent)) return null;
      return Math.min(sanitizeExtent(offset), Math.max(0, prefixAt(len) - sanitizeExtent(extent)));
    }
    const frozen = prefixAt(count);
    if (frozen >= sanitizeExtent(extent)) return null;
    const next = offsetForIndex(index, sanitizeExtent(offset) + frozen, extent - frozen, alignment);
    return Math.max(0, next - frozen);
  }
  return { length: len, window, anchor, restoreAnchor, measure, prefixAt, offsetForIndex, retainedRanges, frozenWindow, offsetForFrozenIndex,
    windowFor: (viewport, axis = 1) => window(viewport.offset[axis], viewport.extent[axis], viewport.overscan[axis]),
  };
}

export function virtualViewport({offset = [0, 0], extent = [0, 0], overscan = [0, 0]} = {}) {
  return {offset: offset.map(sanitizeExtent), extent: extent.map(sanitizeExtent), overscan: overscan.map(sanitizeExtent)};
}

export function virtualWindow({
  count = 0,
  itemExtent = 0,
  extents,
  scroll = 0,
  viewport = 0,
  overscan = 0,
} = {}) {
  return createWindowIndex({ count, itemExtent, extents }).window(scroll, viewport, overscan);
}
