//! Host-owned default painter for Runtime [`GPU_VIEW_RENDERER`].
//!
//! Uses the caller's Device/Queue and the current frame encoder/target. It does
//! not request a GPU context or perform CPU readback.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use nana_ui_runtime::{CustomRenderNode, GPU_VIEW_RENDERER, GpuViewPalette, gpu_view_params};
use nana_ui_scene::PrimitiveId;

use crate::gpu_view::GPU_VIEW_SHADER;
use crate::scene_gpu::{
    SceneGpuNode, SceneGpuPassContext, SceneGpuPrepareContext, SceneGpuRenderContext,
    SceneGpuRenderer, SceneGpuRendererRegistry,
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
    fn prepare(&self, node: &SceneGpuNode, context: SceneGpuPrepareContext<'_>) {
        let device = self.prepare_device(&context);
        let queue = self.prepare_queue(&context);
        let mut state = self.state.lock().expect("default gpu-view pipeline");
        let prepared =
            state.get_or_insert_with(|| PreparedGpuView::new(device, context.target_format));
        if prepared.format != context.target_format {
            *prepared = PreparedGpuView::new(device, context.target_format);
        }
        let scale = if context.scale_factor.is_finite() && context.scale_factor > 0.0 {
            context.scale_factor
        } else {
            1.0
        };
        let viewport = [
            context.bounds.x * scale,
            context.bounds.y * scale,
            context.bounds.width * scale,
            context.bounds.height * scale,
        ];
        let palette = self.node_palette(&node.custom);
        let uniform = ViewUniform {
            color_a: palette.background,
            color_b: palette.accent,
            parameters: [self.node_seed(&node.custom), 0.0, 0.0, 0.0],
        };
        if !prepared.slots.contains_key(&node.id) {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui default gpu-view uniform"),
                size: std::mem::size_of::<ViewUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nana-ui default gpu-view bind group"),
                layout: &prepared.bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            prepared.slots.insert(
                node.id,
                PreparedSlot {
                    buffer,
                    bind_group,
                    viewport,
                },
            );
        }
        let entry = prepared
            .slots
            .get_mut(&node.id)
            .expect("default gpu-view slot prepared");
        entry.viewport = viewport;
        queue.write_buffer(&entry.buffer, 0, bytemuck::bytes_of(&uniform));
        if let Some(work) = context.gpu_work {
            work.record_upload(std::mem::size_of::<ViewUniform>());
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
                dest_size: [
                    context.bounds.x.saturating_add(context.bounds.width).max(1),
                    context
                        .bounds
                        .y
                        .saturating_add(context.bounds.height)
                        .max(1),
                ],
                gpu_work: context.gpu_work,
            },
        );
        drop(render_pass);
    }

    fn draw_in_pass(
        &self,
        node: &SceneGpuNode,
        pass: &mut wgpu::RenderPass<'_>,
        context: SceneGpuPassContext<'_>,
    ) -> bool {
        let state = self.state.lock().expect("default gpu-view pipeline");
        let Some(prepared) = state.as_ref() else {
            return false;
        };
        let Some(slot) = prepared.slots.get(&node.id) else {
            return false;
        };
        if context.bounds.width == 0 || context.bounds.height == 0 {
            return false;
        }
        pass.set_viewport(
            slot.viewport[0],
            slot.viewport[1],
            slot.viewport[2].max(1.0),
            slot.viewport[3].max(1.0),
            0.0,
            1.0,
        );
        pass.set_scissor_rect(
            context.clip.x,
            context.clip.y,
            context.clip.width,
            context.clip.height,
        );
        pass.set_pipeline(&prepared.pipeline);
        pass.set_bind_group(0, &slot.bind_group, &[]);
        pass.draw(0..3, 0..1);
        pass.set_viewport(
            0.0,
            0.0,
            context.dest_size[0].max(1) as f32,
            context.dest_size[1].max(1) as f32,
            0.0,
            1.0,
        );
        if let Some(work) = context.gpu_work {
            work.record_draw_batch();
            work.record_draw_call();
        }
        true
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

struct PreparedGpuView {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    slots: HashMap<PrimitiveId, PreparedSlot>,
}

impl PreparedGpuView {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui default gpu-view shader"),
            source: wgpu::ShaderSource::Wgsl(GPU_VIEW_SHADER.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui default gpu-view bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui default gpu-view pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui default gpu-view pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                buffers: &[],
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
            bind_group_layout,
            format,
            slots: HashMap::new(),
        }
    }
}

struct PreparedSlot {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    viewport: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ViewUniform {
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
