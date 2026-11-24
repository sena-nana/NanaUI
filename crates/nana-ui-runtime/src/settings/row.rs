use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, FlexWrap, JustifySpec, LengthSpec, SemanticColorRole,
    UI_METRICS,
};

use super::group::{generates_box, resolve_row_hint_placement, sync_row_group};
use super::{document_of, reconcile_children};
use crate::view_components::{Text, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AppContext, ComponentView, Entity,
    FrameworkError, InteractionState, MutationQueue, NodeKind, NodeStyle, StableNodeId,
    TextContent, UiWorld,
};

pub(super) const ROW_PADDING_Y: f32 = nana_ui_core::space::SM;
const ROW_GROUP_PADDING_Y: f32 = nana_ui_core::space::XXS;
const ROW_MULTILINE_PADDING_Y: f32 = nana_ui_core::space::LG;
pub(super) const ROW_MULTILINE_GROUP_PADDING_Y: f32 = nana_ui_core::space::XS;
const ROW_STACK_GAP: f32 = nana_ui_core::space::MD;
pub(super) const ROW_STACK_GAP_LOOSE: f32 = nana_ui_core::space::XL;
const ROW_INLINE_GAP: f32 = nana_ui_core::space::MD;
const ROW_INLINE_GAP_LOOSE: f32 = nana_ui_core::space::XXL;
pub(super) const ROW_COPY_GAP: f32 = nana_ui_core::space::XS;
const ROW_LABEL_ACCESSORY_GAP: f32 = nana_ui_core::space::XS;
/// Between a row's main line and what is under it (the opened hint, the
/// notice).
const ROW_UNDER_GAP: f32 = nana_ui_core::space::XS;
/// Between a column row's control and its trailing node.
const ROW_TRAILING_GAP: f32 = nana_ui_core::space::XXS;
/// Between a column row's label column and its control, and between its
/// label and control once it stacks.
const ROW_COLUMN_GAP: f32 = nana_ui_core::space::XS;
const ROW_COLUMN_STACK_GAP: f32 = nana_ui_core::space::XXS;
/// Rows the framework assembles (appearance, about) put their control under
/// the copy below this width: beside a segmented picker, a narrower row
/// leaves the label and hint a column a few characters wide.
pub(crate) const ASSEMBLED_ROW_STACK_BELOW: f32 = 280.0;

/// Where a settings row puts its hint ([`SettingsRow::hint`](field@SettingsRow::hint)).
///
/// The place suits how the row is used, so its container chooses it, not
/// each row: a settings page, visited seldom and read when visited, keeps
/// the hint under the name ([`Self::Below`]); an inspector or a form, glanced
/// at while adjusting, keeps each row one line tall and puts the hint behind
/// a help button after the name ([`Self::Help`]). Set it on a
/// [`SettingsGroup`] or a [`SettingsCard`] and every row inside follows; a
/// row's own [`SettingsRow::hint_placement`](field@SettingsRow::hint_placement) wins over its container's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum HintPlacement {
    /// Under the name, in the row's copy.
    #[default]
    Below,
    /// Behind a help button after the name: hovering it previews the hint,
    /// pressing it (with a pointer, a touch or the keyboard) opens the hint
    /// in place under the row, and pressing again closes it.
    Help,
}

/// Non-interactive chrome wrapping an application-owned control child.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsRow {
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
    pub hint_is_path: bool,
    /// Where the hint goes, this row's own choice. `None` follows the
    /// nearest [`SettingsGroup`] or [`SettingsCard`] above it that sets one,
    /// and [`HintPlacement::Below`] when none does.
    pub hint_placement: Option<HintPlacement>,
    /// Why the row's value is wrong (an empty required field, a port that
    /// is taken). While set it shows on the row's notice line, under the
    /// row, as a Danger notice, in place of any [`Self::notice`](field@Self::notice); the hint
    /// stays where it is. The control marks itself invalid; the row only
    /// says why.
    pub error: Option<Arc<str>>,
    /// A line under the row about its value: a caution, or with
    /// [`Self::notice_tone`](field@Self::notice_tone) Danger, a problem. It belongs to the row: it
    /// stands above the row's divider, spans the row, and the control stays
    /// centred on the name. `None` or empty takes no room.
    pub notice: Option<Arc<str>>,
    /// How [`Self::notice`](field@Self::notice) reads: Warning (the default) or Danger.
    pub notice_tone: nana_ui_core::ValidationIntent,
    /// The placement the row's group or card gives it, written by them; see
    /// [`Self::hint_placement`](field@Self::hint_placement).
    pub(crate) inherited_hint_placement: Option<HintPlacement>,
    /// Whether the hint behind the help button is open under the row. The
    /// help button flips it; it is the row's own state, as a hover card's
    /// `open` is the card's.
    pub(crate) help_open: bool,
    pub stacked: bool,
    /// Stack below this row's laid-out width in logical pixels. `stacked`
    /// remains an unconditional override; `None` preserves inline layout.
    pub stack_below: Option<f32>,
    pub divided: bool,
    pub loose: bool,
    pub first_in_group: bool,
    pub last_in_group: bool,
    /// Written by the row's [`SettingsGroup`]: the shown member above this
    /// row is not a settings row (a value list, a list, a notice). A divided
    /// row then draws the line above itself too, so a settings row's edge is
    /// ruled on both sides whatever stands next to it; its own bottom line
    /// only ever covers the edge below.
    pub(crate) rule_above: bool,
    pub control: Option<StableNodeId>,
    /// The application's mark right after the label, before a help button or
    /// [`Self::label_accessory`]: a category seal. It keeps its width; the
    /// label gives way and elides.
    pub label_mark: Option<StableNodeId>,
    /// The application's node after the label (and any help button), on the
    /// label's line: a status pill, a badge. It keeps its width; the label
    /// gives way and elides.
    pub label_accessory: Option<StableNodeId>,
    /// Lay the row out as an inspector's: the label in a column this many
    /// logical pixels wide, the control growing from the column's edge and
    /// the row only as tall as its control. Rows given the same width line
    /// their controls up, card after card. `None` keeps the settings layout:
    /// the copy on the left, the control pushed to the right edge.
    pub label_column: Option<f32>,
    /// Colours the label takes in place of the theme's, for a row the
    /// application sets apart by its name's colour (see
    /// [`nana_ui_core::ColorOverrides`]). Only the label: the hint, the
    /// notice and the control keep the theme's. `None` keeps the theme's.
    pub label_colors: Option<Arc<nana_ui_core::ColorOverrides>>,
    /// The application's node after the control, on its line: the row's own
    /// actions. It keeps its width; the control gives way.
    pub trailing: Option<StableNodeId>,
    pub label_slot: Option<StableNodeId>,
    pub hint_slot: Option<StableNodeId>,
    pub style: NodeStyle,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SettingsRowAssembled {
    pub(crate) help_slot: Option<StableNodeId>,
    pub(crate) help_text_slot: Option<StableNodeId>,
    pub(crate) notice_slot: Option<StableNodeId>,
    pub(crate) notice_glyph_slot: Option<StableNodeId>,
    pub(crate) notice_text_slot: Option<StableNodeId>,
    pub(crate) main_slot: Option<StableNodeId>,
    pub(crate) error_shown: Option<bool>,
    pub(crate) copy_slot: Option<StableNodeId>,
    pub(crate) label_line_slot: Option<StableNodeId>,
    pub(crate) flash_slot: Option<StableNodeId>,
    pub(crate) control_slot: Option<StableNodeId>,
    pub(crate) value_slot: Option<StableNodeId>,
    pub(crate) trailing_slot: Option<StableNodeId>,
    pub(crate) last_generates_box: Option<bool>,
}

impl SettingsRow {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            hint: None,
            hint_is_path: false,
            hint_placement: None,
            error: None,
            notice: None,
            notice_tone: nana_ui_core::ValidationIntent::Warning,
            inherited_hint_placement: None,
            help_open: false,
            stacked: false,
            stack_below: None,
            divided: false,
            loose: false,
            first_in_group: false,
            last_in_group: false,
            rule_above: false,
            control: None,
            label_mark: None,
            label_accessory: None,
            label_column: None,
            label_colors: None,
            trailing: None,
            label_slot: None,
            hint_slot: None,
            style: NodeStyle::default(),
        }
    }

    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn path_hint(mut self, path: impl Into<Arc<str>>) -> Self {
        self.hint = Some(path.into());
        self.hint_is_path = true;
        self
    }

    /// Say why the value is wrong, on the notice line; see [`Self::error`].
    pub fn error(mut self, error: impl Into<Arc<str>>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Put the hint where `placement` says, whatever the row's container
    /// says; see [`Self::hint_placement`](field@Self::hint_placement).
    pub fn hint_placement(mut self, placement: HintPlacement) -> Self {
        self.hint_placement = Some(placement);
        self
    }

    /// Show `notice` on a line under the row; see [`Self::notice`](field@Self::notice).
    pub fn notice(mut self, notice: impl Into<Arc<str>>) -> Self {
        self.notice = Some(notice.into());
        self
    }

    /// How the notice reads; see [`Self::notice_tone`](field@Self::notice_tone).
    pub fn notice_tone(mut self, tone: nana_ui_core::ValidationIntent) -> Self {
        self.notice_tone = tone;
        self
    }

    /// Where the hint goes now: the row's own choice, else its container's,
    /// else under the name.
    pub fn effective_hint_placement(&self) -> HintPlacement {
        self.hint_placement
            .or(self.inherited_hint_placement)
            .unwrap_or_default()
    }

    /// Whether the hint is behind a help button: it goes there and there is
    /// one.
    fn shows_help(&self) -> bool {
        self.effective_hint_placement() == HintPlacement::Help
            && self.hint.as_deref().is_some_and(|hint| !hint.is_empty())
    }

    /// Whether a non-empty hint sits directly under the name in the row's copy.
    fn shows_below_hint(&self) -> bool {
        self.effective_hint_placement() == HintPlacement::Below
            && self.hint.as_deref().is_some_and(|hint| !hint.is_empty())
    }

    /// Single-line rows have a `ControlSize::Small` box that buffers the
    /// label inside the row; rows with a below-hint or a stacked control under
    /// a label place bare copy directly against the row's padding, so they
    /// step up to `Lg`/`Xs` bands.
    fn uses_multiline_bands(&self, stacked: bool) -> bool {
        self.shows_below_hint() || (stacked && !self.label.is_empty())
    }

    /// What the notice line shows, and how: the error first, else the
    /// notice; `None` when it has nothing to show.
    fn shown_notice(&self) -> Option<(&Arc<str>, nana_ui_core::ValidationIntent)> {
        if let Some(error) = self.error.as_ref() {
            return Some((error, nana_ui_core::ValidationIntent::Danger));
        }
        self.notice
            .as_ref()
            .filter(|notice| !notice.is_empty())
            .map(|notice| (notice, self.notice_tone))
    }

    pub fn stacked(mut self, stacked: bool) -> Self {
        self.stacked = stacked;
        self
    }

    /// Opt into responsive stacking using the row width, not the window width.
    /// Non-finite or non-positive thresholds disable responsive stacking.
    pub fn stack_below(mut self, width: f32) -> Self {
        self.stack_below = (width.is_finite() && width > 0.0).then_some(width);
        self
    }

    /// Draw a hairline under this row, separating it from the next one. The
    /// last row of a group draws none: the group's own edge already ends it.
    pub fn divided(mut self, divided: bool) -> Self {
        self.divided = divided;
        self
    }

    pub fn loose(mut self, loose: bool) -> Self {
        self.loose = loose;
        self
    }

    pub fn first_in_group(mut self) -> Self {
        self.first_in_group = true;
        self
    }

    pub fn last_in_group(mut self) -> Self {
        self.last_in_group = true;
        self
    }

    pub fn control_child(mut self, control: StableNodeId) -> Self {
        self.control = Some(control);
        self
    }

    /// Put `mark` immediately after the label, before a help button or
    /// [`Self::label_accessory`]. Assembled by
    /// [`AppContext::assemble_settings_row`], like the control.
    pub fn label_mark(mut self, mark: StableNodeId) -> Self {
        self.label_mark = Some(mark);
        self
    }

    /// Put `accessory` after the label (and any help button), on its line.
    /// Assembled by [`AppContext::assemble_settings_row`], like the control.
    pub fn label_accessory(mut self, accessory: StableNodeId) -> Self {
        self.label_accessory = Some(accessory);
        self
    }

    /// Put the label in a column `width` logical pixels wide and grow the
    /// control from its edge; see [`Self::label_column`]. Non-finite or
    /// non-positive widths keep the settings layout.
    pub fn label_column(mut self, width: f32) -> Self {
        self.label_column = (width.is_finite() && width > 0.0).then_some(width);
        self
    }

    /// Put `trailing` after the control, on its line. Assembled by
    /// [`AppContext::assemble_settings_row`], like the control.
    pub fn trailing_child(mut self, trailing: StableNodeId) -> Self {
        self.trailing = Some(trailing);
        self
    }

    pub fn label_slot(mut self, label: StableNodeId) -> Self {
        self.label_slot = Some(label);
        self
    }

    pub fn hint_slot(mut self, hint: StableNodeId) -> Self {
        self.hint_slot = Some(hint);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn gap(&self, stacked: bool) -> f32 {
        if self.label_column.is_some() {
            return if stacked {
                ROW_COLUMN_STACK_GAP
            } else {
                ROW_COLUMN_GAP
            };
        }
        match (stacked, self.loose) {
            (true, true) => ROW_STACK_GAP_LOOSE,
            (true, false) => ROW_STACK_GAP,
            (false, true) => ROW_INLINE_GAP_LOOSE,
            (false, false) => ROW_INLINE_GAP,
        }
    }

    fn effective_stacked(&self, id: StableNodeId, world: &UiWorld) -> bool {
        self.stacked
            || self.stack_below.is_some_and(|threshold| {
                threshold.is_finite()
                    && threshold > 0.0
                    && world
                        .component_layout_box(id)
                        .is_some_and(|bounds| bounds.width < threshold)
            })
    }

    fn effective_style(&self, stacked: bool, main_slot: Option<StableNodeId>) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        style.background = None;
        // A divider, not a box. This used to set the uniform `border_width`,
        // which strokes all four sides: stacked rows overlapped their edges
        // into something that passed for dividers, but a card holding one row
        // got a square frame inside its rounded corners.
        let divider = self.divided && !self.last_in_group;
        let rule_above = self.divided && self.rule_above && !self.first_in_group;
        style.border = (divider || rule_above).then_some(SemanticColorRole::BorderSoft);
        let layout = Arc::make_mut(&mut style.layout);
        // The containing block of the flash tint.
        if !layout.position.establishes_containing_block() {
            layout.position = nana_ui_core::PositionSpec::Relative;
        }
        layout.width = Some(LengthSpec::Percent(100.0));
        if main_slot.is_some() {
            // The main line, then what is under it (the opened hint, the
            // notice), each across the row.
            layout.direction = Some(FlexDirection::Column);
            layout.align_items = AlignSpec::Stretch;
            layout.justify_content = JustifySpec::Start;
            layout.flex_wrap = FlexWrap::NoWrap;
            layout.gap = Some(LengthSpec::Px(ROW_UNDER_GAP));
            layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xs));
        } else {
            self.line_layout(layout, stacked);
        }
        if self.label_column.is_some() {
            // An inspector row: no band above or below the control.
            layout.padding_top = Some(LengthSpec::Px(0.0));
            layout.padding_bottom = Some(LengthSpec::Px(0.0));
            layout.theme.padding_top = None;
            layout.theme.padding_bottom = None;
        } else {
            let (edge_step, edge_px, middle_step, middle_px) = if self.uses_multiline_bands(stacked)
            {
                (
                    nana_ui_core::SpacingStep::Xs,
                    ROW_MULTILINE_GROUP_PADDING_Y,
                    nana_ui_core::SpacingStep::Lg,
                    ROW_MULTILINE_PADDING_Y,
                )
            } else {
                (
                    nana_ui_core::SpacingStep::Xxs,
                    ROW_GROUP_PADDING_Y,
                    nana_ui_core::SpacingStep::Sm,
                    ROW_PADDING_Y,
                )
            };
            let padded_y = layout.padding.is_some() || layout.theme.padding.is_some();
            if !padded_y && layout.padding_top.is_none() && layout.theme.padding_top.is_none() {
                layout.theme.padding_top =
                    Some(crate::theme_intent::space(if self.first_in_group {
                        edge_step
                    } else {
                        middle_step
                    }));
                layout.padding_top = Some(LengthSpec::Px(if self.first_in_group {
                    edge_px
                } else {
                    middle_px
                }));
            }
            if !padded_y && layout.padding_bottom.is_none() && layout.theme.padding_bottom.is_none()
            {
                layout.theme.padding_bottom =
                    Some(crate::theme_intent::space(if self.last_in_group {
                        edge_step
                    } else {
                        middle_step
                    }));
                layout.padding_bottom = Some(LengthSpec::Px(if self.last_in_group {
                    edge_px
                } else {
                    middle_px
                }));
            }
        }
        // Only the bottom edge, and the top one under other content, are
        // named; the others fall back to the unset uniform width, which is
        // none.
        layout.border_width = None;
        if divider
            || rule_above
            || layout.border_sides.bottom_width.is_some()
            || layout.border_sides.top_width.is_some()
        {
            let sides = &mut *layout.border_sides;
            sides.bottom_width = divider.then_some(nana_ui_core::HAIRLINE);
            sides.top_width = rule_above.then_some(nana_ui_core::HAIRLINE);
        }
        layout.theme.border_bottom_width = divider
            .then(|| nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
        layout.theme.border_top_width = rule_above
            .then(|| nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
        crate::theme_intent::font(layout, nana_ui_core::TypeRole::Body);
        crate::theme_intent::line_from_type(layout, nana_ui_core::TypeRole::Body);
        crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Medium);
        style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
        style
    }

    /// How the copy and the control share their line: the row's own layout,
    /// or its main line's once the row has something under it.
    fn line_layout(&self, layout: &mut nana_ui_core::LayoutStyle, stacked: bool) {
        layout.direction = Some(if stacked {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        });
        // Centred on the whole copy, hint included: the control balances
        // the block it sits beside, not just the name's line.
        layout.align_items = if stacked {
            AlignSpec::Stretch
        } else {
            AlignSpec::Center
        };
        layout.justify_content = if stacked {
            JustifySpec::Start
        } else {
            JustifySpec::SpaceBetween
        };
        layout.flex_wrap = FlexWrap::NoWrap;
        layout.gap = Some(LengthSpec::Px(self.gap(stacked)));
        layout.theme.gap = Some(crate::theme_intent::space(match (stacked, self.loose) {
            (true, true) => nana_ui_core::SpacingStep::Xl,
            (true, false) => nana_ui_core::SpacingStep::Md,
            (false, true) => nana_ui_core::SpacingStep::Xxl,
            (false, false) => nana_ui_core::SpacingStep::Md,
        }));
        if self.label_column.is_some() {
            // An inspector row: the label column, then the control from its
            // edge.
            layout.justify_content = JustifySpec::Start;
            layout.theme.gap = Some(crate::theme_intent::space(if stacked {
                nana_ui_core::SpacingStep::Xxs
            } else {
                nana_ui_core::SpacingStep::Xs
            }));
        }
    }
}

impl ComponentView for SettingsRow {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::SETTINGS_ROW.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_settings_row),
        slot_projector: Some(Self::project_with_slots),
        lifecycle: Some(sync_settings_row),
        post_layout: Some(|cx, id| {
            cx.refresh_settings_row_layout(id)?;
            Ok(false)
        }),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Lays out the control slots the application provides.
    const ALWAYS_REPROJECT: bool = true;

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.project_with_assembled(id, world, mutations, None);
    }
}

impl SettingsRow {
    fn project_with_slots(
        cx: &AppContext,
        id: StableNodeId,
        view: &Self,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        view.project_with_assembled(
            id,
            world,
            mutations,
            cx.assembled_slot::<SettingsRowAssembled>(id),
        );
    }

    fn project_with_assembled(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        mutations: &mut MutationQueue,
        assembled: Option<&SettingsRowAssembled>,
    ) {
        let copy_slot = assembled.and_then(|s| s.copy_slot);
        let main_slot = assembled.and_then(|s| s.main_slot);
        // Structured children/slots paint label and hint. Rust-only rows without
        // a tree keep the label as node text so Scene can still show it.
        let has_slots = self.label_slot.is_some()
            || self.hint_slot.is_some()
            || self.control.is_some()
            || copy_slot.is_some()
            || self.label_mark.is_some()
            || self.label_accessory.is_some();
        let has_children = world.node(id).is_some_and(|node| !node.children.is_empty());
        let visible_label = if has_slots || has_children {
            ""
        } else {
            self.label.as_ref()
        };
        if world.text(id) != Some(visible_label) {
            mutations.set_text(
                id,
                TextContent {
                    value: visible_label.to_owned().into(),
                },
            );
        }
        if world.widget_visual_ref(id).is_some() {
            mutations.set_widget_visual_handle(id, None);
        }
        let stacked = self.effective_stacked(id, world);
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(stacked, main_slot),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            {
                // A Danger notice says what is wrong, as an error does; a
                // Warning one describes the row.
                let notice = self.shown_notice();
                let problem = notice
                    .filter(|(_, tone)| *tone == nana_ui_core::ValidationIntent::Danger)
                    .map(|(text, _)| Arc::clone(text));
                let caution = notice
                    .filter(|(_, tone)| *tone == nana_ui_core::ValidationIntent::Warning)
                    .map(|(text, _)| Arc::clone(text));
                AccessibilityState {
                    role: AccessibilityRole::Generic,
                    label: Some(Arc::clone(&self.label)),
                    invalid: problem.is_some(),
                    value: problem.or_else(|| self.hint.clone()),
                    description: caution,
                    ..AccessibilityState::default()
                }
            },
        );
        self.project_slots(world, mutations, stacked, assembled);
    }

    fn project_slots(
        &self,
        world: &UiWorld,
        mutations: &mut MutationQueue,
        stacked: bool,
        assembled: Option<&SettingsRowAssembled>,
    ) {
        let assembled = assembled.copied().unwrap_or_default();
        if let Some(copy) = assembled.copy_slot {
            SettingsRowCopy {
                stacked,
                column: self.label_column,
            }
            .project(copy, world, mutations);
        }
        if let Some(line) = assembled.label_line_slot {
            SettingsRowLabelLine.project(line, world, mutations);
        }
        if let Some(label) = self.label_slot {
            let mut text = Text::new(self.label.as_ref());
            text.style = label_slot_style(
                self.label.is_empty(),
                assembled.label_line_slot.is_some(),
                self.label_column.is_some(),
            );
            text.style.color_overrides = self.label_colors.clone();
            text.project(label, world, mutations);
        }
        if let Some(control) = assembled.control_slot {
            SettingsRowControl {
                stacked,
                grows: self.label_column.is_some(),
            }
            .project(control, world, mutations);
        }
        if let Some(flash) = assembled.flash_slot {
            SettingsRowFlash.project(flash, world, mutations);
        }
        if let Some(main) = assembled.main_slot {
            SettingsRowMain {
                line: {
                    let mut style = NodeStyle::default();
                    self.line_layout(Arc::make_mut(&mut style.layout), stacked);
                    style
                },
            }
            .project(main, world, mutations);
        }
        let help = self.shows_help();
        if let Some(text) = assembled.help_text_slot {
            Text::new(self.hint.as_deref().unwrap_or(""))
                .style(hint_slot_style(!(help && self.help_open)))
                .project(text, world, mutations);
        }
        let notice = self.shown_notice();
        if let Some(slot) = assembled.notice_slot {
            notice_box_style(notice.map(|(_, tone)| tone)).project(slot, world, mutations);
        }
        let tone = notice.map_or(nana_ui_core::ValidationIntent::Warning, |(_, tone)| tone);
        if let Some(glyph) = assembled.notice_glyph_slot {
            notice_glyph(tone).project(glyph, world, mutations);
        }
        if let Some(text) = assembled.notice_text_slot {
            notice_text(notice.map_or("", |(text, _)| text.as_ref()), tone)
                .project(text, world, mutations);
        }
        if let Some(hint) = self.hint_slot {
            let value = self.hint.as_deref().unwrap_or("");
            // Behind a help button the hint is not under the name.
            let style = hint_slot_style(self.hint.is_none() || help);
            if self.hint_is_path {
                crate::PathText::new(value)
                    .style(style)
                    .project(hint, world, mutations);
            } else {
                Text::new(value)
                    .style(style)
                    .project(hint, world, mutations);
            }
        }
    }
}

/// A row without a label (a control that names itself, such as a segmented
/// picker) keeps no empty line above or beside its control.
///
/// On a line with an accessory the label is as wide as it reads, so the
/// accessory sits right after it, and it is the one that gives way when the
/// line is short: it shrinks and elides, the accessory keeps its width.
///
/// In a label column the label is the quieter half of the row, as in an
/// inspector: muted and regular, so the values carry the row.
pub(super) fn label_slot_style(hidden: bool, on_line: bool, column: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.foreground = Some(if column {
        SemanticColorRole::Muted
    } else {
        SemanticColorRole::Text
    });
    style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
    let layout = Arc::make_mut(&mut style.layout);
    crate::theme_intent::font(layout, nana_ui_core::TypeRole::Body);
    crate::theme_intent::line_from_type(layout, nana_ui_core::TypeRole::Body);
    crate::theme_intent::weight(
        layout,
        if column {
            nana_ui_core::TextWeight::Regular
        } else {
            nana_ui_core::TextWeight::Medium
        },
    );
    if !on_line {
        layout.width = Some(LengthSpec::Fill);
        layout.flex_grow = Some(1.0);
    }
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
    layout.text_overflow_ellipsis = true;
    layout.hidden = hidden;
    style
}

/// The colour a notice of `tone` takes, for its glyph, an error's text
/// and, faintly, its box.
fn notice_role(tone: nana_ui_core::ValidationIntent) -> SemanticColorRole {
    match tone {
        nana_ui_core::ValidationIntent::Warning => SemanticColorRole::Warning,
        nana_ui_core::ValidationIntent::Danger => SemanticColorRole::Danger,
    }
}

/// The notice line's box: a faint tint of the tone's colour with the
/// control radius, across the row, the glyph on the text's first line.
/// `None` hides it: a row without a notice keeps no room or gap for one.
pub(super) fn notice_box_style(tone: Option<nana_ui_core::ValidationIntent>) -> crate::Stack {
    let role = notice_role(tone.unwrap_or(nana_ui_core::ValidationIntent::Warning));
    crate::Stack::row(nana_ui_core::space::XS)
        .gap_step(nana_ui_core::SpacingStep::Xs)
        .width(LengthSpec::Fill)
        .align(AlignSpec::Start)
        .padding_step(nana_ui_core::SpacingStep::Sm, nana_ui_core::SpacingStep::Xs)
        .radius(nana_ui_core::RadiusTier::Sm)
        .surface_mix(nana_ui_core::SemanticColorMix::alpha(
            role,
            nana_ui_core::OpacityTokens::DARK.notice_tint,
        ))
        .with_layout(|layout| layout.hidden = tone.is_none())
}

/// The notice's glyph: a caution triangle or an error circle, as tall as the
/// body text and centred on the text's first line.
pub(super) fn notice_glyph(tone: nana_ui_core::ValidationIntent) -> crate::IconGlyph {
    let icon = match tone {
        nana_ui_core::ValidationIntent::Warning => nana_ui_core::Icon::AlertTriangle,
        nana_ui_core::ValidationIntent::Danger => nana_ui_core::Icon::CircleX,
    };
    let mut glyph = crate::IconGlyph::new(icon)
        .size_role(nana_ui_core::TypeRole::Body)
        .role(notice_role(tone))
        .decorative();
    // Half of what the text's line box is taller than the glyph, above it.
    let layout = Arc::make_mut(&mut glyph.style.layout);
    layout.theme.margin_top = Some(
        nana_ui_core::ThemeExpr::from(nana_ui_core::ThemeToken::Line(nana_ui_core::LineRole::Line))
            .plus(
                nana_ui_core::ThemeExpr::from(nana_ui_core::ThemeToken::Type(
                    nana_ui_core::TypeRole::Body,
                ))
                .scaled(-1.0),
            )
            .scaled(0.5),
    );
    glyph
}

/// The notice's words at the hint's size, wrapping. An error's are in the
/// danger colour; a caution's in the text colour, the glyph and the tint
/// carrying its tone: a caution colour is a bright one (yellow), which
/// small words on its own tint do not read in.
pub(super) fn notice_text(value: &str, tone: nana_ui_core::ValidationIntent) -> Text {
    let mut style = hint_slot_style(false);
    style.foreground = Some(match tone {
        nana_ui_core::ValidationIntent::Warning => SemanticColorRole::Text,
        nana_ui_core::ValidationIntent::Danger => notice_role(tone),
    });
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = None;
        layout.flex_grow = Some(1.0);
    }
    Text::new(value).style(style)
}

pub(super) fn hint_slot_style(hidden: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.foreground = Some(SemanticColorRole::Muted);
    let layout = Arc::make_mut(&mut style.layout);
    crate::theme_intent::font(layout, nana_ui_core::TypeRole::Meta);
    crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Regular);
    // Its own line box: the row's is the label's, too tight for the lines of
    // a hint that wraps.
    crate::theme_intent::line(layout, nana_ui_core::LineRole::Line);
    layout.width = Some(LengthSpec::Fill);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    // The hint says what the setting does; cut to one line it stops saying
    // it. It wraps under the label, while the label stays a single line.
    layout.hidden = hidden;
    style
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SettingsRowCopy {
    stacked: bool,
    /// The row's label column, see [`SettingsRow::label_column`].
    column: Option<f32>,
}

impl SettingsRowCopy {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(LengthSpec::Px(ROW_COPY_GAP));
        layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xs));
        layout.flex_grow = Some(if self.stacked { 0.0 } else { 1.0 });
        layout.flex_shrink = Some(1.0);
        layout.min_width = Some(LengthSpec::Px(0.0));
        if !self.stacked && self.column.is_none() {
            // Single-line rows share the small-control band height so switch,
            // link, value and framed-field rows stand at one uniform height.
            layout.justify_content = JustifySpec::Center;
            layout.min_height = Some(LengthSpec::Px(UI_METRICS.compact_control_height));
            layout.theme.min_height =
                Some(nana_ui_core::ThemeToken::Control(ControlSize::Small).into());
        }
        if let (Some(width), false) = (self.column, self.stacked) {
            // the column keeps its width; the control takes the rest
            layout.width = Some(LengthSpec::Px(width));
            layout.flex_grow = Some(0.0);
            layout.flex_shrink = Some(0.0);
        }
        style
    }
}

/// The control and the row's trailing node, on one line. In a label column
/// it grows from the column's edge to the row's end; without one it keeps
/// the control's width at the row's end.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SettingsRowControl {
    stacked: bool,
    grows: bool,
}

impl SettingsRowControl {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.flex_wrap = FlexWrap::NoWrap;
        layout.gap = Some(LengthSpec::Px(ROW_TRAILING_GAP));
        layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
        layout.min_width = Some(LengthSpec::Px(0.0));
        if self.stacked {
            layout.width = Some(LengthSpec::Fill);
        } else if self.grows {
            layout.flex_grow = Some(1.0);
            layout.flex_shrink = Some(1.0);
            layout.flex_basis = Some(LengthSpec::Px(0.0));
        } else {
            layout.flex_shrink = Some(0.0);
        }
        style
    }
}

/// What the control keeps beside a trailing node, however wide that node
/// asks to be: three of the installed theme's control heights, 96 px in the
/// default theme.
fn row_control_min() -> nana_ui_core::ThemeExpr {
    nana_ui_core::ThemeExpr::token(nana_ui_core::ThemeToken::Control(ControlSize::Medium))
        .scaled(3.0)
}

/// The box a control sits in beside a trailing node: it grows into the room
/// the trailing node leaves and never goes below [`row_control_min`].
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SettingsRowValue;

impl ComponentView for SettingsRowValue {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row-value".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.direction = Some(FlexDirection::Row);
            layout.align_items = AlignSpec::Center;
            layout.flex_grow = Some(1.0);
            layout.flex_shrink = Some(1.0);
            layout.flex_basis = Some(LengthSpec::Px(0.0));
            layout.theme.min_width = Some(row_control_min());
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// The box a row's trailing node sits in: as wide as the node asks while
/// that fits, and the one that gives way when it does not, down to nothing.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SettingsRowTrailing;

impl ComponentView for SettingsRowTrailing {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row-trailing".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.direction = Some(FlexDirection::Row);
            layout.align_items = AlignSpec::Center;
            layout.flex_grow = Some(0.0);
            layout.flex_shrink = Some(1.0);
            layout.min_width = Some(LengthSpec::Px(0.0));
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

impl ComponentView for SettingsRowControl {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row-control".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

impl ComponentView for SettingsRowCopy {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row-copy".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// A row's main line once the row has something under it: the copy and the
/// control, laid out as the row lays them out otherwise ([`SettingsRow`]'s
/// `line_layout`), so the control stays centred on the copy and not on what
/// is under it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SettingsRowMain {
    /// The line's layout, as the row's projection works it out.
    line: NodeStyle,
}

impl ComponentView for SettingsRowMain {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row-main".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = self.line.clone();
        Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Percent(100.0));
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// The label's line when the row has a help button or a label accessory:
/// the label, the help button, then the accessory, centred on each other.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SettingsRowLabelLine;

impl SettingsRowLabelLine {
    fn style() -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Start;
        layout.flex_wrap = FlexWrap::NoWrap;
        layout.gap = Some(LengthSpec::Px(ROW_LABEL_ACCESSORY_GAP));
        layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xs));
        layout.width = Some(LengthSpec::Fill);
        layout.flex_shrink = Some(1.0);
        layout.min_width = Some(LengthSpec::Px(0.0));
        style
    }
}

impl ComponentView for SettingsRowLabelLine {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row-label-line".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &Self::style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// The tint a flashing row paints under itself. It is out of flow, so the
/// row lays out as it would without it; the row is its containing block, so
/// it covers the row's padding box top to bottom and reaches past the copy
/// and the control by a list item's inset on either side, as a selected list
/// row's pill does past its text. Pointers go through it and assistive technology
/// does not see it. Its opacity is the flash: see
/// [`AppContext::flash_settings_row`].
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SettingsRowFlash;

impl SettingsRowFlash {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        style.background = Some(SemanticColorRole::Selected);
        style.radius = Some(nana_ui_core::RadiusTier::Sm);
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = nana_ui_core::PositionSpec::Absolute;
        layout.pointer_events = Some(nana_ui_core::PointerEventsSpec::None);
        let bleed = nana_ui_core::ControlPadding::ListItem;
        let bleed_px = bleed.resolve(nana_ui_core::UI_METRICS);
        let bleed = nana_ui_core::ThemeExpr::from(nana_ui_core::ThemeToken::ControlPadding(bleed))
            .scaled(-1.0);
        layout.offset_left = Some(LengthSpec::Px(-bleed_px));
        layout.offset_right = Some(LengthSpec::Px(-bleed_px));
        layout.theme.offset_left = Some(bleed.clone());
        layout.theme.offset_right = Some(bleed);
        layout.offset_top = Some(LengthSpec::Px(0.0));
        layout.offset_bottom = Some(LengthSpec::Px(0.0));
        style
    }
}

impl ComponentView for SettingsRowFlash {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        animation_sample: Some(|cx, id, sample, now| {
            let _ = cx.advance_settings_row_flash(id, sample.property, now);
            false
        }),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-row-flash".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                hidden: true,
                ..AccessibilityState::default()
            },
        );
    }
}

/// A flash's tint, from `start` for `length`: held for the `hold` share of
/// it, then faded on `easing`,
/// or held throughout while motion is `reduced`. It fills forwards: a frame
/// painted between its end and the tint being taken away shows where it
/// ended, not the tint at full.
fn settings_row_flash_animation(
    flash: StableNodeId,
    start: std::time::Duration,
    length: std::time::Duration,
    hold: f32,
    easing: crate::Easing,
    reduced: bool,
) -> Option<crate::AnimationSpec> {
    let id = crate::component_animation_id(crate::component_animation_kinds::ATTENTION, flash)?;
    let shown = crate::MotionValue::Scalar(1.0);
    let to = if reduced {
        crate::MotionTo::Value(shown)
    } else {
        // The stops name no easing of their own: the fade, from the hold's
        // end, runs on the track's.
        crate::MotionTo::Keyframes(vec![
            crate::Keyframe {
                offset: hold,
                value: shown,
                easing: None,
            },
            crate::Keyframe {
                offset: 1.0,
                value: crate::MotionValue::Scalar(0.0),
                easing: None,
            },
        ])
    };
    Some(
        // A theme's durations are never zero.
        crate::AnimationSpec::new(
            id,
            flash,
            start,
            length,
            crate::framework::COMPONENT_FRAME_INTERVAL,
            easing,
        )
        .with_property(crate::AnimatableProperty::Opacity)
        .with_range(shown, to)
        .with_playback(crate::AnimationPlayback::running(
            crate::AnimationIteration::ONCE,
            crate::AnimationDirection::Normal,
            crate::AnimationFillMode::Forwards,
        )),
    )
}

/// A row written: where its hint goes may have changed (its container's
/// placement, its own), so are its parts (the help button, the notice line)
/// the ones that need, and its group's edges.
pub(super) fn sync_settings_row(
    context: &mut AppContext,
    row: Entity<SettingsRow>,
) -> Result<(), FrameworkError> {
    resolve_row_hint_placement(context, row.stable_id())?;
    sync_row_parts(context, row)?;
    let visible_box = generates_box(context.world(), row.stable_id());
    let mut assembled = context
        .assembled_slot::<SettingsRowAssembled>(row.stable_id())
        .copied()
        .unwrap_or_default();
    if assembled.last_generates_box != Some(visible_box) {
        assembled.last_generates_box = Some(visible_box);
        context.set_assembled_slot(row.stable_id(), assembled);
        sync_row_group(context, row)?;
    }
    Ok(())
}

/// Give an assembled row the parts it lacks now (a help button, a notice
/// line) and bring its help button in line with the hint and whether it is
/// open. A write from outside the view layer (a group handing down a
/// placement, the application setting a notice on a row it built by hand)
/// reaches the row through here; the view layer reassembles its own rows.
fn sync_row_parts(
    context: &mut AppContext,
    row: Entity<SettingsRow>,
) -> Result<(), FrameworkError> {
    let world = context.world();
    let live = |slot: Option<StableNodeId>| slot.filter(|id| world.contains(*id));
    let assembled = context
        .assembled_slot::<SettingsRowAssembled>(row.stable_id())
        .copied()
        .unwrap_or_default();
    let lacking = live(assembled.copy_slot).is_some()
        && context.read(row, |row| {
            row.shows_help() != live(assembled.help_slot).is_some()
                || (row.shown_notice().is_some() && live(assembled.notice_slot).is_none())
        })?;
    if lacking && !context.reactive_flushing() {
        context.assemble_settings_row(row)?;
        return Ok(());
    }
    sync_row_help(context, row)
}

/// The help button after a row's name, for `hint`: a heading tool's square
/// (24px at the default metrics) to aim at, hanging out of the name's line
/// on every side so the line and the row keep their height; its glyph is
/// the body text's size. Its name is the hint itself. Hovering it previews
/// the hint, until the hint is `open` under the row.
fn help_button(hint: &Arc<str>, open: bool) -> crate::IconButton {
    let mut button = crate::IconButton::new(nana_ui_core::Icon::HelpCircle, Arc::clone(hint))
        .compact(true)
        .expanded(open);
    if !open {
        button = button.with_tooltip(Arc::clone(hint));
    }
    let overhang = (nana_ui_core::SpacingTokens::DEFAULT.page
        - nana_ui_core::TypographyTokens::DEFAULT.body)
        * -0.5;
    let theme_overhang = nana_ui_core::ThemeExpr::from(nana_ui_core::ThemeToken::Space(
        nana_ui_core::SpacingStep::Page,
    ))
    .plus(
        nana_ui_core::ThemeExpr::from(nana_ui_core::ThemeToken::Type(nana_ui_core::TypeRole::Body))
            .scaled(-1.0),
    )
    .scaled(-0.5);
    let layout = Arc::make_mut(&mut button.style.layout);
    layout.margin_top = Some(LengthSpec::Px(overhang));
    layout.margin_bottom = Some(LengthSpec::Px(overhang));
    layout.margin_left = Some(LengthSpec::Px(overhang));
    layout.margin_right = Some(LengthSpec::Px(overhang));
    layout.theme.margin_top = Some(theme_overhang.clone());
    layout.theme.margin_bottom = Some(theme_overhang.clone());
    layout.theme.margin_left = Some(theme_overhang.clone());
    layout.theme.margin_right = Some(theme_overhang);
    button
}

/// Bring `row`'s help button in line with its hint and whether the hint is
/// open: named by the hint, saying whether it is expanded, and previewing
/// the hint on hover only while it is closed.
fn sync_row_help(context: &mut AppContext, row: Entity<SettingsRow>) -> Result<(), FrameworkError> {
    let Some(button) = context
        .assembled_slot::<SettingsRowAssembled>(row.stable_id())
        .and_then(|s| s.help_slot)
    else {
        return Ok(());
    };
    let (hint, open) = context.read(row, |row| {
        (row.hint.clone().unwrap_or_default(), row.help_open)
    })?;
    let button = Entity::<crate::IconButton>::from_stable_id(button);
    let next = help_button(&hint, open);
    let stale = context
        .read(button, |current| {
            current.label != next.label
                || current.tooltip != next.tooltip
                || current.expanded != next.expanded
        })
        .unwrap_or(false);
    if stale {
        context.update_component(button, |current, _| {
            current.label = next.label;
            current.tooltip = next.tooltip;
            current.expanded = next.expanded;
        })?;
    }
    Ok(())
}

/// Whether `id` is a [`SettingsGroup`] or a [`SettingsCard`], and what
/// Let a settings row's label name the control it holds for assistive
/// technology; a control with a label of its own keeps that. A control that
/// left the row (`previous` children) stops being named by it.
fn label_row_control(
    context: &mut AppContext,
    label: StableNodeId,
    control: Option<StableNodeId>,
    previous: &[StableNodeId],
) -> Result<(), FrameworkError> {
    let mut queue = MutationQueue::new();
    let world = context.world();
    for &old in previous {
        if Some(old) != control && world.labelled_by(old) == Some(label) {
            queue.set_labelled_by(old, None);
        }
    }
    if let Some(control) = control
        && world.labelled_by(control) != Some(label)
    {
        queue.set_labelled_by(control, Some(label));
    }
    if !queue.is_empty() {
        context.commit_mutations(queue)?;
    }
    Ok(())
}

pub(super) fn styled_text(
    value: impl Into<String>,
    color: SemanticColorRole,
    role: nana_ui_core::TypeRole,
    weight: nana_ui_core::TextWeight,
) -> Text {
    let mut style = NodeStyle::default();
    style.foreground = Some(color);
    let layout = Arc::make_mut(&mut style.layout);
    crate::theme_intent::font(layout, role);
    crate::theme_intent::weight(layout, weight);
    Text::new(value).style(style)
}

pub(super) fn row_label_text(label: &str) -> Text {
    Text::new(label).style(label_slot_style(label.is_empty(), false, false))
}

pub(super) fn row_hint_text(hint: &str, hidden: bool) -> Text {
    Text::new(hint).style(hint_slot_style(hidden))
}

pub(super) fn sync_text(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    next: Text,
) -> Result<Entity<Text>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::from_stable_id(id);
        context.update_component(entity, |text, _| {
            *text = next;
        })?;
        Ok(entity)
    } else {
        let entity = context.create_detached_component(document, next)?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

pub(super) fn mount_settings_row(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    label: &str,
    hint: Option<&str>,
    divided: bool,
    first: bool,
    last: bool,
    control: StableNodeId,
) -> Result<Entity<SettingsRow>, FrameworkError> {
    let mut label_slot = None;
    let mut hint_slot = None;
    let mut copy_slot = None;
    if let Some(row_id) = *slot {
        if let Ok((existing_label, existing_hint)) = context
            .read(Entity::<SettingsRow>::from_stable_id(row_id), |row| {
                (row.label_slot, row.hint_slot)
            })
        {
            label_slot = existing_label;
            hint_slot = existing_hint;
        }
        copy_slot = context
            .assembled_slot::<SettingsRowAssembled>(row_id)
            .and_then(|s| s.copy_slot);
        if label_slot.is_none()
            && let Some(first_child) = context
                .world()
                .node(row_id)
                .and_then(|node| node.children.first().copied())
                .filter(|child| *child != control)
        {
            let nested = context
                .world()
                .node(first_child)
                .map(|node| node.children.clone())
                .unwrap_or_default();
            if nested.is_empty() {
                label_slot = Some(first_child);
            }
        }
    }
    let label_text = sync_text(context, document, &mut label_slot, row_label_text(label))?;
    let hint_text = sync_text(
        context,
        document,
        &mut hint_slot,
        row_hint_text(hint.unwrap_or(""), hint.is_none()),
    )?;
    let copy = if let Some(id) = copy_slot {
        Entity::<SettingsRowCopy>::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(
            document,
            SettingsRowCopy {
                stacked: false,
                column: None,
            },
        )?;
        copy_slot = Some(entity.stable_id());
        entity
    };
    let hint_id = Some(hint_text.stable_id());
    let copy_id = Some(copy.stable_id());
    let apply = |row: &mut SettingsRow, _: &mut crate::ViewContext<'_, SettingsRow>| {
        row.label = Arc::from(label);
        row.hint = hint.map(Arc::from);
        row.divided = divided;
        row.first_in_group = first;
        row.last_in_group = last;
        row.stack_below = Some(ASSEMBLED_ROW_STACK_BELOW);
        row.control = Some(control);
        row.label_slot = Some(label_text.stable_id());
        row.hint_slot = hint_id;
    };
    let row = if let Some(id) = *slot {
        let entity = Entity::<SettingsRow>::from_stable_id(id);
        let mut assembled = context
            .assembled_slot::<SettingsRowAssembled>(id)
            .copied()
            .unwrap_or_default();
        assembled.copy_slot = copy_id;
        context.set_assembled_slot(id, assembled);
        context.update_component(entity, apply)?;
        entity
    } else {
        let mut row = SettingsRow::new(label)
            .divided(divided)
            .stack_below(ASSEMBLED_ROW_STACK_BELOW)
            .control_child(control)
            .label_slot(label_text.stable_id());
        if let Some(hint) = hint {
            row = row.hint(hint);
        }
        row = row.hint_slot(hint_text.stable_id());
        if first {
            row = row.first_in_group();
        }
        if last {
            row = row.last_in_group();
        }
        let entity = context.create_detached_component(document, row)?;
        context.set_assembled_slot(
            entity.stable_id(),
            SettingsRowAssembled {
                copy_slot,
                ..Default::default()
            },
        );
        context.reproject_component(entity)?;
        *slot = Some(entity.stable_id());
        entity
    };
    reconcile_children(
        context,
        copy,
        &[label_text.stable_id(), hint_text.stable_id()],
    )?;
    let previous = context
        .world()
        .node(row.stable_id())
        .map(|node| node.children.to_vec())
        .unwrap_or_default();
    let flash = context
        .assembled_slot::<SettingsRowAssembled>(row.stable_id())
        .and_then(|s| s.flash_slot)
        .filter(|id| context.world().contains(*id));
    let children: Vec<_> = flash
        .into_iter()
        .chain([copy.stable_id(), control])
        .collect();
    reconcile_children(context, row, &children)?;
    label_row_control(context, label_text.stable_id(), Some(control), &previous)?;
    Ok(row)
}

impl AppContext {
    pub(crate) fn settings_row_line_node(&self, id: StableNodeId) -> StableNodeId {
        self.assembled_slot::<SettingsRowAssembled>(id)
            .and_then(|s| s.main_slot)
            .unwrap_or(id)
    }

    pub(crate) fn refresh_settings_row_layout(
        &mut self,
        id: StableNodeId,
    ) -> Result<(), FrameworkError> {
        let row = Entity::<SettingsRow>::from_stable_id(id);
        let line_node = self.settings_row_line_node(id);
        let needs_projection = self
            .read(row, |row| {
                let direction = if row.effective_stacked(id, self.world()) {
                    FlexDirection::Column
                } else {
                    FlexDirection::Row
                };
                self.world()
                    .node_style(line_node)
                    .is_some_and(|style| style.layout.direction != Some(direction))
            })
            .unwrap_or(false);
        if needs_projection {
            self.reproject_component(row)?;
        }
        Ok(())
    }

    /// Mount a settings row with painted label / optional hint / control slots.
    pub fn mount_settings_leaf_row(
        &mut self,
        document: crate::DocumentId,
        label: &str,
        hint: Option<&str>,
        control: StableNodeId,
    ) -> Result<Entity<SettingsRow>, FrameworkError> {
        let mut slot = None;
        mount_settings_row(
            self, document, &mut slot, label, hint, false, true, true, control,
        )
    }

    /// Give a row its label, hint and copy nodes and put its children in
    /// order: the copy (label over hint), then the control. A row with a
    /// [`SettingsRow::label_accessory`] or a help button
    /// ([`HintPlacement::Help`]) puts the label and those on one line in the
    /// copy; without either the label is the copy's own child, as it always
    /// was. A row with something under it (the hint opened from its help
    /// button, a notice or an error) holds its copy and control in a main
    /// line and stacks what is under it after that line. What the label and
    /// hint say is the row's own `label` / `hint`; its projection writes
    /// them. The view layer runs this after it builds a row
    /// (`TypeBehavior::slot_assembler`); a row made with
    /// [`Self::mount_settings_leaf_row`] is already assembled.
    /// Returns whether anything changed.
    pub fn assemble_settings_row(
        &mut self,
        row: Entity<SettingsRow>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, row.stable_id())?;
        resolve_row_hint_placement(self, row.stable_id())?;
        let (label, hint, hint_is_path, control, label_slot, mut hint_slot) =
            self.read(row, |row| {
                (
                    row.label.clone(),
                    row.hint.clone(),
                    row.hint_is_path,
                    row.control,
                    row.label_slot,
                    row.hint_slot,
                )
            })?;
        let existing_assembled = self
            .assembled_slot::<SettingsRowAssembled>(row.stable_id())
            .copied()
            .unwrap_or_default();
        let copy_slot = existing_assembled.copy_slot;
        let (mark, accessory) = self.read(row, |row| (row.label_mark, row.label_accessory))?;
        let line_slot = existing_assembled.label_line_slot;
        let flash_slot = existing_assembled.flash_slot;
        let (error, wants_notice) = self.read(row, |row| {
            (row.error.is_some(), row.shown_notice().is_some())
        })?;
        let error_shown = existing_assembled.error_shown;
        let (wants_help, help_open) = self.read(row, |row| (row.shows_help(), row.help_open))?;
        let help_slot = existing_assembled.help_slot;
        let help_text_slot = existing_assembled.help_text_slot;
        let notice_slot = existing_assembled.notice_slot;
        let notice_glyph_slot = existing_assembled.notice_glyph_slot;
        let notice_text_slot = existing_assembled.notice_text_slot;
        let main_slot = existing_assembled.main_slot;
        let (column, trailing) = self.read(row, |row| (row.label_column, row.trailing))?;
        let control_slot = existing_assembled.control_slot;
        let value_slot = existing_assembled.value_slot;
        let trailing_slot = existing_assembled.trailing_slot;

        let live = |context: &Self, slot: Option<StableNodeId>| {
            slot.filter(|id| context.world().contains(*id))
        };
        let mut created = false;
        let next_label_slot = match live(self, label_slot) {
            Some(id) => id,
            None => {
                created = true;
                self.create_detached_component(document, row_label_text(&label))?
                    .stable_id()
            }
        };
        if let Some(id) = live(self, hint_slot) {
            let was_path = self
                .read(Entity::<crate::PathText>::from_stable_id(id), |_| ())
                .is_ok();
            if was_path != hint_is_path {
                if was_path {
                    self.remove_view(Entity::<crate::PathText>::from_stable_id(id))?;
                } else {
                    self.remove_view(Entity::<Text>::from_stable_id(id))?;
                }
                hint_slot = None;
            }
        }
        // Behind a help button the hint is not under the name.
        let hint_hidden = hint.is_none() || wants_help;
        let next_hint_slot = match live(self, hint_slot) {
            Some(id) => id,
            None => {
                created = true;
                if hint_is_path {
                    self.create_detached_component(
                        document,
                        crate::PathText::new(hint.as_deref().unwrap_or(""))
                            .style(hint_slot_style(hint_hidden)),
                    )?
                    .stable_id()
                } else {
                    self.create_detached_component(
                        document,
                        row_hint_text(hint.as_deref().unwrap_or(""), hint_hidden),
                    )?
                    .stable_id()
                }
            }
        };
        let next_copy_slot = match live(self, copy_slot) {
            Some(id) => id,
            None => {
                created = true;
                self.create_detached_component(
                    document,
                    SettingsRowCopy {
                        stacked: false,
                        column: None,
                    },
                )?
                .stable_id()
            }
        };
        if hint_is_path {
            self.update_component(
                Entity::<crate::PathText>::from_stable_id(next_hint_slot),
                |text, _| {
                    text.value = hint.clone().unwrap_or_default();
                    text.style = hint_slot_style(hint_hidden);
                },
            )?;
        }

        let mut changed = false;
        // The help button goes with the hint behind it; the opened hint's
        // node is made with it and kept, hidden, after it goes: the row keeps
        // one shape meanwhile.
        let live_help = live(self, help_slot);
        let help = match (wants_help, live_help) {
            (true, Some(id)) => Some(id),
            (true, None) => {
                let hint = hint.clone().unwrap_or_default();
                let button =
                    self.create_detached_component(document, help_button(&hint, help_open))?;
                self.observe(button, row, |row, _: &Activate, cx| {
                    row.help_open = !row.help_open;
                    // The row's lifecycle brings the button along.
                    cx.reassemble();
                })?;
                Some(button.stable_id())
            }
            (false, _) => None,
        };
        let help_text = match (live(self, help_text_slot), help) {
            (Some(id), _) => Some(id),
            (None, Some(_)) => Some(
                self.create_detached_component(
                    document,
                    Text::new(hint.as_deref().unwrap_or("")).style(hint_slot_style(true)),
                )?
                .stable_id(),
            ),
            (None, None) => None,
        };
        // The notice line is made the first time there is a notice or an
        // error and kept, hidden, after they clear: a row that was wrong
        // once is likely to be again.
        let (notice, next_notice_glyph, next_notice_text) =
            match (live(self, notice_slot), wants_notice) {
                (Some(id), _) => (Some(id), notice_glyph_slot, notice_text_slot),
                (None, true) => {
                    let glyph = self
                        .create_detached_component(
                            document,
                            notice_glyph(nana_ui_core::ValidationIntent::Warning),
                        )?
                        .stable_id();
                    let text = self
                        .create_detached_component(
                            document,
                            notice_text("", nana_ui_core::ValidationIntent::Warning),
                        )?
                        .stable_id();
                    let notice = self
                        .create_detached_component(document, notice_box_style(None))?
                        .stable_id();
                    self.reconcile_children(notice, &[glyph, text])?;
                    (Some(notice), Some(glyph), Some(text))
                }
                (None, false) => (None, notice_glyph_slot, notice_text_slot),
            };
        // Something under the row puts the copy and the control on a main
        // line of their own, kept from then on.
        let main = match (
            live(self, main_slot),
            help_text.is_some() || notice.is_some(),
        ) {
            (Some(id), _) => Some(id),
            (None, true) => Some(
                self.create_detached_component(
                    document,
                    SettingsRowMain {
                        line: NodeStyle::default(),
                    },
                )?
                .stable_id(),
            ),
            (None, false) => None,
        };

        let copy = Entity::<SettingsRowCopy>::from_stable_id(next_copy_slot);
        let mark = live(self, mark);
        let accessory = live(self, accessory);
        let live_line = live(self, line_slot);
        let line = match (
            mark.is_some() || accessory.is_some() || help.is_some(),
            live_line,
        ) {
            (true, Some(line)) => Some(line),
            (true, None) => Some(
                self.create_detached_component(document, SettingsRowLabelLine)?
                    .stable_id(),
            ),
            (false, _) => None,
        };
        let line_changed = line != line_slot;
        if let Some(line) = line {
            let on_line: Vec<_> = std::iter::once(next_label_slot)
                .chain(mark)
                .chain(help)
                .chain(accessory)
                .collect();
            changed |= self.reconcile_children(line, &on_line)?;
        }
        let copy_children = [line.unwrap_or(next_label_slot), next_hint_slot];
        changed |= reconcile_children(self, copy, &copy_children)?;
        if line_changed {
            // The label's width rule follows where it sits; the row's
            // projection writes it too, from the slot written below.
            let text = Entity::<Text>::from_stable_id(next_label_slot);
            if self.read(text, |_| ()).is_ok() {
                self.update_component(text, |text, _| {
                    text.style =
                        label_slot_style(label.is_empty(), line.is_some(), column.is_some());
                })?;
            }
        }

        let control = live(self, control);
        // a label column or a trailing node puts the control in a box of its
        // own, which holds the trailing node after it
        let trailing = live(self, trailing);
        let live_box = live(self, control_slot);
        let control_box = match (column.is_some() || trailing.is_some(), live_box) {
            (true, Some(id)) => Some(id),
            (true, None) => Some(
                self.create_detached_component(
                    document,
                    SettingsRowControl {
                        stacked: false,
                        grows: column.is_some(),
                    },
                )?
                .stable_id(),
            ),
            (false, _) => None,
        };
        // beside a trailing node the control sits in a box that keeps its
        // minimum, and the trailing node in one that gives way
        let (live_value, live_trailing) = (live(self, value_slot), live(self, trailing_slot));
        let (value_box, trailing_box) = match (trailing, live_value, live_trailing) {
            (Some(_), Some(value), Some(trailing)) => (Some(value), Some(trailing)),
            (Some(_), _, _) => (
                Some(
                    self.create_detached_component(document, SettingsRowValue)?
                        .stable_id(),
                ),
                Some(
                    self.create_detached_component(document, SettingsRowTrailing)?
                        .stable_id(),
                ),
            ),
            (None, _, _) => (None, None),
        };
        if let (Some(value_box), Some(trailing_box), Some(trailing)) =
            (value_box, trailing_box, trailing)
        {
            changed |=
                self.reconcile_children(value_box, &control.into_iter().collect::<Vec<_>>())?;
            changed |= self.reconcile_children(trailing_box, &[trailing])?;
        }
        if let Some(control_box) = control_box {
            let held: Vec<_> = match (value_box, trailing_box) {
                (Some(value), Some(trailing)) => vec![value, trailing],
                _ => control.into_iter().collect(),
            };
            changed |= self.reconcile_children(control_box, &held)?;
        }

        let box_changed = control_box != control_slot;
        let line_parts: Vec<_> = std::iter::once(next_copy_slot)
            .chain(control_box.or(control))
            .collect();
        let children: Vec<_> = match main {
            Some(main) => {
                changed |= self.reconcile_children(main, &line_parts)?;
                live(self, flash_slot)
                    .into_iter()
                    .chain([main])
                    .chain(help_text)
                    .chain(notice)
                    .collect()
            }
            None => live(self, flash_slot)
                .into_iter()
                .chain(line_parts)
                .collect(),
        };
        let previous = self
            .world()
            .node(row.stable_id())
            .map(|node| node.children.to_vec())
            .unwrap_or_default();
        changed |= reconcile_children(self, row, &children)?;

        let slots_changed = created
            || help != live_help
            || help_text != help_text_slot
            || notice != notice_slot
            || next_notice_glyph != notice_glyph_slot
            || next_notice_text != notice_text_slot
            || main != main_slot
            || line_changed
            || error_shown != Some(error)
            || value_box != value_slot
            || trailing_box != trailing_slot
            || box_changed;
        if slots_changed {
            self.set_assembled_slot(
                row.stable_id(),
                SettingsRowAssembled {
                    help_slot: help,
                    help_text_slot: help_text,
                    notice_slot: notice,
                    notice_glyph_slot: next_notice_glyph,
                    notice_text_slot: next_notice_text,
                    main_slot: main,
                    error_shown: Some(error),
                    copy_slot: Some(next_copy_slot),
                    label_line_slot: line,
                    flash_slot,
                    control_slot: control_box,
                    value_slot: value_box,
                    trailing_slot: trailing_box,
                    last_generates_box: existing_assembled.last_generates_box,
                },
            );
            if label_slot != Some(next_label_slot) || hint_slot != Some(next_hint_slot) {
                self.update_component(row, |row, _| {
                    row.label_slot = Some(next_label_slot);
                    row.hint_slot = Some(next_hint_slot);
                })?;
            } else {
                self.reproject_component(row)?;
            }
        }

        // Hand back children from retired slots before removing them.
        if line_changed && let Some(old) = live_line.filter(|old| Some(*old) != line) {
            self.reconcile_children(old, &[])?;
            self.remove_view(Entity::<SettingsRowLabelLine>::from_stable_id(old))?;
        }
        if let Some(old) = live_help.filter(|old| Some(*old) != help) {
            self.remove_view(Entity::<crate::IconButton>::from_stable_id(old))?;
        }
        if value_box != value_slot || trailing_box != trailing_slot {
            if let Some(old) = live_value.filter(|old| Some(*old) != value_box) {
                self.reconcile_children(old, &[])?;
                self.remove_view(Entity::<SettingsRowValue>::from_stable_id(old))?;
            }
            if let Some(old) = live_trailing.filter(|old| Some(*old) != trailing_box) {
                self.reconcile_children(old, &[])?;
                self.remove_view(Entity::<SettingsRowTrailing>::from_stable_id(old))?;
            }
            changed = true;
        }
        if box_changed && let Some(old) = live_box.filter(|old| Some(*old) != control_box) {
            self.reconcile_children(old, &[])?;
            self.remove_view(Entity::<SettingsRowControl>::from_stable_id(old))?;
        }

        // A reason that has just appeared on a row already there takes the
        // focus to the field it is about, when the focus is in this document
        // (on the button that submitted, say): the user lands where the fix
        // goes, and a screen reader reads the field and the reason. A row
        // built with its error, or focus in another window, is left alone.
        if error
            && error_shown == Some(false)
            && self.world().focused(document).is_some()
            && let Some(control) = control
        {
            self.focus_first_in(document, control)?;
        }

        if help != live_help || help_text != help_text_slot || main != main_slot {
            changed = true;
        }
        sync_row_help(self, row)?;
        label_row_control(self, next_label_slot, control, &previous)?;
        Ok(created || changed || line_changed || box_changed)
    }

    /// Flash `row` to draw the eye to it, for a row the application has just
    /// brought into view because the user came to see it: an accent tint
    /// shows under the row, holds, and fades out, and then nothing of it
    /// remains. Flashing a row that is still flashing starts it over.
    ///
    /// The tint is a node of the row's own, out of flow and not hit, so the
    /// row lays out and takes pointers as it would without it; it reaches
    /// past the row's copy and control by a list item's inset on either side
    /// and covers the row top to bottom. The flash is its opacity on the
    /// presentation layer, on a timeline from the frame after this call
    /// (the frame the row first shows in, when the call also scrolled it
    /// there) for the installed theme's [`nana_ui_core::MotionRole::Attention`]:
    /// full for the first fifth, then out on the theme's standard easing.
    /// Under reduced motion ([`Self::reduced_motion`]) the tint stays full
    /// for that whole time instead of fading, then goes at once. The tint
    /// leaves with the row, should the row go first.
    pub fn flash_settings_row(&mut self, row: Entity<SettingsRow>) -> Result<(), FrameworkError> {
        let document = document_of(self, row.stable_id())?;
        let flash = self
            .assembled_slot::<SettingsRowAssembled>(row.stable_id())
            .and_then(|s| s.flash_slot);
        let flash = match flash.filter(|id| self.world().contains(*id)) {
            Some(id) => id,
            None => {
                let id = self
                    .create_detached_component(document, SettingsRowFlash)?
                    .stable_id();
                let mut assembled = self
                    .assembled_slot::<SettingsRowAssembled>(row.stable_id())
                    .copied()
                    .unwrap_or_default();
                assembled.flash_slot = Some(id);
                self.set_assembled_slot(row.stable_id(), assembled);
                self.reproject_component(row)?;
                // First, so it paints under everything else in the row.
                let children: Vec<_> = std::iter::once(id)
                    .chain(
                        self.world()
                            .node(row.stable_id())
                            .map(|node| node.children.to_vec())
                            .unwrap_or_default(),
                    )
                    .collect();
                self.reconcile_children(row.stable_id(), &children)?;
                id
            }
        };
        self.arm_attention(flash)
    }

    /// One of a flash's timelines ended at `now`: the frame-long one
    /// [`Self::flash_settings_row`] armed starts the flash from this frame;
    /// the flash itself takes its tint away.
    pub(crate) fn advance_settings_row_flash(
        &mut self,
        flash: StableNodeId,
        property: crate::AnimatableProperty,
        now: std::time::Duration,
    ) -> Result<(), FrameworkError> {
        let entity = Entity::<SettingsRowFlash>::from_stable_id(flash);
        if self.read(entity, |_| ()).is_err() {
            return Ok(());
        }
        if property == crate::AnimatableProperty::Progress {
            let theme = self.world().theme();
            let length = theme.duration(nana_ui_core::MotionRole::Attention);
            let motion = theme.motion();
            let easing = motion.easing(nana_ui_core::EasingRole::Standard);
            if let Some(spec) = settings_row_flash_animation(
                flash,
                now,
                length,
                motion.attention_hold,
                easing,
                self.reduced_motion(),
            ) {
                let mut mutations = MutationQueue::new();
                mutations.start_animation(spec);
                self.commit_mutations(mutations)?;
            }
            return Ok(());
        }
        if let Some(parent) = self.world().parent_id(flash) {
            if let Some(mut assembled) =
                self.assembled_slot::<SettingsRowAssembled>(parent).copied()
                && assembled.flash_slot == Some(flash)
            {
                assembled.flash_slot = None;
                self.set_assembled_slot(parent, assembled);
                self.reproject_component(Entity::<SettingsRow>::from_stable_id(parent))?;
            }
        }
        self.remove_view(entity)?;
        Ok(())
    }
}
