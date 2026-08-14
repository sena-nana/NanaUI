use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use iced::widget::shader;
use iced::{Rectangle, wgpu};
use nana_ui_runtime::CustomRenderNode;
use nana_ui_scene::{PrimitiveId, ScenePrimitiveKind, UiScene};

use crate::{LogicalRect, PhysicalRect, RenderSlot};

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
}

pub struct SceneGpuRenderContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub target: &'a wgpu::TextureView,
    pub bounds: PhysicalRect,
    pub clip: PhysicalRect,
}

/// Advanced WGPU compatibility extension for direct Scene graph passes.
///
/// Implementations receive NanaUI's existing Device/Queue during prepare and
/// the current frame encoder/target during render. They must not create a
/// second GPU context or submit the encoder themselves.
pub trait SceneGpuRenderer: fmt::Debug + Send + Sync + 'static {
    fn prepare(&self, node: &SceneGpuNode, context: SceneGpuPrepareContext<'_>);

    fn render(&self, node: &SceneGpuNode, context: SceneGpuRenderContext<'_>);
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

#[derive(Debug, Clone)]
pub(crate) struct SceneGpuPrimitive {
    node: SceneGpuNode,
    renderer: Arc<dyn SceneGpuRenderer>,
}

impl SceneGpuPrimitive {
    pub(crate) fn new(node: SceneGpuNode, renderer: Arc<dyn SceneGpuRenderer>) -> Self {
        Self { node, renderer }
    }
}

impl shader::Primitive for SceneGpuPrimitive {
    type Pipeline = SceneGpuPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        pipeline.device = Some(device.clone());
        pipeline.queue = Some(queue.clone());
        let slot = RenderSlot::new(
            self.node.id.node.get(),
            LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
            viewport.scale_factor(),
        );
        pipeline.prepared.insert(
            self.node.id,
            PreparedSceneGpuNode {
                bounds: slot.physical,
                used: true,
            },
        );
        self.renderer.prepare(
            &self.node,
            SceneGpuPrepareContext {
                device,
                queue,
                target_format: pipeline.target_format,
                bounds: slot.logical,
                scale_factor: viewport.scale_factor(),
            },
        );
    }

    fn render(
        &self,
        pipeline: &Self::Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(prepared) = pipeline.prepared.get(&self.node.id) else {
            return;
        };
        let clip = PhysicalRect {
            x: clip_bounds.x,
            y: clip_bounds.y,
            width: clip_bounds.width,
            height: clip_bounds.height,
        };
        let bounds = intersect(prepared.bounds, clip);
        if bounds.width == 0 || bounds.height == 0 {
            return;
        }
        self.renderer.render(
            &self.node,
            SceneGpuRenderContext {
                device: pipeline
                    .device
                    .as_ref()
                    .expect("scene GPU pipeline prepared before render"),
                queue: pipeline
                    .queue
                    .as_ref()
                    .expect("scene GPU pipeline prepared before render"),
                encoder,
                target,
                bounds,
                clip,
            },
        );
    }
}

pub(crate) struct SceneGpuPipeline {
    target_format: wgpu::TextureFormat,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    prepared: HashMap<PrimitiveId, PreparedSceneGpuNode>,
}

impl shader::Pipeline for SceneGpuPipeline {
    fn new(_device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            target_format: format,
            device: None,
            queue: None,
            prepared: HashMap::new(),
        }
    }

    fn trim(&mut self) {
        self.prepared.retain(|_, prepared| {
            let retain = prepared.used;
            prepared.used = false;
            retain
        });
    }
}

struct PreparedSceneGpuNode {
    bounds: PhysicalRect,
    used: bool,
}

fn intersect(left: PhysicalRect, right: PhysicalRect) -> PhysicalRect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left
        .x
        .saturating_add(left.width)
        .min(right.x.saturating_add(right.width));
    let bottom_edge = left
        .y
        .saturating_add(left.height)
        .min(right.y.saturating_add(right.height));
    PhysicalRect {
        x,
        y,
        width: right_edge.saturating_sub(x),
        height: bottom_edge.saturating_sub(y),
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

    #[test]
    fn physical_bounds_intersection_is_saturating() {
        assert_eq!(
            intersect(
                PhysicalRect {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                },
                PhysicalRect {
                    x: 25,
                    y: 0,
                    width: 30,
                    height: 30,
                },
            ),
            PhysicalRect {
                x: 25,
                y: 20,
                width: 15,
                height: 10,
            }
        );
    }
}
