/**
 * NanaAvatar — circular Cover-fit host-texture slot (`nana-avatar`).
 */
import { h } from "@vue/runtime-core";

export const NanaAvatar = {
  name: "NanaAvatar",
  props: {
    src: { type: String, default: "" },
    value: { type: String, default: "" },
    label: { type: String, default: "" },
    size: { type: [Number, String], default: 32 },
  },
  setup(props, { attrs }) {
    return () => {
      const value = props.value || props.src;
      return h("nana-avatar", {
        ...attrs,
        class: ["nana-avatar", attrs.class].flat().filter(Boolean).join(" "),
        value,
        src: value,
        label: props.label,
        size: props.size,
        "data-agent-id": attrs["data-agent-id"] || "nana.avatar",
      });
    };
  },
};

export default NanaAvatar;
