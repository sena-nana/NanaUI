/**
 * NanaSettingsCard — semantic peer of Rust `SettingsCard` / Lilia `UiCard` settings title.
 */
import { h } from "@vue/runtime-core";

export const NanaSettingsCard = {
  name: "NanaSettingsCard",
  props: {
    title: { type: String, default: "" },
    agentId: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const agentId =
        props.agentId || attrs["data-agent-id"] || attrs["agent-id"] || "nana.settings-card";
      return h(
        "section",
        {
          ...attrs,
          // Prefer SettingsCard kind via nana-settings-card; avoid ui-card → Card.
          title: props.title,
          label: props.title,
          class: ["nana-settings-card", "lilia-card", attrs.class]
            .filter(Boolean)
            .join(" "),
          "data-agent-id": agentId,
        },
        [
          props.title
            ? h("div", { class: "nana-settings-card__title" }, props.title)
            : null,
          h("div", { class: "nana-settings-card__body" }, slots.default?.()),
        ],
      );
    };
  },
};

export default NanaSettingsCard;
