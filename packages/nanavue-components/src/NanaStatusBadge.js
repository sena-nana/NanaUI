/**
 * NanaStatusBadge — semantic peer of Runtime `StatusBadge` (`nana-status-badge`).
 */
import { h } from "@vue/runtime-core";

export const NanaStatusBadge = {
  name: "NanaStatusBadge",
  props: {
    label: { type: String, default: "" },
    tone: { type: String, default: "neutral" },
    compact: { type: Boolean, default: false },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const label =
        props.label ||
        (typeof slots.default === "function"
          ? slots
              .default()
              .map((vnode) => (typeof vnode.children === "string" ? vnode.children : ""))
              .join("")
          : "");
      return h("nana-status-badge", {
        ...attrs,
        class: ["nana-status-badge", `nana-status--${props.tone}`, attrs.class]
          .flat()
          .filter(Boolean)
          .join(" "),
        label,
        tone: props.tone,
        compact: props.compact,
        "data-agent-id": attrs["data-agent-id"] || "nana.status-badge",
      });
    };
  },
};

export default NanaStatusBadge;
