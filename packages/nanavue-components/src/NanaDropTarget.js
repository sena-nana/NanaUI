/**
 * NanaDropTarget — file drop surface. Semantic peer of Runtime drop-target
 * (`nana-drop-target` → Stack + DropAccepts::files). Framework resolves OS
 * file drags onto this node and paints hover chrome.
 */
import { h } from "@vue/runtime-core";

export const NanaDropTarget = {
  name: "NanaDropTarget",
  props: {
    accepts: { type: String, default: "files" },
    disabled: { type: Boolean, default: false },
  },
  emits: ["filedrop", "filehover", "fileleave", "drop", "dragenter", "dragleave"],
  setup(props, { slots, emit, attrs }) {
    return () =>
      h(
        "nana-drop-target",
        {
          ...attrs,
          class: ["nana-drop-target", props.disabled ? "is-disabled" : "", attrs.class]
            .filter(Boolean)
            .join(" "),
          "drop-accepts": props.disabled ? "false" : props.accepts,
          "data-agent-id": attrs["data-agent-id"] || "nana.drop-target",
          onFiledrop: (ev) => emit("filedrop", ev),
          onFilehover: (ev) => emit("filehover", ev),
          onFileleave: (ev) => emit("fileleave", ev),
          onDrop: (ev) => emit("drop", ev),
          onDragenter: (ev) => emit("dragenter", ev),
          onDragleave: (ev) => emit("dragleave", ev),
        },
        slots.default?.(),
      );
  },
};

export default NanaDropTarget;
