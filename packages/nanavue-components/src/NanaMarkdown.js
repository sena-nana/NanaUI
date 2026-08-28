/**
 * NanaMarkdown — native markdown source host.
 * Semantic peer of Runtime `NativeMarkdown` (`nana-native-markdown`).
 *
 * Vue supplies the source string. Parsing stays on the host.
 * Optional `mermaidRenderer` / `mathRenderer` names are identity only.
 */
import { h } from "@vue/runtime-core";

export const NanaMarkdown = {
  name: "NanaMarkdown",
  props: {
    value: { type: String, default: undefined },
    modelValue: { type: String, default: undefined },
    mermaidRenderer: { type: String, default: undefined },
    mathRenderer: { type: String, default: undefined },
  },
  emits: ["update:modelValue"],
  setup(props, { emit, attrs }) {
    function onInput(ev) {
      const next = ev?.value ?? ev;
      if (next === undefined || next === null) return;
      emit("update:modelValue", String(next));
    }

    return () => {
      const source =
        props.modelValue !== undefined && props.modelValue !== null
          ? props.modelValue
          : props.value ?? "";
      return h("nana-native-markdown", {
        ...attrs,
        class: ["nana-markdown", attrs.class].filter(Boolean).join(" "),
        value: source,
        "model-value": source,
        mermaidRenderer: props.mermaidRenderer,
        "mermaid-renderer": props.mermaidRenderer,
        mathRenderer: props.mathRenderer,
        "math-renderer": props.mathRenderer,
        "data-agent-id": attrs["data-agent-id"] || "nana.markdown",
        onInput,
        onChange: onInput,
      });
    };
  },
};

export default NanaMarkdown;
