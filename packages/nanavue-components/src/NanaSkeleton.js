/**
 * NanaSkeleton — semantic peer of Runtime `Skeleton` (`nana-skeleton`).
 */
import { h } from "@vue/runtime-core";

export const NanaSkeleton = {
  name: "NanaSkeleton",
  props: {
    width: { type: [Number, String], default: undefined },
    height: { type: [Number, String], default: undefined },
  },
  setup(props, { attrs }) {
    return () =>
      h("nana-skeleton", {
        ...attrs,
        class: ["nana-skeleton", attrs.class].flat().filter(Boolean).join(" "),
        width: props.width,
        height: props.height,
        "data-agent-id": attrs["data-agent-id"] || "nana.skeleton",
      });
  },
};

export default NanaSkeleton;
