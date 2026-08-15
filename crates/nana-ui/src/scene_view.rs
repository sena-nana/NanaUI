//! Iced compatibility painter for standard Nana [`UiScene`] primitives.
//!
//! This adapter is deliberately strict: registered host textures are resolved
//! in scene order; unknown custom GPU nodes and affine transforms that Iced
//! cannot reproduce are rejected instead of being silently skipped.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use iced::advanced::Renderer as _;
use iced::advanced::graphics::geometry::Renderer as _;
use iced::advanced::text::{Alignment, Ellipsis, LineHeight, Renderer as _, Shaping, Wrapping};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Text, layout, mouse, renderer};
use iced::alignment;
use iced::font::Weight;
use iced::widget::canvas;
use iced::{
    Background, Border, Color, Element, Font, Length, Pixels, Point, Rectangle, Shadow, Size,
    Theme, Vector,
};
use iced_wgpu::primitive::Renderer as _;
use nana_ui_runtime::{StableNodeId, TextHorizontalAlignment, TextShaping, TextVerticalAlignment};
use nana_ui_scene::{
    PrimitiveId, RenderOperation, ResourceId, ScenePrimitive, ScenePrimitiveKind, SceneRect,
    UiScene,
};

use crate::gpu_texture::GpuTexturePrimitive;
use crate::icons::{paint_icon_geometry, paint_spinner_geometry};
use crate::scene_gpu::SceneGpuPrimitive;
use crate::{
    HostTextureBinding, HostTextureLayer, HostTextureRegistry, SceneGpuNode,
    SceneGpuRendererRegistry,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenePaintError {
    InvalidRenderGraph,
    CustomPrimitive(PrimitiveId),
    UnsupportedTransform(PrimitiveId),
    UnsupportedClipTransform(PrimitiveId),
    UnsupportedTextStyle(PrimitiveId),
    UnsupportedCustomRenderer(PrimitiveId),
    MissingCustomResource(PrimitiveId),
    MissingNode(StableNodeId),
}

impl fmt::Display for ScenePaintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRenderGraph => formatter.write_str("scene render graph is invalid"),
            Self::CustomPrimitive(id) => write!(
                formatter,
                "scene primitive {}:{} requires a registered custom renderer",
                id.node.get(),
                id.slot
            ),
            Self::UnsupportedTransform(id) => write!(
                formatter,
                "scene primitive {}:{} uses an affine transform unsupported by the Iced compatibility painter",
                id.node.get(),
                id.slot
            ),
            Self::UnsupportedClipTransform(id) => write!(
                formatter,
                "scene primitive {}:{} uses a transformed clip unsupported by the Iced compatibility painter",
                id.node.get(),
                id.slot
            ),
            Self::UnsupportedTextStyle(id) => write!(
                formatter,
                "scene primitive {}:{} uses text styling unsupported by the Iced compatibility painter",
                id.node.get(),
                id.slot
            ),
            Self::UnsupportedCustomRenderer(id) => write!(
                formatter,
                "scene primitive {}:{} names an unsupported custom renderer",
                id.node.get(),
                id.slot
            ),
            Self::MissingCustomResource(id) => write!(
                formatter,
                "scene primitive {}:{} references an unavailable host resource",
                id.node.get(),
                id.slot
            ),
            Self::MissingNode(id) => write!(formatter, "scene node {} is unavailable", id.get()),
        }
    }
}

impl std::error::Error for ScenePaintError {}

/// Resolves custom scene nodes through the same host-texture adapter used by
/// [`IcedSceneView`]. Compatibility trees such as Vue may look up the binding
/// by stable node while retaining their semantic widget layout.
#[derive(Debug, Clone)]
pub struct HostTextureSceneResolver {
    bindings: HashMap<u64, HostTextureBinding>,
}

impl HostTextureSceneResolver {
    pub fn new(
        scene: &UiScene,
        host_textures: &HostTextureRegistry,
    ) -> Result<Self, ScenePaintError> {
        let graph = scene
            .frame_graph(ResourceId(1))
            .map_err(|_| ScenePaintError::InvalidRenderGraph)?;
        let mut bindings = HashMap::new();
        for operation in graph.passes.iter().flat_map(|pass| &pass.operations) {
            let RenderOperation::InvokeCustom(id) = operation else {
                continue;
            };
            let Some(primitive) = scene.primitive(*id) else {
                continue;
            };
            let ScenePrimitiveKind::Custom(custom) = &primitive.kind else {
                continue;
            };
            if custom.renderer.as_ref() != "nana.host-texture" {
                return Err(ScenePaintError::UnsupportedCustomRenderer(*id));
            }
            let binding = host_textures
                .get(custom.resource.as_ref())
                .ok_or(ScenePaintError::MissingCustomResource(*id))?;
            bindings.insert(primitive.node.get(), binding);
        }
        Ok(Self { bindings })
    }

    pub fn binding(&self, node: u64) -> Option<HostTextureBinding> {
        self.bindings.get(&node).cloned()
    }
}

/// A real compatibility widget that paints Runtime-produced quads and text.
/// Input remains owned by `UiWorld`; this type owns no retained widget state.
#[derive(Debug, Clone)]
pub struct IcedSceneView<'scene> {
    scene: SceneSource<'scene>,
    host_textures: Option<HostTextureRegistry>,
    gpu_renderers: Option<SceneGpuRendererRegistry>,
    operations: Arc<[RenderOperation]>,
    size: Size,
    scene_origin: Point,
}

#[derive(Debug, Clone)]
enum SceneSource<'scene> {
    Borrowed(&'scene UiScene),
    Shared(Arc<UiScene>),
}

impl SceneSource<'_> {
    fn get(&self) -> &UiScene {
        match self {
            Self::Borrowed(scene) => scene,
            Self::Shared(scene) => scene,
        }
    }
}

impl<'scene> IcedSceneView<'scene> {
    pub fn new(scene: &'scene UiScene, size: Size) -> Result<Self, ScenePaintError> {
        Self::borrowed(scene, None, None, size)
    }

    /// Paint exactly one retained Runtime subtree inside an Iced layout slot.
    /// The subtree's absolute scene origin is normalized to the widget origin;
    /// no second widget state tree is created.
    pub fn for_node(
        scene: &'scene UiScene,
        node: StableNodeId,
        size: Size,
    ) -> Result<Self, ScenePaintError> {
        let mut view = Self::borrowed(scene, None, None, size)?;
        view.restrict_to_node(node)?;
        Ok(view)
    }

    pub fn with_host_textures(
        scene: &'scene UiScene,
        host_textures: HostTextureRegistry,
        size: Size,
    ) -> Result<Self, ScenePaintError> {
        Self::borrowed(scene, Some(host_textures), None, size)
    }

    pub fn with_gpu_renderers(
        scene: &'scene UiScene,
        gpu_renderers: SceneGpuRendererRegistry,
        size: Size,
    ) -> Result<Self, ScenePaintError> {
        Self::borrowed(scene, None, Some(gpu_renderers), size)
    }

    pub fn with_gpu_resources(
        scene: &'scene UiScene,
        host_textures: Option<HostTextureRegistry>,
        gpu_renderers: Option<SceneGpuRendererRegistry>,
        size: Size,
    ) -> Result<Self, ScenePaintError> {
        Self::borrowed(scene, host_textures, gpu_renderers, size)
    }

    pub fn from_shared(
        scene: Arc<UiScene>,
        host_textures: Option<HostTextureRegistry>,
        size: Size,
    ) -> Result<IcedSceneView<'static>, ScenePaintError> {
        Self::from_shared_with_renderers(scene, host_textures, None, size)
    }

    pub fn from_shared_node(
        scene: Arc<UiScene>,
        node: StableNodeId,
        host_textures: Option<HostTextureRegistry>,
        size: Size,
    ) -> Result<IcedSceneView<'static>, ScenePaintError> {
        let mut view = Self::from_shared(scene, host_textures, size)?;
        view.restrict_to_node(node)?;
        Ok(view)
    }

    pub fn from_shared_with_renderers(
        scene: Arc<UiScene>,
        host_textures: Option<HostTextureRegistry>,
        gpu_renderers: Option<SceneGpuRendererRegistry>,
        size: Size,
    ) -> Result<IcedSceneView<'static>, ScenePaintError> {
        let operations = validate_scene(&scene, host_textures.as_ref(), gpu_renderers.as_ref())?;
        Ok(IcedSceneView {
            scene: SceneSource::Shared(scene),
            host_textures,
            gpu_renderers,
            operations,
            size,
            scene_origin: Point::ORIGIN,
        })
    }

    pub fn scene(&self) -> &UiScene {
        self.scene.get()
    }

    fn borrowed(
        scene: &'scene UiScene,
        host_textures: Option<HostTextureRegistry>,
        gpu_renderers: Option<SceneGpuRendererRegistry>,
        size: Size,
    ) -> Result<Self, ScenePaintError> {
        let operations = validate_scene(scene, host_textures.as_ref(), gpu_renderers.as_ref())?;
        Ok(Self {
            scene: SceneSource::Borrowed(scene),
            host_textures,
            gpu_renderers,
            operations,
            size,
            scene_origin: Point::ORIGIN,
        })
    }

    fn restrict_to_node(&mut self, node: StableNodeId) -> Result<(), ScenePaintError> {
        let bounds = self
            .scene()
            .node_bounds(node)
            .ok_or(ScenePaintError::MissingNode(node))?;
        self.scene_origin = Point::new(bounds.x, bounds.y);
        self.operations = self
            .operations
            .iter()
            .filter(|operation| {
                let id = match operation {
                    RenderOperation::PrepareExternal(id)
                    | RenderOperation::Draw(id)
                    | RenderOperation::InvokeCustom(id) => *id,
                };
                self.scene().is_node_in_subtree(node, id.node)
            })
            .cloned()
            .collect::<Vec<_>>()
            .into();
        Ok(())
    }
}

fn validate_scene(
    scene: &UiScene,
    host_textures: Option<&HostTextureRegistry>,
    gpu_renderers: Option<&SceneGpuRendererRegistry>,
) -> Result<Arc<[RenderOperation]>, ScenePaintError> {
    let graph = scene
        .frame_graph(ResourceId(1))
        .map_err(|_| ScenePaintError::InvalidRenderGraph)?;
    for primitive in scene.primitives() {
        if let ScenePrimitiveKind::Custom(custom) = &primitive.kind {
            if custom.renderer.as_ref() == "nana.host-texture" {
                let Some(host_textures) = host_textures else {
                    return Err(ScenePaintError::CustomPrimitive(primitive.id));
                };
                if host_textures.get(custom.resource.as_ref()).is_none() {
                    return Err(ScenePaintError::MissingCustomResource(primitive.id));
                }
            } else if gpu_renderers
                .and_then(|renderers| renderers.get(custom.renderer.as_ref()))
                .is_none()
            {
                return Err(ScenePaintError::UnsupportedCustomRenderer(primitive.id));
            }
        }
        if !is_translation(primitive.transform.0) {
            return Err(ScenePaintError::UnsupportedTransform(primitive.id));
        }
        if primitive
            .clips
            .iter()
            .any(|clip| !is_translation(clip.transform.0))
        {
            return Err(ScenePaintError::UnsupportedClipTransform(primitive.id));
        }
        if let ScenePrimitiveKind::Text {
            family,
            letter_spacing,
            ..
        } = &primitive.kind
            && (*letter_spacing != 0.0 || !supported_family(family.as_deref()))
        {
            return Err(ScenePaintError::UnsupportedTextStyle(primitive.id));
        }
    }
    Ok(graph
        .passes
        .into_iter()
        .flat_map(|pass| pass.operations)
        .filter(|operation| !matches!(operation, RenderOperation::PrepareExternal(_)))
        .collect::<Vec<_>>()
        .into())
}

impl<Message> Widget<Message, Theme, iced::Renderer> for IcedSceneView<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(
            Length::Fixed(self.size.width),
            Length::Fixed(self.size.height),
        )
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.resolve(
            Length::Fixed(self.size.width),
            Length::Fixed(self.size.height),
            self.size,
        ))
    }

    fn draw(
        &self,
        _tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let origin =
            layout.bounds().position() - Vector::new(self.scene_origin.x, self.scene_origin.y);
        for operation in self.operations.iter() {
            let id = match operation {
                RenderOperation::PrepareExternal(_) => continue,
                RenderOperation::Draw(id) | RenderOperation::InvokeCustom(id) => *id,
            };
            let Some(primitive) = self.scene().primitive(id) else {
                continue;
            };
            let clip = primitive.clips.iter().try_fold(*viewport, |visible, clip| {
                visible.intersection(&translated_rect(clip.bounds, clip.transform.0, origin))
            });
            let Some(clip) = clip else {
                continue;
            };
            renderer.with_layer(clip, |renderer| match &primitive.kind {
                ScenePrimitiveKind::Custom(custom) => {
                    let bounds = translated_rect(primitive.bounds, primitive.transform.0, origin);
                    if custom.renderer.as_ref() == "nana.host-texture" {
                        let binding = self
                            .host_textures
                            .as_ref()
                            .and_then(|registry| registry.get(custom.resource.as_ref()))
                            .expect("validated host texture remains registered");
                        renderer.draw_primitive(
                            bounds,
                            GpuTexturePrimitive::from_scene(
                                primitive.id.node.get(),
                                primitive.id.slot,
                                HostTextureLayer::from_binding(binding)
                                    .with_opacity(primitive.opacity),
                            ),
                        );
                    } else {
                        let gpu_renderer = self
                            .gpu_renderers
                            .as_ref()
                            .and_then(|registry| registry.get(custom.renderer.as_ref()))
                            .expect("validated scene GPU renderer remains registered");
                        renderer.draw_primitive(
                            bounds,
                            SceneGpuPrimitive::new(
                                SceneGpuNode {
                                    id: primitive.id,
                                    custom: custom.clone(),
                                    opacity: primitive.opacity,
                                },
                                gpu_renderer,
                            ),
                        );
                    }
                }
                _ => paint_primitive(renderer, primitive, origin, clip),
            });
        }
    }
}

impl<'a, Message: 'a> From<IcedSceneView<'a>> for Element<'a, Message> {
    fn from(view: IcedSceneView<'a>) -> Self {
        Element::new(view)
    }
}

fn paint_primitive(
    renderer: &mut iced::Renderer,
    primitive: &ScenePrimitive,
    origin: Point,
    clip: Rectangle,
) {
    let bounds = translated_rect(primitive.bounds, primitive.transform.0, origin);
    match &primitive.kind {
        ScenePrimitiveKind::Quad {
            background,
            border_color,
            border_width,
            corner_radius,
            shadow,
        } => paint_quad(
            renderer,
            bounds,
            *background,
            *border_color,
            *border_width,
            *corner_radius,
            *shadow,
            primitive.opacity,
        ),
        ScenePrimitiveKind::QuadBatch {
            bounds,
            background,
            border_color,
            border_width,
            corner_radius,
            shadow,
        } => {
            for bounds in bounds {
                paint_quad(
                    renderer,
                    translated_rect(*bounds, primitive.transform.0, origin),
                    *background,
                    *border_color,
                    *border_width,
                    *corner_radius,
                    *shadow,
                    primitive.opacity,
                );
            }
        }
        ScenePrimitiveKind::Text {
            content,
            color,
            size,
            weight,
            family,
            line_height,
            wrap,
            ellipsis,
            shaping,
            horizontal_alignment,
            vertical_alignment,
            ..
        } => {
            let align_x = match horizontal_alignment {
                TextHorizontalAlignment::Start => Alignment::Left,
                TextHorizontalAlignment::Center => Alignment::Center,
                TextHorizontalAlignment::End => Alignment::Right,
            };
            let align_y = match vertical_alignment {
                TextVerticalAlignment::Top => alignment::Vertical::Top,
                TextVerticalAlignment::Center => alignment::Vertical::Center,
                TextVerticalAlignment::Bottom => alignment::Vertical::Bottom,
            };
            let position = Point::new(
                match align_x {
                    Alignment::Left | Alignment::Default | Alignment::Justified => bounds.x,
                    Alignment::Center => bounds.center_x(),
                    Alignment::Right => bounds.x + bounds.width,
                },
                match align_y {
                    alignment::Vertical::Top => bounds.y,
                    alignment::Vertical::Center => bounds.center_y(),
                    alignment::Vertical::Bottom => bounds.y + bounds.height,
                },
            );
            renderer.fill_text(
                Text {
                    content: content.clone(),
                    bounds: bounds.size(),
                    size: Pixels(*size),
                    line_height: match line_height {
                        Some(nana_ui_core::LineHeightSpec::Relative(value)) => {
                            LineHeight::Relative(*value)
                        }
                        Some(nana_ui_core::LineHeightSpec::Absolute(value)) => {
                            LineHeight::Absolute(Pixels(*value))
                        }
                        None => LineHeight::Relative(1.2),
                    },
                    font: scene_font(renderer.default_font(), family.as_deref(), *weight),
                    align_x,
                    align_y,
                    shaping: match shaping {
                        TextShaping::Auto => Shaping::Auto,
                        TextShaping::Advanced => Shaping::Advanced,
                    },
                    wrapping: if *wrap {
                        Wrapping::Word
                    } else {
                        Wrapping::None
                    },
                    ellipsis: if *ellipsis {
                        Ellipsis::End
                    } else {
                        Ellipsis::None
                    },
                    hint_factor: renderer.hint_factor(),
                },
                position,
                color_with_opacity(color.unwrap_or([0.0, 0.0, 0.0, 1.0]), primitive.opacity),
                clip,
            );
        }
        ScenePrimitiveKind::Icon { icon, color } => {
            renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                let mut frame = canvas::Frame::new(renderer, bounds.size());
                let scale = bounds.width.min(bounds.height) / 24.0;
                let offset = Point::new(
                    (bounds.width - 24.0 * scale) / 2.0,
                    (bounds.height - 24.0 * scale) / 2.0,
                );
                paint_icon_geometry(
                    &mut frame,
                    *icon,
                    scale,
                    offset,
                    color_with_opacity(color.unwrap_or([0.0, 0.0, 0.0, 1.0]), primitive.opacity),
                    1.7,
                );
                renderer.draw_geometry(frame.into_geometry());
            });
        }
        ScenePrimitiveKind::Spinner { phase, color } => {
            renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                let mut frame = canvas::Frame::new(renderer, bounds.size());
                paint_spinner_geometry(
                    &mut frame,
                    bounds.size(),
                    *phase,
                    color_with_opacity(color.unwrap_or([0.0, 0.0, 0.0, 1.0]), primitive.opacity),
                );
                renderer.draw_geometry(frame.into_geometry());
            });
        }
        ScenePrimitiveKind::Custom(_) => {
            unreachable!("custom primitives are rejected by IcedSceneView::new")
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_quad(
    renderer: &mut iced::Renderer,
    bounds: Rectangle,
    background: Option<[f32; 4]>,
    border_color: Option<[f32; 4]>,
    border_width: f32,
    corner_radius: f32,
    shadow: Option<nana_ui_runtime::ComponentElevation>,
    opacity: f32,
) {
    renderer.fill_quad(
        renderer::Quad {
            bounds,
            border: Border::default()
                .color(color_with_opacity(
                    border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]),
                    opacity,
                ))
                .width(border_width)
                .rounded(corner_radius),
            shadow: shadow.map_or_else(Shadow::default, |shadow| Shadow {
                color: color_with_opacity(shadow.color, opacity),
                offset: Vector::new(0.0, shadow.offset_y),
                blur_radius: shadow.blur_radius,
            }),
            ..renderer::Quad::default()
        },
        Background::Color(color_with_opacity(
            background.unwrap_or([0.0, 0.0, 0.0, 0.0]),
            opacity,
        )),
    );
}

fn translated_rect(bounds: SceneRect, transform: [f32; 6], origin: Point) -> Rectangle {
    Rectangle {
        x: origin.x + bounds.x + transform[4],
        y: origin.y + bounds.y + transform[5],
        width: bounds.width,
        height: bounds.height,
    }
}

fn is_translation([a, b, c, d, _, _]: [f32; 6]) -> bool {
    a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0
}

fn color_with_opacity([r, g, b, a]: [f32; 4], opacity: f32) -> Color {
    Color::from_rgba(r, g, b, a * opacity)
}

fn scene_font(default: Font, family: Option<&str>, weight: Option<u16>) -> Font {
    let family = family.unwrap_or_default().to_ascii_lowercase();
    let font = if family.contains("mono") {
        Font::MONOSPACE
    } else {
        default
    };
    font.weight(match weight.unwrap_or(400) {
        0..=149 => Weight::Thin,
        150..=249 => Weight::ExtraLight,
        250..=349 => Weight::Light,
        350..=449 => Weight::Normal,
        450..=549 => Weight::Medium,
        550..=649 => Weight::Semibold,
        650..=749 => Weight::Bold,
        750..=849 => Weight::ExtraBold,
        _ => Weight::Black,
    })
}

fn supported_family(family: Option<&str>) -> bool {
    family.is_none_or(|family| {
        let family = family.trim().to_ascii_lowercase();
        family.is_empty()
            || family == "sans-serif"
            || family == "system-ui"
            || family == "monospace"
            || family.contains("mono")
    })
}

#[cfg(test)]
mod tests {
    use nana_ui_runtime::{
        AppContext, Button as RuntimeButton, Card as RuntimeCard, CustomRenderNode, DocumentId,
        LayoutBox, MutationQueue,
    };

    use super::*;

    #[derive(Debug)]
    struct NoopSceneRenderer;

    impl crate::SceneGpuRenderer for NoopSceneRenderer {
        fn prepare(
            &self,
            _node: &crate::SceneGpuNode,
            _context: crate::SceneGpuPrepareContext<'_>,
        ) {
        }

        fn render(&self, _node: &crate::SceneGpuNode, _context: crate::SceneGpuRenderContext<'_>) {}
    }

    #[test]
    fn runtime_button_reaches_standard_painter_and_custom_content_is_explicit() {
        let mut context = AppContext::new();
        let button = context
            .create_component(DocumentId::new(1).unwrap(), RuntimeButton::new("构建"))
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 40.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        let mut scene = UiScene::new();
        scene.apply_delta(
            context.world().extract_nodes(&work.render_extraction),
            work.render_removals,
        );
        let view = IcedSceneView::new(&scene, Size::new(100.0, 40.0)).unwrap();
        assert_eq!(view.scene().primitives().count(), 2);

        let mut custom = MutationQueue::new();
        custom.set_custom_render(
            button.stable_id(),
            Some(CustomRenderNode {
                renderer: "nana.host-texture".into(),
                resource: "preview".into(),
                revision: 1,
            }),
        );
        context.commit_mutations(custom).unwrap();
        let work = context.take_system_work();
        scene.apply_delta(
            context.world().extract_nodes(&work.render_extraction),
            work.render_removals,
        );
        assert!(matches!(
            IcedSceneView::new(&scene, Size::new(100.0, 40.0)),
            Err(ScenePaintError::CustomPrimitive(_))
        ));
        assert!(matches!(
            IcedSceneView::with_host_textures(
                &scene,
                HostTextureRegistry::new(),
                Size::new(100.0, 40.0)
            ),
            Err(ScenePaintError::MissingCustomResource(_))
        ));
    }

    #[test]
    fn registered_custom_renderer_is_an_executable_scene_operation() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, RuntimeButton::new("Preview"))
            .unwrap();
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 4.0,
                y: 8.0,
                width: 120.0,
                height: 60.0,
            },
        );
        mutations.set_custom_render(
            button.stable_id(),
            Some(CustomRenderNode {
                renderer: "live2d.direct".into(),
                resource: "model".into(),
                revision: 7,
            }),
        );
        context.commit_mutations(mutations).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        let mut scene = UiScene::new();
        scene.apply_delta(
            context.world().extract_nodes(&work.render_extraction),
            work.render_removals,
        );
        let mut renderers = SceneGpuRendererRegistry::new();
        renderers.insert("live2d.direct", Arc::new(NoopSceneRenderer));
        let view =
            IcedSceneView::with_gpu_renderers(&scene, renderers, Size::new(160.0, 100.0)).unwrap();
        assert!(view.operations.iter().any(|operation| matches!(
            operation,
            RenderOperation::InvokeCustom(id) if *id == PrimitiveId {
                node: button.stable_id(),
                slot: 1,
            }
        )));
    }

    #[test]
    fn node_view_paints_only_the_selected_runtime_subtree() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let card = context
            .create_component(document, RuntimeCard::new().title("Actions"))
            .unwrap();
        let child = context
            .create_component(document, RuntimeButton::new("Build"))
            .unwrap();
        let outside = context
            .create_component(document, RuntimeButton::new("Outside"))
            .unwrap();
        context.append_child(card, child).unwrap();
        let mut layout = MutationQueue::new();
        for (id, x, y, width, height) in [
            (card.stable_id(), 40.0, 20.0, 160.0, 80.0),
            (child.stable_id(), 52.0, 48.0, 80.0, 32.0),
            (outside.stable_id(), 240.0, 20.0, 80.0, 32.0),
        ] {
            layout.write_layout(
                id,
                LayoutBox {
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
        context.commit_mutations(layout).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        let mut scene = UiScene::new();
        scene.apply_delta(
            context.world().extract_nodes(&work.render_extraction),
            work.render_removals,
        );

        let view =
            IcedSceneView::for_node(&scene, card.stable_id(), Size::new(160.0, 80.0)).unwrap();
        assert_eq!(view.scene_origin, Point::new(40.0, 20.0));
        assert!(view.operations.iter().all(|operation| {
            let primitive = match operation {
                RenderOperation::PrepareExternal(id)
                | RenderOperation::Draw(id)
                | RenderOperation::InvokeCustom(id) => *id,
            };
            scene.is_node_in_subtree(card.stable_id(), primitive.node)
        }));
        assert!(view.operations.iter().any(|operation| matches!(
            operation,
            RenderOperation::Draw(id) if id.node == child.stable_id()
        )));
        assert!(!view.operations.iter().any(|operation| matches!(
            operation,
            RenderOperation::Draw(id) if id.node == outside.stable_id()
        )));
    }
}
