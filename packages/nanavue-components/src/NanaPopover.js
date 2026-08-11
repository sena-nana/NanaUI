/**
 * NanaPopover — anchored popover overlay.
 * Semantic peer of Rust `Popover` (`nana-popover`).
 *
 * `label` is the trigger text; default slot is popover body (or `description`/`hint`
 * when empty). Open via `open` / boolean `modelValue`.
 */
import { h } from "@vue/runtime-core";

function resolveOpen(props) {
  if (typeof props.open === "boolean") return props.open;
  if (typeof props.modelValue === "boolean") return props.modelValue;
  return false;
}

export const NanaPopover = {
  name: "NanaPopover",
  props: {
    open: { type: Boolean, default: undefined },
    modelValue: { type: Boolean, default: undefined },
    label: { type: String, default: "" },
    title: { type: String, default: "" },
    description: { type: String, default: "" },
    hint: { type: String, default: "" },
  },
  emits: ["update:open", "update:modelValue", "change", "close", "toggle"],
  setup(props, { slots, emit, attrs }) {
    function emitOpen(next, ev) {
      emit("update:open", next);
      emit("update:modelValue", next);
      emit("change", next, ev);
      emit("toggle", next, ev);
      if (!next) emit("close", ev);
    }

    function onChange(ev) {
      const next =
        typeof ev?.value === "boolean"
          ? ev.value
          : typeof ev === "boolean"
            ? ev
            : !resolveOpen(props);
      emitOpen(!!next, ev);
    }

    return () => {
      const open = resolveOpen(props);
      const label = props.label || props.title;
      const description = props.description || props.hint;
      return h(
        "nana-popover",
        {
          ...attrs,
          class: ["nana-popover", attrs.class].filter(Boolean).join(" "),
          label,
          title: label,
          hint: description,
          description,
          open,
          active: open,
          toggled: open,
          "model-value": open,
          "data-agent-id": attrs["data-agent-id"] || "nana.popover",
          onChange,
          onClose: (ev) => emitOpen(false, ev),
        },
        slots.default?.(),
      );
    };
  },
};

export default NanaPopover;
