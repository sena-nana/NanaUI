/**
 * NanaSettingsCollapsibleCard — Runtime `SettingsCollapsibleCard`.
 */
import { h } from "@vue/runtime-core";

function flatten(nodes) {
  return (Array.isArray(nodes) ? nodes : nodes == null ? [] : [nodes])
    .flat(Infinity)
    .filter(Boolean);
}

function wrapSlot(name, nodes) {
  const children = flatten(nodes);
  if (!children.length) return null;
  return h("div", { "data-slot": name }, children);
}

export const NanaSettingsCollapsibleCard = {
  name: "NanaSettingsCollapsibleCard",
  props: {
    open: { type: Boolean, default: false },
    modelValue: { type: Boolean, default: undefined },
    disabled: { type: Boolean, default: false },
  },
  emits: ["update:open", "update:modelValue"],
  setup(props, { slots, emit, attrs }) {
    return () => {
      const open = props.modelValue !== undefined ? props.modelValue : props.open;
      function onToggle(ev) {
        const next =
          typeof ev?.value === "boolean"
            ? ev.value
            : typeof ev?.open === "boolean"
              ? ev.open
              : !open;
        emit("update:open", next);
        emit("update:modelValue", next);
      }
      const children = [
        wrapSlot("summary", slots.summary?.() || slots.header?.()),
        wrapSlot("details", slots.details?.() || slots.body?.() || slots.default?.()),
        wrapSlot("accessory", slots.accessory?.()),
      ].filter(Boolean);
      return h(
        "nana-settings-collapsible-card",
        {
          ...attrs,
          class: ["nana-settings-collapsible-card", attrs.class]
            .flat()
            .filter(Boolean)
            .join(" "),
          open,
          active: open,
          toggled: open,
          disabled: props.disabled,
          "data-agent-id": attrs["data-agent-id"] || "nana.settings-collapsible-card",
          onChange: onToggle,
          onToggle,
        },
        children,
      );
    };
  },
};

export default NanaSettingsCollapsibleCard;
