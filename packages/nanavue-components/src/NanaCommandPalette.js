/**
 * NanaCommandPalette — modal command list.
 * Semantic peer of Runtime `CommandPalette` (`nana-command-palette`).
 *
 * Open via `open` / boolean `modelValue`. `options` are action rows.
 */
import { h } from "@vue/runtime-core";

function resolveOpen(props) {
  if (typeof props.open === "boolean") return props.open;
  if (typeof props.modelValue === "boolean") return props.modelValue;
  return false;
}

function normalizeOptions(options) {
  return (options || []).map((option) => {
    const value = option.value ?? option.action ?? option.key ?? option.label;
    return {
      value,
      action: value,
      label: option.label ?? String(value ?? ""),
      category: option.category || undefined,
      shortcut: option.shortcut || undefined,
      disabled: !!option.disabled,
    };
  });
}

export const NanaCommandPalette = {
  name: "NanaCommandPalette",
  props: {
    open: { type: Boolean, default: undefined },
    modelValue: { type: Boolean, default: undefined },
    title: { type: String, default: "" },
    placeholder: { type: String, default: "" },
    query: { type: String, default: "" },
    options: { type: Array, default: () => [] },
  },
  emits: ["update:open", "update:modelValue", "select", "change"],
  setup(props, { emit, attrs }) {
    function emitOpen(next, ev) {
      emit("update:open", next);
      emit("update:modelValue", next);
      if (typeof ev?.value === "boolean" || typeof ev === "boolean") {
        emit("change", next, ev);
      }
    }

    function onSelect(ev) {
      const value = ev?.value ?? ev?.action ?? ev;
      emit("select", value, ev);
      emit("change", value, ev);
    }

    function onChange(ev) {
      if (typeof ev?.value === "boolean" || typeof ev === "boolean") {
        emitOpen(!!(typeof ev?.value === "boolean" ? ev.value : ev), ev);
        return;
      }
      onSelect(ev);
    }

    return () => {
      const open = resolveOpen(props);
      const options = normalizeOptions(props.options);
      return h("nana-command-palette", {
        ...attrs,
        class: ["nana-command-palette", attrs.class].filter(Boolean).join(" "),
        role: attrs.role || "dialog",
        title: props.title,
        label: props.title,
        placeholder: props.placeholder,
        query: props.query,
        options,
        items: options,
        open,
        active: open,
        toggled: open,
        "model-value": open,
        "data-agent-id": attrs["data-agent-id"] || "nana.command-palette",
        onSelect,
        onChange,
        onClose: (ev) => emitOpen(false, ev),
      });
    };
  },
};

export default NanaCommandPalette;
