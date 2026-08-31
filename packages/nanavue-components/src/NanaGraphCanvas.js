/**
 * NanaGraphCanvas — node/edge canvas host.
 * Semantic peer of Runtime `GraphCanvas` (`nana-graph-canvas`).
 *
 * `nodes` / `edges` / `model` / `viewport` / `selection` pass through.
 * Host projects an empty `GraphModel` when the payload cannot be
 * interpreted. Default slot children are per-node interiors: set
 * `data-slot` (or `data-node`) to a graph node id. HostTexture children
 * also set `data-nana-gpu`. Default paint stays grid/frames/edges.
 */
import { h } from "@vue/runtime-core";

export const NanaGraphCanvas = {
  name: "NanaGraphCanvas",
  props: {
    nodes: { type: Array, default: undefined },
    edges: { type: Array, default: undefined },
    model: { default: undefined },
    viewport: { default: undefined },
    selection: { default: undefined },
  },
  setup(props, { attrs, slots }) {
    return () =>
      h(
        "nana-graph-canvas",
        {
          ...attrs,
          class: ["nana-graph-canvas", attrs.class].filter(Boolean).join(" "),
          nodes: props.nodes,
          edges: props.edges,
          model: props.model,
          viewport: props.viewport,
          selection: props.selection,
          "data-agent-id": attrs["data-agent-id"] || "nana.graph-canvas",
        },
        slots.default?.(),
      );
  },
};

export default NanaGraphCanvas;
