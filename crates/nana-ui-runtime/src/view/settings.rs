//! A settings row as a view: its label, hint and grouping are bindable, and
//! its control is a slot.
//!
//! ```ignore
//! settings_row("Mipmap")
//!     .hint("远处贴图用更小的层级")
//!     .divided(true)
//!     .control(switch("").checked(mipmaps).on_change(|e| set_mipmaps(e.checked)))
//! ```
//!
//! The row builds its copy (label over hint) and orders its children itself
//! (`TypeBehavior::slot_assembler`), as [`AppContext::mount_settings_leaf_row`]
//! does for a row built by hand; a view never calls
//! [`AppContext::assemble_settings_row`].
//!
//! [`AppContext::mount_settings_leaf_row`]: crate::AppContext::mount_settings_leaf_row
//! [`AppContext::assemble_settings_row`]: crate::AppContext::assemble_settings_row

use std::sync::Arc;

use super::controls::StyledComponent;
use super::node::{El, IntoView, widget};
use super::prop::{FieldWrite, IntoProp};
use crate::{NodeStyle, SettingsCollapsibleCard, SettingsPage, SettingsRow};

/// A row labelled `label`, laid out as the rows the framework assembles:
/// its control goes under its copy once the row is narrower than they fit.
#[track_caller]
pub fn settings_row(label: impl IntoProp<Arc<str>>) -> El<SettingsRow> {
    widget(SettingsRow::new("").stack_below(crate::settings::ASSEMBLED_ROW_STACK_BELOW))
        .label(label)
}

impl StyledComponent for SettingsRow {
    fn node_style(&self) -> &NodeStyle {
        &self.style
    }

    fn node_style_mut(&mut self) -> &mut NodeStyle {
        &mut self.style
    }
}

/// Bindable fields of [`SettingsRow`].
#[allow(non_camel_case_types)]
#[doc(hidden)]
pub mod settings_row {
    pub struct label;
    pub struct hint;
    pub struct hint_is_path;
    pub struct error;
    pub struct divided;
    pub struct stacked;
    pub struct first_in_group;
    pub struct last_in_group;
    pub struct label_column;
}

macro_rules! row_fields {
    ($($field:ident: $ty:ty),* $(,)?) => {
        $(
            impl FieldWrite<SettingsRow, $ty> for settings_row::$field {
                const FIELD: &'static str = concat!("SettingsRow.", stringify!($field));

                fn write(target: &mut SettingsRow, value: $ty) {
                    target.$field = value;
                }

                fn differs(target: &SettingsRow, value: &$ty) -> bool {
                    target.$field != *value
                }
            }
        )*

        impl<K> El<SettingsRow, K> {
            $(
                #[track_caller]
                pub fn $field(self, value: impl IntoProp<$ty>) -> Self {
                    self.prop::<$ty, settings_row::$field>(value)
                }
            )*
        }
    };
}

row_fields! {
    label: Arc<str>,
    hint: Option<Arc<str>>,
    hint_is_path: bool,
    error: Option<Arc<str>>,
    divided: bool,
    stacked: bool,
    first_in_group: bool,
    last_in_group: bool,
    label_column: Option<f32>,
}

impl<K> El<SettingsCollapsibleCard, K> {
    /// What the card shows in its header, collapsed or not.
    pub fn summary(self, view: impl IntoView) -> Self {
        self.slot(view, SettingsCollapsibleCard::summary)
    }

    /// What it shows while expanded.
    pub fn details(self, view: impl IntoView) -> Self {
        self.slot(view, SettingsCollapsibleCard::details)
    }

    /// A control at the end of the header.
    pub fn accessory(self, view: impl IntoView) -> Self {
        self.slot(view, SettingsCollapsibleCard::accessory)
    }
}

impl<K> El<SettingsPage, K> {
    /// The page's content, under its header in its scroll area.
    pub fn content(self, view: impl IntoView) -> Self {
        self.slot(view, SettingsPage::content)
    }
}

impl<K> El<SettingsRow, K> {
    /// The row's control: built before the row, placed after its copy.
    pub fn control(self, view: impl IntoView) -> Self {
        self.slot(view, SettingsRow::control_child)
    }

    /// Something after the label, on its line, such as a status pill: it
    /// keeps its width and the label elides first. The row's name is still
    /// its label; the accessory is read as itself.
    pub fn label_accessory(self, view: impl IntoView) -> Self {
        self.slot(view, SettingsRow::label_accessory)
    }

    /// Something after the control, on its line, such as the row's own
    /// actions: it keeps its width and the control gives way.
    pub fn trailing(self, view: impl IntoView) -> Self {
        self.slot(view, SettingsRow::trailing_child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{signal, switch};
    use crate::{AppContext, DocumentId, Entity, StableNodeId, Stack, Switch};

    fn children(cx: &AppContext, id: StableNodeId) -> Vec<StableNodeId> {
        cx.world()
            .node(id)
            .map(|node| node.children.clone())
            .unwrap_or_default()
    }

    /// What the world shows at `id`: a row projects its copy straight into
    /// the world.
    fn text(cx: &AppContext, id: StableNodeId) -> String {
        cx.world().text(id).unwrap_or_default().to_owned()
    }

    /// A row from a view has the shape of one built by hand: its copy (label
    /// over hint), then its control, and the row points at all three.
    #[test]
    fn a_view_row_is_assembled_like_a_leaf_row() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        let label = signal(Arc::<str>::from("提高 GPU 优先级"));
        let hint = signal(Some(Arc::<str>::from("优先把渲染交给高性能 GPU")));
        cx.mount_view(root.stable_id(), move || {
            settings_row(label)
                .hint(hint)
                .divided(true)
                .control(switch("").checked(true))
        })
        .unwrap();
        cx.flush_reactive().unwrap();

        let row = children(&cx, root.stable_id())[0];
        let entity = Entity::<SettingsRow>::from_stable_id(row);
        let (control, label_slot, hint_slot, copy) = cx
            .read(entity, |row| {
                (row.control, row.label_slot, row.hint_slot, row.copy_slot)
            })
            .unwrap();
        let (control, label_slot, hint_slot, copy) = (
            control.unwrap(),
            label_slot.unwrap(),
            hint_slot.unwrap(),
            copy.unwrap(),
        );
        assert_eq!(children(&cx, row), [copy, control]);
        assert_eq!(children(&cx, copy), [label_slot, hint_slot]);
        assert!(
            cx.read(Entity::<Switch>::from_stable_id(control), |s| s.checked)
                .unwrap()
        );
        assert_eq!(text(&cx, label_slot), "提高 GPU 优先级");
        assert_eq!(text(&cx, hint_slot), "优先把渲染交给高性能 GPU");

        // The copy follows the row's bindings; the nodes stay.
        label.set(Arc::from("HDR"));
        hint.set(None);
        cx.flush_reactive().unwrap();
        assert_eq!(children(&cx, row), [copy, control]);
        assert_eq!(text(&cx, label_slot), "HDR");
        assert!(
            cx.world().node_style(hint_slot).unwrap().layout.hidden,
            "a row without a hint keeps no empty line"
        );
    }

    /// The row's label names the control it holds for assistive technology,
    /// and follows the row's label; a control with a label of its own keeps
    /// it, and a select is named by its row, not by the option it shows.
    #[test]
    fn a_row_label_names_the_control_it_holds() {
        use crate::view::{IntoView, column, select};
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let label = signal(Arc::<str>::from("深色模式"));
        let view = cx
            .mount_view_root(document, move || {
                column()
                    .children((
                        settings_row(label).control(switch("")),
                        settings_row("通知").control(switch("静音")),
                        settings_row("字号").control(
                            select()
                                .options(vec![crate::SelectOption::new("m", "中")])
                                .value(Some(Arc::from("m"))),
                        ),
                    ))
                    .into_any()
            })
            .unwrap();
        cx.layout_document(document, crate::LayoutViewport::new(800.0, 600.0))
            .unwrap();
        let rows = children(&cx, view.roots()[0]);
        let control = |cx: &AppContext, row: StableNodeId| {
            cx.read(Entity::<SettingsRow>::from_stable_id(row), |row| {
                row.control.unwrap()
            })
            .unwrap()
        };
        let name = |cx: &AppContext, id: StableNodeId| {
            cx.world()
                .project_accessibility(document)
                .into_iter()
                .find(|node| node.id == id)
                .and_then(|node| node.label.map(|label| label.to_string()))
        };
        let (dark, mute, size) = (
            control(&cx, rows[0]),
            control(&cx, rows[1]),
            control(&cx, rows[2]),
        );
        assert_eq!(name(&cx, dark).as_deref(), Some("深色模式"));
        assert_eq!(name(&cx, mute).as_deref(), Some("静音"));
        assert_eq!(name(&cx, size).as_deref(), Some("字号"));

        cx.take_system_work();
        label.set(Arc::from("HDR"));
        cx.flush_reactive().unwrap();
        let work = cx.take_system_work();
        let delta = cx.world().project_accessibility_delta(&work);
        let renamed = delta.updated.iter().find(|node| node.id == dark);
        assert_eq!(
            renamed.and_then(|node| node.label.as_deref()),
            Some("HDR"),
            "the control is projected again when the label it is named by changes"
        );
    }

    #[test]
    fn a_view_row_matches_a_leaf_row_node_for_node() {
        let document = DocumentId::new(1).unwrap();
        let mut built = AppContext::new();
        let root = built
            .create_component(document, Stack::column(0.0))
            .unwrap();
        let control = built
            .create_detached_component(document, Switch::new("", false))
            .unwrap();
        let row = built
            .mount_settings_leaf_row(document, "Mipmap", Some("远处贴图"), control.stable_id())
            .unwrap();
        built.append_child(root, row).unwrap();

        let mut viewed = AppContext::new();
        let view_root = viewed
            .create_component(document, Stack::column(0.0))
            .unwrap();
        viewed
            .mount_view(view_root.stable_id(), || {
                settings_row("Mipmap")
                    .hint("远处贴图")
                    .first_in_group(true)
                    .last_in_group(true)
                    .control(switch(""))
            })
            .unwrap();
        viewed.flush_reactive().unwrap();

        fn shape(cx: &AppContext, id: StableNodeId) -> Vec<(usize, String, String)> {
            let mut out = Vec::new();
            let mut stack = vec![(id, 0)];
            while let Some((id, depth)) = stack.pop() {
                let node = cx.world().node(id).unwrap();
                out.push((
                    depth,
                    format!("{:?}", node.kind),
                    cx.world().text(id).unwrap_or_default().to_owned(),
                ));
                for child in node.children.iter().rev() {
                    stack.push((*child, depth + 1));
                }
            }
            out
        }
        let built_row = children(&built, root.stable_id())[0];
        let viewed_row = children(&viewed, view_root.stable_id())[0];
        assert_eq!(shape(&built, built_row), shape(&viewed, viewed_row));
        let fields = |cx: &AppContext, row: StableNodeId| {
            cx.read(Entity::<SettingsRow>::from_stable_id(row), |row| {
                (
                    row.label.clone(),
                    row.hint.clone(),
                    row.stack_below,
                    row.first_in_group,
                    row.last_in_group,
                    row.divided,
                )
            })
            .unwrap()
        };
        assert_eq!(fields(&built, built_row), fields(&viewed, viewed_row));
    }

    fn row_parts(
        cx: &AppContext,
        row: StableNodeId,
    ) -> (
        StableNodeId,
        StableNodeId,
        StableNodeId,
        Option<StableNodeId>,
    ) {
        cx.read(Entity::<SettingsRow>::from_stable_id(row), |row| {
            (
                row.copy_slot.unwrap(),
                row.label_slot.unwrap(),
                row.hint_slot.unwrap(),
                row.label_line_slot,
            )
        })
        .unwrap()
    }

    /// Lay `document` out with its text shaped, as a frame does.
    fn lay_out(cx: &mut AppContext, document: DocumentId, width: f32) {
        let viewport = crate::LayoutViewport::new(width, 600.0);
        cx.layout_document(document, viewport).unwrap();
        cx.shape_text_for_layout(document, &mut crate::MeasureTextShaper)
            .unwrap();
        cx.layout_document(document, viewport).unwrap();
    }

    fn bounds(cx: &AppContext, id: StableNodeId) -> crate::LayoutBox {
        cx.world().canonical_layout_box(id).unwrap()
    }

    /// The accessory goes on the label's line, right after the label and
    /// centred on it, in a row inline or stacked; the row is still named by
    /// its label, and the accessory is read as itself.
    #[test]
    fn a_label_accessory_sits_after_the_label_on_its_line() {
        use crate::view::{IntoView, column, text as pill};
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let view = cx
            .mount_view_root(document, || {
                column()
                    .children((
                        settings_row("水印去除")
                            .hint("去掉输出画面角落的标记")
                            .label_accessory(pill("尚未购买"))
                            .control(switch("")),
                        settings_row("水印去除")
                            .stacked(true)
                            .label_accessory(crate::view::status_badge("尚未购买"))
                            .control(switch("")),
                    ))
                    .into_any()
            })
            .unwrap();
        lay_out(&mut cx, document, 800.0);
        for row in children(&cx, view.roots()[0]) {
            let (copy, label, hint, line) = row_parts(&cx, row);
            let line = line.expect("a row with an accessory has a label line");
            let accessory = cx
                .read(Entity::<SettingsRow>::from_stable_id(row), |row| {
                    row.label_accessory.unwrap()
                })
                .unwrap();
            assert_eq!(children(&cx, copy), [line, hint]);
            assert_eq!(children(&cx, line), [label, accessory]);
            let (label_box, accessory_box) = (bounds(&cx, label), bounds(&cx, accessory));
            assert!(accessory_box.width > 0.0);
            let gap = accessory_box.x - (label_box.x + label_box.width);
            assert!(
                (gap - nana_ui_core::space::XS).abs() < 0.5,
                "the accessory follows the label at the small gap, not the line's end: {gap}"
            );
            let centre = |b: crate::LayoutBox| b.y + b.height / 2.0;
            assert!((centre(label_box) - centre(accessory_box)).abs() < 0.5);

            let named = cx
                .world()
                .project_accessibility(document)
                .into_iter()
                .find(|node| node.id == row)
                .and_then(|node| node.label.map(|label| label.to_string()));
            assert_eq!(named.as_deref(), Some("水印去除"));
        }
    }

    /// A label too long for its line gives way: it elides, and the accessory
    /// keeps the width it has beside a short one.
    #[test]
    fn a_long_label_elides_before_its_accessory_shrinks() {
        use crate::view::{IntoView, column, text as pill};
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let view = cx
            .mount_view_root(document, || {
                column()
                    .children((
                        settings_row("短").label_accessory(pill("尚未购买")),
                        settings_row("一个长到这一行放不下的设置项名字，后面还跟着更多的字")
                            .label_accessory(pill("尚未购买")),
                    ))
                    .into_any()
            })
            .unwrap();
        lay_out(&mut cx, document, 240.0);
        let rows = children(&cx, view.roots()[0]);
        let accessory = |cx: &AppContext, row: StableNodeId| {
            cx.read(Entity::<SettingsRow>::from_stable_id(row), |row| {
                row.label_accessory.unwrap()
            })
            .unwrap()
        };
        let natural = bounds(&cx, accessory(&cx, rows[0])).width;
        let (_, label, _, line) = row_parts(&cx, rows[1]);
        let line = bounds(&cx, line.unwrap());
        let squeezed = bounds(&cx, accessory(&cx, rows[1]));
        assert!(natural > 0.0);
        assert!((squeezed.width - natural).abs() < 0.5);
        assert!(squeezed.x + squeezed.width <= line.x + line.width + 0.5);
        assert!(bounds(&cx, label).width < line.width - natural);
    }

    /// A row without an accessory keeps its label in the copy itself; an
    /// accessory taken away hands the label back and its line goes, and one
    /// hidden by its own `when` leaves the label where it was.
    #[test]
    fn a_label_accessory_comes_and_goes() {
        use crate::view::{text as pill, when};
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        let shown = signal(true);
        cx.mount_view(root.stable_id(), move || {
            settings_row("水印去除")
                .label_accessory(when(shown, || pill("尚未购买")))
                .control(switch(""))
        })
        .unwrap();
        cx.flush_reactive().unwrap();
        let row = children(&cx, root.stable_id())[0];
        let entity = Entity::<SettingsRow>::from_stable_id(row);
        let (copy, label, hint, line) = row_parts(&cx, row);
        let line = line.unwrap();
        let slot = cx.read(entity, |row| row.label_accessory.unwrap()).unwrap();
        assert_eq!(children(&cx, line), [label, slot]);
        assert_eq!(children(&cx, slot).len(), 1);

        shown.set(false);
        cx.flush_reactive().unwrap();
        assert_eq!(children(&cx, line), [label, slot]);
        assert!(children(&cx, slot).is_empty(), "the pill is gone");

        cx.update_component(entity, |row, _| row.label_accessory = None)
            .unwrap();
        assert!(cx.assemble_settings_row(entity).unwrap());
        assert_eq!(children(&cx, copy), [label, hint]);
        assert!(
            !cx.world().contains(line),
            "the line went with the accessory"
        );
        assert_eq!(cx.read(entity, |row| row.label_line_slot).unwrap(), None);
        let style = cx.world().node_style(label).unwrap();
        assert_eq!(style.layout.width, Some(nana_ui_core::LengthSpec::Fill));
        assert_eq!(style.layout.flex_grow, Some(1.0));
        assert!(
            !cx.assemble_settings_row(entity).unwrap(),
            "and it stays put"
        );
    }

    /// A row with no accessory is built as it was: no line, the label in the
    /// copy and filling it.
    #[test]
    fn a_row_without_an_accessory_has_no_label_line() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        cx.mount_view(root.stable_id(), || settings_row("HDR").control(switch("")))
            .unwrap();
        cx.flush_reactive().unwrap();
        let row = children(&cx, root.stable_id())[0];
        let (copy, label, hint, line) = row_parts(&cx, row);
        assert_eq!(line, None);
        assert_eq!(children(&cx, copy), [label, hint]);
        let style = cx.world().node_style(label).unwrap();
        assert_eq!(style.layout.width, Some(nana_ui_core::LengthSpec::Fill));
    }

    mod flash {
        use super::*;
        use crate::{AnimatableProperty, MotionValue, MutationQueue};
        use std::time::Duration;

        const FRAME: Duration = Duration::from_millis(16);

        fn flash_of(cx: &AppContext, row: StableNodeId) -> Option<StableNodeId> {
            cx.read(Entity::<SettingsRow>::from_stable_id(row), |row| {
                row.flash_slot
            })
            .unwrap()
        }

        fn opacity(cx: &AppContext, id: StableNodeId, at: Duration) -> Option<f32> {
            match cx
                .world()
                .presentation_applied_value(id, AnimatableProperty::Opacity, at)
            {
                Some(MotionValue::Scalar(value)) => Some(value),
                _ => None,
            }
        }

        fn mounted() -> (AppContext, DocumentId, StableNodeId, StableNodeId) {
            let document = DocumentId::new(1).unwrap();
            let mut cx = AppContext::new();
            let root = cx.create_component(document, Stack::column(0.0)).unwrap();
            cx.mount_view(root.stable_id(), || {
                (
                    settings_row("HDR").divided(true).control(switch("")),
                    settings_row("Mipmap").control(switch("")),
                )
            })
            .unwrap();
            cx.flush_reactive().unwrap();
            lay_out(&mut cx, document, 480.0);
            let row = children(&cx, root.stable_id())[0];
            (cx, document, root.stable_id(), row)
        }

        fn length(cx: &AppContext) -> Duration {
            cx.world()
                .theme()
                .duration(nana_ui_core::MotionRole::Attention)
        }

        /// The tint shows at once, holds, fades on the presentation layer
        /// from the next frame, and is gone at the end; the row's own
        /// background and layout never change, and pointers go through it.
        #[test]
        fn a_flash_tints_the_row_then_leaves_nothing() {
            let (mut cx, document, _, row) = mounted();
            let entity = Entity::<SettingsRow>::from_stable_id(row);
            let before = children(&cx, row);
            let boxes =
                |cx: &AppContext| before.iter().map(|id| bounds(cx, *id)).collect::<Vec<_>>();
            let laid_out = boxes(&cx);
            let row_box = bounds(&cx, row);
            let control = cx.read(entity, |row| row.control.unwrap()).unwrap();
            let control_box = bounds(&cx, control);
            let hit = cx.world().hit_test(
                document,
                control_box.x + control_box.width / 2.0,
                control_box.y + control_box.height / 2.0,
            );

            cx.flash_settings_row(entity).unwrap();
            let flash = flash_of(&cx, row).expect("the row paints a tint");
            assert_eq!(children(&cx, row)[0], flash, "under everything else");
            let style = cx.world().node_style(flash).unwrap();
            assert_eq!(
                style.background,
                Some(nana_ui_core::SemanticColorRole::Selected)
            );
            assert_eq!(cx.world().node_style(row).unwrap().background, None);
            assert_eq!(opacity(&cx, flash, Duration::ZERO), None, "shown in full");

            lay_out(&mut cx, document, 480.0);
            assert_eq!(boxes(&cx), laid_out, "out of flow");
            assert_eq!(bounds(&cx, row), row_box);
            let tint = bounds(&cx, flash);
            let inset = nana_ui_core::ControlPadding::ListItem.resolve(cx.world().theme_metrics());
            assert!((tint.x - (row_box.x - inset)).abs() < 0.5);
            assert!((tint.width - (row_box.width + inset * 2.0)).abs() < 0.5);
            assert!((tint.y - row_box.y).abs() < 0.5);
            // The divider is the row's border, under its padding box.
            assert!(tint.height <= row_box.height && tint.height > row_box.height - 2.0);
            assert_eq!(
                cx.world().hit_test(
                    document,
                    control_box.x + control_box.width / 2.0,
                    control_box.y + control_box.height / 2.0,
                ),
                hit
            );
            assert_ne!(
                cx.world().hit_test(document, tint.x + 1.0, tint.y + 1.0),
                Some(flash)
            );

            // The next frame starts it.
            let start = FRAME;
            cx.advance_animations(start);
            let length = length(&cx);
            assert_eq!(opacity(&cx, flash, start + length / 10), Some(1.0), "held");
            let fading = opacity(&cx, flash, start + length / 2).unwrap();
            assert!(fading > 0.0 && fading < 1.0, "{fading}");
            assert!(opacity(&cx, flash, start + length * 9 / 10).unwrap() < fading);

            cx.advance_animations(start + length / 2);
            assert!(cx.world().contains(flash));
            cx.advance_animations(start + length);
            assert!(!cx.world().contains(flash), "nothing is left of it");
            assert_eq!(flash_of(&cx, row), None);
            assert_eq!(children(&cx, row), before);
            assert_eq!(cx.world().node_style(row).unwrap().background, None);
            let id =
                crate::component_animation_id(crate::component_animation_kinds::ATTENTION, flash)
                    .unwrap();
            assert!(!cx.world().animation_is_live(id));

            // A view commit after it reassembles the row as it was.
            assert!(!cx.assemble_settings_row(entity).unwrap());
        }

        /// The hold is the installed theme's share of the flash.
        #[test]
        fn the_hold_is_the_installed_themes() {
            let (mut cx, _, _, row) = mounted();
            let mut held = nana_ui_core::ThemeDefinition::NANA_DARK;
            held.motion.attention_hold = 0.6;
            let mut install = MutationQueue::new();
            install.set_theme_tokens(Arc::new(held.bump().compile().expect("compiles")));
            cx.commit_mutations(install).unwrap();
            cx.flash_settings_row(Entity::from_stable_id(row)).unwrap();
            let flash = flash_of(&cx, row).unwrap();
            let start = FRAME;
            cx.advance_animations(start);
            let length = length(&cx);
            assert_eq!(opacity(&cx, flash, start + length / 2), Some(1.0), "held");
            assert!(opacity(&cx, flash, start + length * 8 / 10).unwrap() < 1.0);
        }

        /// Reduced motion keeps the tint still for the same time, then takes
        /// it away at once.
        #[test]
        fn reduced_motion_holds_the_tint_without_fading() {
            let (mut cx, _, _, row) = mounted();
            cx.set_reduced_motion(true);
            cx.flash_settings_row(Entity::from_stable_id(row)).unwrap();
            let flash = flash_of(&cx, row).unwrap();
            let start = FRAME;
            cx.advance_animations(start);
            let length = length(&cx);
            for share in [1, 5, 9] {
                assert_eq!(opacity(&cx, flash, start + length * share / 10), Some(1.0));
            }
            cx.advance_animations(start + length / 2);
            assert!(cx.world().contains(flash));
            cx.advance_animations(start + length);
            assert!(!cx.world().contains(flash));
        }

        /// Flashing again while it fades shows it in full and runs it over
        /// from the next frame, on the same tint.
        #[test]
        fn flashing_again_starts_over() {
            let (mut cx, _, _, row) = mounted();
            let entity = Entity::<SettingsRow>::from_stable_id(row);
            cx.flash_settings_row(entity).unwrap();
            let flash = flash_of(&cx, row).unwrap();
            let first = FRAME;
            cx.advance_animations(first);
            let length = length(&cx);
            let later = first + length * 7 / 10;
            cx.advance_animations(later);
            assert!(opacity(&cx, flash, later).unwrap() < 1.0);

            cx.flash_settings_row(entity).unwrap();
            assert_eq!(flash_of(&cx, row), Some(flash), "the same tint");
            assert_eq!(opacity(&cx, flash, later), None, "full again");
            let again = later + FRAME;
            cx.advance_animations(again);
            assert_eq!(opacity(&cx, flash, again + length / 10), Some(1.0));
            cx.advance_animations(first + length);
            assert!(cx.world().contains(flash), "the first run's end is past");
            cx.advance_animations(again + length);
            assert!(!cx.world().contains(flash));
        }

        /// A row taken away mid-flash takes its tint and timeline with it.
        #[test]
        fn a_row_removed_mid_flash_takes_the_flash_with_it() {
            let (mut cx, _, _, row) = mounted();
            let entity = Entity::<SettingsRow>::from_stable_id(row);
            cx.flash_settings_row(entity).unwrap();
            let flash = flash_of(&cx, row).unwrap();
            cx.advance_animations(FRAME);
            let id =
                crate::component_animation_id(crate::component_animation_kinds::ATTENTION, flash)
                    .unwrap();
            assert!(cx.world().animation_is_live(id));
            cx.remove_view(entity).unwrap();
            assert!(!cx.world().contains(flash));
            assert!(!cx.world().animation_is_live(id));
            cx.advance_animations(FRAME + length(&cx));
        }
    }

    /// Rows with one label column line their controls up at its edge, are
    /// only as tall as their controls, keep a trailing node after the
    /// control, and still put the control under the label once narrow.
    #[test]
    fn label_column_rows_line_their_controls_up() {
        use crate::view::{IntoView, column, row, text_input};
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let width = signal(360.0_f32);
        let view = cx
            .mount_view_root(document, move || {
                column()
                    .bind(move |c: &mut Stack| {
                        c.layout_mut().width = Some(nana_ui_core::LengthSpec::Px(width.get()))
                    })
                    .children((
                        settings_row("名称")
                            .label_column(Some(96.0))
                            .control(text_input().bind(|c| {
                                c.style.layout_mut().width = Some(nana_ui_core::LengthSpec::Fill)
                            })),
                        settings_row("一个很长很长的标签")
                            .label_column(Some(96.0))
                            .control(switch(""))
                            .trailing(switch("重置")),
                        // a trailing row of buttons that asks for the full width
                        settings_row("路径")
                            .label_column(Some(96.0))
                            .control(text_input().key("path").bind(|c| {
                                let l = c.style.layout_mut();
                                l.flex_basis = Some(nana_ui_core::LengthSpec::Px(0.0));
                                l.flex_grow = Some(1.0);
                                l.min_width = Some(nana_ui_core::LengthSpec::Px(0.0));
                            }))
                            .trailing(
                                row()
                                    .bind(|c: &mut Stack| {
                                        c.layout_mut().width = Some(nana_ui_core::LengthSpec::Fill)
                                    })
                                    .children((switch("浏览"),)),
                            ),
                    ))
                    .into_any()
            })
            .unwrap();
        let viewport = crate::LayoutViewport::new(800.0, 600.0);
        let settle = |cx: &mut AppContext| {
            cx.flush_reactive().unwrap();
            cx.layout_document(document, viewport).unwrap();
            cx.layout_document(document, viewport).unwrap();
        };
        settle(&mut cx);
        let rows = children(&cx, view.roots()[0]);
        let parts = |cx: &AppContext, row: StableNodeId| {
            cx.read(Entity::<SettingsRow>::from_stable_id(row), |row| {
                (row.control.unwrap(), row.copy_slot.unwrap(), row.trailing)
            })
            .unwrap()
        };
        let layout =
            |cx: &AppContext, id: StableNodeId| cx.world().canonical_layout_box(id).unwrap();
        let (name, name_copy, _) = parts(&cx, rows[0]);
        let (toggle, toggle_copy, reset) = parts(&cx, rows[1]);
        let reset = reset.unwrap();
        for (row, copy, control) in [(rows[0], name_copy, name), (rows[1], toggle_copy, toggle)] {
            let (row, copy, control) = (layout(&cx, row), layout(&cx, copy), layout(&cx, control));
            assert_eq!(
                copy.width, 96.0,
                "the label keeps its column, long or short"
            );
            assert_eq!(
                control.x,
                row.x + 96.0 + ROW_GAP,
                "controls start at the column's edge"
            );
            assert_eq!(
                row.height,
                control.height.max(copy.height),
                "no band around the row"
            );
        }
        assert_eq!(
            layout(&cx, name).x + layout(&cx, name).width,
            layout(&cx, rows[0]).x + 360.0,
            "a control that fills reaches the row's end"
        );
        assert!(layout(&cx, reset).x >= layout(&cx, toggle).x + layout(&cx, toggle).width);
        let (path, _, browse) = parts(&cx, rows[2]);
        let (path_box, browse_box) = (layout(&cx, path), layout(&cx, browse.unwrap()));
        assert!(
            path_box.width >= 96.0 - 0.5,
            "the control keeps its minimum beside a full-width trailing node: {path_box:?} {browse_box:?}"
        );
        assert!(browse_box.x + browse_box.width <= layout(&cx, rows[2]).x + 360.0 + 0.5);
        let path_of = cx.assembly_path(path).unwrap_or_default();
        assert!(
            path_of.rsplit('/').next() == Some("path"),
            "a slot's control keeps the key it was declared with: {path_of}"
        );

        // the minimum is the installed theme's: three control heights
        let value = cx.world().parent_id(path).unwrap();
        assert_eq!(layout(&cx, value).width, 96.0, "held at its minimum");
        let compact = nana_ui_core::COMPACT_METRICS;
        cx.set_style_tokens(
            nana_ui_core::ThemeAppearance::Dark,
            compact,
            nana_ui_core::SemanticPalette::dark(),
            nana_ui_core::SemanticPalette::dark().surface,
        )
        .unwrap();
        settle(&mut cx);
        assert_eq!(layout(&cx, value).width, compact.control_height * 3.0);
        cx.set_style_tokens(
            nana_ui_core::ThemeAppearance::Dark,
            nana_ui_core::UI_METRICS,
            nana_ui_core::SemanticPalette::dark(),
            nana_ui_core::SemanticPalette::dark().surface,
        )
        .unwrap();

        // narrower than a label column and a control, the control goes under
        width.set(180.0);
        settle(&mut cx);
        let (copy, control) = (layout(&cx, name_copy), layout(&cx, name));
        assert!(control.y >= copy.y + copy.height);
    }

    const ROW_GAP: f32 = nana_ui_core::space::XS;
}
