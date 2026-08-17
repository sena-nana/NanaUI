/**
 * NanaTextarea — multi-line text field.
 * Semantic peer of Rust `Textarea` (`nana-textarea` → Runtime Textarea / EditorStore).
 */
import { h } from "@vue/runtime-core";

export const NanaTextarea = {
  name: "NanaTextarea",
  props: {
    modelValue: { type: String, default: "" },
    placeholder: { type: String, default: "" },
    disabled: { type: Boolean, default: false },
    invalid: { type: Boolean, default: false },
    size: { type: String, default: "medium" },
    /** Optional fixed height in px (maps to layout height). */
    height: { type: [Number, String], default: undefined },
    /** Runtime syntax language (`rs`, `js`, …). Empty keeps solid committed text. */
    language: { type: String, default: "" },
  },
  emits: ["update:modelValue", "input"],
  setup(props, { emit, attrs }) {
    return () => {
      const style =
        props.height != null
          ? {
              ...(typeof attrs.style === "object" && attrs.style ? attrs.style : {}),
              height:
                typeof props.height === "number" ? `${props.height}px` : String(props.height),
            }
          : attrs.style;
      return h("nana-textarea", {
        ...attrs,
        style,
        value: props.modelValue,
        "model-value": props.modelValue,
        placeholder: props.placeholder,
        disabled: props.disabled,
        invalid: props.invalid,
        size: props.size,
        language: props.language || undefined,
        "data-agent-id": attrs["data-agent-id"] || "nana.textarea",
        onInput: (ev) => {
          const value = ev?.value ?? ev ?? "";
          emit("update:modelValue", String(value));
          emit("input", String(value), ev);
        },
      });
    };
  },
};

export default NanaTextarea;
