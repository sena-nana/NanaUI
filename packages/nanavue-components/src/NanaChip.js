/**
 * NanaChip — compact selectable chip (Button Selected/Subtle variant).
 * Semantic peer of NanaUI chip styling via `nana-chip` → Iced Button.
 */
import { computed, h } from "@vue/runtime-core";

export const NanaChip = {
  name: "NanaChip",
  props: {
    selected: { type: Boolean, default: false },
    disabled: { type: Boolean, default: false },
    label: { type: String, default: "" },
  },
  emits: ["select"],
  setup(props, { slots, emit, attrs }) {
    const resolvedLabel = computed(() => {
      if (props.label) return props.label;
      const slot = slots.default?.();
      if (!slot || !slot.length) return "";
      return slot
        .map((vnode) => (typeof vnode.children === "string" ? vnode.children : ""))
        .join("");
    });
    function onSelect(ev) {
      if (props.disabled) {
        ev?.preventDefault?.();
        return;
      }
      emit("select", ev);
    }
    return () =>
      h("nana-chip", {
        ...attrs,
        class: [
          "nana-chip",
          props.selected ? "is-selected" : "",
          props.disabled ? "is-disabled" : "",
          attrs.class,
        ]
          .filter(Boolean)
          .join(" "),
        label: resolvedLabel.value,
        selected: props.selected,
        active: props.selected,
        disabled: props.disabled,
        "aria-pressed": props.selected ? "true" : "false",
        "data-agent-id": attrs["data-agent-id"] || "nana.chip",
        onSelect,
        onClick: onSelect,
        onPress: onSelect,
      });
  },
};

export default NanaChip;
