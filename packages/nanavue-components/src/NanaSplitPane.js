/**
 * NanaSplitPane — host tag for Runtime `SplitPane` (`nana-split-pane`).
 * `axis` is `horizontal` | `vertical`. First two children are panes.
 * Optional `nana-split-handle` / third child is the resize handle.
 */
import { h } from "@vue/runtime-core";

export const NanaSplitPane = {
  name: "NanaSplitPane",
  props: {
    axis: { type: String, default: "horizontal" },
    size: { type: Number, default: undefined },
    defaultSize: { type: Number, default: undefined },
    min: { type: Number, default: undefined },
    max: { type: Number, default: undefined },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const axis = props.axis === "vertical" ? "vertical" : "horizontal";
      return h(
        "nana-split-pane",
        {
          ...attrs,
          class: ["nana-split-pane", attrs.class].flat().filter(Boolean).join(" "),
          axis,
          size: props.size,
          "default-size": props.defaultSize,
          min: props.min,
          max: props.max,
          "data-agent-id": attrs["data-agent-id"] || "nana.split-pane",
        },
        slots.default?.(),
      );
    };
  },
};

export default NanaSplitPane;
