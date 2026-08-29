/**
 * NanaButton — **L2** semantic peer of NanaUI `Button` (no DOM/CSS paint).
 *
 * Props (`kind` / `size` / …) map to Style Model Semantics; skips CSS parsing.
 * Renders an HTML `button` host node so Rust `MessageBridge` mirrors props and
 * the Scene host draws the real control. Mixable with L1 `createElement` in the same tree.
 */
import { computed, h } from "@vue/runtime-core";

export const NanaButton = {
  name: "NanaButton",
  props: {
    kind: { type: String, default: "ghost" },
    size: { type: String, default: "medium" },
    disabled: { type: Boolean, default: false },
    loading: { type: Boolean, default: false },
    label: { type: String, default: "" },
  },
  emits: ["press"],
  setup(props, { slots, emit, attrs }) {
    const resolvedLabel = computed(() => {
      if (props.label) return props.label;
      const slot = slots.default?.();
      if (!slot || !slot.length) return "";
      // Flatten text children when callers pass a string slot.
      return slot
        .map((vnode) => (typeof vnode.children === "string" ? vnode.children : ""))
        .join("");
    });

    function onPress(ev) {
      if (props.disabled || props.loading) {
        ev?.preventDefault?.();
        return;
      }
      emit("press", ev);
    }

    return () =>
      h("button", {
        ...attrs,
        kind: props.kind,
        size: props.size,
        disabled: props.disabled,
        loading: props.loading,
        label: resolvedLabel.value,
        "data-agent-id": attrs["data-agent-id"] || "nana.button",
        "data-kind": props.kind,
        "data-size": props.size,
        onPress,
        onClick: onPress,
      });
  },
};

export default NanaButton;
