/**
 * NanaSidebarRow — semantic peer of Rust `SidebarRow` (no DOM paint).
 */
import { h } from "@vue/runtime-core";

export const NanaSidebarRow = {
  name: "NanaSidebarRow",
  props: {
    label: { type: String, default: "" },
    active: { type: Boolean, default: false },
    muted: { type: Boolean, default: false },
    disabled: { type: Boolean, default: false },
    agentId: { type: String, default: "" },
  },
  emits: ["select"],
  setup(props, { slots, emit, attrs }) {
    function onSelect(ev) {
      if (props.disabled) {
        ev?.preventDefault?.();
        return;
      }
      emit("select", ev);
    }

    return () => {
      const label =
        props.label ||
        (typeof slots.default === "function"
          ? slots
              .default()
              .map((vnode) => (typeof vnode.children === "string" ? vnode.children : ""))
              .join("")
          : "");
      return h("nana-sidebar-row", {
        ...attrs,
        class: ["nana-sidebar-row", attrs.class].flat().filter(Boolean).join(" "),
        label,
        active: props.active,
        muted: props.muted,
        disabled: props.disabled,
        "data-agent-id":
          props.agentId || attrs["data-agent-id"] || "nana.sidebar-row",
        "data-lilia-selected": props.active ? "true" : undefined,
        onSelect,
        onClick: onSelect,
        onPress: onSelect,
      });
    };
  },
};

export default NanaSidebarRow;
