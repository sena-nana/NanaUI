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

function prefixPartitionPoint(prefixAt, len, offset, inclusive) {
  const before = (value) => (inclusive ? value <= offset : value < offset);
  if (!before(0)) return 0;
  let index = 0;
  let step = 1;
  while (step <= Math.floor(len / 2)) step <<= 1;
  while (step > 0) {
    const next = index + step;
    if (next <= len && before(prefixAt(next))) index = next;
    step >>= 1;
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
}) {
  if (len === 0) return emptyWindow(total);
  const scroll = Math.min(sanitizeExtent(scrollOffset), total);
  const viewport = sanitizeExtent(viewportExtent);
  const overscan = sanitizeExtent(overscanExtent);
  const startOffset = Math.max(0, scroll - overscan);
  const endOffset = Math.min(total, scroll + viewport + overscan);
  const start = Math.max(
    0,
    Math.min(len - 1, prefixPartitionPoint(prefixAt, len, startOffset, true) - 1),
  );
  const end = Math.min(
    len,
    Math.max(start + 1, prefixPartitionPoint(prefixAt, len, endOffset, false)),
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
  const n = Math.max(0, Math.floor(Number(count) || 0));
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
  const items = Array.isArray(extents) ? extents.map(sanitizeExtent) : [];
  const len = items.length;
  if (len === 0) return emptyWindow(0);
  const prefix = new Array(len + 1).fill(0);
  for (let i = 0; i < len; i += 1) prefix[i + 1] = prefix[i] + items[i];
  return windowFromPrefix({
    len,
    total: prefix[len],
    prefixAt: (end) => prefix[Math.min(end, len)],
    scrollOffset,
    viewportExtent,
    overscanExtent,
  });
}

export function virtualWindow({
  count = 0,
  itemExtent = 0,
  extents,
  scroll = 0,
  viewport = 0,
  overscan = 0,
} = {}) {
  if (Array.isArray(extents) && extents.length) {
    return variableWindow(extents, scroll, viewport, overscan);
  }
  return uniformWindow(count, itemExtent, scroll, viewport, overscan);
}
