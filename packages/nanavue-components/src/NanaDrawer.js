/**
 * NanaDrawer — side sheet overlay.
 * Semantic peer of Rust `Drawer` (`nana-drawer`, `side=left|right`).
 *
 * Footer slot children are tagged `nana-drawer-footer` so iced partitions them.
 * Footer buttons with confirm/cancel class tokens close / SelectValue the drawer.
 */
import { h } from "@vue/runtime-core";

function resolveOpen(props) {
  if (typeof props.open === "boolean") return props.open;
  if (typeof props.modelValue === "boolean") return props.modelValue;
  return false;
}

export const NanaDrawerFooter = {
  name: "NanaDrawerFooter",
  setup(_props, { slots, attrs }) {
    return () =>
      h(
        "nana-row",
        {
          ...attrs,
          class: ["nana-drawer-footer", attrs.class].filter(Boolean).join(" "),
          role: "contentinfo",
          "data-slot": "drawer-footer",
          "data-agent-id": attrs["data-agent-id"] || "nana.drawer.footer",
        },
        slots.default?.(),
      );
  },
};

export const NanaDrawer = {
  name: "NanaDrawer",
  props: {
    open: { type: Boolean, default: undefined },
    modelValue: { type: [Boolean, String], default: undefined },
    title: { type: String, default: "" },
    label: { type: String, default: "" },
    description: { type: String, default: "" },
    hint: { type: String, default: "" },
    /** `left` | `right` (default right). */
    side: { type: String, default: "right" },
    /** Drawer width in px (≥240). */
    width: { type: [Number, String], default: undefined },
  },
  emits: ["update:open", "update:modelValue", "change", "close", "confirm", "select", "press"],
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
      const children = [];
      if (slots.default) children.push(...(slots.default() || []));
      if (slots.footer) {
        children.push(
          h(
            NanaDrawerFooter,
            { "data-agent-id": attrs["data-agent-id"] ? `${attrs["data-agent-id"]}.footer` : undefined },
            { default: () => slots.footer() },
          ),
        );
      }

      const style =
        props.width != null
          ? {
              ...(typeof attrs.style === "object" && attrs.style ? attrs.style : {}),
              width: typeof props.width === "number" ? `${props.width}px` : String(props.width),
            }
          : attrs.style;

      return h(
        "nana-drawer",
        {
          ...attrs,
          class: ["nana-drawer", attrs.class].filter(Boolean).join(" "),
          style,
          label: title,
          title,
          hint: description,
          description,
          side: props.side,
          open,
          active: open,
          toggled: open,
          "model-value": typeof props.modelValue === "boolean" ? props.modelValue : open,
          "data-agent-id": attrs["data-agent-id"] || "nana.drawer",
          onChange,
          onSelect,
          onPress: (ev) => emit("press", ev),
          onClose: (ev) => emitOpen(false, ev),
        },
        children,
      );
    };
  },
};

export default NanaDrawer;
