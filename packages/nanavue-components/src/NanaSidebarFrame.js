/**
 * NanaSidebarFrame — semantic peer of Rust `SidebarFrame` (no DOM paint).
 * Children (top / body / footer) are inserted as semantic descendants when present.
 *
 * Region contract: do **not** auto-stamp `data-region`. Nested frames inside an
 * outer workspace region (e.g. LiliaResourcePanel) must stay in-tree; only an
 * explicit `data-region` / `region` attr (or NanaWorkspaceShell aside) opts into
 * DesktopShell Navigation extraction.
 *
 * Layout contract: always stamp `nana-sidebar-frame` (+ slot BEM classes) so
 * stylesheet / class layout hints apply. Do not rely on WidgetKind alone.
 */
import { h } from "@vue/runtime-core";

function mergeClass(...parts) {
  return parts
    .flatMap((p) => {
      if (!p) return [];
      if (Array.isArray(p)) return p;
      if (typeof p === "string") return p.split(/\s+/);
      return [];
    })
    .filter(Boolean)
    .filter((c, i, arr) => arr.indexOf(c) === i)
    .join(" ");
}

export const NanaSidebarFrame = {
  name: "NanaSidebarFrame",
  props: {
    agentId: { type: String, default: "nana.sidebar-frame" },
    ariaLabel: { type: String, default: undefined },
    /** When true and no footer slot, still reserve footer region (Lilia default). */
    defaultFooter: { type: Boolean, default: false },
    /**
     * Optional DesktopShell region tag. Empty by default so nested sidebars do
     * not leave an empty outer shell track in Primary.
     */
    region: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const hasFooter = !!slots.footer || props.defaultFooter;
      const children = [];
      if (slots.top) {
        children.push(
          h(
            "nana-column",
            {
              class: "nana-sidebar-frame__top",
              "data-slot": "sidebar-top",
            },
            slots.top(),
          ),
        );
      }
      children.push(
        h(
          "nana-column",
          {
            class: "nana-sidebar-frame__body",
            "data-slot": "sidebar-body",
          },
          slots.body?.() || slots.default?.(),
        ),
      );
      if (hasFooter) {
        children.push(
          h(
            "nana-column",
            {
              class: "nana-sidebar-frame__footer",
              "data-slot": "sidebar-footer",
            },
            slots.footer?.() || [],
          ),
        );
      }
      const region =
        props.region ||
        attrs["data-region"] ||
        attrs.region ||
        "";
      const frameAttrs = {
        ...attrs,
        class: mergeClass("nana-sidebar-frame", attrs.class),
        "aria-label": props.ariaLabel,
        "data-agent-id":
          props.agentId || attrs["data-agent-id"] || "nana.sidebar-frame",
      };
      if (region) {
        frameAttrs["data-region"] = region;
      }
      return h("nana-sidebar-frame", frameAttrs, children);
    };
  },
};

export default NanaSidebarFrame;
