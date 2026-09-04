/**
 * NanaChip — compact selectable token. Semantic peer of Runtime `Chip`
 * (`nana-chip`).
 */
import { computed, h } from "@vue/runtime-core";

export const NanaChip = {
  name: "NanaChip",
  props: {
    selected: { type: Boolean, default: false },
    disabled: { type: Boolean, default: false },
    dismissible: { type: Boolean, default: false },
    label: { type: String, default: "" },
  },
  emits: ["select", "dismiss"],
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
    function onDismiss(ev) {
      if (props.disabled) {
        ev?.preventDefault?.();
        return;
      }
      emit("dismiss", ev);
    }
    return () =>
      h("nana-chip", {
        ...attrs,
        class: [
          "nana-chip",
          props.selected ? "is-selected" : "",
          props.disabled ? "is-disabled" : "",
          props.dismissible ? "is-dismissible" : "",
          attrs.class,
        ]
          .filter(Boolean)
          .join(" "),
        label: resolvedLabel.value,
        selected: props.selected,
        active: props.selected,
        disabled: props.disabled,
        dismissible: props.dismissible,
        "aria-pressed": props.selected ? "true" : "false",
        "data-agent-id": attrs["data-agent-id"] || "nana.chip",
        onSelect,
        onClick: onSelect,
        onPress: onSelect,
        onDismiss,
      });
  },
};

export default NanaChip;
