/**
 * NanaContextMenu — anchored context / action menu.
 * Semantic peer of Rust `ContextMenu` (`nana-context-menu` → MenuStore / AnchoredActionMenu).
 *
 * Nested items use `parent/child` option values. Search enables when ≥6 options
 * or when `search` is true / class contains `search`.
 */
import { h } from "@vue/runtime-core";

function resolveOpen(props) {
  if (typeof props.open === "boolean") return props.open;
  if (typeof props.modelValue === "boolean") return props.modelValue;
  return false;
}

function normalizeOptions(options) {
  return (options || []).map((option) => ({
    value: option.value ?? option.key ?? option.label,
    label: option.label ?? String(option.value ?? option.key ?? ""),
    disabled: !!option.disabled,
  }));
}

export const NanaContextMenu = {
  name: "NanaContextMenu",
  props: {
    open: { type: Boolean, default: undefined },
    modelValue: { type: [Boolean, String], default: undefined },
    options: { type: Array, default: () => [] },
    label: { type: String, default: "" },
    /** Anchor X in logical px. */
    anchorX: { type: Number, default: 96 },
    /** Anchor Y in logical px. */
    anchorY: { type: Number, default: 96 },
    /** Force searchable menu (also auto when options ≥ 6). */
    search: { type: Boolean, default: false },
  },
  emits: ["update:open", "update:modelValue", "change", "close", "select"],
  setup(props, { slots, emit, attrs }) {
    function emitOpen(next, ev) {
      emit("update:open", next);
      if (typeof props.modelValue !== "string") {
        emit("update:modelValue", next);
      }
      emit("change", next, ev);
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

    function onSelect(ev) {
      const value = ev?.value ?? ev;
      emit("select", value, ev);
      if (typeof props.modelValue === "string" || props.modelValue === undefined) {
        emit("update:modelValue", value);
      }
      emitOpen(false, ev);
    }

    return () => {
      const open = resolveOpen(props);
      const classNames = [
        "nana-context-menu",
        props.search ? "search" : "",
        attrs.class,
      ]
        .filter(Boolean)
        .join(" ");

      return h(
        "nana-context-menu",
        {
          ...attrs,
          class: classNames,
          role: "menu",
          label: props.label,
          options: normalizeOptions(props.options),
          open,
          active: open,
          toggled: open,
          "model-value": typeof props.modelValue === "boolean" ? props.modelValue : open,
          "anchor-x": props.anchorX,
          "anchor-y": props.anchorY,
          "data-anchor-x": props.anchorX,
          "data-anchor-y": props.anchorY,
          "data-agent-id": attrs["data-agent-id"] || "nana.context-menu",
          onChange,
          onSelect,
          onClose: (ev) => emitOpen(false, ev),
        },
        slots.default?.(),
      );
    };
  },
};

export default NanaContextMenu;
