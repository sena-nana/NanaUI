//! L2 布局映射：标准布局标签 / class → Style Model Layout + Column/Row 合同。
//!
//! ## L2 边界
//! - 产出 [`LayoutStyle`]（Layout 切片），不解析完整 CSS cascade（L1）。
//! - 控件语义 class → [`WidgetKind`] 走 [`crate::widget_map`]，本模块不发明 Semantics。
//! - DesktopShell / `region_views` 投影仍在 bridge（本回合不搬家；见边界文档）。

use crate::bridge::WidgetKind;
use crate::css_map::{DisplaySpec, FlexDirection, LayoutStyle, LengthSpec};

/// 布局相关 tag 默认方向与间距。
pub fn default_layout_for_kind(kind: WidgetKind) -> LayoutStyle {
    let mut layout = LayoutStyle::default();
    match kind {
        WidgetKind::Row => {
            layout.direction = Some(FlexDirection::Row);
            // Gap is not implied by kind — author via CSS / `gap-*` class hints.
        }
        WidgetKind::Column
        | WidgetKind::Box
        | WidgetKind::SidebarFrame
        | WidgetKind::SettingsCard => {
            layout.direction = Some(FlexDirection::Column);
        }
        WidgetKind::Card => {
            layout.direction = Some(FlexDirection::Column);
            layout.gap = Some(LengthSpec::Px(8.0));
            layout.padding = Some(crate::css_map::LengthSpec::Px(12.0));
        }
        _ => {}
    }
    layout
}

/// `display:block` → Column；`display:flex` + row direction → Row（P0 display）。
pub fn apply_display_to_kind(kind: WidgetKind, layout: &LayoutStyle) -> WidgetKind {
    if !kind.is_layout() {
        return kind;
    }
    match layout.display {
        Some(DisplaySpec::None) => kind,
        Some(DisplaySpec::Block) => {
            if matches!(kind, WidgetKind::Row) {
                WidgetKind::Column
            } else {
                kind
            }
        }
        Some(
            DisplaySpec::Flex
            | DisplaySpec::InlineFlex
            | DisplaySpec::Grid
            | DisplaySpec::InlineGrid,
        ) => apply_direction_to_kind(kind, layout),
        None => apply_direction_to_kind(kind, layout),
    }
}

/// 根据 [`LayoutStyle::direction`] 在 Row/Column 间切换（仅布局类节点）。
pub fn apply_direction_to_kind(kind: WidgetKind, layout: &LayoutStyle) -> WidgetKind {
    if !kind.is_layout() {
        return kind;
    }
    match layout.direction {
        Some(FlexDirection::Row)
            if matches!(
                kind,
                WidgetKind::Column
                    | WidgetKind::Box
                    | WidgetKind::Card
                    | WidgetKind::SidebarFrame
                    | WidgetKind::SettingsCard
            ) =>
        {
            WidgetKind::Row
        }
        Some(FlexDirection::Column) if matches!(kind, WidgetKind::Row) => WidgetKind::Column,
        _ => kind,
    }
}

/// 布局 tag → `WidgetKind`（不含控件）。
pub fn layout_kind_from_tag(tag: &str) -> Option<WidgetKind> {
    match tag.trim().to_ascii_lowercase().as_str() {
        "nana-column" | "nana-col" | "nana-vstack" => Some(WidgetKind::Column),
        "nana-row" | "nana-hstack" => Some(WidgetKind::Row),
        "nana-box" | "nana-container" | "nana-layout" => Some(WidgetKind::Box),
        "nana-card" => Some(WidgetKind::Card),
        "nana-sidebar-frame" => Some(WidgetKind::SidebarFrame),
        "nana-settings-card" => Some(WidgetKind::SettingsCard),
        "div" | "section" | "article" | "main" | "aside" | "nav" | "header" | "footer" | "ul"
        | "ol" | "form" | "fieldset" | "body" | "template" | "fragment" => Some(WidgetKind::Column),
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
    fn direction_flips_column_to_row() {
        let mut layout = LayoutStyle::default();
        layout.direction = Some(FlexDirection::Row);
        assert_eq!(
            apply_direction_to_kind(WidgetKind::Column, &layout),
            WidgetKind::Row
        );
    }

    #[test]
    fn layout_tags_map() {
        assert_eq!(layout_kind_from_tag("nana-row"), Some(WidgetKind::Row));
        assert_eq!(layout_kind_from_tag("div"), Some(WidgetKind::Column));
    }

    #[test]
    fn display_block_forces_column() {
        let mut layout = LayoutStyle::default();
        layout.display = Some(DisplaySpec::Block);
        assert_eq!(
            apply_display_to_kind(WidgetKind::Row, &layout),
            WidgetKind::Column
        );
    }
}
