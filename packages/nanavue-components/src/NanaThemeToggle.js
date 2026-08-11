/**
 * NanaThemeToggle — Appearance theme segmented control (Dark / Light).
 * Mirrors `nana_ui_core::ThemeMode` + Lilia AppearanceSection theme row.
 */
import { computed, h } from "@vue/runtime-core";
import { NanaSegmented } from "./NanaSegmented.js";
import { installNativeAppearance } from "./appearance.js";

const THEME_OPTIONS = [
  {
    value: "dark",
    label: "暗色",
    agentId: "settings.appearance.theme.dark",
  },
  {
    value: "light",
    label: "浅色",
    agentId: "settings.appearance.theme.light",
  },
];

export const NanaThemeToggle = {
  name: "NanaThemeToggle",
  props: {
    modelValue: { type: String, default: "" },
  },
  emits: ["update:modelValue", "change"],
  setup(props, { emit, attrs }) {
    const appearance = installNativeAppearance();
    const theme = computed(() => {
      if (props.modelValue === "light" || props.modelValue === "dark") {
        return props.modelValue;
      }
      return appearance.theme.value === "dark" ? "dark" : "light";
    });

    function select(next) {
      const value = next === "dark" ? "dark" : "light";
      appearance.setTheme(value);
      emit("update:modelValue", value);
      emit("change", value);
    }

    return () =>
      h(NanaSegmented, {
        ...attrs,
        class: ["nana-theme-toggle", attrs.class].filter(Boolean).join(" "),
        modelValue: theme.value,
        options: THEME_OPTIONS,
        "aria-label": attrs["aria-label"] || "主题",
        "data-agent-id": attrs["data-agent-id"] || "nana.theme-toggle",
        "onUpdate:modelValue": select,
      });
  },
};

export default NanaThemeToggle;
