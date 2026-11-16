//! Compact list-row image slot. NanaUI never stores pixels or codecs.
//!
//! Hosts declare a HostTexture slot and an optional aspect. The box is
//! [`ControlSize`] height × aspect (default 1:1). Empty, loading, ready, and
//! unavailable share that geometry. Ready samples `"nana.host-texture"` with
//! configurable fit (Contain by default); empty identities omit the Scene node.
//!
//! A [`Thumbnail::pickable`] one also reports where on its picture the
//! pointer is ([`ThumbnailPick`]), in coordinates of the picture itself, so a
//! host can map a hover or a press onto its own image data.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ContentFit, ControlHeight, ControlSize, ImageSampling, LengthSpec, LogicalRect,
    OverflowSpec, PointerEventsSpec, PositionSpec, RadiusTier, SemanticColorRole, SquareSize,
    ThemeMetrics, space,
};
use nana_ui_input::InputModifiers;

use crate::gpu_slots::TextureSlotName;
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, CustomRenderNode, Entity,
    FrameworkError, HOST_TEXTURE_RENDERER, InteractionState, MutationQueue, NodeKind, NodePointer,
    NodePointerHooks, NodeStyle, SpinnerVisual, StableNodeId, Stack, TextContent, UiWorld,
};

/// Width ÷ height when the host does not declare an aspect.
pub const DEFAULT_ASPECT: f32 = 1.0;

/// Loading glyph: twice the compact [`crate::Spinner`] (14 → 28). Scene clamps to the box.
const SPINNER_SIZE: f32 = nana_ui_core::UI_METRICS.icon_button_size;

/// Presentation of a [`Thumbnail`] box. All four states keep the same size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThumbnailState {
    #[default]
    Empty,
    Loading,
    Ready,
    Unavailable,
}

/// A point on a [`Thumbnail`]'s picture: `0, 0` is its top-left corner and
/// `1, 1` its bottom-right, whatever the box's size and fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThumbnailPoint {
    pub x: f32,
    pub y: f32,
}

/// What the pointer does on a [`Thumbnail::pickable`] thumbnail's picture.
///
/// Only the picture counts: the bands [`ContentFit::Contain`] leaves around
/// it are not part of it. The control does not take focus; a host that picks
/// from the picture offers the same choices in a keyboard-reachable list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThumbnailPick {
    /// The pointer moved on the picture (`Some`), or off it (`None`: into a
    /// band, out of the box, or the press it held was cancelled). `None`
    /// is said once per time it leaves.
    Hover(Option<ThumbnailPoint>),
    /// A primary press on the picture, with the modifier keys held.
    Press {
        point: ThumbnailPoint,
        modifiers: InputModifiers,
    },
}

/// Compact image control for [`crate::ListItem`] leading slots.
///
/// Pointer events stay off so the parent row owns hit-testing, unless the
/// thumbnail is [`Self::pickable`]. Overlay children (see [`Self::badge`])
/// are clipped to this box's radius.
#[derive(Debug, Clone, PartialEq)]
pub struct Thumbnail {
    /// The registry slot shown. Re-registering it is all a host does to show
    /// new pixels; the control carries no revision of its own.
    pub resource: TextureSlotName,
    pub state: ThumbnailState,
    pub size: ControlSize,
    pub aspect: f32,
    pub fit: ContentFit,
    /// Level 0 by default (the host sizes the slot from `painted_extent`);
    /// [`ImageSampling::Mipmap`] samples the host's mip chain trilinearly.
    pub sampling: ImageSampling,
    pub label: Arc<str>,
    /// Paint the thumbnail while omitting it from the accessibility tree.
    /// Useful for repeated/decorative layers such as a collection cover.
    pub decorative: bool,
    /// Report the pointer's place on the picture as [`ThumbnailPick`]
    /// instead of leaving hit-testing to the parent.
    pub pickable: bool,
    /// The slot's texture size in pixels, which places the picture inside
    /// the box for `fit`. Without it the whole box is taken as the picture,
    /// which is exact only for [`ContentFit::Fill`] or a box of the
    /// texture's aspect.
    pub intrinsic_size: Option<(u32, u32)>,
    pub style: NodeStyle,
    /// Whether the last [`ThumbnailPick::Hover`] was on the picture.
    hovering: bool,
}

impl Thumbnail {
    /// Ready when `resource` is non-empty after trim; otherwise empty.
    pub fn new(resource: impl Into<TextureSlotName>) -> Self {
        let resource = resource.into();
        let state = if resource.is_empty() {
            ThumbnailState::Empty
        } else {
            ThumbnailState::Ready
        };
        Self {
            resource,
            state,
            size: ControlSize::Small,
            aspect: DEFAULT_ASPECT,
            fit: ContentFit::Contain,
            sampling: ImageSampling::Resample,
            label: Arc::from(""),
            decorative: false,
            pickable: false,
            intrinsic_size: None,
            style: NodeStyle::default(),
            hovering: false,
        }
    }

    pub fn empty() -> Self {
        Self::new("").state(ThumbnailState::Empty)
    }

    pub fn loading() -> Self {
        Self::new("").state(ThumbnailState::Loading)
    }

    pub fn unavailable() -> Self {
        Self::new("").state(ThumbnailState::Unavailable)
    }

    pub const fn state(mut self, state: ThumbnailState) -> Self {
        self.state = state;
        self
    }

    pub fn aspect(mut self, aspect: f32) -> Self {
        self.aspect = sanitize_aspect(aspect);
        self
    }

    pub fn fit(mut self, fit: ContentFit) -> Self {
        self.fit = fit;
        self
    }

    /// How the slot's texture is sampled; see [`ImageSampling`].
    pub const fn sampling(mut self, sampling: ImageSampling) -> Self {
        self.sampling = sampling;
        self
    }

    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = label.into();
        self
    }

    /// Keep the thumbnail visible while exposing it as decorative content.
    /// The thumbnail and its descendants are omitted from accessibility
    /// projection, equivalent to `aria-hidden="true"`.
    pub const fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Report hovers and presses on the picture as [`ThumbnailPick`].
    pub const fn pickable(mut self) -> Self {
        self.pickable = true;
        self
    }

    /// The slot's texture size in pixels; see [`Self::intrinsic_size`].
    /// A zero side clears it.
    pub fn intrinsic_size(mut self, width: u32, height: u32) -> Self {
        self.intrinsic_size = (width > 0 && height > 0).then_some((width, height));
        self
    }

    /// Where the picture sits in a `width × height` box: the box itself
    /// unless [`Self::intrinsic_size`] is known, then that size under `fit`.
    pub fn picture_rect(&self, width: f32, height: f32) -> LogicalRect {
        let frame = LogicalRect::new(0.0, 0.0, width, height);
        match self.intrinsic_size {
            Some((w, h)) => frame.fitted(w as f32, h as f32, self.fit),
            None => frame,
        }
    }

    /// The point on the picture under `x, y` (px from the box's top-left in
    /// a `width × height` box), or `None` off the picture.
    pub fn picture_point(&self, width: f32, height: f32, x: f32, y: f32) -> Option<ThumbnailPoint> {
        let picture = self.picture_rect(width, height);
        let frame = LogicalRect::new(0.0, 0.0, width, height);
        let on = |v: f32, start: f32, len: f32| v >= start && v <= start + len;
        // the box clips a covering picture: only what shows can be picked
        if picture.width <= 0.0
            || picture.height <= 0.0
            || !on(x, picture.x, picture.width)
            || !on(y, picture.y, picture.height)
            || !on(x, frame.x, frame.width)
            || !on(y, frame.y, frame.height)
        {
            return None;
        }
        Some(ThumbnailPoint {
            x: (x - picture.x) / picture.width,
            y: (y - picture.y) / picture.height,
        })
    }

    /// Corner overlay chrome for duration, viewer counts, and similar cover
    /// metadata. Absolute, not hittable, clipped by this thumbnail's radius.
    pub fn badge() -> Stack {
        Stack::row(0.0)
            .surface(SemanticColorRole::Background)
            .radius(nana_ui_core::RadiusTier::Sm)
            .with_layout(|layout| {
                layout.position = PositionSpec::Absolute;
                layout.offset_right = Some(LengthSpec::Px(space::SM));
                layout.offset_bottom = Some(LengthSpec::Px(space::SM));
                layout.theme.offset_right =
                    Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
                layout.theme.offset_bottom =
                    Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
                layout.theme.padding_left =
                    Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
                layout.theme.padding_right =
                    Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
                layout.theme.padding_top =
                    Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
                layout.theme.padding_bottom =
                    Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
                layout.z_index = Some(2);
                layout.padding_left = Some(LengthSpec::Px(space::SM));
                layout.padding_right = Some(LengthSpec::Px(space::SM));
                layout.padding_top = Some(LengthSpec::Px(space::XXS));
                layout.padding_bottom = Some(LengthSpec::Px(space::XXS));
                layout.align_items = AlignSpec::Center;
                layout.pointer_events = Some(PointerEventsSpec::None);
            })
    }

    /// Logical box: control height × host aspect (default 1:1).
    pub fn box_extent(&self, metrics: ThemeMetrics) -> (f32, f32) {
        let height = self.size.height_in(metrics);
        (height * sanitize_aspect(self.aspect), height)
    }

    /// Host-texture Scene node. Only Ready with a non-empty slot attaches one.
    pub fn custom_render(&self) -> Option<CustomRenderNode> {
        if self.state != ThumbnailState::Ready || self.resource.is_empty() {
            return None;
        }
        Some(
            CustomRenderNode::new(HOST_TEXTURE_RENDERER, self.resource.to_resource(), 0)
                .with_fit(self.fit)
                .with_sampling(self.sampling),
        )
    }

    fn effective_style(&self, _world: &UiWorld) -> NodeStyle {
        let mut style = self.style.clone();
        style.background = match self.state {
            ThumbnailState::Ready => None,
            _ => Some(SemanticColorRole::Subtle),
        };
        style.border = None;
        style.foreground = Some(SemanticColorRole::Muted);
        let unset_width = style.layout.width.is_none();
        let unset_height = style.layout.height.is_none();
        // A host that names a tier or a length keeps it; only an unrounded
        // thumbnail takes the compact default.
        let unset_radius = style.layout.border_radius.is_none() && style.radius.is_none();
        let aspect = sanitize_aspect(self.aspect);
        {
            let layout = Arc::make_mut(&mut style.layout);
            if unset_width {
                layout.aspect_ratio = Some(aspect);
            }
            layout.flex_grow.get_or_insert(0.0);
            layout.flex_shrink.get_or_insert(0.0);
            layout.border_width = Some(0.0);
            layout.overflow_x = OverflowSpec::Hidden;
            layout.overflow_y = OverflowSpec::Hidden;
            if !layout.position.establishes_containing_block() {
                layout.position = PositionSpec::Relative;
            }
        }
        if unset_radius {
            style.radius = Some(RadiusTier::Xs);
        }
        if unset_height {
            style.control_height = Some(ControlHeight::Exact(self.size));
        }
        if unset_width && aspect == DEFAULT_ASPECT {
            style.square = Some(SquareSize::Control(self.size));
        }
        style
    }
}

impl Default for Thumbnail {
    fn default() -> Self {
        Self::empty()
    }
}

const THUMBNAIL_POINTER: NodePointerHooks<Thumbnail> = NodePointerHooks {
    moved: |cx, thumbnail, pointer| {
        let Some(point) = pointer_point(cx, thumbnail, &pointer)? else {
            return Ok(false);
        };
        hover(cx, thumbnail, point)?;
        Ok(true)
    },
    left: |cx, thumbnail| hover(cx, thumbnail, None),
    pressed: |cx, thumbnail, pointer| {
        if pointer.button != 0 {
            return Ok(false);
        }
        let Some(Some(point)) = pointer_point(cx, thumbnail, &pointer)? else {
            return Ok(false);
        };
        let modifiers = pointer.modifiers;
        cx.update_component(thumbnail, |_, view| {
            view.emit(ThumbnailPick::Press { point, modifiers })
        })?;
        Ok(true)
    },
    released: |_, _, _| Ok(true),
    wheel: |_, _, _| Ok(false),
};

/// The picture point under `pointer`: `None` when the thumbnail does not
/// pick, `Some(None)` off its picture.
fn pointer_point(
    cx: &AppContext,
    thumbnail: Entity<Thumbnail>,
    pointer: &NodePointer,
) -> Result<Option<Option<ThumbnailPoint>>, FrameworkError> {
    let Some(bounds) = cx.world().component_layout_box(thumbnail.stable_id()) else {
        return Ok(None);
    };
    cx.read(thumbnail, |thumbnail| {
        thumbnail
            .pickable
            .then(|| thumbnail.picture_point(bounds.width, bounds.height, pointer.x, pointer.y))
    })
}

/// Say the pointer is at `point`, and that it left only once.
fn hover(
    cx: &mut AppContext,
    thumbnail: Entity<Thumbnail>,
    point: Option<ThumbnailPoint>,
) -> Result<(), FrameworkError> {
    if point.is_none() && !cx.read(thumbnail, |thumbnail| thumbnail.hovering)? {
        return Ok(());
    }
    cx.update_component(thumbnail, |thumbnail, view| {
        thumbnail.hovering = point.is_some();
        view.emit(ThumbnailPick::Hover(point));
    })
}

impl ComponentView for Thumbnail {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::THUMBNAIL.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        pointer: Some(&THUMBNAIL_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn reconcile(&mut self, next: Self) {
        let hovering = self.hovering && next.pickable;
        *self = next;
        self.hovering = hovering;
    }

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "thumbnail".into(),
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
        let custom = self.custom_render();
        if world.custom_render(id) != custom.as_ref() {
            mutations.set_custom_render(id, custom);
        }
        let visual = (self.state == ThumbnailState::Loading).then(|| SpinnerVisual {
            label: Arc::from(""),
            size: SPINNER_SIZE.min(self.box_extent(world.theme_metrics()).1),
            phase: 0.0,
        });
        if world.widget_visual_as::<SpinnerVisual>(id) != visual.as_ref() {
            mutations.set_widget_visual(id, visual);
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(world),
            InteractionState {
                pointer_events: self.pickable,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Image,
                label: (!self.label.is_empty()).then(|| Arc::clone(&self.label)),
                hidden: self.decorative,
                busy: self.state == ThumbnailState::Loading,
                invalid: self.state == ThumbnailState::Unavailable,
                ..AccessibilityState::default()
            },
        );
    }
}

fn sanitize_aspect(aspect: f32) -> f32 {
    if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        DEFAULT_ASPECT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport, ListItem, ListItemSlots};
    use nana_ui_core::UI_METRICS;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn mount(component: Thumbnail) -> (crate::UiWorld, StableNodeId) {
        let mut world = crate::UiWorld::new();
        let id = StableNodeId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id, document(), component.node_kind());
        component.project(id, &world, &mut queue);
        world.commit(queue).unwrap();
        (world, id)
    }

    #[test]
    fn box_is_control_height_by_host_aspect() {
        let square = ControlSize::Small.height_in(UI_METRICS);
        assert_eq!(Thumbnail::empty().box_extent(UI_METRICS), (square, square));
        let wide = Thumbnail::empty().aspect(16.0 / 9.0).box_extent(UI_METRICS);
        assert_eq!(wide.1, square);
        assert!((wide.0 - square * 16.0 / 9.0).abs() < f32::EPSILON);
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                Thumbnail::empty().aspect(invalid).box_extent(UI_METRICS),
                (square, square)
            );
        }
        let mut context = AppContext::new();
        let expected = (square, square);
        for thumb in [
            Thumbnail::empty(),
            Thumbnail::loading(),
            Thumbnail::new("stage.thumb"),
            Thumbnail::unavailable(),
        ] {
            let entity = context.create_component(document(), thumb).unwrap();
            context
                .layout_document(document(), LayoutViewport::new(240.0, 80.0))
                .unwrap();
            let bounds = context
                .world()
                .canonical_layout_box(entity.stable_id())
                .unwrap();
            assert_eq!((bounds.width, bounds.height), expected);
        }
        let wide = context
            .create_component(document(), Thumbnail::empty().aspect(16.0 / 9.0))
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(240.0, 80.0))
            .unwrap();
        let bounds = context
            .world()
            .canonical_layout_box(wide.stable_id())
            .unwrap();
        assert_eq!(bounds.height, square);
        assert!((bounds.width - square * 16.0 / 9.0).abs() < 0.01);
    }

    #[test]
    fn ready_samples_host_texture_contain_otherwise_omits_node() {
        let node = Thumbnail::new("stage.thumb")
            .custom_render()
            .expect("ready slot");
        assert_eq!(node.renderer.as_ref(), HOST_TEXTURE_RENDERER);
        assert_eq!(node.resource.as_ref(), "stage.thumb");
        assert_eq!(node.fit, ContentFit::Contain);
        assert!(Thumbnail::new("").custom_render().is_none());
        assert!(
            Thumbnail::new("stage.thumb")
                .state(ThumbnailState::Loading)
                .custom_render()
                .is_none()
        );
        let (world, id) = mount(Thumbnail::new("stage.thumb"));
        assert_eq!(
            world.custom_render(id).map(|node| node.fit),
            Some(ContentFit::Contain)
        );
        let (world, id) = mount(Thumbnail::loading());
        assert!(world.custom_render(id).is_none());
        assert!(world.widget_visual_as::<SpinnerVisual>(id).is_some());
        assert!(world.accessibility(id).unwrap().busy);
        match world.widget_visual_as::<SpinnerVisual>(id) {
            Some(SpinnerVisual { size, label, .. }) => {
                assert_eq!(*size, SPINNER_SIZE);
                assert!(label.is_empty());
            }
            None => panic!("loading thumbnail must project a spinner"),
        }
        let (world, id) = mount(Thumbnail::unavailable());
        assert!(world.accessibility(id).unwrap().invalid);
        assert_eq!(
            world.interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
    }

    #[test]
    fn picture_point_skips_the_contain_bands() {
        // a square picture in a 200×100 box: x 50..150 is the picture
        let thumb = Thumbnail::new("sheet").intrinsic_size(512, 512);
        assert_eq!(
            thumb.picture_point(200.0, 100.0, 100.0, 25.0),
            Some(ThumbnailPoint { x: 0.5, y: 0.25 })
        );
        assert_eq!(
            thumb.picture_point(200.0, 100.0, 50.0, 0.0),
            Some(ThumbnailPoint { x: 0.0, y: 0.0 })
        );
        assert_eq!(thumb.picture_point(200.0, 100.0, 49.0, 50.0), None);
        assert_eq!(thumb.picture_point(200.0, 100.0, 151.0, 50.0), None);
        // without a size the box is the picture
        assert_eq!(
            Thumbnail::new("sheet").picture_point(200.0, 100.0, 20.0, 50.0),
            Some(ThumbnailPoint { x: 0.1, y: 0.5 })
        );
        // a covering picture can only be picked where the box shows it
        let cover = thumb.clone().fit(ContentFit::Cover);
        assert_eq!(
            cover.picture_point(200.0, 100.0, 100.0, 50.0),
            Some(ThumbnailPoint { x: 0.5, y: 0.5 })
        );
        assert_eq!(cover.picture_point(200.0, 100.0, 100.0, 120.0), None);
        assert_eq!(
            Thumbnail::new("sheet").intrinsic_size(0, 4).intrinsic_size,
            None
        );
    }

    #[test]
    fn pickable_thumbnail_reports_hover_and_press_on_its_picture() {
        use nana_ui_input::{InputPayload, PointerInput, PointerPhase};
        use std::sync::Mutex;

        let mut context = AppContext::new();
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(200.0));
        layout.height = Some(LengthSpec::Px(100.0));
        let thumbnail = context
            .create_component(
                document(),
                Thumbnail::new("sheet")
                    .intrinsic_size(64, 64)
                    .pickable()
                    .style(style),
            )
            .unwrap();
        assert_eq!(
            context.world().interaction(thumbnail.stable_id()),
            Some(InteractionState {
                pointer_events: true,
                focusable: false,
            })
        );
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        context
            .on(thumbnail, move |_, pick: &ThumbnailPick, _| {
                sink.lock().unwrap().push(*pick)
            })
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(240.0, 120.0))
            .unwrap();
        let origin = context
            .world()
            .canonical_layout_box(thumbnail.stable_id())
            .unwrap();
        context.rebuild_hit_test(document());
        let mut input = crate::HeadlessInput::bind(&mut context, document());
        let mut route = |context: &mut AppContext, phase, x: f32, y: f32| {
            input
                .route(
                    context,
                    InputPayload::Pointer(PointerInput::mouse(phase, origin.x + x, origin.y + y)),
                )
                .unwrap();
        };
        let take = || std::mem::take(&mut *heard.lock().unwrap());

        route(&mut context, PointerPhase::Move, 100.0, 50.0);
        let point = ThumbnailPoint { x: 0.5, y: 0.5 };
        assert_eq!(take(), [ThumbnailPick::Hover(Some(point))]);
        // into the band: said once
        route(&mut context, PointerPhase::Move, 20.0, 50.0);
        route(&mut context, PointerPhase::Move, 10.0, 50.0);
        assert_eq!(take(), [ThumbnailPick::Hover(None)]);
        // a press on the band picks nothing
        route(&mut context, PointerPhase::Down, 20.0, 50.0);
        route(&mut context, PointerPhase::Up, 20.0, 50.0);
        assert_eq!(take(), []);
        route(&mut context, PointerPhase::Move, 75.0, 25.0);
        route(&mut context, PointerPhase::Down, 75.0, 25.0);
        route(&mut context, PointerPhase::Up, 75.0, 25.0);
        let point = ThumbnailPoint { x: 0.25, y: 0.25 };
        let heard_now = take();
        assert_eq!(heard_now.first(), Some(&ThumbnailPick::Hover(Some(point))));
        assert!(
            heard_now
                .iter()
                .any(|pick| matches!(pick, ThumbnailPick::Press { point: at, .. } if *at == point)),
            "{heard_now:?}"
        );
        // leaving the box
        route(&mut context, PointerPhase::Move, 230.0, 50.0);
        assert_eq!(take().last(), Some(&ThumbnailPick::Hover(None)));
    }

    #[test]
    fn a_thumbnail_that_does_not_pick_stays_out_of_hit_testing() {
        let (world, id) = mount(Thumbnail::new("sheet").intrinsic_size(64, 64));
        assert_eq!(
            world.interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
    }

    #[test]
    fn decorative_thumbnail_is_hidden_from_accessibility_projection() {
        let (world, id) = mount(Thumbnail::new("stage.thumb").decorative());
        let accessibility = world.accessibility(id).expect("thumbnail semantics");
        assert!(accessibility.hidden);
    }

    #[test]
    fn responsive_cover_keeps_geometry_across_resource_states() {
        let mut context = AppContext::new();
        let root = context
            .create_component(
                document(),
                crate::Stack::column(0.0)
                    .width(LengthSpec::Fill)
                    .height(LengthSpec::Px(135.0)),
            )
            .unwrap();
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.border_radius = Some(10.0);
        let thumbnail = context
            .create_detached_component(
                document(),
                Thumbnail::loading().fit(ContentFit::Cover).style(style),
            )
            .unwrap();
        context.append_child(root, thumbnail).unwrap();
        for width in [240.0, 360.0] {
            for state in [
                ThumbnailState::Empty,
                ThumbnailState::Loading,
                ThumbnailState::Ready,
                ThumbnailState::Unavailable,
            ] {
                context
                    .update_component(thumbnail, |view, _| {
                        view.state = state;
                        view.resource = "cover".into();
                    })
                    .unwrap();
                context
                    .layout_document(document(), LayoutViewport::new(width, 200.0))
                    .unwrap();
                let bounds = context
                    .world()
                    .canonical_layout_box(thumbnail.stable_id())
                    .unwrap();
                assert_eq!((bounds.width, bounds.height), (width, 135.0));
                assert_eq!(
                    context
                        .world()
                        .custom_render(thumbnail.stable_id())
                        .map(|node| node.fit),
                    (state == ThumbnailState::Ready).then_some(ContentFit::Cover)
                );
            }
        }
    }

    #[test]
    fn list_item_leading_keeps_the_box() {
        let mut context = AppContext::new();
        let item = context
            .create_component(document(), ListItem::new("角色"))
            .unwrap();
        let leading = context
            .create_component(document(), Thumbnail::empty())
            .unwrap();
        context.append_child(item, leading).unwrap();
        context
            .set_list_item_slots(
                item,
                ListItemSlots {
                    leading: Some(leading.stable_id()),
                    content: None,
                    trailing: None,
                },
            )
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(240.0, 80.0))
            .unwrap();
        let thumb = context
            .world()
            .canonical_layout_box(leading.stable_id())
            .unwrap();
        assert_eq!(thumb.width, ControlSize::Small.height_in(UI_METRICS));
        assert_eq!(thumb.height, ControlSize::Small.height_in(UI_METRICS));
        assert_eq!(
            context.world().node(item.stable_id()).unwrap().children,
            vec![leading.stable_id()]
        );
    }

    /// A frame that rounds its picture with the frame's own tier must get
    /// that tier, or the picture's corners stand outside the frame's.
    #[test]
    fn host_radius_tier_is_kept() {
        let (world, id) = mount(Thumbnail::empty());
        assert_eq!(world.node_style(id).unwrap().radius, Some(RadiusTier::Xs));
        let mut style = NodeStyle::default();
        style.radius = Some(RadiusTier::Md);
        let (world, id) = mount(Thumbnail::new("stage.thumb").style(style));
        assert_eq!(world.node_style(id).unwrap().radius, Some(RadiusTier::Md));
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).border_radius = Some(6.0);
        let (world, id) = mount(Thumbnail::empty().style(style));
        let node = world.node_style(id).unwrap();
        assert_eq!(node.radius, None);
        assert_eq!(node.layout.border_radius, Some(6.0));
    }

    /// Every view of one slot shares revision 0, so this control can share a
    /// slot with any other view; which texture it is lives in the registry.
    /// Pixel evidence is in `nana-ui-devtools/tests/host_texture_paint.rs`.
    #[test]
    fn host_texture_node_carries_no_revision() {
        let control = Thumbnail::new("cover:a");
        assert_eq!(control.custom_render().expect("ready slot").revision, 0);
    }

    #[test]
    fn badge_is_absolute_and_not_hittable_inside_the_cover() {
        let mut context = AppContext::new();
        let thumbnail = context
            .create_component(
                document(),
                Thumbnail::empty().style({
                    let mut style = NodeStyle::default();
                    let layout = Arc::make_mut(&mut style.layout);
                    layout.width = Some(LengthSpec::Px(160.0));
                    layout.height = Some(LengthSpec::Px(90.0));
                    style
                }),
            )
            .unwrap();
        let badge = context
            .create_detached_component(document(), Thumbnail::badge())
            .unwrap();
        context.append_child(thumbnail, badge).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(240.0, 120.0))
            .unwrap();
        let cover = context
            .world()
            .canonical_layout_box(thumbnail.stable_id())
            .unwrap();
        let badge_box = context
            .world()
            .canonical_layout_box(badge.stable_id())
            .unwrap();
        let cover_style = context.world().node_style(thumbnail.stable_id()).unwrap();
        assert_eq!(cover_style.layout.position, PositionSpec::Relative);
        assert_eq!(
            context.world().interaction(badge.stable_id()),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        assert!(
            badge_box.x + badge_box.width <= cover.x + cover.width + 0.5,
            "badge={badge_box:?} cover={cover:?}"
        );
        assert!(
            badge_box.y + badge_box.height <= cover.y + cover.height + 0.5,
            "badge={badge_box:?} cover={cover:?}"
        );
    }
}
