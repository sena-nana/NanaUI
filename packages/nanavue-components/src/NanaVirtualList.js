/**
 * NanaVirtualList — visible-window list over Runtime `ScrollView`.
 * Geometry matches `VirtualListLayout::window`.
 */
import { computed, h, onMounted, onUpdated, ref } from "@vue/runtime-core";
import { createWindowIndex } from "./virtual-window.js";

export function hostExtent(el, axis) {
  if (!el) return 0;
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

  function measure() {
    const nextH = hostExtent(host.value, "y");
    const nextW = hostExtent(host.value, "x");
    if (nextH > 0) height.value = nextH;
    if (nextW > 0) width.value = nextW;
  }

  onMounted(measure);
  onUpdated(measure);

  return {
    x,
    y,
    width,
    height,
    measure,
    bindHost: (el) => {
      host.value = el;
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
  return h("div", { class: className, style });
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

export const NanaVirtualList = {
  name: "NanaVirtualList",
  props: {
    count: { type: Number, default: 0 },
    itemExtent: { type: Number, default: 32 },
    extents: { type: Array, default: undefined },
    overscan: { type: Number, default: 64 },
    scrollbars: { type: String, default: "auto" },
    keyAt: { type: Function, default: undefined },
  },
  setup(props, { slots, attrs }) {
    const { y, height, bindHost, onScroll } = useScrollWindow();
    const sizes = computed(() =>
      createWindowIndex({
        count: props.count,
        itemExtent: props.itemExtent,
        extents: props.extents,
      }),
    );

    const windowed = computed(() => sizes.value.window(y.value, height.value, props.overscan));

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
        windowChildren(windowed.value, "nana-virtual-list", "y", (index) => {
          const key = props.keyAt ? props.keyAt(index) : index;
          return h(
            "div",
            { key, class: "nana-virtual-list__item" },
            slots.default?.({ index, key }) || [],
          );
        }),
      );
  },
};

export default NanaVirtualList;
