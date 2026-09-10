//! Nana-owned WGPU painter for [`UiScene`].
//!
//! This is the product Scene backend. It paints [`UiScene`] through the host GPU.
//! The host owns Device/Queue/encoder. HostTexture is sampled in document order
//! inside the current dest (or opacity-group) pass; groups do not open a pass
//! per HostTexture slot.

mod backdrop;
mod buffer_upload;
mod clip;
mod color;
mod dest;
mod host_texture;
mod icon;
pub(crate) mod image_url;
mod mesh;
mod quad;
mod text;
pub(crate) mod url_texture_cache;
mod validate;

use std::{sync::Arc, time::Instant};

use nana_ui_core::GpuWorkObservation;
use nana_ui_scene::{RenderOperation, ScenePrimitiveKind, UiScene};

use crate::{
    HostTextureRegistry, PhysicalRect,
    gpu_work::{GpuStageTimings, GpuWorkSink},
    scene_gpu::{
        SceneGpuBatchNode, SceneGpuBatchPassContext, SceneGpuNode, SceneGpuPassContext,
        SceneGpuPrepareContext, SceneGpuRenderContext, SceneGpuRenderer, SceneGpuRendererRegistry,
    },
};

pub use image_url::{
    resolve_background_image_url, resolved_resource_is_allowed, set_background_image_url_base,
};
use validate::validate_scene;
pub use validate::{HostTextureSceneResolver, ScenePaintError};

use backdrop::BackdropPipeline;
use clip::{
    FragmentClip, LogicalRect, extra_fragment_clips, fragment_clip, intersect_clips, local_rect,
    mesh_extra_fragment_clips, overlaps_physical, paint_origin, paint_transform, physical_bounds,
    physical_scissor, transformed_aabb, transformed_aabb_projective, union_physical,
};
use color::{pack_linear, with_opacity};
use dest::{DestPassCounts, DestTarget, GroupSlot};
use host_texture::{HostTexturePipeline, PreparedHostTexture};
use icon::{IconPipeline, PreparedIcon};
use mesh::{MeshPipeline, MeshRange, StrokeStyle};
use quad::QuadPipeline;
use text::{PreparedText, TextPipeline};

#[derive(Clone, Copy, PartialEq)]
pub struct ScenePaintViewport {
    /// Scene dest rect size in logical pixels.
    pub logical_size: [f32; 2],
    /// Full target texture size in physical pixels.
    pub physical_size: [u32; 2],
    pub scale_factor: f32,
    pub scene_origin: [f32; 2],
    /// Logical position of scene (0, 0) on the target.
    pub target_origin: [f32; 2],
    pub clear_color: [f32; 4],
    /// Clear the whole target; otherwise keep existing pixels (`LoadOp::Load`).
    pub clear: bool,
}

pub struct SceneWgpuPainter {
    targets: std::collections::HashMap<RenderTargetId, TargetState>,
    prepared_batch: Option<PreparedBatch>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    quads: QuadPipeline,
    meshes: MeshPipeline,
    icons: IconPipeline,
    text: TextPipeline,
    host_textures: HostTexturePipeline,
    backdrop: BackdropPipeline,
    dest: Option<DestTarget>,
    /// Shared across resize-driven `DestTarget` recreations so the blit
    /// pipeline is not recompiled on every interactive resize event. `None`
    /// when the host device lacks `PIPELINE_CACHE`.
    dest_pipeline_cache: Option<wgpu::PipelineCache>,
    last_gpu_work: Option<GpuWorkObservation>,
    last_gpu_timings: Option<GpuStageTimings>,
    last_dest_pass_counts: Option<DestPassCounts>,
    /// Last fully scene-described dest; host textures / GPU slots skip reuse.
    painted: Option<PaintedDest>,
    image_revision: u64,
}

#[derive(Clone, Copy, PartialEq)]
struct PaintedDest {
    instance: u64,
    viewport: ScenePaintViewport,
    size: [u32; 2],
}

/// Host-owned identity for a window or offscreen presentation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderTargetId(pub u64);

#[derive(Default)]
struct TargetState {
    prepared_batch: Option<PreparedBatch>,
    quads: Option<quad::QuadPipelineTarget>,
    meshes: Option<mesh::MeshPipelineTarget>,
    icons: Option<icon::IconPipelineTarget>,
    text: Option<text::TextPipelineTarget>,
    backdrop: Option<backdrop::BackdropPipelineTarget>,
    host_textures: crate::gpu_texture::GpuTextureTarget,
    dest: Option<DestTarget>,
    painted: Option<PaintedDest>,
    image_revision: u64,
}

struct PreparedBatch {
    key: PaintedDest,
    resources: Vec<TextureBindingKey>,
    renderers: Vec<(usize, u64)>,
    image_revision: u64,
    commands: Vec<DrawCommand>,
    max_group_depth: usize,
    group_slots: Vec<dest::GroupSlot>,
    /// Read off the document order that built `commands`; it cannot be
    /// recovered from the merged list.
    glyph_then_quad: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct TextureBindingKey {
    identity: u64,
    generation: u64,
    width: u32,
    height: u32,
    alpha: crate::HostTextureAlphaMode,
}

enum DrawCommand {
    Quads {
        range: std::ops::Range<u32>,
        scissor: PhysicalRect,
    },
    Mesh {
        range: MeshRange,
        scissor: PhysicalRect,
    },
    Text {
        prepared: PreparedText,
        scissor: PhysicalRect,
    },
    Icon {
        first: usize,
        count: usize,
        scissor: PhysicalRect,
    },
    HostTexture(PreparedHostTexture),
    Custom {
        node: SceneGpuNode,
        renderer: Arc<dyn SceneGpuRenderer>,
        bounds: PhysicalRect,
        clip: PhysicalRect,
    },
    Backdrop {
        index: u32,
    },
    PushGroup {
        layer: usize,
        slot: u32,
    },
    PopGroup,
}

impl SceneWgpuPainter {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            targets: std::collections::HashMap::new(),
            prepared_batch: None,
            device: device.clone(),
            queue: queue.clone(),
            format,
            quads: QuadPipeline::new(device, format),
            meshes: MeshPipeline::new(device, format),
            icons: IconPipeline::new(device, format),
            text: TextPipeline::new(device, queue, format),
            host_textures: HostTexturePipeline::new(device, queue, format),
            backdrop: BackdropPipeline::new(device, format),
            dest: None,
            // Pipeline-cache reuse requires a host-enabled device feature;
            // the painter must not demand it, so degrade to per-recreate
            // compilation when the host did not opt in.
            // SAFETY: `data: None` loads no untrusted cache blob; the cache
            // only lets the driver reuse compilation state in-process.
            dest_pipeline_cache: device
                .features()
                .contains(wgpu::Features::PIPELINE_CACHE)
                .then(|| unsafe {
                    device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                        label: Some("nana-ui.scene.dest.cache"),
                        data: None,
                        fallback: true,
                    })
                }),
            last_gpu_work: None,
            last_gpu_timings: None,
            last_dest_pass_counts: None,
            painted: None,
            image_revision: 0,
        }
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Wake the owning event loop when an asynchronous URL image completes.
    pub fn set_image_waker(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        let keyed: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |_| wake());
        self.set_image_update_waker(keyed);
    }

    /// Wake the owning host with the URL/resource key that completed.
    /// Hosts can use this to redraw only targets that reference the resource.
    pub fn set_image_update_waker(&mut self, wake: Arc<dyn Fn(&str) + Send + Sync>) {
        self.quads.set_image_waker(wake.clone());
        self.host_textures.set_image_waker(wake);
    }

    pub fn has_image_updates(&self) -> bool {
        self.quads.has_image_updates() || self.host_textures.has_image_updates()
    }

    /// Offscreen hosts can await completion and paint again without polling pixels.
    pub fn has_pending_images(&self) -> bool {
        self.quads.has_pending_images() || self.host_textures.has_pending_images()
    }

    /// Completion generation, used by layered snapshot hosts to repaint earlier layers.
    pub fn image_revision(&self) -> u64 {
        self.image_revision
    }

    /// GPU counters from the last `paint` that encoded a real command buffer.
    /// `None` if this painter has not encoded, or the last `paint` was skipped.
    pub fn last_gpu_work(&self) -> Option<GpuWorkObservation> {
        self.last_gpu_work
    }

    /// Batch / upload / encode timings from the last encoded `paint`. Submit is
    /// filled only after the host calls [`Self::record_submit`].
    pub fn last_gpu_timings(&self) -> Option<GpuStageTimings> {
        self.last_gpu_timings
    }

    /// Text shape-cache counters from the last `paint`: (hits, misses,
    /// evictions). Tests use this to pin the reshaping-skip contract.
    pub fn text_shape_cache_stats(&self) -> (usize, usize, usize) {
        self.text.shape_cache_stats()
    }

    /// Affine (rotated / skewed) text GPU resource cache counters: (hits,
    /// misses, evictions). Each miss creates an atlas texture, a bind group and
    /// a vertex buffer, so a static transform must not keep missing.
    pub fn affine_text_cache_stats(&self) -> (usize, usize, usize) {
        self.text.affine_cache_stats()
    }

    /// Record host `queue.submit` duration for the last encoded frame.
    pub fn record_submit(&mut self, duration: std::time::Duration) {
        if let Some(timings) = &mut self.last_gpu_timings {
            timings.submit = timings.submit.saturating_add(duration);
        }
    }

    /// Release target-owned textures when a host closes a window or viewport.
    pub fn remove_target(&mut self, id: RenderTargetId) {
        self.targets.remove(&id);
    }

    /// Paint with isolated target state while sharing device pipelines/caches.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_target(
        &mut self,
        id: RenderTargetId,
        scene: &UiScene,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        let mut state = self.targets.remove(&id).unwrap_or_default();
        if state.image_revision != self.image_revision {
            state.painted = None;
            QuadPipeline::invalidate_target_image_bindings(&mut state.quads);
            HostTexturePipeline::invalidate_target_image_bindings(&mut state.host_textures);
        }
        self.swap_target_state(&mut state);
        let result = self.paint(
            scene,
            encoder,
            target,
            viewport,
            host_textures,
            gpu_renderers,
        );
        self.swap_target_state(&mut state);
        state.image_revision = self.image_revision;
        self.targets.insert(id, state);
        result
    }
    fn swap_target_state(&mut self, state: &mut TargetState) {
        std::mem::swap(&mut self.dest, &mut state.dest);
        std::mem::swap(&mut self.painted, &mut state.painted);
        std::mem::swap(&mut self.prepared_batch, &mut state.prepared_batch);
        self.quads.swap_target(&mut state.quads, &self.device);
        self.meshes.swap_target(&mut state.meshes, &self.device);
        self.icons.swap_target(&mut state.icons, &self.device);
        self.text
            .swap_target(&mut state.text, &self.device, &self.queue, self.format);
        self.backdrop.swap_target(&mut state.backdrop, &self.device);
        self.host_textures.swap_target(&mut state.host_textures);
    }

    pub fn paint(
        &mut self,
        scene: &UiScene,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        // A rejected frame must not report a previous target's successful work.
        self.last_gpu_work = None;
        self.last_gpu_timings = None;
        self.last_dest_pass_counts = None;
        let quad_images = self.quads.poll_images();
        let mask_images = self.host_textures.poll_images();
        if quad_images || mask_images {
            self.painted = None;
            self.image_revision = self.image_revision.wrapping_add(1);
            // The active target is held in the painter fields while `paint`
            // runs; invalidate its URL-backed bindings immediately. Other
            // targets are invalidated when their revision is observed above.
            self.quads.invalidate_image_bindings();
            self.host_textures.invalidate_image_bindings();
        }
        let instance = scene.instance_id();
        // Also resolves this frame's `PreparedBatch` key, so nothing below
        // walks `FramePlan::custom_nodes` again.
        let resolved = validate_scene(scene, host_textures, gpu_renderers)?;
        if viewport.physical_size[0] == 0 || viewport.physical_size[1] == 0 {
            self.last_gpu_work = None;
            self.last_gpu_timings = None;
            self.last_dest_pass_counts = None;
            self.painted = None;
            return Ok(());
        }

        let scale = if viewport.scale_factor.is_finite() && viewport.scale_factor > 0.0 {
            viewport.scale_factor
        } else {
            1.0
        };
        let dest_physical = [
            (viewport.logical_size[0] * scale).round().max(1.0) as u32,
            (viewport.logical_size[1] * scale).round().max(1.0) as u32,
        ];
        let blit_origin = [
            viewport.target_origin[0] * scale,
            viewport.target_origin[1] * scale,
        ];
        let origin = paint_origin([0.0, 0.0], viewport.scene_origin);
        let viewport_clip = LogicalRect::viewport([0.0, 0.0], viewport.logical_size);
        let gpu_work = GpuWorkSink::new();
        let clear = wgpu::Color {
            r: viewport.clear_color[0] as f64,
            g: viewport.clear_color[1] as f64,
            b: viewport.clear_color[2] as f64,
            a: viewport.clear_color[3] as f64,
        };
        let painted = PaintedDest {
            instance,
            viewport,
            size: dest_physical,
        };

        if self.painted == Some(painted)
            && let Some(dest) = self.dest.as_ref()
            && dest.width == dest_physical[0]
            && dest.height == dest_physical[1]
        {
            let encode_started = Instant::now();
            let mut dest_passes = DestPassCounts {
                msaa_allocated: dest.msaa_allocated,
                ..DestPassCounts::default()
            };
            dest.blit(
                encoder,
                target,
                blit_origin[0],
                blit_origin[1],
                viewport.physical_size,
                viewport.clear.then_some(clear),
                Some(&gpu_work),
                &mut dest_passes,
            );
            self.last_gpu_work = Some(gpu_work.snapshot());
            self.last_gpu_timings = Some(GpuStageTimings {
                batch: std::time::Duration::ZERO,
                gpu_upload: std::time::Duration::ZERO,
                encode: encode_started.elapsed(),
                submit: std::time::Duration::ZERO,
            });
            self.last_dest_pass_counts = Some(dest_passes);
            return Ok(());
        }

        let cached = self.prepared_batch.take().filter(|batch| {
            resolved.cacheable
                && batch.key == painted
                && batch.resources == resolved.resources
                && batch.renderers == resolved.renderers
                && batch.image_revision == self.image_revision
        });
        let reused = cached.is_some();
        let (commands, max_group_depth, group_slots_uniforms, glyph_then_quad, batch, gpu_upload) =
            if let Some(cached) = cached {
                (
                    cached.commands,
                    cached.max_group_depth,
                    cached.group_slots,
                    cached.glyph_then_quad,
                    std::time::Duration::ZERO,
                    std::time::Duration::ZERO,
                )
            } else {
                let batch_started = Instant::now();
                self.quads.begin_frame();
                self.host_textures.begin_frame();
                self.meshes.begin_frame();
                self.icons.begin_frame(dest_physical);
                self.text.begin_frame(&self.queue, dest_physical);
                self.backdrop.begin_frame();

                let mut commands = Vec::new();
                let mut batching = Batching::default();
                let mut group_stack: Vec<nana_ui_scene::OpacityGroup> = Vec::new();
                let mut group_depth = 0usize;
                let mut group_slots = 0u32;
                let mut max_group_depth = 0usize;
                let mut group_slots_uniforms = Vec::new();
                // Scissors/text clips round to physical pixels. Include their
                // possible edge coverage even at fractional origins/low DPI.
                let edge_padding = 1.0 / scale;
                let operations = scene
                    .visible_operations(nana_ui_scene::SceneRect {
                        x: viewport.scene_origin[0] - edge_padding,
                        y: viewport.scene_origin[1] - edge_padding,
                        width: viewport.logical_size[0] + edge_padding * 2.0,
                        height: viewport.logical_size[1] + edge_padding * 2.0,
                    })
                    .map_err(|_| ScenePaintError::InvalidRenderGraph)?;
                for operation in operations.iter() {
                    let id = match operation {
                        RenderOperation::PrepareExternal(_) => continue,
                        RenderOperation::Draw(id) | RenderOperation::InvokeCustom(id) => *id,
                    };
                    let Some(primitive) = scene.draw_primitive(id) else {
                        continue;
                    };
                    if sync_opacity_groups(
                        &mut commands,
                        &mut group_stack,
                        &mut group_depth,
                        &mut group_slots,
                        &mut max_group_depth,
                        &mut group_slots_uniforms,
                        scene.opacity_groups(primitive.node),
                        scene,
                        origin,
                        scale,
                    ) {
                        batching.close_all();
                    }
                    let Some(clip) = intersect_clips(viewport_clip, &primitive.clips, origin)
                    else {
                        continue;
                    };
                    let Some(scissor) = physical_scissor(clip, scale, dest_physical) else {
                        continue;
                    };
                    let frag_clip = fragment_clip(&primitive.clips, origin);
                    let (affine, persp) =
                        paint_transform(primitive.transform.0, primitive.transform.1, origin);
                    let bounds = local_rect(primitive.bounds);
                    // Wrapping drains and re-inserts this primitive's commands
                    // behind a `PushGroup`, so no batch may span the edit.
                    let clip_dests = clip_dests_for(&primitive.kind, &primitive.clips, origin);
                    if !clip_dests.is_empty() {
                        batching.close_all();
                    }
                    let command_start = commands.len();
                    match &primitive.kind {
                        ScenePrimitiveKind::Quad {
                            background,
                            border_color,
                            border_width,
                            corner_radius,
                            shadow,
                            surface,
                        } => {
                            let painted = quad_painted_bounds(
                                bounds,
                                shadow.as_ref(),
                                surface,
                                affine,
                                persp,
                                scale,
                                scissor,
                                dest_physical,
                            );
                            if let Some(index) = self.quads.push(
                                &self.device,
                                &self.queue,
                                bounds,
                                clip,
                                frag_clip,
                                affine,
                                persp,
                                *background,
                                *border_color,
                                *border_width,
                                *corner_radius,
                                *shadow,
                                primitive.opacity,
                                surface,
                            ) {
                                if let Some(filter) =
                                    surface.backdrop_filter.filter(|f| f.is_active())
                                {
                                    let world_bounds =
                                        if clip::is_translation_projective(affine, persp) {
                                            bounds
                                        } else {
                                            transformed_aabb_projective(bounds, affine, persp)
                                        };
                                    let phys = physical_bounds(world_bounds, scale, scissor);
                                    let radii = corner_radius.map(|r| r * scale);
                                    let bidx = self.backdrop.push(
                                        index,
                                        [
                                            phys.x as f32,
                                            phys.y as f32,
                                            phys.width as f32,
                                            phys.height as f32,
                                        ],
                                        radii,
                                        filter,
                                        frag_clip.for_physical_pixels(scale),
                                        scale,
                                        dest_physical,
                                        bounds,
                                        affine,
                                    );
                                    batching.close_all();
                                    commands.push(DrawCommand::Backdrop { index: bidx });
                                }
                                for quad_index in index..self.quads.pending_len() {
                                    push_quad(
                                        &mut commands,
                                        &mut batching,
                                        quad_index,
                                        scissor,
                                        painted,
                                    );
                                }
                            }
                        }
                        ScenePrimitiveKind::QuadBatch {
                            bounds: batch,
                            background,
                            border_color,
                            border_width,
                            corner_radius,
                            shadow,
                            surface,
                        } => {
                            for item in batch {
                                let item_bounds = local_rect(*item);
                                let painted = quad_painted_bounds(
                                    item_bounds,
                                    shadow.as_ref(),
                                    surface,
                                    affine,
                                    persp,
                                    scale,
                                    scissor,
                                    dest_physical,
                                );
                                if let Some(index) = self.quads.push(
                                    &self.device,
                                    &self.queue,
                                    item_bounds,
                                    clip,
                                    frag_clip,
                                    affine,
                                    persp,
                                    *background,
                                    *border_color,
                                    *border_width,
                                    *corner_radius,
                                    *shadow,
                                    primitive.opacity,
                                    surface,
                                ) {
                                    if let Some(filter) =
                                        surface.backdrop_filter.filter(|f| f.is_active())
                                    {
                                        let world_bounds =
                                            if clip::is_translation_projective(affine, persp) {
                                                item_bounds
                                            } else {
                                                transformed_aabb_projective(
                                                    item_bounds,
                                                    affine,
                                                    persp,
                                                )
                                            };
                                        let phys = physical_bounds(world_bounds, scale, scissor);
                                        let radii = corner_radius.map(|r| r * scale);
                                        let bidx = self.backdrop.push(
                                            index,
                                            [
                                                phys.x as f32,
                                                phys.y as f32,
                                                phys.width as f32,
                                                phys.height as f32,
                                            ],
                                            radii,
                                            filter,
                                            frag_clip.for_physical_pixels(scale),
                                            scale,
                                            dest_physical,
                                            item_bounds,
                                            affine,
                                        );
                                        batching.close_all();
                                        commands.push(DrawCommand::Backdrop { index: bidx });
                                    }
                                    for quad_index in index..self.quads.pending_len() {
                                        push_quad(
                                            &mut commands,
                                            &mut batching,
                                            quad_index,
                                            scissor,
                                            painted,
                                        );
                                    }
                                }
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
                            max_lines,
                            shaping,
                            horizontal_alignment,
                            vertical_alignment,
                            spans,
                            letter_spacing,
                            text_shadow,
                            underline: _,
                            line_through: _,
                            font_features,
                            italic,
                            wrap_break,
                            opentype,
                        } => {
                            let mut push_text =
                                |commands: &mut Vec<DrawCommand>,
                                 batching: &mut Batching,
                                 extra_offset: [f32; 2],
                                 color_override: Option<[f32; 4]>| {
                                    let prepared = self.text.prepare(
                                        &self.device,
                                        &self.queue,
                                        encoder,
                                        bounds,
                                        clip,
                                        scale,
                                        content,
                                        color_override.or(*color),
                                        *size,
                                        *weight,
                                        family.as_deref(),
                                        *line_height,
                                        *wrap,
                                        *wrap_break,
                                        *italic,
                                        *ellipsis,
                                        *max_lines,
                                        *shaping,
                                        *horizontal_alignment,
                                        *vertical_alignment,
                                        spans,
                                        *letter_spacing,
                                        font_features,
                                        opentype,
                                        affine,
                                        persp,
                                        frag_clip,
                                        primitive.opacity,
                                        extra_offset,
                                    );
                                    if let Some(prepared) = prepared {
                                        let painted = painted_bounds(
                                            prepared.ink,
                                            0.0,
                                            affine,
                                            persp,
                                            scale,
                                            scissor,
                                        );
                                        push_text_run(
                                            commands,
                                            batching,
                                            &mut self.text,
                                            prepared,
                                            scissor,
                                            painted,
                                        );
                                    }
                                };
                            if let Some(shadow) = text_shadow {
                                let base_color = with_opacity(shadow.color, primitive.opacity);
                                for (dx, dy, alpha_scale) in text_shadow_draw_offsets(*shadow) {
                                    let scaled = [
                                        base_color[0],
                                        base_color[1],
                                        base_color[2],
                                        base_color[3] * alpha_scale,
                                    ];
                                    push_text(
                                        &mut commands,
                                        &mut batching,
                                        [shadow.offset_x + dx, shadow.offset_y + dy],
                                        Some(scaled),
                                    );
                                }
                            }
                            push_text(&mut commands, &mut batching, [0.0, 0.0], None);
                        }
                        ScenePrimitiveKind::QuadColorBatch {
                            bounds: batch,
                            colors,
                            border_color,
                            border_width,
                            corner_radius,
                        } => {
                            // Per-item solid colors (editor color swatches); no
                            // shadow and no surface paint by construction, so the
                            // batch collapses to one quads.push per item.
                            let no_shadow: Option<nana_ui_runtime::ComponentElevation> = None;
                            let default_surface = nana_ui_scene::QuadSurfacePaint::default();
                            for (item, color) in batch.iter().zip(colors.iter()) {
                                let item_bounds = local_rect(*item);
                                let painted = quad_painted_bounds(
                                    item_bounds,
                                    None,
                                    &default_surface,
                                    affine,
                                    persp,
                                    scale,
                                    scissor,
                                    dest_physical,
                                );
                                if let Some(index) = self.quads.push(
                                    &self.device,
                                    &self.queue,
                                    item_bounds,
                                    clip,
                                    frag_clip,
                                    affine,
                                    persp,
                                    Some(*color),
                                    *border_color,
                                    *border_width,
                                    *corner_radius,
                                    no_shadow,
                                    primitive.opacity,
                                    &default_surface,
                                ) {
                                    for quad_index in index..self.quads.pending_len() {
                                        push_quad(
                                            &mut commands,
                                            &mut batching,
                                            quad_index,
                                            scissor,
                                            painted,
                                        );
                                    }
                                }
                            }
                        }
                        ScenePrimitiveKind::Icon { icon, color } => {
                            if let Some(prepared) = self.icons.prepare(
                                &self.device,
                                &self.queue,
                                bounds,
                                affine,
                                persp,
                                scale,
                                *icon,
                                color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                                primitive.opacity,
                                frag_clip,
                            ) {
                                push_icon(
                                    &mut commands,
                                    &mut batching,
                                    &self.icons,
                                    prepared,
                                    scissor,
                                    painted_bounds(bounds, 0.0, affine, persp, scale, scissor),
                                );
                            }
                        }
                        ScenePrimitiveKind::IconBatch {
                            bounds: batch,
                            icon,
                            color,
                        } => {
                            for item in batch {
                                let item_bounds = local_rect(*item);
                                if let Some(prepared) = self.icons.prepare(
                                    &self.device,
                                    &self.queue,
                                    item_bounds,
                                    affine,
                                    persp,
                                    scale,
                                    *icon,
                                    color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                                    primitive.opacity,
                                    frag_clip,
                                ) {
                                    push_icon(
                                        &mut commands,
                                        &mut batching,
                                        &self.icons,
                                        prepared,
                                        scissor,
                                        painted_bounds(
                                            item_bounds,
                                            0.0,
                                            affine,
                                            persp,
                                            scale,
                                            scissor,
                                        ),
                                    );
                                }
                            }
                        }
                        ScenePrimitiveKind::Spinner { phase, color } => {
                            if let Some(range) = self.meshes.push_spinner(
                                bounds,
                                mesh_affine(affine, persp),
                                *phase,
                                color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                                primitive.opacity,
                                frag_clip,
                            ) {
                                push_mesh_draw(
                                    &mut commands,
                                    &mut batching,
                                    range,
                                    scissor,
                                    painted_bounds(
                                        bounds,
                                        0.0,
                                        mesh_affine(affine, persp),
                                        [0.0, 0.0],
                                        scale,
                                        scissor,
                                    ),
                                );
                            }
                        }
                        ScenePrimitiveKind::Stroke {
                            points,
                            width,
                            color,
                            widths,
                            cap,
                            pattern,
                        } => {
                            let (dash, dash_offset, colors, path_length) = match pattern.as_deref()
                            {
                                Some(pattern) => (
                                    pattern.dash.as_slice(),
                                    pattern.dash_offset,
                                    pattern.colors.as_slice(),
                                    pattern.path_length,
                                ),
                                None => ([].as_slice(), 0.0, [].as_slice(), 0.0),
                            };
                            if let Some(range) = self.meshes.push_stroke_with_path_length(
                                points,
                                StrokeStyle {
                                    width: *width,
                                    widths,
                                    cap: *cap,
                                    dash,
                                    dash_offset,
                                    colors,
                                },
                                mesh_affine(affine, persp),
                                *color,
                                primitive.opacity,
                                frag_clip,
                                path_length,
                            ) {
                                // Caps and joins reach half a stroke width past
                                // the polyline; one more pixel covers AA.
                                let reach =
                                    widths.iter().copied().fold(*width, f32::max).max(0.0) + 1.0;
                                push_mesh_draw(
                                    &mut commands,
                                    &mut batching,
                                    range,
                                    scissor,
                                    painted_bounds(
                                        polyline_bounds(points).unwrap_or(bounds),
                                        reach,
                                        mesh_affine(affine, persp),
                                        [0.0, 0.0],
                                        scale,
                                        scissor,
                                    ),
                                );
                            }
                        }
                        ScenePrimitiveKind::Custom { node: custom, mask } => {
                            if custom.renderer.as_ref() == "nana.host-texture" {
                                // The registry is a shared RwLock: an entry validated at
                                // frame start can be removed before prepare. Skip the
                                // node for this frame instead of panicking.
                                let Some(binding) = host_textures
                                    .and_then(|registry| registry.get(custom.resource.as_ref()))
                                else {
                                    continue;
                                };
                                let dest = nana_ui_core::LogicalRect::new(
                                    bounds.x,
                                    bounds.y,
                                    bounds.width,
                                    bounds.height,
                                )
                                .fitted(
                                    binding.width as f32,
                                    binding.height as f32,
                                    custom.fit,
                                );
                                let (rounded_clip, corner_radius) = scene
                                    .primitive(nana_ui_scene::PrimitiveId {
                                        node: primitive.id.node,
                                        slot: 0,
                                    })
                                    .and_then(|quad| match &quad.kind {
                                        ScenePrimitiveKind::Quad { corner_radius, .. } => {
                                            let radius = corner_radius
                                                .iter()
                                                .copied()
                                                .fold(0.0f32, f32::max);
                                            Some((local_rect(quad.bounds), radius))
                                        }
                                        _ => None,
                                    })
                                    .unwrap_or((bounds, 0.0));
                                batching.close_all();
                                commands.push(DrawCommand::HostTexture(
                                    self.host_textures.prepare(
                                        &self.device,
                                        &self.queue,
                                        binding,
                                        primitive.id.node.get(),
                                        primitive.id.slot,
                                        LogicalRect::from_xywh(
                                            dest.x,
                                            dest.y,
                                            dest.width,
                                            dest.height,
                                        ),
                                        affine,
                                        persp,
                                        scissor,
                                        primitive.opacity,
                                        corner_radius,
                                        rounded_clip,
                                        frag_clip,
                                        dest_physical,
                                        scale,
                                        mask.clone(),
                                        Some(&gpu_work),
                                        custom.checkerboard,
                                        custom.zoom,
                                    ),
                                ));
                            } else {
                                let Some(renderer) = gpu_renderers
                                    .and_then(|registry| registry.get(custom.renderer.as_ref()))
                                else {
                                    continue;
                                };
                                let node = SceneGpuNode {
                                    id: primitive.id,
                                    custom: custom.clone(),
                                    opacity: primitive.opacity,
                                };
                                let custom_bounds = custom_paint_bounds(bounds, affine, persp);
                                renderer.prepare(
                                    &node,
                                    SceneGpuPrepareContext {
                                        device: &self.device,
                                        queue: &self.queue,
                                        target_format: self.format,
                                        bounds: custom_bounds.to_core(),
                                        scale_factor: scale,
                                        dest_size: dest_physical,
                                        gpu_work: Some(&gpu_work),
                                    },
                                );
                                batching.close_all();
                                commands.push(DrawCommand::Custom {
                                    node,
                                    renderer,
                                    bounds: physical_bounds(custom_bounds, scale, scissor),
                                    clip: scissor,
                                });
                            }
                        }
                    }
                    if !clip_dests.is_empty()
                        && wrap_drawn_with_clip_dests(
                            &mut commands,
                            command_start,
                            &mut group_depth,
                            &mut group_slots,
                            &mut max_group_depth,
                            &mut group_slots_uniforms,
                            &clip_dests,
                            scale,
                        )
                    {
                        batching.close_all();
                    }
                }
                if sync_opacity_groups(
                    &mut commands,
                    &mut group_stack,
                    &mut group_depth,
                    &mut group_slots,
                    &mut max_group_depth,
                    &mut group_slots_uniforms,
                    Vec::new(),
                    scene,
                    origin,
                    scale,
                ) {
                    batching.close_all();
                }
                // Text prepare stays inside the batch window: it is the same
                // work the per-primitive prepare did, only once per run.
                self.text.flush_runs(&self.device, &self.queue, encoder);
                let batch = batch_started.elapsed();

                let upload_started = Instant::now();
                self.quads.upload(
                    &self.device,
                    &self.queue,
                    dest_physical,
                    scale,
                    Some(&gpu_work),
                );
                self.meshes.upload(
                    &self.device,
                    &self.queue,
                    dest_physical,
                    scale,
                    Some(&gpu_work),
                );
                self.icons
                    .upload(&self.device, &self.queue, Some(&gpu_work));
                self.backdrop
                    .upload(&self.device, &self.queue, dest_physical, Some(&gpu_work));
                let gpu_upload = upload_started.elapsed();

                (
                    commands,
                    max_group_depth,
                    group_slots_uniforms,
                    batching.glyph_then_quad,
                    batch,
                    gpu_upload,
                )
            };

        let encode_started = Instant::now();
        // The MSAA-geometry then single-sample-glyph optimization is valid only
        // when glyphs form a suffix. A later panel, modal scrim or mesh must
        // cover earlier text/icons in document order, so `Batching` reads that
        // half off the emit order — folding a quad into an earlier batch can
        // leave glyphs at the end and must not change the sample count. The
        // commands that rule out MSAA outright are never merged, so they are
        // still read from the list.
        let gpu_interleaved = glyph_then_quad
            || commands.iter().any(|command| {
                matches!(
                    command,
                    DrawCommand::HostTexture(_)
                        | DrawCommand::Custom { .. }
                        | DrawCommand::PushGroup { .. }
                        | DrawCommand::PopGroup
                        | DrawCommand::Backdrop { .. }
                )
            })
            || self.backdrop.needs_backdrop();
        DestTarget::ensure(
            &mut self.dest,
            &self.device,
            self.dest_pipeline_cache.as_ref(),
            self.format,
            dest_physical[0],
            dest_physical[1],
            !gpu_interleaved,
        );
        if max_group_depth > 0 {
            self.dest.as_mut().expect("dest target").prepare_groups(
                &self.device,
                &self.queue,
                max_group_depth,
                &group_slots_uniforms,
                Some(&gpu_work),
            );
        }
        let dest = self.dest.as_ref().expect("dest target");
        let sample_count = if gpu_interleaved { 1 } else { 4 };
        let initial_load = if viewport.clear {
            wgpu::LoadOp::Clear(clear)
        } else {
            wgpu::LoadOp::Load
        };
        let mut dest_passes = DestPassCounts {
            msaa_allocated: dest.msaa_allocated,
            ..DestPassCounts::default()
        };
        if gpu_interleaved {
            encode_ordered(
                &mut EncodeOrdered {
                    quads: &self.quads,
                    meshes: &self.meshes,
                    icons: &self.icons,
                    text: &self.text,
                    host_textures: &self.host_textures,
                    backdrop: &mut self.backdrop,
                    device: &self.device,
                    queue: &self.queue,
                    gpu_work: &gpu_work,
                },
                encoder,
                dest,
                dest_physical,
                initial_load,
                &commands,
                &mut dest_passes,
            );
        } else {
            let mut pass = dest.begin_msaa_pass(encoder, clear, &mut dest_passes);
            restore_dest_viewport(&mut pass, dest_physical);
            for command in &commands {
                match command {
                    DrawCommand::Quads { range, scissor } => {
                        self.quads.draw(
                            &mut pass,
                            range.clone(),
                            *scissor,
                            sample_count,
                            Some(&gpu_work),
                        );
                    }
                    DrawCommand::Mesh { range, scissor } => {
                        self.meshes
                            .draw(&mut pass, range, *scissor, sample_count, Some(&gpu_work));
                    }
                    DrawCommand::Text { .. }
                    | DrawCommand::Icon { .. }
                    | DrawCommand::HostTexture(_)
                    | DrawCommand::Backdrop { .. }
                    | DrawCommand::Custom { .. }
                    | DrawCommand::PushGroup { .. }
                    | DrawCommand::PopGroup => {}
                }
            }
            drop(pass);
            if commands.iter().any(|command| {
                matches!(command, DrawCommand::Text { .. } | DrawCommand::Icon { .. })
            }) {
                let mut pass = dest.begin_color_pass(encoder, wgpu::LoadOp::Load, &mut dest_passes);
                restore_dest_viewport(&mut pass, dest_physical);
                for command in &commands {
                    match command {
                        DrawCommand::Text { prepared, scissor } => {
                            self.text
                                .draw(&mut pass, prepared, *scissor, Some(&gpu_work));
                        }
                        DrawCommand::Icon {
                            first,
                            count,
                            scissor,
                        } => {
                            self.icons
                                .draw(&mut pass, *first, *count, *scissor, Some(&gpu_work));
                        }
                        _ => {}
                    }
                }
                drop(pass);
            }
        }
        dest.blit(
            encoder,
            target,
            blit_origin[0],
            blit_origin[1],
            viewport.physical_size,
            viewport.clear.then_some(clear),
            Some(&gpu_work),
            &mut dest_passes,
        );
        let encode = encode_started.elapsed();
        if !reused {
            self.quads.finish_frame();
            self.host_textures.trim();
        }
        for _ in 0..self.text.take_frame_gpu_allocations() {
            gpu_work.record_realloc();
        }
        self.last_gpu_work = Some(gpu_work.snapshot());
        self.last_gpu_timings = Some(GpuStageTimings {
            batch,
            gpu_upload,
            encode,
            submit: std::time::Duration::ZERO,
        });
        self.last_dest_pass_counts = Some(dest_passes);
        // Host-driven pixels (host textures, GPU slots) can change while the
        // scene stands still, so only fully scene-described frames may be
        // reused by the next paint.
        self.painted = commands
            .iter()
            .all(|command| {
                !matches!(
                    command,
                    DrawCommand::HostTexture(_) | DrawCommand::Custom { .. }
                )
            })
            .then_some(painted);
        if resolved.cacheable {
            self.prepared_batch = Some(PreparedBatch {
                key: painted,
                resources: resolved.resources,
                renderers: resolved.renderers,
                image_revision: self.image_revision,
                commands,
                max_group_depth,
                group_slots: group_slots_uniforms,
                glyph_then_quad,
            });
        }
        Ok(())
    }
}

/// Stroke/spinner cannot take a homography without a new mesh pipeline.
/// Planar 3D paints identity here so we do not squash like `scaleX(cos)`.
fn mesh_affine(affine: [f32; 6], persp: [f32; 2]) -> [f32; 6] {
    if persp[0].abs() > 1e-8 || persp[1].abs() > 1e-8 {
        clip::IDENTITY_AFFINE
    } else {
        affine
    }
}

/// Non-HostTexture custom renderers have no projective VS. Identity dest
/// when `(g,h)` is live; 2D affine still maps the AABB.
fn custom_paint_bounds(bounds: LogicalRect, affine: [f32; 6], persp: [f32; 2]) -> LogicalRect {
    if persp[0].abs() > 1e-8 || persp[1].abs() > 1e-8 {
        bounds
    } else {
        transformed_aabb(bounds, affine)
    }
}

fn clip_dests_for(
    kind: &ScenePrimitiveKind,
    clips: &[nana_ui_scene::ClipRegion],
    origin: [f32; 2],
) -> Vec<FragmentClip> {
    // Custom has no vertex clip so wrap every rotated parallelogram; built-ins wrap extras only.
    let keep_innermost = matches!(
        kind,
        ScenePrimitiveKind::Custom { node: custom, .. } if custom.renderer.as_ref() != "nana.host-texture"
    );
    if keep_innermost {
        let mut dest = clip::rotated_fragment_clips(clips, origin);
        dest.extend(clip::polygon_fragment_clips(clips, origin));
        dest
    } else if matches!(
        kind,
        ScenePrimitiveKind::Stroke { .. } | ScenePrimitiveKind::Spinner { .. }
    ) {
        mesh_extra_fragment_clips(clips, origin)
    } else {
        extra_fragment_clips(clips, origin)
    }
}

#[allow(clippy::too_many_arguments)]
fn wrap_drawn_with_clip_dests(
    commands: &mut Vec<DrawCommand>,
    start: usize,
    depth: &mut usize,
    slots: &mut u32,
    max_depth: &mut usize,
    uniforms: &mut Vec<GroupSlot>,
    clips: &[FragmentClip],
    scale: f32,
) -> bool {
    if clips.is_empty() || commands.len() == start {
        return false;
    }
    let drawn: Vec<_> = commands.drain(start..).collect();
    for clip in clips {
        let layer = *depth;
        let slot = *slots;
        *slots = slots.saturating_add(1);
        *depth = depth.saturating_add(1);
        *max_depth = (*max_depth).max(*depth);
        uniforms.push(GroupSlot::clip(clip.for_physical_pixels(scale)));
        commands.push(DrawCommand::PushGroup { layer, slot });
    }
    commands.extend(drawn);
    for _ in clips {
        *depth = depth.saturating_sub(1);
        commands.push(DrawCommand::PopGroup);
    }
    true
}

#[expect(
    clippy::too_many_arguments,
    reason = "Explicit fields of the host or GPU projection contract"
)]
///
/// Reports whether a `PushGroup` or `PopGroup` is now in the list; no batch may
/// span one. A group that opened and closed with nothing inside takes its own
/// `PushGroup` back out, and since nothing was emitted while it was open, no
/// batch can name a command at or past it.
fn sync_opacity_groups(
    commands: &mut Vec<DrawCommand>,
    stack: &mut Vec<nana_ui_scene::OpacityGroup>,
    depth: &mut usize,
    slots: &mut u32,
    max_depth: &mut usize,
    uniforms: &mut Vec<GroupSlot>,
    needed: Vec<nana_ui_scene::OpacityGroup>,
    scene: &nana_ui_scene::UiScene,
    origin: [f32; 2],
    scale: f32,
) -> bool {
    let common = stack
        .iter()
        .zip(needed.iter())
        .take_while(|(open, want)| open.node == want.node)
        .count();
    let mut emitted = false;
    while stack.len() > common {
        emitted |= pop_opacity_group(commands, stack, depth, slots, uniforms);
    }
    for group in needed.into_iter().skip(common) {
        let layer = *depth;
        let slot = *slots;
        *slots = slots.saturating_add(1);
        *depth = depth.saturating_add(1);
        *max_depth = (*max_depth).max(*depth);
        uniforms.push(dest_group_slot(&group, scene, origin, scale));
        commands.push(DrawCommand::PushGroup { layer, slot });
        stack.push(group);
        emitted = true;
    }
    emitted
}

fn dest_group_slot(
    group: &nana_ui_scene::OpacityGroup,
    scene: &nana_ui_scene::UiScene,
    origin: [f32; 2],
    scale: f32,
) -> GroupSlot {
    let filter = group.filter;
    let pad = filter.dest_extent_pad();
    let clip = if pad > 0.0 {
        scene
            .draw_node_bounds(group.node)
            .map(|bounds| {
                clip::FragmentClip {
                    rect: [
                        bounds.x - origin[0] - pad,
                        bounds.y - origin[1] - pad,
                        (bounds.width + pad * 2.0).max(0.0),
                        (bounds.height + pad * 2.0).max(0.0),
                    ],
                    inv_abcd: [1.0, 0.0, 0.0, 1.0],
                    inv_ef: [0.0, 0.0],
                    corner_radius: 0.0,
                    polygon_count: 0,
                    polygon: [[0.0; 2]; 8],
                }
                .for_physical_pixels(scale)
            })
            .unwrap_or(clip::FragmentClip::PASS)
    } else {
        clip::FragmentClip::PASS
    };
    let physical_blur = if scale.is_finite() && scale > 0.0 {
        filter.blur_radius * scale
    } else {
        filter.blur_radius
    };
    let mut slot = GroupSlot::dest(
        group.opacity * filter.opacity,
        [filter.brightness, filter.saturate, filter.contrast],
        filter.hue_rotate_deg,
        physical_blur,
        group.mix_blend.gpu_index(),
        clip,
    );
    slot.filter_invert = filter.invert;
    if let Some(shadow) = filter.drop_shadow {
        let (ox, oy, blur) = if scale.is_finite() && scale > 0.0 {
            (
                shadow.offset_x * scale,
                shadow.offset_y * scale,
                shadow.blur_radius * scale,
            )
        } else {
            (shadow.offset_x, shadow.offset_y, shadow.blur_radius)
        };
        slot.drop_shadow_offset = [ox, oy];
        slot.drop_shadow_blur = blur;
        slot.drop_shadow_color = pack_linear(shadow.color);
    }
    slot
}

fn pop_opacity_group(
    commands: &mut Vec<DrawCommand>,
    stack: &mut Vec<nana_ui_scene::OpacityGroup>,
    depth: &mut usize,
    slots: &mut u32,
    uniforms: &mut Vec<GroupSlot>,
) -> bool {
    stack.pop();
    *depth = depth.saturating_sub(1);
    if matches!(commands.last(), Some(DrawCommand::PushGroup { .. })) {
        commands.pop();
        uniforms.pop();
        *slots = slots.saturating_sub(1);
        return false;
    }
    commands.push(DrawCommand::PopGroup);
    true
}

/// A draw command later primitives may still be folded into.
struct OpenBatch {
    /// Position of the command in the display list.
    command: usize,
    /// Union of the dest-pixel bounds of everything already in it.
    painted: PhysicalRect,
}

/// Display-list batching state.
///
/// A primitive may join a batch that is not the newest one, which draws it
/// earlier than its document position. That is sound exactly when it covers no
/// pixel of anything emitted after that batch — and everything emitted after it
/// is still open, because every command whose place in the list is part of the
/// GPU contract (`HostTexture`, `Custom`, `Backdrop`, `PushGroup`, `PopGroup`)
/// closes all of them. Overlapping primitives therefore keep painter's-algorithm
/// order to the pixel, and GPU content never moves.
#[derive(Default)]
struct Batching {
    open: Vec<OpenBatch>,
    saw_glyph: bool,
    /// A quad or mesh followed a glyph in **document order**. Read from the
    /// emit order rather than from the merged list, because folding a quad into
    /// an earlier batch can leave glyphs at the end and that must not turn a
    /// single-sampled frame into a 4x MSAA one.
    glyph_then_quad: bool,
}

impl Batching {
    /// Close every batch. Required around anything that fixes a command's
    /// position or moves commands within the list.
    fn close_all(&mut self) {
        self.open.clear();
    }

    fn note_glyph(&mut self) {
        self.saw_glyph = true;
    }

    fn note_geometry(&mut self) {
        self.glyph_then_quad |= self.saw_glyph;
    }

    /// Position in `commands` of the batch this primitive may join. The
    /// candidate is identified by `extendable`, which reads the command itself
    /// — including its scissor — so a batch stores no copy of either.
    fn target(
        &mut self,
        commands: &[DrawCommand],
        painted: PhysicalRect,
        extendable: impl Fn(&DrawCommand) -> bool,
    ) -> Option<usize> {
        let position = self
            .open
            .iter()
            .rposition(|batch| extendable(&commands[batch.command]))?;
        if self.open[position + 1..]
            .iter()
            .any(|later| overlaps_physical(painted, later.painted))
        {
            return None;
        }
        let batch = &mut self.open[position];
        batch.painted = union_physical(batch.painted, painted);
        Some(batch.command)
    }

    fn open(&mut self, command: usize, painted: PhysicalRect) {
        self.open.push(OpenBatch { command, painted });
    }
}

/// Dest-pixel bounds a primitive can paint, used only to decide whether folding
/// it into an earlier batch would cross something. It must never under-cover
/// what the GPU paints, so callers pass the outset their paint adds (shadow,
/// outline, stroke half-width) before the transform.
fn painted_bounds(
    local: LogicalRect,
    outset: f32,
    affine: [f32; 6],
    persp: [f32; 2],
    scale: f32,
    scissor: PhysicalRect,
) -> PhysicalRect {
    let padded = LogicalRect::from_xywh(
        local.x - outset,
        local.y - outset,
        local.width + outset * 2.0,
        local.height + outset * 2.0,
    );
    physical_bounds(
        transformed_aabb_projective(padded, affine, persp),
        scale,
        scissor,
    )
}

/// AABB of a polyline in its own local space. `None` when it has no finite
/// point, which is also when the mesh pipeline refuses it.
fn polyline_bounds(points: &[[f32; 2]]) -> Option<LogicalRect> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for point in points {
        if !point[0].is_finite() || !point[1].is_finite() {
            return None;
        }
        min = [min[0].min(point[0]), min[1].min(point[1])];
        max = [max[0].max(point[0]), max[1].max(point[1])];
    }
    (min[0] <= max[0])
        .then(|| LogicalRect::from_xywh(min[0], min[1], max[0] - min[0], max[1] - min[1]))
}

/// [`painted_bounds`] for a quad: its box grown by the largest outer shadow and
/// the outline. The antialiased edge needs no margin of its own — the box is
/// rounded outwards to whole pixels when it is put in dest space. A filtered
/// surface has no rectangular reach, so it takes the whole dest and can then
/// neither join a batch nor be jumped over.
#[allow(clippy::too_many_arguments)]
fn quad_painted_bounds(
    local: LogicalRect,
    shadow: Option<&nana_ui_runtime::ComponentElevation>,
    surface: &nana_ui_scene::QuadSurfacePaint,
    affine: [f32; 6],
    persp: [f32; 2],
    scale: f32,
    scissor: PhysicalRect,
    dest_physical: [u32; 2],
) -> PhysicalRect {
    if surface.filter.is_some() || surface.backdrop_filter.is_some() {
        return PhysicalRect {
            x: 0,
            y: 0,
            width: dest_physical[0],
            height: dest_physical[1],
        };
    }
    let outset = shadow
        .into_iter()
        .chain(surface.extra_shadows.iter())
        .filter(|shadow| !shadow.inset)
        .map(|shadow| {
            shadow.offset_x.abs().max(shadow.offset_y.abs())
                + shadow.blur_radius * 3.0
                + shadow.spread_radius.max(0.0)
        })
        .fold(surface.outline_width.max(0.0) + 1.0, f32::max);
    painted_bounds(local, outset, affine, persp, scale, scissor)
}

/// Extend an open icon batch when the new slot shares its atlas, scissor and
/// vertex adjacency.
fn push_icon(
    commands: &mut Vec<DrawCommand>,
    batching: &mut Batching,
    icons: &IconPipeline,
    prepared: PreparedIcon,
    scissor: PhysicalRect,
    painted: PhysicalRect,
) {
    batching.note_glyph();
    let slot = prepared.index;
    if let Some(target) = batching.target(commands, painted, |command| {
        matches!(command, DrawCommand::Icon { first, count, scissor: open }
            if *open == scissor && icons.can_extend_run(*first, *count, slot))
    }) {
        let DrawCommand::Icon { count, .. } = &mut commands[target] else {
            unreachable!("an icon batch names an icon command")
        };
        *count += 1;
        return;
    }
    batching.open(commands.len(), painted);
    commands.push(DrawCommand::Icon {
        first: slot,
        count: 1,
        scissor,
    });
}

/// Fold a prepared text into an open text run.
fn push_text_run(
    commands: &mut Vec<DrawCommand>,
    batching: &mut Batching,
    text: &mut TextPipeline,
    prepared: PreparedText,
    scissor: PhysicalRect,
    painted: PhysicalRect,
) {
    batching.note_glyph();
    if let Some(target) = batching.target(commands, painted, |command| {
        matches!(command, DrawCommand::Text { prepared: previous, scissor: open }
            if *open == scissor && text.can_merge_runs(previous, &prepared))
    }) {
        let DrawCommand::Text {
            prepared: previous, ..
        } = &commands[target]
        else {
            unreachable!("a text batch names a text command")
        };
        text.merge_runs(previous, &prepared);
        return;
    }
    batching.open(commands.len(), painted);
    commands.push(DrawCommand::Text { prepared, scissor });
}

fn push_quad(
    commands: &mut Vec<DrawCommand>,
    batching: &mut Batching,
    index: u32,
    scissor: PhysicalRect,
    painted: PhysicalRect,
) {
    batching.note_geometry();
    if let Some(target) = batching.target(commands, painted, |command| {
        matches!(command, DrawCommand::Quads { range, scissor: open }
            if *open == scissor && range.end == index)
    }) {
        let DrawCommand::Quads { range, .. } = &mut commands[target] else {
            unreachable!("a quad batch names a quad command")
        };
        range.end = index + 1;
        return;
    }
    batching.open(commands.len(), painted);
    commands.push(DrawCommand::Quads {
        range: index..index + 1,
        scissor,
    });
}

fn push_mesh_draw(
    commands: &mut Vec<DrawCommand>,
    batching: &mut Batching,
    range: MeshRange,
    scissor: PhysicalRect,
    painted: PhysicalRect,
) {
    batching.note_geometry();
    if let Some(target) = batching.target(commands, painted, |command| {
        matches!(command, DrawCommand::Mesh { range: previous, scissor: open }
            if *open == scissor
                && previous.first_instance + previous.instance_count == range.first_instance)
    }) {
        let DrawCommand::Mesh {
            range: previous, ..
        } = &mut commands[target]
        else {
            unreachable!("a mesh batch names a mesh command")
        };
        previous.instance_count += range.instance_count;
        return;
    }
    batching.open(commands.len(), painted);
    commands.push(DrawCommand::Mesh { range, scissor });
}

fn text_shadow_draw_offsets(shadow: nana_ui_core::TextShadowSpec) -> Vec<(f32, f32, f32)> {
    let mut out = vec![(0.0, 0.0, 1.0)];
    let blur = shadow.blur_radius.max(0.0);
    if blur > 0.5 {
        let step = blur * 0.35;
        out.push((step, 0.0, 0.55));
        out.push((-step, 0.0, 0.55));
        out.push((0.0, step, 0.55));
        out.push((0.0, -step, 0.55));
    }
    out
}

struct EncodeOrdered<'a> {
    quads: &'a QuadPipeline,
    meshes: &'a MeshPipeline,
    icons: &'a IconPipeline,
    text: &'a TextPipeline,
    host_textures: &'a HostTexturePipeline,
    backdrop: &'a mut BackdropPipeline,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    gpu_work: &'a GpuWorkSink,
}

struct GroupFrame {
    layer: usize,
    slot: u32,
}

fn encode_ordered(
    pipelines: &mut EncodeOrdered<'_>,
    encoder: &mut wgpu::CommandEncoder,
    dest: &DestTarget,
    dest_physical: [u32; 2],
    initial_load: wgpu::LoadOp<wgpu::Color>,
    commands: &[DrawCommand],
    dest_passes: &mut DestPassCounts,
) {
    const SAMPLE_COUNT: u32 = 1;
    let mut index = 0;
    let mut dest_load = initial_load;
    let mut stack: Vec<GroupFrame> = Vec::new();
    let mut layer_ready = Vec::new();
    while index < commands.len() {
        match &commands[index] {
            DrawCommand::PushGroup { layer, slot } => {
                if *layer >= layer_ready.len() {
                    layer_ready.resize(*layer + 1, false);
                }
                layer_ready[*layer] = false;
                stack.push(GroupFrame {
                    layer: *layer,
                    slot: *slot,
                });
                index += 1;
            }
            DrawCommand::PopGroup => {
                let Some(frame) = stack.pop() else {
                    index += 1;
                    continue;
                };
                let mut pass = match stack.last() {
                    Some(parent) => {
                        let load = group_layer_load(&mut layer_ready, parent.layer);
                        dest.begin_group_pass(encoder, parent.layer, load, dest_passes)
                    }
                    None => {
                        let pass = dest.begin_color_pass(encoder, dest_load, dest_passes);
                        dest_load = wgpu::LoadOp::Load;
                        pass
                    }
                };
                restore_dest_viewport(&mut pass, dest_physical);
                dest.composite_group(&mut pass, frame.layer, frame.slot, Some(pipelines.gpu_work));
                drop(pass);
                index += 1;
            }
            _ => {
                if let DrawCommand::Backdrop {
                    index: backdrop_index,
                } = &commands[index]
                {
                    let Some(request) = pipelines.backdrop.request(*backdrop_index).copied() else {
                        index += 1;
                        continue;
                    };
                    ensure_backdrop_target_ready(
                        dest,
                        encoder,
                        dest_physical,
                        &mut dest_load,
                        dest_passes,
                    );
                    pipelines.backdrop.encode(
                        pipelines.device,
                        encoder,
                        dest.color_view(),
                        dest_physical,
                        dest.color_view(),
                        &request,
                        pipelines.quads.paint_buffer(),
                        Some(pipelines.gpu_work),
                        &mut dest_passes.backdrop,
                    );
                    dest_load = wgpu::LoadOp::Load;
                    index += 1;
                    continue;
                }
                let mut pass = match stack.last() {
                    Some(frame) => {
                        let load = group_layer_load(&mut layer_ready, frame.layer);
                        dest.begin_group_pass(encoder, frame.layer, load, dest_passes)
                    }
                    None => {
                        let pass = dest.begin_color_pass(encoder, dest_load, dest_passes);
                        dest_load = wgpu::LoadOp::Load;
                        pass
                    }
                };
                restore_dest_viewport(&mut pass, dest_physical);
                while index < commands.len() {
                    match &commands[index] {
                        DrawCommand::PushGroup { .. } | DrawCommand::PopGroup => break,
                        DrawCommand::Backdrop { .. } => break,
                        DrawCommand::Quads { range, scissor } => {
                            pipelines.quads.draw(
                                &mut pass,
                                range.clone(),
                                *scissor,
                                SAMPLE_COUNT,
                                Some(pipelines.gpu_work),
                            );
                        }
                        DrawCommand::Mesh { range, scissor } => {
                            pipelines.meshes.draw(
                                &mut pass,
                                range,
                                *scissor,
                                SAMPLE_COUNT,
                                Some(pipelines.gpu_work),
                            );
                        }
                        DrawCommand::Text { prepared, scissor } => {
                            pipelines.text.draw(
                                &mut pass,
                                prepared,
                                *scissor,
                                Some(pipelines.gpu_work),
                            );
                        }
                        DrawCommand::Icon {
                            first,
                            count,
                            scissor,
                        } => {
                            pipelines.icons.draw(
                                &mut pass,
                                *first,
                                *count,
                                *scissor,
                                Some(pipelines.gpu_work),
                            );
                        }
                        DrawCommand::HostTexture(prepared) => {
                            pipelines.host_textures.draw(
                                prepared,
                                &mut pass,
                                dest_physical,
                                Some(pipelines.gpu_work),
                            );
                            restore_dest_viewport(&mut pass, dest_physical);
                        }
                        DrawCommand::Custom {
                            node,
                            renderer,
                            bounds,
                            clip,
                        } if bounds.width > 0 && bounds.height > 0 => {
                            if !node.custom.dedicated_pass {
                                let encoded = draw_custom_run(
                                    commands,
                                    index,
                                    renderer,
                                    &mut pass,
                                    dest_physical,
                                    pipelines.device,
                                    pipelines.queue,
                                    pipelines.gpu_work,
                                );
                                if encoded > 0 {
                                    restore_dest_viewport(&mut pass, dest_physical);
                                    index += encoded;
                                    continue;
                                }
                                if renderer.draw_in_pass(
                                    node,
                                    &mut pass,
                                    SceneGpuPassContext {
                                        device: pipelines.device,
                                        queue: pipelines.queue,
                                        bounds: *bounds,
                                        clip: *clip,
                                        dest_size: dest_physical,
                                        gpu_work: Some(pipelines.gpu_work),
                                    },
                                ) {
                                    restore_dest_viewport(&mut pass, dest_physical);
                                    index += 1;
                                    continue;
                                }
                            }
                            break;
                        }
                        DrawCommand::Custom { .. } => {}
                    }
                    index += 1;
                }
                drop(pass);
                if let Some(DrawCommand::Custom {
                    node,
                    renderer,
                    bounds,
                    clip,
                }) = commands.get(index)
                {
                    if bounds.width > 0 && bounds.height > 0 {
                        let target = match stack.last() {
                            Some(frame) => dest.group_view(frame.layer),
                            None => dest.color_view(),
                        };
                        renderer.render(
                            node,
                            SceneGpuRenderContext {
                                device: pipelines.device,
                                queue: pipelines.queue,
                                encoder,
                                target,
                                bounds: *bounds,
                                clip: *clip,
                                dest_size: dest_physical,
                                gpu_work: Some(pipelines.gpu_work),
                            },
                        );
                    }
                    index += 1;
                }
            }
        }
    }
}

/// Offer the renderer the run of `Custom` commands starting at `start` that
/// share its renderer instance and want no dedicated pass.
///
/// The run is a contiguous slice of the display list, so any other command
/// between two custom nodes ends it and document order is untouched. Returns
/// how many commands the renderer encoded; `0` means it declined and the caller
/// falls back to the single-node path.
#[allow(clippy::too_many_arguments)]
fn draw_custom_run(
    commands: &[DrawCommand],
    start: usize,
    renderer: &Arc<dyn SceneGpuRenderer>,
    pass: &mut wgpu::RenderPass<'_>,
    dest_physical: [u32; 2],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu_work: &GpuWorkSink,
) -> usize {
    let capacity = renderer.batch_capacity();
    if capacity <= 1 {
        return 0;
    }
    let mut run: Vec<SceneGpuBatchNode<'_>> = Vec::new();
    for command in &commands[start..] {
        let DrawCommand::Custom {
            node,
            renderer: candidate,
            bounds,
            clip,
        } = command
        else {
            break;
        };
        if !Arc::ptr_eq(renderer, candidate)
            || node.custom.dedicated_pass
            || bounds.width == 0
            || bounds.height == 0
            || run.len() == capacity
        {
            break;
        }
        run.push(SceneGpuBatchNode {
            node,
            bounds: *bounds,
            clip: *clip,
        });
    }
    if run.len() < 2 {
        return 0;
    }
    let encoded = renderer.draw_batch_in_pass(
        &run,
        pass,
        SceneGpuBatchPassContext {
            device,
            queue,
            dest_size: dest_physical,
            gpu_work: Some(gpu_work),
        },
    );
    encoded.min(run.len())
}

fn group_layer_load(ready: &mut Vec<bool>, layer: usize) -> wgpu::LoadOp<wgpu::Color> {
    if layer >= ready.len() {
        ready.resize(layer + 1, false);
    }
    if ready[layer] {
        wgpu::LoadOp::Load
    } else {
        ready[layer] = true;
        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
    }
}

fn restore_dest_viewport(pass: &mut wgpu::RenderPass<'_>, dest_physical: [u32; 2]) {
    pass.set_viewport(
        0.0,
        0.0,
        dest_physical[0].max(1) as f32,
        dest_physical[1].max(1) as f32,
        0.0,
        1.0,
    );
}

fn ensure_backdrop_target_ready(
    dest: &DestTarget,
    encoder: &mut wgpu::CommandEncoder,
    dest_physical: [u32; 2],
    dest_load: &mut wgpu::LoadOp<wgpu::Color>,
    dest_passes: &mut DestPassCounts,
) {
    if matches!(*dest_load, wgpu::LoadOp::Clear(_)) {
        let mut pass = dest.begin_color_pass(encoder, *dest_load, dest_passes);
        restore_dest_viewport(&mut pass, dest_physical);
        drop(pass);
        *dest_load = wgpu::LoadOp::Load;
    }
}

#[cfg(test)]
mod tests;
