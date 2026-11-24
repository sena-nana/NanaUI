#![allow(unused_imports)]
use std::sync::Arc;

use super::super::ComponentView;
use super::super::component_view::{
    TextFieldProjection, apply_input_recipe, compact_icon_style, control_layout,
    format_range_value, project_common, project_text_field, range_field_style, shared_label,
    text_area_footer_extent, text_field_style, tool_box, with_unit,
};
use super::super::events::*;
use crate::layout_edits::{LayoutEdit, LengthBits, StackPreset, edit_layout};
use crate::{
    AccessibilityRole, AccessibilityState, HighlightRequest, InteractionState, MutationQueue,
    NodeKind, NodeStyle, OverlayHostState, ScrollOffset, SemanticPaint, StableNodeId, TextCodeFold,
    TextColorSwatchSpan, TextCompletion, TextContent, TextDiagnosticSpan, TextEditorRenderOptions,
    TextGitMark, TextHorizontalAlignment, TextHover, TextInlay, TextInputState, TextMatchSpan,
    TextSignatureHelp, TextVerticalAlignment, UiWorld,
};
use crate::{AppContext, Entity};

/// A retained row's vertical position relative to its scroll viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollAnchor {
    pub row: StableNodeId,
    pub viewport_y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollView {
    /// Follow the vertical end whenever layout publishes new scroll geometry.
    pub follow_end: bool,
    pub(crate) pending_anchor: Option<ScrollAnchor>,
    pub axes: ScrollAxes,
    pub label: Option<Arc<str>>,
    /// This view's own scrollbar visibility. `None` follows the installed
    /// theme's policy ([`nana_ui_core::ScrollbarRecipe::visibility`]).
    pub scrollbars: Option<nana_ui_core::ScrollbarVisibility>,
    /// Pointer is inside the container, so auto-hiding bars show.
    pub hovered: bool,
    /// The axis whose lane the pointer is on: its thumb grows and its track
    /// shows.
    pub hot: Option<nana_ui_core::ScrollbarAxis>,
    /// A scroll, a drag ending or the pointer leaving keeps auto-hiding bars
    /// shown until the theme's hide delay runs out.
    pub held: bool,
    pub dragging: Option<ScrollbarDragState>,
    pub style: NodeStyle,
}

/// Transient thumb drag. The scroll offset stays authoritative in the world;
/// only the grab anchor lives here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarDragState {
    pub pointer_id: u64,
    pub axis: nana_ui_core::ScrollbarAxis,
    /// Distance from the thumb's leading edge to the grab point.
    pub grab_offset: f32,
    pub initial_offset: ScrollOffset,
}

impl ScrollView {
    /// Keep the newest content visible after layout. Applications turn this
    /// off while the user reads older content.
    pub fn follow_end(mut self, enabled: bool) -> Self {
        self.follow_end = enabled;
        self
    }

    pub fn new(axes: ScrollAxes) -> Self {
        Self {
            follow_end: false,
            pending_anchor: None,
            axes,
            label: None,
            scrollbars: None,
            hovered: false,
            hot: None,
            held: false,
            dragging: None,
            style: NodeStyle::default(),
        }
    }

    /// Edit the layout in place (size, flex), keeping the rest of the style.
    pub fn with_layout(mut self, f: impl FnOnce(&mut nana_ui_core::LayoutStyle)) -> Self {
        f(Arc::make_mut(&mut self.style.layout));
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Reach `gutter` out into the container's margin on both sides and inset
    /// the content by as much: the content keeps its container's content
    /// line and width, while the scrollport — and the scrollbar drawn at its
    /// edge — runs out into the margin, so the bar sits in the empty margin
    /// instead of over the content, and the margin scrolls the list too. A
    /// margin narrower than the theme's lane and its inset narrows the lane
    /// to fit ([`nana_ui_core::ScrollbarSkin::room`]). Give the container a
    /// horizontal padding of at least `gutter`; a
    /// [`scroll_foot_fade`](crate::view::scroll_foot_fade) over the area
    /// takes the same gutter (`clear_of`).
    ///
    /// Pixels hold under one theme only: in a Card, whose padding follows
    /// the metrics, prefer [`Self::gutter_theme`] with
    /// [`nana_ui_core::ThemeToken::PanelPaddingX`]. A later call replaces an
    /// earlier one of either kind.
    pub fn gutter(mut self, gutter: f32) -> Self {
        use nana_ui_core::LengthSpec::Px;
        let gutter = gutter.max(0.0);
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.margin_left = Some(Px(-gutter));
        layout.margin_right = Some(Px(-gutter));
        layout.padding_left = Some(Px(gutter));
        layout.padding_right = Some(Px(gutter));
        if !layout.theme.is_empty() {
            let theme = &mut *layout.theme;
            theme.margin_left = None;
            theme.margin_right = None;
            theme.padding_left = None;
            theme.padding_right = None;
            theme.scrollbar_room = None;
        }
        let skin = layout.paint.scrollbar.unwrap_or_default();
        layout.paint.scrollbar = Some(nana_ui_core::ScrollbarSkin {
            room: Some(gutter),
            ..skin
        });
        self
    }

    /// [`Self::gutter`] as a theme value, resolved against the installed
    /// theme and again whenever it changes: the margin, the content inset
    /// and the bar's room all follow it. Inside a Card, name the card's own
    /// padding, `gutter_theme(ThemeToken::PanelPaddingX)`, and the area runs
    /// exactly to the card's edge under any metrics.
    pub fn gutter_theme(mut self, gutter: impl Into<nana_ui_core::ThemeExpr>) -> Self {
        let gutter = gutter.into();
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.margin_left = None;
        layout.margin_right = None;
        layout.padding_left = None;
        layout.padding_right = None;
        let theme = &mut *layout.theme;
        theme.margin_left = Some(gutter.clone().scaled(-1.0));
        theme.margin_right = Some(gutter.clone().scaled(-1.0));
        theme.padding_left = Some(gutter.clone());
        theme.padding_right = Some(gutter.clone());
        theme.scrollbar_room = Some(gutter);
        self
    }

    /// Reach as far above the content as heading tools hang out of their
    /// line ([`heading_overhang`](crate::view::heading_overhang)), with the
    /// content inset by as much, so a heading at the top keeps its tools
    /// whole instead of having them clipped by the scrollport.
    pub fn heading_room(mut self) -> Self {
        let room = crate::view::heading_overhang();
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.theme.margin_top = Some(room.clone().scaled(-1.0));
        layout.theme.padding_top = Some(room);
        self
    }

    /// Fade the content out toward the top and bottom edges of the
    /// scrollport, `top` and `bottom` logical pixels deep. The view and
    /// everything it holds composite as one layer whose alpha ramps to zero
    /// at each edge, so whatever is behind the view shows through — floating
    /// cards over a live stage included — where
    /// [`scroll_foot_fade`](crate::view::scroll_foot_fade) would paint a
    /// surface colour. The bars stay unfaded.
    ///
    /// A band reaches only as far as there is content past its edge: the
    /// top one grows in as the content scrolls away from its start, the
    /// bottom one shrinks away as the end arrives, so neither pops. Zero
    /// turns a band off; both zero is no fade. Stored on the view's style
    /// ([`nana_ui_core::PaintStyle::scroll_edge_fade`]), so call it after
    /// [`Self::style`].
    pub fn edge_fade(mut self, top: f32, bottom: f32) -> Self {
        let fade = nana_ui_core::ScrollEdgeFade::new(top, bottom);
        Arc::make_mut(&mut self.style.layout).paint.scroll_edge_fade =
            fade.is_active().then_some(fade);
        self
    }

    /// Choose overlay auto-hide, resident, or no bars at all for this view,
    /// whatever the theme's policy says.
    pub fn scrollbars(mut self, visibility: nana_ui_core::ScrollbarVisibility) -> Self {
        self.scrollbars = Some(visibility);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Whether this view asks its auto-hiding bars to show: the pointer is
    /// inside it, a lane is hot, a thumb is dragged, or a hold after a scroll
    /// or a leave has not run out. Resident and hidden bars do not ask; what
    /// is actually drawn is [`crate::AppContext::scrollbars_visible`].
    pub fn scrollbars_revealed(&self) -> bool {
        self.hovered || self.hot.is_some() || self.dragging.is_some() || self.held
    }

    /// This view's visibility, else the installed theme's policy.
    pub fn scrollbar_visibility(&self, world: &UiWorld) -> nana_ui_core::ScrollbarVisibility {
        self.scrollbars
            .unwrap_or_else(|| world.theme().recipes().scrollbar().visibility)
    }

    pub(crate) fn scrollbar_visibility_for(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> nana_ui_core::ScrollbarVisibility {
        self.scrollbars
            .unwrap_or_else(|| world.node_theme(id).recipes().scrollbar().visibility)
    }

    fn projected_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        if matches!(self.axes, ScrollAxes::Horizontal | ScrollAxes::Both) {
            layout.overflow_x = nana_ui_core::OverflowSpec::Scroll;
        }
        if matches!(self.axes, ScrollAxes::Vertical | ScrollAxes::Both) {
            layout.overflow_y = nana_ui_core::OverflowSpec::Scroll;
        }
        style
    }

    /// Project scrollport style, interaction, and accessibility without the
    /// scrollbar visual.
    ///
    /// Containers that lend their own node to a scrollport — [`SidebarFrame`]'s
    /// body, for one — use this so they do not stamp a scrollbar over whatever
    /// visual the node's real component owns.
    ///
    /// [`SidebarFrame`]: crate::SidebarFrame
    pub fn project_scrollport(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        project_common(
            id,
            world,
            mutations,
            &self.projected_style(),
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Open [`WidgetVisual`](crate::WidgetVisual) backing a [`ScrollView`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarVisual {
    pub axes: ScrollAxes,
    pub visibility: nana_ui_core::ScrollbarVisibility,
    pub shown: bool,
    pub hot: Option<nana_ui_core::ScrollbarAxis>,
    pub dragging: Option<nana_ui_core::ScrollbarAxis>,
    pub presence: f32,
    pub expand: [f32; 2],
}

impl ScrollbarVisual {
    /// Returns `true` when at least auto-hide/resident visibility allows showing a bar.
    pub fn is_shown(&self) -> bool {
        match self.visibility {
            nana_ui_core::ScrollbarVisibility::AutoHide => self.presence > 0.0,
            nana_ui_core::ScrollbarVisibility::Always => true,
            nana_ui_core::ScrollbarVisibility::Hidden => false,
        }
    }

    pub(crate) fn targets(&self) -> crate::components::ScrollbarTargets {
        crate::components::ScrollbarTargets::new(
            self.visibility,
            self.shown,
            self.hot,
            self.dragging,
        )
    }

    pub(crate) fn bars_in(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: crate::LayoutBox,
        source_layout: &nana_ui_core::LayoutStyle,
        style_model: nana_ui_core::StyleModelRef,
        theme: &crate::CompiledTheme,
    ) -> Option<crate::ScrollbarBars> {
        let presence = match self.visibility {
            nana_ui_core::ScrollbarVisibility::AutoHide => self.presence,
            nana_ui_core::ScrollbarVisibility::Always => 1.0,
            nana_ui_core::ScrollbarVisibility::Hidden => return None,
        };
        // A bar that has faded out is gone: it neither paints nor takes the pointer.
        if presence.is_nan() || presence <= 0.0 {
            return None;
        }
        let metrics = world.scroll_metrics(id)?;
        let offset = world.scroll_offset(id).unwrap_or_default();
        let skin = source_layout.paint.scrollbar;
        let base = style_model.metrics.scrollbar;
        let chrome = skin.map(|skin| skin.metrics(base)).unwrap_or(base);
        let recipe = theme.recipes().scrollbar();
        let edges = source_layout.resolved_border_edges();
        let padding_box = nana_ui_core::ScrollbarRect {
            x: bounds.x + edges.left,
            y: bounds.y + edges.top,
            width: (bounds.width - edges.left - edges.right).max(0.0),
            height: (bounds.height - edges.top - edges.bottom).max(0.0),
        };
        let scrolls = |axis: nana_ui_core::ScrollbarAxis| match axis {
            nana_ui_core::ScrollbarAxis::Horizontal => {
                self.axes.horizontal() && metrics.content_width > metrics.viewport_width
            }
            nana_ui_core::ScrollbarAxis::Vertical => {
                self.axes.vertical() && metrics.content_height > metrics.viewport_height
            }
        };
        let layout_box = |rect: nana_ui_core::ScrollbarRect| crate::LayoutBox {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        };
        let bar = |axis: nana_ui_core::ScrollbarAxis| {
            if !scrolls(axis) {
                return None;
            }
            let horizontal = matches!(axis, nana_ui_core::ScrollbarAxis::Horizontal);
            let corner = scrolls(if horizontal {
                nana_ui_core::ScrollbarAxis::Vertical
            } else {
                nana_ui_core::ScrollbarAxis::Horizontal
            });
            let extent = if horizontal {
                nana_ui_core::ScrollbarExtent {
                    viewport: metrics.viewport_width,
                    content: metrics.content_width,
                    offset: offset.x,
                    min_offset: metrics.origin_x,
                }
            } else {
                nana_ui_core::ScrollbarExtent {
                    viewport: metrics.viewport_height,
                    content: metrics.content_height,
                    offset: offset.y,
                    min_offset: metrics.origin_y,
                }
            };
            let grown = self.expand[usize::from(!horizontal)];
            let layout =
                nana_ui_core::scrollbar_layout(axis, padding_box, corner, extent, grown, chrome)?;
            let paint = recipe.paint(
                style_model,
                skin,
                nana_ui_core::ScrollbarPaintState {
                    presence,
                    expand: grown,
                    dragging: self.dragging == Some(axis),
                    resident: matches!(self.visibility, nana_ui_core::ScrollbarVisibility::Always),
                },
            );
            Some(crate::ScrollbarBar {
                lane: layout_box(layout.lane),
                track: layout_box(layout.track),
                thumb: layout_box(layout.thumb),
                track_background: paint.track,
                thumb_background: paint.thumb,
                border_width: theme.border().hairline,
                track_border: paint.track_border,
                thumb_border: paint.thumb_border,
                track_radius: layout.track_radius,
                thumb_radius: layout.thumb_radius,
                min_offset: layout.axis.min_offset,
                max_offset: layout.axis.max_offset,
            })
        };
        let horizontal = bar(nana_ui_core::ScrollbarAxis::Horizontal);
        let vertical = bar(nana_ui_core::ScrollbarAxis::Vertical);
        (horizontal.is_some() || vertical.is_some()).then_some(crate::ScrollbarBars {
            horizontal,
            vertical,
        })
    }
}

impl crate::WidgetVisual for ScrollbarVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_SCROLLBAR)
    }

    fn is_valid(&self) -> bool {
        std::iter::once(self.presence)
            .chain(self.expand)
            .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
    }

    fn is_scroll_container(&self) -> bool {
        true
    }

    fn has_scrollbar_chrome(&self) -> bool {
        self.visibility != nana_ui_core::ScrollbarVisibility::Hidden
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let mut out = crate::WidgetPaintOutput {
            has_scrollbar_chrome: self.has_scrollbar_chrome(),
            ..crate::WidgetPaintOutput::new()
        };
        if let Some(bars) = self.bars_in(
            ctx.id,
            ctx.world,
            ctx.bounds,
            &ctx.source_style.layout,
            ctx.style_model,
            ctx.theme,
        ) {
            // Track then thumb, the horizontal bar first: vertical paints last.
            for (first, bar) in [(0, bars.horizontal), (2, bars.vertical)] {
                let Some(bar) = bar else {
                    continue;
                };
                if let Some(background) = bar.track_background {
                    out.push(
                        crate::scrollbar_chrome_slot(first),
                        crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                            rect: bar.track,
                            fill: Some(background),
                            border: bar.track_border.map(|color| (color, bar.border_width)),
                            corner_radius: [bar.track_radius; 4],
                            ..crate::WidgetQuad::default()
                        }),
                    );
                }
                out.push(
                    crate::scrollbar_chrome_slot(first + 1),
                    crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                        rect: bar.thumb,
                        fill: Some(bar.thumb_background),
                        border: bar.thumb_border.map(|color| (color, bar.border_width)),
                        corner_radius: [bar.thumb_radius; 4],
                        ..crate::WidgetQuad::default()
                    }),
                );
            }
        }
        out
    }
}

impl ScrollView {
    pub(crate) fn project_scrollbar_visual(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        let visibility = self.scrollbar_visibility_for(id, world);
        let shown = self.scrollbars_revealed();
        let dragging = self.dragging.map(|drag| drag.axis);
        let targets =
            crate::components::ScrollbarTargets::new(visibility, shown, self.hot, dragging);
        // The bars carry on from where their tracks have them; the world
        // starts, turns or cancels the tracks as this visual moves their
        // targets. They land at once on the first projection and under
        // reduced motion, and only an auto-hiding bar fades.
        let stored = world.widget_visual_as::<ScrollbarVisual>(id).copied();
        let (presence, expand) = match stored {
            Some(ScrollbarVisual {
                presence, expand, ..
            }) if !world.reduced_motion() => (presence, expand),
            _ => (targets.presence, targets.expand),
        };
        let visual = ScrollbarVisual {
            axes: self.axes,
            visibility,
            shown,
            hot: self.hot,
            dragging,
            presence: if visibility == nana_ui_core::ScrollbarVisibility::AutoHide {
                presence
            } else {
                targets.presence
            },
            expand: if visibility == nana_ui_core::ScrollbarVisibility::Hidden {
                targets.expand
            } else {
                expand
            },
        };
        if stored.as_ref() != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
    }
}

static SCROLLBAR_CAPABILITIES: crate::framework::ScrollbarCapabilities =
    crate::framework::ScrollbarCapabilities {
        near_hit: AppContext::scrollbar_target_near_hit,
        hot_sync: AppContext::sync_scrollbar_hot,
        begin_drag: AppContext::begin_scrollbar_drag,
    };

static SCROLL_VIEW_POINTER_OPS: crate::framework::PointerOps = crate::framework::PointerOps {
    apply_scroll: Some(AppContext::apply_scroll_view_to),
    hold_scrollbars: Some(AppContext::hold_scrollbars),
    ..crate::framework::PointerOps::EMPTY
};

impl ComponentView for ScrollView {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::SCROLL_VIEW.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(|caps| {
            caps.scrollbar = Some(&SCROLLBAR_CAPABILITIES);
            caps.add_frame_ticker(AppContext::release_scrollbar_holds);
        }),
        lifecycle: Some(crate::framework::lifecycle_hooks::scroll_view),
        pointer_ops: Some(&SCROLL_VIEW_POINTER_OPS),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        next.hovered = self.hovered;
        next.hot = self.hot;
        next.held = self.held;
        next.dragging = self.dragging;
        next.pending_anchor = self.pending_anchor;
        *self = next;
    }

    fn wants_recipe_reproject() -> bool {
        true
    }

    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::RECIPE_SCROLLBAR
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "scroll".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.project_scrollbar_visual(id, world, mutations);
        self.project_scrollport(id, world, mutations);
    }
}
