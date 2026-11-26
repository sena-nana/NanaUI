//! What the framework's own controls say: the title bar's buttons, a
//! dialog's confirm and cancel, a media bar's play and pause, the built-in
//! appearance settings page.
//!
//! One table per document, Chinese by default, with built-in `zh-CN`, `en-US`,
//! and `ja-JP` tables via [`FrameworkStrings::for_locale`]. A consumer can also
//! replace it as a whole ([`FrameworkStrings::default`] then field writes) or
//! key by key ([`FrameworkStrings::set`], for strings that arrive as JSON or
//! from a script). A label an application gives a control itself always wins
//! over the table.
//!
//! A template value names its arguments in braces (`拒绝块{hunk}`); fill it
//! with [`fill`].

use std::sync::Arc;

macro_rules! framework_strings {
    ($( $(#[$doc:meta])* $field:ident = $key:literal => { zh: $zh:literal, en: $en:literal, ja: $ja:literal }; )*) => {
        /// See the [module docs](self).
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct FrameworkStrings {
            $( $(#[$doc])* pub $field: Arc<str>, )*
        }

        impl Default for FrameworkStrings {
            fn default() -> Self {
                Self::zh_cn()
            }
        }

        impl FrameworkStrings {
            /// Built-in locale tags shipped directly in [`FrameworkStrings`].
            pub const BUILT_IN_LOCALES: &'static [&'static str] = &["zh-CN", "en-US", "ja-JP"];

            /// Every key [`Self::set`] and [`Self::get`] take, in table order.
            pub const KEYS: &'static [&'static str] = &[$($key),*];

            /// Built-in Simplified Chinese (`zh-CN`) table.
            pub fn zh_cn() -> Self {
                Self { $( $field: Arc::from($zh), )* }
            }

            /// Built-in English (`en-US`) table.
            pub fn en_us() -> Self {
                Self { $( $field: Arc::from($en), )* }
            }

            /// Built-in Japanese (`ja-JP`) table.
            pub fn ja_jp() -> Self {
                Self { $( $field: Arc::from($ja), )* }
            }

            /// Built-in table for BCP-47 `locale` (matching primary language
            /// subtag case-insensitively: `en*` → English, `ja*` → Japanese,
            /// `zh*` or unrecognized → Simplified Chinese).
            pub fn for_locale(locale: &str) -> Self {
                let primary = locale
                    .trim()
                    .split(['-', '_'])
                    .next()
                    .unwrap_or_default();
                if primary.eq_ignore_ascii_case("en") {
                    Self::en_us()
                } else if primary.eq_ignore_ascii_case("ja") {
                    Self::ja_jp()
                } else {
                    Self::zh_cn()
                }
            }

            /// The string under `key`.
            pub fn get(&self, key: &str) -> Option<&str> {
                match key {
                    $( $key => Some(&self.$field), )*
                    _ => None,
                }
            }

            /// Replace the string under `key`. Whether there was one.
            pub fn set(&mut self, key: &str, value: impl Into<Arc<str>>) -> bool {
                match key {
                    $( $key => { self.$field = value.into(); true } )*
                    _ => false,
                }
            }
        }
    };
}

framework_strings! {
    window_minimize = "window.minimize" => {
        zh: "最小化",
        en: "Minimize",
        ja: "最小化"
    };
    window_maximize = "window.maximize" => {
        zh: "最大化",
        en: "Maximize",
        ja: "最大化"
    };
    window_restore = "window.restore" => {
        zh: "还原",
        en: "Restore",
        ja: "元に戻す"
    };
    window_close = "window.close" => {
        zh: "关闭",
        en: "Close",
        ja: "閉じる"
    };

    dialog_confirm = "dialog.confirm" => {
        zh: "确认",
        en: "Confirm",
        ja: "確認"
    };
    dialog_cancel = "dialog.cancel" => {
        zh: "取消",
        en: "Cancel",
        ja: "キャンセル"
    };

    image_viewer_close = "image_viewer.close" => {
        zh: "关闭",
        en: "Close",
        ja: "閉じる"
    };
    image_viewer_previous = "image_viewer.previous" => {
        zh: "上一张",
        en: "Previous",
        ja: "前へ"
    };
    image_viewer_next = "image_viewer.next" => {
        zh: "下一张",
        en: "Next",
        ja: "次へ"
    };

    media_play = "media.play" => {
        zh: "播放",
        en: "Play",
        ja: "再生"
    };
    media_pause = "media.pause" => {
        zh: "暂停",
        en: "Pause",
        ja: "一時停止"
    };
    media_volume = "media.volume" => {
        zh: "音量",
        en: "Volume",
        ja: "音量"
    };
    media_mute = "media.mute" => {
        zh: "静音",
        en: "Mute",
        ja: "ミュート"
    };
    media_unmute = "media.unmute" => {
        zh: "取消静音",
        en: "Unmute",
        ja: "ミュート解除"
    };
    media_fullscreen = "media.fullscreen" => {
        zh: "全屏",
        en: "Full screen",
        ja: "全画面"
    };
    media_exit_fullscreen = "media.exit_fullscreen" => {
        zh: "退出全屏",
        en: "Exit full screen",
        ja: "全画面を終了"
    };
    media_progress = "media.progress" => {
        zh: "进度",
        en: "Progress",
        ja: "再生位置"
    };
    media_controls = "media.controls" => {
        zh: "播放控制",
        en: "Playback controls",
        ja: "再生コントロール"
    };
    media_settings = "media.settings" => {
        zh: "播放设置",
        en: "Playback settings",
        ja: "再生設定"
    };

    toast_close = "toast.close" => {
        zh: "关闭通知",
        en: "Dismiss notification",
        ja: "通知を閉じる"
    };
    toast_region = "toast.region" => {
        zh: "通知",
        en: "Notifications",
        ja: "通知"
    };
    /// `{percent}`: 0–100.
    toast_percent = "toast.percent" => {
        zh: "{percent}%",
        en: "{percent}%",
        ja: "{percent}%"
    };

    chip_remove = "chip.remove" => {
        zh: "移除",
        en: "Remove",
        ja: "削除"
    };
    /// What a range's reset button is called: it puts the default value back.
    range_field_reset = "range_field.reset" => {
        zh: "恢复默认",
        en: "Reset to default",
        ja: "既定値に戻す"
    };
    /// What assistive technology calls a Markdown diagram without a title.
    diagram_label = "diagram.label" => {
        zh: "图表",
        en: "Diagram",
        ja: "図"
    };
    /// A diagram's element counts. `{nodes}`, `{edges}`.
    diagram_summary = "diagram.summary" => {
        zh: "{nodes} 个节点，{edges} 条连线",
        en: "{nodes} nodes, {edges} connections",
        ja: "{nodes} 個のノード、{edges} 本の接続"
    };
    /// Titles of a Markdown GFM alert (`> [!NOTE]` and its kin).
    markdown_alert_note = "markdown.alert.note" => {
        zh: "说明",
        en: "Note",
        ja: "注記"
    };
    markdown_alert_tip = "markdown.alert.tip" => {
        zh: "提示",
        en: "Tip",
        ja: "ヒント"
    };
    markdown_alert_important = "markdown.alert.important" => {
        zh: "重要",
        en: "Important",
        ja: "重要"
    };
    markdown_alert_warning = "markdown.alert.warning" => {
        zh: "警告",
        en: "Warning",
        ja: "警告"
    };
    markdown_alert_caution = "markdown.alert.caution" => {
        zh: "注意",
        en: "Caution",
        ja: "注意"
    };
    file_tab_close = "file_tab.close" => {
        zh: "关闭文件",
        en: "Close file",
        ja: "ファイルを閉じる"
    };
    size_field_label = "size_field.label" => {
        zh: "尺寸",
        en: "Size",
        ja: "サイズ"
    };
    size_field_width = "size_field.width" => {
        zh: "宽度",
        en: "Width",
        ja: "幅"
    };
    size_field_height = "size_field.height" => {
        zh: "高度",
        en: "Height",
        ja: "高さ"
    };
    size_field_presets = "size_field.presets" => {
        zh: "预设尺寸",
        en: "Size presets",
        ja: "サイズのプリセット"
    };
    color_field_label = "color_field.label" => {
        zh: "颜色",
        en: "Color",
        ja: "色"
    };
    color_field_eyedropper = "color_field.eyedropper" => {
        zh: "吸取颜色",
        en: "Pick color",
        ja: "色をスポイト"
    };
    path_field_label = "path_field.label" => {
        zh: "路径",
        en: "Path",
        ja: "パス"
    };
    path_field_browse = "path_field.browse" => {
        zh: "浏览",
        en: "Browse",
        ja: "参照"
    };
    browser_view_label = "browser_view.label" => {
        zh: "网页",
        en: "Web page",
        ja: "Web ページ"
    };
    dynamic_form_auto = "dynamic_form.auto" => {
        zh: "自动选择",
        en: "Automatic",
        ja: "自動"
    };

    command_palette_placeholder = "command_palette.placeholder" => {
        zh: "搜索操作",
        en: "Search actions",
        ja: "操作を検索"
    };
    command_palette_empty = "command_palette.empty" => {
        zh: "没有可用操作",
        en: "No actions available",
        ja: "使える操作がありません"
    };
    menu_search = "menu.search" => {
        zh: "搜索操作",
        en: "Search actions",
        ja: "操作を検索"
    };

    /// What assistive technology calls an area's editor type select.
    area_editor_type = "area_layout.editor_type" => {
        zh: "编辑器类型",
        en: "Editor type",
        ja: "エディターの種類"
    };

    date_picker_previous_month = "date_picker.previous_month" => {
        zh: "上一月",
        en: "Previous month",
        ja: "前の月"
    };
    date_picker_next_month = "date_picker.next_month" => {
        zh: "下一月",
        en: "Next month",
        ja: "次の月"
    };
    /// `{month}`: 1–12.
    calendar_month = "calendar.month" => {
        zh: "{month}月",
        en: "Month {month}",
        ja: "{month}月"
    };
    calendar_monday = "calendar.monday" => {
        zh: "周一",
        en: "Mon",
        ja: "月"
    };
    calendar_wednesday = "calendar.wednesday" => {
        zh: "周三",
        en: "Wed",
        ja: "水"
    };
    calendar_friday = "calendar.friday" => {
        zh: "周五",
        en: "Fri",
        ja: "金"
    };

    find_title = "find.title" => {
        zh: "查找替换",
        en: "Find and replace",
        ja: "検索と置換"
    };
    find_query = "find.query" => {
        zh: "查找",
        en: "Find",
        ja: "検索"
    };
    find_replacement = "find.replacement" => {
        zh: "替换为",
        en: "Replace with",
        ja: "置換後"
    };
    find_previous = "find.previous" => {
        zh: "上一处",
        en: "Previous match",
        ja: "前を検索"
    };
    find_next = "find.next" => {
        zh: "下一处",
        en: "Next match",
        ja: "次を検索"
    };
    find_replace = "find.replace" => {
        zh: "替换",
        en: "Replace",
        ja: "置換"
    };
    find_replace_all = "find.replace_all" => {
        zh: "全部替换",
        en: "Replace all",
        ja: "すべて置換"
    };
    find_collapse = "find.collapse" => {
        zh: "收起查找",
        en: "Hide find",
        ja: "検索を閉じる"
    };

    diff_title = "diff.title" => {
        zh: "差异",
        en: "Differences",
        ja: "差分"
    };
    diff_unified = "diff.unified" => {
        zh: "统一",
        en: "Unified",
        ja: "統合"
    };
    diff_split = "diff.split" => {
        zh: "分栏",
        en: "Split",
        ja: "左右"
    };
    /// `{hunk}`: the hunk's index.
    diff_accept_hunk = "diff.accept_hunk" => {
        zh: "接受块{hunk}",
        en: "Accept hunk {hunk}",
        ja: "ハンク {hunk} を適用"
    };
    /// `{hunk}`: the hunk's index.
    diff_reject_hunk = "diff.reject_hunk" => {
        zh: "拒绝块{hunk}",
        en: "Reject hunk {hunk}",
        ja: "ハンク {hunk} を破棄"
    };
    /// `{hunk}`, `{line}`: the hunk's and the line's index.
    diff_accept_line = "diff.accept_line" => {
        zh: "接受行{hunk}.{line}",
        en: "Accept line {hunk}.{line}",
        ja: "行 {hunk}.{line} を適用"
    };
    /// `{hunk}`, `{line}`: the hunk's and the line's index.
    diff_reject_line = "diff.reject_line" => {
        zh: "拒绝行{hunk}.{line}",
        en: "Reject line {hunk}.{line}",
        ja: "行 {hunk}.{line} を破棄"
    };

    theme_light = "theme.light" => {
        zh: "浅色",
        en: "Light",
        ja: "ライト"
    };
    theme_dark = "theme.dark" => {
        zh: "深色",
        en: "Dark",
        ja: "ダーク"
    };
    theme_custom = "theme.custom" => {
        zh: "自定义",
        en: "Custom",
        ja: "カスタム"
    };
    material_solid = "material.solid" => {
        zh: "实色",
        en: "Solid",
        ja: "単色"
    };
    material_translucent = "material.translucent" => {
        zh: "透明",
        en: "Transparent",
        ja: "透明"
    };
    material_region_sidebar = "material.region.sidebar" => {
        zh: "侧边栏",
        en: "Sidebar",
        ja: "サイドバー"
    };
    material_region_main = "material.region.main" => {
        zh: "主内容区",
        en: "Main area",
        ja: "メイン領域"
    };

    settings_back = "settings.back" => {
        zh: "返回",
        en: "Back",
        ja: "戻る"
    };
    settings_theme = "settings.theme" => {
        zh: "主题",
        en: "Theme",
        ja: "テーマ"
    };
    settings_theme_hint = "settings.theme.hint" => {
        zh: "选择应用配色，立即生效",
        en: "Choose the app's colors. Takes effect right away.",
        ja: "アプリの配色を選びます。すぐに反映されます。"
    };
    settings_theme_dark = "settings.theme.dark" => {
        zh: "暗色",
        en: "Dark",
        ja: "ダーク"
    };
    settings_theme_light = "settings.theme.light" => {
        zh: "浅色",
        en: "Light",
        ja: "ライト"
    };
    settings_material = "settings.material" => {
        zh: "窗口材质",
        en: "Window material",
        ja: "ウィンドウの質感"
    };
    settings_material_hint = "settings.material.hint" => {
        zh: "可选择实色或透明背景；设备支持时，也可使用系统模糊效果。",
        en: "Choose a solid or transparent background. Where the device supports it, the system blur is available too.",
        ja: "単色か透明の背景を選べます。デバイスが対応していれば、システムのぼかし効果も使えます。"
    };
    settings_material_state = "settings.material.state" => {
        zh: "材质状态",
        en: "Material status",
        ja: "質感の状態"
    };
    settings_material_state_hint = "settings.material.state.hint" => {
        zh: "显示窗口当前使用的外观效果。",
        en: "The look the window uses now.",
        ja: "ウィンドウが今使っている外観です。"
    };
    settings_region = "settings.region" => {
        zh: "透明区域",
        en: "Transparent area",
        ja: "透明にする領域"
    };
    settings_region_hint = "settings.region.hint" => {
        zh: "选择侧边栏或主内容区显示透明材质。",
        en: "Choose whether the sidebar or the main area shows the transparent material.",
        ja: "サイドバーとメイン領域のどちらを透明にするか選びます。"
    };
    settings_region_solid_hint = "settings.region.solid_hint" => {
        zh: "实色模式不显示透明区域；切回透明材质后会恢复当前选择。",
        en: "Solid mode shows no transparent area. Your choice comes back when you switch to the transparent material.",
        ja: "単色モードでは透明な領域はありません。透明に戻すと今の選択が戻ります。"
    };
    settings_title_bar_follows = "settings.title_bar_follows" => {
        zh: "标题栏跟随侧边栏透明",
        en: "Title bar follows a transparent sidebar",
        ja: "タイトルバーをサイドバーの透明に合わせる"
    };
    settings_title_bar_follows_hint = "settings.title_bar_follows.hint" => {
        zh: "侧边栏透明时，整个标题栏同步显示透明材质。",
        en: "While the sidebar is transparent, the whole title bar shows the transparent material too.",
        ja: "サイドバーが透明なとき、タイトルバー全体も透明になります。"
    };
    settings_title_bar_follows_main_hint = "settings.title_bar_follows.main_hint" => {
        zh: "仅在侧边栏使用透明材质时生效；当前选择会保留。",
        en: "Applies only while the sidebar uses the transparent material. Your choice is kept.",
        ja: "サイドバーが透明なときだけ有効です。今の選択は保たれます。"
    };
    settings_opacity = "settings.opacity" => {
        zh: "材质不透明度",
        en: "Material opacity",
        ja: "質感の不透明度"
    };
    settings_opacity_hint = "settings.opacity.hint" => {
        zh: "调节透明区域材质的前景色覆盖程度。",
        en: "How much the foreground color covers the transparent material.",
        ja: "透明な領域を前景色がどれだけ覆うかを調整します。"
    };
    settings_opacity_solid_hint = "settings.opacity.solid_hint" => {
        zh: "实色模式不使用透明度；切回透明材质后会恢复当前数值。",
        en: "Solid mode uses no opacity. The value comes back when you switch to the transparent material.",
        ja: "単色モードでは不透明度を使いません。透明に戻すと今の値が戻ります。"
    };
    settings_main_radius = "settings.main_radius" => {
        zh: "主区域圆角",
        en: "Main area corners",
        ja: "メイン領域の角丸"
    };
    settings_workspace_corners = "settings.workspace_corners" => {
        zh: "工作区边缘",
        en: "Workspace edges",
        ja: "ワークスペースの縁"
    };
    settings_radius = "settings.radius" => {
        zh: "组件圆角半径",
        en: "Control corner radius",
        ja: "コントロールの角丸の半径"
    };
    settings_defaults = "settings.defaults" => {
        zh: "默认样式",
        en: "Default style",
        ja: "既定のスタイル"
    };
    settings_defaults_hint = "settings.defaults.hint" => {
        zh: "恢复主题、材质与圆角默认值。",
        en: "Restore the default theme, material and corners.",
        ja: "テーマ、質感、角丸を既定に戻します。"
    };
    settings_restore = "settings.restore" => {
        zh: "恢复默认",
        en: "Restore defaults",
        ja: "既定に戻す"
    };
    settings_name = "settings.name" => {
        zh: "名称",
        en: "Name",
        ja: "名前"
    };
    settings_version = "settings.version" => {
        zh: "版本",
        en: "Version",
        ja: "バージョン"
    };
}

/// `template` with each `{name}` replaced by its value in `args`; a name
/// `args` lacks is left as written.
pub fn fill(template: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 8);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &after[..close];
        match args.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => {
                use std::fmt::Write;
                let _ = write!(out, "{value}");
            }
            None => out.push_str(&rest[open..open + close + 2]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_reach_every_field_and_set_replaces_one() {
        let mut strings = FrameworkStrings::default();
        assert_eq!(strings.get("window.close"), Some("关闭"));
        for key in FrameworkStrings::KEYS {
            assert!(
                strings.get(key).is_some_and(|value| !value.is_empty()),
                "{key}"
            );
        }
        assert!(strings.set("window.close", "Close"));
        assert_eq!(&*strings.window_close, "Close");
        assert!(!strings.set("window.nothing", "x"));
        let unique: std::collections::HashSet<_> = FrameworkStrings::KEYS.iter().collect();
        assert_eq!(unique.len(), FrameworkStrings::KEYS.len());
    }

    #[test]
    fn built_in_locales_cover_all_keys_and_preserve_placeholders() {
        fn placeholders(template: &str) -> Vec<&str> {
            let mut vars = Vec::new();
            let mut rest = template;
            while let Some(open) = rest.find('{') {
                let after = &rest[open + 1..];
                let Some(close) = after.find('}') else {
                    break;
                };
                vars.push(&after[..close]);
                rest = &after[close + 1..];
            }
            vars.sort_unstable();
            vars
        }

        let zh = FrameworkStrings::zh_cn();
        let en = FrameworkStrings::en_us();
        let ja = FrameworkStrings::ja_jp();
        assert_eq!(zh, FrameworkStrings::default());

        for key in FrameworkStrings::KEYS {
            let zh_val = zh.get(key).expect("zh key");
            let en_val = en.get(key).expect("en key");
            let ja_val = ja.get(key).expect("ja key");
            assert!(!zh_val.is_empty(), "empty zh for {key}");
            assert!(!en_val.is_empty(), "empty en for {key}");
            assert!(!ja_val.is_empty(), "empty ja for {key}");
            assert_eq!(
                placeholders(zh_val),
                placeholders(en_val),
                "placeholder mismatch zh vs en for {key}"
            );
            assert_eq!(
                placeholders(zh_val),
                placeholders(ja_val),
                "placeholder mismatch zh vs ja for {key}"
            );
        }

        assert_eq!(FrameworkStrings::for_locale("en"), en);
        assert_eq!(FrameworkStrings::for_locale("en-US"), en);
        assert_eq!(FrameworkStrings::for_locale("EN_gb"), en);
        assert_eq!(FrameworkStrings::for_locale("ja"), ja);
        assert_eq!(FrameworkStrings::for_locale("ja-JP"), ja);
        assert_eq!(FrameworkStrings::for_locale("zh-CN"), zh);
        assert_eq!(FrameworkStrings::for_locale("fr-FR"), zh);
        assert_eq!(en.get("window.close"), Some("Close"));
        assert_eq!(ja.get("window.close"), Some("閉じる"));
        assert_eq!(
            fill(&en.diagram_summary, &[("nodes", &4), ("edges", &6)]),
            "4 nodes, 6 connections"
        );
        assert_eq!(
            fill(&ja.diff_accept_line, &[("hunk", &2), ("line", &5)]),
            "行 2.5 を適用"
        );
    }

    #[test]
    fn fill_names_its_arguments() {
        assert_eq!(fill("拒绝块{hunk}", &[("hunk", &3)]), "拒绝块3");
        assert_eq!(
            fill(
                "Reject line {line} of hunk {hunk}",
                &[("hunk", &1), ("line", &2)]
            ),
            "Reject line 2 of hunk 1"
        );
        assert_eq!(fill("{missing} kept", &[]), "{missing} kept");
        assert_eq!(fill("unclosed {brace", &[]), "unclosed {brace");
    }
}
