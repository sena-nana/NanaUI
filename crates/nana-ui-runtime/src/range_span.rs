//! Two-thumb range with a live indicator ([`RangeSpanField`]).
//!
//! The field node paints the track, the segment between the thumbs and the
//! indicator through a [`Painter`], so moving the indicator changes one
//! paint key and nothing else: no layout, no other node. Each thumb is a
//! child node of its own ([`RangeSpanHandle`]): a sequential focus stop, a
//! `Slider` for assistive technology, placed by absolute offsets inside the
//! field. Pointer, keyboard and assistive input all write the field, which
//! emits [`RangeSpanInput`] / [`RangeSpanChanged`] / [`RangeSpanDragging`]
//! with the same meaning [`crate::RangeField`]'s three events have.
//!
//! With [`RangeSpanField::value_thumbs`] each thumb is a capsule with its
//! value written in it, as a [`crate::RangeField::value_thumb`] is: a click
//! on one (or Enter, Space or an assistive press while it has focus) types a
//! value in a number field laid over it, and [`RangeSpanEntered`] says which
//! thumb was typed and what, before the pair it lands on is committed.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use nana_ui_core::{
    ControlHeight, ControlSize, LengthSpec, NumberFieldSpec, PositionSpec, SemanticColorRole,
    SpacingTokens, TypographyTokens, space,
};

use crate::view_components::{NumberChanged, NumberInput, format_range_value, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, BoxPaint, ComponentView, DocumentId, Entity,
    FocusWithinChanged, FrameworkError, InteractionState, LayoutBox, MutationQueue, NodeKind,
    NodePainter, NodeStyle, PaintContext, PaintText, Painter, RangeAdjustment,
    SelectionOrientation, StableNodeId, TextHorizontalAlignment, TextVerticalAlignment, UiWorld,
};

/// Which way a [`RangeSpanField`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum RangeSpanOrientation {
    /// Minimum at the left, maximum at the right.
    #[default]
    Horizontal,
    /// Minimum at the bottom, maximum at the top.
    Vertical,
}

/// One of the two thumbs of a [`RangeSpanField`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeSpanThumb {
    Low,
    High,
}

/// Diameter of a [`RangeSpanField`] thumb: the steps a [`crate::RangeField`]
/// thumb takes.
pub const fn range_span_thumb_extent(size: ControlSize) -> f32 {
    match size {
        ControlSize::Small => space::XL,
        ControlSize::Medium => space::XXL,
        ControlSize::Large => space::XXXL,
    }
}

/// How far a thumb's focus ring reaches past the thumb: a hairline gap and a
/// ring as wide as the smallest spacing step, as on a [`crate::RangeField`].
pub(crate) const fn focus_ring_outset() -> f32 {
    nana_ui_core::HAIRLINE + space::XXS
}

/// Distance, along the main axis, from either end of a [`RangeSpanField`]
/// node to the end of its track: where the minimum and the maximum sit.
///
/// The field has no padding or border of its own, so this is measured from
/// the node's edge. It is half a thumb plus its focus ring, so a thumb at
/// either end, ring included, stays inside the node. An application drawing
/// next to the control (a scale, a live meter) aligns to it.
pub const fn range_span_track_inset(size: ControlSize) -> f32 {
    range_span_thumb_extent(size) / 2.0 + focus_ring_outset()
}

/// A value thumb's height ([`RangeSpanField::value_thumbs`]): the height of
/// a [`crate::RangeField::value_thumb`] capsule of the same size.
pub fn range_span_value_thumb_height(
    size: ControlSize,
    typography: TypographyTokens,
    spacing: SpacingTokens,
) -> f32 {
    crate::world::range_value_thumb_height(size, typography, spacing)
}

/// [`range_span_track_inset`] of a vertical field with value thumbs: half a
/// capsule's height plus its focus ring.
pub fn range_span_value_track_inset(
    size: ControlSize,
    typography: TypographyTokens,
    spacing: SpacingTokens,
) -> f32 {
    range_span_value_thumb_height(size, typography, spacing) / 2.0 + focus_ring_outset()
}

/// A two-thumb range: `low ..= high` inside `minimum ..= maximum`, with an
/// optional live [`indicator`](Self::indicator).
#[derive(Debug, Clone, PartialEq)]
pub struct RangeSpanField {
    pub low: f64,
    pub high: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub step: f64,
    pub page_step: f64,
    /// A live value marked on the track. It does not snap to the step and
    /// takes no input; a value outside the range is drawn at the nearer end,
    /// a non-finite one not at all.
    pub indicator: Option<f64>,
    pub orientation: RangeSpanOrientation,
    /// Names the pair for assistive technology, and each thumb that has no
    /// name of its own.
    pub label: Option<Arc<str>>,
    /// The low thumb's accessible name ("输入下限").
    pub low_label: Option<Arc<str>>,
    /// The high thumb's accessible name ("输入上限").
    pub high_label: Option<Arc<str>>,
    pub size: ControlSize,
    pub disabled: bool,
    pub dragging: Option<RangeSpanDragState>,
    /// Each thumb is a capsule with its value in it, and a click on it types
    /// a value; see the module documentation.
    pub value_thumbs: bool,
    pub style: NodeStyle,
    /// The two thumb nodes, low first, once assembled.
    pub(crate) thumbs: Option<[StableNodeId; 2]>,
    /// The value capsules' measure, once assembled with value thumbs.
    pub(crate) capsule: Option<ValueCapsule>,
    /// The thumb a value is being typed for.
    pub(crate) editing: Option<RangeSpanThumb>,
    /// The number field typing lays over the thumb, once assembled.
    pub(crate) input: Option<StableNodeId>,
}

/// How large a value thumb is and how its value is written: every thumb of
/// a field as wide as its widest value, so none changes width as it moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ValueCapsule {
    width: f32,
    height: f32,
    font_size: f32,
    font_weight: u16,
}

impl ValueCapsule {
    /// Along the track and across it.
    fn extents(self, orientation: RangeSpanOrientation) -> (f32, f32) {
        match orientation {
            RangeSpanOrientation::Horizontal => (self.width, self.height),
            RangeSpanOrientation::Vertical => (self.height, self.width),
        }
    }
}

/// A pointer drag on a [`RangeSpanField`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeSpanDragState {
    pub pointer_id: u64,
    /// The thumb that moves. `None` while both thumbs sit on one value and
    /// the press landed on them: the first movement decides (up the range
    /// takes the high thumb, down the low one).
    pub thumb: Option<RangeSpanThumb>,
    /// Thumb value minus pointer value when the press grabbed a thumb, so it
    /// does not jump to the pointer; zero for a press on the track.
    pub grab_offset: f64,
    pub initial_low: f64,
    pub initial_high: f64,
    /// The thumb the press landed on; `None` for a press on the track.
    pub grabbed: Option<RangeSpanThumb>,
    /// Where the press landed, in layout space.
    pub press: (f32, f32),
    /// The pointer has travelled far enough from the press to be a drag
    /// rather than a click.
    pub moved: bool,
}

/// How far a press may wander and still be a click on a value thumb.
const CLICK_SLOP: f32 = space::XS;

/// A value typed for one thumb of a field with
/// [`value_thumbs`](RangeSpanField::value_thumbs): the value, quantized into
/// the range but not held on its side of the other thumb. Emitted before
/// the [`RangeSpanInput`] / [`RangeSpanChanged`] of the pair it lands on,
/// which is the value and the other thumb in order: a value typed past the
/// other thumb takes its place. An application that keeps the pair the
/// other way round reads this to know which end was typed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeSpanEntered {
    pub thumb: RangeSpanThumb,
    pub value: f64,
}

impl RangeSpanThumb {
    /// The thumb on the other side.
    pub fn other(self) -> Self {
        match self {
            RangeSpanThumb::Low => RangeSpanThumb::High,
            RangeSpanThumb::High => RangeSpanThumb::Low,
        }
    }
}

/// Every pair the field shows, including a drag or keyboard step in flight.
/// Observe this for live previews.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeSpanInput {
    pub low: f64,
    pub high: f64,
}

/// A committed pair: pointer release, a keyboard step, an accessibility
/// `Increment` / `Decrement` / `SetValue`, or
/// [`AppContext::set_range_span`]. A drag emits it once, on release, and
/// only when a thumb moved; a cancelled drag never does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeSpanChanged {
    pub low: f64,
    pub high: f64,
}

/// A pointer drag on the field began (`true`) or ended (`false`), after the
/// [`RangeSpanChanged`] a release commits, or the [`RangeSpanInput`] a cancel
/// restores. Keyboard and accessibility steps are not drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeSpanDragging {
    pub dragging: bool,
}

impl RangeSpanField {
    /// Builds a range span, repairing inconsistent input instead of failing.
    ///
    /// Bounds and `step` are repaired as [`crate::RangeField::new`] repairs
    /// them. A non-finite `low` / `high` falls back to the minimum / maximum;
    /// both are clamped, quantized, and swapped if they arrive crossed.
    pub fn new(low: f64, high: f64, minimum: f64, maximum: f64, step: f64) -> Self {
        let (minimum, maximum) = if minimum.is_finite() && maximum.is_finite() && minimum < maximum
        {
            (minimum, maximum)
        } else {
            (0.0, 1.0)
        };
        let step = if step.is_finite() && step > 0.0 {
            step
        } else {
            (maximum - minimum) / 100.0
        };
        let mut field = Self {
            low: minimum,
            high: maximum,
            minimum,
            maximum,
            step,
            page_step: step * 10.0,
            indicator: None,
            orientation: RangeSpanOrientation::Horizontal,
            label: None,
            low_label: None,
            high_label: None,
            size: ControlSize::Medium,
            disabled: false,
            dragging: None,
            value_thumbs: false,
            style: NodeStyle::default(),
            thumbs: None,
            capsule: None,
            editing: None,
            input: None,
        };
        field.set_span(low, high);
        field
    }

    pub fn orientation(mut self, orientation: RangeSpanOrientation) -> Self {
        self.orientation = orientation;
        self
    }
    /// [`RangeSpanOrientation::Vertical`]: minimum at the bottom.
    pub fn vertical(self) -> Self {
        self.orientation(RangeSpanOrientation::Vertical)
    }
    pub fn indicator(mut self, indicator: Option<f64>) -> Self {
        self.indicator = indicator;
        self
    }
    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn low_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.low_label = Some(label.into());
        self
    }
    pub fn high_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.high_label = Some(label.into());
        self
    }
    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    /// See [`Self::value_thumbs`](field@Self::value_thumbs).
    pub fn value_thumbs(mut self, value_thumbs: bool) -> Self {
        self.value_thumbs = value_thumbs;
        self
    }
    /// Sets the page step, ignoring a non-finite or non-positive value.
    pub fn page_step(mut self, page_step: f64) -> Self {
        if page_step.is_finite() && page_step > 0.0 {
            self.page_step = page_step;
        }
        self
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// The value of one thumb.
    pub fn value(&self, thumb: RangeSpanThumb) -> f64 {
        match thumb {
            RangeSpanThumb::Low => self.low,
            RangeSpanThumb::High => self.high,
        }
    }

    /// `value` clamped into the range and onto the step grid. A grid with a
    /// short decimal spelling lands on it exactly: `0.6`, not
    /// `0.6000000000000001`.
    pub fn quantize(&self, value: f64) -> f64 {
        let steps = ((value.clamp(self.minimum, self.maximum) - self.minimum) / self.step).round();
        let mut snapped = self.minimum + steps * self.step;
        if let (Some(step), Some(origin)) =
            (decimal_places(self.step), decimal_places(self.minimum))
        {
            let scale = 10_f64.powi(step.max(origin));
            snapped = (snapped * scale).round() / scale;
        }
        snapped.clamp(self.minimum, self.maximum)
    }

    /// Where `value` sits along the range, `0.0` at the minimum.
    pub fn ratio(&self, value: f64) -> f32 {
        let span = self.maximum - self.minimum;
        if !(value.is_finite() && span > 0.0) {
            return 0.0;
        }
        ((value - self.minimum) / span).clamp(0.0, 1.0) as f32
    }

    /// The value `thumb` would take for `value`: quantized, and held on its
    /// side of the other thumb, so the two never cross.
    pub fn clamp_thumb(&self, thumb: RangeSpanThumb, value: f64) -> f64 {
        let value = self.quantize(value);
        match thumb {
            RangeSpanThumb::Low => value.min(self.high),
            RangeSpanThumb::High => value.max(self.low),
        }
    }

    /// Sets both thumbs, quantized, in order whichever way they arrive.
    pub fn set_span(&mut self, low: f64, high: f64) {
        let low = if low.is_finite() {
            self.quantize(low)
        } else {
            self.minimum
        };
        let high = if high.is_finite() {
            self.quantize(high)
        } else {
            self.maximum
        };
        (self.low, self.high) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
    }

    /// What [`Self::assign`] stores for `thumb`.
    pub(crate) fn assigned(&self, thumb: RangeSpanThumb, value: f64) -> f64 {
        if value.is_finite() {
            self.quantize(value)
        } else {
            match thumb {
                RangeSpanThumb::Low => self.minimum,
                RangeSpanThumb::High => self.maximum,
            }
        }
    }

    /// An application write of one thumb: quantized, and pushing the other
    /// thumb along rather than stopping at it, so writing `low` then `high`
    /// lands on the same pair as writing them the other way round.
    pub(crate) fn assign(&mut self, thumb: RangeSpanThumb, value: f64) {
        let value = self.assigned(thumb, value);
        match thumb {
            RangeSpanThumb::Low => {
                self.low = value;
                self.high = self.high.max(value);
            }
            RangeSpanThumb::High => {
                self.high = value;
                self.low = self.low.min(value);
            }
        }
    }

    /// See [`range_span_track_inset`]; with value thumbs, half a capsule
    /// along the track plus its focus ring.
    pub fn track_inset(&self) -> f32 {
        match self.value_capsule() {
            Some(capsule) => capsule.extents(self.orientation).0 / 2.0 + focus_ring_outset(),
            None => range_span_track_inset(self.size),
        }
    }

    /// The thumbs' capsule, while they are value thumbs and measured.
    fn value_capsule(&self) -> Option<ValueCapsule> {
        self.capsule.filter(|_| self.value_thumbs)
    }

    /// The thumbs' extent across the track, focus ring included.
    fn thumb_girth(&self) -> f32 {
        match self.value_capsule() {
            Some(capsule) => capsule.extents(self.orientation).1 + 2.0 * focus_ring_outset(),
            None => 2.0 * range_span_track_inset(self.size),
        }
    }

    /// The (unquantized) value a pointer at `x`, `y` points at, for a field
    /// laid out at `bounds`. Both are in the same space.
    pub fn value_at(&self, bounds: LayoutBox, x: f32, y: f32) -> f64 {
        let inset = self.track_inset();
        let (position, extent) = match self.orientation {
            RangeSpanOrientation::Horizontal => (x - bounds.x, bounds.width),
            RangeSpanOrientation::Vertical => (bounds.y + bounds.height - y, bounds.height),
        };
        let length = extent - 2.0 * inset;
        let ratio = if length > 0.0 {
            ((position - inset) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.minimum + f64::from(ratio) * (self.maximum - self.minimum)
    }

    /// The thumb a press at `value` on the track moves: the nearer one, the
    /// low one on a tie. `None` while both sit on `value` itself.
    pub fn nearer_thumb(&self, value: f64) -> Option<RangeSpanThumb> {
        if value < self.low {
            Some(RangeSpanThumb::Low)
        } else if value > self.high {
            Some(RangeSpanThumb::High)
        } else if self.low == self.high {
            None
        } else if value - self.low <= self.high - value {
            Some(RangeSpanThumb::Low)
        } else {
            Some(RangeSpanThumb::High)
        }
    }

    /// The node of one thumb, once the field is assembled.
    pub fn thumb_node(&self, thumb: RangeSpanThumb) -> Option<StableNodeId> {
        self.thumbs.map(|[low, high]| match thumb {
            RangeSpanThumb::Low => low,
            RangeSpanThumb::High => high,
        })
    }

    fn handle(&self, thumb: RangeSpanThumb) -> RangeSpanHandle {
        let value = self.value(thumb);
        // A thumb ranges up to the other one, as the multi-thumb slider
        // pattern has it, so a screen reader announces where it can go.
        let (minimum, maximum) = match thumb {
            RangeSpanThumb::Low => (self.minimum, self.high),
            RangeSpanThumb::High => (self.low, self.maximum),
        };
        let own = match thumb {
            RangeSpanThumb::Low => &self.low_label,
            RangeSpanThumb::High => &self.high_label,
        };
        RangeSpanHandle {
            thumb,
            ratio: self.ratio(value),
            orientation: self.orientation,
            size: self.size,
            disabled: self.disabled,
            label: own.clone().or_else(|| self.label.clone()),
            value,
            minimum,
            maximum,
            step: self.step,
            value_text: format_range_value(value, self.step),
            capsule: self.value_capsule(),
            editing: self.editing == Some(thumb),
        }
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let inset = self.track_inset();
        let vertical = self.orientation == RangeSpanOrientation::Vertical;
        let layout = &style.layout;
        let needs_layout = layout.position == PositionSpec::Static
            || layout.width.is_none()
            || (vertical && layout.height.is_none());
        if needs_layout {
            let layout = Arc::make_mut(&mut style.layout);
            // The thumbs are placed against this box.
            if layout.position == PositionSpec::Static {
                layout.position = PositionSpec::Relative;
            }
            if vertical {
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Px(self.thumb_girth()));
                }
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
            } else if layout.width.is_none() {
                layout.width = Some(LengthSpec::Fill);
            }
        }
        if !vertical && style.control_height.is_none() && style.layout.height.is_none() {
            style.control_height = Some(ControlHeight::Min(self.size));
        }
        style.painter = Some(NodePainter::new(TrackPainter {
            vertical,
            inset,
            low: self.ratio(self.low),
            high: self.ratio(self.high),
            indicator: self
                .indicator
                .filter(|value| value.is_finite())
                .map(|value| self.ratio(value)),
            under_thumbs: self.value_thumbs,
        }));
        style
    }
}

static RANGE_SPAN_DRAG_OPS: crate::framework::ActiveDragOps = crate::framework::ActiveDragOps {
    pre_gesture: false,
    cancel_on_touch_scroll: true,
    update: |cx, doc, _, ptr, x, y, _| cx.update_range_span_drag_at(doc, ptr, x, y),
    end: |cx, doc, _, ptr, _, _, _, cancel| cx.end_range_span_drag(doc, ptr, cancel),
};

static RANGE_SPAN_CAPABILITIES: crate::framework::RangeSpanCapabilities =
    crate::framework::RangeSpanCapabilities {
        target: |cx, id| cx.range_span_target(id).map(|(e, t)| (e.stable_id(), t)),
        begin_drag: AppContext::begin_range_span_drag_at,
        adjust_thumb: |cx, id, thumb, adj| {
            cx.adjust_range_span(Entity::from_stable_id(id), thumb, adj)
        },
        set_thumb: |cx, id, thumb, val| {
            cx.set_range_span_thumb(Entity::from_stable_id(id), thumb, val)
        },
    };

static RANGE_SPAN_POINTER_OPS: crate::framework::PointerOps = crate::framework::PointerOps {
    is_value_range: true,
    end_value_drag: Some(AppContext::end_range_span_drag_on),
    ..crate::framework::PointerOps::EMPTY
};

impl ComponentView for RangeSpanField {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(|caps| {
            caps.range_span = Some(&RANGE_SPAN_CAPABILITIES);
            caps.add_step_adjuster(AppContext::adjust_focused_range_span);
            caps.add_edit_ender(AppContext::end_focused_range_span_edit);
            crate::view_components::install_number_input_capabilities(caps);
        }),
        lifecycle: Some(crate::AppContext::sync_range_span),
        pointer_ops: Some(&RANGE_SPAN_POINTER_OPS),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Declarative properties replace the props; the thumb nodes and a drag
    /// in flight are the field's own.
    fn reconcile(&mut self, mut next: Self) {
        next.thumbs = self.thumbs;
        next.dragging = self.dragging;
        next.capsule = self.capsule;
        next.input = self.input;
        next.editing = self.editing.filter(|_| next.value_thumbs && !next.disabled);
        *self = next;
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "range-span".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: !self.disabled,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: self.label.clone(),
                disabled: self.disabled,
                orientation: Some(selection_orientation(self.orientation)),
                ..AccessibilityState::default()
            },
        );
    }
}

/// The decimal places `value` is written with, up to six; `None` past that.
pub(crate) fn decimal_places(value: f64) -> Option<i32> {
    (0..=6).find(|places| {
        let scaled = value * 10_f64.powi(*places);
        (scaled - scaled.round()).abs() <= f64::EPSILON * scaled.abs().max(1.0)
    })
}

fn selection_orientation(orientation: RangeSpanOrientation) -> SelectionOrientation {
    match orientation {
        RangeSpanOrientation::Horizontal => SelectionOrientation::Horizontal,
        RangeSpanOrientation::Vertical => SelectionOrientation::Vertical,
    }
}

/// One thumb of a [`RangeSpanField`]: assembled by the field, which writes
/// every prop. Applications reach it through [`RangeSpanField::thumb_node`].
#[derive(Debug, Clone, PartialEq)]
pub struct RangeSpanHandle {
    thumb: RangeSpanThumb,
    ratio: f32,
    orientation: RangeSpanOrientation,
    size: ControlSize,
    disabled: bool,
    label: Option<Arc<str>>,
    value: f64,
    minimum: f64,
    maximum: f64,
    step: f64,
    value_text: Arc<str>,
    capsule: Option<ValueCapsule>,
    /// A value is being typed for this thumb: the number field covers it.
    editing: bool,
}

impl RangeSpanHandle {
    pub fn thumb(&self) -> RangeSpanThumb {
        self.thumb
    }

    fn style(&self) -> NodeStyle {
        let ring = focus_ring_outset();
        // The node is the thumb plus its ring, `along` long down the track
        // and `girth` across it.
        let (along_extent, girth, painter) = match self.capsule {
            Some(capsule) => {
                let (along, across) = capsule.extents(self.orientation);
                (
                    along + 2.0 * ring,
                    across + 2.0 * ring,
                    NodePainter::new(ValueThumbPainter {
                        text: Arc::clone(&self.value_text),
                        font_size: capsule.font_size,
                        font_weight: capsule.font_weight,
                        editing: self.editing,
                    }),
                )
            }
            None => {
                let extent = 2.0 * range_span_track_inset(self.size);
                (
                    extent,
                    extent,
                    NodePainter::new(ThumbPainter {
                        extent: range_span_thumb_extent(self.size),
                    }),
                )
            }
        };
        let (left, top) = thumb_offsets(self.orientation, self.ratio, along_extent, girth);
        let (width, height) = match self.orientation {
            RangeSpanOrientation::Horizontal => (along_extent, girth),
            RangeSpanOrientation::Vertical => (girth, along_extent),
        };
        NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                position: PositionSpec::Absolute,
                offset_left: Some(left),
                offset_top: Some(top),
                width: Some(LengthSpec::Px(width)),
                height: Some(LengthSpec::Px(height)),
                ..nana_ui_core::LayoutStyle::default()
            }),
            painter: Some(painter),
            ..NodeStyle::default()
        }
    }
}

/// Where a box `along` long and `girth` across sits in the field's box so
/// that its centre is on the track point `ratio` of the way along (the
/// track running half the box's length in from each end) and on the
/// track's line.
fn thumb_offsets(
    orientation: RangeSpanOrientation,
    ratio: f32,
    along: f32,
    girth: f32,
) -> (LengthSpec, LengthSpec) {
    let at = |ratio: f32| LengthSpec::percent_offset_inline(ratio * 100.0, -ratio * along);
    let centred = LengthSpec::percent_offset_inline(50.0, -girth / 2.0);
    match orientation {
        RangeSpanOrientation::Horizontal => (at(ratio), centred),
        RangeSpanOrientation::Vertical => (centred, at(1.0 - ratio)),
    }
}

impl ComponentView for RangeSpanHandle {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::activate_range_span_handle),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "range-span-thumb".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style(),
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Slider,
                label: self.label.clone(),
                value: Some(Arc::clone(&self.value_text)),
                disabled: self.disabled,
                orientation: Some(selection_orientation(self.orientation)),
                numeric_minimum: Some(self.minimum),
                numeric_maximum: Some(self.maximum),
                numeric_step: Some(self.step),
                numeric_value: Some(self.value),
                ..AccessibilityState::default()
            },
        );
    }
}

/// The track, the segment between the thumbs, and the indicator.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TrackPainter {
    vertical: bool,
    inset: f32,
    low: f32,
    high: f32,
    indicator: Option<f32>,
    /// The indicator is drawn under the thumbs, so a value capsule's text
    /// stays readable where the live value passes it.
    under_thumbs: bool,
}

impl TrackPainter {
    /// The stretch of the track between two ratios, `girth` thick.
    fn segment(&self, size: [f32; 2], from: f32, to: f32, girth: f32) -> LayoutBox {
        let [width, height] = size;
        if self.vertical {
            let length = (height - 2.0 * self.inset).max(0.0);
            LayoutBox {
                x: (width - girth) / 2.0,
                y: height - self.inset - to * length,
                width: girth,
                height: (to - from) * length,
            }
        } else {
            let length = (width - 2.0 * self.inset).max(0.0);
            LayoutBox {
                x: self.inset + from * length,
                y: (height - girth) / 2.0,
                width: (to - from) * length,
                height: girth,
            }
        }
    }
}

impl Painter for TrackPainter {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        let disabled = cx.state().disabled;
        let size = cx.size();
        let girth = space::XS;
        let (track, fill) = if disabled {
            (SemanticColorRole::Border, SemanticColorRole::Faint)
        } else {
            (SemanticColorRole::BorderStrong, SemanticColorRole::Accent)
        };
        // The neutral track runs outside the span only; the span itself is a
        // thinner solid accent bar centred on the track line.
        let span = girth * 2.0 / 3.0;
        for (from, to, role, thickness) in [
            (0.0, self.low, track, girth),
            (self.low, self.high, fill, span),
            (self.high, 1.0, track, girth),
        ] {
            if to > from {
                cx.rounded_rect(
                    self.segment(size, from, to, thickness),
                    thickness / 2.0,
                    BoxPaint::fill(role),
                );
            }
        }
        if self.under_thumbs {
            self.paint_indicator(cx);
        }
    }

    /// Over plain thumbs, so the live value stays readable where a thumb
    /// sits; value thumbs keep their text instead.
    fn paint_over_children(&self, cx: &mut PaintContext<'_>) {
        if !self.under_thumbs {
            self.paint_indicator(cx);
        }
    }

    fn paint_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.vertical.hash(&mut hasher);
        self.inset.to_bits().hash(&mut hasher);
        self.low.to_bits().hash(&mut hasher);
        self.high.to_bits().hash(&mut hasher);
        self.indicator.map(f32::to_bits).hash(&mut hasher);
        self.under_thumbs.hash(&mut hasher);
        hasher.finish()
    }
}

impl TrackPainter {
    fn paint_indicator(&self, cx: &mut PaintContext<'_>) {
        let Some(ratio) = self.indicator else {
            return;
        };
        let dot = space::MD;
        let mut point = self.segment(cx.size(), ratio, ratio, dot);
        if self.vertical {
            point.y -= dot / 2.0;
            point.height = dot;
        } else {
            point.x -= dot / 2.0;
            point.width = dot;
        }
        let color = if cx.state().disabled {
            SemanticColorRole::Faint
        } else {
            SemanticColorRole::Text
        };
        cx.rounded_rect(point, dot / 2.0, BoxPaint::fill(color));
    }
}

/// A thumb, styled as a [`crate::RangeField`] thumb, and its focus ring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ThumbPainter {
    pub(crate) extent: f32,
}

impl Painter for ThumbPainter {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        let state = cx.state();
        let bounds = cx.bounds();
        let outset = focus_ring_outset();
        let (fill, border) = if state.disabled {
            (SemanticColorRole::Faint, SemanticColorRole::Border)
        } else if state.hovered || state.pressed {
            (
                SemanticColorRole::AccentStrong,
                SemanticColorRole::BorderStrong,
            )
        } else {
            (SemanticColorRole::Accent, SemanticColorRole::BorderStrong)
        };
        let thumb = LayoutBox {
            x: bounds.x + outset,
            y: bounds.y + outset,
            width: self.extent,
            height: self.extent,
        };
        cx.rounded_rect(
            thumb,
            self.extent / 2.0,
            BoxPaint::fill(fill).border(border, nana_ui_core::HAIRLINE),
        );
        if state.focused {
            cx.rounded_rect(
                bounds,
                bounds.width.max(bounds.height) / 2.0,
                BoxPaint::default().border(SemanticColorRole::FocusBorder, space::XXS),
            );
        }
    }

    fn paint_key(&self) -> u64 {
        u64::from(self.extent.to_bits())
    }
}

/// A value thumb: an accent capsule with the value in it, as a
/// [`crate::RangeField::value_thumb`] draws it, and its focus ring. Not drawn
/// while its value is typed: the number field lies over it.
#[derive(Debug, Clone, PartialEq)]
struct ValueThumbPainter {
    text: Arc<str>,
    font_size: f32,
    font_weight: u16,
    editing: bool,
}

impl Painter for ValueThumbPainter {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        if self.editing {
            return;
        }
        let state = cx.state();
        let bounds = cx.bounds();
        let outset = focus_ring_outset();
        let capsule = LayoutBox {
            x: bounds.x + outset,
            y: bounds.y + outset,
            width: (bounds.width - 2.0 * outset).max(0.0),
            height: (bounds.height - 2.0 * outset).max(0.0),
        };
        // A disabled capsule loses the accent; its text takes the faint
        // colour, as a disabled value thumb's does.
        let (fill, text) = if state.disabled {
            (SemanticColorRole::Border, SemanticColorRole::Faint)
        } else if state.hovered || state.pressed {
            (
                SemanticColorRole::AccentStrong,
                SemanticColorRole::AccentText,
            )
        } else {
            (SemanticColorRole::Accent, SemanticColorRole::AccentText)
        };
        cx.rounded_rect(capsule, capsule.height / 2.0, BoxPaint::fill(fill));
        cx.text(
            capsule,
            PaintText::new(Arc::clone(&self.text))
                .size(self.font_size)
                .weight(self.font_weight)
                .color(text)
                .align(
                    TextHorizontalAlignment::Center,
                    TextVerticalAlignment::Center,
                ),
        );
        if state.focused {
            cx.rounded_rect(
                bounds,
                bounds.height.min(bounds.width) / 2.0,
                BoxPaint::default().border(SemanticColorRole::FocusBorder, space::XXS),
            );
        }
    }

    fn paint_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut hasher);
        self.font_size.to_bits().hash(&mut hasher);
        self.font_weight.hash(&mut hasher);
        self.editing.hash(&mut hasher);
        hasher.finish()
    }
}

impl AppContext {
    /// The field `id` belongs to, and the thumb when `id` is one: what a
    /// press, a key or an assistive action on `id` writes.
    pub fn range_span_target(
        &self,
        id: StableNodeId,
    ) -> Option<(Entity<RangeSpanField>, Option<RangeSpanThumb>)> {
        if let Some(field) = self.view_entity::<RangeSpanField>(id) {
            return Some((field, None));
        }
        let handle = self.view_entity::<RangeSpanHandle>(id)?;
        let thumb = self.read(handle, RangeSpanHandle::thumb).ok()?;
        let field = self.view_entity::<RangeSpanField>(self.world().parent_id(id)?)?;
        Some((field, Some(thumb)))
    }

    /// Commits both thumbs (in order whichever way they arrive): emits
    /// [`RangeSpanInput`] then [`RangeSpanChanged`] when either moved. During
    /// a drag the committed pair is also what a cancel restores.
    pub fn set_range_span(
        &mut self,
        entity: Entity<RangeSpanField>,
        low: f64,
        high: f64,
    ) -> Result<bool, FrameworkError> {
        if !low.is_finite() || !high.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.stable_id()));
        }
        if self.read(entity, |field| field.disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |field, cx| {
            let before = (field.low, field.high);
            field.set_span(low, high);
            if before == (field.low, field.high) {
                return false;
            }
            commit_span(field, cx);
            true
        })
    }

    /// Commits one thumb, held on its side of the other.
    pub fn set_range_span_thumb(
        &mut self,
        entity: Entity<RangeSpanField>,
        thumb: RangeSpanThumb,
        value: f64,
    ) -> Result<bool, FrameworkError> {
        self.write_range_span_thumb(entity, thumb, value, true)
    }

    /// Moves the live indicator. Only the field's paint changes.
    pub fn set_range_span_indicator(
        &mut self,
        entity: Entity<RangeSpanField>,
        indicator: Option<f64>,
    ) -> Result<(), FrameworkError> {
        self.update_component(entity, |field, _| field.indicator = indicator)
    }

    fn write_range_span_thumb(
        &mut self,
        entity: Entity<RangeSpanField>,
        thumb: RangeSpanThumb,
        value: f64,
        commit: bool,
    ) -> Result<bool, FrameworkError> {
        if !value.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.stable_id()));
        }
        if self.read(entity, |field| field.disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |field, cx| {
            let next = field.clamp_thumb(thumb, value);
            if field.value(thumb) == next {
                return false;
            }
            match thumb {
                RangeSpanThumb::Low => field.low = next,
                RangeSpanThumb::High => field.high = next,
            }
            if commit {
                commit_span(field, cx);
            } else {
                cx.emit(RangeSpanInput {
                    low: field.low,
                    high: field.high,
                });
            }
            true
        })
    }

    /// Steps one thumb, as a key on it does.
    pub fn adjust_range_span(
        &mut self,
        entity: Entity<RangeSpanField>,
        thumb: RangeSpanThumb,
        adjustment: RangeAdjustment,
    ) -> Result<bool, FrameworkError> {
        let value = self.read(entity, |field| {
            let value = field.value(thumb);
            match adjustment {
                RangeAdjustment::Decrement => value - field.step,
                RangeAdjustment::Increment => value + field.step,
                RangeAdjustment::PageDecrement => value - field.page_step,
                RangeAdjustment::PageIncrement => value + field.page_step,
                RangeAdjustment::Minimum => field.minimum,
                RangeAdjustment::Maximum => field.maximum,
            }
        })?;
        self.set_range_span_thumb(entity, thumb, value)
    }

    /// Steps the focused thumb, if a range span thumb has focus.
    pub fn adjust_focused_range_span(
        &mut self,
        document: DocumentId,
        adjustment: RangeAdjustment,
    ) -> Result<bool, FrameworkError> {
        let Some((field, Some(thumb))) = self
            .world()
            .focused(document)
            .and_then(|id| self.range_span_target(id))
        else {
            return Ok(false);
        };
        self.adjust_range_span(field, thumb, adjustment)
    }

    /// [`Self::begin_range_span_drag`] at a window point.
    pub(crate) fn begin_range_span_drag_at(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let (x, y) = self
            .world()
            .pointer_layout_position(target, x, y)
            .unwrap_or((x, y));
        self.begin_range_span_drag(document, pointer_id, target, x, y)
    }

    /// [`Self::update_range_span_drag`] at a window point.
    pub(crate) fn update_range_span_drag_at(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let (x, y) = self
            .world()
            .pointer_layout_position(target, x, y)
            .unwrap_or((x, y));
        self.update_range_span_drag(document, pointer_id, x, y)
    }

    /// Starts a drag from a press on a field or one of its thumbs, at `x`,
    /// `y` in layout space. A press on a thumb grabs that thumb where it is;
    /// a press on the track moves the nearer thumb to the pointer. The field
    /// captures the pointer and the moving thumb takes focus.
    pub fn begin_range_span_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some((field, grabbed)) = self.range_span_target(target) else {
            return Ok(false);
        };
        let Some(bounds) = self.world().layout_box(field.stable_id()) else {
            return Ok(false);
        };
        let snapshot = self.read(field, Clone::clone)?;
        if snapshot.disabled {
            return Ok(false);
        }
        let pointer = snapshot.value_at(bounds, x, y);
        let (thumb, grab_offset) = match grabbed {
            Some(grabbed) => {
                let thumb = if snapshot.low != snapshot.high {
                    Some(grabbed)
                } else if snapshot.low <= snapshot.minimum {
                    Some(RangeSpanThumb::High)
                } else if snapshot.high >= snapshot.maximum {
                    Some(RangeSpanThumb::Low)
                } else {
                    None
                };
                (thumb, snapshot.value(grabbed) - pointer)
            }
            None => (snapshot.nearer_thumb(pointer), 0.0),
        };
        let field_id = field.stable_id();
        self.update_component(field, |field, cx| {
            field.dragging = Some(RangeSpanDragState {
                pointer_id,
                thumb,
                grab_offset,
                initial_low: field.low,
                initial_high: field.high,
                grabbed,
                press: (x, y),
                moved: false,
            });
            cx.mutations().capture_pointer(pointer_id, field_id);
            cx.emit(RangeSpanDragging { dragging: true });
        })?;
        self.begin_active_drag(document, pointer_id, field_id, &RANGE_SPAN_DRAG_OPS);
        if let Some(focus) = thumb
            .or(grabbed)
            .and_then(|thumb| snapshot.thumb_node(thumb))
        {
            self.focus_node_in_place(document, focus)?;
        }
        if grabbed.is_none() {
            self.drag_range_span_to(field, pointer)?;
        }
        Ok(true)
    }

    /// Moves the dragged thumb to the pointer at `x`, `y` in layout space.
    pub fn update_range_span_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(field) = self.view_entity::<RangeSpanField>(target) else {
            return Ok(false);
        };
        let Some(bounds) = self.world().layout_box(target) else {
            return Ok(false);
        };
        let Some((pointer, offset, moved)) = self.read(field, |field| {
            let drag = field
                .dragging
                .filter(|drag| drag.pointer_id == pointer_id)?;
            let (press_x, press_y) = drag.press;
            let moved = drag.moved || (x - press_x).hypot(y - press_y) > CLICK_SLOP;
            Some((field.value_at(bounds, x, y), drag.grab_offset, moved))
        })?
        else {
            return Ok(false);
        };
        self.update_component(field, |field, _| {
            if let Some(drag) = field.dragging.as_mut() {
                drag.moved = moved;
            }
        })?;
        // A value thumb stays put until the press is a drag: a click on it
        // types a value instead.
        if !moved && self.read(field, |field| field.value_capsule().is_some())? {
            return Ok(false);
        }
        self.drag_range_span_to(field, pointer + offset)
    }

    /// Moves the dragged thumb to `value`, first picking it if the press
    /// landed on two thumbs sitting together.
    fn drag_range_span_to(
        &mut self,
        field: Entity<RangeSpanField>,
        value: f64,
    ) -> Result<bool, FrameworkError> {
        let Some((thumb, picked, node)) = self.read(field, |field| {
            let drag = field.dragging?;
            let thumb = drag.thumb.or_else(|| field.nearer_thumb(value))?;
            Some((thumb, drag.thumb.is_none(), field.thumb_node(thumb)))
        })?
        else {
            return Ok(false);
        };
        if picked {
            self.update_component(field, |field, _| {
                if let Some(drag) = field.dragging.as_mut() {
                    drag.thumb = Some(thumb);
                }
            })?;
            if let (Some(document), Some(node)) =
                (self.world().document_of(field.stable_id()), node)
            {
                self.focus_node_in_place(document, node)?;
            }
        }
        self.write_range_span_thumb(field, thumb, value, false)
    }

    /// Ends the drag `pointer_id` holds. Release commits with
    /// [`RangeSpanChanged`] when a thumb moved; cancel, or a field disabled
    /// mid-drag, restores the pair the drag started from and commits nothing.
    pub fn end_range_span_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        self.end_range_span_drag_on(target, pointer_id, cancel)
    }

    pub(crate) fn end_range_span_drag_on(
        &mut self,
        target: StableNodeId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(field) = self.view_entity::<RangeSpanField>(target) else {
            return Ok(false);
        };
        let document = self.world().document_of(target);
        let held = document.and_then(|document| self.world().pointer_capture(document, pointer_id))
            == Some(target);
        if let Some(document) = document {
            self.clear_active_drag(document, pointer_id, target);
        }
        let mut clicked = None;
        let ended = self.update_component(field, |field, cx| {
            if held {
                cx.mutations().release_pointer(pointer_id, target);
            }
            let Some(drag) = field.dragging.take_if(|drag| drag.pointer_id == pointer_id) else {
                return false;
            };
            if !cancel && !drag.moved && field.value_capsule().is_some() {
                clicked = drag.grabbed;
            }
            if (field.low, field.high) != (drag.initial_low, drag.initial_high) {
                if cancel || field.disabled {
                    field.low = drag.initial_low;
                    field.high = drag.initial_high;
                    cx.emit(RangeSpanInput {
                        low: field.low,
                        high: field.high,
                    });
                } else {
                    cx.emit(RangeSpanChanged {
                        low: field.low,
                        high: field.high,
                    });
                }
            }
            cx.emit(RangeSpanDragging { dragging: false });
            true
        })?;
        if let Some(thumb) = clicked {
            // A click on a value thumb types a value.
            self.begin_range_span_edit_focus(field, thumb, false)?;
        }
        Ok(ended)
    }

    /// Lifecycle: create the two thumb nodes (and, with value thumbs, the
    /// number field typing uses) once, then keep them on the field's props.
    pub(crate) fn sync_range_span(
        &mut self,
        field: Entity<RangeSpanField>,
    ) -> Result<(), FrameworkError> {
        let capsule = self.measure_value_capsule(field)?;
        if self.read(field, |field| {
            field.value_thumbs && field.capsule != capsule
        })? {
            // Writing the measure runs this upkeep again, with it in place.
            return self.update_component(field, |field, _| field.capsule = capsule);
        }
        let snapshot = self.read(field, Clone::clone)?;
        let document = self
            .world()
            .node(field.stable_id())
            .ok_or(FrameworkError::MissingView(field.stable_id()))?
            .document;
        let assembled = snapshot.thumbs.filter(|thumbs| {
            thumbs.iter().all(|id| {
                self.world().parent_id(*id) == Some(field.stable_id())
                    && self.view_entity::<RangeSpanHandle>(*id).is_some()
            })
        });
        let Some([low, high]) = assembled else {
            let low =
                self.create_detached_component(document, snapshot.handle(RangeSpanThumb::Low))?;
            let high =
                self.create_detached_component(document, snapshot.handle(RangeSpanThumb::High))?;
            let thumbs = [low.stable_id(), high.stable_id()];
            self.append_children(field.stable_id(), &thumbs)?;
            return self.update_component(field, |field, _| field.thumbs = Some(thumbs));
        };
        for (thumb, id) in [(RangeSpanThumb::Low, low), (RangeSpanThumb::High, high)] {
            let next = snapshot.handle(thumb);
            self.update_component(
                Entity::<RangeSpanHandle>::from_stable_id(id),
                |handle, _| {
                    if *handle != next {
                        *handle = next;
                    }
                },
            )?;
        }
        if !snapshot.value_thumbs {
            if let Some(input) = snapshot.input {
                self.update_component(Entity::<NumberInput>::from_stable_id(input), |input, _| {
                    Arc::make_mut(&mut input.style.layout).hidden = true;
                })?;
            }
            return Ok(());
        }
        let input = match snapshot.input.filter(|id| self.world().contains(*id)) {
            Some(input) => input,
            None => {
                let input =
                    self.create_detached_component(document, NumberInput::continuous(0.0))?;
                self.observe(input, field, |field, event: &NumberChanged, cx| {
                    let Some(thumb) = field.editing else {
                        return;
                    };
                    if field.disabled {
                        return;
                    }
                    let value = field.quantize(event.value);
                    cx.emit(RangeSpanEntered { thumb, value });
                    let before = (field.low, field.high);
                    field.set_span(value, field.value(thumb.other()));
                    if before != (field.low, field.high) {
                        commit_span(field, cx);
                    }
                })?;
                // Typing ends when the focus leaves the field.
                self.on(field, |field, event: &FocusWithinChanged, _| {
                    if !event.focused {
                        field.editing = None;
                    }
                })?;
                self.append_children(field.stable_id(), &[low, high, input.stable_id()])?;
                let id = input.stable_id();
                return self.update_component(field, |field, _| field.input = Some(id));
            }
        };
        let theme = self.world().theme();
        let field_height = snapshot.size.height_in(theme.metrics());
        let spacing = theme.spacing();
        let edited = snapshot.editing.filter(|_| !snapshot.disabled);
        let placement = edited
            .zip(snapshot.value_capsule())
            .map(|(thumb, capsule)| {
                // Over the capsule, a little wider, as a range's typing is.
                let width = capsule.width + spacing.md + spacing.md;
                let (along, girth) = match snapshot.orientation {
                    RangeSpanOrientation::Horizontal => (width, field_height),
                    RangeSpanOrientation::Vertical => (field_height, width),
                };
                let travel = capsule.extents(snapshot.orientation).0 + 2.0 * focus_ring_outset();
                let ratio = snapshot.ratio(snapshot.value(thumb));
                // Centred where the thumb is: its offsets, moved by half the
                // difference between the field and the thumb.
                let (left, top) = thumb_offsets(snapshot.orientation, ratio, travel, girth);
                let shift = (travel - along) / 2.0;
                let shifted = |offset: LengthSpec| match offset.as_percent_offset() {
                    Some((percent, offset_px)) => {
                        LengthSpec::percent_offset_inline(percent, offset_px + shift)
                    }
                    None => offset,
                };
                let (left, top) = match snapshot.orientation {
                    RangeSpanOrientation::Horizontal => (shifted(left), top),
                    RangeSpanOrientation::Vertical => (left, shifted(top)),
                };
                (left, top, width, snapshot.handle(thumb).label)
            });
        let spec = NumberFieldSpec {
            minimum: Some(snapshot.minimum),
            maximum: Some(snapshot.maximum),
            step: snapshot.step,
            precision: format_range_value(snapshot.step, snapshot.step)
                .split_once('.')
                .map_or(0, |(_, decimals)| decimals.len() as u8),
        };
        self.update_component(Entity::<NumberInput>::from_stable_id(input), |input, _| {
            input.spec = spec;
            if input.size != snapshot.size {
                // The builder's size writes the style the field is drawn with.
                *input = std::mem::replace(input, NumberInput::continuous(0.0)).size(snapshot.size);
            }
            input.steppers = false;
            input.disabled = snapshot.disabled;
            let layout = Arc::make_mut(&mut input.style.layout);
            layout.hidden = placement.is_none();
            layout.position = PositionSpec::Absolute;
            if let Some((left, top, width, label)) = placement.clone() {
                layout.offset_left = Some(left);
                layout.offset_top = Some(top);
                layout.width = Some(LengthSpec::Px(width));
                // Held to the height it is centred by.
                layout.height = Some(LengthSpec::Px(field_height));
                input.label = label;
            }
        })?;
        Ok(())
    }

    /// The value capsules of `field`, measured on its own text: as wide as
    /// the widest of its bounds and its thumbs' values. `None` without value
    /// thumbs.
    fn measure_value_capsule(
        &self,
        field: Entity<RangeSpanField>,
    ) -> Result<Option<ValueCapsule>, FrameworkError> {
        let snapshot = self.read(field, Clone::clone)?;
        if !snapshot.value_thumbs {
            return Ok(None);
        }
        let theme = self.world().theme();
        let (typography, spacing) = (theme.typography(), theme.spacing());
        let font_size = snapshot.size.text_size_in(typography);
        let font_weight = typography.medium;
        let measure = self.world().chrome_text_measure(field.stable_id());
        let text = [
            snapshot.minimum,
            snapshot.maximum,
            snapshot.low,
            snapshot.high,
        ]
        .into_iter()
        .map(|value| {
            measure.tabular_width(
                &format_range_value(value, snapshot.step),
                font_size,
                Some(font_weight),
            )
        })
        .fold(0.0, f32::max);
        Ok(Some(ValueCapsule {
            width: (text + spacing.md + spacing.md).max(spacing.page + spacing.page),
            height: range_span_value_thumb_height(snapshot.size, typography, spacing),
            font_size,
            font_weight,
        }))
    }

    /// Start typing a value for `thumb`: its capsule makes way for the
    /// number field, which shows the value, selected, with the focus.
    /// Refused unless the field draws value thumbs and is enabled.
    pub fn begin_range_span_edit(
        &mut self,
        field: Entity<RangeSpanField>,
        thumb: RangeSpanThumb,
    ) -> Result<bool, FrameworkError> {
        self.begin_range_span_edit_focus(field, thumb, true)
    }

    fn begin_range_span_edit_focus(
        &mut self,
        field: Entity<RangeSpanField>,
        thumb: RangeSpanThumb,
        reveal: bool,
    ) -> Result<bool, FrameworkError> {
        let document = self.document_of_node(field.stable_id())?;
        let Some(value) = self.update_component(field, |field, _| {
            if field.disabled || field.value_capsule().is_none() {
                return None;
            }
            field.editing = Some(thumb);
            Some(field.value(thumb))
        })?
        else {
            return Ok(false);
        };
        let Some(input) = self.read(field, |field| field.input)? else {
            return Ok(false);
        };
        self.update_component(Entity::<NumberInput>::from_stable_id(input), |input, _| {
            input.assign(value);
        })?;
        let moved = self.focus_node_in_place(document, input)?;
        if reveal && moved {
            let target = self
                .read(field, |field| field.thumb_node(thumb))?
                .filter(|node| !self.has_empty_layout_box(*node))
                .unwrap_or(field.stable_id());
            self.reveal_focused_target(target)?;
        }
        self.select_all_focused_text(document)?;
        Ok(true)
    }

    /// Leave typing on the field whose number field has the focus (Enter
    /// committed it, Escape reverted it): the typed thumb takes the focus
    /// back.
    pub(crate) fn end_focused_range_span_edit(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(field) = self
            .world()
            .focused(document)
            .and_then(|input| self.world().parent_id(input))
            .and_then(|parent| self.view_entity::<RangeSpanField>(parent))
        else {
            return Ok(false);
        };
        let Some(thumb) = self.update_component(field, |field, _| field.editing.take())? else {
            return Ok(false);
        };
        if let Some(node) = self.read(field, |field| field.thumb_node(thumb))? {
            self.focus_node_in_place(document, node)?;
        }
        Ok(true)
    }

    /// Enter, Space or an assistive press on a value thumb types a value;
    /// a plain thumb has no activation.
    pub(crate) fn activate_range_span_handle(
        &mut self,
        handle: Entity<RangeSpanHandle>,
    ) -> Result<bool, FrameworkError> {
        let Some((field, Some(thumb))) = self.range_span_target(handle.stable_id()) else {
            return Ok(false);
        };
        self.begin_range_span_edit(field, thumb)
    }
}

/// Emits the pair as shown and as committed.
fn commit_span(field: &mut RangeSpanField, cx: &mut crate::ViewContext<'_, RangeSpanField>) {
    if let Some(drag) = field.dragging.as_mut() {
        drag.initial_low = field.low;
        drag.initial_high = field.high;
    }
    cx.emit(RangeSpanInput {
        low: field.low,
        high: field.high,
    });
    cx.emit(RangeSpanChanged {
        low: field.low,
        high: field.high,
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nana_ui_input::{InputModifiers, KeyInput, KeyState, PointerPhase};

    use super::*;
    use crate::{
        AccessibilityAction, AccessibilityActionRequest, HeadlessInput, LayoutViewport, Stack,
    };

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Seen {
        Input(f64, f64),
        Changed(f64, f64),
        Dragging(bool),
    }

    /// A field laid out alone in a 300 x 200 document at the origin.
    fn mounted(
        field: RangeSpanField,
    ) -> (AppContext, Entity<RangeSpanField>, Arc<Mutex<Vec<Seen>>>) {
        let mut context = AppContext::new();
        let field = context.create_component(document(), field).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        context
            .on(field, move |_, event: &RangeSpanInput, _| {
                log.lock().unwrap().push(Seen::Input(event.low, event.high));
            })
            .unwrap();
        let log = Arc::clone(&seen);
        context
            .on(field, move |_, event: &RangeSpanChanged, _| {
                log.lock()
                    .unwrap()
                    .push(Seen::Changed(event.low, event.high));
            })
            .unwrap();
        let log = Arc::clone(&seen);
        context
            .on(field, move |_, event: &RangeSpanDragging, _| {
                log.lock().unwrap().push(Seen::Dragging(event.dragging));
            })
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(300.0, 200.0))
            .unwrap();
        context.rebuild_hit_test(document());
        (context, field, seen)
    }

    fn pair(context: &AppContext, field: Entity<RangeSpanField>) -> (f64, f64) {
        context
            .read(field, |field| (field.low, field.high))
            .unwrap()
    }

    fn thumb(
        context: &AppContext,
        field: Entity<RangeSpanField>,
        thumb: RangeSpanThumb,
    ) -> StableNodeId {
        context
            .read(field, |field| field.thumb_node(thumb))
            .unwrap()
            .expect("thumbs are assembled with the field")
    }

    fn centre(context: &AppContext, id: StableNodeId) -> (f32, f32) {
        let bounds = context.world().layout_box(id).unwrap();
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    }

    fn key(input: &mut HeadlessInput, context: &mut AppContext, name: &'static str) {
        input
            .press(
                context,
                KeyInput::named(name, name, KeyState::Pressed, InputModifiers::default()),
                None,
                None,
            )
            .unwrap();
    }

    fn vertical_field() -> RangeSpanField {
        let mut field = RangeSpanField::new(0.2, 0.6, 0.0, 1.0, 0.1).vertical();
        Arc::make_mut(&mut field.style.layout).height = Some(LengthSpec::Px(180.0));
        field
    }

    #[test]
    fn construction_repairs_bounds_orders_the_thumbs_and_snaps_to_the_step() {
        let field = RangeSpanField::new(0.83, 0.12, 0.0, 1.0, 0.25);
        assert_eq!((field.low, field.high), (0.0, 0.75));
        let field = RangeSpanField::new(f64::NAN, f64::INFINITY, 5.0, 1.0, -1.0);
        assert_eq!((field.minimum, field.maximum, field.step), (0.0, 1.0, 0.01));
        assert_eq!((field.low, field.high), (0.0, 1.0));
        let field = RangeSpanField::new(-3.0, 9.0, 0.0, 1.0, 0.1);
        assert_eq!((field.low, field.high), (0.0, 1.0));
    }

    #[test]
    fn a_thumb_stops_at_the_other_one_and_snaps_to_the_step() {
        let field = RangeSpanField::new(0.2, 0.6, 0.0, 1.0, 0.1);
        assert_eq!(field.clamp_thumb(RangeSpanThumb::Low, 0.9), field.high);
        assert_eq!(field.clamp_thumb(RangeSpanThumb::High, 0.04), field.low);
        assert!((field.clamp_thumb(RangeSpanThumb::Low, 0.33) - 0.3).abs() < 1e-12);
        assert_eq!(field.clamp_thumb(RangeSpanThumb::High, 7.0), 1.0);
    }

    #[test]
    fn an_application_write_pushes_the_other_thumb_so_binding_order_does_not_matter() {
        let start = RangeSpanField::new(0.2, 0.4, 0.0, 1.0, 0.1);
        let mut low_first = start.clone();
        low_first.assign(RangeSpanThumb::Low, 0.7);
        low_first.assign(RangeSpanThumb::High, 0.9);
        let mut high_first = start.clone();
        high_first.assign(RangeSpanThumb::High, 0.9);
        high_first.assign(RangeSpanThumb::Low, 0.7);
        assert_eq!(
            (low_first.low, low_first.high),
            (high_first.low, high_first.high)
        );
        assert!((low_first.low - 0.7).abs() < 1e-12 && (low_first.high - 0.9).abs() < 1e-12);
    }

    #[test]
    fn pointer_values_map_through_the_inset_and_vertical_puts_the_minimum_at_the_bottom() {
        let horizontal = RangeSpanField::new(0.0, 1.0, 0.0, 100.0, 1.0);
        let inset = horizontal.track_inset();
        assert_eq!(inset, range_span_track_inset(ControlSize::Medium));
        assert_eq!(
            inset,
            range_span_thumb_extent(ControlSize::Medium) / 2.0 + 3.0
        );
        let bounds = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 200.0 + 2.0 * inset,
            height: 200.0 + 2.0 * inset,
        };
        assert_eq!(horizontal.value_at(bounds, 10.0 + inset, 0.0), 0.0);
        assert_eq!(horizontal.value_at(bounds, 10.0 + inset + 50.0, 0.0), 25.0);
        assert_eq!(horizontal.value_at(bounds, 0.0, 0.0), 0.0);
        assert_eq!(horizontal.value_at(bounds, 999.0, 0.0), 100.0);

        let vertical = horizontal.clone().vertical();
        let bottom = bounds.y + bounds.height - inset;
        assert_eq!(vertical.value_at(bounds, 0.0, bottom), 0.0);
        assert_eq!(vertical.value_at(bounds, 0.0, bottom - 150.0), 75.0);
        assert_eq!(vertical.value_at(bounds, 0.0, bounds.y), 100.0);
    }

    #[test]
    fn a_press_between_the_thumbs_takes_the_nearer_one() {
        let field = RangeSpanField::new(0.2, 0.6, 0.0, 1.0, 0.1);
        assert_eq!(field.nearer_thumb(0.1), Some(RangeSpanThumb::Low));
        assert_eq!(field.nearer_thumb(0.35), Some(RangeSpanThumb::Low));
        assert_eq!(field.nearer_thumb(0.45), Some(RangeSpanThumb::High));
        assert_eq!(field.nearer_thumb(0.9), Some(RangeSpanThumb::High));
        let together = RangeSpanField::new(0.5, 0.5, 0.0, 1.0, 0.1);
        assert_eq!(together.nearer_thumb(0.5), None);
        assert_eq!(together.nearer_thumb(0.4), Some(RangeSpanThumb::Low));
    }

    #[test]
    fn thumbs_are_slider_nodes_placed_on_the_track_low_first() {
        let (context, field, _) = mounted(
            vertical_field()
                .low_label("输入下限")
                .high_label("输入上限"),
        );
        let bounds = context.world().layout_box(field.stable_id()).unwrap();
        let inset = range_span_track_inset(ControlSize::Medium);
        assert_eq!(bounds.height, 180.0);
        assert_eq!(bounds.width, inset * 2.0);
        let low = thumb(&context, field, RangeSpanThumb::Low);
        let high = thumb(&context, field, RangeSpanThumb::High);
        assert_eq!(
            context.world().node(field.stable_id()).unwrap().children,
            vec![low, high]
        );
        let length = bounds.height - 2.0 * inset;
        let (x, y) = centre(&context, low);
        assert!((x - (bounds.x + inset)).abs() < 0.01);
        assert!(
            (y - (bounds.y + bounds.height - inset - 0.2 * length)).abs() < 0.01,
            "{y}"
        );
        let (_, y) = centre(&context, high);
        assert!(
            (y - (bounds.y + bounds.height - inset - 0.6 * length)).abs() < 0.01,
            "{y}"
        );

        let low = context.world().accessibility(low).unwrap();
        assert_eq!(low.role, AccessibilityRole::Slider);
        assert_eq!(low.label.as_deref(), Some("输入下限"));
        assert_eq!(low.orientation, Some(SelectionOrientation::Vertical));
        assert_eq!(
            (
                low.numeric_minimum,
                low.numeric_maximum,
                low.numeric_step,
                low.numeric_value
            ),
            (Some(0.0), Some(0.6), Some(0.1), Some(0.2))
        );
        assert_eq!(low.value.as_deref(), Some("0.2"));
        let high = context.world().accessibility(high).unwrap();
        assert_eq!(high.label.as_deref(), Some("输入上限"));
        assert_eq!(
            (
                high.numeric_minimum,
                high.numeric_maximum,
                high.numeric_value
            ),
            (Some(0.2), Some(1.0), Some(0.6))
        );
    }

    fn value_field() -> RangeSpanField {
        let mut field = RangeSpanField::new(-10.0, 20.0, -30.0, 30.0, 1.0)
            .vertical()
            .size(ControlSize::Small)
            .value_thumbs(true)
            .low_label("下端")
            .high_label("上端");
        Arc::make_mut(&mut field.style.layout).height = Some(LengthSpec::Px(180.0));
        field
    }

    fn relayout(context: &mut AppContext) {
        context
            .layout_document(document(), LayoutViewport::new(300.0, 200.0))
            .unwrap();
        context.rebuild_hit_test(document());
    }

    fn typed(input: &mut HeadlessInput, context: &mut AppContext, text: &str) {
        input
            .route(
                context,
                nana_ui_input::InputPayload::Text(nana_ui_input::CommittedText {
                    text: text.to_owned(),
                    key: None,
                }),
            )
            .unwrap();
    }

    #[test]
    fn value_thumbs_are_capsules_as_wide_as_the_widest_value_and_set_the_inset() {
        let (mut context, field, _) = mounted(value_field());
        relayout(&mut context);
        let capsule = context.read(field, |field| field.capsule).unwrap().unwrap();
        let theme = context.world().theme();
        let height =
            range_span_value_thumb_height(ControlSize::Small, theme.typography(), theme.spacing());
        assert_eq!(capsule.height, height);
        let inset =
            range_span_value_track_inset(ControlSize::Small, theme.typography(), theme.spacing());
        assert_eq!(
            context.read(field, RangeSpanField::track_inset).unwrap(),
            inset
        );
        let bounds = context.world().layout_box(field.stable_id()).unwrap();
        // The field is as wide as a capsule and its ring.
        assert_eq!(bounds.width, capsule.width + 2.0 * focus_ring_outset());
        let low = thumb(&context, field, RangeSpanThumb::Low);
        let thumb_box = context.world().layout_box(low).unwrap();
        assert_eq!(thumb_box.height, height + 2.0 * focus_ring_outset());
        let length = bounds.height - 2.0 * inset;
        let (_, y) = centre(&context, low);
        let expected = bounds.y + bounds.height - inset - (20.0 / 60.0) * length;
        assert!((y - expected).abs() < 0.01, "{y} vs {expected}");
    }

    #[test]
    fn a_click_on_a_value_thumb_types_and_a_value_past_the_other_thumb_takes_its_place() {
        let (mut context, field, seen) = mounted(value_field());
        relayout(&mut context);
        let entered = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&entered);
        context
            .on(field, move |_, event: &RangeSpanEntered, _| {
                log.lock().unwrap().push((event.thumb, event.value));
            })
            .unwrap();
        let low = thumb(&context, field, RangeSpanThumb::Low);
        let (x, y) = centre(&context, low);
        let mut input = HeadlessInput::bind(&mut context, document());
        // A press that wanders less than the slop is a click: nothing moves.
        input
            .pointer(&mut context, PointerPhase::Down, x, y)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, x, y - 2.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Up, x, y - 2.0)
            .unwrap();
        assert_eq!(pair(&context, field), (-10.0, 20.0));
        assert_eq!(
            context.read(field, |field| field.editing).unwrap(),
            Some(RangeSpanThumb::Low)
        );
        let number = context.read(field, |field| field.input).unwrap().unwrap();
        assert_eq!(context.world().focused(document()), Some(number));
        relayout(&mut context);
        let over = context.world().layout_box(number).unwrap();
        assert!(
            (over.y + over.height / 2.0 - y).abs() < 1.0,
            "the number field lies over the thumb: {over:?} at {y}"
        );
        seen.lock().unwrap().clear();
        typed(&mut input, &mut context, "25");
        key(&mut input, &mut context, "Enter");
        assert_eq!(*entered.lock().unwrap(), vec![(RangeSpanThumb::Low, 25.0)]);
        assert_eq!(pair(&context, field), (20.0, 25.0));
        assert_eq!(
            *seen.lock().unwrap(),
            vec![Seen::Input(20.0, 25.0), Seen::Changed(20.0, 25.0)]
        );
        assert_eq!(context.read(field, |field| field.editing).unwrap(), None);
        assert_eq!(context.world().focused(document()), Some(low));

        // Escape leaves typing and changes nothing.
        key(&mut input, &mut context, "Enter");
        assert!(
            context
                .read(field, |field| field.editing)
                .unwrap()
                .is_some()
        );
        typed(&mut input, &mut context, "0");
        key(&mut input, &mut context, "Escape");
        assert_eq!(pair(&context, field), (20.0, 25.0));
        assert_eq!(context.read(field, |field| field.editing).unwrap(), None);
    }

    #[test]
    fn a_value_thumb_drags_once_the_press_has_moved() {
        let (mut context, field, seen) = mounted(value_field());
        relayout(&mut context);
        let high = thumb(&context, field, RangeSpanThumb::High);
        let (x, y) = centre(&context, high);
        let mut input = HeadlessInput::bind(&mut context, document());
        input
            .pointer(&mut context, PointerPhase::Down, x, y)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, x, y - 30.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Up, x, y - 30.0)
            .unwrap();
        let (low, moved) = pair(&context, field);
        assert_eq!(low, -10.0);
        assert!(moved > 20.0, "{moved}");
        assert_eq!(context.read(field, |field| field.editing).unwrap(), None);
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, Seen::Changed(_, high) if *high == moved))
        );
    }

    #[test]
    fn a_press_on_a_thumb_grabs_it_captures_and_commits_once_on_release() {
        let (mut context, field, seen) = mounted(vertical_field());
        let high = thumb(&context, field, RangeSpanThumb::High);
        let (x, y) = centre(&context, high);
        let inset = range_span_track_inset(ControlSize::Medium);
        let tenth = (180.0 - 2.0 * inset) / 10.0;
        let mut input = HeadlessInput::bind(&mut context, document());
        // Off the thumb's centre: grabbing does not jump it to the pointer.
        input
            .pointer(&mut context, PointerPhase::Down, x, y + 3.0)
            .unwrap();
        assert_eq!(
            context.world().pointer_capture(document(), 1),
            Some(field.stable_id())
        );
        assert_eq!(context.world().focused(document()), Some(high));
        assert_eq!(pair(&context, field), (0.2, 0.6));
        input
            .pointer(&mut context, PointerPhase::Move, x, y + 3.0 - 2.0 * tenth)
            .unwrap();
        assert_eq!(pair(&context, field), (0.2, 0.8));
        // Dragged past the low thumb, it stops there.
        input
            .pointer(&mut context, PointerPhase::Move, x, 199.0)
            .unwrap();
        assert_eq!(pair(&context, field), (0.2, 0.2));
        input
            .pointer(&mut context, PointerPhase::Move, x, y + 3.0 - tenth)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Up, x, y + 3.0 - tenth)
            .unwrap();
        let (low, high) = pair(&context, field);
        assert_eq!(low, 0.2);
        assert!((high - 0.7).abs() < 1e-9);
        assert_eq!(context.world().pointer_capture(document(), 1), None);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first(), Some(&Seen::Dragging(true)));
        assert_eq!(seen.last(), Some(&Seen::Dragging(false)));
        let commits: Vec<_> = seen
            .iter()
            .filter(|event| matches!(event, Seen::Changed(..)))
            .collect();
        assert_eq!(commits, vec![&Seen::Changed(0.2, high)]);
    }

    #[test]
    fn a_press_on_the_track_moves_the_nearer_thumb_and_a_cancel_restores() {
        let (mut context, field, seen) = mounted(vertical_field());
        let bounds = context.world().layout_box(field.stable_id()).unwrap();
        let inset = range_span_track_inset(ControlSize::Medium);
        let length = bounds.height - 2.0 * inset;
        let at = |ratio: f32| bounds.y + bounds.height - inset - ratio * length;
        let x = bounds.x + 1.0;
        let mut input = HeadlessInput::bind(&mut context, document());
        input
            .pointer(&mut context, PointerPhase::Down, x, at(0.9))
            .unwrap();
        let (low, high) = pair(&context, field);
        assert_eq!(low, 0.2);
        assert!((high - 0.9).abs() < 1e-9);
        assert_eq!(
            context.world().focused(document()),
            Some(thumb(&context, field, RangeSpanThumb::High))
        );
        input
            .pointer(&mut context, PointerPhase::Cancel, x, at(0.9))
            .unwrap();
        assert_eq!(pair(&context, field), (0.2, 0.6));
        assert_eq!(context.world().pointer_capture(document(), 1), None);
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, Seen::Changed(..))),
            "a cancelled drag commits nothing"
        );

        input
            .pointer(&mut context, PointerPhase::Down, x, at(0.0))
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Up, x, at(0.0))
            .unwrap();
        assert_eq!(pair(&context, field), (0.0, 0.6));
    }

    #[test]
    fn thumbs_sitting_together_follow_the_first_movement() {
        let mut field = RangeSpanField::new(0.5, 0.5, 0.0, 1.0, 0.1);
        Arc::make_mut(&mut field.style.layout).width = Some(LengthSpec::Px(220.0));
        let (mut context, field, _) = mounted(field);
        let high = thumb(&context, field, RangeSpanThumb::High);
        let (x, y) = centre(&context, high);
        let tenth = (220.0 - 2.0 * range_span_track_inset(ControlSize::Medium)) / 10.0;
        let mut input = HeadlessInput::bind(&mut context, document());
        input
            .pointer(&mut context, PointerPhase::Down, x, y)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, x - 2.0 * tenth, y)
            .unwrap();
        let (low, high_value) = pair(&context, field);
        assert!(
            (low - 0.3).abs() < 1e-9 && high_value == 0.5,
            "{low} {high_value}"
        );
        assert_eq!(
            context.world().focused(document()),
            Some(thumb(&context, field, RangeSpanThumb::Low))
        );
        input
            .pointer(&mut context, PointerPhase::Up, x - 2.0 * tenth, y)
            .unwrap();
    }

    #[test]
    fn tab_walks_low_then_high_and_keys_step_the_focused_thumb() {
        let (mut context, field, seen) = mounted(vertical_field());
        let low = thumb(&context, field, RangeSpanThumb::Low);
        let high = thumb(&context, field, RangeSpanThumb::High);
        let mut input = HeadlessInput::bind(&mut context, document());
        key(&mut input, &mut context, "Tab");
        assert_eq!(context.world().focused(document()), Some(low));
        key(&mut input, &mut context, "ArrowUp");
        let (value, _) = pair(&context, field);
        assert!((value - 0.3).abs() < 1e-9);
        key(&mut input, &mut context, "End");
        assert_eq!(
            pair(&context, field),
            (0.6, 0.6),
            "the low thumb stops at the high one"
        );
        key(&mut input, &mut context, "Home");
        assert_eq!(pair(&context, field), (0.0, 0.6));

        key(&mut input, &mut context, "Tab");
        assert_eq!(context.world().focused(document()), Some(high));
        key(&mut input, &mut context, "PageUp");
        assert_eq!(pair(&context, field), (0.0, 1.0));
        key(&mut input, &mut context, "ArrowDown");
        let (_, value) = pair(&context, field);
        assert!((value - 0.9).abs() < 1e-9);
        assert_eq!(
            seen.lock().unwrap().last(),
            Some(&Seen::Changed(0.0, value)),
            "a key step is a commit"
        );
    }

    #[test]
    fn assistive_actions_step_and_set_one_thumb() {
        let (mut context, field, _) = mounted(RangeSpanField::new(0.2, 0.6, 0.0, 1.0, 0.1));
        let low = thumb(&context, field, RangeSpanThumb::Low);
        let high = thumb(&context, field, RangeSpanThumb::High);
        let act = |context: &mut AppContext, target, action| {
            context
                .apply_accessibility_action(
                    document(),
                    AccessibilityActionRequest { target, action },
                )
                .unwrap()
        };
        assert!(act(&mut context, low, AccessibilityAction::Increment));
        assert!((pair(&context, field).0 - 0.3).abs() < 1e-9);
        assert!(act(&mut context, high, AccessibilityAction::Decrement));
        assert!((pair(&context, field).1 - 0.5).abs() < 1e-9);
        assert!(act(
            &mut context,
            high,
            AccessibilityAction::SetValue("0.9".into())
        ));
        assert!((pair(&context, field).1 - 0.9).abs() < 1e-9);
        assert!(!act(
            &mut context,
            low,
            AccessibilityAction::SetValue("many".into())
        ));
        assert!(act(
            &mut context,
            low,
            AccessibilityAction::SetValue("2".into())
        ));
        assert!(
            (pair(&context, field).0 - 0.9).abs() < 1e-9,
            "held at the high thumb"
        );
        let state = context.world().accessibility(high).unwrap();
        assert_eq!(state.numeric_minimum, pair(&context, field).0.into());
    }

    #[test]
    fn moving_the_indicator_repaints_the_field_and_nothing_else() {
        let (mut context, field, seen) = mounted(vertical_field().indicator(Some(0.4)));
        let _ = context.take_system_work();
        let before = context
            .world()
            .node_style(field.stable_id())
            .unwrap()
            .painter
            .clone();
        context.set_range_span_indicator(field, Some(0.45)).unwrap();
        let work = context.take_system_work();
        assert!(
            work.layout_frontier_seeds.is_empty(),
            "{:?}",
            work.layout_frontier_seeds
        );
        assert_eq!(work.render_extraction, vec![field.stable_id()]);
        assert!(work.accessibility.is_empty());
        assert_ne!(
            context
                .world()
                .node_style(field.stable_id())
                .unwrap()
                .painter,
            before,
            "a new paint key"
        );
        assert!(
            seen.lock().unwrap().is_empty(),
            "the indicator is not a value"
        );
    }

    #[test]
    fn the_span_is_a_thin_accent_bar_between_neutral_track_ends() {
        use crate::custom_paint::{PaintOp, ResolvedPaint, record};
        let theme = nana_ui_core::builtin_theme_arc(nana_ui_core::ThemeAppearance::Light);
        let painter = TrackPainter {
            vertical: false,
            inset: 10.0,
            low: 0.25,
            high: 0.75,
            indicator: None,
            under_thumbs: false,
        };
        let bars = |disabled: bool| {
            let state = crate::PaintState {
                disabled,
                ..crate::PaintState::default()
            };
            record(
                &painter,
                theme.as_ref(),
                [220.0, 32.0],
                state,
                &crate::custom_paint::EmPaintTextHost,
            )
            .behind_children
            .iter()
            .filter_map(|op| match op {
                PaintOp::RoundedRect {
                    rect,
                    fill: Some(ResolvedPaint::Solid(color)),
                    ..
                } => Some((*rect, *color)),
                _ => None,
            })
            .collect::<Vec<_>>()
        };
        let color = |role| theme.style_model().color(role).as_rgba_array();
        let drawn = bars(false);
        assert_eq!(drawn.len(), 3);
        let (before, span, after) = (drawn[0], drawn[1], drawn[2]);
        assert_eq!(before.1, color(SemanticColorRole::BorderStrong));
        assert_eq!(after.1, color(SemanticColorRole::BorderStrong));
        assert_eq!(span.1, color(SemanticColorRole::Accent));
        assert!((span.0.height - before.0.height * 2.0 / 3.0).abs() < 1e-4);
        let centre = |rect: LayoutBox| rect.y + rect.height / 2.0;
        assert!((centre(span.0) - centre(before.0)).abs() < 1e-4);
        assert!((span.0.x - (before.0.x + before.0.width)).abs() < 1e-4);
        let disabled = bars(true);
        assert_eq!(disabled[1].1, color(SemanticColorRole::Faint));
        assert_eq!(disabled[0].1, color(SemanticColorRole::Border));
    }

    #[test]
    fn disabling_takes_the_thumbs_out_of_focus_and_input() {
        let (mut context, field, _) = mounted(RangeSpanField::new(0.2, 0.6, 0.0, 1.0, 0.1));
        context
            .update_component(field, |field, _| field.disabled = true)
            .unwrap();
        let low = thumb(&context, field, RangeSpanThumb::Low);
        assert!(!context.world().interaction(low).unwrap().focusable);
        assert!(context.world().accessibility(low).unwrap().disabled);
        assert!(!context.set_range_span(field, 0.0, 1.0).unwrap());
        assert_eq!(pair(&context, field), (0.2, 0.6));
    }

    #[test]
    fn set_range_span_commits_in_order() {
        let (mut context, field, seen) = mounted(RangeSpanField::new(0.2, 0.6, 0.0, 1.0, 0.1));
        assert!(context.set_range_span(field, 0.9, 0.1).unwrap());
        let (low, high) = pair(&context, field);
        assert!((low - 0.1).abs() < 1e-9 && (high - 0.9).abs() < 1e-9);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![Seen::Input(low, high), Seen::Changed(low, high)]
        );
        assert!(!context.set_range_span(field, 0.1, 0.9).unwrap());
    }

    #[test]
    fn a_field_inside_a_layout_keeps_its_thumbs_through_a_rebuild() {
        let mut context = AppContext::new();
        let root = context
            .create_component(document(), Stack::column(0.0))
            .unwrap();
        let field = context
            .create_detached_component(document(), RangeSpanField::new(0.2, 0.6, 0.0, 1.0, 0.1))
            .unwrap();
        context.append_child(root, field).unwrap();
        let thumbs = context.read(field, |field| field.thumbs).unwrap();
        assert!(thumbs.is_some());
        let mut next = RangeSpanField::new(0.3, 0.6, 0.0, 1.0, 0.1);
        context
            .update_component(field, |field, _| field.reconcile(next.clone()))
            .unwrap();
        next.thumbs = thumbs;
        assert_eq!(context.read(field, |field| field.thumbs).unwrap(), thumbs);
        assert_eq!(
            context
                .world()
                .node(field.stable_id())
                .unwrap()
                .children
                .len(),
            2
        );
    }
}
