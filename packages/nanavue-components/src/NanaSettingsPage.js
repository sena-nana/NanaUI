/**
 * NanaSettingsPage — host tag for Runtime `SettingsPage` (`nana-settings-page`).
 * Resolves active tab from settings model prop / inject("liliaSettings") + tab prop.
 * Header / scroll chrome is assembled by Runtime; this wrapper only hosts content.
 */
import { computed, h, inject } from "@vue/runtime-core";
import { getLiliaSettings } from "./appearance.js";

function flatten(nodes) {
  return (Array.isArray(nodes) ? nodes : nodes == null ? [] : [nodes])
    .flat(Infinity)
    .filter(Boolean);
}

function normalizeTab(model, value) {
  const candidate = Array.isArray(value) ? value[0] : value;
  const raw = typeof candidate === "string" ? candidate : "";
  const aliases = model?.aliases || {};
  const resolved = aliases[raw] ?? raw;
  const tabs = model?.tabs || [];
  if (tabs.some((tab) => tab.key === resolved)) return resolved;
  return model?.defaultTab || "appearance";
}

function resolveView(model, value) {
  const key = normalizeTab(model, value);
  const tab = (model?.tabs || []).find((item) => item.key === key);
  const fullPageTabs = model?.fullPageTabs;
  const fullPage =
    fullPageTabs instanceof Set
      ? fullPageTabs.has(key)
      : Array.isArray(fullPageTabs)
        ? fullPageTabs.includes(key)
        : !!tab?.fullPage;
  return {
    key,
    label: tab?.label || "",
    section: model?.sections?.[key] || null,
    props: model?.sectionProps?.[key] || tab?.props || {},
    fullPage,
  };
}

export const NanaSettingsPage = {
  name: "NanaSettingsPage",
  props: {
    tab: { type: [String, Array], default: undefined },
    settings: { type: Object, default: null },
    contentPadding: { type: [Number, Object], default: undefined },
    contentGap: { type: Number, default: undefined },
    hideHeader: { type: Boolean, default: undefined },
  },
  setup(props, { attrs, slots }) {
    const injected = inject("liliaSettings", null);
    const model = computed(
      () => props.settings || injected || getLiliaSettings() || globalThis.__nanaLiliaSettings,
    );
    const activeView = computed(() => resolveView(model.value, props.tab));

    return () => {
      const view = activeView.value;
      const hideHeader =
        props.hideHeader !== undefined ? !!props.hideHeader : !!model.value?.hideHeader;
      const slotted = flatten(slots.default?.());
      const content = slotted.length
        ? slotted
        : view.section
          ? [h(view.section, { ...view.props })]
          : [h("div", { class: "nana-settings-page__empty" }, "No settings section")];
      return h(
        "nana-settings-page",
        {
          ...attrs,
          class: ["nana-settings-page", "settings-page", attrs.class]
            .flat()
            .filter(Boolean)
            .join(" "),
          settings: props.settings != null ? props.settings : model.value,
          tab: props.tab !== undefined ? props.tab : view.key,
          "hide-header": hideHeader,
          "content-padding": props.contentPadding,
          "content-gap": props.contentGap,
          "data-agent-id":
            attrs["data-agent-id"] ||
            (view.section ? `settings.page.${view.key}` : "nana.settings.page.empty"),
        },
        content,
      );
    };
  },
};

export default NanaSettingsPage;
