/**
 * NanaGpu — Vue host for Runtime `GpuTextureView` (`<nana-gpu>`).
 * `source` is the host texture slot (`data-nana-gpu`). Not a 2D canvas.
 */
import { h } from "@vue/runtime-core";

export const NanaGpu = {
  name: "NanaGpu",
  props: {
    source: { type: [String, Number], default: "default" },
  },
  setup(props, { attrs }) {
    return () => {
      const source = String(props.source || attrs["data-nana-gpu"] || "default");
      return h("nana-gpu", {
        ...attrs,
        class: ["nana-gpu", attrs.class].flat().filter(Boolean).join(" "),
        source,
        "data-nana-gpu": source,
        "data-agent-id": attrs["data-agent-id"] || "nana.gpu",
      });
    };
  },
};

export default NanaGpu;
