//! Iced compatibility painter for standard Nana [`UiScene`] primitives.
//!
//! This adapter is deliberately strict: registered host textures are resolved
//! in scene order; unknown custom GPU nodes and affine transforms that Iced
//! cannot reproduce are rejected instead of being silently skipped.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use iced::advanced::Renderer as _;
use iced::advanced::text::{Alignment, Ellipsis, LineHeight, Renderer as _, Shaping, Wrapping};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Text, layout, mouse, renderer};
use iced::alignment;
use iced::font::Weight;
use iced::{
    Background, Border, Color, Element, Font, Length, Pixels, Point, Rectangle, Size, Theme,
};
use iced_wgpu::primitive::Renderer as _;
use nana_ui_runtime::{TextHorizontalAlignment, TextVerticalAlignment};
use nana_ui_scene::{
    PrimitiveId, RenderOperation, ResourceId, ScenePrimitive, ScenePrimitiveKind, SceneRect,
    UiScene,
};

use crate::gpu_texture::GpuTexturePrimitive;
use crate::{HostTextureBinding, HostTextureLayer, HostTextureRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenePaintError {
    InvalidRenderGraph,
    CustomPrimitive(PrimitiveId),
    UnsupportedTransform(PrimitiveId),
    UnsupportedClipTransform(PrimitiveId),
    UnsupportedTextStyle(PrimitiveId),
    UnsupportedCustomRenderer(PrimitiveId),
    MissingCustomResource(PrimitiveId),
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
    size: Size,
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
        validate_scene(scene, None)?;
        Ok(Self {
            scene: SceneSource::Borrowed(scene),
            host_textures: None,
            size,
        })
    }

    pub fn with_host_textures(
        scene: &'scene UiScene,
        host_textures: HostTextureRegistry,
        size: Size,
    ) -> Result<Self, ScenePaintError> {
        validate_scene(scene, Some(&host_textures))?;
        Ok(Self {
            scene: SceneSource::Borrowed(scene),
            host_textures: Some(host_textures),
            size,
        })
    }

    pub fn from_shared(
        scene: Arc<UiScene>,
        host_textures: Option<HostTextureRegistry>,
        size: Size,
    ) -> Result<IcedSceneView<'static>, ScenePaintError> {
        validate_scene(&scene, host_textures.as_ref())?;
        Ok(IcedSceneView {
            scene: SceneSource::Shared(scene),
            host_textures,
            size,
        })
    }

    pub fn scene(&self) -> &UiScene {
        self.scene.get()
    }
}

fn validate_scene(
    scene: &UiScene,
    host_textures: Option<&HostTextureRegistry>,
) -> Result<(), ScenePaintError> {
    for primitive in scene.primitives() {
        if let ScenePrimitiveKind::Custom(custom) = &primitive.kind {
            if custom.renderer.as_ref() != "nana.host-texture" {
                return Err(ScenePaintError::UnsupportedCustomRenderer(primitive.id));
            }
            let Some(host_textures) = host_textures else {
                return Err(ScenePaintError::CustomPrimitive(primitive.id));
            };
            if host_textures.get(custom.resource.as_ref()).is_none() {
                return Err(ScenePaintError::MissingCustomResource(primitive.id));
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
    Ok(())
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
        let origin = layout.bounds().position();
        for primitive in self.scene().primitives() {
            let clip = primitive.clips.iter().try_fold(*viewport, |visible, clip| {
                visible.intersection(&translated_rect(clip.bounds, clip.transform.0, origin))
            });
            let Some(clip) = clip else {
                continue;
            };
            renderer.with_layer(clip, |renderer| match &primitive.kind {
                ScenePrimitiveKind::Custom(custom) => {
                    let Some(binding) = self
                        .host_textures
                        .as_ref()
                        .and_then(|registry| registry.get(custom.resource.as_ref()))
                    else {
                        return;
                    };
                    let bounds = translated_rect(primitive.bounds, primitive.transform.0, origin);
                    renderer.draw_primitive(
                        bounds,
                        GpuTexturePrimitive::from_layer(
                            HostTextureLayer::new(binding.texture).with_opacity(primitive.opacity),
                        ),
                    );
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
        } => renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: Border::default()
                    .color(color_with_opacity(
                        border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]),
                        primitive.opacity,
                    ))
                    .width(*border_width)
                    .rounded(*corner_radius),
                ..renderer::Quad::default()
            },
            Background::Color(color_with_opacity(
                background.unwrap_or([0.0, 0.0, 0.0, 0.0]),
                primitive.opacity,
            )),
        ),
        ScenePrimitiveKind::Text {
            content,
            color,
            size,
            weight,
            family,
            line_height,
            wrap,
            ellipsis,
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
                    shaping: Shaping::Advanced,
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
        ScenePrimitiveKind::Custom(_) => {
            unreachable!("custom primitives are rejected by IcedSceneView::new")
        }
    }
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
        AppContext, Button as RuntimeButton, CustomRenderNode, DocumentId, LayoutBox, MutationQueue,
    };

    use super::*;

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
}
