use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use nana_ui_runtime::CustomRenderNode;
use nana_ui_scene::{PrimitiveId, ScenePrimitiveKind, UiScene};

use crate::gpu_work::GpuWorkSink;
use crate::{LogicalRect, PhysicalRect};

#[derive(Debug, Clone)]
pub struct SceneGpuNode {
    pub id: PrimitiveId,
    pub custom: CustomRenderNode,
    pub opacity: f32,
}

pub struct SceneGpuPrepareContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub target_format: wgpu::TextureFormat,
    pub bounds: LogicalRect,
    pub scale_factor: f32,
    pub gpu_work: Option<&'a GpuWorkSink>,
}

pub struct SceneGpuRenderContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub target: &'a wgpu::TextureView,
    pub bounds: PhysicalRect,
    pub clip: PhysicalRect,
    pub gpu_work: Option<&'a GpuWorkSink>,
}

/// In-pass encode for a [`SceneGpuRenderer`] that can share the Scene dest.
///
/// `dest_size` is the current dest viewport in physical pixels so implementations
/// can restore it after changing scissor or viewport.
pub struct SceneGpuPassContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub bounds: PhysicalRect,
    pub clip: PhysicalRect,
    pub dest_size: [u32; 2],
    pub gpu_work: Option<&'a GpuWorkSink>,
}

/// Advanced WGPU compatibility extension for direct Scene graph passes.
///
/// Implementations receive NanaUI's existing Device/Queue during prepare and
/// the current frame encoder/target during render. They must not create a
/// second GPU context or submit the encoder themselves.
pub trait SceneGpuRenderer: fmt::Debug + Send + Sync + 'static {
    fn prepare(&self, node: &SceneGpuNode, context: SceneGpuPrepareContext<'_>);

    fn render(&self, node: &SceneGpuNode, context: SceneGpuRenderContext<'_>);

    /// Draw into the caller's current pass. Return `true` if this node was encoded.
    ///
    /// The default returns `false`, so the painter ends the main pass and calls
    /// [`Self::render`]. Prefer joining when the renderer can use the dest
    /// sample count and format.
    fn draw_in_pass(
        &self,
        _node: &SceneGpuNode,
        _pass: &mut wgpu::RenderPass<'_>,
        _context: SceneGpuPassContext<'_>,
    ) -> bool {
        false
    }
}

pub struct SceneResourceEncodeContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
}

/// Produces an external Scene resource before the compatibility painter reads
/// it. Each graph preparation pass receives a host-owned command encoder;
/// NanaUI submits successful passes before UI sampling on the same queue.
pub trait SceneResourceProducer: fmt::Debug + Send + Sync + 'static {
    /// Encode one preparation pass. Returning an error drops this pass without
    /// submission; implementations must not retain a pending submission token
    /// until all fallible encoding work has succeeded.
    fn encode(
        &self,
        node: &CustomRenderNode,
        context: SceneResourceEncodeContext<'_>,
    ) -> Result<(), String>;

    fn submitted(
        &self,
        _node: &CustomRenderNode,
        _device: &wgpu::Device,
        _submission: wgpu::SubmissionIndex,
    ) {
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneResourceProduceError {
    pub resource: Arc<str>,
    pub message: String,
}

impl fmt::Display for SceneResourceProduceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "scene resource producer `{}` failed: {}",
            self.resource, self.message
        )
    }
}

impl std::error::Error for SceneResourceProduceError {}

#[derive(Debug, Clone, Default)]
pub struct SceneResourceProducerRegistry {
    producers: HashMap<Arc<str>, Arc<dyn SceneResourceProducer>>,
}

impl SceneResourceProducerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        resource: impl Into<Arc<str>>,
        producer: Arc<dyn SceneResourceProducer>,
    ) -> Option<Arc<dyn SceneResourceProducer>> {
        self.producers.insert(resource.into(), producer)
    }

    pub fn get(&self, resource: &str) -> Option<Arc<dyn SceneResourceProducer>> {
        self.producers.get(resource).cloned()
    }

    pub fn encode_scene(
        &self,
        scene: &UiScene,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Option<wgpu::SubmissionIndex>, SceneResourceProduceError> {
        let graph = scene
            .frame_graph(nana_ui_scene::ResourceId(1))
            .map_err(|error| SceneResourceProduceError {
                resource: Arc::from("render-graph"),
                message: error.to_string(),
            })?;
        let nodes = graph
            .passes
            .iter()
            .flat_map(|pass| &pass.operations)
            .filter_map(|operation| {
                let nana_ui_scene::RenderOperation::PrepareExternal(id) = operation else {
                    return None;
                };
                let primitive = scene.primitive(*id)?;
                let ScenePrimitiveKind::Custom(node) = &primitive.kind else {
                    return None;
                };
                self.producers
                    .contains_key(node.resource.as_ref())
                    .then(|| node.clone())
            })
            .collect::<Vec<_>>();
        if nodes.is_empty() {
            return Ok(None);
        }
        let mut last_submission = None;
        for node in &nodes {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("NanaUI Scene external resource producer"),
            });
            self.producers[&node.resource]
                .encode(
                    node,
                    SceneResourceEncodeContext {
                        device,
                        queue,
                        encoder: &mut encoder,
                    },
                )
                .map_err(|message| SceneResourceProduceError {
                    resource: node.resource.clone(),
                    message,
                })?;
            let submission = queue.submit([encoder.finish()]);
            self.producers[&node.resource].submitted(node, device, submission.clone());
            last_submission = Some(submission);
        }
        Ok(last_submission)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SceneGpuRendererRegistry {
    renderers: HashMap<Arc<str>, Arc<dyn SceneGpuRenderer>>,
}

impl SceneGpuRendererRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        name: impl Into<Arc<str>>,
        renderer: Arc<dyn SceneGpuRenderer>,
    ) -> Option<Arc<dyn SceneGpuRenderer>> {
        self.renderers.insert(name.into(), renderer)
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn SceneGpuRenderer>> {
        self.renderers.get(name).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.renderers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct NoopRenderer;

    impl SceneGpuRenderer for NoopRenderer {
        fn prepare(&self, _node: &SceneGpuNode, _context: SceneGpuPrepareContext<'_>) {}

        fn render(&self, _node: &SceneGpuNode, _context: SceneGpuRenderContext<'_>) {}
    }

    #[test]
    fn registry_replaces_renderer_by_stable_name() {
        let mut registry = SceneGpuRendererRegistry::new();
        assert!(registry.insert("live2d", Arc::new(NoopRenderer)).is_none());
        assert!(registry.get("live2d").is_some());
        assert!(registry.insert("live2d", Arc::new(NoopRenderer)).is_some());
    }
}
