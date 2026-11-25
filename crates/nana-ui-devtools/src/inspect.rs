//! Live inspection overlay for a hosted development window: what to draw
//! over the product scene while a developer inspects it.
//!
//! [`InspectOverlay`] turns a document's retained state into plain draw
//! data — rectangles in document coordinates, each with a role and a colour,
//! and text labels — that the development host paints above its own frame
//! however it likes. It shows:
//!
//! - **Box model** of the node under the pointer: margin, border, padding and
//!   content boxes, and a label with its size and edges.
//! - **Relayout**: every node the latest layout pass recomputed. A laid-out
//!   node without the highlight reused its cached layout.
//! - **Bindings**: where the hovered node's element and bound fields were
//!   declared, why layout last reached it, and (with `reactive-trace`) the
//!   signal writes that last patched it.
//!
//! The overlay is development tooling, not product UI: it adds nothing to
//! the retained tree or the scene, and nothing on the product frame path
//! calls into it. While it is off, [`InspectOverlay::update`] returns at its
//! first check without allocating and leaves [`OverlayFrame`] empty, so the
//! host draws nothing; turning it on allocates only inside `update`.

use std::fmt::Write as _;
use std::time::Duration;

use nana_ui::runtime::{
    AnimatableProperty, AnimationClass, AppContext, DocumentId, LayoutBox, MotionEvaluatorBackend,
    MotionHandle, MotionInspectorEntry, MotionValue, StableNodeId,
};
use nana_ui_core::{FontSizeContext, PaddingSpec};

/// Most relayout rectangles one frame draws; a full relayout of a large
/// document is summarised past this.
pub const MAX_RELAYOUT_QUADS: usize = 4096;

/// What a rectangle shows, in the order a frame lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuadRole {
    /// A node the latest layout pass recomputed (a layout cache miss).
    Relayout,
    /// An active motion track evaluated on the GPU compositor path.
    MotionGpu,
    /// An active motion track evaluated on the CPU (Layout/Paint/Discrete or CPU fallback).
    MotionCpu,
    /// The hovered node's margin box.
    Margin,
    /// Its border box.
    Border,
    /// Its padding box.
    Padding,
    /// Its content box.
    Content,
}

impl QuadRole {
    /// The default fill, linear RGBA. Box-model fills nest, so each band
    /// shows only where the inner box does not cover it.
    pub const fn color(self) -> [f32; 4] {
        match self {
            Self::Relayout => [0.85, 0.20, 0.75, 0.22],
            Self::MotionGpu => [0.16, 0.78, 0.56, 0.34],
            Self::MotionCpu => [0.95, 0.58, 0.16, 0.34],
            Self::Margin => [0.96, 0.70, 0.42, 0.55],
            Self::Border => [1.00, 0.86, 0.50, 0.60],
            Self::Padding => [0.58, 0.77, 0.49, 0.55],
            Self::Content => [0.44, 0.66, 0.86, 0.55],
        }
    }
}

/// One rectangle to fill, in document coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayQuad {
    pub rect: LayoutBox,
    pub role: QuadRole,
    pub color: [f32; 4],
}

/// One line of text to draw at `anchor` (its top-left, in document
/// coordinates).
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayLabel {
    pub anchor: (f32, f32),
    pub text: String,
}

/// The hovered node's box model.
#[derive(Debug, Clone, PartialEq)]
pub struct HoveredBox {
    pub node: StableNodeId,
    /// The control's tag (`"Button"`), for a built-in control.
    pub control: Option<&'static str>,
    pub margin_box: LayoutBox,
    pub border_box: LayoutBox,
    pub padding_box: LayoutBox,
    pub content_box: LayoutBox,
    pub margin: PaddingSpec,
    pub border: PaddingSpec,
    pub padding: PaddingSpec,
}

/// Which parts the overlay draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectLayers {
    pub box_model: bool,
    pub relayout: bool,
    pub bindings: bool,
    pub motion: bool,
}

impl Default for InspectLayers {
    fn default() -> Self {
        Self {
            box_model: true,
            relayout: true,
            bindings: true,
            motion: true,
        }
    }
}

/// One structured Motion Inspector row projected from [`MotionInspectorEntry`].
#[derive(Debug, Clone, PartialEq)]
pub struct MotionInspectorRow {
    pub node: StableNodeId,
    pub property: AnimatableProperty,
    pub animation_class: AnimationClass,
    pub evaluator: MotionEvaluatorBackend,
    pub layer: Option<u64>,
    pub layer_promotion_reason: Option<&'static str>,
    pub cpu_fallback: Option<String>,
    pub ineffective_reason: Option<String>,
    pub logical: Option<MotionValue>,
    pub presentation: Option<MotionValue>,
    pub next_deadline: Option<Duration>,
    pub gpu_handle: Option<MotionHandle>,
    pub has_live_handle: bool,
    pub rect: Option<LayoutBox>,
    pub summary: String,
    pub diagnostics: String,
}

/// Draw data for one frame. Empty while the overlay is off.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct OverlayFrame {
    /// Rectangles in paint order: relayout highlights, active motion overlay
    /// tracks, then the hovered node's margin, border, padding and content boxes.
    pub quads: Vec<OverlayQuad>,
    /// Text in paint order, after every quad.
    pub labels: Vec<OverlayLabel>,
    /// Visual Motion Inspector panel rows backed by `UiWorld::inspect_motion`.
    pub motion_rows: Vec<MotionInspectorRow>,
    pub hovered: Option<HoveredBox>,
    /// Nodes of this document the latest layout pass recomputed, including
    /// any past [`MAX_RELAYOUT_QUADS`].
    pub relaid_out: usize,
}

impl OverlayFrame {
    /// Nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty() && self.labels.is_empty() && self.motion_rows.is_empty()
    }

    fn clear(&mut self) {
        self.quads.clear();
        self.labels.clear();
        self.motion_rows.clear();
        self.hovered = None;
        self.relaid_out = 0;
    }
}

/// A development host's inspection mode for one window.
#[derive(Debug, Default)]
pub struct InspectOverlay {
    enabled: bool,
    layers: InspectLayers,
    pointer: Option<(f32, f32)>,
    frame: OverlayFrame,
}

impl InspectOverlay {
    /// Off, with every layer selected for when it is turned on.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Turn inspection on or off. Off clears the frame at once.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.frame.clear();
        }
    }

    /// Flip inspection, as a development shortcut would; returns the new state.
    pub fn toggle(&mut self) -> bool {
        self.set_enabled(!self.enabled);
        self.enabled
    }

    pub fn layers(&self) -> InspectLayers {
        self.layers
    }

    pub fn set_layers(&mut self, layers: InspectLayers) {
        self.layers = layers;
    }

    /// Where the pointer is, in document coordinates; `None` when it left
    /// the window. Hover inspects the node there.
    pub fn set_pointer(&mut self, pointer: Option<(f32, f32)>) {
        self.pointer = pointer;
    }

    /// The latest frame's draw data.
    pub fn frame(&self) -> &OverlayFrame {
        &self.frame
    }

    /// Rebuild the draw data from `document` after its layout pass. Off,
    /// this returns at once: no allocation, no lookup, an empty frame.
    pub fn update(&mut self, context: &AppContext, document: DocumentId) -> &OverlayFrame {
        self.update_with_motion_annotation(context, document, |_| {})
    }

    /// Rebuild draw data from `document` and let the caller annotate
    /// [`MotionInspectorEntry`] rows (for example via
    /// `UiScene::annotate_motion_inspector`) without performing GPU readback.
    /// Off, this returns at once with zero allocations and zero queries.
    pub fn update_with_motion_annotation(
        &mut self,
        context: &AppContext,
        document: DocumentId,
        annotate: impl FnOnce(&mut [MotionInspectorEntry]),
    ) -> &OverlayFrame {
        if !self.enabled {
            return &self.frame;
        }
        self.frame.clear();
        if self.layers.relayout {
            self.collect_relayout(context, document);
        }
        if self.layers.motion {
            self.collect_motion(context, document, annotate);
        }
        if let Some((x, y)) = self.pointer
            && (self.layers.box_model || self.layers.bindings)
            && let Some(node) = context.world().hit_test(document, x, y)
        {
            self.collect_hovered(context, node);
        }
        &self.frame
    }

    fn collect_motion(
        &mut self,
        context: &AppContext,
        document: DocumentId,
        annotate: impl FnOnce(&mut [MotionInspectorEntry]),
    ) {
        let world = context.world();
        let mut entries = world.inspect_motion();
        entries.retain(|entry| {
            StableNodeId::new(entry.node).and_then(|id| world.document_of(id)) == Some(document)
        });
        if entries.is_empty() {
            return;
        }
        annotate(&mut entries);

        let mut fallback_y = 8.0f32;
        for entry in entries {
            let Some(node) = StableNodeId::new(entry.node) else {
                continue;
            };
            let rect = world.layout_result(node).map(|result| result.border_box);
            let role = match entry.evaluator {
                MotionEvaluatorBackend::Gpu => QuadRole::MotionGpu,
                MotionEvaluatorBackend::Cpu => QuadRole::MotionCpu,
            };
            if let Some(rect) = rect {
                self.frame.quads.push(OverlayQuad {
                    rect,
                    role,
                    color: role.color(),
                });
            }

            let has_live_handle = entry.gpu_handle.is_some_and(|handle| !handle.is_null());
            let summary = entry.format_summary();
            let diagnostics = entry.format_diagnostics();

            let (anchor_x, mut line_y) = match rect {
                Some(rect) => (rect.x, rect.y),
                None => (8.0, fallback_y),
            };
            let mut push_label = |text: String| {
                self.frame.labels.push(OverlayLabel {
                    anchor: (anchor_x, line_y),
                    text,
                });
                line_y += LABEL_LINE;
            };

            for line in diagnostics.lines() {
                push_label(line.to_owned());
            }
            if !has_live_handle {
                push_label(String::from("GPU handle: none"));
            }
            if entry.base.is_some() || entry.presentation.is_some() {
                let mut values = String::new();
                let _ = write!(
                    values,
                    "Values: logical={} presentation={}",
                    format_opt_motion_value(entry.base),
                    format_opt_motion_value(entry.presentation),
                );
                push_label(values);
            }
            if rect.is_none() {
                fallback_y = line_y + 4.0;
            }

            self.frame.motion_rows.push(MotionInspectorRow {
                node,
                property: entry.property,
                animation_class: entry.class,
                evaluator: entry.evaluator,
                layer: entry.layer,
                layer_promotion_reason: entry.layer_promotion_reason,
                cpu_fallback: entry.cpu_fallback_reason,
                ineffective_reason: entry.ineffective_reason,
                logical: entry.base,
                presentation: entry.presentation,
                next_deadline: entry.next_deadline,
                gpu_handle: entry.gpu_handle,
                has_live_handle,
                rect,
                summary,
                diagnostics,
            });
        }
    }

    fn collect_relayout(&mut self, context: &AppContext, document: DocumentId) {
        let world = context.world();
        for &node in context.last_layout_scope() {
            if world.document_of(node) != Some(document) {
                continue;
            }
            self.frame.relaid_out += 1;
            if self.frame.quads.len() >= MAX_RELAYOUT_QUADS {
                continue;
            }
            if let Some(rect) = world.layout_result(node).map(|result| result.border_box) {
                self.frame.quads.push(OverlayQuad {
                    rect,
                    role: QuadRole::Relayout,
                    color: QuadRole::Relayout.color(),
                });
            }
        }
    }

    fn collect_hovered(&mut self, context: &AppContext, node: StableNodeId) {
        let world = context.world();
        let Some(result) = world.layout_result(node) else {
            return;
        };
        let (border_box, padding_box, content_box) =
            (result.border_box, result.padding_box, result.content_box);
        let margin = resolved_margin(context, node);
        let hovered = HoveredBox {
            node,
            control: context
                .inspect(node)
                .and_then(|inspection| inspection.control),
            margin_box: outset(border_box, margin),
            border_box,
            padding_box,
            content_box,
            margin,
            border: inset_edges(border_box, padding_box),
            padding: inset_edges(padding_box, content_box),
        };

        let mut line_y = hovered.margin_box.y + hovered.margin_box.height + 4.0;
        let anchor_x = hovered.margin_box.x;
        let mut label = |text: String| {
            self.frame.labels.push(OverlayLabel {
                anchor: (anchor_x, line_y),
                text,
            });
            line_y += LABEL_LINE;
        };

        if self.layers.box_model {
            for (rect, role) in [
                (hovered.margin_box, QuadRole::Margin),
                (border_box, QuadRole::Border),
                (padding_box, QuadRole::Padding),
                (content_box, QuadRole::Content),
            ] {
                self.frame.quads.push(OverlayQuad {
                    rect,
                    role,
                    color: role.color(),
                });
            }
            let mut text = String::new();
            let _ = write!(
                text,
                "{} #{}  {}×{}",
                hovered.control.unwrap_or("node"),
                node.get(),
                round(border_box.width),
                round(border_box.height)
            );
            for (name, edges) in [
                ("margin", hovered.margin),
                ("border", hovered.border),
                ("padding", hovered.padding),
            ] {
                if edges != PaddingSpec::default() {
                    let _ = write!(
                        text,
                        "  {name} {} {} {} {}",
                        round(edges.top),
                        round(edges.right),
                        round(edges.bottom),
                        round(edges.left)
                    );
                }
            }
            label(text);
        }

        if self.layers.bindings {
            if let Some(cause) = context.layout_cause(node) {
                let mut text = String::from(if cause.pending {
                    "layout pending:"
                } else {
                    "laid out for:"
                });
                for reason in cause.invalidation.reason.names() {
                    text.push(' ');
                    text.push_str(reason);
                }
                label(text);
            }
            if let Some(bindings) = context.view_bindings(node) {
                label(format!("element {}", bindings.element));
                for (field, at) in &bindings.fields {
                    label(format!("  {field} <- {at}"));
                }
            }
            #[cfg(feature = "reactive-trace")]
            if let Some(why) = context.why_updated(node) {
                for cause in &why.causes {
                    match cause.signal_created {
                        Some(created) => {
                            label(format!("  written {} (signal {created})", cause.written_at))
                        }
                        None => label(format!("  written {}", cause.written_at)),
                    }
                }
            }
        }
        self.frame.hovered = Some(hovered);
    }
}

/// Line height the labels are laid out with, in document pixels.
pub const LABEL_LINE: f32 = 16.0;

fn format_opt_motion_value(value: Option<MotionValue>) -> String {
    match value {
        Some(MotionValue::Scalar(scalar)) => format!("{scalar:.3}"),
        Some(MotionValue::Color([r, g, b, a])) => format!("rgba({r:.3},{g:.3},{b:.3},{a:.3})"),
        Some(MotionValue::Transform(transform)) => format!(
            "matrix({:.2},{:.2},{:.2},{:.2},{:.2},{:.2})",
            transform.a, transform.b, transform.c, transform.d, transform.e, transform.f
        ),
        Some(MotionValue::Filter(filter)) => {
            format!("filter(blur={:.2},op={:.2})", filter.0[6], filter.0[5])
        }
        Some(MotionValue::Discrete(tag)) => format!("discrete({tag})"),
        None => String::from("none"),
    }
}

fn round(value: f32) -> f32 {
    (value * 10.0).round() / 10.0
}

/// The edges between `outer` and the box `inner` inside it.
fn inset_edges(outer: LayoutBox, inner: LayoutBox) -> PaddingSpec {
    PaddingSpec {
        top: inner.y - outer.y,
        right: (outer.x + outer.width) - (inner.x + inner.width),
        bottom: (outer.y + outer.height) - (inner.y + inner.height),
        left: inner.x - outer.x,
    }
}

/// `rect` grown by `edges` (shrunk by negative ones, never past empty).
fn outset(rect: LayoutBox, edges: PaddingSpec) -> LayoutBox {
    LayoutBox {
        x: rect.x - edges.left,
        y: rect.y - edges.top,
        width: (rect.width + edges.left + edges.right).max(0.0),
        height: (rect.height + edges.top + edges.bottom).max(0.0),
    }
}

/// The node's used margin: percentages against its parent's content width,
/// as layout resolves them.
fn resolved_margin(context: &AppContext, node: StableNodeId) -> PaddingSpec {
    let world = context.world();
    let Some(style) = world.node_style(node) else {
        return PaddingSpec::default();
    };
    let percent_base = world
        .parent_id(node)
        .and_then(|parent| world.layout_result(parent))
        .map(|parent| parent.content_box.width);
    let fonts = world
        .computed_style(node)
        .map(|computed| FontSizeContext {
            element_px: computed.font_size,
            ..FontSizeContext::default()
        })
        .unwrap_or_default();
    style
        .layout
        .resolved_margin_against_fonts(percent_base, fonts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use nana_ui::runtime::view::widget;
    use nana_ui::runtime::{LayoutViewport, MeasureTextShaper, Stack};
    use nana_ui_core::LengthSpec;

    thread_local! {
        static COUNTING: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    }

    /// Counts the calling thread's allocations while `COUNTING` is set.
    struct CountingAllocator;

    // SAFETY: forwards every call unchanged to System; the accounting
    // touches only allocation-free thread-local cells.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if COUNTING.try_with(Cell::get).unwrap_or(false) {
                let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
            }
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            if COUNTING.try_with(Cell::get).unwrap_or(false) {
                let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
            }
            unsafe { System.realloc(pointer, layout, size) }
        }
    }

    #[global_allocator]
    static GLOBAL: CountingAllocator = CountingAllocator;

    fn counted<T>(run: impl FnOnce() -> T) -> (T, u64) {
        ALLOCATIONS.with(|count| count.set(0));
        COUNTING.with(|counting| counting.set(true));
        let value = run();
        COUNTING.with(|counting| counting.set(false));
        (value, ALLOCATIONS.with(Cell::get))
    }

    /// A 200×100 padded, bordered box with an 8px margin, inside a column.
    fn document() -> (AppContext, DocumentId) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        context
            .mount_view_root(document, || {
                let inner = Stack::column(0.0).hittable().with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(200.0));
                    layout.height = Some(LengthSpec::Px(100.0));
                    layout.margin = Some(LengthSpec::Px(8.0));
                    layout.padding = Some(LengthSpec::Px(10.0));
                    layout.border_width = Some(2.0);
                });
                widget(Stack::column(0.0)).children(widget(inner))
            })
            .unwrap();
        let nodes = context.world().document_order(document);
        context.resolve_styles(&nodes).unwrap();
        context.shape_text(&nodes, &mut MeasureTextShaper).unwrap();
        context
            .layout_document(document, LayoutViewport::new(400.0, 300.0))
            .unwrap();
        context.rebuild_hit_test(document);
        (context, document)
    }

    #[test]
    fn off_it_allocates_nothing_and_draws_nothing() {
        let (context, document) = document();
        let mut overlay = InspectOverlay::new();
        overlay.set_pointer(Some((50.0, 50.0)));
        let queries = context.world().hit_test_queries();
        let (empty, allocations) = counted(|| {
            let mut empty = true;
            for _ in 0..100 {
                empty &= overlay.update(&context, document).is_empty();
            }
            empty
        });
        assert!(empty);
        assert_eq!(allocations, 0);
        // Not even a hit test.
        assert_eq!(context.world().hit_test_queries(), queries);

        // Turning it off again clears what it drew while on.
        assert!(overlay.toggle());
        assert!(!overlay.update(&context, document).is_empty());
        assert!(!overlay.toggle());
        assert!(overlay.frame().is_empty());
        assert_eq!(overlay.frame().hovered, None);
    }

    #[test]
    fn on_it_draws_the_hovered_box_model_and_the_relayout() {
        let (context, document) = document();
        let mut overlay = InspectOverlay::new();
        overlay.set_enabled(true);
        // Inside the box: margin 8 puts its border box at (8, 8).
        overlay.set_pointer(Some((50.0, 50.0)));
        let frame = overlay.update(&context, document);

        let hovered = frame.hovered.as_ref().expect("a node under the pointer");
        let edges = |all: f32| PaddingSpec {
            top: all,
            right: all,
            bottom: all,
            left: all,
        };
        assert_eq!(hovered.margin, edges(8.0));
        assert_eq!(hovered.border, edges(2.0));
        assert_eq!(hovered.padding, edges(10.0));
        let border = hovered.border_box;
        assert_eq!((border.x, border.y), (8.0, 8.0));
        assert_eq!(
            hovered.margin_box,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: border.width + 16.0,
                height: border.height + 16.0,
            }
        );
        assert_eq!(hovered.content_box.x, 20.0);

        // The first layout recomputed every node: both are highlighted
        // under the four box-model bands.
        assert_eq!(frame.relaid_out, 2);
        let roles: Vec<_> = frame.quads.iter().map(|quad| quad.role).collect();
        assert_eq!(
            roles,
            [
                QuadRole::Relayout,
                QuadRole::Relayout,
                QuadRole::Margin,
                QuadRole::Border,
                QuadRole::Padding,
                QuadRole::Content,
            ]
        );
        let size = &frame.labels[0].text;
        assert!(size.contains("margin 8 8 8 8"), "{size}");
        assert!(size.contains("padding 10 10 10 10"), "{size}");

        // Only the selected layers draw.
        overlay.set_layers(InspectLayers {
            box_model: false,
            relayout: true,
            bindings: false,
            motion: false,
        });
        let frame = overlay.update(&context, document);
        assert_eq!(frame.quads.len(), 2);
        assert!(frame.labels.is_empty());
    }

    #[test]
    fn flush_preserves_relayout_scope_for_inspect_overlay() {
        use nana_ui::runtime::RuntimeDocument;
        use nana_ui::runtime::view::signal;

        let width_signal = std::cell::Cell::new(None);
        let document_id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(document_id);
        document
            .context_mut()
            .mount_view_root(document_id, || {
                let a_width = signal(100.0f32);
                width_signal.set(Some(a_width));
                let inner_a =
                    widget(Stack::column(0.0).hittable()).bind(move |stack: &mut Stack| {
                        let w = a_width.get();
                        *stack = Stack::column(0.0).hittable().with_layout(|layout| {
                            layout.width = Some(LengthSpec::Px(w));
                            layout.height = Some(LengthSpec::Px(40.0));
                        });
                    });
                let panel_a = widget(Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(200.0));
                    layout.height = Some(LengthSpec::Px(80.0));
                }))
                .children(inner_a);
                let inner_b = widget(Stack::column(0.0).hittable().with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(120.0));
                    layout.height = Some(LengthSpec::Px(60.0));
                }));
                let panel_b = widget(Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(200.0));
                    layout.height = Some(LengthSpec::Px(80.0));
                }))
                .children(inner_b);
                widget(Stack::column(0.0)).children((panel_a, panel_b))
            })
            .unwrap();

        let viewport = LayoutViewport::new(400.0, 300.0);
        document.flush(viewport, &mut MeasureTextShaper).unwrap();

        let mut overlay = InspectOverlay::new();
        overlay.set_enabled(true);
        let frame = overlay.update(document.context(), document_id);
        assert_eq!(frame.relaid_out, 5);
        let relayout_count = frame
            .quads
            .iter()
            .filter(|quad| quad.role == QuadRole::Relayout)
            .count();
        assert_eq!(relayout_count, 5);

        // An idle flush with no edits does not wipe the last layout scope.
        let idle = document.flush(viewport, &mut MeasureTextShaper).unwrap();
        assert!(idle.is_idle());
        let frame = overlay.update(document.context(), document_id);
        assert_eq!(frame.relaid_out, 5);

        // Mutating inner_a inside fixed-size panel_a leaves panel_b and
        // inner_b on their cached layout.
        width_signal.get().unwrap().set(150.0);
        document.flush(viewport, &mut MeasureTextShaper).unwrap();
        let frame = overlay.update(document.context(), document_id);
        assert!(frame.relaid_out > 0 && frame.relaid_out < 5, "{frame:?}");
        assert!(
            frame.quads.iter().any(
                |quad| quad.role == QuadRole::Relayout && (quad.rect.width - 150.0).abs() < 0.1
            ),
            "expected resized inner_a in relayout quads: {:?}",
            frame.quads
        );
        assert!(
            !frame.quads.iter().any(
                |quad| quad.role == QuadRole::Relayout && (quad.rect.width - 120.0).abs() < 0.1
            ),
            "expected untouched inner_b to reuse cached layout: {:?}",
            frame.quads
        );
    }

    #[cfg(feature = "reactive-trace")]
    #[test]
    fn reactive_trace_reports_signal_writes_in_overlay() {
        use nana_ui::runtime::RuntimeDocument;
        use nana_ui::runtime::view::signal;

        let width_signal = std::cell::Cell::new(None);
        let document_id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(document_id);
        document
            .context_mut()
            .mount_view_root(document_id, || {
                let a_width = signal(100.0f32);
                width_signal.set(Some(a_width));
                let box_a = widget(Stack::column(0.0).hittable()).bind(move |stack: &mut Stack| {
                    let w = a_width.get();
                    *stack = Stack::column(0.0).hittable().with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(w));
                        layout.height = Some(LengthSpec::Px(40.0));
                    });
                });
                widget(Stack::column(0.0)).children(box_a)
            })
            .unwrap();

        let viewport = LayoutViewport::new(400.0, 300.0);
        document.flush(viewport, &mut MeasureTextShaper).unwrap();
        width_signal.get().unwrap().set(160.0);
        document.flush(viewport, &mut MeasureTextShaper).unwrap();

        let mut overlay = InspectOverlay::new();
        overlay.set_enabled(true);
        overlay.set_pointer(Some((20.0, 20.0)));
        let frame = overlay.update(document.context(), document_id);
        assert!(frame.hovered.is_some());
        assert!(
            frame
                .labels
                .iter()
                .any(|label| label.text.contains("written ")),
            "expected signal write trace in labels: {:?}",
            frame.labels
        );
        assert!(
            frame
                .labels
                .iter()
                .any(|label| label.text.contains("signal ")),
            "expected signal creation location in labels: {:?}",
            frame.labels
        );
    }

    #[test]
    fn motion_inspector_renders_gpu_vs_cpu_handles_and_next_deadline() {
        use nana_ui::runtime::{
            AnimationId, AnimationPlayback, AnimationSpec, Easing, MotionTo, MutationQueue,
        };

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        context
            .mount_view_root(document, || {
                let gpu_box = Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(120.0));
                    layout.height = Some(LengthSpec::Px(40.0));
                });
                let cpu_box = Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(100.0));
                    layout.height = Some(LengthSpec::Px(40.0));
                });
                widget(Stack::column(0.0)).children((widget(gpu_box), widget(cpu_box)))
            })
            .unwrap();
        let nodes = context.world().document_order(document);
        let gpu_node = nodes[1];
        let cpu_node = nodes[2];

        let mut queue = MutationQueue::new();
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                gpu_node,
                Duration::ZERO,
                Duration::from_millis(200),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Opacity)
            .with_range(
                MotionValue::Scalar(1.0),
                MotionTo::Value(MotionValue::Scalar(0.25)),
            )
            .with_playback(AnimationPlayback::default()),
        );
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(2).unwrap(),
                cpu_node,
                Duration::ZERO,
                Duration::from_millis(200),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Width)
            .with_range(
                MotionValue::Scalar(100.0),
                MotionTo::Value(MotionValue::Scalar(180.0)),
            )
            .with_playback(AnimationPlayback::default()),
        );
        context.commit_mutations(queue).unwrap();
        context.advance_animations(Duration::from_millis(32));

        let nodes = context.world().document_order(document);
        context.resolve_styles(&nodes).unwrap();
        context.shape_text(&nodes, &mut MeasureTextShaper).unwrap();
        context
            .layout_document(document, LayoutViewport::new(400.0, 300.0))
            .unwrap();

        let mut overlay = InspectOverlay::new();
        overlay.set_layers(InspectLayers {
            box_model: false,
            relayout: false,
            bindings: false,
            motion: true,
        });

        // Off: zero allocations and empty motion rows.
        let (empty, allocations) = counted(|| overlay.update(&context, document).is_empty());
        assert!(empty);
        assert_eq!(allocations, 0);

        overlay.set_enabled(true);
        let frame = overlay.update_with_motion_annotation(&context, document, |entries| {
            for entry in entries {
                if entry.node == gpu_node.get() {
                    entry.layer = Some(gpu_node.get());
                }
            }
        });

        assert_eq!(frame.motion_rows.len(), 2);
        let gpu_row = frame
            .motion_rows
            .iter()
            .find(|row| row.node == gpu_node)
            .expect("gpu compositor row");
        assert_eq!(gpu_row.animation_class, AnimationClass::Compositor);
        assert_eq!(gpu_row.evaluator, MotionEvaluatorBackend::Gpu);
        assert_eq!(gpu_row.layer, Some(gpu_node.get()));
        assert!(gpu_row.has_live_handle);
        assert!(gpu_row.gpu_handle.is_some_and(|handle| !handle.is_null()));
        assert_eq!(gpu_row.cpu_fallback, None);
        assert!(gpu_row.next_deadline.is_some());
        assert_eq!(gpu_row.logical, Some(MotionValue::Scalar(1.0)));
        assert_ne!(gpu_row.presentation, gpu_row.logical);
        assert!(gpu_row.summary.contains("Class: Compositor"));
        assert!(gpu_row.summary.contains("Evaluator: GPU"));
        assert!(
            gpu_row
                .summary
                .contains(&format!("Layer: #{}", gpu_node.get()))
        );

        let cpu_row = frame
            .motion_rows
            .iter()
            .find(|row| row.node == cpu_node)
            .expect("cpu layout row");
        assert_eq!(cpu_row.animation_class, AnimationClass::Layout);
        assert_eq!(cpu_row.evaluator, MotionEvaluatorBackend::Cpu);
        assert_eq!(cpu_row.layer, None);
        assert!(!cpu_row.has_live_handle);
        assert!(cpu_row.cpu_fallback.is_some());
        assert!(cpu_row.next_deadline.is_some());
        assert!(cpu_row.summary.contains("Class: Layout"));
        assert!(cpu_row.summary.contains("Evaluator: CPU"));

        assert!(frame.quads.iter().any(|q| q.role == QuadRole::MotionGpu));
        assert!(frame.quads.iter().any(|q| q.role == QuadRole::MotionCpu));
        assert!(
            frame
                .labels
                .iter()
                .any(|l| l.text.contains("GPU handle: index=")),
            "expected live handle label: {:?}",
            frame.labels
        );
        assert!(
            frame
                .labels
                .iter()
                .any(|l| l.text.contains("GPU handle: none")),
            "expected absent handle label: {:?}",
            frame.labels
        );
        assert!(
            frame
                .labels
                .iter()
                .any(|l| l.text.contains("Next deadline:")),
            "expected next_deadline label: {:?}",
            frame.labels
        );
        assert!(
            frame
                .labels
                .iter()
                .any(|l| l.text.contains("Values: logical=")),
            "expected logical vs presentation value label: {:?}",
            frame.labels
        );
    }
}
