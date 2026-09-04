/**
 * NanaVirtualTree — flattened visible-row window over Runtime `ScrollView`.
 * Pass the expanded walk (`count` / `extents`); collapsed subtrees stay off the index.
 */
import { computed, h } from "@vue/runtime-core";
import { createWindowIndex } from "./virtual-window.js";
import { useScrollWindow, windowChildren } from "./NanaVirtualList.js";

export const NanaVirtualTree = {
  name: "NanaVirtualTree",
  props: {
    count: { type: Number, default: 0 },
    itemExtent: { type: Number, default: 28 },
    extents: { type: Array, default: undefined },
    overscan: { type: Number, default: 64 },
    scrollbars: { type: String, default: "auto" },
    keyAt: { type: Function, default: undefined },
    depthAt: { type: Function, default: undefined },
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
          class: ["nana-virtual-tree", attrs.class].flat().filter(Boolean).join(" "),
          scrollbars: props.scrollbars,
          axes: "vertical",
          "data-agent-id": attrs["data-agent-id"] || "nana.virtual-tree",
          onScroll,
        },
        windowChildren(windowed.value, "nana-virtual-tree", "y", (index) => {
          const key = props.keyAt ? props.keyAt(index) : index;
          const depth = props.depthAt ? props.depthAt(index) : 0;
          return h(
            "div",
            { key, class: "nana-virtual-tree__row" },
            slots.default?.({ index, key, depth }) || [],
          );
        }),
      );
  },
};

export default NanaVirtualTree;
