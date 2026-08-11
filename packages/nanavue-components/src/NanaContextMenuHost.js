/**
 * NanaContextMenuHost — binds Lilia `useContextMenu` store → NanaContextMenu.
 * Replaces ContextMenuHost.vue (DOM teleport + CSS fixed) for Nana iced Overlay.
 */
import { computed, h, watch } from "@vue/runtime-core";
import { NanaContextMenu } from "./NanaContextMenu.js";

function flattenItems(items, prefix = "") {
  const out = [];
  for (const item of items || []) {
    const id = item.id ?? item.label;
    const value = prefix ? `${prefix}/${id}` : String(id);
    out.push({
      value,
      label: item.label ?? String(id),
      disabled: !!item.disabled,
    });
    if (item.children?.length) {
      out.push(...flattenItems(item.children, value));
    }
  }
  return out;
}

/**
 * @param {object} options
 * @param {() => { open: boolean, x: number, y: number, items: unknown[], searchable?: boolean }} options.useState
 * @param {() => void} [options.close]
 */
export function createNanaContextMenuHost(options) {
  const useState = options.useState;
  const close = options.close || (() => {});

  return {
    name: "NanaContextMenuHost",
    setup(_props, { attrs }) {
      const state = useState();
      const open = computed(() => !!state.open && (state.items?.length ?? 0) > 0);
      const menuOptions = computed(() => flattenItems(state.items || []));
      const searchable = computed(
        () => !!state.searchable || (state.items?.length ?? 0) >= 6,
      );

      watch(open, (next) => {
        if (!next) close();
      });

      return () =>
        h(NanaContextMenu, {
          ...attrs,
          class: ["nana-context-menu-host", "ctx-menu", attrs.class]
            .filter(Boolean)
            .join(" "),
          open: open.value,
          options: menuOptions.value,
          search: searchable.value,
          anchorX: state.x ?? state.anchorX ?? 96,
          anchorY: state.y ?? state.anchorY ?? 96,
          "data-agent-id": attrs["data-agent-id"] || "nana.context-menu-host",
          "onUpdate:open": (next) => {
            if (!next) close();
          },
          onClose: () => close(),
          onSelect: (value) => {
            const flat = flattenItems(state.items || []);
            const match = flat.find((o) => o.value === value);
            const leafId = match?.value?.split("/").pop();
            const find = (items) => {
              for (const item of items || []) {
                const id = String(item.id ?? item.label);
                if (id === leafId || item.label === value) return item;
                const child = find(item.children);
                if (child) return child;
              }
              return null;
            };
            const item = find(state.items);
            if (item?.onSelect) {
              void Promise.resolve(item.onSelect()).finally(() => close());
            } else {
              close();
            }
          },
        });
    },
  };
}

export const NanaContextMenuHost = {
  name: "NanaContextMenuHost",
  props: {
    /** Injected host state reader — set by installLiliaContextMenu. */
    state: { type: Object, default: null },
  },
  setup(props, { attrs }) {
    const fallback = { open: false, x: 96, y: 96, items: [], searchable: false };
    return () => {
      const state = props.state || globalThis.__nanaContextMenuState || fallback;
      const open = !!state.open && (state.items?.length ?? 0) > 0;
      return h(NanaContextMenu, {
        ...attrs,
        class: ["nana-context-menu-host", "ctx-menu", attrs.class]
          .filter(Boolean)
          .join(" "),
        open,
        options: flattenItems(state.items || []),
        search: !!state.searchable || (state.items?.length ?? 0) >= 6,
        anchorX: state.x ?? state.anchorX ?? 96,
        anchorY: state.y ?? state.anchorY ?? 96,
        "data-agent-id": attrs["data-agent-id"] || "nana.context-menu-host",
        "onUpdate:open": (next) => {
          if (!next) globalThis.__nanaContextMenu?.close?.();
        },
        onClose: () => globalThis.__nanaContextMenu?.close?.(),
        onSelect: (value) => {
          globalThis.__nanaContextMenu?.select?.(value);
        },
      });
    };
  },
};

export default NanaContextMenuHost;
