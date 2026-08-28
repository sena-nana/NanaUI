/**
 * NanaValidationMessage — semantic peer of Runtime `ValidationMessage`.
 */
import { h } from "@vue/runtime-core";

export const NanaValidationMessage = {
  name: "NanaValidationMessage",
  props: {
    message: { type: String, default: "" },
    label: { type: String, default: "" },
    hint: { type: String, default: "" },
    intent: { type: String, default: "error" },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const message =
        props.message ||
        props.hint ||
        props.label ||
        (typeof slots.default === "function"
          ? slots
              .default()
              .map((vnode) => (typeof vnode.children === "string" ? vnode.children : ""))
              .join("")
          : "");
      return h("nana-validation-message", {
        ...attrs,
        class: ["nana-validation-message", attrs.class].flat().filter(Boolean).join(" "),
        label: message,
        hint: message,
        intent: props.intent,
        "data-agent-id": attrs["data-agent-id"] || "nana.validation-message",
      });
    };
  },
};

export default NanaValidationMessage;
