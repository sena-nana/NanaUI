//! L2 布局标签 → 初始 `WidgetKind` 与无作者 CSS 时的方向默认。
//!
//! 布局身份是 L3 [`LayoutStyle`](crate::css_map::LayoutStyle)（经 L1 `css_map`
//! 写入）。本模块**不**根据 `display` / `flex-direction` 改 `WidgetKind`。

use crate::bridge::WidgetKind;
use crate::css_map::FlexDirection;

/// 无作者 CSS 时的方向默认。
///
/// kind **只**播种方向：gap 由作者 CSS / `gap-*` 提示提供，内边距由 L3 控件
/// （如 `Card`）提供 —— 返回类型让这两者无法从这里被发明出来。
pub fn kind_default_direction(kind: WidgetKind) -> Option<FlexDirection> {
    match kind {
        WidgetKind::Row | WidgetKind::TableRow => Some(FlexDirection::Row),
        WidgetKind::Column
        | WidgetKind::Box
        | WidgetKind::SidebarFrame
        | WidgetKind::SidebarSection
        | WidgetKind::SidebarFooter
        | WidgetKind::SettingsCard
        | WidgetKind::SettingsCollapsibleCard
        | WidgetKind::Card
        | WidgetKind::List
        | WidgetKind::ScrollView
        | WidgetKind::Table
        | WidgetKind::DesktopShell
        | WidgetKind::PaneChrome => Some(FlexDirection::Column),
        _ => None,
    }
}

/// 布局 tag → `WidgetKind`（不含控件）。`nana-stack` 是 L3 `Stack` 的通用盒。
pub fn layout_kind_from_tag(tag: &str) -> Option<WidgetKind> {
    match tag.trim().to_ascii_lowercase().as_str() {
        "nana-column" => Some(WidgetKind::Column),
        "nana-row" => Some(WidgetKind::Row),
        "nana-stack" | "stack" | "nana-box" => Some(WidgetKind::Box),
        "nana-card" => Some(WidgetKind::Card),
        "nana-sidebar-frame" => Some(WidgetKind::SidebarFrame),
        "nana-settings-card" => Some(WidgetKind::SettingsCard),
        "div" | "section" | "article" | "main" | "aside" | "nav" | "header" | "footer" | "form"
        | "fieldset" | "body" | "template" | "fragment" => Some(WidgetKind::Column),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Kinds seed direction and nothing else: the return type is what now keeps
    // Row from inventing a gap (workspace seams) and Card from inventing padding.
    #[test]
    fn row_kind_gets_row_default() {
        assert_eq!(
            kind_default_direction(WidgetKind::Row),
            Some(FlexDirection::Row)
        );
    }

    #[test]
    fn card_kind_gets_column_default() {
        assert_eq!(
            kind_default_direction(WidgetKind::Card),
            Some(FlexDirection::Column)
        );
    }

    #[test]
    fn layout_tags_map() {
        assert_eq!(layout_kind_from_tag("nana-row"), Some(WidgetKind::Row));
        assert_eq!(layout_kind_from_tag("div"), Some(WidgetKind::Column));
        assert_eq!(layout_kind_from_tag("nana-stack"), Some(WidgetKind::Box));
    }
}
