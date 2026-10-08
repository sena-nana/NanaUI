//! What the framework's own controls say: the title bar's buttons, a
//! dialog's confirm and cancel, a media bar's play and pause, the built-in
//! appearance settings page.
//!
//! One table per document, Chinese by default. A consumer replaces it as a
//! whole ([`FrameworkStrings::default`] then field writes) or key by key
//! ([`FrameworkStrings::set`], for strings that arrive as JSON or from a
//! script). A label an application gives a control itself always wins over
//! the table.
//!
//! A template value names its arguments in braces (`拒绝块{hunk}`); fill it
//! with [`fill`].

use std::sync::Arc;

macro_rules! framework_strings {
    ($( $(#[$doc:meta])* $field:ident = $key:literal => $default:literal; )*) => {
        /// See the [module docs](self).
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct FrameworkStrings {
            $( $(#[$doc])* pub $field: Arc<str>, )*
        }

        impl Default for FrameworkStrings {
            fn default() -> Self {
                Self { $( $field: Arc::from($default), )* }
            }
        }

        impl FrameworkStrings {
            /// Every key [`Self::set`] and [`Self::get`] take, in table order.
            pub const KEYS: &'static [&'static str] = &[$($key),*];

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
    window_minimize = "window.minimize" => "最小化";
    window_maximize = "window.maximize" => "最大化";
    window_restore = "window.restore" => "还原";
    window_close = "window.close" => "关闭";

    dialog_confirm = "dialog.confirm" => "确认";
    dialog_cancel = "dialog.cancel" => "取消";

    image_viewer_close = "image_viewer.close" => "关闭";
    image_viewer_previous = "image_viewer.previous" => "上一张";
    image_viewer_next = "image_viewer.next" => "下一张";

    media_play = "media.play" => "播放";
    media_pause = "media.pause" => "暂停";
    media_volume = "media.volume" => "音量";
    media_mute = "media.mute" => "静音";
    media_unmute = "media.unmute" => "取消静音";
    media_fullscreen = "media.fullscreen" => "全屏";
    media_exit_fullscreen = "media.exit_fullscreen" => "退出全屏";
    media_progress = "media.progress" => "进度";
    media_controls = "media.controls" => "播放控制";
    media_settings = "media.settings" => "播放设置";

    chip_remove = "chip.remove" => "移除";
    file_tab_close = "file_tab.close" => "关闭文件";
    color_field_label = "color_field.label" => "颜色";
    path_field_label = "path_field.label" => "路径";
    path_field_browse = "path_field.browse" => "浏览";
    browser_view_label = "browser_view.label" => "网页";
    dynamic_form_auto = "dynamic_form.auto" => "自动选择";

    command_palette_placeholder = "command_palette.placeholder" => "搜索操作";
    command_palette_empty = "command_palette.empty" => "没有可用操作";
    menu_search = "menu.search" => "搜索操作";

    date_picker_previous_month = "date_picker.previous_month" => "上一月";
    date_picker_next_month = "date_picker.next_month" => "下一月";
    /// `{month}`: 1–12.
    calendar_month = "calendar.month" => "{month}月";
    calendar_monday = "calendar.monday" => "周一";
    calendar_wednesday = "calendar.wednesday" => "周三";
    calendar_friday = "calendar.friday" => "周五";

    find_title = "find.title" => "查找替换";
    find_query = "find.query" => "查找";
    find_replacement = "find.replacement" => "替换为";
    find_previous = "find.previous" => "上一处";
    find_next = "find.next" => "下一处";
    find_replace = "find.replace" => "替换";
    find_replace_all = "find.replace_all" => "全部替换";
    find_collapse = "find.collapse" => "收起查找";

    diff_title = "diff.title" => "差异";
    diff_unified = "diff.unified" => "统一";
    diff_split = "diff.split" => "分栏";
    /// `{hunk}`: the hunk's index.
    diff_accept_hunk = "diff.accept_hunk" => "接受块{hunk}";
    /// `{hunk}`: the hunk's index.
    diff_reject_hunk = "diff.reject_hunk" => "拒绝块{hunk}";
    /// `{hunk}`, `{line}`: the hunk's and the line's index.
    diff_accept_line = "diff.accept_line" => "接受行{hunk}.{line}";
    /// `{hunk}`, `{line}`: the hunk's and the line's index.
    diff_reject_line = "diff.reject_line" => "拒绝行{hunk}.{line}";

    theme_light = "theme.light" => "浅色";
    theme_dark = "theme.dark" => "深色";
    theme_custom = "theme.custom" => "自定义";
    material_solid = "material.solid" => "实色";
    material_translucent = "material.translucent" => "透明";
    material_region_sidebar = "material.region.sidebar" => "侧边栏";
    material_region_main = "material.region.main" => "主内容区";

    settings_back = "settings.back" => "返回";
    settings_theme = "settings.theme" => "主题";
    settings_theme_hint = "settings.theme.hint" => "选择应用配色，立即生效";
    settings_theme_dark = "settings.theme.dark" => "暗色";
    settings_theme_light = "settings.theme.light" => "浅色";
    settings_material = "settings.material" => "窗口材质";
    settings_material_hint = "settings.material.hint" => "可选择实色或透明背景；设备支持时，也可使用系统模糊效果。";
    settings_material_state = "settings.material.state" => "材质状态";
    settings_material_state_hint = "settings.material.state.hint" => "显示窗口当前使用的外观效果。";
    settings_region = "settings.region" => "透明区域";
    settings_region_hint = "settings.region.hint" => "选择侧边栏或主内容区显示透明材质。";
    settings_region_solid_hint = "settings.region.solid_hint" => "实色模式不显示透明区域；切回透明材质后会恢复当前选择。";
    settings_title_bar_follows = "settings.title_bar_follows" => "标题栏跟随侧边栏透明";
    settings_title_bar_follows_hint = "settings.title_bar_follows.hint" => "侧边栏透明时，整个标题栏同步显示透明材质。";
    settings_title_bar_follows_main_hint = "settings.title_bar_follows.main_hint" => "仅在侧边栏使用透明材质时生效；当前选择会保留。";
    settings_opacity = "settings.opacity" => "材质不透明度";
    settings_opacity_hint = "settings.opacity.hint" => "调节透明区域材质的前景色覆盖程度。";
    settings_opacity_solid_hint = "settings.opacity.solid_hint" => "实色模式不使用透明度；切回透明材质后会恢复当前数值。";
    settings_main_radius = "settings.main_radius" => "主区域圆角";
    settings_workspace_corners = "settings.workspace_corners" => "工作区边缘";
    settings_radius = "settings.radius" => "组件圆角半径";
    settings_defaults = "settings.defaults" => "默认样式";
    settings_defaults_hint = "settings.defaults.hint" => "恢复主题、材质与圆角默认值。";
    settings_restore = "settings.restore" => "恢复默认";
    settings_name = "settings.name" => "名称";
    settings_version = "settings.version" => "版本";
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
