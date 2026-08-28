/**
 * NanaThumbnail — semantic peer of Runtime `Thumbnail` (`nana-thumbnail`).
 */
import { h } from "@vue/runtime-core";

export const NanaThumbnail = {
  name: "NanaThumbnail",
  props: {
    src: { type: String, default: "" },
    value: { type: String, default: "" },
    label: { type: String, default: "" },
    size: { type: String, default: "medium" },
    aspect: { type: [Number, String], default: undefined },
    loading: { type: Boolean, default: false },
    invalid: { type: Boolean, default: false },
  },
  setup(props, { attrs }) {
    return () => {
      const value = props.value || props.src;
      return h("nana-thumbnail", {
        ...attrs,
        class: ["nana-thumbnail", attrs.class].flat().filter(Boolean).join(" "),
        value,
        src: value,
        label: props.label,
        size: props.size,
        aspect: props.aspect,
        loading: props.loading,
        invalid: props.invalid,
        "data-agent-id": attrs["data-agent-id"] || "nana.thumbnail",
      });
    };
  },
};

export default NanaThumbnail;
