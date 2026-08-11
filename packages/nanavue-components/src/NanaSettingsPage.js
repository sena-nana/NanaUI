/**
 * NanaSettingsPage — settings content container (peer of Lilia `SettingsPage`).
 * Resolves active tab from settings model prop / inject("liliaSettings") + tab prop.
 */
import { computed, h, inject } from "@vue/runtime-core";
import { getLiliaSettings } from "./appearance.js";

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
        : false;
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
  },
  setup(props, { attrs }) {
    const injected = inject("liliaSettings", null);
    const model = computed(
      () => props.settings || injected || getLiliaSettings() || globalThis.__nanaLiliaSettings,
    );
    const activeView = computed(() => resolveView(model.value, props.tab));

    return () => {
      const view = activeView.value;
      if (!view.section) {
        return h(
          "section",
          {
            class: "nana-settings-page settings-page",
            "data-agent-id": attrs["data-agent-id"] || "nana.settings.page.empty",
          },
          [h("div", { class: "nana-settings-page__empty" }, "No settings section")],
        );
      }
      if (view.fullPage) {
        return h(view.section, {
          ...view.props,
          "data-agent-id": "settings.full-page-section",
        });
      }
      const hideHeader = !!model.value?.hideHeader;
      return h(
        "section",
        {
          ...attrs,
          class: ["nana-settings-page", "settings-page", attrs.class]
            .filter(Boolean)
            .join(" "),
          "data-agent-id":
            attrs["data-agent-id"] || `settings.page.${view.key}`,
        },
        [
          !hideHeader
            ? h("div", { class: "nana-settings-page__header page-header" }, [
                h("h1", null, view.label),
              ])
            : null,
          h(view.section, { ...view.props }),
        ],
      );
    };
  },
};

export default NanaSettingsPage;
