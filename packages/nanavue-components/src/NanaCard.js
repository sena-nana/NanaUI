/**
 * NanaCard — semantic peer of Runtime `Card` (`nana-card`).
 */
import { h } from "@vue/runtime-core";

export const NanaCard = {
  name: "NanaCard",
  props: {
    title: { type: String, default: "" },
    kind: { type: String, default: "surface" },
    loading: { type: Boolean, default: false },
  },
  setup(props, { slots, attrs }) {
    return () =>
      h(
        "nana-card",
        {
          ...attrs,
          class: ["nana-card", attrs.class].flat().filter(Boolean).join(" "),
          label: props.title,
          kind: props.kind,
          loading: props.loading,
          "data-agent-id": attrs["data-agent-id"] || "nana.card",
        },
        slots.default?.(),
      );
  },
};

export default NanaCard;
