use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, JustifySpec, LayoutStyle, LengthSpec,
    LineHeightSpec, SemanticColorRole,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, InteractionStyle,
    MutationQueue, NodeKind, NodeStyle, SemanticPaint, StableNodeId, TextContent,
    TextHorizontalAlignment, TextVerticalAlignment, UiWorld,
};

/// Backend-neutral selection orientation. `Segmented` and `Tabs` are
/// horizontal by design; `Radio` stacks vertically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Shared selection contract with three product surfaces.
///
/// `Segmented` keeps the bordered pill. `Tabs` is the same RadioGroup/roving
/// behavior on an independent tab strip (no outer chrome). `Radio` is the same
/// behavior again as a stacked list of ring indicators, so a radio group costs
/// no second selection engine. Professional reorder, close, and drag/lease
/// behavior lives on [`crate::Tabs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionChrome {
    #[default]
    Segmented,
    Tabs,
    Radio,
}

impl SelectionChrome {
    /// Orientation this chrome lays its options out in.
    pub const fn orientation(self) -> SelectionOrientation {
        match self {
            Self::Segmented | Self::Tabs => SelectionOrientation::Horizontal,
            Self::Radio => SelectionOrientation::Vertical,
        }
    }
}

/// Selection-independent intent consumed by roving-focus components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RovingFocusIntent {
    Previous,
    Next,
    First,
    Last,
}

/// Resolves one enabled tab stop without knowing or changing selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RovingFocusPolicy {
    pub wrap: bool,
}

impl Default for RovingFocusPolicy {
    fn default() -> Self {
        Self { wrap: true }
    }
}

impl RovingFocusPolicy {
    pub fn resolve<T: Copy + Eq>(
        self,
        items: &[(T, bool)],
        current: Option<T>,
        intent: RovingFocusIntent,
    ) -> Option<T> {
        let enabled = items
            .iter()
            .filter_map(|(id, enabled)| enabled.then_some(*id))
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            return None;
        }
        match intent {
            RovingFocusIntent::First => enabled.first().copied(),
            RovingFocusIntent::Last => enabled.last().copied(),
            RovingFocusIntent::Previous | RovingFocusIntent::Next => {
                let current =
                    current.and_then(|current| enabled.iter().position(|id| *id == current));
                let next = match (current, intent) {
                    (Some(0), RovingFocusIntent::Previous) if self.wrap => enabled.len() - 1,
                    (Some(0), RovingFocusIntent::Previous) => 0,
                    (Some(index), RovingFocusIntent::Previous) => index - 1,
                    (Some(index), RovingFocusIntent::Next) if index + 1 < enabled.len() => {
                        index + 1
                    }
                    (Some(_), RovingFocusIntent::Next) if self.wrap => 0,
                    (Some(index), RovingFocusIntent::Next) => index,
                    (None, RovingFocusIntent::Previous) => enabled.len() - 1,
                    (None, RovingFocusIntent::Next) => 0,
                    _ => unreachable!(),
                };
                enabled.get(next).copied()
            }
        }
    }
}

/// A container whose focusable descendants the arrow keys walk, as in a
/// WAI-ARIA toolbar or menu: `El::roving_focus`, or
/// [`AppContext::set_roving_focus_group`](crate::AppContext::set_roving_focus_group).
///
/// Its items are the focusable, enabled, reachable nodes under it whose
/// nearest group is this one, in document order: a group nested inside
/// keeps its own items. With focus on an item, the arrows along
/// `orientation` (ArrowUp / ArrowDown for a vertical group, ArrowLeft /
/// ArrowRight for a horizontal one, swapped right to left) move it to the
/// previous or next item, and Home / End to the first or last. A control
/// that takes those keys itself, such as a text field or a slider, keeps
/// them. An arrow past an end wraps when `wrap` is set; otherwise focus
/// stays and the group emits [`RovingFocusEdge`]. Every item stays in the
/// Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RovingFocusGroup {
    pub orientation: SelectionOrientation,
    pub wrap: bool,
}

impl RovingFocusGroup {
    /// ArrowLeft / ArrowRight walk the items; no wrap.
    pub const fn horizontal() -> Self {
        Self {
            orientation: SelectionOrientation::Horizontal,
            wrap: false,
        }
    }

    /// ArrowUp / ArrowDown walk the items; no wrap.
    pub const fn vertical() -> Self {
        Self {
            orientation: SelectionOrientation::Vertical,
            wrap: false,
        }
    }

    /// Past the last item comes the first, and before the first the last.
    pub const fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }
}

/// The end of a [`RovingFocusGroup`] an arrow ran past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RovingEdge {
    /// Before the first item: ArrowUp, or ArrowLeft (ArrowRight right to
    /// left).
    Start,
    /// After the last item: ArrowDown, or ArrowRight (ArrowLeft right to
    /// left).
    End,
}

/// An arrow ran past an end of a [`RovingFocusGroup`] that does not wrap.
///
/// Emitted on the group itself; focus has not moved. A popup that the group
/// is in typically closes here and returns focus to its trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RovingFocusEdge {
    pub edge: RovingEdge,
    /// The focused item at that end.
    pub item: StableNodeId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentedControl {
    pub label: Option<Arc<str>>,
    pub orientation: SelectionOrientation,
    pub(crate) chrome: SelectionChrome,
    pub(crate) fill: bool,
    pub(crate) wrap: bool,
    pub(crate) size: ControlSize,
    pub(crate) options: Vec<StableNodeId>,
    pub(crate) selected: Option<StableNodeId>,
    pub(crate) focus_target: Option<StableNodeId>,
    pub(crate) roving_focus: RovingFocusPolicy,
    pub style: NodeStyle,
}

impl SegmentedControl {
    /// The surface its options take: size, chrome and fill.
    pub(crate) fn option_surface(&self) -> (ControlSize, SelectionChrome, bool) {
        (self.size, self.chrome, self.fill)
    }

    pub fn new() -> Self {
        let mut control = Self {
            label: None,
            orientation: SelectionOrientation::Horizontal,
            chrome: SelectionChrome::Segmented,
            fill: false,
            wrap: true,
            size: ControlSize::Medium,
            options: Vec::new(),
            selected: None,
            focus_target: None,
            roving_focus: RovingFocusPolicy::default(),
            style: NodeStyle::default(),
        };
        control.refresh_style();
        control
    }

    /// Same selection engine, radio chrome: a vertical stack of ring options.
    pub fn radio_group() -> Self {
        Self::new().chrome(SelectionChrome::Radio)
    }

    pub fn chrome(mut self, chrome: SelectionChrome) -> Self {
        self.chrome = chrome;
        self.orientation = chrome.orientation();
        self.refresh_style();
        self
    }

    pub const fn chrome_value(&self) -> SelectionChrome {
        self.chrome
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn size(mut self, size: ControlSize) -> Self {
        self.apply_size(size);
        self
    }

    pub(crate) fn apply_size(&mut self, size: ControlSize) {
        self.size = size;
        self.refresh_style();
    }
    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self.refresh_style();
        self
    }
    /// Wrap horizontal segmented options onto more lines when the width
    /// they are given cannot hold them on one; with `fill(true)`, each line
    /// is divided equally without shortening its labels. On by default: a
    /// control with room keeps its one line and its own width, and one in a
    /// narrow card or column wraps instead of overflowing or clipping its
    /// labels. `wrap(false)` keeps one line whatever the width.
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self.refresh_style();
        self
    }

    pub const fn wrap_value(&self) -> bool {
        self.wrap
    }

    fn refresh_style(&mut self) {
        self.style = selection_chrome_style(self.chrome, self.size, self.fill);
        if self.wrap && self.chrome == SelectionChrome::Segmented {
            // One line keeps the control's height; more lines grow it.
            self.style.control_height = Some(nana_ui_core::ControlHeight::Min(self.size));
            let layout = Arc::make_mut(&mut self.style.layout);
            if self.fill {
                layout.width = Some(LengthSpec::Fill);
            } else {
                // Its own width while that fits, the width given when not.
                layout.max_width = Some(LengthSpec::Percent(100.0));
            }
            layout.flex_wrap = nana_ui_core::FlexWrap::Wrap;
            layout.equal_wrap = self.fill;
        }
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn fill_value(&self) -> bool {
        self.fill
    }

    pub const fn size_value(&self) -> ControlSize {
        self.size
    }

    pub fn options(&self) -> &[StableNodeId] {
        &self.options
    }

    pub const fn selected(&self) -> Option<StableNodeId> {
        self.selected
    }

    pub const fn focus_target(&self) -> Option<StableNodeId> {
        self.focus_target
    }
}

impl Default for SegmentedControl {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for SegmentedControl {
    const TYPE_ID: Option<&'static str> =
        Some(crate::component_descriptors::SEGMENTED_CONTROL.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        hooks: Some(|hooks| {
            crate::framework::routes::segmented(hooks);
        }),
        slot_assembler: Some(crate::AppContext::assemble_segmented_control),
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
            tag: "segmented-control".into(),
        }
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
            AccessibilityState {
                role: AccessibilityRole::RadioGroup,
                label: self.label.clone(),
                orientation: Some(self.orientation),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Visual state for [`SegmentedOption`] (Segmented, Tabs, and Radio) nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionOptionVisual {
    pub label: Arc<str>,
    pub icon: Option<Icon>,
    pub selected: bool,
    pub disabled: bool,
    pub size: ControlSize,
    pub show_focus_ring: bool,
    pub indicator: bool,
}

impl SelectionOptionVisual {
    fn parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        text: crate::layout_engine::Size,
    ) -> [Option<crate::layout_engine::parts::FlexPart>; 3] {
        let gap = world.theme().spacing().xs;
        let icon = self.icon.map(|_| {
            let extent = if self.indicator {
                self.size.icon_size()
            } else {
                world
                    .computed_style(id)
                    .map_or(self.size.text_size(), |style| style.font_size)
            };
            (extent, gap)
        });
        crate::layout_engine::parts::selection_option_parts(
            self.indicator
                .then(|| (self.size.indicator_size(), self.size.indicator_gap())),
            icon,
            text,
        )
    }
}

impl crate::WidgetVisual for SelectionOptionVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn visual_parts_read_theme(&self) -> bool {
        self.icon.is_some()
    }

    fn is_document_select_target(&self) -> bool {
        true
    }

    fn content_layout_changed(&self, prev: Option<&dyn std::any::Any>) -> bool {
        let Some(old) = prev.and_then(|any| any.downcast_ref::<Self>()) else {
            return false;
        };
        (self.icon.is_some(), self.indicator, self.size)
            != (old.icon.is_some(), old.indicator, old.size)
    }

    fn invalidates_node_text_constraint(&self) -> bool {
        true
    }

    fn intrinsic_chrome_content(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> crate::WidgetVisualContent {
        let parts = self.parts(id, world, crate::layout_engine::Size::default());
        let chrome = crate::layout_engine::parts::chrome_extent(&parts);
        crate::WidgetVisualContent {
            width: chrome.width,
            height: chrome.height,
            text: 0.0,
        }
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _bounds: crate::LayoutBox,
        content: crate::LayoutBox,
        _writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        let text = world.text_metrics(id).unwrap_or_default();
        let inputs = self.parts(
            id,
            world,
            crate::layout_engine::Size::new(text.width, text.height),
        );
        Some(crate::layout_engine::parts::selection_option_row(
            content,
            inputs,
            world.selection_option_icon_line(id),
            world.right_to_left(id),
        ))
    }

    fn node_text_max_width(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _layout: crate::LayoutBox,
        available_width: f32,
        available_height: f32,
    ) -> Option<f32> {
        let parts = self.parts(id, world, crate::layout_engine::Size::default());
        Some(crate::layout_engine::parts::text_budget(
            parts,
            crate::layout_engine::Size::new(available_width, available_height),
        ))
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let mut out = crate::WidgetPaintOutput {
            suppress_root_surface: false,
            owns_node_text: true,
            ..crate::WidgetPaintOutput::new()
        };
        let indicator_box = ctx.content_part(crate::LayoutPartKind::Indicator);
        let label_bounds = ctx
            .content_part(crate::LayoutPartKind::TextContent)
            .unwrap_or(ctx.content);
        let label_color = ctx.style.color.unwrap_or_else(|| {
            ctx.style_model
                .color(SemanticColorRole::Text)
                .as_rgba_array()
        });
        out.push(
            2,
            crate::WidgetPaintOp::Text(crate::widget_visual::widget_text_region(
                &crate::ComponentTextRegion {
                    bounds: label_bounds,
                    content: crate::TextValue::from(Arc::clone(&self.label)),
                    color: Some(label_color),
                    font_size: ctx.style.font_size,
                    font_weight: ctx.style.font_weight,
                },
                ctx,
                if indicator_box.is_some() {
                    TextHorizontalAlignment::Start
                } else {
                    TextHorizontalAlignment::Center
                },
                true,
            )),
        );
        if let Some(ring) = indicator_box {
            let (ring_color, dot_color) = if self.disabled {
                let faint = ctx
                    .style_model
                    .color(SemanticColorRole::Faint)
                    .as_rgba_array();
                (faint, faint)
            } else if self.selected {
                let accent = ctx
                    .style_model
                    .color(SemanticColorRole::Accent)
                    .as_rgba_array();
                (accent, accent)
            } else {
                (
                    ctx.style_model
                        .color(SemanticColorRole::BorderStrong)
                        .as_rgba_array(),
                    ctx.style_model
                        .color(SemanticColorRole::Accent)
                        .as_rgba_array(),
                )
            };
            let dot_extent = ring.width / 2.5;
            out.push(
                8,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: ring,
                    border: Some((ring_color, if self.selected { 2.0 } else { 1.0 })),
                    border_paint: ctx
                        .matching_text_paint(ring_color)
                        .or_else(|| ctx.matching_border_paint(ring_color)),
                    corner_radius: [ring.height / 2.0; 4],
                    ..crate::WidgetQuad::default()
                }),
            );
            if self.selected {
                let dot = crate::LayoutBox {
                    x: ring.x + (ring.width - dot_extent) / 2.0,
                    y: ring.y + (ring.height - dot_extent) / 2.0,
                    width: dot_extent,
                    height: dot_extent,
                };
                out.push(
                    9,
                    crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                        rect: dot,
                        fill: Some(dot_color),
                        fill_paint: ctx.matching_text_paint(dot_color),
                        corner_radius: [dot.height / 2.0; 4],
                        ..crate::WidgetQuad::default()
                    }),
                );
            }
        }
        if let Some((icon, bounds)) = self
            .icon
            .zip(ctx.content_part(crate::LayoutPartKind::LeadingContent))
        {
            let color = ctx.style.color.unwrap_or_else(|| {
                ctx.style_model
                    .color(SemanticColorRole::Muted)
                    .as_rgba_array()
            });
            out.push(
                3,
                crate::WidgetPaintOp::Icon(crate::WidgetIcon {
                    bounds,
                    glyph: crate::WidgetGlyph::Icon(icon),
                    color: Some(color),
                    paint_color: ctx.matching_text_paint(color),
                    align_to_adjacent_text: false,
                }),
            );
        }
        if self.show_focus_ring && ctx.is_focus_visible() {
            out.push(
                7,
                crate::WidgetPaintOp::FocusRing {
                    bounds: ctx.bounds,
                    color: ctx
                        .style_model
                        .color(SemanticColorRole::FocusBorder)
                        .as_rgba_array(),
                    extra_outset: 1.0,
                },
            );
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentedOption {
    pub label: Arc<str>,
    pub icon: Option<Icon>,
    pub(crate) selected: bool,
    pub(crate) disabled: bool,
    pub(crate) size: ControlSize,
    pub(crate) chrome: SelectionChrome,
    pub(crate) fill: bool,
    pub(crate) wrap: bool,
    /// The control that is placing this option, for the projection that
    /// runs before the option is attached to it (see
    /// [`segmented_track_radius`]).
    pub(crate) track: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl SegmentedOption {
    pub fn new(label: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            selected: false,
            disabled: false,
            size: ControlSize::Medium,
            chrome: SelectionChrome::Segmented,
            fill: false,
            wrap: false,
            track: None,
            style: segmented_option_style(ControlSize::Medium, SelectionChrome::Segmented, false),
        }
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn size(mut self, size: ControlSize) -> Self {
        self.synchronize_surface(size, self.chrome, self.fill);
        self
    }
    pub fn surface(mut self, size: ControlSize, chrome: SelectionChrome, fill: bool) -> Self {
        self.synchronize_surface(size, chrome, fill);
        self
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn selected(&self) -> bool {
        self.selected
    }

    pub const fn disabled_value(&self) -> bool {
        self.disabled
    }

    pub const fn size_value(&self) -> ControlSize {
        self.size
    }

    pub(crate) fn synchronize_surface(
        &mut self,
        size: ControlSize,
        chrome: SelectionChrome,
        fill: bool,
    ) {
        if self.size == size && self.chrome == chrome && self.fill == fill {
            return;
        }
        self.size = size;
        self.chrome = chrome;
        self.fill = fill;
        self.refresh_style();
    }

    pub(crate) fn apply_wrap(&mut self, wrap: bool) {
        let wrap = wrap && self.chrome == SelectionChrome::Segmented;
        if self.wrap == wrap {
            return;
        }
        self.wrap = wrap;
        self.refresh_style();
    }

    fn refresh_style(&mut self) {
        let hidden = self.style.layout.hidden;
        self.style = segmented_option_style(self.size, self.chrome, self.fill);
        Arc::make_mut(&mut self.style.layout).hidden = hidden;
        if self.wrap {
            let layout = Arc::make_mut(&mut self.style.layout);
            // Natural widths decide line membership. The equal-line solver,
            // rather than individual flex growth, assigns the used widths.
            layout.width = None;
            layout.flex_grow = None;
            layout.flex_shrink = None;
            layout.white_space_nowrap = false;
            layout.text_ext.overflow_wrap = Some(nana_ui_core::OverflowWrapSpec::Anywhere);
        }
    }
}

/// The segmented track's own inner padding.
const SEGMENTED_TRACK_PADDING: f32 = nana_ui_core::space::XXS;
/// The segmented track's rule.
const SEGMENTED_TRACK_BORDER: f32 = nana_ui_core::HAIRLINE;
/// How far the selected pill sits inside the track's outer edge.
///
/// Both terms are load-bearing. The pill's corners have to stay concentric
/// with the track's, so its radius is the track's minus **this** — padding
/// *and* rule. Issue #101's constant convergence replaced the literal `3.0`
/// here with `space::XXS`, which dropped the rule and left the pill a pixel
/// rounder than the corner it sits in. Deriving the inset from the two values
/// that produce it is what stops that happening again.
fn segmented_pill_inset(world: &UiWorld) -> f32 {
    world.theme().spacing().xxs + world.theme().border().hairline
}

/// The corner of the track `option` sits in, as the track resolved it: the
/// shape recipe's step, or whatever step or number the application gave the
/// track. An option being placed is projected before it is attached, so the
/// control placing it (`placing`) stands in for its parent; with neither,
/// the recipe's step.
fn segmented_track_radius(
    option: StableNodeId,
    placing: Option<StableNodeId>,
    world: &UiWorld,
) -> f32 {
    world
        .parent_id(option)
        .or(placing)
        .and_then(|track| world.resolved_border_radius(track))
        .unwrap_or_else(|| {
            world
                .theme()
                .recipes()
                .shape()
                .resolve(nana_ui_core::ShapeRole::Segmented, world.theme_metrics())
        })
}

#[cfg(test)]
mod wrap_tests;

pub(crate) fn selection_chrome_style(
    chrome: SelectionChrome,
    size: ControlSize,
    fill: bool,
) -> NodeStyle {
    let (gap, padding, border_width, background, border) = match chrome {
        SelectionChrome::Segmented => (
            nana_ui_core::space::XXS,
            SEGMENTED_TRACK_PADDING,
            SEGMENTED_TRACK_BORDER,
            Some(SemanticColorRole::Background),
            Some(SemanticColorRole::Border),
        ),
        SelectionChrome::Tabs => (4.0, 0.0, 0.0, None, None),
        SelectionChrome::Radio => (2.0, 0.0, 0.0, None, None),
    };
    let vertical = matches!(chrome.orientation(), SelectionOrientation::Vertical);
    let mut layout = LayoutStyle {
        direction: Some(if vertical {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        }),
        gap: Some(LengthSpec::Px(gap)),
        padding: Some(LengthSpec::Px(padding)),
        width: Some(if fill {
            LengthSpec::Percent(100.0)
        } else {
            LengthSpec::Shrink
        }),
        height: if vertical {
            Some(LengthSpec::Shrink)
        } else {
            None
        },
        align_items: if vertical {
            AlignSpec::Stretch
        } else {
            AlignSpec::Center
        },
        border_width: Some(border_width),
        ..LayoutStyle::default()
    };
    layout.theme.gap = Some(crate::theme_intent::space(match chrome {
        SelectionChrome::Tabs => nana_ui_core::SpacingStep::Xs,
        SelectionChrome::Segmented | SelectionChrome::Radio => nana_ui_core::SpacingStep::Xxs,
    }));
    if chrome == SelectionChrome::Segmented {
        layout.theme.padding = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
        layout.theme.border_width =
            Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
    }
    NodeStyle {
        layout: Arc::new(layout),
        background,
        border,
        control_height: (!vertical).then_some(nana_ui_core::ControlHeight::Exact(size)),
        // A segmented control is a control, so by default it takes the
        // controls' step like Button and TextInput; the theme's shape recipe
        // decides (`ShapeRole::Segmented`).
        shape: matches!(chrome, SelectionChrome::Segmented)
            .then_some(nana_ui_core::ShapeRole::Segmented),
        ..NodeStyle::default()
    }
}

fn segmented_option_style(size: ControlSize, chrome: SelectionChrome, fill: bool) -> NodeStyle {
    let radio = matches!(chrome, SelectionChrome::Radio);
    // A radio row's ring and its gap are content parts (#205); the padding is
    // the row's own inset on both sides.
    let (padding_left, padding_right) = if radio {
        (
            Some(LengthSpec::Px(nana_ui_core::RADIO_ROW_INSET)),
            Some(LengthSpec::Px(nana_ui_core::RADIO_ROW_INSET)),
        )
    } else {
        // Segmented / Tabs inset is `padding_x_in + 2`, spent at project against
        // the installed metrics so a density change can move it.
        (None, None)
    };
    let mut layout = LayoutStyle {
        padding_left,
        padding_right,
        font_size: Some(size.text_size()),
        font_weight: Some(if radio {
            nana_ui_core::type_scale::REGULAR
        } else {
            nana_ui_core::type_scale::MEDIUM
        }),
        line_height: Some(LineHeightSpec::Absolute(size.line_height())),
        white_space_nowrap: true,
        align_self: Some(if radio {
            AlignSpec::Stretch
        } else {
            AlignSpec::Center
        }),
        justify_content: if radio {
            JustifySpec::Start
        } else {
            JustifySpec::Center
        },
        border_radius: None,
        flex_grow: fill.then_some(1.0),
        flex_shrink: fill.then_some(1.0),
        width: fill.then_some(LengthSpec::Fill),
        ..LayoutStyle::default()
    };
    layout.theme.control_typography(size);
    crate::theme_intent::weight(
        &mut layout,
        if radio {
            nana_ui_core::TextWeight::Regular
        } else {
            nana_ui_core::TextWeight::Medium
        },
    );
    // A radio row keeps its label at rest colour and never fills: selection
    // reads from the ring, not from a highlighted row.
    let selected = if radio {
        SemanticPaint::default()
    } else {
        SemanticPaint {
            foreground: Some(SemanticColorRole::Text),
            background: Some(SemanticColorRole::Selected),
            ..SemanticPaint::default()
        }
    };
    NodeStyle {
        layout: Arc::new(layout),
        // A tab and a radio row stand alone, so the recipe gives their step.
        // A segmented pill has no role: it is cut from its track's corner at
        // projection.
        shape: match chrome {
            SelectionChrome::Segmented => None,
            SelectionChrome::Tabs => Some(nana_ui_core::ShapeRole::Tab),
            SelectionChrome::Radio => Some(nana_ui_core::ShapeRole::RadioRow),
        },
        foreground: Some(if radio {
            SemanticColorRole::Text
        } else {
            SemanticColorRole::Muted
        }),
        interaction: InteractionStyle {
            selected,
            selected_hovered: SemanticPaint {
                background: Some(if radio {
                    SemanticColorRole::Hover
                } else {
                    SemanticColorRole::SelectedHover
                }),
                ..SemanticPaint::default()
            },
            selected_pressed: SemanticPaint {
                background: Some(if radio {
                    SemanticColorRole::Active
                } else {
                    SemanticColorRole::SelectedPressed
                }),
                ..SemanticPaint::default()
            },
            hovered: SemanticPaint {
                foreground: Some(SemanticColorRole::Text),
                background: Some(SemanticColorRole::Hover),
                ..SemanticPaint::default()
            },
            pressed: SemanticPaint {
                background: Some(SemanticColorRole::Active),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint::FOCUS_SURFACE,
            disabled: SemanticPaint {
                foreground: Some(SemanticColorRole::Faint),
                ..SemanticPaint::default()
            },
            ..Default::default()
        }
        .into(),
        text_horizontal_alignment: if radio {
            TextHorizontalAlignment::Start
        } else {
            TextHorizontalAlignment::Center
        },
        text_vertical_alignment: TextVerticalAlignment::Center,
        control_height: (!matches!(chrome, SelectionChrome::Segmented))
            .then_some(nana_ui_core::ControlHeight::Exact(size)),
        ..NodeStyle::default()
    }
}

impl ComponentView for SegmentedOption {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        hooks: Some(|hooks| {
            crate::framework::routes::segmented(hooks);
        }),
        activation: Some(crate::AppContext::activate_segmented_option),
        slot_assembler: Some(crate::AppContext::assemble_segmented_option),
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
            tag: match self.chrome {
                SelectionChrome::Segmented => "segmented-option".into(),
                SelectionChrome::Tabs => "tab".into(),
                SelectionChrome::Radio => "radio".into(),
            },
        }
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    /// A segmented pill is cut from its track's corner, which the shape
    /// recipe decides. (A tab or radio row names its role on its style and
    /// needs no reprojection.)
    fn wants_recipe_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.label.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.label.to_string().into(),
                },
            );
        }
        let visual = SelectionOptionVisual {
            label: Arc::clone(&self.label),
            icon: self.icon,
            selected: self.selected,
            disabled: self.disabled,
            size: self.size,
            show_focus_ring: matches!(self.chrome, SelectionChrome::Radio),
            indicator: matches!(self.chrome, SelectionChrome::Radio),
        };
        if world.widget_visual_as::<SelectionOptionVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        let metrics = world.theme_metrics();
        // A tab or radio row resolves its `shape` like any node; only a
        // segmented pill spends a number here, cut from its track.
        let pill = matches!(self.chrome, SelectionChrome::Segmented).then(|| {
            (segmented_track_radius(id, self.track, world) - segmented_pill_inset(world)).max(0.0)
        });
        {
            let layout = Arc::make_mut(&mut effective_style.layout);
            if let Some(radius) = pill {
                layout.border_radius = Some(radius);
            }
            if matches!(self.chrome, SelectionChrome::Segmented) {
                let height = Some(LengthSpec::Px(
                    (self.size.height_in(metrics) - world.theme().spacing().sm).max(0.0),
                ));
                if self.wrap {
                    layout.min_height = height;
                } else {
                    layout.height = height;
                }
            }
        }
        {
            // The ring, the icon and their gaps are content parts (#205); the
            // padding is the option's own inset. A segmented item's is spent
            // against the installed metrics so a density change can move it.
            let radio = matches!(self.chrome, SelectionChrome::Radio);
            let inset = if radio {
                nana_ui_core::RADIO_ROW_INSET
            } else {
                self.size.padding_x_in(world.theme_metrics()) + world.theme().spacing().xxs
            };
            let layout = Arc::make_mut(&mut effective_style.layout);
            layout.padding_left = Some(LengthSpec::Px(inset));
            if !radio {
                layout.padding_right = Some(LengthSpec::Px(inset));
            }
        }
        project_common(
            id,
            world,
            mutations,
            &effective_style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: match self.chrome {
                    SelectionChrome::Segmented | SelectionChrome::Radio => AccessibilityRole::Radio,
                    SelectionChrome::Tabs => AccessibilityRole::Tab,
                },
                label: Some(Arc::clone(&self.label)),
                disabled: self.disabled,
                // Segmented and radio chrome both announce as radio. A radio
                // needs `selected` as well as `checked`; a tab needs `selected`
                // and does not carry a checked state.
                selected: Some(self.selected),
                checked: (!matches!(self.chrome, SelectionChrome::Tabs)).then_some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentedSelectionRequested {
    pub option: StableNodeId,
}

/// Emitted on the option the user chose, after [`SegmentedSelectionRequested`]
/// on its control, so each option can carry its own handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentedOptionChosen;

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    #[test]
    fn roving_focus_skips_disabled_and_wraps() {
        let items = [(id(1), true), (id(2), false), (id(3), true)];
        let policy = RovingFocusPolicy::default();
        assert_eq!(
            policy.resolve(&items, Some(id(1)), RovingFocusIntent::Next),
            Some(id(3))
        );
        assert_eq!(
            policy.resolve(&items, Some(id(3)), RovingFocusIntent::Next),
            Some(id(1))
        );
        assert_eq!(
            policy.resolve(&items, Some(id(1)), RovingFocusIntent::Previous),
            Some(id(3))
        );
        assert_eq!(
            policy.resolve(&items, None, RovingFocusIntent::First),
            Some(id(1))
        );
        assert_eq!(
            policy.resolve(&items, None, RovingFocusIntent::Last),
            Some(id(3))
        );
    }

    #[test]
    fn segmented_sizes_preserve_concentric_layout_and_semantic_states() {
        for size in [ControlSize::Small, ControlSize::Medium, ControlSize::Large] {
            let option = SegmentedOption::new("Preview").size(size);
            assert!(option.style.layout.height.is_none());
            assert_eq!(option.style.layout.font_size, Some(size.text_size()));
            assert!(option.style.layout.white_space_nowrap);
            assert!(option.style.layout.border_radius.is_none());
            assert_eq!(
                option.style.interaction.selected.background,
                Some(SemanticColorRole::Selected)
            );
            let control = SegmentedControl::new().size(size).wrap(false);
            assert_eq!(
                control.style.control_height,
                Some(nana_ui_core::ControlHeight::Exact(size))
            );
            // Wrapping (the default) floors the height at the same step.
            assert_eq!(
                SegmentedControl::new().size(size).style.control_height,
                Some(nana_ui_core::ControlHeight::Min(size))
            );
            assert_eq!(
                control.style.shape,
                Some(nana_ui_core::ShapeRole::Segmented)
            );
            assert_eq!(control.style.layout.border_width, Some(1.0));
            assert!(control.style.layout.border_radius.is_none());
            assert_eq!(control.style.layout.width, Some(LengthSpec::Shrink));
        }
    }

    #[test]
    fn tabs_chrome_uses_independent_surface_and_tab_roles() {
        let tabs = crate::Tabs::new("preview");
        assert_eq!(tabs.size, ControlSize::Small);
        assert_eq!(tabs.style.layout.border_width, Some(0.0));
        assert_eq!(tabs.style.layout.gap, Some(LengthSpec::Px(4.0)));
        assert!(tabs.style.background.is_none());
        assert_eq!(tabs.node_kind(), NodeKind::Element { tag: "tabs".into() });

        let option = SegmentedOption::new("Preview").surface(
            ControlSize::Small,
            SelectionChrome::Tabs,
            false,
        );
        assert_eq!(
            option.style.control_height,
            Some(nana_ui_core::ControlHeight::Exact(ControlSize::Small))
        );
        assert_eq!(option.node_kind(), NodeKind::Element { tag: "tab".into() });
    }

    #[test]
    fn tabs_options_do_not_request_a_focus_ring() {
        let mut context = crate::AppContext::new();
        let document = crate::DocumentId::new(1).unwrap();
        let tab = context
            .create_component(
                document,
                SegmentedOption::new("Code").surface(
                    ControlSize::Small,
                    SelectionChrome::Tabs,
                    false,
                ),
            )
            .unwrap();
        let segmented = context
            .create_component(
                document,
                SegmentedOption::new("Code").size(ControlSize::Medium),
            )
            .unwrap();
        assert!(matches!(
            context
                .world()
                .widget_visual_as::<SelectionOptionVisual>(tab.stable_id()),
            Some(SelectionOptionVisual {
                show_focus_ring: false,
                ..
            })
        ));
        assert!(matches!(
            context
                .world()
                .widget_visual_as::<SelectionOptionVisual>(segmented.stable_id()),
            Some(SelectionOptionVisual {
                show_focus_ring: false,
                ..
            })
        ));
    }

    #[test]
    fn radio_chrome_stacks_vertically_and_leaves_room_for_the_ring() {
        let group = SegmentedControl::radio_group();
        assert_eq!(group.chrome_value(), SelectionChrome::Radio);
        assert_eq!(group.orientation, SelectionOrientation::Vertical);
        assert_eq!(group.style.layout.direction, Some(FlexDirection::Column));
        assert_eq!(group.style.layout.height, Some(LengthSpec::Shrink));
        assert_eq!(group.style.layout.border_width, Some(0.0));
        assert!(group.style.background.is_none());

        for size in [ControlSize::Small, ControlSize::Medium, ControlSize::Large] {
            let option =
                SegmentedOption::new("Manual").surface(size, SelectionChrome::Radio, false);
            // The ring and its gap are content parts, not padding (#205).
            assert_eq!(
                option.style.layout.padding_left,
                Some(LengthSpec::Px(nana_ui_core::RADIO_ROW_INSET))
            );
            assert_eq!(
                option.style.control_height,
                Some(nana_ui_core::ControlHeight::Exact(size))
            );
            assert_eq!(option.style.layout.justify_content, JustifySpec::Start);
            // Selection reads from the ring, so the row is never filled.
            assert!(option.style.interaction.selected.background.is_none());
            assert_eq!(
                option.node_kind(),
                NodeKind::Element {
                    tag: "radio".into()
                }
            );
        }
    }

    #[test]
    fn a_radio_group_selects_one_option_and_publishes_ring_geometry() {
        let mut context = crate::AppContext::new();
        let document = crate::DocumentId::new(1).unwrap();
        let group = context
            .create_component(document, SegmentedControl::radio_group().label("Mode"))
            .unwrap();
        let manual = context
            .create_component(document, SegmentedOption::new("Manual"))
            .unwrap();
        let automatic = context
            .create_component(document, SegmentedOption::new("Automatic"))
            .unwrap();
        context.append_child(group, manual).unwrap();
        context.append_child(group, automatic).unwrap();
        assert!(
            context
                .set_segmented_options(group, vec![manual, automatic], Some(automatic))
                .unwrap()
        );

        // Both options inherit radio chrome from the group, not from their own
        // construction, so the group stays the single source of surface truth.
        for option in [manual, automatic] {
            assert!(matches!(
                context
                    .world()
                    .widget_visual_as::<SelectionOptionVisual>(option.stable_id()),
                Some(SelectionOptionVisual {
                    indicator: true,
                    show_focus_ring: true,
                    ..
                })
            ));
            assert_eq!(
                context
                    .world()
                    .accessibility(option.stable_id())
                    .map(|state| state.role),
                Some(AccessibilityRole::Radio)
            );
        }
        assert_eq!(
            context
                .world()
                .accessibility(manual.stable_id())
                .and_then(|state| state.checked),
            Some(false)
        );
        assert_eq!(
            context
                .world()
                .accessibility(automatic.stable_id())
                .and_then(|state| state.checked),
            Some(true)
        );
        assert_eq!(
            context
                .world()
                .accessibility(manual.stable_id())
                .and_then(|state| state.selected),
            Some(false)
        );
        assert_eq!(
            context
                .world()
                .accessibility(automatic.stable_id())
                .and_then(|state| state.selected),
            Some(true)
        );
        assert_eq!(
            context
                .world()
                .accessibility(group.stable_id())
                .and_then(|state| state.orientation),
            Some(SelectionOrientation::Vertical)
        );

        let mut mutations = crate::MutationQueue::new();
        for (index, option) in [manual, automatic].iter().enumerate() {
            mutations.write_layout(
                option.stable_id(),
                crate::LayoutBox {
                    x: 0.0,
                    y: index as f32 * 30.0,
                    width: 160.0,
                    height: 28.0,
                },
            );
        }
        context.commit_mutations(mutations).unwrap();

        let ring = |option: crate::Entity<SegmentedOption>| {
            let paint = context
                .world()
                .widget_paint(option.stable_id())
                .expect("selection option paint");
            let quad = |slot| {
                paint
                    .ops
                    .iter()
                    .find(|tagged| tagged.slot == slot)
                    .map(|tagged| match &tagged.op {
                        crate::WidgetPaintOp::Quad(quad) => quad.rect,
                        other => panic!("expected quad at slot {slot}, got {other:?}"),
                    })
            };
            (quad(8), quad(9))
        };
        let (unselected_ring, unselected_dot) = ring(manual);
        let (selected_ring, selected_dot) = ring(automatic);
        let unselected = unselected_ring.expect("radio ring");
        let selected = selected_ring.expect("radio ring");
        assert!(unselected_dot.is_none());
        let dot = selected_dot.expect("selected radio dot");
        // The dot sits concentric inside the ring.
        assert!((dot.x + dot.width / 2.0 - (selected.x + selected.width / 2.0)).abs() < 0.001);
        assert!(dot.width < selected.width);
        assert_eq!(
            unselected.x,
            nana_ui_core::RADIO_ROW_INSET,
            "ring hugs the row inset"
        );
    }

    #[test]
    fn every_selection_chrome_publishes_selected() {
        for chrome in [
            SelectionChrome::Segmented,
            SelectionChrome::Radio,
            SelectionChrome::Tabs,
        ] {
            let mut context = crate::AppContext::new();
            let document = crate::DocumentId::new(1).unwrap();
            let on = context
                .create_component(
                    document,
                    SegmentedOption::new("On")
                        .surface(ControlSize::Small, chrome, false)
                        .with_selected(true),
                )
                .unwrap();
            let off = context
                .create_component(
                    document,
                    SegmentedOption::new("Off")
                        .surface(ControlSize::Small, chrome, false)
                        .with_selected(false),
                )
                .unwrap();
            for (option, selected) in [(on, true), (off, false)] {
                let state = context
                    .world()
                    .accessibility(option.stable_id())
                    .expect("selection option semantics");
                assert_eq!(state.selected, Some(selected), "{chrome:?}");
                if chrome == SelectionChrome::Tabs {
                    assert_eq!(state.role, AccessibilityRole::Tab);
                    assert_eq!(state.checked, None);
                } else {
                    assert_eq!(state.role, AccessibilityRole::Radio);
                    assert_eq!(state.checked, Some(selected));
                }
            }
        }
    }
}
