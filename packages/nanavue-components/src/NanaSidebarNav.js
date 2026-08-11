/**
 * NanaSidebarNav — workspace region navigation items.
 * Semantic peer of GlobalNavigation / SectionNavigation region rows.
 *
 * Region contract: do **not** auto-stamp `data-region`. Callers that want
 * DesktopShell SectionNavigation extraction must pass `data-region` /
 * `region` explicitly (e.g. standalone nav shells). Nested tab lists inside
 * an outer sidebar must stay in Primary with their parent shell.
 */
import { h } from "@vue/runtime-core";

export const NanaSidebarNav = {
  name: "NanaSidebarNav",
  props: {
    items: {
      type: Array,
      default: () => [],
    },
    activeKey: { type: String, default: "" },
    region: { type: String, default: "" },
  },
  emits: ["select"],
  setup(props, { emit, attrs, slots }) {
    return () => {
      const region =
        props.region ||
        attrs["data-region"] ||
        attrs.region ||
        "";
      const navAttrs = {
        ...attrs,
        class: ["nana-sidebar-nav", attrs.class].filter(Boolean).join(" "),
        "aria-label": attrs["aria-label"] || "Workspace navigation",
        "data-agent-id": attrs["data-agent-id"] || "nana.sidebar-nav",
      };
      if (region) {
        navAttrs["data-region"] = region;
      }
      return h(
        "nav",
        navAttrs,
        props.items.length
          ? props.items.map((item) => {
              const key = String(item.key ?? item.id ?? item.label ?? "");
              const active = key && key === props.activeKey;
              return h(
                "button",
                {
                  type: "button",
                  class: [
                    "nana-sidebar-nav__item",
                    active ? "is-active" : "",
                  ]
                    .filter(Boolean)
                    .join(" "),
                  "data-agent-id": item.agentId || `nana.nav.${key}`,
                  "aria-current": active ? "page" : undefined,
                  onClick: (ev) => emit("select", item, ev),
                },
                item.label || key,
              );
            })
          : slots.default?.(),
      );
    };
  },
};

export default NanaSidebarNav;
