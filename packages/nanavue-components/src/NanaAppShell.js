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

function nodeSlot(node) {
  const props = (node && node.props) || {};
  return String(props["data-slot"] || props.dataSlot || "");
}

function isTitleBarNode(node) {
  if (!node || typeof node !== "object") return false;
  const type = node.type;
  const props = node.props || {};
  const tag = typeof type === "string" ? type : String(type?.name || "");
  const slot = nodeSlot(node);
  const cls = String(props.class || "");
  return (
    slot === "title-bar" ||
    slot === "titlebar" ||
    cls.includes("nana-app-title-bar") ||
    /title-bar|titlebar/i.test(tag)
  );
}

function isOverlayNode(node) {
  if (!node || typeof node !== "object") return false;
  const slot = nodeSlot(node);
  const cls = String((node.props || {}).class || "");
  return slot === "overlay" || cls.includes("nana-app-shell__overlay");
}

function isBodyNode(node) {
  return nodeSlot(node) === "body";
}

export const NanaAppShell = {
  name: "NanaAppShell",
  props: {
    title: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const slotted = flatten(slots.default?.());
      const titleBars = slotted.filter(isTitleBarNode);
      const overlays = slotted.filter(isOverlayNode);
      const bodyNodes = slotted.filter((node) => !isTitleBarNode(node) && !isOverlayNode(node));
      const children = [];
      if (props.title && titleBars.length === 0) {
        children.push(
          h("nana-app-title-bar", {
            class: "nana-app-title-bar",
            title: props.title,
            "data-slot": "title-bar",
          }),
        );
      }
      children.push(...titleBars);
      if (bodyNodes.length === 1 && isBodyNode(bodyNodes[0])) {
        children.push(bodyNodes[0]);
      } else if (bodyNodes.length) {
        children.push(
          h(
            "div",
            {
              class: "nana-app-shell__body",
              "data-slot": "body",
            },
            bodyNodes,
          ),
        );
      }
      children.push(...overlays);
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
