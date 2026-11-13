use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(test)]
use nana_ui_core::UI_METRICS;
use nana_ui_core::{
    AlignSpec, AppearanceEvent, AppearanceSettings, BackdropTarget, ButtonKind, CardKind,
    ControlSize, FlexDirection, FlexWrap, Icon, JustifySpec, LengthSpec, LineHeightSpec,
    OverflowSpec, PaddingSpec, SemanticColorRole, SettingsModel, SettingsState, SettingsTabId,
    ThemeAppearance, ThemeChoice, ThemeId, WindowMaterialMode,
};

use crate::view_components::{
    Button, Card, RangeField, RangeInput, Switch, Text, ToggleChanged, project_common,
};
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AppContext, ComponentView, Entity,
    FrameworkError, InteractionState, InteractionStyle, ListItemSlots, MutationQueue, NodeKind,
    NodeStyle, ScrollAxes, ScrollView, SegmentedControl, SegmentedOption,
    SegmentedSelectionRequested, SemanticPaint, SidebarFrame, SidebarRow, SidebarRowState,
    StableNodeId, StandardVisual, TextContent, UiWorld,
};

const ROW_PADDING_Y: f32 = nana_ui_core::space::SM;
const ROW_GROUP_PADDING_Y: f32 = nana_ui_core::space::XXS;
const ROW_STACK_GAP: f32 = nana_ui_core::space::SM;
const ROW_STACK_GAP_LOOSE: f32 = nana_ui_core::space::LG;
const ROW_INLINE_GAP: f32 = nana_ui_core::space::MD;
const ROW_INLINE_GAP_LOOSE: f32 = nana_ui_core::space::XXL;
const ROW_COPY_GAP: f32 = nana_ui_core::space::XXS;
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
    /// The help button after the label, only while the hint goes behind
    /// one ([`HintPlacement::Help`]) and there is a hint.
    pub(crate) help_slot: Option<StableNodeId>,
    /// The hint opened in place under the row, made with the help button
    /// and hidden while it is closed.
    pub(crate) help_text_slot: Option<StableNodeId>,
    /// The notice line under the row (its tinted box, glyph and text), made
    /// the first time the row has a notice or an error and hidden while it
    /// has neither.
    pub(crate) notice_slot: Option<StableNodeId>,
    pub(crate) notice_glyph_slot: Option<StableNodeId>,
    pub(crate) notice_text_slot: Option<StableNodeId>,
    /// The row's main line (the copy, then the control), only once the row
    /// has something under it (an opened hint, a notice): the row is then a
    /// column of that line and what is under it, and the control stays
    /// centred on the line.
    pub(crate) main_slot: Option<StableNodeId>,
    /// Whether the last assembly showed an error; `None` before the first.
    /// A reason that appears on a row already there takes the focus to the
    /// row's control (see [`AppContext::assemble_settings_row`]).
    pub(crate) error_shown: Option<bool>,
    pub(crate) copy_slot: Option<StableNodeId>,
    /// The line holding the label, its help button and its accessory, only
    /// while there is a help button or an accessory: without either the
    /// label sits in the copy itself.
    pub(crate) label_line_slot: Option<StableNodeId>,
    /// The tint [`AppContext::flash_settings_row`] paints under the row, only
    /// while it flashes.
    pub(crate) flash_slot: Option<StableNodeId>,
    /// Holds the control and the trailing node, only while the row has a
    /// label column or a trailing node.
    pub(crate) control_slot: Option<StableNodeId>,
    /// Hold the control and the trailing node apart, only while there is a
    /// trailing node: the control keeps at least [`row_control_min`] and the
    /// trailing node gives way, so one that asks for the full width (a plain
    /// row of buttons) cannot squeeze the control out.
    pub(crate) value_slot: Option<StableNodeId>,
    pub(crate) trailing_slot: Option<StableNodeId>,
    pub style: NodeStyle,
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
            help_slot: None,
            help_text_slot: None,
            notice_slot: None,
            notice_glyph_slot: None,
            notice_text_slot: None,
            main_slot: None,
            error_shown: None,
            copy_slot: None,
            label_line_slot: None,
            flash_slot: None,
            control_slot: None,
            value_slot: None,
            trailing_slot: None,
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

    pub fn copy_slot(mut self, copy: StableNodeId) -> Self {
        self.copy_slot = Some(copy);
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

    fn effective_style(&self, stacked: bool) -> NodeStyle {
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
        if self.main_slot.is_some() {
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
            let padded_y = layout.padding.is_some() || layout.theme.padding.is_some();
            if !padded_y && layout.padding_top.is_none() && layout.theme.padding_top.is_none() {
                layout.theme.padding_top =
                    Some(crate::theme_intent::space(if self.first_in_group {
                        nana_ui_core::SpacingStep::Xxs
                    } else {
                        nana_ui_core::SpacingStep::Sm
                    }));
                layout.padding_top = Some(LengthSpec::Px(if self.first_in_group {
                    ROW_GROUP_PADDING_Y
                } else {
                    ROW_PADDING_Y
                }));
            }
            if !padded_y && layout.padding_bottom.is_none() && layout.theme.padding_bottom.is_none()
            {
                layout.theme.padding_bottom =
                    Some(crate::theme_intent::space(if self.last_in_group {
                        nana_ui_core::SpacingStep::Xxs
                    } else {
                        nana_ui_core::SpacingStep::Sm
                    }));
                layout.padding_bottom = Some(LengthSpec::Px(if self.last_in_group {
                    ROW_GROUP_PADDING_Y
                } else {
                    ROW_PADDING_Y
                }));
            }
        }
        // Only the bottom edge, and the top one under other content, are
        // named; the others fall back to the unset uniform width, which is
        // none.
        layout.border_width = None;
        layout.border_bottom_width = divider.then_some(nana_ui_core::HAIRLINE);
        layout.theme.border_bottom_width = divider
            .then(|| nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
        layout.border_top_width = rule_above.then_some(nana_ui_core::HAIRLINE);
        layout.theme.border_top_width = rule_above
            .then(|| nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
        layout.font_size = Some(nana_ui_core::type_scale::BODY);
        crate::theme_intent::font(layout, nana_ui_core::TypeRole::Body);
        layout.line_height = Some(LineHeightSpec::Absolute(nana_ui_core::type_scale::BODY));
        layout.theme.line_height =
            Some(nana_ui_core::ThemeToken::Type(nana_ui_core::TypeRole::Body).into());
        layout.font_weight = Some(nana_ui_core::type_scale::MEDIUM);
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
            (true, true) => nana_ui_core::SpacingStep::Lg,
            (true, false) => nana_ui_core::SpacingStep::Sm,
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

    /// The node whose direction says whether the row is stacked: the row,
    /// or its main line.
    fn line_node(&self, id: StableNodeId) -> StableNodeId {
        self.main_slot.unwrap_or(id)
    }
}

impl ComponentView for SettingsRow {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        hooks: Some(|hooks| {
            crate::framework::routes::settings_row(hooks);
        }),
        slot_assembler: Some(AppContext::assemble_settings_row),
        lifecycle: Some(sync_settings_row),
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
        // Structured children/slots paint label and hint. Rust-only rows without
        // a tree keep the label as node text so Scene can still show it.
        let has_slots = self.label_slot.is_some()
            || self.hint_slot.is_some()
            || self.control.is_some()
            || self.copy_slot.is_some()
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
        if world.standard_visual(id).is_some() {
            mutations.set_standard_visual(id, None);
        }
        let stacked = self.effective_stacked(id, world);
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(stacked),
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
        self.project_slots(world, mutations, stacked);
    }
}

impl SettingsRow {
    fn project_slots(&self, world: &UiWorld, mutations: &mut MutationQueue, stacked: bool) {
        if let Some(copy) = self.copy_slot {
            SettingsRowCopy {
                stacked,
                column: self.label_column,
            }
            .project(copy, world, mutations);
        }
        if let Some(line) = self.label_line_slot {
            SettingsRowLabelLine.project(line, world, mutations);
        }
        if let Some(label) = self.label_slot {
            let mut text = Text::new(self.label.as_ref());
            text.style = label_slot_style(
                self.label.is_empty(),
                self.label_line_slot.is_some(),
                self.label_column.is_some(),
            );
            text.style.color_overrides = self.label_colors.clone();
            text.project(label, world, mutations);
        }
        if let Some(control) = self.control_slot {
            SettingsRowControl {
                stacked,
                grows: self.label_column.is_some(),
            }
            .project(control, world, mutations);
        }
        if let Some(flash) = self.flash_slot {
            SettingsRowFlash.project(flash, world, mutations);
        }
        if let Some(main) = self.main_slot {
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
        if let Some(text) = self.help_text_slot {
            Text::new(self.hint.as_deref().unwrap_or(""))
                .style(hint_slot_style(!(help && self.help_open)))
                .project(text, world, mutations);
        }
        let notice = self.shown_notice();
        if let Some(slot) = self.notice_slot {
            notice_box_style(notice.map(|(_, tone)| tone)).project(slot, world, mutations);
        }
        let tone = notice.map_or(nana_ui_core::ValidationIntent::Warning, |(_, tone)| tone);
        if let Some(glyph) = self.notice_glyph_slot {
            notice_glyph(tone).project(glyph, world, mutations);
        }
        if let Some(text) = self.notice_text_slot {
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
fn label_slot_style(hidden: bool, on_line: bool, column: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.foreground = Some(if column {
        SemanticColorRole::Muted
    } else {
        SemanticColorRole::Text
    });
    style.text_vertical_alignment = crate::TextVerticalAlignment::Center;
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(nana_ui_core::type_scale::BODY);
    crate::theme_intent::font(layout, nana_ui_core::TypeRole::Body);
    layout.line_height = Some(LineHeightSpec::Absolute(nana_ui_core::type_scale::BODY));
    layout.theme.line_height =
        Some(nana_ui_core::ThemeToken::Type(nana_ui_core::TypeRole::Body).into());
    let (weight, step) = if column {
        (
            nana_ui_core::type_scale::REGULAR,
            nana_ui_core::TextWeight::Regular,
        )
    } else {
        (
            nana_ui_core::type_scale::MEDIUM,
            nana_ui_core::TextWeight::Medium,
        )
    };
    layout.font_weight = Some(weight);
    crate::theme_intent::weight(layout, step);
    if !on_line {
        layout.width = Some(LengthSpec::Fill);
        layout.flex_grow = Some(1.0);
    }
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.white_space_nowrap = true;
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

/// How faint a notice's box is: a tint of its colour, as a callout's.
const NOTICE_TINT: f32 = 0.10;

/// The notice line's box: a faint tint of the tone's colour with the
/// control radius, across the row, the glyph on the text's first line.
/// `None` hides it: a row without a notice keeps no room or gap for one.
fn notice_box_style(tone: Option<nana_ui_core::ValidationIntent>) -> crate::Stack {
    let role = notice_role(tone.unwrap_or(nana_ui_core::ValidationIntent::Warning));
    crate::Stack::row(nana_ui_core::space::XS)
        .gap_step(nana_ui_core::SpacingStep::Xs)
        .width(LengthSpec::Fill)
        .align(AlignSpec::Start)
        .padding_step(nana_ui_core::SpacingStep::Sm, nana_ui_core::SpacingStep::Xs)
        .radius(nana_ui_core::RadiusTier::Sm)
        .surface_mix(nana_ui_core::SemanticColorMix::alpha(role, NOTICE_TINT))
        .with_layout(|layout| layout.hidden = tone.is_none())
}

/// The notice's glyph: a caution triangle or an error circle, as tall as the
/// body text and centred on the text's first line.
fn notice_glyph(tone: nana_ui_core::ValidationIntent) -> crate::IconGlyph {
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
fn notice_text(value: &str, tone: nana_ui_core::ValidationIntent) -> Text {
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

fn hint_slot_style(hidden: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.foreground = Some(SemanticColorRole::Muted);
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(nana_ui_core::type_scale::META);
    crate::theme_intent::font(layout, nana_ui_core::TypeRole::Meta);
    layout.font_weight = Some(nana_ui_core::type_scale::REGULAR);
    crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Regular);
    // Its own line box: the row's is the label's, too tight for the lines of
    // a hint that wraps.
    layout.line_height = Some(LineHeightSpec::Absolute(nana_ui_core::type_scale::LINE));
    layout.theme.line_height =
        Some(nana_ui_core::ThemeToken::Line(nana_ui_core::LineRole::Line).into());
    layout.width = Some(LengthSpec::Fill);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    // The hint says what the setting does; cut to one line it stops saying
    // it. It wraps under the label, while the label stays a single line.
    layout.hidden = hidden;
    style
}

#[derive(Debug, Clone, PartialEq)]
struct SettingsRowCopy {
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
        layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
        layout.flex_grow = Some(if self.stacked { 0.0 } else { 1.0 });
        layout.flex_shrink = Some(1.0);
        layout.min_width = Some(LengthSpec::Px(0.0));
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
struct SettingsRowControl {
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
struct SettingsRowValue;

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
struct SettingsRowTrailing;

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
struct SettingsRowMain {
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
struct SettingsRowLabelLine;

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
struct SettingsRowFlash;

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
        hooks: Some(|hooks| {
            crate::framework::routes::settings_row(hooks);
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

/// Surface grouping for settings rows.
///
/// [`Self::hint_placement`](field@Self::hint_placement) says where the hints of every
/// [`SettingsRow`] in the card go, through the groups inside, as on a
/// [`SettingsGroup`]: a card of inspector groups sets it once.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsCard {
    pub title: Arc<str>,
    pub style: NodeStyle,
    /// Where the hints of the rows inside go; see
    /// [`SettingsGroup::hint_placement`](field@SettingsGroup::hint_placement).
    pub hint_placement: Option<HintPlacement>,
    /// The placement the card last handed down to its rows.
    pub(crate) pushed_hint_placement: Option<HintPlacement>,
}

impl SettingsCard {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            style: NodeStyle::default(),
            hint_placement: None,
            pushed_hint_placement: None,
        }
    }

    /// Put the hints of the rows inside where `placement` says.
    pub fn hint_placement(mut self, placement: HintPlacement) -> Self {
        self.hint_placement = Some(placement);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn card(&self) -> Card {
        let mut card = Card::new().kind(CardKind::Surface);
        if !self.title.is_empty() {
            card = card.title(Arc::clone(&self.title));
        }
        let mut style = self.style.clone();
        Arc::make_mut(&mut style.layout)
            .width
            .get_or_insert(LengthSpec::Fill);
        card.style = style;
        card
    }
}

impl ComponentView for SettingsCard {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        lifecycle: Some(sync_settings_card),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.card().project(id, world, mutations);
    }
}

/// A column of settings rows and other content whose group edges follow what
/// is shown.
///
/// The first shown member of the group gets the group's edge air above it,
/// the last one below it, and a divided row draws no divider under the last
/// shown member — however the rows are shown or hidden. A [`SettingsRow`]
/// directly in the group has its `first_in_group` / `last_in_group` written
/// by the group; any other member (a badge, a value list, a row of buttons)
/// takes its place in the order without being written. The container of a
/// keyed list or a conditional block is read through, so its rows are
/// members in their own right. A [`crate::Divider`] member is a separator:
/// it shows only between two shown members.
///
/// [`Self::hint_placement`](field@Self::hint_placement) says where the hints of the rows inside go, for
/// every [`SettingsRow`] below the group (through any container, a nested
/// group or card that sets its own aside); a row's own placement wins.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsGroup {
    pub style: NodeStyle,
    /// Where the hints of the rows inside go; `None` leaves it to the group
    /// or card around this one, and to [`HintPlacement::Below`] when none
    /// says.
    pub hint_placement: Option<HintPlacement>,
    /// The placement the group last handed down to its rows.
    pub(crate) pushed_hint_placement: Option<HintPlacement>,
}

impl Default for SettingsGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsGroup {
    pub fn new() -> Self {
        let mut style = NodeStyle::default();
        let layout = style.layout_mut();
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        Self {
            style,
            hint_placement: None,
            pushed_hint_placement: None,
        }
    }

    /// Put the hints of the rows inside where `placement` says; see
    /// [`Self::hint_placement`](field@Self::hint_placement).
    pub fn hint_placement(mut self, placement: HintPlacement) -> Self {
        self.hint_placement = Some(placement);
        self
    }

    /// Space between members.
    pub fn gap(mut self, gap: f32) -> Self {
        self.style.layout_mut().gap = Some(LengthSpec::Px(gap.max(0.0)));
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for SettingsGroup {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        hooks: Some(|hooks| {
            crate::framework::routes::settings_group(hooks);
        }),
        lifecycle: Some(sync_settings_group),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-group".into(),
        }
    }

    /// A member shown or hidden moves the group's edges.
    fn wants_child_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState::default(),
        );
    }
}

/// Whether `id` takes part in layout: neither hidden nor `display: none`.
fn generates_box(world: &UiWorld, id: StableNodeId) -> bool {
    world.node_style(id).is_some_and(|style| {
        !style.layout.hidden && style.layout.display != Some(nana_ui_core::DisplaySpec::None)
    })
}

/// A group's members in order, read through structural containers, which
/// land in `containers`. A separator is listed whether it shows or not: the
/// group decides that.
fn group_members(
    context: &AppContext,
    node: StableNodeId,
    members: &mut Vec<(StableNodeId, bool)>,
    containers: &mut Vec<StableNodeId>,
) {
    let Some(snapshot) = context.world().node(node) else {
        return;
    };
    for child in snapshot.children {
        let separator = context
            .read(Entity::<crate::Divider>::from_stable_id(child), |_| ())
            .is_ok();
        if separator {
            members.push((child, true));
            continue;
        }
        if !generates_box(context.world(), child) {
            continue;
        }
        let structural = context
            .read(Entity::<crate::Stack>::from_stable_id(child), |stack| {
                stack.is_structural()
            })
            .unwrap_or(false);
        if structural {
            containers.push(child);
            group_members(context, child, members, containers);
        } else {
            members.push((child, false));
        }
    }
}

std::thread_local! {
    /// Set while a group marks its rows, so the rows it writes do not ask
    /// it again.
    static SYNCING_GROUP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A row shown or hidden changes its group while the row itself is out being
/// written, so the group cannot read it then: once the row is back, it asks
/// its group (through any list or conditional block container) again.
fn sync_row_group(
    context: &mut AppContext,
    row: Entity<SettingsRow>,
) -> Result<(), FrameworkError> {
    if SYNCING_GROUP.get() {
        return Ok(());
    }
    let mut node = row.stable_id();
    while let Some(parent) = context.world().parent_id(node) {
        let group = Entity::<SettingsGroup>::from_stable_id(parent);
        if context.read(group, |_| ()).is_ok() {
            return sync_settings_group(context, group);
        }
        let structural = context
            .read(Entity::<crate::Stack>::from_stable_id(parent), |stack| {
                stack.is_structural()
            })
            .unwrap_or(false);
        if !structural {
            break;
        }
        node = parent;
    }
    Ok(())
}

fn sync_settings_group(
    context: &mut AppContext,
    group: Entity<SettingsGroup>,
) -> Result<(), FrameworkError> {
    if context.defer_settings_group(group.stable_id()) {
        return Ok(());
    }
    mark_settings_group(context, group)
}

/// Mark `group`'s edges now.
pub(crate) fn mark_settings_group(
    context: &mut AppContext,
    group: Entity<SettingsGroup>,
) -> Result<(), FrameworkError> {
    let reentered = SYNCING_GROUP.replace(true);
    let result = mark_group_edges(context, group);
    SYNCING_GROUP.set(reentered);
    result
}

fn mark_group_edges(
    context: &mut AppContext,
    group: Entity<SettingsGroup>,
) -> Result<(), FrameworkError> {
    let mut members = Vec::new();
    let mut containers = Vec::new();
    group_members(context, group.stable_id(), &mut members, &mut containers);
    // Rows inside a list or a conditional block hide on their container,
    // not on the group, so the group watches those containers too.
    context.watch_child_structures(group.stable_id(), &containers);

    let shown: Vec<StableNodeId> = members
        .iter()
        .filter(|(_, separator)| !separator)
        .map(|(id, _)| *id)
        .collect();
    let (first, last) = (shown.first().copied(), shown.last().copied());
    // A row under a shown member that is not a settings row rules its top
    // edge: a settings row's edge is lined whichever side the other content
    // is on (docs/guide/composition.md「分隔线」).
    let is_row = |id: StableNodeId| {
        context
            .read(Entity::<SettingsRow>::from_stable_id(id), |_| ())
            .is_ok()
    };
    let after_other: HashSet<StableNodeId> = shown
        .windows(2)
        .filter(|pair| !is_row(pair[0]) && is_row(pair[1]))
        .map(|pair| pair[1])
        .collect();
    for (index, &(id, separator)) in members.iter().enumerate() {
        if separator {
            let before = members[..index].iter().any(|(_, separator)| !separator);
            let after = members[index + 1..].iter().any(|(_, separator)| !separator);
            // Of two separators with nothing shown between them, the later
            // one stands.
            let next_is_separator = members
                .get(index + 1)
                .is_some_and(|(_, separator)| *separator);
            let hidden = !(before && after) || next_is_separator;
            let divider = Entity::<crate::Divider>::from_stable_id(id);
            let current = context.read(divider, |divider| divider.style.layout.hidden);
            if current.is_ok_and(|current| current != hidden) {
                context.update_component(divider, |divider, _| {
                    divider.style.layout_mut().hidden = hidden;
                })?;
            }
            continue;
        }
        let row = Entity::<SettingsRow>::from_stable_id(id);
        let edges = (
            Some(id) == first,
            Some(id) == last,
            after_other.contains(&id),
        );
        let current = context.read(row, |row| {
            (row.first_in_group, row.last_in_group, row.rule_above)
        });
        if current.is_ok_and(|current| current != edges) {
            context.update_component(row, |row, _| {
                (row.first_in_group, row.last_in_group, row.rule_above) = edges;
            })?;
        }
    }
    let (own, pushed) = context.read(group, |group| {
        (group.hint_placement, group.pushed_hint_placement)
    })?;
    if hand_down_hint_placement(context, group.stable_id(), own, pushed)? {
        context.update_component(group, |group, _| group.pushed_hint_placement = own)?;
    }
    Ok(())
}

std::thread_local! {
    /// The rows being assembled now, so a write the assembly makes does not
    /// start the same row's assembly again from its lifecycle.
    static ROW_ASSEMBLING: std::cell::RefCell<HashSet<StableNodeId>> =
        std::cell::RefCell::new(HashSet::new());
}

/// A row written: where its hint goes may have changed (its container's
/// placement, its own), so are its parts (the help button, the notice line)
/// the ones that need, and its group's edges.
fn sync_settings_row(
    context: &mut AppContext,
    row: Entity<SettingsRow>,
) -> Result<(), FrameworkError> {
    resolve_row_hint_placement(context, row.stable_id())?;
    sync_row_parts(context, row)?;
    sync_row_group(context, row)
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
    let id = row.stable_id();
    if ROW_ASSEMBLING.with_borrow(|rows| rows.contains(&id)) {
        return Ok(());
    }
    let world = context.world();
    let live = |slot: Option<StableNodeId>| slot.filter(|id| world.contains(*id));
    let lacking = context.read(row, |row| {
        live(row.copy_slot).is_some()
            && (row.shows_help() != live(row.help_slot).is_some()
                || (row.shown_notice().is_some() && live(row.notice_slot).is_none()))
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
        button = button.tooltip(
            Arc::clone(hint),
            nana_ui_core::TooltipConfig {
                placement: nana_ui_core::TooltipPlacement::Bottom,
                ..nana_ui_core::TooltipConfig::default()
            },
        );
    }
    let overhang = (nana_ui_core::space::PAGE - nana_ui_core::type_scale::BODY) * -0.5;
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
    let Some((button, hint, open)) = context.read(row, |row| {
        row.help_slot
            .map(|id| (id, row.hint.clone().unwrap_or_default(), row.help_open))
    })?
    else {
        return Ok(());
    };
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
/// placement it sets when it is.
fn hint_scope(context: &AppContext, id: StableNodeId) -> Option<Option<HintPlacement>> {
    context
        .read(Entity::<SettingsGroup>::from_stable_id(id), |group| {
            group.hint_placement
        })
        .or_else(|_| {
            context.read(Entity::<SettingsCard>::from_stable_id(id), |card| {
                card.hint_placement
            })
        })
        .ok()
}

/// The placement the nearest group or card above `id` that sets one gives.
fn inherited_hint_placement(context: &AppContext, id: StableNodeId) -> Option<HintPlacement> {
    let mut node = context.world().parent_id(id);
    while let Some(parent) = node {
        if let Some(Some(placement)) = hint_scope(context, parent) {
            return Some(placement);
        }
        node = context.world().parent_id(parent);
    }
    None
}

/// Write the placement `row`'s containers give it. While a view is being
/// built the row may not stand in its container yet: it is looked at once
/// the build is done.
pub(crate) fn resolve_row_hint_placement(
    context: &mut AppContext,
    row: StableNodeId,
) -> Result<(), FrameworkError> {
    if context.defer_settings_row(row) {
        return Ok(());
    }
    resolve_row_hint_placement_now(context, row)
}

pub(crate) fn resolve_row_hint_placement_now(
    context: &mut AppContext,
    row: StableNodeId,
) -> Result<(), FrameworkError> {
    let inherited = inherited_hint_placement(context, row);
    let row = Entity::<SettingsRow>::from_stable_id(row);
    if context.read(row, |row| row.inherited_hint_placement != inherited)? {
        context.update_component(row, |row, _| row.inherited_hint_placement = inherited)?;
    }
    Ok(())
}

/// The rows under `node` that take their placement from it: every
/// [`SettingsRow`] below it, through any container, except under a group or
/// card that sets a placement of its own.
fn scope_rows(context: &AppContext, node: StableNodeId, rows: &mut Vec<StableNodeId>) {
    let Some(snapshot) = context.world().node(node) else {
        return;
    };
    for child in snapshot.children {
        if context
            .read(Entity::<SettingsRow>::from_stable_id(child), |_| ())
            .is_ok()
        {
            rows.push(child);
        } else if !matches!(hint_scope(context, child), Some(Some(_))) {
            scope_rows(context, child, rows);
        }
    }
}

/// A group or card whose placement changed hands it down to the rows under
/// it. `pushed` is the placement it handed down last; returns whether it
/// handed one down now.
fn hand_down_hint_placement(
    context: &mut AppContext,
    scope: StableNodeId,
    own: Option<HintPlacement>,
    pushed: Option<HintPlacement>,
) -> Result<bool, FrameworkError> {
    if own == pushed {
        return Ok(false);
    }
    let mut rows = Vec::new();
    scope_rows(context, scope, &mut rows);
    for row in rows {
        resolve_row_hint_placement(context, row)?;
    }
    Ok(true)
}

fn sync_settings_card(
    context: &mut AppContext,
    card: Entity<SettingsCard>,
) -> Result<(), FrameworkError> {
    let (own, pushed) = context.read(card, |card| {
        (card.hint_placement, card.pushed_hint_placement)
    })?;
    if hand_down_hint_placement(context, card.stable_id(), own, pushed)? {
        context.update_component(card, |card, _| card.pushed_hint_placement = own)?;
    }
    Ok(())
}

const COLLAPSIBLE_HEADER_GAP: f32 = nana_ui_core::space::MD;
const COLLAPSIBLE_BODY_GAP: f32 = nana_ui_core::space::XL;
#[cfg(test)]
const DISCLOSURE_ICON_SIZE: f32 = nana_ui_core::type_scale::LINE;

/// Controlled collapse. Header is keyboard-activatable; values stay host-owned.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsCollapsibleCard {
    pub summary: Option<StableNodeId>,
    pub details: Option<StableNodeId>,
    pub accessory: Option<StableNodeId>,
    pub(crate) header: Option<StableNodeId>,
    pub(crate) disclosure: Option<StableNodeId>,
    pub(crate) divider: Option<StableNodeId>,
    pub expanded: bool,
    pub disabled: bool,
    pub style: NodeStyle,
}

impl SettingsCollapsibleCard {
    pub fn new(expanded: bool) -> Self {
        Self {
            summary: None,
            details: None,
            accessory: None,
            header: None,
            disclosure: None,
            divider: None,
            expanded,
            disabled: false,
            style: NodeStyle::default(),
        }
    }

    pub fn summary(mut self, summary: StableNodeId) -> Self {
        self.summary = Some(summary);
        self
    }

    pub fn details(mut self, details: StableNodeId) -> Self {
        self.details = Some(details);
        self
    }

    pub fn accessory(mut self, accessory: StableNodeId) -> Self {
        self.accessory = Some(accessory);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn card(&self) -> Card {
        let mut card = Card::new().kind(CardKind::Surface);
        let mut style = card.style.clone();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Fill);
            layout.direction = Some(FlexDirection::Column);
            layout.gap = Some(LengthSpec::Px(COLLAPSIBLE_BODY_GAP));
            layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xl));
        }
        style.interaction = if self.disabled {
            InteractionStyle {
                disabled: SemanticPaint {
                    foreground: Some(SemanticColorRole::Faint),
                    ..SemanticPaint::default()
                },
                ..InteractionStyle::default()
            }
        } else {
            InteractionStyle {
                hovered: SemanticPaint {
                    background: Some(SemanticColorRole::Hover),
                    ..SemanticPaint::default()
                },
                pressed: SemanticPaint {
                    background: Some(SemanticColorRole::Active),
                    ..SemanticPaint::default()
                },
                focused: SemanticPaint::FOCUS_SURFACE,
                ..InteractionStyle::default()
            }
        };
        card.style = style;
        card
    }
}

impl ComponentView for SettingsCollapsibleCard {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::activate_settings_collapsible_card),
        slot_assembler: Some(AppContext::assemble_settings_collapsible_card),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Lays out and hides the summary and detail slots the application provides.
    const ALWAYS_REPROJECT: bool = true;

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-collapsible-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.card().project(id, world, mutations);
        project_common(
            id,
            world,
            mutations,
            &self.card().style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                selected: Some(self.expanded),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
        self.project_slots(world, mutations);
    }
}

fn project_slots_hidden(
    id: StableNodeId,
    hidden: bool,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    let Some(current) = world.node_style(id) else {
        return;
    };
    if current.layout.hidden == hidden {
        return;
    }
    let mut style = current.clone();
    Arc::make_mut(&mut style.layout).hidden = hidden;
    mutations.set_style(id, style);
}

fn project_summary_slot(id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
    let Some(current) = world.node_style(id) else {
        return;
    };
    let layout = current.layout.as_ref();
    let needs_width = layout.width.is_none();
    let needs_grow = layout.flex_grow.is_none();
    let needs_color = current.foreground != Some(SemanticColorRole::Text);
    if !needs_width && !needs_grow && !needs_color {
        return;
    }
    let mut style = current.clone();
    if needs_color {
        style.foreground = Some(SemanticColorRole::Text);
    }
    let layout = Arc::make_mut(&mut style.layout);
    if needs_width {
        layout.width = Some(LengthSpec::Fill);
    }
    if needs_grow {
        layout.flex_grow = Some(1.0);
    }
    mutations.set_style(id, style);
}

fn disclosure_icon_kind(expanded: bool) -> Icon {
    if expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    }
}

impl SettingsCollapsibleCard {
    fn project_slots(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        if let Some(header) = self.header {
            SettingsCollapsibleHeader.project(header, world, mutations);
        }
        if let Some(summary) = self.summary {
            project_summary_slot(summary, world, mutations);
        }
        if let Some(disclosure) = self.disclosure {
            SettingsDisclosure {
                expanded: self.expanded,
            }
            .project(disclosure, world, mutations);
        }
        if let Some(divider) = self.divider {
            SettingsCollapsibleDivider {
                hidden: !self.expanded,
            }
            .project(divider, world, mutations);
        }
        if let Some(details) = self.details {
            project_slots_hidden(details, !self.expanded, world, mutations);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SettingsCollapsibleHeader;

impl SettingsCollapsibleHeader {
    fn style() -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(LengthSpec::Px(COLLAPSIBLE_HEADER_GAP));
        layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Md));
        style
    }
}

impl ComponentView for SettingsCollapsibleHeader {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-collapsible-header".into(),
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

#[derive(Debug, Clone, PartialEq)]
struct SettingsCollapsibleDivider {
    hidden: bool,
}

impl SettingsCollapsibleDivider {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        style.background = Some(SemanticColorRole::BorderSoft);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(nana_ui_core::HAIRLINE));
        layout.min_height = Some(LengthSpec::Px(nana_ui_core::HAIRLINE));
        let extent = nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline);
        layout.theme.height = Some(extent.into());
        layout.theme.min_height = Some(extent.into());
        layout.hidden = self.hidden;
        style
    }
}

impl ComponentView for SettingsCollapsibleDivider {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-collapsible-divider".into(),
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

/// Visual chevron only. Activation stays on the card so accessory controls
/// remain independently interactive.
#[derive(Debug, Clone, PartialEq)]
struct SettingsDisclosure {
    expanded: bool,
}

impl ComponentView for SettingsDisclosure {
    fn wants_metrics_reproject() -> bool {
        true
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-disclosure".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::Icon {
            icon: disclosure_icon_kind(self.expanded),
            size: world.theme().typography().line,
            tooltip: None,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut style = NodeStyle::default();
        style.foreground = Some(SemanticColorRole::Muted);
        style.square = Some(nana_ui_core::SquareSize::Control(ControlSize::Small));
        style.control_height = Some(nana_ui_core::ControlHeight::Exact(ControlSize::Small));
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.aspect_ratio = Some(1.0);
            layout.flex_grow = Some(0.0);
            layout.flex_shrink = Some(0.0);
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
                selected: Some(self.expanded),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Host-owned appearance snapshot. Events stay [`AppearanceEvent`]; values stay outside NanaUI.
#[derive(Debug, Clone, PartialEq)]
pub struct AppearanceSection {
    pub theme_id: ThemeId,
    pub theme_options: Vec<ThemeChoice>,
    pub appearance: AppearanceSettings,
    pub material_status: Option<Arc<str>>,
    pub platform_hint: Option<Arc<str>>,
    /// Native materials the host is willing to offer besides 实色 / 透明.
    pub available_materials: Vec<WindowMaterialMode>,
    pub assembly: Option<AppearanceSectionAssembly>,
}

/// Retained children created by [`AppContext::assemble_appearance_section`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppearanceSectionAssembly {
    pub theme_row: Option<StableNodeId>,
    pub theme_control: Option<StableNodeId>,
    pub theme_dark: Option<StableNodeId>,
    pub theme_light: Option<StableNodeId>,
    pub theme_options: Vec<StableNodeId>,
    pub material_row: Option<StableNodeId>,
    pub material_control: Option<StableNodeId>,
    pub material_solid: Option<StableNodeId>,
    pub material_translucent: Option<StableNodeId>,
    pub material_vibrancy: Option<StableNodeId>,
    pub material_mica: Option<StableNodeId>,
    pub material_acrylic: Option<StableNodeId>,
    pub material_status_row: Option<StableNodeId>,
    pub material_status_value: Option<StableNodeId>,
    pub target_row: Option<StableNodeId>,
    pub target_control: Option<StableNodeId>,
    pub target_sidebar: Option<StableNodeId>,
    pub target_main: Option<StableNodeId>,
    pub titlebar_row: Option<StableNodeId>,
    pub titlebar_switch: Option<StableNodeId>,
    pub opacity_row: Option<StableNodeId>,
    pub opacity_range: Option<StableNodeId>,
    pub opacity_text: Option<StableNodeId>,
    pub workspace_row: Option<StableNodeId>,
    pub workspace_switch: Option<StableNodeId>,
    pub radius_row: Option<StableNodeId>,
    pub radius_range: Option<StableNodeId>,
    pub reset_row: Option<StableNodeId>,
    pub reset_button: Option<StableNodeId>,
}

impl AppearanceSection {
    pub fn new(theme: ThemeAppearance, appearance: AppearanceSettings) -> Self {
        let theme_id = match theme {
            ThemeAppearance::Light => ThemeId::new("nana.light"),
            ThemeAppearance::Dark => ThemeId::new("nana.dark"),
            ThemeAppearance::Custom => ThemeId::new("nana.light"),
        };
        Self {
            theme_id,
            theme_options: Vec::new(),
            appearance,
            material_status: None,
            platform_hint: None,
            available_materials: Vec::new(),
            assembly: None,
        }
    }

    pub fn from_theme_id(theme_id: ThemeId, appearance: AppearanceSettings) -> Self {
        Self {
            theme_id,
            theme_options: Vec::new(),
            appearance,
            material_status: None,
            platform_hint: None,
            available_materials: Vec::new(),
            assembly: None,
        }
    }

    pub fn with_theme_options(mut self, selected: ThemeId, options: Vec<ThemeChoice>) -> Self {
        self.theme_id = selected;
        self.theme_options = options;
        self
    }

    pub fn material_status(mut self, status: impl Into<Arc<str>>) -> Self {
        self.material_status = Some(status.into());
        self
    }

    pub fn platform_hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.platform_hint = Some(hint.into());
        self
    }

    pub fn available_materials(mut self, modes: impl Into<Vec<WindowMaterialMode>>) -> Self {
        self.available_materials = modes.into();
        self
    }
}

impl ComponentView for AppearanceSection {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_appearance_section),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "appearance-section".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        SettingsCard::new("").project(id, world, mutations);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutMetadata {
    pub product_title: Arc<str>,
    pub version: Arc<str>,
    pub description: Option<Arc<str>>,
}

impl AboutMetadata {
    pub fn new(product_title: impl Into<Arc<str>>, version: impl Into<Arc<str>>) -> Self {
        Self {
            product_title: product_title.into(),
            version: version.into(),
            description: None,
        }
    }

    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Injected metadata only. No application constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutSection {
    pub metadata: AboutMetadata,
    pub assembly: Option<AboutSectionAssembly>,
}

/// Retained children created by [`AppContext::assemble_about_section`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AboutSectionAssembly {
    pub name_row: Option<StableNodeId>,
    pub name_value: Option<StableNodeId>,
    pub version_row: Option<StableNodeId>,
    pub version_value: Option<StableNodeId>,
    pub description: Option<StableNodeId>,
}

impl AboutSection {
    pub fn new(metadata: AboutMetadata) -> Self {
        Self {
            metadata,
            assembly: None,
        }
    }
}

impl ComponentView for AboutSection {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_about_section),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "about-section".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        SettingsCard::new("").project(id, world, mutations);
        let accessibility = AccessibilityState {
            role: AccessibilityRole::Generic,
            label: Some(Arc::clone(&self.metadata.product_title)),
            value: Some(Arc::clone(&self.metadata.version)),
            description: self.metadata.description.clone(),
            ..AccessibilityState::default()
        };
        if world.accessibility(id) != Some(&accessibility) {
            mutations.set_accessibility(id, accessibility);
        }
    }
}

const SETTINGS_SIDEBAR_GAP: f32 = nana_ui_core::space::XL;
const SETTINGS_SIDEBAR_TAB_GAP: f32 = nana_ui_core::space::XXS;
const SETTINGS_SIDEBAR_ICON_SIZE: f32 = ControlSize::Small.icon_size();
const SETTINGS_PAGE_GAP: f32 = nana_ui_core::space::XXXL;
/// Tighter than the other three: the page sits under a title row that already
/// contributes its own leading.
const SETTINGS_PAGE_PADDING_TOP: f32 = nana_ui_core::space::PAGE_TIGHT;
const SETTINGS_PAGE_PADDING_RIGHT: f32 = nana_ui_core::space::PAGE;
const SETTINGS_PAGE_PADDING_BOTTOM: f32 = nana_ui_core::space::PAGE;
const SETTINGS_PAGE_PADDING_LEFT: f32 = nana_ui_core::space::PAGE;
const SETTINGS_PAGE_TITLE_SIZE: f32 = nana_ui_core::type_scale::TITLE;
const SETTINGS_PAGE_TITLE_WEIGHT: u16 = nana_ui_core::type_scale::SEMIBOLD;

/// Host-owned navigation snapshot. Activate emits [`SettingsBack`] / [`SettingsTabSelected`].
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsSidebar {
    pub model: SettingsModel,
    pub state: SettingsState,
    pub assembly: Option<SettingsSidebarAssembly>,
}

/// Retained children created by [`AppContext::assemble_settings_sidebar`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SettingsSidebarAssembly {
    pub back_row: Option<StableNodeId>,
    pub back_icon: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub tab_rows: HashMap<SettingsTabId, StableNodeId>,
    pub tab_leadings: HashMap<SettingsTabId, StableNodeId>,
}

impl SettingsSidebar {
    pub fn new(model: SettingsModel, state: SettingsState) -> Self {
        Self {
            model,
            state,
            assembly: None,
        }
    }

    pub fn row_for_tab(&self, id: &SettingsTabId) -> Option<StableNodeId> {
        self.assembly.as_ref()?.tab_rows.get(id).copied()
    }
}

/// SettingsSidebar is always hosted inside the shell's wrapping SidebarFrame,
/// which supplies the standard frame padding; this frame must stay padding-free.
fn settings_sidebar_frame_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.padding_top = Some(LengthSpec::Px(0.0));
    layout.padding_right = Some(LengthSpec::Px(0.0));
    layout.padding_bottom = Some(LengthSpec::Px(0.0));
    layout.padding_left = Some(LengthSpec::Px(0.0));
    style
}

impl ComponentView for SettingsSidebar {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_settings_sidebar),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-sidebar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut frame = SidebarFrame::new()
            .gap(SETTINGS_SIDEBAR_GAP)
            .style(settings_sidebar_frame_style());
        if let Some(assembly) = &self.assembly {
            if let Some(top) = assembly.back_row {
                frame = frame.top(top);
            }
            if let Some(body) = assembly.body {
                frame = frame.body(body);
            }
        }
        frame.project(id, world, mutations);
    }
}

/// Host-owned page snapshot. Content stays application-owned.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsPage {
    pub model: SettingsModel,
    pub state: SettingsState,
    pub content: Option<StableNodeId>,
    /// Insets of the internal scrolling body; None uses the standard page frame.
    pub content_padding: Option<PaddingSpec>,
    pub content_gap: Option<f32>,
    /// Font size of the page title; `None` is the settings title size.
    pub title_size: Option<f32>,
    /// Font weight of the page title; `None` is semibold.
    pub title_weight: Option<u16>,
    pub assembly: Option<SettingsPageAssembly>,
}

/// Retained chrome created by [`AppContext::assemble_settings_page`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsPageAssembly {
    pub scroll: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub title: Option<StableNodeId>,
}

impl SettingsPage {
    pub fn new(model: SettingsModel, state: SettingsState) -> Self {
        Self {
            model,
            state,
            content: None,
            content_padding: None,
            content_gap: None,
            title_size: None,
            title_weight: None,
            assembly: None,
        }
    }

    /// The page title's font size, kept through every assembly (a tab
    /// switch re-assembles the page).
    pub fn title_size(mut self, size: f32) -> Self {
        self.title_size = Some(size.max(1.0));
        self
    }

    /// The page title's font weight, kept through every assembly.
    pub fn title_weight(mut self, weight: u16) -> Self {
        self.title_weight = Some(weight.clamp(1, 1000));
        self
    }

    pub fn content_padding(mut self, padding: PaddingSpec) -> Self {
        self.content_padding = Some(PaddingSpec {
            top: padding.top.max(0.0),
            right: padding.right.max(0.0),
            bottom: padding.bottom.max(0.0),
            left: padding.left.max(0.0),
        });
        self
    }

    pub fn content_gap(mut self, gap: f32) -> Self {
        self.content_gap = Some(gap.max(0.0));
        self
    }

    pub fn content(mut self, content: StableNodeId) -> Self {
        self.content = Some(content);
        self
    }
}

impl ComponentView for SettingsPage {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_settings_page),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-page".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &settings_page_style(),
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

/// Activate on the back row. Host owns navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsBack;

/// Activate on a tab row. Host applies [`SettingsState::select`] then re-assembles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsTabSelected {
    pub tab: SettingsTabId,
}

#[derive(Debug, Clone, PartialEq)]
struct SettingsSidebarLeading {
    icon: Option<Icon>,
    selected: bool,
}

impl SettingsSidebarLeading {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        style.foreground = Some(if self.selected {
            SemanticColorRole::Text
        } else {
            SemanticColorRole::Muted
        });
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE));
        layout.height = Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE));
        layout.min_width = Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE));
        layout.min_height = Some(LengthSpec::Px(SETTINGS_SIDEBAR_ICON_SIZE));
        let extent = nana_ui_core::ThemeToken::Type(nana_ui_core::TypeRole::Meta);
        layout.theme.width = Some(extent.into());
        layout.theme.height = Some(extent.into());
        layout.theme.min_width = Some(extent.into());
        layout.theme.min_height = Some(extent.into());
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        style
    }
}

impl ComponentView for SettingsSidebarLeading {
    fn wants_metrics_reproject() -> bool {
        true
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-sidebar-leading".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        let visual = self.icon.map(|icon| StandardVisual::Icon {
            icon,
            size: world.theme().typography().meta,
            tooltip: None,
        });
        if world.standard_visual(id) != visual {
            mutations.set_standard_visual(id, visual);
        }
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

#[derive(Debug, Clone, PartialEq, Default)]
struct SettingsPageBody {
    padding: Option<PaddingSpec>,
    gap: Option<f32>,
}

impl SettingsPageBody {
    fn style(&self) -> NodeStyle {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        layout.gap = Some(LengthSpec::Px(self.gap.unwrap_or(SETTINGS_PAGE_GAP)));
        if self.gap.is_none() {
            layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxxl));
        }
        if self.padding.is_none() {
            layout.theme.padding_top = Some(crate::theme_intent::space(
                nana_ui_core::SpacingStep::PageTight,
            ));
            layout.theme.padding_right =
                Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Page));
            layout.theme.padding_bottom =
                Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Page));
            layout.theme.padding_left =
                Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Page));
        }
        let padding = self.padding.unwrap_or(PaddingSpec {
            top: SETTINGS_PAGE_PADDING_TOP,
            right: SETTINGS_PAGE_PADDING_RIGHT,
            bottom: SETTINGS_PAGE_PADDING_BOTTOM,
            left: SETTINGS_PAGE_PADDING_LEFT,
        });
        layout.padding_top = Some(LengthSpec::Px(padding.top));
        layout.padding_right = Some(LengthSpec::Px(padding.right));
        layout.padding_bottom = Some(LengthSpec::Px(padding.bottom));
        layout.padding_left = Some(LengthSpec::Px(padding.left));
        style
    }
}

impl ComponentView for SettingsPageBody {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "settings-page-body".into(),
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

fn settings_page_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::Background);
    style.foreground = Some(SemanticColorRole::Text);
    let layout = Arc::make_mut(&mut style.layout);
    layout.direction = Some(FlexDirection::Column);
    layout.align_items = AlignSpec::Stretch;
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.overflow_x = OverflowSpec::Hidden;
    layout.overflow_y = OverflowSpec::Hidden;
    style
}

fn settings_page_scroll() -> ScrollView {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.min_height = Some(LengthSpec::Px(0.0));
    ScrollView::new(ScrollAxes::Vertical).style(style)
}

fn settings_sidebar_body() -> ScrollView {
    let mut scroll = SidebarFrame::vertical_body_scroll();
    let layout = Arc::make_mut(&mut scroll.style.layout);
    layout.direction = Some(FlexDirection::Column);
    layout.gap = Some(LengthSpec::Px(SETTINGS_SIDEBAR_TAB_GAP));
    layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
    scroll
}

fn page_title_text(label: &str, size: Option<f32>, weight: Option<u16>) -> Text {
    let mut text = styled_text(
        label,
        SemanticColorRole::Text,
        size.unwrap_or(SETTINGS_PAGE_TITLE_SIZE),
        weight.unwrap_or(SETTINGS_PAGE_TITLE_WEIGHT),
    );
    if let Some(size) = size {
        text = text.font_size(size);
    }
    if let Some(weight) = weight {
        text = text.font_weight(weight);
    }
    text
}

fn ensure_settings_leading(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    icon: Option<Icon>,
    selected: bool,
) -> Result<Entity<SettingsSidebarLeading>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::<SettingsSidebarLeading>::from_stable_id(id);
        context.update_component(entity, |leading, _| {
            leading.icon = icon;
            leading.selected = selected;
        })?;
        Ok(entity)
    } else {
        let entity = context
            .create_detached_component(document, SettingsSidebarLeading { icon, selected })?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

fn ensure_settings_nav_row(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    label: &str,
    state: SidebarRowState,
    leading: StableNodeId,
) -> Result<(Entity<SidebarRow>, bool), FrameworkError> {
    let created = slot.is_none();
    let row = if let Some(id) = *slot {
        let entity = Entity::<SidebarRow>::from_stable_id(id);
        context.update_component(entity, |row, _| {
            row.label = Arc::from(label);
            row.state = state;
            row.slots.leading = Some(leading);
        })?;
        entity
    } else {
        let entity = context.create_detached_component(
            document,
            SidebarRow::new(label).state(state).slots(ListItemSlots {
                leading: Some(leading),
                content: None,
                trailing: None,
            }),
        )?;
        *slot = Some(entity.stable_id());
        entity
    };
    reconcile_children(context, row, &[leading])?;
    Ok((row, created))
}

/// Apply a host-owned appearance event. Theme is not stored on [`AppearanceSettings`].
pub fn apply_appearance_event(
    theme_id: &mut nana_ui_core::ThemeId,
    appearance: &mut AppearanceSettings,
    event: AppearanceEvent,
) -> bool {
    match event {
        AppearanceEvent::Theme(next) => {
            if *theme_id == next {
                return false;
            }
            *theme_id = next;
            true
        }
        AppearanceEvent::StandardRadius(radius) => {
            appearance.set_standard_radius(f32::from(radius))
        }
        AppearanceEvent::WorkspaceCorners(enabled) => {
            appearance.set_workspace_corners_enabled(enabled)
        }
        AppearanceEvent::WindowMaterial(mode) => appearance.set_window_material(mode),
        AppearanceEvent::BackdropTarget(target) => appearance.set_backdrop_target(target),
        AppearanceEvent::BackdropOpacity(opacity) => appearance.set_backdrop_opacity(opacity),
        AppearanceEvent::TitlebarFollowsSidebar(enabled) => {
            appearance.set_titlebar_follows_sidebar(enabled)
        }
        AppearanceEvent::Reset => {
            let reset = nana_ui_core::ThemeId::new("nana.light");
            let theme_changed = *theme_id != reset;
            *theme_id = reset;
            appearance.reset() || theme_changed
        }
    }
}

fn appearance_mode(appearance: &AppearanceSettings) -> (bool, bool) {
    let solid_mode = !appearance.window_material().wants_transparent_surface();
    let titlebar_follow_disabled =
        solid_mode || !matches!(appearance.backdrop_target(), BackdropTarget::Sidebar);
    (solid_mode, titlebar_follow_disabled)
}

fn opacity_percent(appearance: &AppearanceSettings) -> f64 {
    f64::from((appearance.backdrop_opacity() * 100.0).round())
}

fn document_of(
    context: &AppContext,
    id: StableNodeId,
) -> Result<crate::DocumentId, FrameworkError> {
    context
        .world()
        .node(id)
        .map(|node| node.document)
        .ok_or(FrameworkError::MissingView(id))
}

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

fn reconcile_children<C: ComponentView>(
    context: &mut AppContext,
    parent: Entity<C>,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    context.reconcile_children(parent.stable_id(), ordered)
}

fn styled_text(value: impl Into<String>, color: SemanticColorRole, size: f32, weight: u16) -> Text {
    let mut style = NodeStyle::default();
    style.foreground = Some(color);
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(size);
    layout.font_weight = Some(weight);
    let role = match size {
        n if n <= 11.0 => nana_ui_core::TypeRole::Hint,
        n if n <= 12.0 => nana_ui_core::TypeRole::Meta,
        n if n <= 13.0 => nana_ui_core::TypeRole::Body,
        n if n <= 14.0 => nana_ui_core::TypeRole::Section,
        n if n <= 16.0 => nana_ui_core::TypeRole::Heading,
        n if n <= 18.0 => nana_ui_core::TypeRole::Title,
        _ => nana_ui_core::TypeRole::Display,
    };
    crate::theme_intent::font(layout, role);
    crate::theme_intent::weight(
        layout,
        match weight {
            0..=400 => nana_ui_core::TextWeight::Regular,
            401..=500 => nana_ui_core::TextWeight::Medium,
            501..=600 => nana_ui_core::TextWeight::Semibold,
            _ => nana_ui_core::TextWeight::Bold,
        },
    );
    Text::new(value).style(style)
}

fn row_label_text(label: &str) -> Text {
    Text::new(label).style(label_slot_style(label.is_empty(), false, false))
}

fn row_hint_text(hint: &str, hidden: bool) -> Text {
    Text::new(hint).style(hint_slot_style(hidden))
}

fn sync_text(
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

fn mount_settings_row(
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
        if let Ok((existing_label, existing_hint, existing_copy)) = context
            .read(Entity::<SettingsRow>::from_stable_id(row_id), |row| {
                (row.label_slot, row.hint_slot, row.copy_slot)
            })
        {
            label_slot = existing_label;
            hint_slot = existing_hint;
            copy_slot = existing_copy;
        }
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
        row.copy_slot = copy_id;
    };
    let row = if let Some(id) = *slot {
        let entity = Entity::<SettingsRow>::from_stable_id(id);
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
        row.copy_slot = copy_slot;
        let entity = context.create_detached_component(document, row)?;
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
        .read(row, |row| row.flash_slot)?
        .filter(|id| context.world().contains(*id));
    let children: Vec<_> = flash
        .into_iter()
        .chain([copy.stable_id(), control])
        .collect();
    reconcile_children(context, row, &children)?;
    label_row_control(context, label_text.stable_id(), Some(control), &previous)?;
    Ok(row)
}

fn ensure_segmented_pair(
    context: &mut AppContext,
    document: crate::DocumentId,
    control_slot: &mut Option<StableNodeId>,
    first_slot: &mut Option<StableNodeId>,
    second_slot: &mut Option<StableNodeId>,
    first: SegmentedOption,
    second: SegmentedOption,
    selected_first: bool,
    first_disabled: bool,
    second_disabled: bool,
) -> Result<
    (
        Entity<SegmentedControl>,
        Entity<SegmentedOption>,
        Entity<SegmentedOption>,
    ),
    FrameworkError,
> {
    let created = control_slot.is_none();
    let control = if let Some(id) = *control_slot {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, SegmentedControl::new())?;
        *control_slot = Some(entity.stable_id());
        entity
    };
    let first_entity = if let Some(id) = *first_slot {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, first)?;
        *first_slot = Some(entity.stable_id());
        entity
    };
    let second_entity = if let Some(id) = *second_slot {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, second)?;
        *second_slot = Some(entity.stable_id());
        entity
    };
    if created {
        context.set_segmented_options(
            control,
            vec![first_entity, second_entity],
            Some(if selected_first {
                first_entity
            } else {
                second_entity
            }),
        )?;
    } else {
        context.set_segmented_selection(
            control,
            Some(if selected_first {
                first_entity
            } else {
                second_entity
            }),
        )?;
    }
    context.set_segmented_option_disabled(control, first_entity, first_disabled)?;
    context.set_segmented_option_disabled(control, second_entity, second_disabled)?;
    Ok((control, first_entity, second_entity))
}

fn offered_window_materials(
    available: &[WindowMaterialMode],
    current: WindowMaterialMode,
) -> Vec<WindowMaterialMode> {
    let mut modes = vec![WindowMaterialMode::Solid, WindowMaterialMode::Translucent];
    for mode in available
        .iter()
        .copied()
        .chain(std::iter::once(current))
        .filter(|mode| mode.wants_native())
    {
        if !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    modes
}

fn material_option_slot(
    assembly: &mut AppearanceSectionAssembly,
    mode: WindowMaterialMode,
) -> &mut Option<StableNodeId> {
    match mode {
        WindowMaterialMode::Solid => &mut assembly.material_solid,
        WindowMaterialMode::Translucent => &mut assembly.material_translucent,
        WindowMaterialMode::Vibrancy => &mut assembly.material_vibrancy,
        WindowMaterialMode::Mica => &mut assembly.material_mica,
        WindowMaterialMode::Acrylic => &mut assembly.material_acrylic,
    }
}

fn ensure_material_options(
    context: &mut AppContext,
    document: crate::DocumentId,
    assembly: &mut AppearanceSectionAssembly,
    offered: &[WindowMaterialMode],
    selected: WindowMaterialMode,
) -> Result<Entity<SegmentedControl>, FrameworkError> {
    let control = if let Some(id) = assembly.material_control {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, SegmentedControl::new())?;
        assembly.material_control = Some(entity.stable_id());
        entity
    };
    let mut options = Vec::with_capacity(offered.len());
    for mode in offered {
        let slot = material_option_slot(assembly, *mode);
        let entity = if let Some(id) = *slot {
            Entity::from_stable_id(id)
        } else {
            let entity = {
                let label = mode.label(context.world().framework_strings()).to_owned();
                context.create_detached_component(document, SegmentedOption::new(label))?
            };
            *slot = Some(entity.stable_id());
            entity
        };
        options.push(entity);
    }
    let selected_id = *material_option_slot(assembly, selected);
    let selected = options
        .iter()
        .copied()
        .find(|option| selected_id == Some(option.stable_id()))
        .or_else(|| options.first().copied());
    context.set_segmented_options(control, options, selected)?;
    Ok(control)
}

fn ensure_theme_options(
    context: &mut AppContext,
    document: crate::DocumentId,
    assembly: &mut AppearanceSectionAssembly,
    offered: &[ThemeChoice],
    selected: Option<&str>,
) -> Result<
    (
        Vec<Entity<SegmentedOption>>,
        Option<Entity<SegmentedOption>>,
    ),
    FrameworkError,
> {
    let mut options = Vec::with_capacity(offered.len());
    let previous = std::mem::take(&mut assembly.theme_options);
    for choice in offered {
        let entity = if let Some(id) = assembly
            .theme_options
            .get(options.len())
            .or_else(|| previous.get(options.len()))
            .copied()
        {
            Entity::from_stable_id(id)
        } else {
            // The built-in themes are named in the framework's words.
            let strings = context.world().framework_strings();
            let label = match choice.id.as_str() {
                "nana.light" => Arc::clone(&strings.theme_light),
                "nana.dark" => Arc::clone(&strings.theme_dark),
                _ => Arc::clone(&choice.label),
            };
            context.create_detached_component(document, SegmentedOption::new(label))?
        };
        assembly.theme_options.push(entity.stable_id());
        options.push(entity);
    }
    let selected = selected.and_then(|id| {
        offered
            .iter()
            .position(|choice| choice.id.as_str() == id)
            .and_then(|index| options.get(index).copied())
    });
    let selected = selected.or_else(|| options.first().copied());
    Ok((options, selected))
}

fn ensure_switch(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    label: &str,
    checked: bool,
    disabled: bool,
) -> Result<Entity<Switch>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::<Switch>::from_stable_id(id);
        context.update_component(entity, |switch, _| {
            switch.checked = checked;
            switch.disabled = disabled;
        })?;
        Ok(entity)
    } else {
        let entity = context
            .create_detached_component(document, Switch::new(label, checked).disabled(disabled))?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

fn ensure_range(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    value: f64,
    minimum: f64,
    maximum: f64,
    unit: &str,
    disabled: bool,
) -> Result<Entity<RangeField>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::<RangeField>::from_stable_id(id);
        context.update_component(entity, |range, _| {
            range.value = range.quantize(value);
            range.disabled = disabled;
        })?;
        Ok(entity)
    } else {
        let mut field = RangeField::new(value, minimum, maximum, 1.0).unit(unit);
        field.disabled = disabled;
        let entity = context.create_detached_component(document, field)?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

impl AppContext {
    pub(crate) fn refresh_settings_row_layout(
        &mut self,
        id: StableNodeId,
    ) -> Result<(), FrameworkError> {
        let row = Entity::<SettingsRow>::from_stable_id(id);
        let needs_projection = self
            .read(row, |row| {
                let direction = if row.effective_stacked(id, self.world()) {
                    FlexDirection::Column
                } else {
                    FlexDirection::Row
                };
                self.world()
                    .node_style(row.line_node(id))
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
        let id = row.stable_id();
        if !ROW_ASSEMBLING.with_borrow_mut(|rows| rows.insert(id)) {
            return Ok(false);
        }
        let result = self.assemble_settings_row_parts(row);
        ROW_ASSEMBLING.with_borrow_mut(|rows| rows.remove(&id));
        result
    }

    fn assemble_settings_row_parts(
        &mut self,
        row: Entity<SettingsRow>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, row.stable_id())?;
        resolve_row_hint_placement(self, row.stable_id())?;
        let snapshot = self.read(row, |row| {
            (
                row.label.clone(),
                row.hint.clone(),
                row.hint_is_path,
                row.control,
                row.label_slot,
                row.hint_slot,
                row.copy_slot,
            )
        })?;
        let (label, hint, hint_is_path, control, label_slot, mut hint_slot, copy_slot) = snapshot;
        let (mark, accessory, line_slot, flash_slot) = self.read(row, |row| {
            (
                row.label_mark,
                row.label_accessory,
                row.label_line_slot,
                row.flash_slot,
            )
        })?;
        let (error, error_shown, wants_notice) = self.read(row, |row| {
            (
                row.error.is_some(),
                row.error_shown,
                row.shown_notice().is_some(),
            )
        })?;
        let (wants_help, help_open, help_slot, help_text_slot, notice_slot, main_slot) = self
            .read(row, |row| {
                (
                    row.shows_help(),
                    row.help_open,
                    row.help_slot,
                    row.help_text_slot,
                    row.notice_slot,
                    row.main_slot,
                )
            })?;
        let (column, trailing, control_slot, value_slot, trailing_slot) =
            self.read(row, |row| {
                (
                    row.label_column,
                    row.trailing,
                    row.control_slot,
                    row.value_slot,
                    row.trailing_slot,
                )
            })?;
        let live = |context: &Self, slot: Option<StableNodeId>| {
            slot.filter(|id| context.world().contains(*id))
        };
        let mut created = false;
        let label_slot = match live(self, label_slot) {
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
        let hint_slot = match live(self, hint_slot) {
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
        let copy_slot = match live(self, copy_slot) {
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
        if created {
            self.update_component(row, |row, _| {
                row.label_slot = Some(label_slot);
                row.hint_slot = Some(hint_slot);
                row.copy_slot = Some(copy_slot);
            })?;
        }
        if hint_is_path {
            self.update_component(
                Entity::<crate::PathText>::from_stable_id(hint_slot),
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
        let notice = match (live(self, notice_slot), wants_notice) {
            (Some(id), _) => Some(id),
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
                self.update_component(row, |row, _| {
                    row.notice_slot = Some(notice);
                    row.notice_glyph_slot = Some(glyph);
                    row.notice_text_slot = Some(text);
                })?;
                Some(notice)
            }
            (None, false) => None,
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
        if help != live_help || help_text != help_text_slot || main != main_slot {
            self.update_component(row, |row, _| {
                row.help_slot = help;
                row.help_text_slot = help_text;
                row.main_slot = main;
            })?;
            changed = true;
        }
        let copy = Entity::<SettingsRowCopy>::from_stable_id(copy_slot);
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
            let on_line: Vec<_> = std::iter::once(label_slot)
                .chain(mark)
                .chain(help)
                .chain(accessory)
                .collect();
            changed |= self.reconcile_children(line, &on_line)?;
        }
        let copy_children = [line.unwrap_or(label_slot), hint_slot];
        changed |= reconcile_children(self, copy, &copy_children)?;
        if line_changed {
            // The label's width rule follows where it sits; the row's
            // projection writes it too, from the slot written here.
            self.update_component(row, |row, _| row.label_line_slot = line)?;
            let text = Entity::<Text>::from_stable_id(label_slot);
            if self.read(text, |_| ()).is_ok() {
                self.update_component(text, |text, _| {
                    text.style =
                        label_slot_style(label.is_empty(), line.is_some(), column.is_some());
                })?;
            }
            if let Some(old) = live_line.filter(|old| Some(*old) != line) {
                // The accessory left with its slot: hand back whatever is
                // still on the line before the line goes.
                self.reconcile_children(old, &[])?;
                self.remove_view(Entity::<SettingsRowLabelLine>::from_stable_id(old))?;
            }
        }
        if let Some(old) = live_help.filter(|old| Some(*old) != help) {
            // The hint came out from behind the button: the button goes,
            // its line handed back above.
            self.remove_view(Entity::<crate::IconButton>::from_stable_id(old))?;
        }
        let control = live(self, control);
        if error_shown != Some(error) {
            self.update_component(row, |row, _| row.error_shown = Some(error))?;
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
        if value_box != value_slot || trailing_box != trailing_slot {
            self.update_component(row, |row, _| {
                row.value_slot = value_box;
                row.trailing_slot = trailing_box;
            })?;
            // hand back what the old boxes still hold before they go
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
        let box_changed = control_box != control_slot;
        let line_parts: Vec<_> = std::iter::once(copy_slot)
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
        if box_changed {
            self.update_component(row, |row, _| row.control_slot = control_box)?;
            if let Some(old) = live_box.filter(|old| Some(*old) != control_box) {
                // the control is back on the row: hand back what the box
                // still holds before it goes
                self.reconcile_children(old, &[])?;
                self.remove_view(Entity::<SettingsRowControl>::from_stable_id(old))?;
            }
        }
        sync_row_help(self, row)?;
        label_row_control(self, label_slot, control, &previous)?;
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
        let flash = self.read(row, |row| row.flash_slot)?;
        let flash = match flash.filter(|id| self.world().contains(*id)) {
            Some(id) => id,
            None => {
                let id = self
                    .create_detached_component(document, SettingsRowFlash)?
                    .stable_id();
                self.update_component(row, |row, _| row.flash_slot = Some(id))?;
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
            let row = Entity::<SettingsRow>::from_stable_id(parent);
            if self
                .read(row, |row| row.flash_slot == Some(flash))
                .unwrap_or(false)
            {
                self.update_component(row, |row, _| row.flash_slot = None)?;
            }
        }
        self.remove_view(entity)?;
        Ok(())
    }

    /// Mount or refresh the appearance row/control contract from the
    /// host snapshot stored on [`AppearanceSection`].
    ///
    /// Child activations re-emit [`AppearanceEvent`] from the section. The
    /// host binds `on(section, AppearanceEvent)` and applies the event with
    /// [`apply_appearance_event`] after the interaction returns, then writes
    /// the snapshot back and calls this method again.
    pub fn assemble_appearance_section(
        &mut self,
        section: Entity<AppearanceSection>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, section.stable_id())?;
        let strings = self.world().framework_strings().clone();
        let snapshot = self.read(section, |section| {
            (
                section.theme_id.clone(),
                section.theme_options.clone(),
                section.appearance,
                section.material_status.clone(),
                section.platform_hint.clone(),
                section.available_materials.clone(),
                section.assembly.clone().unwrap_or_default(),
            )
        })?;
        let (
            theme_id,
            theme_options,
            appearance,
            material_status,
            platform_hint,
            available_materials,
            mut assembly,
        ) = snapshot;
        let (solid_mode, titlebar_follow_disabled) = appearance_mode(&appearance);
        let created_theme = assembly.theme_control.is_none();
        let created_material = assembly.material_control.is_none();
        let created_target = assembly.target_control.is_none();
        let created_titlebar = assembly.titlebar_switch.is_none();
        let created_workspace = assembly.workspace_switch.is_none();
        let created_radius = assembly.radius_range.is_none();
        let created_reset = assembly.reset_button.is_none();
        let mut created_opacity_range = false;
        let (theme_control, theme_dark, theme_light): (
            Entity<SegmentedControl>,
            Option<Entity<SegmentedOption>>,
            Option<Entity<SegmentedOption>>,
        ) = if theme_options.is_empty() {
            let (control, dark, light) = ensure_segmented_pair(
                self,
                document,
                &mut assembly.theme_control,
                &mut assembly.theme_dark,
                &mut assembly.theme_light,
                SegmentedOption::new(strings.settings_theme_dark.as_ref()).icon(Icon::Moon),
                SegmentedOption::new(strings.settings_theme_light.as_ref()).icon(Icon::Appearance),
                theme_id.as_str() == "nana.dark",
                false,
                false,
            )?;
            (control, Some(dark), Some(light))
        } else {
            let control = if let Some(id) = assembly.theme_control {
                Entity::from_stable_id(id)
            } else {
                let entity = self.create_detached_component(document, SegmentedControl::new())?;
                assembly.theme_control = Some(entity.stable_id());
                entity
            };
            let (options, selected) = ensure_theme_options(
                self,
                document,
                &mut assembly,
                &theme_options,
                Some(theme_id.as_str()),
            )?;
            self.set_segmented_options(control, options, selected)?;
            (control, None, None)
        };
        assembly.theme_control = Some(theme_control.stable_id());
        assembly.theme_dark = theme_dark.map(|entity| entity.stable_id());
        assembly.theme_light = theme_light.map(|entity| entity.stable_id());
        let theme_row = mount_settings_row(
            self,
            document,
            &mut assembly.theme_row,
            strings.settings_theme.as_ref(),
            Some(strings.settings_theme_hint.as_ref()),
            true,
            true,
            false,
            theme_control.stable_id(),
        )?;

        let offered_materials =
            offered_window_materials(&available_materials, appearance.window_material());
        let material_control = ensure_material_options(
            self,
            document,
            &mut assembly,
            &offered_materials,
            appearance.window_material(),
        )?;
        let material_hint = platform_hint
            .as_deref()
            .unwrap_or(strings.settings_material_hint.as_ref());
        let material_row = mount_settings_row(
            self,
            document,
            &mut assembly.material_row,
            strings.settings_material.as_ref(),
            Some(material_hint),
            true,
            false,
            false,
            material_control.stable_id(),
        )?;

        let status_row = if let Some(status) = material_status.as_deref() {
            let status_value = sync_text(
                self,
                document,
                &mut assembly.material_status_value,
                styled_text(
                    status,
                    SemanticColorRole::Muted,
                    nana_ui_core::type_scale::META,
                    nana_ui_core::type_scale::REGULAR,
                ),
            )?;
            Some(mount_settings_row(
                self,
                document,
                &mut assembly.material_status_row,
                strings.settings_material_state.as_ref(),
                Some(strings.settings_material_state_hint.as_ref()),
                true,
                false,
                false,
                status_value.stable_id(),
            )?)
        } else {
            None
        };

        let (target_control, target_sidebar, target_main) = ensure_segmented_pair(
            self,
            document,
            &mut assembly.target_control,
            &mut assembly.target_sidebar,
            &mut assembly.target_main,
            SegmentedOption::new(strings.material_region_sidebar.as_ref()).disabled(solid_mode),
            SegmentedOption::new(strings.material_region_main.as_ref()).disabled(solid_mode),
            matches!(appearance.backdrop_target(), BackdropTarget::Sidebar),
            solid_mode,
            solid_mode,
        )?;
        assembly.target_control = Some(target_control.stable_id());
        assembly.target_sidebar = Some(target_sidebar.stable_id());
        assembly.target_main = Some(target_main.stable_id());
        let target_hint = if solid_mode {
            strings.settings_region_solid_hint.as_ref()
        } else {
            strings.settings_region_hint.as_ref()
        };
        let target_row = mount_settings_row(
            self,
            document,
            &mut assembly.target_row,
            strings.settings_region.as_ref(),
            Some(target_hint),
            true,
            false,
            false,
            target_control.stable_id(),
        )?;

        let titlebar_switch = ensure_switch(
            self,
            document,
            &mut assembly.titlebar_switch,
            "",
            appearance.titlebar_follows_sidebar(),
            titlebar_follow_disabled,
        )?;
        let titlebar_hint = if titlebar_follow_disabled {
            strings.settings_title_bar_follows_main_hint.as_ref()
        } else {
            strings.settings_title_bar_follows_hint.as_ref()
        };
        let titlebar_row = mount_settings_row(
            self,
            document,
            &mut assembly.titlebar_row,
            strings.settings_title_bar_follows.as_ref(),
            Some(titlebar_hint),
            true,
            false,
            false,
            titlebar_switch.stable_id(),
        )?;

        let opacity = opacity_percent(&appearance);
        // Solid mode keeps the slider in place, disabled, so the row has the
        // same control as every other row and the kept value stays visible.
        created_opacity_range |= assembly.opacity_range.is_none();
        let opacity_control = ensure_range(
            self,
            document,
            &mut assembly.opacity_range,
            opacity,
            f64::from(AppearanceSettings::MIN_BACKDROP_OPACITY) * 100.0,
            f64::from(AppearanceSettings::MAX_BACKDROP_OPACITY) * 100.0,
            "%",
            solid_mode,
        )?
        .stable_id();
        assembly.opacity_text = None;
        let opacity_hint = if solid_mode {
            strings.settings_opacity_solid_hint.as_ref()
        } else {
            strings.settings_opacity_hint.as_ref()
        };
        let opacity_row = mount_settings_row(
            self,
            document,
            &mut assembly.opacity_row,
            strings.settings_opacity.as_ref(),
            Some(opacity_hint),
            true,
            false,
            false,
            opacity_control,
        )?;

        let workspace_switch = ensure_switch(
            self,
            document,
            &mut assembly.workspace_switch,
            "",
            appearance.workspace_corners_enabled(),
            false,
        )?;
        // The row names the setting; a second label inside the switch would
        // sit mid-row and push the track off the column of other controls.
        let workspace_row = mount_settings_row(
            self,
            document,
            &mut assembly.workspace_row,
            strings.settings_main_radius.as_ref(),
            None,
            true,
            false,
            false,
            workspace_switch.stable_id(),
        )?;

        let radius_range = ensure_range(
            self,
            document,
            &mut assembly.radius_range,
            f64::from(appearance.standard_radius()),
            f64::from(AppearanceSettings::MIN_STANDARD_RADIUS),
            f64::from(AppearanceSettings::MAX_STANDARD_RADIUS),
            " px",
            false,
        )?;
        let radius_row = mount_settings_row(
            self,
            document,
            &mut assembly.radius_row,
            strings.settings_radius.as_ref(),
            None,
            true,
            false,
            false,
            radius_range.stable_id(),
        )?;

        let reset_button = if let Some(id) = assembly.reset_button {
            Entity::<Button>::from_stable_id(id)
        } else {
            let entity = self.create_detached_component(
                document,
                Button::new(strings.settings_restore.as_ref())
                    .kind(ButtonKind::Subtle)
                    .size(ControlSize::Small),
            )?;
            assembly.reset_button = Some(entity.stable_id());
            entity
        };
        let reset_row = mount_settings_row(
            self,
            document,
            &mut assembly.reset_row,
            strings.settings_defaults.as_ref(),
            Some(strings.settings_defaults_hint.as_ref()),
            false,
            false,
            true,
            reset_button.stable_id(),
        )?;

        if created_theme && !theme_options.is_empty() {
            let option_ids = assembly.theme_options.clone();
            let theme_ids: Vec<_> = theme_options
                .iter()
                .map(|choice| choice.id.clone())
                .collect();
            self.observe(
                theme_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let Some(index) = option_ids.iter().position(|id| *id == event.option) else {
                        return;
                    };
                    if let Some(theme_id) = theme_ids.get(index) {
                        cx.emit(AppearanceEvent::Theme(theme_id.clone()));
                    }
                },
            )?;
        } else if created_theme {
            let (Some(dark), Some(light)) = (theme_dark, theme_light) else {
                return Err(FrameworkError::InvalidComponentValue(
                    theme_control.stable_id(),
                ));
            };
            let dark = dark.stable_id();
            let light = light.stable_id();
            self.observe(
                theme_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let next = if event.option == dark {
                        nana_ui_core::ThemeId::new("nana.dark")
                    } else if event.option == light {
                        nana_ui_core::ThemeId::new("nana.light")
                    } else {
                        return;
                    };
                    cx.emit(AppearanceEvent::Theme(next));
                },
            )?;
        }
        if created_material {
            let solid = assembly.material_solid;
            let translucent = assembly.material_translucent;
            let vibrancy = assembly.material_vibrancy;
            let mica = assembly.material_mica;
            let acrylic = assembly.material_acrylic;
            self.observe(
                material_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let next = if Some(event.option) == solid {
                        WindowMaterialMode::Solid
                    } else if Some(event.option) == translucent {
                        WindowMaterialMode::Translucent
                    } else if Some(event.option) == vibrancy {
                        WindowMaterialMode::Vibrancy
                    } else if Some(event.option) == mica {
                        WindowMaterialMode::Mica
                    } else if Some(event.option) == acrylic {
                        WindowMaterialMode::Acrylic
                    } else {
                        return;
                    };
                    cx.emit(AppearanceEvent::WindowMaterial(next));
                },
            )?;
        }
        if created_target {
            let sidebar = target_sidebar.stable_id();
            let main = target_main.stable_id();
            self.observe(
                target_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let next = if event.option == sidebar {
                        BackdropTarget::Sidebar
                    } else if event.option == main {
                        BackdropTarget::Main
                    } else {
                        return;
                    };
                    cx.emit(AppearanceEvent::BackdropTarget(next));
                },
            )?;
        }
        if created_titlebar {
            self.observe(titlebar_switch, section, |_, event: &ToggleChanged, cx| {
                cx.emit(AppearanceEvent::TitlebarFollowsSidebar(event.checked));
            })?;
        }
        if created_opacity_range && let Some(id) = assembly.opacity_range {
            self.observe(
                Entity::<RangeField>::from_stable_id(id),
                section,
                |_, event: &RangeInput, cx| {
                    cx.emit(AppearanceEvent::BackdropOpacity(event.value as f32 / 100.0));
                },
            )?;
        }
        if created_workspace {
            self.observe(workspace_switch, section, |_, event: &ToggleChanged, cx| {
                cx.emit(AppearanceEvent::WorkspaceCorners(event.checked));
            })?;
        }
        if created_radius {
            self.observe(radius_range, section, |_, event: &RangeInput, cx| {
                cx.emit(AppearanceEvent::StandardRadius(event.value.round() as u8));
            })?;
        }
        if created_reset {
            self.observe(reset_button, section, |_, _: &Activate, cx| {
                cx.emit(AppearanceEvent::Reset);
            })?;
        }

        let mut ordered = vec![theme_row.stable_id(), material_row.stable_id()];
        if let Some(status_row) = status_row {
            ordered.push(status_row.stable_id());
        }
        ordered.extend([
            target_row.stable_id(),
            titlebar_row.stable_id(),
            opacity_row.stable_id(),
            workspace_row.stable_id(),
            radius_row.stable_id(),
            reset_row.stable_id(),
        ]);
        self.update_component(section, |section, _| {
            section.assembly = Some(assembly);
        })?;
        reconcile_children(self, section, &ordered)
    }

    /// Mount or refresh name/version/description rows from injected metadata.
    pub fn assemble_about_section(
        &mut self,
        section: Entity<AboutSection>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, section.stable_id())?;
        let strings = self.world().framework_strings().clone();
        let (metadata, mut assembly) = self.read(section, |section| {
            (
                section.metadata.clone(),
                section.assembly.clone().unwrap_or_default(),
            )
        })?;
        let name_value = sync_text(
            self,
            document,
            &mut assembly.name_value,
            styled_text(
                metadata.product_title.as_ref(),
                SemanticColorRole::Text,
                12.0,
                500,
            ),
        )?;
        // The description describes the product, so it is the name row's
        // hint rather than a loose line under the version.
        let name_row = mount_settings_row(
            self,
            document,
            &mut assembly.name_row,
            strings.settings_name.as_ref(),
            metadata.description.as_deref(),
            true,
            true,
            false,
            name_value.stable_id(),
        )?;
        let version_value = sync_text(
            self,
            document,
            &mut assembly.version_value,
            styled_text(
                metadata.version.as_ref(),
                SemanticColorRole::Muted,
                12.0,
                400,
            ),
        )?;
        let version_row = mount_settings_row(
            self,
            document,
            &mut assembly.version_row,
            strings.settings_version.as_ref(),
            None,
            false,
            false,
            true,
            version_value.stable_id(),
        )?;
        let ordered = vec![name_row.stable_id(), version_row.stable_id()];
        assembly.description = None;
        self.update_component(section, |section, _| {
            section.assembly = Some(assembly);
        })?;
        reconcile_children(self, section, &ordered)
    }

    /// Mount header chrome and host slots. Details stay hidden while collapsed.
    pub fn assemble_settings_collapsible_card(
        &mut self,
        card: Entity<SettingsCollapsibleCard>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, card.stable_id())?;
        let snapshot = self.read(card, |card| {
            (
                card.summary,
                card.details,
                card.accessory,
                card.header,
                card.disclosure,
                card.divider,
                card.expanded,
            )
        })?;
        let (summary, details, accessory, header, disclosure, divider, expanded) = snapshot;
        let header = if let Some(id) = header {
            Entity::from_stable_id(id)
        } else {
            self.create_detached_component(document, SettingsCollapsibleHeader)?
        };
        let disclosure = if let Some(id) = disclosure {
            let entity = Entity::<SettingsDisclosure>::from_stable_id(id);
            self.update_component(entity, |mark, _| {
                mark.expanded = expanded;
            })?;
            entity
        } else {
            self.create_detached_component(document, SettingsDisclosure { expanded })?
        };
        let divider = if let Some(id) = divider {
            let entity = Entity::<SettingsCollapsibleDivider>::from_stable_id(id);
            self.update_component(entity, |mark, _| {
                mark.hidden = !expanded;
            })?;
            entity
        } else {
            self.create_detached_component(
                document,
                SettingsCollapsibleDivider { hidden: !expanded },
            )?
        };
        let mut header_children = Vec::new();
        if let Some(summary) = summary {
            header_children.push(summary);
        }
        if let Some(accessory) = accessory {
            header_children.push(accessory);
        }
        header_children.push(disclosure.stable_id());
        reconcile_children(self, header, &header_children)?;
        self.update_component(card, |card, _| {
            card.header = Some(header.stable_id());
            card.disclosure = Some(disclosure.stable_id());
            card.divider = Some(divider.stable_id());
        })?;
        let mut ordered = vec![header.stable_id(), divider.stable_id()];
        if let Some(details) = details {
            ordered.push(details);
        }
        reconcile_children(self, card, &ordered)
    }

    /// Mount back + tab rows from the host snapshot on [`SettingsSidebar`].
    ///
    /// Row activation re-emits [`SettingsBack`] / [`SettingsTabSelected`] from
    /// the sidebar. The host binds those events, updates [`SettingsState`],
    /// writes the snapshot back, and calls this method again.
    pub fn assemble_settings_sidebar(
        &mut self,
        sidebar: Entity<SettingsSidebar>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, sidebar.stable_id())?;
        let strings = self.world().framework_strings().clone();
        let (model, state, mut assembly) = self.read(sidebar, |sidebar| {
            (
                sidebar.model.clone(),
                sidebar.state.clone(),
                sidebar.assembly.clone().unwrap_or_default(),
            )
        })?;
        let mut back_icon = assembly.back_icon;
        let back_leading =
            ensure_settings_leading(self, document, &mut back_icon, Some(Icon::ArrowLeft), true)?;
        assembly.back_icon = back_icon;
        let mut back_row = assembly.back_row;
        let (back, created_back) = ensure_settings_nav_row(
            self,
            document,
            &mut back_row,
            strings.settings_back.as_ref(),
            SidebarRowState::Idle,
            back_leading.stable_id(),
        )?;
        assembly.back_row = back_row;
        if created_back {
            self.observe(back, sidebar, |_, _: &Activate, cx| {
                cx.emit(SettingsBack);
            })?;
        }

        let body = if let Some(id) = assembly.body {
            Entity::<ScrollView>::from_stable_id(id)
        } else {
            let entity = self.create_detached_component(document, settings_sidebar_body())?;
            assembly.body = Some(entity.stable_id());
            entity
        };

        let mut tab_children = Vec::new();
        for tab in model.tabs() {
            let selected = state.active_tab() == tab.id();
            let row_state = if selected {
                SidebarRowState::Active
            } else {
                SidebarRowState::Idle
            };
            let mut leading_slot = assembly.tab_leadings.get(tab.id()).copied();
            let leading = ensure_settings_leading(
                self,
                document,
                &mut leading_slot,
                tab.icon_value(),
                selected,
            )?;
            assembly
                .tab_leadings
                .insert(tab.id().clone(), leading.stable_id());
            let mut row_slot = assembly.tab_rows.get(tab.id()).copied();
            let (row, created_row) = ensure_settings_nav_row(
                self,
                document,
                &mut row_slot,
                tab.label(),
                row_state,
                leading.stable_id(),
            )?;
            assembly.tab_rows.insert(tab.id().clone(), row.stable_id());
            if created_row {
                let tab_id = tab.id().clone();
                self.observe(row, sidebar, move |_, _: &Activate, cx| {
                    cx.emit(SettingsTabSelected {
                        tab: tab_id.clone(),
                    });
                })?;
            }
            tab_children.push(row.stable_id());
        }

        let body_changed = reconcile_children(self, body, &tab_children)?;
        self.update_component(sidebar, |sidebar, _| {
            sidebar.assembly = Some(assembly.clone());
        })?;
        let frame_changed =
            reconcile_children(self, sidebar, &[back.stable_id(), body.stable_id()])?;
        Ok(body_changed || frame_changed)
    }

    /// Mount header + scroll chrome, or a fill child when the tab is full-page.
    pub fn assemble_settings_page(
        &mut self,
        page: Entity<SettingsPage>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, page.stable_id())?;
        let (model, state, content, padding, gap, title_font, mut assembly) =
            self.read(page, |page| {
                (
                    page.model.clone(),
                    page.state.clone(),
                    page.content,
                    page.content_padding,
                    page.content_gap,
                    (page.title_size, page.title_weight),
                    page.assembly.clone().unwrap_or_default(),
                )
            })?;
        let tab = state.active_view(&model);
        let full_page = tab.full_page_value();
        let show_header = !full_page && !model.hide_header_value();

        if show_header {
            sync_text(
                self,
                document,
                &mut assembly.title,
                page_title_text(tab.label(), title_font.0, title_font.1),
            )?;
        }

        if full_page {
            self.update_component(page, |page, _| {
                page.assembly = Some(assembly);
            })?;
            let ordered = content.into_iter().collect::<Vec<_>>();
            return reconcile_children(self, page, &ordered);
        }

        let body_style = SettingsPageBody { padding, gap };
        let body = if let Some(id) = assembly.body {
            let entity = Entity::<SettingsPageBody>::from_stable_id(id);
            self.update_component(entity, |body, _| *body = body_style)?;
            entity
        } else {
            let entity = self.create_detached_component(document, body_style)?;
            assembly.body = Some(entity.stable_id());
            entity
        };
        let scroll = if let Some(id) = assembly.scroll {
            Entity::<ScrollView>::from_stable_id(id)
        } else {
            let entity = self.create_detached_component(document, settings_page_scroll())?;
            assembly.scroll = Some(entity.stable_id());
            entity
        };

        let mut column_children = Vec::new();
        if show_header && let Some(title) = assembly.title {
            column_children.push(title);
        }
        if let Some(content) = content {
            column_children.push(content);
        }
        reconcile_children(self, body, &column_children)?;
        reconcile_children(self, scroll, &[body.stable_id()])?;
        let scroll_id = scroll.stable_id();
        self.update_component(page, |page, _| {
            page.assembly = Some(assembly);
        })?;
        reconcile_children(self, page, &[scroll_id])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::{AppContext, DocumentId, Entity, StandardVisual};
    use nana_ui_core::{BackdropTarget, WindowMaterialMode};

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
        assert_eq!(context.world().standard_visual(id), None);
        assert_eq!(context.world().text(id), Some(""));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.layout.direction, Some(FlexDirection::Column));
        assert_eq!(style.layout.justify_content, JustifySpec::Start);
        assert_eq!(style.layout.flex_wrap, FlexWrap::NoWrap);
        assert_eq!(style.layout.gap, Some(LengthSpec::Px(ROW_STACK_GAP_LOOSE)));
        assert_eq!(
            style.layout.padding_top,
            Some(LengthSpec::Px(ROW_GROUP_PADDING_Y))
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
            Some(LineHeightSpec::Absolute(nana_ui_core::type_scale::BODY))
        );
        assert_eq!(
            label_style.layout.font_weight,
            Some(nana_ui_core::type_scale::MEDIUM)
        );
        assert_eq!(label_style.foreground, Some(SemanticColorRole::Text));
        assert!(label_style.layout.white_space_nowrap);
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
            context.world().standard_visual(id),
            Some(StandardVisual::Card {
                ref title,
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
            context.world().standard_visual(section.stable_id()),
            Some(StandardVisual::Card {
                ref title,
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
            context.world().standard_visual(section.stable_id()),
            Some(StandardVisual::Card {
                ref title,
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
            context.world().standard_visual(disclosure.unwrap()),
            Some(StandardVisual::Icon {
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
            context.world().standard_visual(disclosure.unwrap()),
            Some(StandardVisual::Icon {
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
            context.world().standard_visual(icon),
            Some(StandardVisual::Icon {
                icon: Icon::Appearance,
                size: SETTINGS_SIDEBAR_ICON_SIZE,
                tooltip: None,
            })
        );
        assert_eq!(context.world().standard_visual(spacer), None);
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
            context.world().standard_visual(section.stable_id()),
            Some(StandardVisual::Card {
                ref title,
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
    use crate::{DocumentId, LayoutViewport, PathText};

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
