/**
 * NanaSettingsRow — semantic peer of Rust `SettingsRow` / Lilia `SettingsRow`.
 */
import { h } from "@vue/runtime-core";

export const NanaSettingsRow = {
  name: "NanaSettingsRow",
  props: {
    label: { type: String, default: "" },
    hint: { type: String, default: "" },
    stacked: { type: Boolean, default: false },
    divided: { type: Boolean, default: true },
    loose: { type: Boolean, default: false },
    firstInGroup: { type: Boolean, default: false },
    lastInGroup: { type: Boolean, default: false },
    agentId: { type: String, default: "" },
  },
  setup(props, { slots, attrs }) {
    return () => {
      const agentId =
        props.agentId || attrs["data-agent-id"] || attrs["agent-id"] || "nana.settings-row";
      const hasLabel = !!(props.label || props.hint || slots.label || slots.hint);
      return h(
        "div",
        {
          ...attrs,
          // Forward semantic props so the Rust bridge can drive SettingsRow::view
          // without scraping nested __label text nodes.
          label: props.label,
          hint: props.hint,
          stacked: props.stacked,
          divided: props.divided,
          loose: props.loose,
          "first-in-group": props.firstInGroup,
          "last-in-group": props.lastInGroup,
          class: [
            "nana-settings-row",
            "settings-row",
            props.stacked ? "nana-settings-row--stacked settings-row--stacked" : "",
            props.divided ? "nana-settings-row--divided settings-row--divided" : "",
            props.firstInGroup ? "is-first" : "",
            props.lastInGroup ? "is-last" : "",
            attrs.class,
          ]
            .filter(Boolean)
            .join(" "),
          "data-agent-id": agentId,
        },
        [
          hasLabel
            ? h("div", { class: "nana-settings-row__label settings-row__label" }, [
                slots.label?.() || props.label,
                props.hint || slots.hint
                  ? h(
                      "div",
                      { class: "nana-settings-row__hint settings-row__hint" },
                      slots.hint?.() || props.hint,
                    )
                  : null,
              ])
            : null,
          h(
            "div",
            {
              class: [
                "nana-settings-row__control",
                "settings-row__control",
                props.loose ? "nana-settings-row__control--loose settings-row__control--loose" : "",
              ]
                .filter(Boolean)
                .join(" "),
            },
            slots.default?.(),
          ),
        ],
      );
    };
  },
};

export default NanaSettingsRow;
