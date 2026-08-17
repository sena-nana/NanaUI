/**
 * NanaImageViewer — overlay viewer for a host texture slot.
 * Semantic peer of Runtime `ImageViewer` (`nana-image-viewer`).
 *
 * `src` / `value` is the host texture id only. This wrapper does not
 * fetch or decode pixels.
 */
import { h } from "@vue/runtime-core";

export const NanaImageViewer = {
  name: "NanaImageViewer",
  props: {
    open: { type: Boolean, default: false },
    src: { type: String, default: "" },
    value: { type: String, default: "" },
  },
  emits: ["update:open", "close"],
  setup(props, { emit, attrs }) {
    function emitClose(ev) {
      emit("update:open", false);
      emit("close", ev);
    }

    function onChange(ev) {
      if (ev === false || ev?.value === false) emitClose(ev);
    }

    return () => {
      const src = props.src || props.value || "";
      return h("nana-image-viewer", {
        ...attrs,
        class: ["nana-image-viewer", attrs.class].filter(Boolean).join(" "),
        open: props.open,
        active: props.open,
        toggled: props.open,
        src,
        value: src,
        "data-agent-id": attrs["data-agent-id"] || "nana.image-viewer",
        onClose: emitClose,
        onChange,
      });
    };
  },
};

export default NanaImageViewer;
