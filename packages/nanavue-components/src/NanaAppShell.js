/**
 * NanaAppShell — host tag for Runtime `AppShell` (`nana-app-shell`).
 * `title` becomes an `nana-app-title-bar` child when none is supplied.
 * Default slot is the body; an overlay child may use `data-slot="overlay"`.
 */
import { h } from "@vue/runtime-core";

function flatten(nodes) {
  return (Array.isArray(nodes) ? nodes : nodes == null ? [] : [nodes])
    .flat(Infinity)
    .filter(Boolean);
}

function isTitleBarNode(node) {
  if (!node || typeof node !== "object") return false;
  const type = node.type;
  const props = node.props || {};
  const tag = typeof type === "string" ? type : String(type?.name || "");
  const slot = props["data-slot"] || props.dataSlot;
  const cls = String(props.class || "");
  return (
    slot === "title-bar" ||
    slot === "titlebar" ||
    cls.includes("nana-app-title-bar") ||
    /title-bar|titlebar/i.test(tag)
  );
}

export const NanaAppShell = {
  name: "NanaAppShell",
  props: {
    title: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const slotted = flatten(slots.default?.());
      const children = [];
      if (props.title && !slotted.some(isTitleBarNode)) {
        children.push(
          h(
            "nana-app-title-bar",
            {
              class: "nana-app-title-bar",
              title: props.title,
              "data-slot": "title-bar",
            },
            props.title,
          ),
        );
      }
      children.push(...slotted);
      return h(
        "nana-app-shell",
        {
          ...attrs,
          class: ["nana-app-shell", attrs.class].flat().filter(Boolean).join(" "),
          title: props.title,
          "data-agent-id": attrs["data-agent-id"] || "nana.app-shell",
        },
        children,
      );
    };
  },
};

export default NanaAppShell;
