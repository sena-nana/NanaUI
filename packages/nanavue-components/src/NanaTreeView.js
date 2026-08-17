/**
 * NanaTreeView — flattened disclosure tree.
 * Semantic peer of Runtime `TreeView` (`nana-tree-view`).
 *
 * `options` / `nodes` are nested application-owned identities.
 */
import { h } from "@vue/runtime-core";

function normalizeNodes(nodes) {
  return (nodes || []).map((node) => {
    const value = node.value ?? node.id ?? node.key ?? node.label;
    return {
      value,
      id: value,
      label: node.label ?? String(value ?? ""),
      children: normalizeNodes(node.children),
      expanded: !!node.expanded,
      selected: !!node.selected,
      disabled: !!node.disabled,
      icon: node.icon,
    };
  });
}

export const NanaTreeView = {
  name: "NanaTreeView",
  props: {
    options: { type: Array, default: undefined },
    nodes: { type: Array, default: undefined },
    size: { type: String, default: "small" },
  },
  emits: ["select", "toggle", "update:modelValue"],
  setup(props, { emit, attrs }) {
    function onSelect(ev) {
      const value = ev?.value ?? ev?.id ?? ev;
      emit("select", value, ev);
      emit("update:modelValue", value);
    }

    function onToggle(ev) {
      emit("toggle", ev?.value ?? ev?.id ?? ev, ev);
    }

    return () => {
      const nodes = normalizeNodes(props.nodes ?? props.options ?? []);
      return h("nana-tree-view", {
        ...attrs,
        class: ["nana-tree-view", attrs.class].filter(Boolean).join(" "),
        role: attrs.role || "list",
        options: nodes,
        nodes,
        size: props.size,
        "data-agent-id": attrs["data-agent-id"] || "nana.tree-view",
        onSelect,
        onToggle,
      });
    };
  },
};

export default NanaTreeView;
