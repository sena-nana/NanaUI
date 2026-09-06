/**
 * NanaVirtualTree — flattened visible-row window over Runtime `ScrollView`.
 * Pass the expanded walk (`count` / `extents`); collapsed subtrees stay off the index.
 */
import { computed, h } from "@vue/runtime-core";
import { createWindowIndex, virtualViewport } from "./virtual-window.js";
import { useScrollWindow, retainedWindowChildren, retainedKeyIndices, useVirtualActivity } from "./NanaVirtualList.js";

export const NanaVirtualTree = {
  name: "NanaVirtualTree",
  props: {
    count: { type: Number, default: 0 },
    itemExtent: { type: Number, default: 28 },
    extents: { type: Array, default: undefined },
    overscan: { type: Number, default: 64 },
    scrollbars: { type: String, default: "auto" },
    keyAt: { type: Function, default: undefined },
    indexOfKey: { type: Function, default: undefined },
    retainedKeys: { type: Array, default: () => [] },
    depthAt: { type: Function, default: undefined },
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
          class: ["nana-virtual-tree", attrs.class].flat().filter(Boolean).join(" "),
          scrollbars: props.scrollbars,
          axes: "vertical",
          "data-agent-id": attrs["data-agent-id"] || "nana.virtual-tree",
          onScroll,
        },
        retainedWindowChildren(sizes.value, windowed.value, [...retainedKeyIndices(props, sizes.value.length), ...activity.indices()], "nana-virtual-tree", "y", (index) => {
          const key = props.keyAt ? props.keyAt(index) : index;
          const depth = props.depthAt ? props.depthAt(index) : 0;
          return h(
            "div",
            { key, ...activity.handlers(index), class: "nana-virtual-tree__row", style: { height: `${sizes.value.prefixAt(index + 1) - sizes.value.prefixAt(index)}px`, flexShrink: 0 } },
            slots.default?.({ index, key, depth }) || [],
          );
        }),
      );
  },
};

export default NanaVirtualTree;
