//! A curve over the unit square with points that move up and down
//! ([`CurveField`]).
//!
//! The field node paints the plot and the curve through a [`Painter`]. Each
//! point is a child node of its own ([`CurvePoint`]): a sequential focus
//! stop, a vertical `Slider` for assistive technology, placed by absolute
//! offsets inside the field. A point sits at a fixed `x`, evenly spaced
//! between the ends, and only its height changes. Pointer, keyboard and
//! assistive input all write the field, which emits [`CurveInput`] /
//! [`CurveChanged`] / [`CurveDragging`] with the meaning
//! [`crate::RangeSpanField`]'s three events have.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use nana_ui_core::{ControlSize, LengthSpec, PositionSpec, SemanticColorRole, space};

use crate::range_span::{ThumbPainter, decimal_places, focus_ring_outset};
use crate::view_components::{format_range_value, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, BoxPaint, ComponentView, DocumentId, Entity,
    FrameworkError, InteractionState, LayoutBox, LineJoin, MutationQueue, NodeKind, NodePainter,
    NodeStyle, PaintContext, PaintPath, Painter, RangeAdjustment, SelectionOrientation,
    StableNodeId, StrokeStyle, UiWorld,
};

/// Diameter of a [`CurveField`] point: one spacing step smaller than a
/// [`crate::RangeSpanField`] thumb, so neighbouring points stay apart.
pub const fn curve_point_extent(size: ControlSize) -> f32 {
    match size {
        ControlSize::Small => space::LG,
        ControlSize::Medium => space::XL,
        ControlSize::Large => space::XXL,
    }
}

/// Distance from each edge of a [`CurveField`] node to its plot: half a
/// point plus its focus ring, so a point at a corner, ring included, stays
/// inside the node. An application drawing next to the plot (a scale, a
/// live marker) aligns to it.
pub const fn curve_plot_inset(size: ControlSize) -> f32 {
    curve_point_extent(size) / 2.0 + focus_ring_outset()
}

/// A curve over the unit square through [`values`](Self::values): point
/// `k` of `n` sits at `x = (k + 1) / (n + 1)` and at height `values[k]` in
/// `0 ..= 1`. The curve runs straight from point to point, and from the
/// fixed [`ends`](Self::ends) when there are any.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveField {
    /// One height per point, left to right, each in `0 ..= 1` on the step
    /// grid. Their number is the number of points.
    pub values: Vec<f64>,
    pub step: f64,
    pub page_step: f64,
    /// Heights at `x = 0` and `x = 1` the curve starts and ends on. They do
    /// not move and take no input. `None` starts and ends the curve on the
    /// first and last point.
    pub ends: Option<(f64, f64)>,
    /// Names the curve for assistive technology.
    pub label: Option<Arc<str>>,
    /// Each point's accessible name ("点 1"), left to right. A point past
    /// the list is named by its number.
    pub point_labels: Vec<Arc<str>>,
    pub size: ControlSize,
    pub disabled: bool,
    pub dragging: Option<CurveDragState>,
    pub style: NodeStyle,
    /// The point nodes, left to right, once assembled.
    pub(crate) points: Vec<StableNodeId>,
}

/// A pointer drag on a [`CurveField`].
#[derive(Debug, Clone, PartialEq)]
pub struct CurveDragState {
    pub pointer_id: u64,
    /// The point that moves.
    pub point: usize,
    /// Point height minus pointer height when the press grabbed the point,
    /// so it does not jump to the pointer; zero for a press on the plot.
    pub grab_offset: f64,
    pub initial: Vec<f64>,
}

/// Every set of heights the field shows, including a drag or keyboard step
/// in flight. Observe this for live previews.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveInput {
    /// The point that moved; `None` when the heights were replaced at once.
    pub point: Option<usize>,
    pub values: Vec<f64>,
}

/// Committed heights: pointer release, a keyboard step, an accessibility
/// `Increment` / `Decrement` / `SetValue`, or
/// [`AppContext::set_curve_values`] / [`AppContext::set_curve_point`]. A
/// drag emits it once, on release, and only when the point moved; a
/// cancelled drag never does.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveChanged {
    /// The point that moved; `None` when the heights were replaced at once.
    pub point: Option<usize>,
    pub values: Vec<f64>,
}

/// A pointer drag on the field began (`true`) or ended (`false`), after the
/// [`CurveChanged`] a release commits, or the [`CurveInput`] a cancel
/// restores. Keyboard and accessibility steps are not drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurveDragging {
    pub dragging: bool,
}

impl CurveField {
    /// Builds a curve through `values`, repairing inconsistent input
    /// instead of failing: a non-finite or non-positive `step` becomes
    /// `0.01`; each height is clamped into `0 ..= 1` and onto the step
    /// grid, and a non-finite one is `0`.
    pub fn new(values: impl Into<Vec<f64>>, step: f64) -> Self {
        let step = if step.is_finite() && step > 0.0 {
            step.min(1.0)
        } else {
            0.01
        };
        let mut field = Self {
            values: Vec::new(),
            step,
            page_step: step * 10.0,
            ends: None,
            label: None,
            point_labels: Vec::new(),
            size: ControlSize::Medium,
            disabled: false,
            dragging: None,
            style: NodeStyle::default(),
            points: Vec::new(),
        };
        field.set_values(values.into());
        field
    }

    /// Fixed heights at `x = 0` and `x = 1`, clamped into `0 ..= 1`.
    pub fn ends(mut self, start: f64, end: f64) -> Self {
        let clamp = |value: f64| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        self.ends = Some((clamp(start), clamp(end)));
        self
    }
    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn point_labels<L: Into<Arc<str>>>(mut self, labels: impl IntoIterator<Item = L>) -> Self {
        self.point_labels = labels.into_iter().map(Into::into).collect();
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

    /// Where point `index` sits across the plot, `0.0` at the left edge.
    pub fn position(&self, index: usize) -> f64 {
        (index + 1) as f64 / (self.values.len() + 1) as f64
    }

    /// `value` clamped into `0 ..= 1` and onto the step grid; a non-finite
    /// value is `0`. A grid with a short decimal spelling lands on it
    /// exactly: `0.6`, not `0.6000000000000001`.
    pub fn quantize(&self, value: f64) -> f64 {
        if !value.is_finite() {
            return 0.0;
        }
        let mut snapped = (value.clamp(0.0, 1.0) / self.step).round() * self.step;
        if let Some(places) = decimal_places(self.step) {
            let scale = 10_f64.powi(places);
            snapped = (snapped * scale).round() / scale;
        }
        snapped.clamp(0.0, 1.0)
    }

    /// Replaces every height, quantized. The number of points follows.
    pub fn set_values(&mut self, values: Vec<f64>) {
        self.values = values
            .into_iter()
            .map(|value| self.quantize(value))
            .collect();
    }

    /// What [`Self::set_values`] would store for `values`.
    pub(crate) fn quantized(&self, values: &[f64]) -> Vec<f64> {
        values.iter().map(|value| self.quantize(*value)).collect()
    }

    /// See [`curve_plot_inset`].
    pub fn plot_inset(&self) -> f32 {
        curve_plot_inset(self.size)
    }

    /// The (unquantized) height a pointer at `y` points at, for a field laid
    /// out at `bounds`. Both are in the same space.
    pub fn value_at(&self, bounds: LayoutBox, y: f32) -> f64 {
        let inset = self.plot_inset();
        let length = bounds.height - 2.0 * inset;
        let ratio = if length > 0.0 {
            ((bounds.y + bounds.height - inset - y) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        f64::from(ratio)
    }

    /// The point nearest to a pointer at `x`, for a field laid out at
    /// `bounds`: the left one on a tie, `None` without points.
    pub fn nearest_point(&self, bounds: LayoutBox, x: f32) -> Option<usize> {
        let inset = self.plot_inset();
        let length = (bounds.width - 2.0 * inset).max(f32::EPSILON);
        let across = f64::from((x - bounds.x - inset) / length);
        let count = self.values.len();
        if count == 0 {
            return None;
        }
        let nearest = (across * (count + 1) as f64).round() as i64 - 1;
        Some(nearest.clamp(0, count as i64 - 1) as usize)
    }

    /// The node of point `index`, once the field is assembled.
    pub fn point_node(&self, index: usize) -> Option<StableNodeId> {
        self.points.get(index).copied()
    }

    /// The accessible name of point `index`.
    pub fn point_label(&self, index: usize) -> Arc<str> {
        self.point_labels
            .get(index)
            .cloned()
            .unwrap_or_else(|| Arc::from((index + 1).to_string()))
    }

    fn handle(&self, index: usize) -> CurvePoint {
        let value = self.values[index];
        CurvePoint {
            index,
            across: self.position(index) as f32,
            height: value as f32,
            size: self.size,
            disabled: self.disabled,
            label: self.point_label(index),
            value,
            step: self.step,
            value_text: percent_text(value, self.step),
        }
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = &style.layout;
        let needs_layout = layout.position == PositionSpec::Static
            || layout.width.is_none()
            || (layout.height.is_none() && layout.aspect_ratio.is_none());
        if needs_layout {
            let layout = Arc::make_mut(&mut style.layout);
            // The points are placed against this box.
            if layout.position == PositionSpec::Static {
                layout.position = PositionSpec::Relative;
            }
            if layout.width.is_none() {
                layout.width = Some(LengthSpec::Fill);
            }
            // A square plot unless the application sizes it.
            if layout.height.is_none() && layout.aspect_ratio.is_none() {
                layout.aspect_ratio = Some(1.0);
            }
        }
        let mut points = Vec::with_capacity(self.values.len() + 2);
        if let Some((start, _)) = self.ends {
            points.push([0.0, start as f32]);
        }
        points.extend(
            self.values
                .iter()
                .enumerate()
                .map(|(index, value)| [self.position(index) as f32, *value as f32]),
        );
        if let Some((_, end)) = self.ends {
            points.push([1.0, end as f32]);
        }
        style.painter = Some(NodePainter::new(PlotPainter {
            inset: self.plot_inset(),
            points: points.into(),
        }));
        style
    }
}

static CURVE_DRAG_OPS: crate::framework::ActiveDragOps = crate::framework::ActiveDragOps {
    pre_gesture: false,
    cancel_on_touch_scroll: false,
    update: |cx, doc, _, ptr, x, y, _| cx.update_curve_drag_at(doc, ptr, x, y),
    end: |cx, doc, _, ptr, _, _, _, cancel| cx.end_curve_drag(doc, ptr, cancel),
};

static CURVE_CAPABILITIES: crate::framework::CurveCapabilities =
    crate::framework::CurveCapabilities {
        target: |cx, id| {
            cx.curve_target(id)
                .map(|(entity, point)| (entity.stable_id(), point))
        },
        begin_drag: AppContext::begin_curve_drag_at,
        focus_neighbour: AppContext::focus_curve_neighbour,
        adjust_point: |cx, id, index, adj| {
            cx.adjust_curve_point(Entity::from_stable_id(id), index, adj)
        },
        set_point: |cx, id, index, val| cx.set_curve_point(Entity::from_stable_id(id), index, val),
    };

static CURVE_POINTER_OPS: crate::framework::PointerOps = crate::framework::PointerOps {
    end_value_drag: Some(AppContext::end_curve_drag_on),
    ..crate::framework::PointerOps::EMPTY
};

impl ComponentView for CurveField {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(|caps| {
            caps.curve_field = Some(&CURVE_CAPABILITIES);
            caps.add_step_adjuster(AppContext::adjust_focused_curve);
        }),
        lifecycle: Some(crate::AppContext::sync_curve_field),
        pointer_ops: Some(&CURVE_POINTER_OPS),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Declarative properties replace the props; the point nodes and a drag
    /// in flight are the field's own.
    fn reconcile(&mut self, mut next: Self) {
        next.points = std::mem::take(&mut self.points);
        next.dragging = self.dragging.take();
        *self = next;
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "curve".into(),
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
                ..AccessibilityState::default()
            },
        );
    }
}

/// `value` as a percentage, with as many decimals as the step needs.
fn percent_text(value: f64, step: f64) -> Arc<str> {
    let number = format_range_value(value * 100.0, step * 100.0);
    Arc::from(format!("{number}%"))
}

/// One point of a [`CurveField`]: assembled by the field, which writes
/// every prop. Applications reach it through [`CurveField::point_node`].
#[derive(Debug, Clone, PartialEq)]
pub struct CurvePoint {
    index: usize,
    across: f32,
    height: f32,
    size: ControlSize,
    disabled: bool,
    label: Arc<str>,
    value: f64,
    step: f64,
    value_text: Arc<str>,
}

impl CurvePoint {
    /// Which point this is, counted from the left.
    pub fn index(&self) -> usize {
        self.index
    }

    fn style(&self) -> NodeStyle {
        let inset = curve_plot_inset(self.size);
        // The node is the point plus its ring, centred on the plot point
        // `inset + ratio * (extent - 2 * inset)` of the field's box.
        let along =
            |ratio: f32| LengthSpec::percent_offset_inline(ratio * 100.0, -2.0 * ratio * inset);
        NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                position: PositionSpec::Absolute,
                offset_left: Some(along(self.across)),
                offset_top: Some(along(1.0 - self.height)),
                width: Some(LengthSpec::Px(inset * 2.0)),
                height: Some(LengthSpec::Px(inset * 2.0)),
                ..nana_ui_core::LayoutStyle::default()
            }),
            painter: Some(NodePainter::new(ThumbPainter {
                extent: curve_point_extent(self.size),
            })),
            ..NodeStyle::default()
        }
    }
}

impl ComponentView for CurvePoint {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "curve-point".into(),
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
                label: Some(Arc::clone(&self.label)),
                value: Some(Arc::clone(&self.value_text)),
                disabled: self.disabled,
                orientation: Some(SelectionOrientation::Vertical),
                numeric_minimum: Some(0.0),
                numeric_maximum: Some(1.0),
                numeric_step: Some(self.step),
                numeric_value: Some(self.value),
                ..AccessibilityState::default()
            },
        );
    }
}

/// The plot frame, its quarter grid, and the curve.
#[derive(Debug, Clone, PartialEq)]
struct PlotPainter {
    inset: f32,
    /// The curve's corners over the unit square, `y` up, left to right.
    points: Arc<[[f32; 2]]>,
}

impl Painter for PlotPainter {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        let [width, height] = cx.size();
        let (w, h) = (width - 2.0 * self.inset, height - 2.0 * self.inset);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let disabled = cx.state().disabled;
        let at = |[x, y]: [f32; 2]| (self.inset + x * w, self.inset + (1.0 - y) * h);
        let mut grid = PaintPath::new();
        for quarter in 1..4 {
            let t = quarter as f32 / 4.0;
            grid.move_to(self.inset + t * w, self.inset)
                .line_to(self.inset + t * w, self.inset + h);
            grid.move_to(self.inset, self.inset + t * h)
                .line_to(self.inset + w, self.inset + t * h);
        }
        cx.stroke_path(
            &grid,
            StrokeStyle::new(nana_ui_core::HAIRLINE),
            SemanticColorRole::BorderSoft,
        );
        cx.rounded_rect(
            LayoutBox {
                x: self.inset,
                y: self.inset,
                width: w,
                height: h,
            },
            0.0,
            BoxPaint::default().border(SemanticColorRole::Border, nana_ui_core::HAIRLINE),
        );
        let mut line = PaintPath::new();
        for (index, point) in self.points.iter().enumerate() {
            let (x, y) = at(*point);
            if index == 0 {
                line.move_to(x, y);
            } else {
                line.line_to(x, y);
            }
        }
        if self.points.len() >= 2 {
            cx.stroke_path(
                &line,
                StrokeStyle::new(space::XXS).join(LineJoin::Round),
                if disabled {
                    SemanticColorRole::Faint
                } else {
                    SemanticColorRole::Accent
                },
            );
        }
    }

    fn paint_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.inset.to_bits().hash(&mut hasher);
        for [x, y] in self.points.iter() {
            x.to_bits().hash(&mut hasher);
            y.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl AppContext {
    /// The field `id` belongs to, and the point when `id` is one: what a
    /// press, a key or an assistive action on `id` writes.
    pub fn curve_target(&self, id: StableNodeId) -> Option<(Entity<CurveField>, Option<usize>)> {
        if let Some(field) = self.view_entity::<CurveField>(id) {
            return Some((field, None));
        }
        let point = self.view_entity::<CurvePoint>(id)?;
        let index = self.read(point, CurvePoint::index).ok()?;
        let field = self.view_entity::<CurveField>(self.world().parent_id(id)?)?;
        Some((field, Some(index)))
    }

    /// Commits every height at once: emits [`CurveInput`] then
    /// [`CurveChanged`] when any changed. The number of points follows
    /// `values`. During a drag the committed heights are also what a cancel
    /// restores.
    pub fn set_curve_values(
        &mut self,
        entity: Entity<CurveField>,
        values: Vec<f64>,
    ) -> Result<bool, FrameworkError> {
        if values.iter().any(|value| !value.is_finite()) {
            return Err(FrameworkError::InvalidComponentValue(entity.stable_id()));
        }
        if self.read(entity, |field| field.disabled)? {
            return Ok(false);
        }
        self.update_component(entity, |field, cx| {
            let next = field.quantized(&values);
            if next == field.values {
                return false;
            }
            if field
                .dragging
                .as_ref()
                .is_some_and(|drag| drag.point >= next.len())
            {
                field.dragging = None;
            }
            field.values = next;
            commit_curve(field, None, cx);
            true
        })
    }

    /// Commits the height of point `index`.
    pub fn set_curve_point(
        &mut self,
        entity: Entity<CurveField>,
        index: usize,
        value: f64,
    ) -> Result<bool, FrameworkError> {
        self.write_curve_point(entity, index, value, true)
    }

    fn write_curve_point(
        &mut self,
        entity: Entity<CurveField>,
        index: usize,
        value: f64,
        commit: bool,
    ) -> Result<bool, FrameworkError> {
        if !value.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.stable_id()));
        }
        if self.read(entity, |field| {
            field.disabled || index >= field.values.len()
        })? {
            return Ok(false);
        }
        self.update_component(entity, |field, cx| {
            let next = field.quantize(value);
            if field.values[index] == next {
                return false;
            }
            field.values[index] = next;
            if commit {
                commit_curve(field, Some(index), cx);
            } else {
                cx.emit(CurveInput {
                    point: Some(index),
                    values: field.values.clone(),
                });
            }
            true
        })
    }

    /// Steps point `index`, as a key on it does.
    pub fn adjust_curve_point(
        &mut self,
        entity: Entity<CurveField>,
        index: usize,
        adjustment: RangeAdjustment,
    ) -> Result<bool, FrameworkError> {
        let Some(value) = self.read(entity, |field| {
            let value = *field.values.get(index)?;
            Some(match adjustment {
                RangeAdjustment::Decrement => value - field.step,
                RangeAdjustment::Increment => value + field.step,
                RangeAdjustment::PageDecrement => value - field.page_step,
                RangeAdjustment::PageIncrement => value + field.page_step,
                RangeAdjustment::Minimum => 0.0,
                RangeAdjustment::Maximum => 1.0,
            })
        })?
        else {
            return Ok(false);
        };
        self.set_curve_point(entity, index, value)
    }

    /// Steps the focused point, if a curve point has focus.
    pub fn adjust_focused_curve(
        &mut self,
        document: DocumentId,
        adjustment: RangeAdjustment,
    ) -> Result<bool, FrameworkError> {
        let Some((field, Some(index))) = self
            .world()
            .focused(document)
            .and_then(|id| self.curve_target(id))
        else {
            return Ok(false);
        };
        self.adjust_curve_point(field, index, adjustment)
    }

    /// Moves focus from the focused point to its neighbour, the next one
    /// when `forward`. A key on the first or last point that would leave the
    /// curve stays on it and is still taken.
    pub fn focus_curve_neighbour(
        &mut self,
        document: DocumentId,
        forward: bool,
    ) -> Result<bool, FrameworkError> {
        let Some((field, Some(index))) = self
            .world()
            .focused(document)
            .and_then(|id| self.curve_target(id))
        else {
            return Ok(false);
        };
        let next = if forward {
            index.checked_add(1)
        } else {
            index.checked_sub(1)
        };
        if let Some(node) = next.and_then(|next| {
            self.read(field, |field| field.point_node(next))
                .ok()
                .flatten()
        }) {
            self.focus_node(document, node)?;
        }
        Ok(true)
    }

    /// [`Self::begin_curve_drag`] at a window point.
    pub(crate) fn begin_curve_drag_at(
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
        self.begin_curve_drag(document, pointer_id, target, x, y)
    }

    /// [`Self::update_curve_drag`] at a window point.
    pub(crate) fn update_curve_drag_at(
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
        self.update_curve_drag(document, pointer_id, x, y)
    }

    /// Starts a drag from a press on a field or one of its points, at `x`,
    /// `y` in layout space. A press on a point grabs it where it is; a press
    /// on the plot moves the point nearest across to the pointer's height.
    /// The field captures the pointer and the moving point takes focus.
    pub fn begin_curve_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some((field, grabbed)) = self.curve_target(target) else {
            return Ok(false);
        };
        let Some(bounds) = self.world().layout_box(field.stable_id()) else {
            return Ok(false);
        };
        let snapshot = self.read(field, Clone::clone)?;
        if snapshot.disabled {
            return Ok(false);
        }
        let pointer = snapshot.value_at(bounds, y);
        let Some((point, grab_offset)) = (match grabbed {
            Some(index) => snapshot
                .values
                .get(index)
                .map(|value| (index, value - pointer)),
            None => snapshot.nearest_point(bounds, x).map(|index| (index, 0.0)),
        }) else {
            return Ok(false);
        };
        let field_id = field.stable_id();
        self.update_component(field, |field, cx| {
            field.dragging = Some(CurveDragState {
                pointer_id,
                point,
                grab_offset,
                initial: field.values.clone(),
            });
            cx.mutations().capture_pointer(pointer_id, field_id);
            cx.emit(CurveDragging { dragging: true });
        })?;
        self.begin_active_drag(document, pointer_id, field_id, &CURVE_DRAG_OPS);
        if let Some(node) = snapshot.point_node(point) {
            self.focus_node_in_place(document, node)?;
        }
        if grabbed.is_none() {
            self.write_curve_point(field, point, pointer, false)?;
        }
        Ok(true)
    }

    /// Moves the dragged point to the pointer's height at `y` in layout
    /// space; `x` does not move it.
    pub fn update_curve_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        _x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(field) = self.view_entity::<CurveField>(target) else {
            return Ok(false);
        };
        let Some(bounds) = self.world().layout_box(target) else {
            return Ok(false);
        };
        let Some((point, value)) = self.read(field, |field| {
            let drag = field
                .dragging
                .as_ref()
                .filter(|drag| drag.pointer_id == pointer_id)?;
            Some((drag.point, field.value_at(bounds, y) + drag.grab_offset))
        })?
        else {
            return Ok(false);
        };
        self.write_curve_point(field, point, value, false)
    }

    /// Ends the drag `pointer_id` holds. Release commits with
    /// [`CurveChanged`] when the point moved; cancel, or a field disabled
    /// mid-drag, restores the heights the drag started from and commits
    /// nothing.
    pub fn end_curve_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        self.end_curve_drag_on(target, pointer_id, cancel)
    }

    pub(crate) fn end_curve_drag_on(
        &mut self,
        target: StableNodeId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(field) = self.view_entity::<CurveField>(target) else {
            return Ok(false);
        };
        let document = self.world().document_of(target);
        let held = document.and_then(|document| self.world().pointer_capture(document, pointer_id))
            == Some(target);
        if let Some(document) = document {
            self.clear_active_drag(document, pointer_id, target);
        }
        self.update_component(field, |field, cx| {
            if held {
                cx.mutations().release_pointer(pointer_id, target);
            }
            let Some(drag) = field.dragging.take_if(|drag| drag.pointer_id == pointer_id) else {
                return false;
            };
            if field.values != drag.initial {
                if cancel || field.disabled || field.values.len() != drag.initial.len() {
                    if field.values.len() == drag.initial.len() {
                        field.values = drag.initial;
                    }
                    cx.emit(CurveInput {
                        point: Some(drag.point),
                        values: field.values.clone(),
                    });
                } else {
                    cx.emit(CurveChanged {
                        point: Some(drag.point),
                        values: field.values.clone(),
                    });
                }
            }
            cx.emit(CurveDragging { dragging: false });
            true
        })
    }

    /// Lifecycle: create one point node per height, again when their
    /// number changes, and keep them on the field's props.
    pub(crate) fn sync_curve_field(
        &mut self,
        field: Entity<CurveField>,
    ) -> Result<(), FrameworkError> {
        let snapshot = self.read(field, Clone::clone)?;
        let assembled = snapshot.points.len() == snapshot.values.len()
            && snapshot.points.iter().all(|id| {
                self.world().parent_id(*id) == Some(field.stable_id())
                    && self.view_entity::<CurvePoint>(*id).is_some()
            });
        if !assembled {
            for id in &snapshot.points {
                if self.world().contains(*id) {
                    self.despawn_node(*id)?;
                }
            }
            let document = self
                .world()
                .node(field.stable_id())
                .ok_or(FrameworkError::MissingView(field.stable_id()))?
                .document;
            let mut points = Vec::with_capacity(snapshot.values.len());
            for index in 0..snapshot.values.len() {
                let point = self.create_detached_component(document, snapshot.handle(index))?;
                points.push(point.stable_id());
            }
            self.append_children(field.stable_id(), &points)?;
            return self.update_component(field, |field, _| field.points = points);
        }
        for (index, id) in snapshot.points.iter().enumerate() {
            let next = snapshot.handle(index);
            self.update_component(Entity::<CurvePoint>::from_stable_id(*id), |point, _| {
                if *point != next {
                    *point = next;
                }
            })?;
        }
        Ok(())
    }
}

/// Emits the heights as shown and as committed.
fn commit_curve(
    field: &mut CurveField,
    point: Option<usize>,
    cx: &mut crate::ViewContext<'_, CurveField>,
) {
    if let Some(drag) = field.dragging.as_mut() {
        drag.initial = field.values.clone();
    }
    cx.emit(CurveInput {
        point,
        values: field.values.clone(),
    });
    cx.emit(CurveChanged {
        point,
        values: field.values.clone(),
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
        Input(Option<usize>, Vec<f64>),
        Changed(Option<usize>, Vec<f64>),
        Dragging(bool),
    }

    /// The plot side the tests lay a field out at: 200 px between the
    /// insets.
    fn side() -> f32 {
        200.0 + 2.0 * curve_plot_inset(ControlSize::Medium)
    }

    /// A field laid out alone at the origin of a 400 x 400 document, its
    /// plot 200 px square.
    fn mounted(mut field: CurveField) -> (AppContext, Entity<CurveField>, Arc<Mutex<Vec<Seen>>>) {
        Arc::make_mut(&mut field.style.layout).width = Some(LengthSpec::Px(side()));
        let mut context = AppContext::new();
        let field = context.create_component(document(), field).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        context
            .on(field, move |_, event: &CurveInput, _| {
                log.lock()
                    .unwrap()
                    .push(Seen::Input(event.point, event.values.clone()));
            })
            .unwrap();
        let log = Arc::clone(&seen);
        context
            .on(field, move |_, event: &CurveChanged, _| {
                log.lock()
                    .unwrap()
                    .push(Seen::Changed(event.point, event.values.clone()));
            })
            .unwrap();
        let log = Arc::clone(&seen);
        context
            .on(field, move |_, event: &CurveDragging, _| {
                log.lock().unwrap().push(Seen::Dragging(event.dragging));
            })
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(400.0, 400.0))
            .unwrap();
        context.rebuild_hit_test(document());
        (context, field, seen)
    }

    fn values(context: &AppContext, field: Entity<CurveField>) -> Vec<f64> {
        context.read(field, |field| field.values.clone()).unwrap()
    }

    fn point(context: &AppContext, field: Entity<CurveField>, index: usize) -> StableNodeId {
        context
            .read(field, |field| field.point_node(index))
            .unwrap()
            .expect("points are assembled with the field")
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

    fn three() -> CurveField {
        CurveField::new(vec![0.25, 0.5, 0.75], 0.05).point_labels(["点 1", "点 2", "点 3"])
    }

    fn close(left: &[f64], right: &[f64]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .all(|(left, right)| (left - right).abs() < 1e-9)
    }

    #[test]
    fn construction_clamps_and_snaps_every_height_and_repairs_the_step() {
        let field = CurveField::new(vec![-1.0, 0.33, 7.0, f64::NAN], 0.1);
        assert_eq!(field.values, vec![0.0, 0.3, 1.0, 0.0]);
        let field = CurveField::new(vec![0.5], -2.0);
        assert_eq!(field.step, 0.01);
        assert_eq!(field.page_step, 0.1);
        let field = CurveField::new(vec![0.0; 6], 0.001);
        assert_eq!(field.position(0), 1.0 / 7.0);
        assert_eq!(field.position(5), 6.0 / 7.0);
    }

    #[test]
    fn a_pointer_height_maps_through_the_inset_and_the_nearest_point_is_found_across() {
        let field = CurveField::new(vec![0.0; 3], 0.01);
        let inset = field.plot_inset();
        assert_eq!(
            inset,
            curve_point_extent(ControlSize::Medium) / 2.0 + focus_ring_outset()
        );
        let bounds = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 200.0 + 2.0 * inset,
            height: 200.0 + 2.0 * inset,
        };
        let bottom = bounds.y + bounds.height - inset;
        assert_eq!(field.value_at(bounds, bottom), 0.0);
        assert_eq!(field.value_at(bounds, bottom - 50.0), 0.25);
        assert_eq!(field.value_at(bounds, 0.0), 1.0);
        assert_eq!(field.value_at(bounds, 999.0), 0.0);
        let at = |ratio: f32| bounds.x + inset + ratio * 200.0;
        assert_eq!(field.nearest_point(bounds, at(0.0)), Some(0));
        assert_eq!(field.nearest_point(bounds, at(0.3)), Some(0));
        assert_eq!(field.nearest_point(bounds, at(0.45)), Some(1));
        assert_eq!(field.nearest_point(bounds, at(0.7)), Some(2));
        assert_eq!(field.nearest_point(bounds, at(1.0)), Some(2));
        assert_eq!(
            CurveField::new(Vec::new(), 0.1).nearest_point(bounds, 0.0),
            None
        );
    }

    #[test]
    fn points_are_vertical_sliders_placed_on_the_plot_in_percent() {
        let (context, field, _) = mounted(three().label("曲线"));
        let bounds = context.world().layout_box(field.stable_id()).unwrap();
        assert_eq!(
            (bounds.width, bounds.height),
            (side(), side()),
            "a square plot by default"
        );
        let inset = curve_plot_inset(ControlSize::Medium);
        let children = context.world().node(field.stable_id()).unwrap().children;
        assert_eq!(children.len(), 3);
        for (index, height) in [0.25_f32, 0.5, 0.75].into_iter().enumerate() {
            let id = point(&context, field, index);
            assert_eq!(children[index], id);
            let (x, y) = centre(&context, id);
            let across = (index + 1) as f32 / 4.0;
            assert!(
                (x - (bounds.x + inset + across * 200.0)).abs() < 0.01,
                "{x}"
            );
            assert!(
                (y - (bounds.y + inset + (1.0 - height) * 200.0)).abs() < 0.01,
                "{y}"
            );
        }
        let second = context
            .world()
            .accessibility(point(&context, field, 1))
            .unwrap();
        assert_eq!(second.role, AccessibilityRole::Slider);
        assert_eq!(second.label.as_deref(), Some("点 2"));
        assert_eq!(second.value.as_deref(), Some("50%"));
        assert_eq!(second.orientation, Some(SelectionOrientation::Vertical));
        assert_eq!(
            (
                second.numeric_minimum,
                second.numeric_maximum,
                second.numeric_step,
                second.numeric_value
            ),
            (Some(0.0), Some(1.0), Some(0.05), Some(0.5))
        );
        let unnamed = CurveField::new(vec![0.123], 0.001);
        assert_eq!(&*unnamed.point_label(0), "1");
        assert_eq!(&*unnamed.handle(0).value_text, "12.3%");
    }

    #[test]
    fn a_press_on_a_point_grabs_it_and_commits_once_on_release() {
        let (mut context, field, seen) = mounted(three());
        let second = point(&context, field, 1);
        let (x, y) = centre(&context, second);
        let mut input = HeadlessInput::bind(&mut context, document());
        // Off the point's centre: grabbing does not jump it to the pointer.
        input
            .pointer(&mut context, PointerPhase::Down, x, y + 3.0)
            .unwrap();
        assert_eq!(
            context.world().pointer_capture(document(), 1),
            Some(field.stable_id())
        );
        assert_eq!(context.world().focused(document()), Some(second));
        assert_eq!(values(&context, field), vec![0.25, 0.5, 0.75]);
        // 40 px up is a fifth; sideways does not move it across.
        input
            .pointer(&mut context, PointerPhase::Move, x + 60.0, y + 3.0 - 40.0)
            .unwrap();
        assert!(close(&values(&context, field), &[0.25, 0.7, 0.75]));
        input
            .pointer(&mut context, PointerPhase::Move, x, -50.0)
            .unwrap();
        assert!(close(&values(&context, field), &[0.25, 1.0, 0.75]));
        input
            .pointer(&mut context, PointerPhase::Move, x, y + 3.0 + 20.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Up, x, y + 3.0 + 20.0)
            .unwrap();
        let after = values(&context, field);
        assert!(close(&after, &[0.25, 0.4, 0.75]), "{after:?}");
        assert_eq!(context.world().pointer_capture(document(), 1), None);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first(), Some(&Seen::Dragging(true)));
        assert_eq!(seen.last(), Some(&Seen::Dragging(false)));
        assert!(
            seen.iter()
                .any(|event| matches!(event, Seen::Input(Some(1), _)))
        );
        let commits: Vec<_> = seen
            .iter()
            .filter(|event| matches!(event, Seen::Changed(..)))
            .collect();
        assert_eq!(commits, vec![&Seen::Changed(Some(1), after)]);
    }

    #[test]
    fn a_press_on_the_plot_moves_the_nearest_point_and_a_cancel_restores() {
        let (mut context, field, seen) = mounted(three());
        let bounds = context.world().layout_box(field.stable_id()).unwrap();
        let inset = curve_plot_inset(ControlSize::Medium);
        let at = |across: f32, height: f32| {
            (
                bounds.x + inset + across * 200.0,
                bounds.y + inset + (1.0 - height) * 200.0,
            )
        };
        let (x, y) = at(0.7, 0.1);
        let mut input = HeadlessInput::bind(&mut context, document());
        input
            .pointer(&mut context, PointerPhase::Down, x, y)
            .unwrap();
        assert!(close(&values(&context, field), &[0.25, 0.5, 0.1]));
        assert_eq!(
            context.world().focused(document()),
            Some(point(&context, field, 2))
        );
        input
            .pointer(&mut context, PointerPhase::Cancel, x, y)
            .unwrap();
        assert_eq!(values(&context, field), vec![0.25, 0.5, 0.75]);
        assert_eq!(context.world().pointer_capture(document(), 1), None);
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, Seen::Changed(..))),
            "a cancelled drag commits nothing"
        );
    }

    #[test]
    fn tab_walks_the_points_side_arrows_move_between_them_and_up_down_step() {
        let (mut context, field, seen) = mounted(three());
        let mut input = HeadlessInput::bind(&mut context, document());
        key(&mut input, &mut context, "Tab");
        assert_eq!(
            context.world().focused(document()),
            Some(point(&context, field, 0))
        );
        key(&mut input, &mut context, "ArrowUp");
        assert!(close(&values(&context, field), &[0.3, 0.5, 0.75]));
        key(&mut input, &mut context, "ArrowLeft");
        assert_eq!(
            context.world().focused(document()),
            Some(point(&context, field, 0)),
            "the first point keeps focus"
        );
        assert!(
            close(&values(&context, field), &[0.3, 0.5, 0.75]),
            "and its height"
        );
        key(&mut input, &mut context, "ArrowRight");
        key(&mut input, &mut context, "ArrowRight");
        assert_eq!(
            context.world().focused(document()),
            Some(point(&context, field, 2))
        );
        key(&mut input, &mut context, "PageUp");
        assert!(close(&values(&context, field), &[0.3, 0.5, 1.0]));
        key(&mut input, &mut context, "Home");
        key(&mut input, &mut context, "ArrowLeft");
        key(&mut input, &mut context, "ArrowDown");
        let now = values(&context, field);
        assert!(close(&now, &[0.3, 0.45, 0.0]), "{now:?}");
        assert_eq!(
            seen.lock().unwrap().last(),
            Some(&Seen::Changed(Some(1), now)),
            "a key step is a commit"
        );
        key(&mut input, &mut context, "Tab");
        assert_eq!(
            context.world().focused(document()),
            Some(point(&context, field, 2))
        );
    }

    #[test]
    fn assistive_actions_step_and_set_a_point_in_percent_or_as_a_fraction() {
        let (mut context, field, _) = mounted(three());
        let first = point(&context, field, 0);
        let act = |context: &mut AppContext, action| {
            context
                .apply_accessibility_action(
                    document(),
                    AccessibilityActionRequest {
                        target: first,
                        action,
                    },
                )
                .unwrap()
        };
        assert!(act(&mut context, AccessibilityAction::Increment));
        assert!((values(&context, field)[0] - 0.3).abs() < 1e-9);
        assert!(act(&mut context, AccessibilityAction::Decrement));
        assert!((values(&context, field)[0] - 0.25).abs() < 1e-9);
        assert!(act(
            &mut context,
            AccessibilityAction::SetValue("60%".into())
        ));
        assert!((values(&context, field)[0] - 0.6).abs() < 1e-9);
        assert!(act(
            &mut context,
            AccessibilityAction::SetValue("0.8".into())
        ));
        assert!((values(&context, field)[0] - 0.8).abs() < 1e-9);
        assert!(!act(
            &mut context,
            AccessibilityAction::SetValue("much".into())
        ));
        assert_eq!(
            context
                .world()
                .accessibility(first)
                .unwrap()
                .value
                .as_deref(),
            Some("80%")
        );
    }

    #[test]
    fn replacing_the_heights_commits_and_a_new_count_reassembles_the_points() {
        let (mut context, field, seen) = mounted(three());
        let old = point(&context, field, 0);
        assert!(
            context
                .set_curve_values(field, vec![0.1, 0.2, 0.3])
                .unwrap()
        );
        assert_eq!(point(&context, field, 0), old, "same count, same nodes");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                Seen::Input(None, vec![0.1, 0.2, 0.3]),
                Seen::Changed(None, vec![0.1, 0.2, 0.3]),
            ]
        );
        assert!(
            !context
                .set_curve_values(field, vec![0.1, 0.2, 0.3])
                .unwrap()
        );
        assert!(
            context
                .set_curve_values(field, vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0])
                .unwrap()
        );
        let children = context.world().node(field.stable_id()).unwrap().children;
        assert_eq!(children.len(), 6);
        assert!(!context.world().contains(old));
        assert_eq!(
            context
                .world()
                .accessibility(children[5])
                .unwrap()
                .label
                .as_deref(),
            Some("6"),
            "past the named points, a number"
        );
        assert!(context.set_curve_values(field, vec![f64::NAN]).is_err());
    }

    #[test]
    fn disabling_takes_the_points_out_of_focus_and_input() {
        let (mut context, field, _) = mounted(three());
        context
            .update_component(field, |field, _| field.disabled = true)
            .unwrap();
        let first = point(&context, field, 0);
        assert!(!context.world().interaction(first).unwrap().focusable);
        assert!(context.world().accessibility(first).unwrap().disabled);
        assert!(!context.set_curve_point(field, 0, 1.0).unwrap());
        assert!(!context.set_curve_values(field, vec![0.0]).unwrap());
        assert_eq!(values(&context, field), vec![0.25, 0.5, 0.75]);
    }

    #[test]
    fn the_curve_runs_from_the_fixed_ends_through_every_point() {
        use crate::custom_paint::{PaintOp, record};
        let theme = nana_ui_core::builtin_theme_arc(nana_ui_core::ThemeAppearance::Light);
        let field = CurveField::new(vec![0.25, 0.5], 0.01).ends(0.0, 1.0);
        let style = field.effective_style();
        let painter = style.painter.as_ref().unwrap().painter();
        let inset = field.plot_inset();
        let size = 90.0 + 2.0 * inset;
        let recorded = record(
            painter,
            theme.as_ref(),
            [size, size],
            crate::PaintState::default(),
            &crate::custom_paint::EmPaintTextHost,
        );
        let strokes: Vec<_> = recorded
            .behind_children
            .iter()
            .filter_map(|op| match op {
                PaintOp::StrokePath { path, stroke, .. } => Some((path.clone(), stroke.width)),
                _ => None,
            })
            .collect();
        let (curve, width) = strokes.last().unwrap();
        assert_eq!(*width, space::XXS);
        let corners: Vec<[f32; 2]> = curve
            .verbs()
            .iter()
            .filter_map(|verb| match verb {
                crate::PathVerb::MoveTo(point) | crate::PathVerb::LineTo(point) => Some(*point),
                _ => None,
            })
            .collect();
        let at = |x: f32, y: f32| [inset + x * 90.0, inset + (1.0 - y) * 90.0];
        let expected = [
            at(0.0, 0.0),
            at(1.0 / 3.0, 0.25),
            at(2.0 / 3.0, 0.5),
            at(1.0, 1.0),
        ];
        assert_eq!(corners.len(), expected.len());
        for (corner, expected) in corners.iter().zip(expected) {
            assert!(
                (corner[0] - expected[0]).abs() < 1e-3 && (corner[1] - expected[1]).abs() < 1e-3,
                "{corner:?} {expected:?}"
            );
        }
        let moved = CurveField::new(vec![0.25, 0.6], 0.01).ends(0.0, 1.0);
        assert_ne!(
            moved
                .effective_style()
                .painter
                .as_ref()
                .unwrap()
                .painter()
                .paint_key(),
            painter.paint_key(),
            "a moved point is a new paint key"
        );
    }

    #[test]
    fn a_field_inside_a_layout_keeps_its_points_through_a_rebuild() {
        let mut context = AppContext::new();
        let root = context
            .create_component(document(), Stack::column(0.0))
            .unwrap();
        let field = context
            .create_detached_component(document(), three())
            .unwrap();
        context.append_child(root, field).unwrap();
        let points = context.read(field, |field| field.points.clone()).unwrap();
        assert_eq!(points.len(), 3);
        context
            .update_component(field, |field, _| {
                field.reconcile(CurveField::new(vec![0.3, 0.5, 0.75], 0.05))
            })
            .unwrap();
        assert_eq!(
            context.read(field, |field| field.points.clone()).unwrap(),
            points
        );
        assert_eq!(
            context.world().node(field.stable_id()).unwrap().children,
            points
        );
    }
}
