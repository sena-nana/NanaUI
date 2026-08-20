//! Scene validation shared by [`super::SceneWgpuPainter`].
//!
//! Unknown custom GPU nodes and transforms the product painter cannot
//! reproduce are rejected instead of being silently skipped.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use nana_ui_runtime::StableNodeId;
use nana_ui_scene::{PrimitiveId, RenderOperation, ResourceId, ScenePrimitiveKind, UiScene};

use crate::scene_gpu::SceneGpuRendererRegistry;
use crate::{HostTextureBinding, HostTextureRegistry};

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
                "scene primitive {}:{} uses an affine transform unsupported by the Scene painter",
                id.node.get(),
                id.slot
            ),
            Self::UnsupportedClipTransform(id) => write!(
                formatter,
                "scene primitive {}:{} uses a transformed clip unsupported by the Scene painter",
                id.node.get(),
                id.slot
            ),
            Self::UnsupportedTextStyle(id) => write!(
                formatter,
                "scene primitive {}:{} uses text styling unsupported by the Scene painter",
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

/// Resolves `"nana.host-texture"` custom scene nodes for compatibility trees
/// such as Vue. Scene GPU painters such as `"gpu-view"` are resolved by
/// [`super::SceneWgpuPainter`], not this lookup.
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
            // Scene GPU painters such as `"gpu-view"` are resolved by
            // `SceneWgpuPainter`, not this host-texture lookup.
            if custom.renderer.as_ref() != "nana.host-texture" {
                continue;
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

pub(crate) fn validate_scene(
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

fn is_translation([a, b, c, d, _, _]: [f32; 6]) -> bool {
    a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0
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
        AppContext, Button as RuntimeButton, CustomRenderNode, DocumentId, GPU_VIEW_RENDERER,
        GpuTextureView, GpuView, LayoutBox, MutationQueue,
    };
    use nana_ui_scene::UiScene;

    use super::*;
    use crate::scene_gpu::{
        SceneGpuNode, SceneGpuPrepareContext, SceneGpuRenderContext, SceneGpuRenderer,
    };
    use crate::{HostTextureRegistry, SceneGpuRendererRegistry, default_scene_gpu_renderers};

    #[derive(Debug)]
    struct NoopSceneRenderer;

    impl SceneGpuRenderer for NoopSceneRenderer {
        fn prepare(&self, _node: &SceneGpuNode, _context: SceneGpuPrepareContext<'_>) {}

        fn render(&self, _node: &SceneGpuNode, _context: SceneGpuRenderContext<'_>) {}
    }

    fn button_scene() -> (UiScene, nana_ui_runtime::StableNodeId) {
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
        (scene, button.stable_id())
    }

    fn gpu_view_scene() -> (UiScene, nana_ui_runtime::StableNodeId) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let view = context.create_component(document, GpuView::new(1)).unwrap();
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            view.stable_id(),
            LayoutBox {
                x: 4.0,
                y: 8.0,
                width: 120.0,
                height: 60.0,
            },
        );
        context.commit_mutations(mutations).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        let mut scene = UiScene::new();
        scene.apply_delta(
            context.world().extract_nodes(&work.render_extraction),
            work.render_removals,
        );
        (scene, view.stable_id())
    }

    fn assert_gpu_view_operation(
        operations: &[RenderOperation],
        scene: &UiScene,
        id: nana_ui_runtime::StableNodeId,
    ) {
        assert!(operations.iter().any(|operation| matches!(
            operation,
            RenderOperation::InvokeCustom(primitive) if primitive.node == id
                && scene.primitive(*primitive).is_some_and(|primitive| {
                    matches!(
                        &primitive.kind,
                        ScenePrimitiveKind::Custom(custom)
                            if custom.renderer.as_ref() == GPU_VIEW_RENDERER
                    )
                })
        )));
    }

    #[test]
    fn runtime_button_validates_and_custom_content_is_explicit() {
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
        let operations = validate_scene(&scene, None, None).unwrap();
        assert!(!operations.is_empty());
        assert_eq!(scene.primitives().count(), 2);

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
            validate_scene(&scene, None, None),
            Err(ScenePaintError::CustomPrimitive(_))
        ));
        assert!(matches!(
            validate_scene(&scene, Some(&HostTextureRegistry::new()), None),
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
        let operations = validate_scene(&scene, None, Some(&renderers)).unwrap();
        assert!(operations.iter().any(|operation| matches!(
            operation,
            RenderOperation::InvokeCustom(id) if *id == PrimitiveId {
                node: button.stable_id(),
                slot: 1,
            }
        )));
    }

    #[test]
    fn host_texture_layers_keep_standard_draws_between_them() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let background = context
            .create_component(document, GpuTextureView::new("live2d.bg"))
            .unwrap();
        let chrome = context
            .create_component(document, RuntimeButton::new("Start"))
            .unwrap();
        let foreground = context
            .create_component(document, GpuTextureView::new("live2d.fg"))
            .unwrap();
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            background.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 80.0,
            },
        );
        mutations.write_layout(
            chrome.stable_id(),
            LayoutBox {
                x: 8.0,
                y: 24.0,
                width: 64.0,
                height: 28.0,
            },
        );
        mutations.write_layout(
            foreground.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 80.0,
            },
        );
        context.commit_mutations(mutations).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        let mut scene = UiScene::new();
        scene.apply_delta(
            context.world().extract_nodes(&work.render_extraction),
            work.render_removals,
        );
        let graph = scene
            .frame_graph(nana_ui_scene::ResourceId(1))
            .expect("layered host textures compile");
        let operations = graph
            .passes
            .iter()
            .flat_map(|pass| pass.operations.iter())
            .filter(|operation| !matches!(operation, RenderOperation::PrepareExternal(_)))
            .collect::<Vec<_>>();
        let background_at = operations.iter().position(|operation| {
            matches!(
                operation,
                RenderOperation::InvokeCustom(id) if id.node == background.stable_id()
            )
        });
        let chrome_at = operations.iter().position(|operation| {
            matches!(
                operation,
                RenderOperation::Draw(id) if id.node == chrome.stable_id()
            )
        });
        let foreground_at = operations.iter().position(|operation| {
            matches!(
                operation,
                RenderOperation::InvokeCustom(id) if id.node == foreground.stable_id()
            )
        });
        assert!(
            background_at.expect("background layer") < chrome_at.expect("chrome draw")
                && chrome_at.expect("chrome draw") < foreground_at.expect("foreground layer"),
            "GUI draws must sit between Live2D layer slots, got {operations:?}"
        );
    }

    #[test]
    fn validate_scene_accepts_gpu_view_with_default_renderers() {
        let (scene, id) = gpu_view_scene();
        let renderers = default_scene_gpu_renderers();
        assert!(renderers.get(GPU_VIEW_RENDERER).is_some());
        assert!(renderers.get("gpu-view").is_some());
        let operations = validate_scene(&scene, None, Some(&renderers)).unwrap();
        assert_gpu_view_operation(&operations, &scene, id);
    }

    #[test]
    fn validate_scene_rejects_gpu_view_without_renderers() {
        let (scene, _) = gpu_view_scene();
        assert!(matches!(
            validate_scene(&scene, None, None),
            Err(ScenePaintError::UnsupportedCustomRenderer(_))
        ));
    }

    #[test]
    fn validate_scene_rejects_gpu_view_with_empty_registry() {
        let (scene, _) = gpu_view_scene();
        assert!(matches!(
            validate_scene(&scene, None, Some(&SceneGpuRendererRegistry::new())),
            Err(ScenePaintError::UnsupportedCustomRenderer(_))
        ));
    }

    #[test]
    fn host_texture_resolver_skips_gpu_view_custom_nodes() {
        let (scene, _) = gpu_view_scene();
        HostTextureSceneResolver::new(&scene, &HostTextureRegistry::new()).unwrap();
    }

    #[test]
    fn empty_button_scene_validates_without_gpu_registry() {
        let (scene, _) = button_scene();
        let operations = validate_scene(&scene, None, None).unwrap();
        assert!(!operations.is_empty());
    }
}
