use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use nana_gpu::{FrameContext, GpuContext, GpuSubmission};
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
    /// Destination size in physical pixels. A change to it invalidates the
    /// painter's prepared batch, so preparation always sees the size its
    /// commands will be encoded against.
    pub dest_size: [u32; 2],
    pub gpu_work: Option<&'a GpuWorkSink>,
}

pub struct SceneGpuRenderContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub target: &'a wgpu::TextureView,
    pub bounds: PhysicalRect,
    pub clip: PhysicalRect,
    /// Size of `target` in physical pixels. A dedicated pass covers the same
    /// destination as the main pass, so viewport and scissor math match.
    pub dest_size: [u32; 2],
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

/// One node of a contiguous, document-ordered run handed to
/// [`SceneGpuRenderer::draw_batch_in_pass`].
pub struct SceneGpuBatchNode<'a> {
    pub node: &'a SceneGpuNode,
    /// Destination rect in physical pixels, inside the current dest or group
    /// target.
    pub bounds: PhysicalRect,
    /// Physical scissor, already intersected with every ancestor clip. Items in
    /// one run may carry different clips.
    pub clip: PhysicalRect,
}

/// In-pass encode context for a run. `dest_size` is the current dest viewport in
/// physical pixels so implementations can restore it.
pub struct SceneGpuBatchPassContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub dest_size: [u32; 2],
    pub gpu_work: Option<&'a GpuWorkSink>,
}

/// Advanced in-pass Scene encode. Prefer [`crate::HostTexture`] /
/// [`crate::GpuTextureView`] for first-time hosts.
///
/// Implementations receive NanaUI's existing Device/Queue during prepare and
/// the current frame encoder/target during render. They must not create a
/// second GPU context or submit the encoder themselves.
pub trait SceneGpuRenderer: fmt::Debug + Send + Sync + 'static {
    /// Opt into reusing prepared UI commands. Change this version whenever
    /// `prepare` must run again. The default is dynamic; rendering still runs
    /// on every requested frame even when preparation is reused.
    fn preparation_version(&self, _node: &CustomRenderNode) -> Option<u64> {
        None
    }

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

    /// Longest run this renderer wants in one [`Self::draw_batch_in_pass`].
    /// `1` (the default) keeps the one-node-per-call path.
    fn batch_capacity(&self) -> usize {
        1
    }

    /// Encode a contiguous, document-ordered run into the caller's pass.
    ///
    /// Returns how many **leading** nodes were encoded. `0` makes the painter
    /// fall back to [`Self::draw_in_pass`] for `nodes[0]`, so an implementation
    /// may encode a prefix — the leading items that share one scissor, say —
    /// and let the painter offer the rest.
    ///
    /// This is not a frame-end batch. The run is a slice of the display list:
    /// any quad, glyph, icon, host texture, backdrop or group boundary between
    /// two nodes ends it, so a node can never be drawn across ordinary UI.
    /// Document order is exactly what it would be without batching; only the
    /// number of draws changes.
    ///
    /// Implementations restore the viewport to `context.dest_size` just as
    /// [`Self::draw_in_pass`] must, and record one draw batch and one draw call
    /// per draw they issue.
    fn draw_batch_in_pass(
        &self,
        _nodes: &[SceneGpuBatchNode<'_>],
        _pass: &mut wgpu::RenderPass<'_>,
        _context: SceneGpuBatchPassContext<'_>,
    ) -> usize {
        0
    }
}

/// One preparation pass of a [`SceneResourceProducer`], recorded into the
/// host's frame.
pub struct SceneResourceEncodeContext<'a> {
    pub gpu: &'a GpuContext,
    frame: &'a mut FrameContext,
}

impl SceneResourceEncodeContext<'_> {
    /// The host frame this pass records into. Never submit it: the host does,
    /// together with the UI paint.
    pub fn frame(&mut self) -> &mut FrameContext {
        self.frame
    }
}

/// Advanced graph-scheduled offscreen on the HostTexture path.
/// Prefer `prepare_window_frame`. Visible frames encode on the Surface encoder
/// and submit with UI paint; hidden ticks encode without a Surface and submit
/// immediately. [`Self::submitted`] means this encode was queued, not that a
/// UI frame sampling it has presented.
pub trait SceneResourceProducer: fmt::Debug + Send + Sync + 'static {
    /// Encode one preparation pass. Returning an error drops this pass without
    /// submission; implementations must not retain a pending submission token
    /// until all fallible encoding work has succeeded.
    fn encode(
        &self,
        node: &CustomRenderNode,
        context: SceneResourceEncodeContext<'_>,
    ) -> Result<(), String>;

    fn submitted(&self, _node: &CustomRenderNode, _submission: &GpuSubmission) {}
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

    /// Encode preparation into the host frame. No submission happens here.
    /// Drop the frame if this returns an error.
    pub fn encode_scene(
        &self,
        scene: &UiScene,
        frame: &mut FrameContext,
    ) -> Result<PreparedSceneResources, SceneResourceProduceError> {
        let gpu = frame.gpu().clone();
        let plan = scene
            .frame_plan()
            .map_err(|error| SceneResourceProduceError {
                resource: Arc::from("render-graph"),
                message: error.to_string(),
            })?;
        let mut prepared = PreparedSceneResources::default();
        for id in plan.preparations.iter() {
            let Some(primitive) = scene.primitive(*id) else {
                continue;
            };
            let ScenePrimitiveKind::Custom { node, .. } = &primitive.kind else {
                continue;
            };
            let Some(producer) = self.producers.get(&node.resource) else {
                continue;
            };
            producer
                .encode(
                    node,
                    SceneResourceEncodeContext {
                        gpu: &gpu,
                        frame: &mut *frame,
                    },
                )
                .map_err(|message| SceneResourceProduceError {
                    resource: node.resource.clone(),
                    message,
                })?;
            prepared.nodes.push((node.clone(), Arc::clone(producer)));
        }
        Ok(prepared)
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

/// Submission callbacks for successfully encoded external resources. Dropping
/// this value without `submitted` does not publish successful production.
#[derive(Default)]
pub struct PreparedSceneResources {
    nodes: Vec<(CustomRenderNode, Arc<dyn SceneResourceProducer>)>,
}

impl PreparedSceneResources {
    /// The host queued this encode. Hidden ticks call this without presenting.
    pub fn submitted(self, submission: &GpuSubmission) {
        for (node, producer) in self.nodes {
            producer.submitted(&node, submission);
        }
    }
}
