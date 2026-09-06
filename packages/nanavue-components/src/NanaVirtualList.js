/**
 * NanaVirtualList — visible-window list over Runtime `ScrollView`.
 * Geometry matches `VirtualListLayout::window`.
 */
import { computed, h, nextTick, onMounted, onUnmounted, onUpdated, ref, shallowRef, watchEffect } from "@vue/runtime-core";
import { createWindowIndex, virtualViewport } from "./virtual-window.js";

export function hostExtent(el, axis) {
  if (!el) return 0;
  const client = Number(axis === "x" ? el.clientWidth : el.clientHeight);
  if (Number.isFinite(client) && client > 0) return client;
  const box = el.getBoundingClientRect?.() || el.layoutBox || {};
  const value = axis === "x" ? Number(box.width) : Number(box.height);
  return Number.isFinite(value) && value > 0 ? value : 0;
}

export function scrollOffset(ev, axis) {
  if (axis === "x") {
    return Number(ev?.scrollLeft ?? ev?.offset?.x ?? ev?.x);
  }
  return Number(ev?.scrollTop ?? ev?.offset?.y ?? ev?.y);
}

export function useScrollWindow() {
  const host = ref(null);
  const x = ref(0);
  const y = ref(0);
  const width = ref(0);
  const height = ref(0);
  let observer = null;

  function measure() {
    const nextH = hostExtent(host.value, "y");
    const nextW = hostExtent(host.value, "x");
    if (nextH > 0) height.value = nextH;
    if (nextW > 0) width.value = nextW;
  }

  onMounted(() => {
    // Runtime layout commits after Vue mount. Observe its published geometry
    // so the first window and later resizes do not wait for a scroll event.
    if (typeof globalThis.ResizeObserver === "function") {
      observer = new globalThis.ResizeObserver(measure);
      if (host.value) observer.observe(host.value);
    }
    measure();
  });
  onUnmounted(() => observer?.disconnect());
  onUpdated(measure);

  return {
    x,
    y,
    width,
    height,
    measure,
    async scrollTo(nextX, nextY) {
      if (!host.value) return false;
      x.value = nextX;
      y.value = nextY;
      // Materialize first: the host must see the new spacer geometry before
      // clamping the requested offset to the content extent.
      await nextTick();
      if (!host.value) return false;
      host.value.scrollTo(nextX, nextY);
      return true;
    },
    bindHost: (el) => {
      if (host.value === el) return;
      observer?.disconnect();
      host.value = el;
      if (el) observer?.observe(el);
    },
    onScroll: (ev) => {
      const nextY = scrollOffset(ev, "y");
      const nextX = scrollOffset(ev, "x");
      if (Number.isFinite(nextY)) y.value = nextY;
      if (Number.isFinite(nextX)) x.value = nextX;
      measure();
    },
  };
}

export function spacer(className, style) {
  return h("div", { class: className, style: { ...style, pointerEvents: "none" } });
}

export function windowChildren(win, classPrefix, axis, renderItem) {
  const sizeKey = axis === "x" ? "width" : "height";
  const children = [];
  if (win.leading > 0) {
    children.push(spacer(`${classPrefix}__spacer`, { [sizeKey]: `${win.leading}px` }));
  }
  for (let index = win.start; index < win.end; index += 1) {
    children.push(renderItem(index));
  }
  if (win.trailing > 0) {
    children.push(spacer(`${classPrefix}__spacer`, { [sizeKey]: `${win.trailing}px` }));
  }
  return children;
}

// Gaps stay in their original positions, so an offscreen editor can keep its
// keyed instance without shifting visible rows or mounting intervening data.
export function retainedWindowChildren(sizes, win, retainedIndices, classPrefix, axis, renderItem) {
  const sizeKey = axis === "x" ? "width" : "height";
  const children = [];
  let offset = 0;
  const gap = (end, key) => {
    if (end > offset) children.push(h("div", {
      key: Symbol(`nana.virtual.gap:${key}`), class: `${classPrefix}__spacer`,
      style: { [sizeKey]: `${end - offset}px`, flexShrink: 0, pointerEvents: "none" },
    }));
  };
  for (const range of sizes.retainedRanges(win, retainedIndices)) {
    gap(sizes.prefixAt(range.start), range.start);
    for (let index = range.start; index < range.end; index++) children.push(renderItem(index));
    offset = sizes.prefixAt(range.end);
  }
  gap(win.total, "end");
  return children;
}

export function retainedKeyIndices(props, count) {
  if (!props.retainedKeys.length) return [];
  if (!props.indexOfKey || !props.keyAt) {
    throw new TypeError("retainedKeys requires keyAt and indexOfKey");
  }
  // Missing/deleted keys are released. Verify the inverse so stale indices
  // after a reorder never pin a different business item.
  return props.retainedKeys.map(key => {
    const index = props.indexOfKey(key);
    return Number.isInteger(index) && index >= 0 && index < count && Object.is(props.keyAt(index), key) ? index : -1;
  });
}

// One focus owner and one composition owner per viewport, independent of data size.
// A custom key can follow reorders through indexOfKey; without an inverse it is
// retained only while its original position still identifies the same item.
export function useVirtualActivity(...axes) {
  const focused = shallowRef(null);
  const composing = shallowRef(null);
  function resolve(session, axis) {
    const { count, keyAt, indexOfKey } = axes[axis]();
    const item = session.items[axis];
    const index = indexOfKey ? indexOfKey(item.key) : item.index;
    return Number.isInteger(index) && index >= 0 && index < count &&
      Object.is(keyAt ? keyAt(index) : index, item.key) ? index : -1;
  }
  // Deletion/collapse ends ownership, so a later reused key cannot resurrect it.
  watchEffect(() => {
    for (const owner of [focused, composing]) {
      if (owner.value && axes.some((_, axis) => resolve(owner.value, axis) < 0)) owner.value = null;
    }
  });
  onUnmounted(() => { focused.value = composing.value = null; });
  return {
    indices(axis = 0) {
      return [focused.value, composing.value].filter(Boolean).map(session => resolve(session, axis));
    },
    handlers(...indices) {
      const items = indices.map((index, axis) => ({
        index, key: axes[axis]().keyAt ? axes[axis]().keyAt(index) : index,
      }));
      const begin = (owner, event) => { owner.value = { target: event.target, items }; };
      const end = (owner, event) => {
        if (owner.value?.target === event.target) owner.value = null;
      };
      return {
        onFocusCapture: event => begin(focused, event),
        onBlurCapture: event => end(focused, event),
        onCompositionstartCapture: event => begin(composing, event),
        onCompositionendCapture: event => end(composing, event),
      };
    },
  };
}

export const NanaVirtualList = {
  name: "NanaVirtualList",
  props: {
    count: { type: Number, default: 0 },
    itemExtent: { type: Number, default: 32 },
    extents: { type: Array, default: undefined },
    overscan: { type: Number, default: 64 },
    scrollbars: { type: String, default: "auto" },
    keyAt: { type: Function, default: undefined },
    indexOfKey: { type: Function, default: undefined },
    retainedKeys: { type: Array, default: () => [] },
  },
  setup(props, { slots, attrs, expose }) {
    const { y, height, bindHost, onScroll, scrollTo } = useScrollWindow();
    const sizes = computed(() =>
      createWindowIndex({
        count: props.count,
        itemExtent: props.itemExtent,
        extents: props.extents,
      }),
    );

    const activity = useVirtualActivity(() => ({ count: sizes.value.length, keyAt: props.keyAt, indexOfKey: props.indexOfKey }));
    const windowed = computed(() => sizes.value.windowFor(virtualViewport({
      offset: [0, y.value], extent: [0, height.value], overscan: [0, props.overscan],
    })));

    expose({
      async scrollToIndex(index, alignment = "nearest") {
        const offset = sizes.value.offsetForIndex(index, y.value, height.value, alignment);
        return offset !== null && await scrollTo(0, offset);
      },
    });

    return () =>
      h(
        "nana-scroll-view",
        {
          ...attrs,
          ref: bindHost,
          class: ["nana-virtual-list", attrs.class].flat().filter(Boolean).join(" "),
          scrollbars: props.scrollbars,
          axes: "vertical",
          "data-agent-id": attrs["data-agent-id"] || "nana.virtual-list",
          onScroll,
        },
        retainedWindowChildren(sizes.value, windowed.value, [...retainedKeyIndices(props, sizes.value.length), ...activity.indices()], "nana-virtual-list", "y", (index) => {
          const key = props.keyAt ? props.keyAt(index) : index;
          return h(
            "div",
            { key, ...activity.handlers(index), class: "nana-virtual-list__item", style: { height: `${sizes.value.prefixAt(index + 1) - sizes.value.prefixAt(index)}px`, flexShrink: 0 } },
            slots.default?.({ index, key }) || [],
          );
        }),
      );
  },
};

export default NanaVirtualList;
