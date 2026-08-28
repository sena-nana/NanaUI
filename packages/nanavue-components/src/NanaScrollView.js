/**
 * NanaScrollView — semantic peer of Runtime `ScrollView` (`nana-scroll-view`).
 * `scrollbars` is `auto` (default) | `always` | `hidden`.
 * `axes` is `vertical` (default) | `horizontal` | `both`.
 */
import { h } from "@vue/runtime-core";

export const NanaScrollView = {
  name: "NanaScrollView",
  props: {
    label: { type: String, default: "" },
    axes: { type: String, default: "vertical" },
    scrollbars: { type: String, default: "auto" },
  },
  setup(props, { slots, attrs }) {
    return () =>
      h(
        "nana-scroll-view",
        {
          ...attrs,
          class: ["nana-scroll-view", attrs.class].flat().filter(Boolean).join(" "),
          label: props.label,
          axes: props.axes,
          scrollbars: props.scrollbars,
          "data-agent-id": attrs["data-agent-id"] || "nana.scroll-view",
        },
        slots.default?.(),
      );
  },
};

export default NanaScrollView;
