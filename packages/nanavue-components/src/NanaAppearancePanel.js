/**
 * NanaAppearancePanel — Settings Appearance section.
 * Aligns with Lilia `AppearanceSection` + Rust `AppearanceSection`:
 * SettingsCard → SettingsRow × (Segmented / Switch / RangeField / status text).
 */
import { computed, h } from "@vue/runtime-core";
import { NanaButton } from "./NanaButton.js";
import { NanaRangeField } from "./NanaRangeField.js";
import { NanaSegmented } from "./NanaSegmented.js";
import { NanaSettingsCard } from "./NanaSettingsCard.js";
import { NanaSettingsRow } from "./NanaSettingsRow.js";
import { NanaSwitch } from "./NanaSwitch.js";
import { NanaThemeToggle } from "./NanaThemeToggle.js";
import {
  BACKDROP_OPACITY_MAX,
  BACKDROP_OPACITY_MIN,
  CORNER_RADIUS_MAX,
  CORNER_RADIUS_MIN,
  installCornerStyle,
  installNativeAppearance,
  resetAppearanceDefaults,
} from "./appearance.js";

const BACKDROP_OPTIONS = [
  {
    value: "solid",
    label: "实色",
    agentId: "settings.appearance.backdrop.solid",
  },
  {
    value: "translucent",
    label: "透明",
    agentId: "settings.appearance.backdrop.translucent",
  },
];

export const NanaAppearancePanel = {
  name: "NanaAppearancePanel",
  props: {
    title: { type: String, default: "外观" },
    description: { type: String, default: "" },
    materialStatus: { type: String, default: "" },
    platformHint: { type: String, default: "" },
  },
  setup(props, { attrs }) {
    const appearance = installNativeAppearance();
    const corner = installCornerStyle();

    const theme = computed(() =>
      appearance.theme.value === "dark" ? "dark" : "light",
    );
    const backdrop = computed(() =>
      appearance.backdropMode.value === "solid" ? "solid" : "translucent",
    );
    const backdropTarget = computed(() =>
      appearance.backdropTarget.value === "main" ? "main" : "sidebar",
    );
    const radius = computed(() => Number(corner.cornerRadius.value));
    const opacityPercent = computed(() =>
      Math.round(Number(appearance.backdropOpacity.value) * 100),
    );
    const titlebarFollow = computed(() => !!appearance.titlebarFollowsSidebar.value);
    const workspaceCorners = computed(() => !!appearance.workspaceCorners.value);
    const solidMode = computed(() => backdrop.value === "solid");
    const titlebarFollowDisabled = computed(
      () => solidMode.value || backdropTarget.value !== "sidebar",
    );
    const materialStatusText = computed(() => {
      if (props.materialStatus) return props.materialStatus;
      return solidMode.value ? "实色背景" : "透明材质";
    });
    const materialHint = computed(
      () =>
        props.platformHint ||
        "本平台首选 Vibrancy。Hosted GPU 表面若与材质层冲突会回退实色，不崩溃。",
    );

    const backdropTargetOptions = computed(() => [
      {
        value: "sidebar",
        label: "侧边栏",
        disabled: solidMode.value,
        agentId: "settings.appearance.backdrop-target.sidebar",
      },
      {
        value: "main",
        label: "主内容区",
        disabled: solidMode.value,
        agentId: "settings.appearance.backdrop-target.main",
      },
    ]);

    const backdropTargetHint = computed(() =>
      solidMode.value
        ? "实色模式不显示透明区域；切回透明材质后会恢复当前选择。"
        : "选择侧边栏或主内容区显示透明材质。",
    );
    const titlebarFollowHint = computed(() =>
      titlebarFollowDisabled.value
        ? "仅在侧边栏使用透明材质时生效；当前选择会保留。"
        : "侧边栏透明时，整个标题栏同步显示透明材质。",
    );
    const opacityHint = computed(() =>
      solidMode.value
        ? "实色模式不使用透明度；切回透明材质后会恢复当前数值。"
        : "调节透明区域材质的前景色覆盖程度。",
    );

    return () =>
      h(
        "div",
        {
          ...attrs,
          class: ["nana-appearance-panel", attrs.class].filter(Boolean).join(" "),
          "data-agent-id": attrs["data-agent-id"] || "settings.appearance",
          "data-theme": theme.value,
          "data-backdrop": backdrop.value,
        },
        [
          props.description
            ? h("div", { class: "nana-appearance-panel__desc" }, props.description)
            : null,
          h(
            NanaSettingsCard,
            { title: props.title, agentId: "settings.appearance" },
            {
              default: () => [
                h(
                  NanaSettingsRow,
                  {
                    label: "主题",
                    hint: "选择应用配色，立即生效",
                    firstInGroup: true,
                    agentId: "settings.appearance.theme-row",
                  },
                  {
                    default: () =>
                      h(NanaThemeToggle, {
                        modelValue: theme.value,
                        "aria-label": "主题",
                      }),
                  },
                ),
                h(
                  NanaSettingsRow,
                  {
                    label: "窗口材质",
                    hint: materialHint.value,
                    agentId: "settings.appearance.backdrop",
                  },
                  {
                    default: () =>
                      h(NanaSegmented, {
                        modelValue: backdrop.value,
                        options: BACKDROP_OPTIONS,
                        "aria-label": "窗口材质",
                        "data-agent-id": "settings.appearance.backdrop.control",
                        "onUpdate:modelValue": (value) =>
                          appearance.setBackdropMode(value),
                      }),
                  },
                ),
                h(
                  NanaSettingsRow,
                  {
                    label: "材质状态",
                    hint: "由宿主经 nana-window 应用后回报；失败时保持可读实色。",
                    agentId: "settings.appearance.material-status",
                  },
                  {
                    default: () =>
                      h(
                        "span",
                        { class: "nana-appearance-panel__muted" },
                        materialStatusText.value,
                      ),
                  },
                ),
                h(
                  NanaSettingsRow,
                  {
                    label: "透明区域",
                    hint: backdropTargetHint.value,
                    agentId: "settings.appearance.backdrop-target",
                  },
                  {
                    default: () =>
                      h(NanaSegmented, {
                        modelValue: backdropTarget.value,
                        options: backdropTargetOptions.value,
                        "aria-label": "透明区域",
                        "data-agent-id": "settings.appearance.backdrop-target.control",
                        "onUpdate:modelValue": (value) =>
                          appearance.setBackdropTarget(value),
                      }),
                  },
                ),
                h(
                  NanaSettingsRow,
                  {
                    label: "标题栏跟随侧边栏透明",
                    hint: titlebarFollowHint.value,
                    agentId: "settings.appearance.titlebar-follow-sidebar-row",
                  },
                  {
                    default: () =>
                      h(NanaSwitch, {
                        modelValue: titlebarFollow.value,
                        disabled: titlebarFollowDisabled.value,
                        agentId: "settings.appearance.titlebar-follow-sidebar",
                        "aria-label": "标题栏跟随侧边栏透明",
                        "onUpdate:modelValue": (value) =>
                          appearance.setTitlebarFollowsSidebar(value),
                      }),
                  },
                ),
                h(
                  NanaSettingsRow,
                  {
                    label: "材质不透明度",
                    hint: opacityHint.value,
                    agentId: "settings.appearance.backdrop-opacity-row",
                  },
                  {
                    default: () =>
                      solidMode.value
                        ? h(
                            "span",
                            { class: "nana-appearance-panel__muted" },
                            `${opacityPercent.value}%`,
                          )
                        : h(NanaRangeField, {
                            modelValue: opacityPercent.value,
                            min: Math.round(BACKDROP_OPACITY_MIN * 100),
                            max: Math.round(BACKDROP_OPACITY_MAX * 100),
                            step: 1,
                            unit: "%",
                            agentId: "settings.appearance.backdrop-opacity",
                            "aria-label": "材质不透明度",
                            "onUpdate:modelValue": (value) =>
                              appearance.setBackdropOpacity(Number(value) / 100),
                          }),
                  },
                ),
                h(
                  NanaSettingsRow,
                  {
                    label: "工作区边缘",
                    agentId: "settings.appearance.workspace-corners-row",
                  },
                  {
                    default: () =>
                      h(NanaSwitch, {
                        modelValue: workspaceCorners.value,
                        label: "主区域圆角",
                        agentId: "settings.appearance.workspace-corners",
                        "aria-label": "主区域圆角",
                        "onUpdate:modelValue": (value) =>
                          appearance.setWorkspaceCorners(value),
                      }),
                  },
                ),
                h(
                  NanaSettingsRow,
                  {
                    label: "组件圆角半径",
                    agentId: "settings.appearance.corner-radius-row",
                  },
                  {
                    default: () =>
                      h(NanaRangeField, {
                        modelValue: radius.value,
                        min: CORNER_RADIUS_MIN,
                        max: CORNER_RADIUS_MAX,
                        step: 1,
                        unit: "px",
                        agentId: "settings.appearance.corner-radius",
                        "aria-label": "圆角半径",
                        "onUpdate:modelValue": (value) =>
                          corner.setCornerRadius(value),
                      }),
                  },
                ),
                h(
                  NanaSettingsRow,
                  {
                    label: "默认样式",
                    hint: "恢复主题、材质与圆角默认值。",
                    lastInGroup: true,
                    agentId: "settings.appearance.reset-row",
                  },
                  {
                    default: () =>
                      h(NanaButton, {
                        kind: "subtle",
                        size: "small",
                        label: "恢复默认",
                        "data-agent-id": "settings.appearance.reset",
                        onPress: () => resetAppearanceDefaults(),
                      }),
                  },
                ),
              ],
            },
          ),
        ],
      );
  },
};

export default NanaAppearancePanel;
