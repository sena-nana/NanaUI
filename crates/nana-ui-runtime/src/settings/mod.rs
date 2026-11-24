mod group;
mod page;
mod row;
mod sections;

pub use group::{SettingsCard, SettingsCollapsibleCard, SettingsGroup};
pub(crate) use group::{mark_settings_group, resolve_row_hint_placement_now};
pub use page::{
    SettingsBack, SettingsPage, SettingsPageAssembly, SettingsSidebar, SettingsSidebarAssembly,
    SettingsTabSelected,
};
pub(crate) use row::ASSEMBLED_ROW_STACK_BELOW;
pub use row::{HintPlacement, SettingsRow};
pub use sections::{
    AboutMetadata, AboutSection, AboutSectionAssembly, AppearanceSection,
    AppearanceSectionAssembly, apply_appearance_event,
};

use crate::{AppContext, ComponentView, Entity, FrameworkError, StableNodeId};

pub(super) fn document_of(
    context: &AppContext,
    id: StableNodeId,
) -> Result<crate::DocumentId, FrameworkError> {
    context
        .world()
        .node(id)
        .map(|node| node.document)
        .ok_or(FrameworkError::MissingView(id))
}

pub(super) fn reconcile_children<C: ComponentView>(
    context: &mut AppContext,
    parent: Entity<C>,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    context.reconcile_children(parent.stable_id(), ordered)
}

#[cfg(test)]
mod tests {
    use super::page::{SETTINGS_SIDEBAR_GAP, SETTINGS_SIDEBAR_ICON_SIZE};
    use super::row::{
        ROW_COPY_GAP, ROW_MULTILINE_GROUP_PADDING_Y, ROW_PADDING_Y, ROW_STACK_GAP_LOOSE,
        row_hint_text, row_label_text,
    };
    use super::*;
    use crate::view_components::{RangeField, Switch, Text};
    use crate::{
        AppContext, DocumentId, Entity, NodeKind, NodeStyle, SegmentedOption, SidebarRow,
        SidebarRowState, StableNodeId,
    };
    use nana_ui_core::{
        AlignSpec, AppearanceEvent, AppearanceSettings, BackdropTarget, CardKind, ControlSize,
        FlexDirection, FlexWrap, Icon, JustifySpec, LengthSpec, SemanticColorRole, SettingsModel,
        SettingsState, SettingsTabId, ThemeAppearance, UI_METRICS, WindowMaterialMode,
    };
    use std::sync::{Arc, Mutex};

    const DISCLOSURE_ICON_SIZE: f32 = nana_ui_core::TypographyTokens::DEFAULT.line;
    const SETTINGS_PAGE_TITLE_SIZE: f32 = nana_ui_core::TypographyTokens::DEFAULT.title;
    const SETTINGS_PAGE_TITLE_WEIGHT: u16 = nana_ui_core::TypographyTokens::DEFAULT.semibold;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn settings_row_responds_to_its_own_width_and_preserves_control() {
        let mut context = AppContext::new();
        let parent = context
            .create_component(
                document(),
                crate::Stack::column(0.0).width(LengthSpec::Px(360.0)),
            )
            .unwrap();
        let control = context
            .create_detached_component(document(), crate::Switch::new("enabled", true))
            .unwrap();
        let row = context
            .mount_settings_leaf_row(document(), "Preference", None, control.stable_id())
            .unwrap();
        context
            .update_component(row, |row, _| row.stack_below = Some(480.0))
            .unwrap();
        context.append_child(parent, row).unwrap();
        let viewport = crate::LayoutViewport::new(1200.0, 800.0);
        let copy = context.read(row, |row| row.copy_slot.unwrap()).unwrap();
        for (width, stacked) in [(360.0, true), (480.0, false), (640.0, false), (479.0, true)] {
            context
                .update_component(parent, |parent, _| {
                    *parent = crate::Stack::column(0.0).width(LengthSpec::Px(width));
                })
                .unwrap();
            // The frame driver settles layout feedback before publishing Scene.
            context.layout_document(document(), viewport).unwrap();
            context.layout_document(document(), viewport).unwrap();
            let row_box = context
                .world()
                .canonical_layout_box(row.stable_id())
                .unwrap();
            assert_eq!(row_box.width, width);
            let control_box = context
                .world()
                .canonical_layout_box(control.stable_id())
                .unwrap();
            let copy_box = context.world().canonical_layout_box(copy).unwrap();
            if stacked {
                assert!(control_box.y >= copy_box.y + copy_box.height);
            } else {
                assert!(control_box.x >= copy_box.x + copy_box.width);
            }
            assert!(context.read(control, |control| control.checked).unwrap());
        }
        context
            .update_component(row, |row, _| {
                row.stack_below = None;
            })
            .unwrap();
        assert_eq!(
            context
                .world()
                .node_style(row.stable_id())
                .unwrap()
                .layout
                .direction,
            Some(FlexDirection::Row)
        );
        context
            .update_component(row, |row, _| {
                row.stacked = true;
                row.stack_below = Some(1.0);
            })
            .unwrap();
        assert_eq!(
            context
                .world()
                .node_style(row.stable_id())
                .unwrap()
                .layout
                .direction,
            Some(FlexDirection::Column)
        );
    }

    /// A row in a 360-wide column holding `control`, laid out as a frame
    /// driver settles it: the row, its label and the control.
    fn laid_out_row(
        context: &mut AppContext,
        control: StableNodeId,
        hint: Option<&str>,
        edit: impl FnOnce(&mut SettingsRow),
    ) -> (Entity<SettingsRow>, StableNodeId) {
        let parent = context
            .create_component(
                document(),
                crate::Stack::column(0.0).width(LengthSpec::Px(360.0)),
            )
            .unwrap();
        let row = context
            .mount_settings_leaf_row(document(), "Preference", hint, control)
            .unwrap();
        context
            .update_component(row, |row, _| {
                row.stack_below = Some(ASSEMBLED_ROW_STACK_BELOW);
                edit(row);
            })
            .unwrap();
        context.assemble_settings_row(row).unwrap();
        context.append_child(parent, row).unwrap();
        let viewport = crate::LayoutViewport::new(1200.0, 800.0);
        context.layout_document(document(), viewport).unwrap();
        context.layout_document(document(), viewport).unwrap();
        let label = context.read(row, |row| row.label_slot.unwrap()).unwrap();
        (row, label)
    }

    fn middle(context: &AppContext, id: StableNodeId) -> f32 {
        let bounds = context.world().canonical_layout_box(id).unwrap();
        bounds.y + bounds.height / 2.0
    }

    fn row_controls(context: &mut AppContext) -> Vec<(&'static str, StableNodeId)> {
        let switch = context
            .create_detached_component(document(), crate::Switch::new("", true))
            .unwrap()
            .stable_id();
        let select = context
            .create_detached_component(
                document(),
                crate::Select::new(Some("a")).options([
                    crate::SelectOption::new("a", "Alpha"),
                    crate::SelectOption::new("b", "Beta"),
                ]),
            )
            .unwrap()
            .stable_id();
        let field = context
            .create_detached_component(document(), crate::TextInput::new("value"))
            .unwrap()
            .stable_id();
        vec![("switch", switch), ("select", select), ("field", field)]
    }

    /// A hint under the name lowers nothing: the control stays centred on
    /// the whole copy, so the row's two sides share one middle.
    #[test]
    fn an_inline_row_with_a_hint_centres_its_control_on_the_copy() {
        let mut context = AppContext::new();
        for (kind, control) in row_controls(&mut context) {
            let (row, label) = laid_out_row(
                &mut context,
                control,
                Some("What this preference changes"),
                |_| {},
            );
            let style = context.world().node_style(row.stable_id()).unwrap();
            assert_eq!(style.layout.align_items, AlignSpec::Center, "{kind}");
            let copy = context.read(row, |row| row.copy_slot).unwrap().unwrap();
            let offset = middle(&context, control) - middle(&context, copy);
            assert!(offset.abs() <= 1.0, "{kind}: {offset}");
            // the hint stays right under the name, the copy's own gap away
            let hint = hint_slot_of(&context, row.stable_id()).unwrap();
            let hint_box = context.world().canonical_layout_box(hint).unwrap();
            let label_box = context.world().canonical_layout_box(label).unwrap();
            let gap = hint_box.y - (label_box.y + label_box.height);
            assert!((gap - ROW_COPY_GAP).abs() <= 1.0, "{kind}: {gap}");
        }
    }

    /// One line of copy, an error and label-column rows centre their
    /// control; stacked rows stretch it. A row with an error holds its copy
    /// and control on a main line above the notice, and that line centres.
    #[test]
    fn rows_keep_their_alignment_whatever_the_copy() {
        let mut context = AppContext::new();
        let cases: [(&str, Option<&str>, fn(&mut SettingsRow), AlignSpec); 4] = [
            ("no hint", None, |_| {}, AlignSpec::Center),
            (
                "error",
                None,
                |row| row.error = Some(Arc::from("Required")),
                AlignSpec::Center,
            ),
            (
                "stacked",
                Some("Hint"),
                |row| row.stacked = true,
                AlignSpec::Stretch,
            ),
            (
                "label column",
                Some("Hint"),
                |row| row.label_column = Some(96.0),
                AlignSpec::Center,
            ),
        ];
        for (case, hint, edit, align) in cases {
            for (kind, control) in row_controls(&mut context) {
                let (row, label) = laid_out_row(&mut context, control, hint, edit);
                let line = context.read(row, |r| r.line_node(row.stable_id())).unwrap();
                let style = context.world().node_style(line).unwrap();
                assert_eq!(style.layout.align_items, align, "{case}, {kind}");
                if case == "error" {
                    // centred on the name, not on the name and the notice
                    let offset = middle(&context, control) - middle(&context, label);
                    assert!(offset.abs() <= 1.0, "{case}, {kind}: {offset}");
                }
            }
        }
        // a one-line row centres its control on the name, as it always has
        let control = row_controls(&mut context)[0].1;
        let (_, label) = laid_out_row(&mut context, control, None, |_| {});
        assert!((middle(&context, control) - middle(&context, label)).abs() <= 1.0);
    }

    #[test]
    fn settings_leaf_row_can_show_and_clear_a_hint_after_mounting_without_one() {
        let mut context = AppContext::new();
        let control = context
            .create_detached_component(document(), crate::Switch::new("enabled", true))
            .unwrap();
        let row = context
            .mount_settings_leaf_row(document(), "Preference", None, control.stable_id())
            .unwrap();
        let (hint, copy) = context
            .read(row, |row| (row.hint_slot.unwrap(), row.copy_slot.unwrap()))
            .unwrap();
        for next in [Some("Details"), None, Some("Updated details")] {
            context
                .update_component(row, |row, _| row.hint = next.map(Arc::from))
                .unwrap();
            assert_eq!(
                visible_hint_text(&context, row.stable_id()).as_deref(),
                next
            );
            assert_eq!(context.world().node(hint).unwrap().parent, Some(copy));
            assert_eq!(
                context.read(row, |row| row.control).unwrap(),
                Some(control.stable_id())
            );
        }
    }

    /// `divided` separates a row from the next one. It used to stroke all four
    /// sides, so a card holding a single row showed a square frame inside its
    /// rounded corners.
    #[test]
    fn a_divided_row_draws_a_line_under_itself_not_a_frame() {
        let mut context = AppContext::new();
        let row = context
            .create_component(document(), SettingsRow::new("独立捕获窗口").divided(true))
            .unwrap();
        let style = context.world().node_style(row.stable_id()).unwrap();
        let edges = style.layout.resolved_border_edges();
        assert_eq!(
            (edges.top, edges.right, edges.bottom, edges.left),
            (0.0, 0.0, nana_ui_core::HAIRLINE, 0.0)
        );
        assert_eq!(style.border, Some(SemanticColorRole::BorderSoft));
    }

    #[test]
    fn settings_row_keeps_slots_and_chrome_flags() {
        let mut context = AppContext::new();
        let control = context
            .create_component(document(), crate::Switch::new("跟随", true))
            .unwrap();
        let row = context
            .create_component(
                document(),
                SettingsRow::new("标题栏跟随侧边栏透明")
                    .hint("仅在侧边栏使用透明材质时生效")
                    .stacked(true)
                    .divided(true)
                    .loose(true)
                    .first_in_group()
                    .last_in_group()
                    .control_child(control.stable_id()),
            )
            .unwrap();
        context.append_child(row, control).unwrap();
        context.reproject_component(row).unwrap();
        let id = row.stable_id();
        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "settings-row".into(),
            }
        );
        assert_eq!(context.world().text(id), Some(""));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.layout.direction, Some(FlexDirection::Column));
        assert_eq!(style.layout.justify_content, JustifySpec::Start);
        assert_eq!(style.layout.flex_wrap, FlexWrap::NoWrap);
        assert_eq!(style.layout.gap, Some(LengthSpec::Px(ROW_STACK_GAP_LOOSE)));
        assert_eq!(
            style.layout.padding_top,
            Some(LengthSpec::Px(ROW_MULTILINE_GROUP_PADDING_Y))
        );
        // Last in its group: the group edge ends it, so no divider.
        assert_eq!(style.border, None);
        assert!(!context.world().interaction(id).unwrap().pointer_events);
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.label.as_deref(), Some("标题栏跟随侧边栏透明"));
        assert_eq!(
            accessibility.value.as_deref(),
            Some("仅在侧边栏使用透明材质时生效")
        );
        context
            .read(row, |row| {
                assert_eq!(row.control, Some(control.stable_id()));
            })
            .unwrap();
    }

    #[test]
    fn settings_row_uses_compact_default_bands_and_keeps_authored_padding() {
        let mut context = AppContext::new();
        let middle = context
            .create_component(document(), SettingsRow::new("中间行"))
            .unwrap();
        let middle_style = context.world().node_style(middle.stable_id()).unwrap();
        assert_eq!(
            (
                middle_style.layout.padding_top,
                middle_style.layout.padding_bottom,
            ),
            (
                Some(LengthSpec::Px(nana_ui_core::space::SM)),
                Some(LengthSpec::Px(nana_ui_core::space::SM)),
            )
        );

        let mut custom = NodeStyle::default();
        custom.layout_mut().padding_top = Some(LengthSpec::Px(10.0));
        let authored = context
            .create_component(
                document(),
                SettingsRow::new("显式上留白")
                    .first_in_group()
                    .style(custom),
            )
            .unwrap();
        let authored_style = context.world().node_style(authored.stable_id()).unwrap();
        assert_eq!(
            authored_style.layout.padding_top,
            Some(LengthSpec::Px(10.0))
        );
        assert_eq!(
            authored_style.layout.padding_bottom,
            Some(LengthSpec::Px(nana_ui_core::space::SM))
        );

        let multiline = context
            .create_component(
                document(),
                SettingsRow::new("多行说明").hint("说明直接排在名称下方"),
            )
            .unwrap();
        let multiline_style = context.world().node_style(multiline.stable_id()).unwrap();
        assert_eq!(
            (
                multiline_style.layout.padding_top,
                multiline_style.layout.padding_bottom,
            ),
            (
                Some(LengthSpec::Px(nana_ui_core::space::LG)),
                Some(LengthSpec::Px(nana_ui_core::space::LG)),
            )
        );

        let help_inline = context
            .create_component(
                document(),
                SettingsRow::new("问号单行")
                    .hint("说明收进问号，单行保持紧凑留白")
                    .hint_placement(HintPlacement::Help),
            )
            .unwrap();
        let help_style = context.world().node_style(help_inline.stable_id()).unwrap();
        assert_eq!(
            (
                help_style.layout.padding_top,
                help_style.layout.padding_bottom,
            ),
            (
                Some(LengthSpec::Px(nana_ui_core::space::SM)),
                Some(LengthSpec::Px(nana_ui_core::space::SM)),
            )
        );
    }

    #[test]
    fn single_line_settings_rows_share_one_height_across_control_families() {
        let mut context = AppContext::new();
        let mut switch_view = crate::Switch::new("", true);
        switch_view.style.layout_mut().height = Some(LengthSpec::Px(24.0));
        switch_view.style.layout_mut().min_height = Some(LengthSpec::Px(24.0));
        let switch = context
            .create_detached_component(document(), switch_view)
            .unwrap()
            .stable_id();
        let select = context
            .create_detached_component(
                document(),
                crate::Select::new(Some("a"))
                    .size(ControlSize::Small)
                    .options([crate::SelectOption::new("a", "Alpha")]),
            )
            .unwrap()
            .stable_id();
        let text_value = context
            .create_detached_component(document(), Text::new("v1.0"))
            .unwrap()
            .stable_id();
        let expected_middle = UI_METRICS.compact_control_height + 2.0 * ROW_PADDING_Y;
        for (kind, control) in [("switch", switch), ("select", select), ("text", text_value)] {
            let (row, _) = laid_out_row(&mut context, control, None, |row| {
                row.first_in_group = false;
                row.last_in_group = false;
            });
            let bounds = context
                .world()
                .canonical_layout_box(row.stable_id())
                .unwrap();
            assert!(
                (bounds.height - expected_middle).abs() < 0.5,
                "{kind} middle row height {} != {expected_middle}",
                bounds.height
            );
        }
    }

    fn hint_slot_of(context: &AppContext, row: StableNodeId) -> Option<StableNodeId> {
        context
            .read(Entity::<SettingsRow>::from_stable_id(row), |row| {
                row.hint_slot
            })
            .ok()
            .flatten()
    }

    fn visible_hint_text(context: &AppContext, row: StableNodeId) -> Option<String> {
        let hint = hint_slot_of(context, row)?;
        let style = context.world().node_style(hint)?;
        if style.layout.hidden {
            return None;
        }
        let text = context.world().text(hint)?.to_owned();
        (!text.is_empty()).then_some(text)
    }

    #[test]
    fn settings_row_paints_muted_hint_child() {
        let mut context = AppContext::new();
        let control = context
            .create_component(document(), crate::Switch::new("跟随", true))
            .unwrap();
        let label = context
            .create_component(document(), row_label_text("主题"))
            .unwrap();
        let hint = context
            .create_component(document(), row_hint_text("选择应用配色，立即生效", false))
            .unwrap();
        let row = context
            .create_component(
                document(),
                SettingsRow::new("主题")
                    .hint("选择应用配色，立即生效")
                    .label_slot(label.stable_id())
                    .hint_slot(hint.stable_id())
                    .control_child(control.stable_id()),
            )
            .unwrap();
        context.append_child(row, label).unwrap();
        context.append_child(row, hint).unwrap();
        context.append_child(row, control).unwrap();
        context.reproject_component(row).unwrap();

        assert_eq!(context.world().text(row.stable_id()), Some(""));
        let row_style = context.world().node_style(row.stable_id()).unwrap();
        assert_eq!(row_style.layout.direction, Some(FlexDirection::Row));
        assert_eq!(row_style.layout.justify_content, JustifySpec::SpaceBetween);
        assert_eq!(row_style.layout.flex_wrap, FlexWrap::NoWrap);
        assert_eq!(row_style.layout.align_items, AlignSpec::Center);
        assert_eq!(context.world().text(label.stable_id()), Some("主题"));
        let label_style = context.world().node_style(label.stable_id()).unwrap();
        assert_eq!(
            label_style.layout.font_size,
            Some(nana_ui_core::type_scale::BODY)
        );
        assert_eq!(
            label_style.layout.line_height,
            Some(nana_ui_core::LineHeightSpec::Absolute(
                nana_ui_core::type_scale::BODY
            ))
        );
        assert_eq!(
            label_style.layout.font_weight,
            Some(nana_ui_core::type_scale::MEDIUM)
        );
        assert_eq!(label_style.foreground, Some(SemanticColorRole::Text));
        assert_eq!(
            label_style.layout.white_space,
            nana_ui_core::WhiteSpaceSpec::Nowrap
        );
        assert!(label_style.layout.text_overflow_ellipsis);
        assert_eq!(label_style.layout.width, Some(LengthSpec::Fill));

        assert_eq!(
            context.world().text(hint.stable_id()),
            Some("选择应用配色，立即生效")
        );
        let hint_style = context.world().node_style(hint.stable_id()).unwrap();
        assert_eq!(
            hint_style.layout.font_size,
            Some(nana_ui_core::type_scale::META)
        );
        assert_eq!(hint_style.foreground, Some(SemanticColorRole::Muted));
        assert!(
            hint_style.layout.text_wraps() && !hint_style.layout.text_overflow_ellipsis,
            "a hint wraps under the label instead of being cut"
        );
        assert!(!hint_style.layout.hidden);
        assert_eq!(
            visible_hint_text(&context, row.stable_id()).as_deref(),
            Some("选择应用配色，立即生效")
        );
    }

    #[test]
    fn settings_row_without_hint_has_no_visible_hint_text() {
        let mut context = AppContext::new();
        let control = context
            .create_component(document(), crate::Switch::new("圆角", true))
            .unwrap();
        let label = context
            .create_component(document(), row_label_text("工作区边缘"))
            .unwrap();
        let leftover = context
            .create_component(document(), row_hint_text("不应显示", false))
            .unwrap();
        let row = context
            .create_component(
                document(),
                SettingsRow::new("工作区边缘")
                    .label_slot(label.stable_id())
                    .hint_slot(leftover.stable_id())
                    .control_child(control.stable_id()),
            )
            .unwrap();
        context.append_child(row, label).unwrap();
        context.append_child(row, leftover).unwrap();
        context.append_child(row, control).unwrap();
        context.reproject_component(row).unwrap();

        assert_eq!(context.world().text(row.stable_id()), Some(""));
        assert_eq!(context.world().text(leftover.stable_id()), Some(""));
        assert!(
            context
                .world()
                .node_style(leftover.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert_eq!(visible_hint_text(&context, row.stable_id()), None);
    }

    #[test]
    fn settings_card_reuses_surface_card_paint() {
        let mut context = AppContext::new();
        let card = context
            .create_component(document(), SettingsCard::new("外观"))
            .unwrap();
        let id = card.stable_id();
        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "settings-card".into(),
            }
        );
        assert!(matches!(
            context.world().widget_visual_as::<crate::CardVisual>(id),
            Some(crate::CardVisual {
                title,
                kind: CardKind::Surface,
                loading: false,
                ..
            }) if title.as_deref() == Some("外观")
        ));
        assert_eq!(
            context.world().node_style(id).unwrap().layout.margin_bottom,
            None
        );
    }

    #[test]
    fn collapsible_card_toggles_until_disabled() {
        let mut context = AppContext::new();
        let card = context
            .create_component(document(), SettingsCollapsibleCard::new(false))
            .unwrap();
        let id = card.stable_id();
        assert!(context.world().interaction(id).unwrap().focusable);
        assert_eq!(
            context.world().accessibility(id).unwrap().selected,
            Some(false)
        );
        assert!(context.activate_settings_collapsible_card(card).unwrap());
        context.read(card, |card| assert!(card.expanded)).unwrap();
        assert_eq!(
            context.world().accessibility(id).unwrap().selected,
            Some(true)
        );

        let locked = context
            .create_component(
                document(),
                SettingsCollapsibleCard::new(true).disabled(true),
            )
            .unwrap();
        assert!(
            !context
                .world()
                .interaction(locked.stable_id())
                .unwrap()
                .focusable
        );
        assert!(!context.activate_settings_collapsible_card(locked).unwrap());
        context.read(locked, |card| assert!(card.expanded)).unwrap();
    }

    #[test]
    fn appearance_events_stay_on_the_host_snapshot() {
        let mut theme = nana_ui_core::ThemeId::new("nana.dark");
        let mut appearance = AppearanceSettings::default();
        assert!(apply_appearance_event(
            &mut theme,
            &mut appearance,
            AppearanceEvent::Theme(nana_ui_core::ThemeId::new("nana.light")),
        ));
        assert_eq!(theme, nana_ui_core::ThemeId::new("nana.light"));
        assert!(apply_appearance_event(
            &mut theme,
            &mut appearance,
            AppearanceEvent::WindowMaterial(WindowMaterialMode::Translucent),
        ));
        assert_eq!(
            appearance.window_material(),
            WindowMaterialMode::Translucent
        );
        assert!(apply_appearance_event(
            &mut theme,
            &mut appearance,
            AppearanceEvent::BackdropTarget(BackdropTarget::Main),
        ));
        assert_eq!(appearance.backdrop_target(), BackdropTarget::Main);
        assert!(apply_appearance_event(
            &mut theme,
            &mut appearance,
            AppearanceEvent::Reset,
        ));
        assert_eq!(theme, nana_ui_core::ThemeId::new("nana.light"));
        assert_eq!(appearance, AppearanceSettings::default());

        let mut context = AppContext::new();
        let section = context
            .create_component(
                document(),
                AppearanceSection::new(ThemeAppearance::Dark, AppearanceSettings::default())
                    .platform_hint("选择窗口使用的透明材质或实色背景。"),
            )
            .unwrap();
        assert_eq!(
            context.world().node(section.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "appearance-section".into(),
            }
        );
        assert!(matches!(
            context
                .world()
                .widget_visual_as::<crate::CardVisual>(section.stable_id()),
            Some(crate::CardVisual {
                title,
                kind: CardKind::Surface,
                ..
            }) if title.is_none()
        ));
        assert_eq!(
            context.world().extract_nodes(&[section.stable_id()])[0]
                .source_style
                .layout
                .resolved_padding()
                .top,
            UI_METRICS.panel_padding_y
        );

        let custom_id = nana_ui_core::ThemeId::from_owned("user.sunset");
        let custom_section =
            AppearanceSection::new(ThemeAppearance::Dark, AppearanceSettings::default())
                .with_theme_options(
                    custom_id.clone(),
                    vec![
                        nana_ui_core::ThemeChoice {
                            id: nana_ui_core::ThemeId::new("nana.light"),
                            label: "浅色".into(),
                            appearance: nana_ui_core::ThemeAppearance::Light,
                        },
                        nana_ui_core::ThemeChoice {
                            id: custom_id.clone(),
                            label: "夕暮".into(),
                            appearance: nana_ui_core::ThemeAppearance::Custom,
                        },
                    ],
                );
        assert_eq!(custom_section.theme_id, custom_id);
        assert_eq!(custom_section.theme_options.len(), 2);
    }

    fn row_labels(context: &AppContext, section: Entity<AppearanceSection>) -> Vec<String> {
        context
            .world()
            .node(section.stable_id())
            .unwrap()
            .children
            .iter()
            .filter_map(|id| {
                context
                    .read(Entity::<SettingsRow>::from_stable_id(*id), |row| {
                        row.label.to_string()
                    })
                    .ok()
            })
            .collect()
    }

    fn assembly_of(
        context: &AppContext,
        section: Entity<AppearanceSection>,
    ) -> AppearanceSectionAssembly {
        context
            .read(section, |section| section.assembly.clone().unwrap())
            .unwrap()
    }

    fn option_disabled(context: &AppContext, id: Option<StableNodeId>) -> bool {
        context
            .read(
                Entity::<SegmentedOption>::from_stable_id(id.unwrap()),
                |option| option.disabled,
            )
            .unwrap()
    }

    fn switch_disabled(context: &AppContext, id: Option<StableNodeId>) -> bool {
        context
            .read(Entity::<Switch>::from_stable_id(id.unwrap()), |switch| {
                switch.disabled
            })
            .unwrap()
    }

    #[test]
    fn appearance_assemble_creates_rows_and_disabled_rules() {
        let mut context = AppContext::new();
        let section = context
            .create_component(
                document(),
                AppearanceSection::new(ThemeAppearance::Dark, AppearanceSettings::default()),
            )
            .unwrap();
        assert!(context.assemble_appearance_section(section).unwrap());
        assert_eq!(
            row_labels(&context, section),
            [
                "主题",
                "窗口材质",
                "透明区域",
                "标题栏跟随侧边栏透明",
                "材质不透明度",
                "主区域圆角",
                "组件圆角半径",
                "默认样式",
            ]
        );
        let solid = assembly_of(&context, section);
        assert_eq!(
            visible_hint_text(&context, solid.theme_row.unwrap()).as_deref(),
            Some("选择应用配色，立即生效")
        );
        let theme_hint = hint_slot_of(&context, solid.theme_row.unwrap()).unwrap();
        let theme_hint_style = context.world().node_style(theme_hint).unwrap();
        assert_eq!(
            theme_hint_style.layout.font_size,
            Some(nana_ui_core::type_scale::META)
        );
        assert_eq!(theme_hint_style.foreground, Some(SemanticColorRole::Muted));
        assert_eq!(
            visible_hint_text(&context, solid.workspace_row.unwrap()),
            None
        );
        assert_eq!(visible_hint_text(&context, solid.radius_row.unwrap()), None);
        assert!(option_disabled(&context, solid.target_sidebar));
        assert!(option_disabled(&context, solid.target_main));
        assert!(switch_disabled(&context, solid.titlebar_switch));
        assert!(solid.opacity_text.is_none());
        assert!(
            context
                .read(
                    Entity::<RangeField>::from_stable_id(solid.opacity_range.unwrap()),
                    |range| range.disabled
                )
                .unwrap()
        );
        assert!(!switch_disabled(&context, solid.workspace_switch));

        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        context
            .on(section, move |_, event: &AppearanceEvent, _| {
                sink.lock().unwrap().push(event.clone());
            })
            .unwrap();
        assert!(
            context
                .toggle_switch(Entity::from_stable_id(solid.workspace_switch.unwrap()))
                .unwrap()
        );
        assert_eq!(
            *events.lock().unwrap(),
            vec![AppearanceEvent::WorkspaceCorners(false)]
        );

        let mut appearance = AppearanceSettings::default();
        appearance.set_window_material(WindowMaterialMode::Translucent);
        context
            .update_component(section, |section, _| {
                section.appearance = appearance;
                section.material_status = Some(Arc::from("transparent"));
            })
            .unwrap();
        context.assemble_appearance_section(section).unwrap();
        let translucent = assembly_of(&context, section);
        assert!(!option_disabled(&context, translucent.target_sidebar));
        assert!(!option_disabled(&context, translucent.target_main));
        assert!(!switch_disabled(&context, translucent.titlebar_switch));
        assert!(translucent.opacity_range.is_some());

        let mut appearance = AppearanceSettings::default();
        appearance.set_window_material(WindowMaterialMode::Mica);
        context
            .update_component(section, |section, _| {
                section.appearance = appearance;
                section.material_status = Some(Arc::from("mica"));
            })
            .unwrap();
        context.assemble_appearance_section(section).unwrap();
        assert_eq!(
            row_labels(&context, section),
            [
                "主题",
                "窗口材质",
                "材质状态",
                "透明区域",
                "标题栏跟随侧边栏透明",
                "材质不透明度",
                "主区域圆角",
                "组件圆角半径",
                "默认样式",
            ]
        );
        let translucent_sidebar = assembly_of(&context, section);
        assert!(!option_disabled(
            &context,
            translucent_sidebar.target_sidebar
        ));
        assert!(!option_disabled(&context, translucent_sidebar.target_main));
        assert!(!switch_disabled(
            &context,
            translucent_sidebar.titlebar_switch
        ));
        assert!(translucent_sidebar.opacity_range.is_some());
        assert!(
            !context
                .world()
                .node_style(translucent_sidebar.opacity_range.unwrap())
                .unwrap()
                .layout
                .hidden
        );
        let range_parent = context
            .world()
            .node(translucent_sidebar.opacity_row.unwrap())
            .unwrap()
            .children
            .last()
            .copied();
        assert_eq!(range_parent, translucent_sidebar.opacity_range);

        appearance.set_backdrop_target(BackdropTarget::Main);
        context
            .update_component(section, |section, _| {
                section.appearance = appearance;
            })
            .unwrap();
        context.assemble_appearance_section(section).unwrap();
        let translucent_main = assembly_of(&context, section);
        assert!(!option_disabled(&context, translucent_main.target_sidebar));
        assert!(switch_disabled(&context, translucent_main.titlebar_switch));
    }

    #[test]
    fn appearance_offers_host_native_materials_and_emits_selection() {
        let mut context = AppContext::new();
        let section = context
            .create_component(
                document(),
                AppearanceSection::new(ThemeAppearance::Dark, AppearanceSettings::default())
                    .available_materials(vec![
                        WindowMaterialMode::Mica,
                        WindowMaterialMode::Acrylic,
                    ]),
            )
            .unwrap();
        assert!(context.assemble_appearance_section(section).unwrap());
        let assembly = assembly_of(&context, section);
        assert_eq!(
            context
                .world()
                .node(assembly.material_control.unwrap())
                .unwrap()
                .children
                .as_slice(),
            &[
                assembly.material_solid.unwrap(),
                assembly.material_translucent.unwrap(),
                assembly.material_mica.unwrap(),
                assembly.material_acrylic.unwrap(),
            ]
        );

        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        context
            .on(section, move |_, event: &AppearanceEvent, _| {
                sink.lock().unwrap().push(event.clone());
            })
            .unwrap();
        assert!(
            context
                .activate_segmented_option(Entity::from_stable_id(assembly.material_mica.unwrap()))
                .unwrap()
        );
        assert_eq!(
            *events.lock().unwrap(),
            vec![AppearanceEvent::WindowMaterial(WindowMaterialMode::Mica)]
        );
    }

    #[test]
    fn about_section_shows_injected_metadata() {
        let mut context = AppContext::new();
        let metadata =
            AboutMetadata::new("Fixture Product", "9.9.9").description("Host-owned blurb");
        let section = context
            .create_component(document(), AboutSection::new(metadata.clone()))
            .unwrap();
        assert!(context.assemble_about_section(section).unwrap());
        let assembly = context
            .read(section, |section| section.assembly.clone().unwrap())
            .unwrap();
        assert_eq!(
            context.world().text(assembly.name_value.unwrap()),
            Some("Fixture Product")
        );
        assert_eq!(
            context.world().text(assembly.version_value.unwrap()),
            Some("9.9.9")
        );
        assert_eq!(
            visible_hint_text(&context, assembly.name_row.unwrap()).as_deref(),
            Some("Host-owned blurb")
        );
        assert!(assembly.description.is_none());
        let labels = context
            .world()
            .node(section.stable_id())
            .unwrap()
            .children
            .iter()
            .filter_map(|id| {
                context
                    .read(Entity::<SettingsRow>::from_stable_id(*id), |row| {
                        row.label.to_string()
                    })
                    .ok()
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            context
                .world()
                .widget_visual_as::<crate::CardVisual>(section.stable_id()),
            Some(crate::CardVisual {
                title,
                kind: CardKind::Surface,
                ..
            }) if title.is_none()
        ));
        assert_eq!(
            context.world().extract_nodes(&[section.stable_id()])[0]
                .source_style
                .layout
                .resolved_padding()
                .top,
            UI_METRICS.panel_padding_y
        );
        assert_eq!(labels, ["名称", "版本"]);
        assert_eq!(
            visible_hint_text(&context, assembly.version_row.unwrap()),
            None
        );
        let mut texts = Vec::new();
        let mut stack = context
            .world()
            .node(section.stable_id())
            .unwrap()
            .children
            .clone();
        while let Some(id) = stack.pop() {
            if let Some(text) = context.world().text(id) {
                texts.push(text.to_owned());
            }
            if let Some(node) = context.world().node(id) {
                stack.extend(node.children.iter().copied());
            }
        }
        assert!(texts.iter().any(|text| text == "Fixture Product"));
        assert!(texts.iter().all(|text| !text.contains("NanaUI")));
    }

    #[test]
    fn collapsible_card_hides_details_until_expanded() {
        let mut context = AppContext::new();
        let summary = context
            .create_component(document(), Text::new("高级"))
            .unwrap();
        let details = context
            .create_component(document(), Text::new("明细"))
            .unwrap();
        let card = context
            .create_component(
                document(),
                SettingsCollapsibleCard::new(false)
                    .summary(summary.stable_id())
                    .details(details.stable_id()),
            )
            .unwrap();
        assert!(context.assemble_settings_collapsible_card(card).unwrap());
        let (header, disclosure, divider) = context
            .read(card, |card| (card.header, card.disclosure, card.divider))
            .unwrap();
        assert_eq!(
            context.world().node(card.stable_id()).unwrap().children,
            vec![header.unwrap(), divider.unwrap(), details.stable_id()]
        );
        assert_eq!(
            context.world().node(header.unwrap()).unwrap().children,
            vec![summary.stable_id(), disclosure.unwrap()]
        );
        assert_eq!(
            context
                .world()
                .node_style(summary.stable_id())
                .unwrap()
                .foreground,
            Some(SemanticColorRole::Text)
        );
        assert_eq!(
            context
                .world()
                .widget_visual_as::<crate::IconVisual>(disclosure.unwrap()),
            Some(&crate::IconVisual {
                icon: Icon::ChevronRight,
                size: DISCLOSURE_ICON_SIZE,
                tooltip: None,
            })
        );
        assert!(
            context
                .world()
                .node_style(disclosure.unwrap())
                .unwrap()
                .layout
                .transforms
                .transform
                .is_none()
        );
        assert!(
            context
                .world()
                .node_style(details.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert!(
            context
                .world()
                .node_style(divider.unwrap())
                .unwrap()
                .layout
                .hidden
        );
        assert!(context.activate_settings_collapsible_card(card).unwrap());
        assert_eq!(
            context
                .world()
                .widget_visual_as::<crate::IconVisual>(disclosure.unwrap()),
            Some(&crate::IconVisual {
                icon: Icon::ChevronDown,
                size: DISCLOSURE_ICON_SIZE,
                tooltip: None,
            })
        );
        assert!(
            context
                .world()
                .node_style(disclosure.unwrap())
                .unwrap()
                .layout
                .transforms
                .transform
                .is_none()
        );
        assert!(
            !context
                .world()
                .node_style(details.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert!(
            !context
                .world()
                .node_style(divider.unwrap())
                .unwrap()
                .layout
                .hidden
        );

        let locked_details = context
            .create_component(document(), Text::new("锁定明细"))
            .unwrap();
        let locked = context
            .create_component(
                document(),
                SettingsCollapsibleCard::new(true)
                    .disabled(true)
                    .details(locked_details.stable_id()),
            )
            .unwrap();
        context.assemble_settings_collapsible_card(locked).unwrap();
        assert!(
            !context
                .world()
                .node_style(locked_details.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert!(!context.activate_settings_collapsible_card(locked).unwrap());
        context.read(locked, |card| assert!(card.expanded)).unwrap();
        assert!(
            !context
                .world()
                .node_style(locked_details.stable_id())
                .unwrap()
                .layout
                .hidden
        );
    }

    fn settings_model(tabs: impl IntoIterator<Item = nana_ui_core::SettingsTab>) -> SettingsModel {
        let tabs: Vec<_> = tabs.into_iter().collect();
        let default = tabs[0].id().clone();
        SettingsModel::new(default, tabs).expect("valid settings model")
    }

    fn is_descendant(context: &AppContext, root: StableNodeId, target: StableNodeId) -> bool {
        let mut current = Some(target);
        while let Some(id) = current {
            if id == root {
                return true;
            }
            current = context.world().parent_id(id);
        }
        false
    }

    fn sidebar_row_state(context: &AppContext, id: StableNodeId) -> SidebarRowState {
        context
            .read(Entity::<SidebarRow>::from_stable_id(id), |row| row.state)
            .unwrap()
    }

    #[test]
    fn settings_sidebar_mounts_back_and_tab_rows() {
        let mut context = AppContext::new();
        let model = settings_model([
            nana_ui_core::SettingsTab::new("appearance", "外观").icon(Icon::Appearance),
            nana_ui_core::SettingsTab::new("workspace", "工作区").icon(Icon::Workspace),
        ]);
        let state = SettingsState::new(&model);
        let sidebar = context
            .create_component(document(), SettingsSidebar::new(model, state))
            .unwrap();
        assert!(context.assemble_settings_sidebar(sidebar).unwrap());
        let assembly = context
            .read(sidebar, |sidebar| sidebar.assembly.clone().unwrap())
            .unwrap();
        let back = assembly.back_row.unwrap();
        let body = assembly.body.unwrap();
        let appearance = assembly
            .tab_rows
            .get(&SettingsTabId::from("appearance"))
            .copied()
            .unwrap();
        let workspace = assembly
            .tab_rows
            .get(&SettingsTabId::from("workspace"))
            .copied()
            .unwrap();
        assert_eq!(
            context.world().node(sidebar.stable_id()).unwrap().children,
            vec![back, body]
        );
        assert_eq!(
            context.world().node(body).unwrap().kind,
            NodeKind::Element {
                tag: "scroll".into(),
            }
        );
        assert_eq!(
            context.world().node(body).unwrap().children,
            vec![appearance, workspace]
        );
        assert_eq!(context.world().text(back), Some("返回"));
        assert_eq!(
            sidebar_row_state(&context, appearance),
            SidebarRowState::Active
        );
        assert_eq!(
            sidebar_row_state(&context, workspace),
            SidebarRowState::Idle
        );
        assert_eq!(
            context
                .world()
                .node_style(sidebar.stable_id())
                .unwrap()
                .layout
                .gap,
            Some(LengthSpec::Px(SETTINGS_SIDEBAR_GAP))
        );
        assert!(
            context
                .world()
                .node_style(body)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert!(
            !context
                .world()
                .node_style(back)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
    }

    #[test]
    fn settings_sidebar_activate_selects_tab_and_reassemble_marks_active() {
        let mut context = AppContext::new();
        let model = settings_model([
            nana_ui_core::SettingsTab::new("appearance", "外观"),
            nana_ui_core::SettingsTab::new("workspace", "工作区"),
        ]);
        let state = SettingsState::new(&model);
        let sidebar = context
            .create_component(document(), SettingsSidebar::new(model, state))
            .unwrap();
        context.assemble_settings_sidebar(sidebar).unwrap();
        let selected = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&selected);
        context
            .on(sidebar, move |_, event: &SettingsTabSelected, _| {
                sink.lock().unwrap().push(event.tab.clone());
            })
            .unwrap();
        let workspace = context
            .read(sidebar, |sidebar| {
                sidebar.row_for_tab(&SettingsTabId::from("workspace"))
            })
            .unwrap()
            .unwrap();
        assert!(
            context
                .activate_sidebar_row(Entity::from_stable_id(workspace))
                .unwrap()
        );
        assert_eq!(
            *selected.lock().unwrap(),
            vec![SettingsTabId::from("workspace")]
        );
        context
            .update_component(sidebar, |sidebar, _| {
                let model = sidebar.model.clone();
                sidebar
                    .state
                    .select(&model, &SettingsTabId::from("workspace"));
            })
            .unwrap();
        context.assemble_settings_sidebar(sidebar).unwrap();
        let appearance = context
            .read(sidebar, |sidebar| {
                sidebar.row_for_tab(&SettingsTabId::from("appearance"))
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            sidebar_row_state(&context, appearance),
            SidebarRowState::Idle
        );
        assert_eq!(
            sidebar_row_state(&context, workspace),
            SidebarRowState::Active
        );
    }

    #[test]
    fn settings_sidebar_missing_icon_is_spacer_without_glyph() {
        let mut context = AppContext::new();
        let model = settings_model([
            nana_ui_core::SettingsTab::new("appearance", "外观").icon(Icon::Appearance),
            nana_ui_core::SettingsTab::new("about", "关于"),
        ]);
        let state = SettingsState::new(&model);
        let sidebar = context
            .create_component(document(), SettingsSidebar::new(model, state))
            .unwrap();
        context.assemble_settings_sidebar(sidebar).unwrap();
        let assembly = context
            .read(sidebar, |sidebar| sidebar.assembly.clone().unwrap())
            .unwrap();
        let icon = assembly
            .tab_leadings
            .get(&SettingsTabId::from("appearance"))
            .copied()
            .unwrap();
        let spacer = assembly
            .tab_leadings
            .get(&SettingsTabId::from("about"))
            .copied()
            .unwrap();
        assert_eq!(
            context.world().widget_visual_as::<crate::IconVisual>(icon),
            Some(&crate::IconVisual {
                icon: Icon::Appearance,
                size: SETTINGS_SIDEBAR_ICON_SIZE,
                tooltip: None,
            })
        );
        assert!(context.world().widget_visual(spacer).is_none());
        let spacer_layout = &context.world().node_style(spacer).unwrap().layout;
        assert_eq!(
            spacer_layout.width,
            Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE))
        );
        assert_eq!(
            spacer_layout.height,
            Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE))
        );
    }

    #[test]
    fn settings_page_header_title_and_scroll_wraps_content() {
        let mut context = AppContext::new();
        let model = settings_model([nana_ui_core::SettingsTab::new("appearance", "外观")]);
        let state = SettingsState::new(&model);
        let content = context
            .create_component(document(), Text::new("内容"))
            .unwrap();
        let page = context
            .create_component(
                document(),
                SettingsPage::new(model, state).content(content.stable_id()),
            )
            .unwrap();
        assert!(context.assemble_settings_page(page).unwrap());
        let assembly = context
            .read(page, |page| page.assembly.clone().unwrap())
            .unwrap();
        let scroll = assembly.scroll.unwrap();
        let body = assembly.body.unwrap();
        let title = assembly.title.unwrap();
        assert_eq!(
            context.world().node(page.stable_id()).unwrap().children,
            vec![scroll]
        );
        assert_eq!(
            context.world().node(scroll).unwrap().kind,
            NodeKind::Element {
                tag: "scroll".into(),
            }
        );
        assert!(
            context
                .world()
                .node_style(scroll)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert_eq!(context.world().node(scroll).unwrap().children, vec![body]);
        assert_eq!(
            context.world().node(body).unwrap().children,
            vec![title, content.stable_id()]
        );
        assert_eq!(context.world().text(title), Some("外观"));
        let title_style = context.world().node_style(title).unwrap();
        assert_eq!(title_style.layout.font_size, Some(SETTINGS_PAGE_TITLE_SIZE));
        assert_eq!(
            title_style.layout.font_weight,
            Some(SETTINGS_PAGE_TITLE_WEIGHT)
        );
        assert_eq!(title_style.foreground, Some(SemanticColorRole::Text));
        assert_eq!(
            context
                .world()
                .node_style(page.stable_id())
                .unwrap()
                .background,
            Some(SemanticColorRole::Background)
        );
    }

    #[test]
    fn settings_page_title_keeps_the_style_it_was_given_across_tab_switches() {
        let mut context = AppContext::new();
        let model = settings_model([
            nana_ui_core::SettingsTab::new("appearance", "外观"),
            nana_ui_core::SettingsTab::new("about", "关于"),
        ]);
        let state = SettingsState::new(&model);
        let page = context
            .create_component(
                document(),
                SettingsPage::new(model.clone(), state)
                    .title_size(26.0)
                    .title_weight(700),
            )
            .unwrap();
        context.assemble_settings_page(page).unwrap();
        context
            .update_component(page, |page, _| {
                page.state
                    .select(&model, &nana_ui_core::SettingsTabId::from("about"));
            })
            .unwrap();
        context.assemble_settings_page(page).unwrap();
        let title = context
            .read(page, |page| page.assembly.clone().unwrap().title.unwrap())
            .unwrap();
        assert_eq!(context.world().text(title), Some("关于"));
        let title_style = context.world().node_style(title).unwrap();
        assert_eq!(title_style.layout.font_size, Some(26.0));
        assert_eq!(title_style.layout.font_weight, Some(700));
    }

    #[test]
    fn settings_page_full_page_fills_with_content_only() {
        let mut context = AppContext::new();
        let model =
            settings_model([nana_ui_core::SettingsTab::new("workspace", "工作区").full_page(true)]);
        let state = SettingsState::new(&model);
        let content = context
            .create_component(document(), Text::new("整页"))
            .unwrap();
        let page = context
            .create_component(
                document(),
                SettingsPage::new(model, state).content(content.stable_id()),
            )
            .unwrap();
        assert!(context.assemble_settings_page(page).unwrap());
        let assembly = context
            .read(page, |page| page.assembly.clone().unwrap())
            .unwrap();
        assert_eq!(
            context.world().node(page.stable_id()).unwrap().children,
            vec![content.stable_id()]
        );
        assert!(
            assembly.title.is_none()
                || !is_descendant(&context, page.stable_id(), assembly.title.unwrap())
        );
        if let Some(scroll) = assembly.scroll {
            assert!(!is_descendant(&context, page.stable_id(), scroll));
        }
        assert!(!is_descendant(
            &context,
            content.stable_id(),
            page.stable_id()
        ));
    }

    #[test]
    fn settings_page_hide_header_omits_title() {
        let mut context = AppContext::new();
        let model = settings_model([nana_ui_core::SettingsTab::new("appearance", "外观")])
            .hide_header(true);
        let state = SettingsState::new(&model);
        let content = context
            .create_component(document(), Text::new("内容"))
            .unwrap();
        let page = context
            .create_component(
                document(),
                SettingsPage::new(model, state).content(content.stable_id()),
            )
            .unwrap();
        assert!(context.assemble_settings_page(page).unwrap());
        let assembly = context
            .read(page, |page| page.assembly.clone().unwrap())
            .unwrap();
        let body = assembly.body.unwrap();
        assert_eq!(
            context.world().node(body).unwrap().children,
            vec![content.stable_id()]
        );
        assert!(
            assembly.title.is_none()
                || !is_descendant(&context, page.stable_id(), assembly.title.unwrap())
        );
        assert_eq!(
            context.world().node(page.stable_id()).unwrap().children,
            vec![assembly.scroll.unwrap()]
        );
    }
}

#[cfg(test)]
mod spacing_tests {
    use super::*;
    use crate::view_components::Text;
    use crate::{AccessibilityRole, AppContext, MutationQueue};
    use nana_ui_core::{
        AppearanceSettings, CardKind, LengthSpec, PaddingSpec, SettingsModel, SettingsState,
        ThemeAppearance, UI_METRICS,
    };

    #[test]
    fn spacing_settings_body_updates_without_replacing_scroll_or_content() {
        let document = crate::DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let model = SettingsModel::new(
            "appearance",
            [nana_ui_core::SettingsTab::new("appearance", "Appearance")],
        )
        .unwrap();
        let state = SettingsState::new(&model);
        let content = context
            .create_component(document, Text::new("Content"))
            .unwrap();
        let page = context
            .create_component(
                document,
                SettingsPage::new(model, state).content(content.stable_id()),
            )
            .unwrap();
        context.assemble_settings_page(page).unwrap();
        let first = context.read(page, |p| p.assembly.clone().unwrap()).unwrap();
        let body = first.body.unwrap();
        assert_eq!(
            context
                .world()
                .node_style(body)
                .unwrap()
                .layout
                .resolved_padding(),
            PaddingSpec {
                top: 20.0,
                right: 24.0,
                bottom: 24.0,
                left: 24.0
            }
        );
        let mut q = MutationQueue::new();
        q.set_scroll_offset(
            first.scroll.unwrap(),
            crate::ScrollOffset { x: 0.0, y: 30.0 },
        );
        context.compat_world_mut().commit(q).unwrap();
        context
            .update_component(page, |p, _| {
                p.content_padding = Some(PaddingSpec::uniform(0.0));
                p.content_gap = Some(7.0);
            })
            .unwrap();
        context.assemble_settings_page(page).unwrap();
        let second = context.read(page, |p| p.assembly.clone().unwrap()).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            context
                .world()
                .scroll_offset(first.scroll.unwrap())
                .unwrap()
                .y,
            30.0
        );
        let layout = &context.world().node_style(body).unwrap().layout;
        assert!(layout.resolved_padding().is_zero());
        assert_eq!(layout.gap, Some(LengthSpec::Px(7.0)));
        assert!(
            context
                .world()
                .node(body)
                .unwrap()
                .children
                .contains(&content.stable_id())
        );
        context
            .update_component(page, |p, _| {
                p.content_padding = None;
                p.content_gap = None;
            })
            .unwrap();
        context.assemble_settings_page(page).unwrap();
        let layout = &context.world().node_style(body).unwrap().layout;
        assert_eq!(layout.resolved_padding().bottom, 24.0);
        assert_eq!(layout.gap, Some(LengthSpec::Px(16.0)));
    }

    /// The appearance section's controls are named by the labels of the
    /// rows that hold them, as a view's settings rows are.
    #[test]
    fn appearance_controls_are_named_by_their_rows() {
        let document = crate::DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let section = context
            .create_component(
                document,
                AppearanceSection::new(ThemeAppearance::Dark, AppearanceSettings::default()),
            )
            .unwrap();
        context.assemble_appearance_section(section).unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(800.0, 900.0))
            .unwrap();
        let named = context
            .world()
            .project_accessibility(document)
            .into_iter()
            .filter(|node| {
                matches!(
                    node.role,
                    AccessibilityRole::RadioGroup
                        | AccessibilityRole::Switch
                        | AccessibilityRole::Slider
                )
            })
            .map(|node| (node.role, node.label.as_deref().unwrap_or("").to_owned()))
            .collect::<Vec<_>>();
        assert!(
            named.contains(&(AccessibilityRole::RadioGroup, "主题".into())),
            "{named:?}"
        );
        assert!(
            named.iter().all(|(_, label)| !label.is_empty()),
            "every control has a name: {named:?}"
        );
    }

    #[test]
    fn appearance_section_as_page_content_does_not_repeat_page_title() {
        let document = crate::DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let model = SettingsModel::new(
            "appearance",
            [nana_ui_core::SettingsTab::new("appearance", "外观")],
        )
        .unwrap();
        let state = SettingsState::new(&model);
        let section = context
            .create_component(
                document,
                AppearanceSection::new(ThemeAppearance::Dark, AppearanceSettings::default()),
            )
            .unwrap();
        let page = context
            .create_component(
                document,
                SettingsPage::new(model, state).content(section.stable_id()),
            )
            .unwrap();
        context.assemble_appearance_section(section).unwrap();
        context.assemble_settings_page(page).unwrap();
        let title = context
            .read(page, |page| page.assembly.clone().unwrap().title)
            .unwrap()
            .expect("settings page title");
        assert_eq!(context.world().text(title), Some("外观"));
        assert!(matches!(
            context
                .world()
                .widget_visual_as::<crate::CardVisual>(section.stable_id()),
            Some(crate::CardVisual {
                title,
                kind: CardKind::Surface,
                ..
            }) if title.is_none()
        ));
        assert_eq!(
            context.world().extract_nodes(&[section.stable_id()])[0]
                .source_style
                .layout
                .resolved_padding()
                .top,
            UI_METRICS.panel_padding_y
        );
    }
}

#[cfg(test)]
mod path_hint_tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport, PathText, Text};

    #[test]
    fn path_hint_updates_full_tooltip_and_can_switch_back_to_prose() {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let row = cx
            .create_component(doc, SettingsRow::new("文件夹").path_hint("/one/two/模型/"))
            .unwrap();
        cx.assemble_settings_row(row).unwrap();
        cx.layout_document(doc, LayoutViewport::new(180.0, 100.0))
            .unwrap();
        let id = cx.read(row, |row| row.hint_slot.unwrap()).unwrap();
        let hint = Entity::<PathText>::from_stable_id(id);
        let tooltip = cx.component_tooltip(hint).unwrap().unwrap();
        assert_eq!(
            &*cx.read(tooltip, |tip| tip.label.clone()).unwrap(),
            "/one/two/模型/"
        );
        cx.update_component(row, |row, _| row.hint = Some("/new/directory/".into()))
            .unwrap();
        cx.assemble_settings_row(row).unwrap();
        assert_eq!(
            &*cx.read(tooltip, |tip| tip.label.clone()).unwrap(),
            "/new/directory/"
        );
        cx.update_component(row, |row, _| {
            row.hint_is_path = false;
            row.hint = Some("文件夹不可用".into());
        })
        .unwrap();
        cx.assemble_settings_row(row).unwrap();
        let id = cx.read(row, |row| row.hint_slot.unwrap()).unwrap();
        assert!(cx.read(Entity::<Text>::from_stable_id(id), |_| ()).is_ok());
        assert!(!cx.world().contains(tooltip.stable_id()));
    }
}
