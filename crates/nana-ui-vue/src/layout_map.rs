//! L2 布局标签 → 初始 `WidgetKind` 与无作者 CSS 时的方向默认。
//!
//! 布局身份是 L3 [`LayoutStyle`]（经 L1 `css_map` 写入）。本模块**不**根据
//! `display` / `flex-direction` 改 `WidgetKind`。

use crate::bridge::WidgetKind;
use crate::css_map::{FlexDirection, LayoutStyle};

/// 无作者 CSS 时的方向默认。Row → `flex-direction: row`；Card 内边距由 L3 `Card` 提供。
pub fn default_layout_for_kind(kind: WidgetKind) -> LayoutStyle {
    let mut layout = LayoutStyle::default();
    match kind {
        WidgetKind::Row => {
            layout.direction = Some(FlexDirection::Row);
        }
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
        | WidgetKind::PaneChrome => {
            layout.direction = Some(FlexDirection::Column);
        }
        WidgetKind::TableRow => {
            layout.direction = Some(FlexDirection::Row);
        }
        _ => {}
    }
    layout
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

    #[test]
    fn row_kind_gets_row_defaults() {
        let layout = default_layout_for_kind(WidgetKind::Row);
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert!(
            layout.gap.is_none(),
            "Row kind must not invent gap (avoids workspace seams)"
        );
    }

    #[test]
    fn card_kind_does_not_invent_padding() {
        let layout = default_layout_for_kind(WidgetKind::Card);
        assert!(layout.padding.is_none());
        assert_eq!(layout.direction, Some(FlexDirection::Column));
    }

    #[test]
    fn layout_tags_map() {
        assert_eq!(layout_kind_from_tag("nana-row"), Some(WidgetKind::Row));
        assert_eq!(layout_kind_from_tag("div"), Some(WidgetKind::Column));
        assert_eq!(layout_kind_from_tag("nana-stack"), Some(WidgetKind::Box));
    }
}
