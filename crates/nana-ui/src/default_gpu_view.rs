//! Host-owned default painter for Runtime [`GPU_VIEW_RENDERER`].
//!
//! Uses the caller's Device/Queue and the current frame encoder/target. It does
//! not request a GPU context or perform CPU readback.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use nana_ui_runtime::{CustomRenderNode, GPU_VIEW_RENDERER, GpuViewPalette, gpu_view_params};
use nana_ui_scene::PrimitiveId;

use crate::gpu_view::GPU_VIEW_SHADER;
use crate::gpu_work::GpuWorkSink;
use crate::scene_gpu::{
    SceneGpuBatchNode, SceneGpuBatchPassContext, SceneGpuNode, SceneGpuPassContext,
    SceneGpuPrepareContext, SceneGpuRenderContext, SceneGpuRenderer, SceneGpuRendererRegistry,
};

/// Scene painter for [`GPU_VIEW_RENDERER`] (`"gpu-view"`).
///
/// The hosted runtime installs this when a program leaves scene GPU renderers
/// unset and host Device/Queue handles are available. [`Self::draw_in_pass`]
/// encodes into the current Scene dest pass (Inline). [`Self::render`] opens a
/// dedicated pass on the same encoder/target when the node asks for one or the
/// painter cannot join.
///
/// Per-node palette and seed arrive in [`nana_ui_runtime::CustomRenderNode`]
/// `params` under [`gpu_view_params`]. The constructor palette is the fallback
/// for nodes that carry no params.
/// Prepare passes a culled slot survives before its buffer and bind group are
/// dropped. Scrolling a node just out of view and back must not rebuild it.
const SLOT_RETAIN_PASSES: u64 = 4;

pub struct DefaultGpuViewRenderer {
    palette: GpuViewPalette,
    device: Option<Arc<wgpu::Device>>,
    queue: Option<Arc<wgpu::Queue>>,
    state: Mutex<Option<PreparedGpuView>>,
}

impl Default for DefaultGpuViewRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultGpuViewRenderer {
    pub fn new() -> Self {
        Self::with_palette(GpuViewPalette::default())
    }

    pub fn with_palette(palette: GpuViewPalette) -> Self {
        Self {
            palette,
            device: None,
            queue: None,
            state: Mutex::new(None),
        }
    }

    /// Retain the already-created host Device/Queue. Pipelines are still built
    /// during prepare from this pair, not from a second GPU context.
    pub fn with_host(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        Self::with_host_palette(device, queue, GpuViewPalette::default())
    }

    pub fn with_host_palette(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        palette: GpuViewPalette,
    ) -> Self {
        Self {
            palette,
            device: Some(device),
            queue: Some(queue),
            state: Mutex::new(None),
        }
    }

    /// Live per-node slots. Test probe for the eviction contract.
    #[cfg(test)]
    pub(crate) fn prepared_slot_count(&self) -> usize {
        self.state
            .lock()
            .expect("default gpu-view pipeline")
            .as_ref()
            .map_or(0, |prepared| prepared.slots.len())
    }

    fn prepare_device<'a>(&'a self, context: &'a SceneGpuPrepareContext<'_>) -> &'a wgpu::Device {
        self.device.as_deref().unwrap_or(context.device)
    }

    fn prepare_queue<'a>(&'a self, context: &'a SceneGpuPrepareContext<'_>) -> &'a wgpu::Queue {
        self.queue.as_deref().unwrap_or(context.queue)
    }

    /// Per-node palette, falling back to the constructor palette when the node
    /// carries no `params`.
    fn node_palette(&self, custom: &CustomRenderNode) -> GpuViewPalette {
        let Some(params) = custom.params.as_ref() else {
            return self.palette;
        };
        if params.len() < gpu_view_params::LEN {
            return self.palette;
        }
        GpuViewPalette {
            background: rgba(params, gpu_view_params::BACKGROUND),
            accent: rgba(params, gpu_view_params::ACCENT),
        }
    }

    /// Per-node seed. Falls back to the revision so a host that omits `params`
    /// still animates on content invalidation.
    fn node_seed(&self, custom: &CustomRenderNode) -> f32 {
        custom
            .param(gpu_view_params::SEED)
            .unwrap_or(custom.revision as f32 * 0.17)
    }
}

fn rgba(params: &[f32], offset: usize) -> [f32; 4] {
    [
        params[offset],
        params[offset + 1],
        params[offset + 2],
        params[offset + 3],
    ]
}

impl fmt::Debug for DefaultGpuViewRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DefaultGpuViewRenderer")
            .field("palette", &self.palette)
            .field("has_host_gpu", &self.device.is_some())
            .finish_non_exhaustive()
    }
}

impl SceneGpuRenderer for DefaultGpuViewRenderer {
    /// Everything [`Self::prepare`] reads off the node: `revision` (the seed
    /// fallback) and `params` (palette and seed). Geometry is deliberately
    /// absent — a bounds or scale change moves `UiScene::instance_id` or the
    /// paint viewport, which already fails the painter's prepared-batch key.
    ///
    /// Without this the painter treats the whole frame as uncacheable and
    /// rebuilds every quad, glyph and icon of the entire tree each frame.
    fn preparation_version(&self, node: &CustomRenderNode) -> Option<u64> {
        let mut hasher = DefaultHasher::new();
        node.revision.hash(&mut hasher);
        match node.params.as_deref() {
            // Bit patterns, not values: `with_params` already replaced every
            // non-finite entry, so there is no NaN to compare unequal to itself.
            Some(params) => {
                params.len().hash(&mut hasher);
                for value in params {
                    value.to_bits().hash(&mut hasher);
                }
            }
            None => u64::MAX.hash(&mut hasher),
        }
        Some(hasher.finish())
    }

    fn prepare(&self, node: &SceneGpuNode, context: SceneGpuPrepareContext<'_>) {
        let device = self.prepare_device(&context);
        let queue = self.prepare_queue(&context);
        let mut state = self.state.lock().expect("default gpu-view pipeline");
        let prepared =
            state.get_or_insert_with(|| PreparedGpuView::new(device, context.target_format));
        if prepared.format != context.target_format {
            *prepared = PreparedGpuView::new(device, context.target_format);
        }
        prepared.begin_prepare_pass();
        let scale = if context.scale_factor.is_finite() && context.scale_factor > 0.0 {
            context.scale_factor
        } else {
            1.0
        };
        let rect = [
            context.bounds.x * scale,
            context.bounds.y * scale,
            context.bounds.width * scale,
            context.bounds.height * scale,
        ];
        let palette = self.node_palette(&node.custom);
        let instance = GpuViewInstance {
            rect,
            color_a: palette.background,
            color_b: palette.accent,
            parameters: [self.node_seed(&node.custom), 0.0, 0.0, 0.0],
        };
        prepared.dest_size = context.dest_size;
        prepared.write_slot(device, queue, node.id, instance);
        if let Some(work) = context.gpu_work {
            work.record_upload(std::mem::size_of::<GpuViewInstance>());
        }
    }

    fn render(&self, node: &SceneGpuNode, context: SceneGpuRenderContext<'_>) {
        if context.bounds.width == 0 || context.bounds.height == 0 {
            return;
        }
        let mut render_pass = context
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nana-ui default gpu-view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: context.target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        self.draw_in_pass(
            node,
            &mut render_pass,
            SceneGpuPassContext {
                device: context.device,
                queue: context.queue,
                bounds: context.bounds,
                clip: context.clip,
                dest_size: context.dest_size,
                gpu_work: context.gpu_work,
            },
        );
        drop(render_pass);
    }

    fn batch_capacity(&self) -> usize {
        // No bound of its own: the painter already caps a run at the next
        // non-matching display-list command.
        usize::MAX
    }

    fn draw_in_pass(
        &self,
        node: &SceneGpuNode,
        pass: &mut wgpu::RenderPass<'_>,
        context: SceneGpuPassContext<'_>,
    ) -> bool {
        if context.bounds.width == 0 || context.bounds.height == 0 {
            return false;
        }
        let mut state = self.state.lock().expect("default gpu-view pipeline");
        let Some(prepared) = state.as_mut() else {
            return false;
        };
        prepared.drawn = true;
        let Some(first) = prepared.slots.get(&node.id).map(|slot| slot.index) else {
            return false;
        };
        prepared.restage_all(context.queue);
        prepared.draw(pass, context.clip, first, 1, context.gpu_work);
        true
    }

    /// Encodes the longest leading stretch of the run whose instances are
    /// already adjacent in the shared buffer, as one instanced draw. Indices are
    /// handed out lowest-free-first in preparation order, so a stable tree keeps
    /// a whole run adjacent; churn costs extra draws, never wrong pixels.
    fn draw_batch_in_pass(
        &self,
        nodes: &[SceneGpuBatchNode<'_>],
        pass: &mut wgpu::RenderPass<'_>,
        context: SceneGpuBatchPassContext<'_>,
    ) -> usize {
        let mut state = self.state.lock().expect("default gpu-view pipeline");
        let Some(prepared) = state.as_mut() else {
            return 0;
        };
        prepared.drawn = true;
        let Some(first_node) = nodes.first() else {
            return 0;
        };
        let Some(first) = prepared
            .slots
            .get(&first_node.node.id)
            .map(|slot| slot.index)
        else {
            return 0;
        };
        let clip = first_node.clip;
        let mut count = 1u32;
        for item in &nodes[1..] {
            let Some(slot) = prepared.slots.get(&item.node.id) else {
                break;
            };
            if item.clip != clip || slot.index != first + count {
                break;
            }
            count += 1;
        }
        if count < 2 {
            return 0;
        }
        prepared.restage_all(context.queue);
        prepared.draw(pass, clip, first, count, context.gpu_work);
        count as usize
    }
}

/// Registry that contains the host default `"gpu-view"` painter.
pub fn default_scene_gpu_renderers() -> SceneGpuRendererRegistry {
    scene_gpu_renderers_with_gpu_view(DefaultGpuViewRenderer::new())
}

/// Same as [`default_scene_gpu_renderers`], retaining host Device/Queue clones.
pub fn default_scene_gpu_renderers_with_host(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
) -> SceneGpuRendererRegistry {
    scene_gpu_renderers_with_gpu_view(DefaultGpuViewRenderer::with_host(device, queue))
}

fn scene_gpu_renderers_with_gpu_view(renderer: DefaultGpuViewRenderer) -> SceneGpuRendererRegistry {
    let mut registry = SceneGpuRendererRegistry::new();
    registry.insert(GPU_VIEW_RENDERER, Arc::new(renderer));
    registry
}

/// Prefer a program-supplied registry. `None` keeps [`fallback`], including a
/// missing fallback when the host has no GPU resources.
pub fn resolve_scene_gpu_renderers(
    program: Option<SceneGpuRendererRegistry>,
    fallback: Option<SceneGpuRendererRegistry>,
) -> Option<SceneGpuRendererRegistry> {
    match program {
        Some(registry) => Some(registry),
        None => fallback,
    }
}

/// Instances a run of `gpu-view` nodes draws from. One vertex buffer, one bind
/// group, one pipeline: N nodes cost one draw when their slots are adjacent.
const INITIAL_INSTANCES: u32 = 32;

struct PreparedGpuView {
    pipeline: wgpu::RenderPipeline,
    /// Dest size the staged instances carry. Stamped during preparation, which
    /// a resize always re-runs because it invalidates the prepared batch.
    dest_size: [u32; 2],
    instances: wgpu::Buffer,
    instance_capacity: u32,
    /// The instance buffer was replaced; every live slot needs rewriting.
    restaged: bool,
    format: wgpu::TextureFormat,
    slots: HashMap<PrimitiveId, PreparedSlot>,
    /// Lowest-free-first, so preparation order keeps a run adjacent.
    free_indices: BinaryHeap<Reverse<u32>>,
    next_index: u32,
    generation: u64,
    drawn: bool,
}

impl PreparedGpuView {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui default gpu-view shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(GPU_VIEW_SHADER)),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui default gpu-view pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui default gpu-view pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuViewInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4,
                        1 => Float32x4,
                        2 => Float32x4,
                        3 => Float32x4,
                    ],
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            dest_size: [0, 0],
            instances: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui default gpu-view instances"),
                size: (INITIAL_INSTANCES as usize * std::mem::size_of::<GpuViewInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            instance_capacity: INITIAL_INSTANCES,
            restaged: false,
            format,
            slots: HashMap::new(),
            free_indices: BinaryHeap::new(),
            next_index: 0,
            generation: 0,
            drawn: false,
        }
    }

    /// Start of a new prepare pass. Slots are keyed by `PrimitiveId`, so a list
    /// that scrolls shader nodes in and out would otherwise retain an instance
    /// index per id that ever existed.
    ///
    /// The painter reuses a prepared batch when nothing changed, so `prepare`
    /// does not run every frame; eviction rides the passes that do run.
    fn begin_prepare_pass(&mut self) {
        if !self.drawn {
            return;
        }
        self.drawn = false;
        self.generation = self.generation.saturating_add(1);
        let oldest = self.generation.saturating_sub(SLOT_RETAIN_PASSES);
        let mut freed = Vec::new();
        self.slots.retain(|_, slot| {
            let live = slot.last_seen >= oldest;
            if !live {
                freed.push(slot.index);
            }
            live
        });
        for index in freed {
            self.free_indices.push(Reverse(index));
        }
    }

    fn write_slot(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: PrimitiveId,
        instance: GpuViewInstance,
    ) {
        let generation = self.generation;
        let index = match self.slots.get(&id) {
            Some(slot) => slot.index,
            None => {
                let index = match self.free_indices.pop() {
                    Some(Reverse(index)) => index,
                    None => {
                        let index = self.next_index;
                        self.next_index = self.next_index.saturating_add(1);
                        index
                    }
                };
                self.grow_instances(device, index + 1);
                index
            }
        };
        let mut instance = instance;
        instance.parameters[2] = self.dest_size[0] as f32;
        instance.parameters[3] = self.dest_size[1] as f32;
        self.slots.insert(
            id,
            PreparedSlot {
                index,
                instance,
                last_seen: generation,
            },
        );
        queue.write_buffer(
            &self.instances,
            index as u64 * std::mem::size_of::<GpuViewInstance>() as u64,
            bytemuck::bytes_of(&instance),
        );
    }

    fn grow_instances(&mut self, device: &wgpu::Device, needed: u32) {
        if needed <= self.instance_capacity {
            return;
        }
        self.instance_capacity = needed.next_power_of_two();
        self.instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui default gpu-view instances"),
            size: (self.instance_capacity as usize * std::mem::size_of::<GpuViewInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.restaged = true;
    }

    /// Rewrite every live instance after the buffer was replaced. Slots not
    /// re-prepared this pass would otherwise point at uninitialized memory.
    fn restage_all(&mut self, queue: &wgpu::Queue) {
        if !self.restaged {
            return;
        }
        self.restaged = false;
        for slot in self.slots.values() {
            queue.write_buffer(
                &self.instances,
                slot.index as u64 * std::mem::size_of::<GpuViewInstance>() as u64,
                bytemuck::bytes_of(&slot.instance),
            );
        }
    }

    fn draw(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        clip: crate::PhysicalRect,
        first: u32,
        count: u32,
        gpu_work: Option<&GpuWorkSink>,
    ) {
        if count == 0 || clip.width == 0 || clip.height == 0 {
            return;
        }
        pass.set_scissor_rect(clip.x, clip.y, clip.width, clip.height);
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, first..first + count);
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            work.record_draw_call();
        }
    }
}

struct PreparedSlot {
    index: u32,
    instance: GpuViewInstance,
    last_seen: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct GpuViewInstance {
    rect: [f32; 4],
    color_a: [f32; 4],
    color_b: [f32; 4],
    parameters: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scene_gpu_renderers_include_gpu_view() {
        let registry = default_scene_gpu_renderers();
        assert!(registry.get(GPU_VIEW_RENDERER).is_some());
        assert!(registry.get("gpu-view").is_some());
    }

    #[test]
    fn resolve_scene_gpu_renderers_keeps_program_registry() {
        let mut program = SceneGpuRendererRegistry::new();
        program.insert("app-renderer", Arc::new(DefaultGpuViewRenderer::new()));
        let resolved =
            resolve_scene_gpu_renderers(Some(program), Some(default_scene_gpu_renderers()))
                .expect("program registry is preserved");
        assert!(resolved.get("app-renderer").is_some());
        assert!(resolved.get(GPU_VIEW_RENDERER).is_none());
    }

    #[test]
    fn resolve_scene_gpu_renderers_does_not_replace_empty_program_registry() {
        let resolved = resolve_scene_gpu_renderers(
            Some(SceneGpuRendererRegistry::new()),
            Some(default_scene_gpu_renderers()),
        )
        .expect("empty Some is not treated as None");
        assert!(resolved.is_empty());
        assert!(resolved.get(GPU_VIEW_RENDERER).is_none());
    }

    #[test]
    fn resolve_scene_gpu_renderers_uses_default_gpu_view_when_program_is_none() {
        let resolved = resolve_scene_gpu_renderers(None, Some(default_scene_gpu_renderers()))
            .expect("fallback registry is used");
        assert!(resolved.get(GPU_VIEW_RENDERER).is_some());
    }

    #[test]
    fn resolve_scene_gpu_renderers_stays_none_without_gpu_fallback() {
        assert!(resolve_scene_gpu_renderers(None, None).is_none());
    }
}
