/**
 * NanaToast — outlined notification.
 * Semantic peer of Runtime `Toast` (`nana-toast`).
 *
 * `title`/`label` is the heading; `description`/`hint` is supporting copy.
 * `tone` is info / success / warning / danger. `dismissible` or `@dismiss`
 * marks a dismiss hit target (no timer).
 */
import { h } from "@vue/runtime-core";

export const NanaToast = {
  name: "NanaToast",
  props: {
    title: { type: String, default: "" },
    label: { type: String, default: "" },
    description: { type: String, default: "" },
    hint: { type: String, default: "" },
    tone: { type: String, default: "info" },
    dismissible: { type: Boolean, default: false },
  },
  emits: ["dismiss", "close"],
  setup(props, { emit, attrs }) {
    function onDismiss(ev) {
      emit("dismiss", ev);
      emit("close", ev);
    }

    return () => {
      const title = props.title || props.label;
      const description = props.description || props.hint;
      return h("nana-toast", {
        ...attrs,
        class: ["nana-toast", attrs.class].filter(Boolean).join(" "),
        label: title,
        title,
        hint: description,
        description,
        tone: props.tone,
        "data-tone": props.tone,
        dismissible: props.dismissible,
        "data-dismissible": props.dismissible ? "" : undefined,
        "data-agent-id": attrs["data-agent-id"] || "nana.toast",
        onDismiss,
        onClose: onDismiss,
      });
    };
  },
};

export default NanaToast;
