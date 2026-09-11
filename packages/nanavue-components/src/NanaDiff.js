/**
 * NanaDiff — structured patch review. Semantic peer of Runtime `DiffView`.
 * Buffer/hunk authority stays with the application; accept/reject are requests.
 */
import { h } from "@vue/runtime-core";

export const NanaDiff = {
  name: "NanaDiff",
  props: {
    hunks: { type: Array, default: () => [] },
    layout: { type: String, default: "unified" },
    disabled: { type: Boolean, default: false },
  },
  emits: ["hunk-accept", "hunk-reject", "line-accept", "line-reject", "layout-change"],
  setup(props, { attrs, emit }) {
    return () =>
      h("nana-diff", {
        ...attrs,
        class: ["nana-diff", props.disabled ? "is-disabled" : "", attrs.class]
          .filter(Boolean)
          .join(" "),
        hunks: JSON.stringify(props.hunks || []),
        layout: props.layout,
        disabled: props.disabled,
        "data-agent-id": attrs["data-agent-id"] || "nana.diff",
        onHunkAccept: (ev) => emit("hunk-accept", ev),
        onHunkReject: (ev) => emit("hunk-reject", ev),
        onLineAccept: (ev) => emit("line-accept", ev),
        onLineReject: (ev) => emit("line-reject", ev),
        onLayoutChange: (ev) => emit("layout-change", ev),
      });
  },
};

export default NanaDiff;
