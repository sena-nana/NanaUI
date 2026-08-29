/**
 * NanaDialog — modal overlay.
 * Semantic peer of Rust `Dialog` / `ConfirmDialog` (`<dialog>`).
 *
 * Open via `open` or boolean `modelValue` (`active`/`open`/`toggled` on host).
 * Set `role="alertdialog"` (or class `confirm`) for ConfirmDialog; `kind="danger"`
 * for destructive confirm.
 */
import { h } from "@vue/runtime-core";

function resolveOpen(props) {
  if (typeof props.open === "boolean") return props.open;
  if (typeof props.modelValue === "boolean") return props.modelValue;
  return false;
}

export const NanaDialog = {
  name: "NanaDialog",
  props: {
    open: { type: Boolean, default: undefined },
    /** Boolean open state when `open` is omitted; string reserved for confirm value. */
    modelValue: { type: [Boolean, String], default: undefined },
    title: { type: String, default: "" },
    label: { type: String, default: "" },
    description: { type: String, default: "" },
    hint: { type: String, default: "" },
    /** `dialog` (default) or `alertdialog` → ConfirmDialog. */
    role: { type: String, default: "dialog" },
    /** Button/confirm kind; `danger` marks ConfirmDialog as destructive. */
    kind: { type: String, default: "" },
    confirm: { type: Boolean, default: false },
  },
  emits: ["update:open", "update:modelValue", "change", "close", "confirm", "select"],
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
      emit("confirm", value, ev);
      if (typeof props.modelValue === "string" || props.modelValue === undefined) {
        emit("update:modelValue", value);
      }
      emitOpen(false, ev);
    }

    return () => {
      const open = resolveOpen(props);
      const title = props.title || props.label;
      const description = props.description || props.hint;
      const role =
        props.confirm || props.role === "alertdialog" ? "alertdialog" : props.role || "dialog";
      const classNames = [
        "nana-dialog",
        props.confirm || role === "alertdialog" ? "nana-confirm-dialog" : "",
        attrs.class,
      ]
        .filter(Boolean)
        .join(" ");

      return h(
        "dialog",
        {
          ...attrs,
          class: classNames,
          role,
          label: title,
          title,
          hint: description,
          description,
          kind: props.kind || undefined,
          "data-variant":
            props.kind === "danger"
              ? "danger"
              : props.confirm || role === "alertdialog"
                ? "confirm"
                : attrs["data-variant"],
          open,
          active: open,
          toggled: open,
          "model-value": typeof props.modelValue === "boolean" ? props.modelValue : open,
          "data-agent-id": attrs["data-agent-id"] || "nana.dialog",
          onChange,
          onSelect,
          onClose: (ev) => emitOpen(false, ev),
        },
        slots.default?.(),
      );
    };
  },
};

export default NanaDialog;
