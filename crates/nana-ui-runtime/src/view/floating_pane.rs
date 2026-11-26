//! Draggable, viewport-clamped, proportional dual-corner resizable floating
//! panel with double-click reset ([`FloatingPane`]).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use nana_ui_core::{
    AlignSpec, ElevationRole, JustifySpec, LengthSpec, MotionRole, OverflowSpec, PositionSpec,
    RadiusTier, SemanticColorRole, SpacingStep,
};

use super::controls::StyledComponent;
use super::node::{AnyView, IntoView, ViewBuilder, widget};
use super::prop::{FieldWrite, IntoProp, PropSource};
use super::reactive::Signal;
use crate::{
    AccessibilityRole, AccessibilityState, Activate, ComponentView, InteractionState,
    MutationQueue, NodeKind, NodeStyle, PointerDragPhase, PointerDragged, StableNodeId, Stack,
    UiWorld,
};

/// Logical position and size of a [`FloatingPane`] inside its viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatingPaneRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Lower corner handle used to resize a [`FloatingPane`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeCorner {
    BottomLeft,
    BottomRight,
}

impl FloatingPaneRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Clamp size and position so the pane stays inside `viewport` with at
    /// least `margin` logical pixels from every edge and at least `min_size`.
    pub fn clamp_to_viewport(
        self,
        viewport: (f32, f32),
        min_size: (f32, f32),
        margin: f32,
    ) -> Self {
        let margin = margin.max(f32::default());
        let max_w = (viewport.0 - margin * 2.0).max(min_size.0.max(1.0));
        let max_h = (viewport.1 - margin * 2.0).max(min_size.1.max(1.0));
        let width = self.width.clamp(min_size.0.max(1.0), max_w);
        let height = self.height.clamp(min_size.1.max(1.0), max_h);
        let max_x = (viewport.0 - margin - width).max(margin);
        let max_y = (viewport.1 - margin - height).max(margin);
        Self {
            x: self.x.clamp(margin, max_x),
            y: self.y.clamp(margin, max_y),
            width,
            height,
        }
    }

    /// Translate by `(dx, dy)` and clamp inside `viewport`.
    pub fn dragged(
        self,
        dx: f32,
        dy: f32,
        viewport: (f32, f32),
        min_size: (f32, f32),
        margin: f32,
    ) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
        .clamp_to_viewport(viewport, min_size, margin)
    }

    /// Resize from `corner` by `(dx, dy)` while anchoring the opposite corner,
    /// optionally locking `width / height == aspect` and clamping inside
    /// `viewport`.
    #[allow(clippy::too_many_arguments)]
    pub fn resized_corner(
        self,
        corner: ResizeCorner,
        dx: f32,
        dy: f32,
        aspect: Option<f32>,
        viewport: (f32, f32),
        min_size: (f32, f32),
        margin: f32,
    ) -> Self {
        let margin = margin.max(f32::default());
        let min_w = min_size.0.max(1.0);
        let min_h = min_size.1.max(1.0);
        let right = self.x + self.width;
        let max_h = (viewport.1 - margin - self.y).max(min_h);
        let max_w = match corner {
            ResizeCorner::BottomRight => (viewport.0 - margin - self.x).max(min_w),
            ResizeCorner::BottomLeft => (right - margin).max(min_w),
        };
        let raw_w = match corner {
            ResizeCorner::BottomRight => self.width + dx,
            ResizeCorner::BottomLeft => self.width - dx,
        };
        let raw_h = self.height + dy;

        let (width, height) = if let Some(ratio) = aspect.filter(|r| r.is_finite() && *r > 0.0) {
            let eff_min_w = min_w.max(min_h * ratio);
            let eff_max_w = max_w.min(max_h * ratio).max(eff_min_w);
            let candidate_w = if dx.abs() >= dy.abs() {
                raw_w
            } else {
                raw_h * ratio
            };
            let w = candidate_w.clamp(eff_min_w, eff_max_w);
            (w, (w / ratio).clamp(min_h, max_h))
        } else {
            (raw_w.clamp(min_w, max_w), raw_h.clamp(min_h, max_h))
        };

        let x = match corner {
            ResizeCorner::BottomRight => self.x,
            ResizeCorner::BottomLeft => right - width,
        };
        Self {
            x,
            y: self.y,
            width,
            height,
        }
        .clamp_to_viewport(viewport, min_size, margin)
    }
}

struct FloatingPaneRectWrite;

impl FieldWrite<Stack, FloatingPaneRect> for FloatingPaneRectWrite {
    const FIELD: &'static str = "FloatingPane.rect";

    fn write(target: &mut Stack, rect: FloatingPaneRect) {
        let layout = Arc::make_mut(&mut target.node_style_mut().layout);
        layout.position = PositionSpec::Absolute;
        layout.offset_left = Some(LengthSpec::Px(rect.x));
        layout.offset_top = Some(LengthSpec::Px(rect.y));
        layout.width = Some(LengthSpec::Px(rect.width));
        layout.height = Some(LengthSpec::Px(rect.height));
    }

    fn differs(target: &Stack, rect: &FloatingPaneRect) -> bool {
        let layout = &target.node_style().layout;
        layout.offset_left != Some(LengthSpec::Px(rect.x))
            || layout.offset_top != Some(LengthSpec::Px(rect.y))
            || layout.width != Some(LengthSpec::Px(rect.width))
            || layout.height != Some(LengthSpec::Px(rect.height))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FloatingDragSurface {
    label: Arc<str>,
    style: NodeStyle,
}

impl FloatingDragSurface {
    fn new(label: impl Into<Arc<str>>) -> Self {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.direction = Some(crate::FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::SpaceBetween;
        Self {
            label: label.into(),
            style,
        }
    }
}

impl ComponentView for FloatingDragSurface {
    const TYPE_ID: Option<&'static str> = None;

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(|cx, entity| {
            cx.update_component(entity, |_, view_cx| {
                view_cx.emit(Activate);
            })?;
            Ok(true)
        }),
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
            tag: "floating-pane-drag-handle".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        crate::view_components::project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.label)),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Draggable, viewport-clamped, dual-corner resizable floating panel.
pub struct FloatingPane {
    rect: Signal<FloatingPaneRect>,
    default_rect: FloatingPaneRect,
    viewport: PropSource<(f32, f32)>,
    min_size: (f32, f32),
    aspect_ratio: Option<f32>,
    draggable: bool,
    resizable: bool,
    header: Option<AnyView>,
    content: AnyView,
    on_change: Option<Arc<Mutex<Box<dyn FnMut(FloatingPaneRect) + Send + 'static>>>>,
}

/// Create a [`FloatingPane`] driven by `rect` containing `content`.
pub fn floating_pane<V: IntoView>(rect: Signal<FloatingPaneRect>, content: V) -> FloatingPane {
    let initial = rect.get_untracked();
    let default_min = nana_ui_core::SpacingTokens::DEFAULT.resolve(SpacingStep::Page) * 4.0;
    FloatingPane {
        rect,
        default_rect: initial,
        viewport: PropSource::Const((initial.x + initial.width, initial.y + initial.height)),
        min_size: (default_min, default_min),
        aspect_ratio: None,
        draggable: true,
        resizable: true,
        header: None,
        content: content.into_any(),
        on_change: None,
    }
}

impl FloatingPane {
    /// Default geometry restored when the header is double-clicked.
    pub fn default_rect(mut self, rect: FloatingPaneRect) -> Self {
        self.default_rect = rect;
        self
    }

    /// Enclosing viewport `(width, height)` used to clamp drags and resizes.
    pub fn viewport(mut self, viewport: impl IntoProp<(f32, f32)>) -> Self {
        self.viewport = viewport.into_source();
        self
    }

    pub fn min_size(mut self, width: f32, height: f32) -> Self {
        self.min_size = (width.max(1.0), height.max(1.0));
        self
    }

    /// Lock proportional resizing to `width / height == ratio` when `Some`.
    pub fn aspect_ratio(mut self, ratio: Option<f32>) -> Self {
        self.aspect_ratio = ratio.filter(|r| r.is_finite() && *r > 0.0);
        self
    }

    pub fn draggable(mut self, draggable: bool) -> Self {
        self.draggable = draggable;
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    pub fn header<V: IntoView>(mut self, header: V) -> Self {
        self.header = Some(header.into_any());
        self
    }

    pub fn on_change(mut self, handler: impl FnMut(FloatingPaneRect) + Send + 'static) -> Self {
        self.on_change = Some(Arc::new(Mutex::new(Box::new(handler))));
        self
    }
}

fn publish_rect(
    rect: Signal<FloatingPaneRect>,
    cb: &Option<Arc<Mutex<Box<dyn FnMut(FloatingPaneRect) + Send + 'static>>>>,
    next: FloatingPaneRect,
) {
    rect.set(next);
    if let Some(cb) = cb
        && let Ok(mut f) = cb.lock()
    {
        f(next);
    }
}

impl IntoView for FloatingPane {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let rect = self.rect;
        let default_rect = self.default_rect;
        let viewport = Arc::new(Mutex::new(self.viewport));
        let min_size = self.min_size;
        let aspect_ratio = self.aspect_ratio;
        let on_change = self.on_change;
        let margin = nana_ui_core::SpacingTokens::DEFAULT.resolve(SpacingStep::Sm);

        let mut children: Vec<AnyView> = Vec::new();

        // Drag / double-click-reset header bar (when draggable or header supplied).
        if self.draggable || self.header.is_some() {
            let drag_origin: Arc<Mutex<Option<FloatingPaneRect>>> = Arc::new(Mutex::new(None));
            let last_click: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
            let double_click_window =
                nana_ui_core::MotionTokens::DEFAULT.duration(MotionRole::SidebarCollapse) * 2;
            let drag_cb = on_change.clone();
            let reset_cb = on_change.clone();
            let vp_reset = Arc::clone(&viewport);
            let vp_drag = Arc::clone(&viewport);

            let mut handle =
                widget(FloatingDragSurface::new("Move floating pane")).on(move |_: &Activate| {
                    let now = Instant::now();
                    let is_double = {
                        let mut guard = last_click.lock().unwrap();
                        let double = guard.is_some_and(|prev| {
                            now.saturating_duration_since(prev) <= double_click_window
                        });
                        *guard = if double { None } else { Some(now) };
                        double
                    };
                    if is_double {
                        let vp = super::untrack(|| vp_reset.lock().unwrap().get());
                        let clamped = default_rect.clamp_to_viewport(vp, min_size, margin);
                        publish_rect(rect, &reset_cb, clamped);
                    }
                });

            if self.draggable {
                handle = handle.on(move |drag: &PointerDragged| match drag.phase {
                    PointerDragPhase::Started => {
                        *drag_origin.lock().unwrap() = Some(rect.get_untracked());
                    }
                    PointerDragPhase::Moved => {
                        if let Some(start) = *drag_origin.lock().unwrap() {
                            let vp = super::untrack(|| vp_drag.lock().unwrap().get());
                            let next = start.dragged(drag.dx, drag.dy, vp, min_size, margin);
                            publish_rect(rect, &drag_cb, next);
                        }
                    }
                    PointerDragPhase::Ended => {
                        *drag_origin.lock().unwrap() = None;
                    }
                    PointerDragPhase::Cancelled => {
                        if let Some(start) = drag_origin.lock().unwrap().take() {
                            publish_rect(rect, &drag_cb, start);
                        }
                    }
                });
            }

            let handle_view = if let Some(header) = self.header {
                handle.children(header).into_any()
            } else {
                handle.into_any()
            };
            children.push(handle_view);
        }

        // Main content area.
        children.push(
            widget(
                Stack::column(f32::default())
                    .width(LengthSpec::Fill)
                    .with_layout(|l| {
                        l.flex_grow = Some(1.0);
                        l.min_height = Some(LengthSpec::Px(f32::default()));
                        l.overflow_x = OverflowSpec::Hidden;
                        l.overflow_y = OverflowSpec::Hidden;
                    }),
            )
            .children(self.content)
            .into_any(),
        );

        // Dual bottom-corner resize handles (bottom-left and bottom-right).
        if self.resizable {
            let grip_extent = nana_ui_core::SpacingTokens::DEFAULT.resolve(SpacingStep::Lg);
            for corner in [ResizeCorner::BottomLeft, ResizeCorner::BottomRight] {
                let resize_origin: Arc<Mutex<Option<FloatingPaneRect>>> =
                    Arc::new(Mutex::new(None));
                let resize_cb = on_change.clone();
                let vp_resize = Arc::clone(&viewport);
                let mut grip = Stack::row(f32::default()).with_layout(|l| {
                    l.position = PositionSpec::Absolute;
                    l.offset_bottom = Some(LengthSpec::Px(f32::default()));
                    l.width = Some(LengthSpec::Px(grip_extent));
                    l.height = Some(LengthSpec::Px(grip_extent));
                    match corner {
                        ResizeCorner::BottomLeft => {
                            l.offset_left = Some(LengthSpec::Px(f32::default()));
                        }
                        ResizeCorner::BottomRight => {
                            l.offset_right = Some(LengthSpec::Px(f32::default()));
                        }
                    }
                });
                grip.node_style_mut().interaction.hovered.background =
                    Some(SemanticColorRole::Hover);
                let grip_view = widget(grip)
                    .on(move |drag: &PointerDragged| match drag.phase {
                        PointerDragPhase::Started => {
                            *resize_origin.lock().unwrap() = Some(rect.get_untracked());
                        }
                        PointerDragPhase::Moved => {
                            if let Some(start) = *resize_origin.lock().unwrap() {
                                let vp = super::untrack(|| vp_resize.lock().unwrap().get());
                                let next = start.resized_corner(
                                    corner,
                                    drag.dx,
                                    drag.dy,
                                    aspect_ratio,
                                    vp,
                                    min_size,
                                    margin,
                                );
                                publish_rect(rect, &resize_cb, next);
                            }
                        }
                        PointerDragPhase::Ended => {
                            *resize_origin.lock().unwrap() = None;
                        }
                        PointerDragPhase::Cancelled => {
                            if let Some(start) = resize_origin.lock().unwrap().take() {
                                publish_rect(rect, &resize_cb, start);
                            }
                        }
                    })
                    .into_any();
                children.push(grip_view);
            }
        }

        let mut shell = Stack::column(f32::default())
            .radius(RadiusTier::Md)
            .surface(SemanticColorRole::Surface)
            .border(SemanticColorRole::BorderSoft, nana_ui_core::HAIRLINE)
            .with_layout(|l| {
                l.position = PositionSpec::Absolute;
                l.overflow_x = OverflowSpec::Hidden;
                l.overflow_y = OverflowSpec::Hidden;
                l.paint.elevation = Some(ElevationRole::Overlay);
            });
        let initial = rect.get_untracked();
        FloatingPaneRectWrite::write(&mut shell, initial);

        widget(shell)
            .prop::<FloatingPaneRect, FloatingPaneRectWrite>(move || rect.get())
            .children(children)
            .build(vb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_pane_drag_and_dual_corner_proportional_resize_clamp_to_viewport() {
        let vp = (800.0, 600.0);
        let min = (120.0, 90.0);
        let start = FloatingPaneRect::new(100.0, 80.0, 200.0, 100.0);

        // Dragging past left/top edge clamps to margin.
        let dragged = start.dragged(-500.0, -500.0, vp, min, 8.0);
        assert_eq!(dragged.x, 8.0);
        assert_eq!(dragged.y, 8.0);
        assert_eq!((dragged.width, dragged.height), (200.0, 100.0));

        // Proportional resize from bottom-right keeps 2:1 aspect ratio.
        let br = start.resized_corner(
            ResizeCorner::BottomRight,
            100.0,
            10.0,
            Some(2.0),
            vp,
            min,
            8.0,
        );
        assert_eq!(br.x, 100.0);
        assert!((br.width - 300.0).abs() < 0.01);
        assert!((br.height - 150.0).abs() < 0.01);

        // Proportional resize from bottom-left anchors right edge (300.0) and keeps 2:1 ratio.
        let bl = start.resized_corner(
            ResizeCorner::BottomLeft,
            -60.0,
            0.0,
            Some(2.0),
            vp,
            min,
            8.0,
        );
        assert!((bl.x + bl.width - 300.0).abs() < 0.01);
        assert!((bl.width - 260.0).abs() < 0.01);
        assert!((bl.height - 130.0).abs() < 0.01);
    }
}
