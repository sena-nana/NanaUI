/**
 * NanaProgress — semantic peer of Runtime `Progress` (`nana-progress`).
 */
import { h } from "@vue/runtime-core";

export const NanaProgress = {
  name: "NanaProgress",
  props: {
    value: { type: Number, default: 0 },
    max: { type: Number, default: 1 },
    label: { type: String, default: "" },
    cancellable: { type: Boolean, default: false },
  },
  emits: ["cancel"],
  setup(props, { emit, attrs }) {
    return () =>
      h("nana-progress", {
        ...attrs,
        class: ["nana-progress", attrs.class].flat().filter(Boolean).join(" "),
        role: attrs.role || "progressbar",
        label: props.label,
        value: props.value,
        progress: props.value,
        max: props.max,
        cancellable: props.cancellable,
        "data-agent-id": attrs["data-agent-id"] || "nana.progress",
        onCancel: (ev) => emit("cancel", ev),
      });
  },
};

export default NanaProgress;
