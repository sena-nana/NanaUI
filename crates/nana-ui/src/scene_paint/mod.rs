//! Nana-owned WGPU painter for [`UiScene`].
//!
//! This is the product Scene backend. It paints [`UiScene`] through the host GPU.
//! The host owns Device/Queue/encoder. HostTexture is sampled in document order
//! inside the current dest (or opacity-group) pass; groups do not open a pass
//! per HostTexture slot.

mod backdrop;
mod clip;
mod color;
mod dest;
mod host_texture;
mod icon;
mod image_url;
mod mesh;
mod quad;
mod text;
mod validate;

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Instant,
};

use nana_ui_core::GpuWorkObservation;
use nana_ui_scene::{RenderOperation, ScenePrimitiveKind, UiScene};

use crate::{
    HostTextureRegistry, PhysicalRect,
    gpu_work::{GpuStageTimings, GpuWorkSink},
    scene_gpu::{
        SceneGpuNode, SceneGpuPassContext, SceneGpuPrepareContext, SceneGpuRenderContext,
        SceneGpuRenderer, SceneGpuRendererRegistry,
    },
};

/// How many distinct scene instances keep a validated operation stream.
const VALIDATED_SCENE_CACHE: usize = 8;

pub use image_url::set_background_image_url_base;
pub(crate) use validate::validate_scene;
pub use validate::{HostTextureSceneResolver, ScenePaintError};

use backdrop::BackdropPipeline;
use clip::{
    FragmentClip, LogicalRect, extra_fragment_clips, fragment_clip, intersect_clips, local_rect,
    paint_origin, paint_transform, physical_bounds, physical_scissor, transformed_aabb,
    transformed_aabb_projective,
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
    /// Validated operation streams keyed by scene instance. An unchanged
    /// scene revalidates nothing: no frame-graph rebuild, no per-primitive
    /// validation scan, no label allocation. Node-changing `apply_delta` and
    /// Clone both refresh the instance, so in-place mutation cannot leave a
    /// stale stream.
    validated_scenes: HashMap<u64, Arc<[RenderOperation]>>,
    validated_order: VecDeque<u64>,
    /// Last fully scene-described dest; host textures / GPU slots skip reuse.
    painted: Option<PaintedDest>,
}

#[derive(Clone, Copy, PartialEq)]
struct PaintedDest {
    instance: u64,
    viewport: ScenePaintViewport,
    size: [u32; 2],
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
        prepared: PreparedIcon,
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
            validated_scenes: HashMap::new(),
            validated_order: VecDeque::new(),
            painted: None,
        }
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
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

    pub fn paint(
        &mut self,
        scene: &UiScene,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: ScenePaintViewport,
        host_textures: Option<&HostTextureRegistry>,
        gpu_renderers: Option<&SceneGpuRendererRegistry>,
    ) -> Result<(), ScenePaintError> {
        let instance = scene.instance_id();
        let operations = match self.validated_scenes.get(&instance) {
            Some(cached) => Arc::clone(cached),
            None => {
                let operations = validate_scene(scene, host_textures, gpu_renderers)?;
                self.validated_scenes
                    .insert(instance, Arc::clone(&operations));
                self.validated_order.push_back(instance);
                while self.validated_scenes.len() > VALIDATED_SCENE_CACHE {
                    let Some(oldest) = self.validated_order.pop_front() else {
                        break;
                    };
                    self.validated_scenes.remove(&oldest);
                }
                operations
            }
        };
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

        let batch_started = Instant::now();
        self.quads.begin_frame();
        self.meshes.begin_frame();
        self.icons.begin_frame(&self.queue, dest_physical);
        self.text.begin_frame(&self.queue, dest_physical);
        self.backdrop.begin_frame();

        let mut commands = Vec::new();
        let mut group_stack: Vec<nana_ui_scene::OpacityGroup> = Vec::new();
        let mut group_depth = 0usize;
        let mut group_slots = 0u32;
        let mut max_group_depth = 0usize;
        let mut group_slots_uniforms = Vec::new();
        for operation in operations.iter() {
            let id = match operation {
                RenderOperation::PrepareExternal(_) => continue,
                RenderOperation::Draw(id) | RenderOperation::InvokeCustom(id) => *id,
            };
            let Some(primitive) = scene.primitive(id) else {
                continue;
            };
            sync_opacity_groups(
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
            );
            let Some(clip) = intersect_clips(viewport_clip, &primitive.clips, origin) else {
                continue;
            };
            let Some(scissor) = physical_scissor(clip, scale, dest_physical) else {
                continue;
            };
            let frag_clip = fragment_clip(&primitive.clips, origin);
            let (affine, persp) =
                paint_transform(primitive.transform.0, primitive.transform.1, origin);
            let bounds = local_rect(primitive.bounds);
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
                        if let Some(filter) = surface.backdrop_filter.filter(|f| f.is_active()) {
                            let world_bounds = if clip::is_translation_projective(affine, persp) {
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
                            commands.push(DrawCommand::Backdrop { index: bidx });
                        }
                        for quad_index in index..self.quads.pending_len() {
                            push_quad(&mut commands, quad_index, scissor);
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
                            if let Some(filter) = surface.backdrop_filter.filter(|f| f.is_active())
                            {
                                let world_bounds = if clip::is_translation_projective(affine, persp)
                                {
                                    item_bounds
                                } else {
                                    transformed_aabb_projective(item_bounds, affine, persp)
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
                                commands.push(DrawCommand::Backdrop { index: bidx });
                            }
                            for quad_index in index..self.quads.pending_len() {
                                push_quad(&mut commands, quad_index, scissor);
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
                } => {
                    let mut push_text =
                        |extra_offset: [f32; 2], color_override: Option<[f32; 4]>| {
                            self.text.prepare(
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
                                affine,
                                persp,
                                frag_clip,
                                primitive.opacity,
                                extra_offset,
                            )
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
                            if let Some(prepared) = push_text(
                                [shadow.offset_x + dx, shadow.offset_y + dy],
                                Some(scaled),
                            ) {
                                commands.push(DrawCommand::Text { prepared, scissor });
                            }
                        }
                    }
                    if let Some(prepared) = push_text([0.0, 0.0], None) {
                        commands.push(DrawCommand::Text { prepared, scissor });
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
                        commands.push(DrawCommand::Icon { prepared, scissor });
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
                        push_mesh_draw(&mut commands, range, scissor);
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
                    let (dash, dash_offset, colors) = match pattern.as_deref() {
                        Some(pattern) => (
                            pattern.dash.as_slice(),
                            pattern.dash_offset,
                            pattern.colors.as_slice(),
                        ),
                        None => ([].as_slice(), 0.0, [].as_slice()),
                    };
                    if let Some(range) = self.meshes.push_stroke(
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
                    ) {
                        push_mesh_draw(&mut commands, range, scissor);
                    }
                }
                ScenePrimitiveKind::Custom(custom) => {
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
                                    let radius =
                                        corner_radius.iter().copied().fold(0.0f32, f32::max);
                                    Some((local_rect(quad.bounds), radius))
                                }
                                _ => None,
                            })
                            .unwrap_or((bounds, 0.0));
                        commands.push(DrawCommand::HostTexture(self.host_textures.prepare(
                            &self.device,
                            &self.queue,
                            binding,
                            primitive.id.node.get(),
                            primitive.id.slot,
                            LogicalRect::from_xywh(dest.x, dest.y, dest.width, dest.height),
                            affine,
                            persp,
                            scissor,
                            primitive.opacity,
                            corner_radius,
                            rounded_clip,
                            frag_clip,
                            dest_physical,
                            scale,
                            Some(&gpu_work),
                        )));
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
                                gpu_work: Some(&gpu_work),
                            },
                        );
                        commands.push(DrawCommand::Custom {
                            node,
                            renderer,
                            bounds: physical_bounds(custom_bounds, scale, scissor),
                            clip: scissor,
                        });
                    }
                }
            }
            wrap_drawn_with_clip_dests(
                &mut commands,
                command_start,
                &mut group_depth,
                &mut group_slots,
                &mut max_group_depth,
                &mut group_slots_uniforms,
                &clip_dests_for(&primitive.kind, &primitive.clips, origin),
                scale,
            );
        }
        sync_opacity_groups(
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
        );
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
        self.icons.upload(&self.device, &self.queue);
        self.backdrop
            .upload(&self.device, &self.queue, dest_physical, Some(&gpu_work));
        let gpu_upload = upload_started.elapsed();

        let encode_started = Instant::now();
        let gpu_interleaved = commands.iter().any(|command| {
            matches!(
                command,
                DrawCommand::HostTexture(_)
                    | DrawCommand::Custom { .. }
                    | DrawCommand::PushGroup { .. }
                    | DrawCommand::Backdrop { .. }
            )
        }) || self.backdrop.needs_backdrop();
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
                        DrawCommand::Icon { prepared, scissor } => {
                            self.icons
                                .draw(&mut pass, prepared, *scissor, Some(&gpu_work));
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
        self.host_textures.trim();
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

fn push_mesh_draw(commands: &mut Vec<DrawCommand>, range: MeshRange, scissor: PhysicalRect) {
    if let Some(DrawCommand::Mesh {
        range: previous,
        scissor: previous_scissor,
    }) = commands.last_mut()
        && *previous_scissor == scissor
        && previous.first_instance + previous.instance_count == range.first_instance
    {
        previous.instance_count += range.instance_count;
        return;
    }
    commands.push(DrawCommand::Mesh { range, scissor });
}

fn clip_dests_for(
    kind: &ScenePrimitiveKind,
    clips: &[nana_ui_scene::ClipRegion],
    origin: [f32; 2],
) -> Vec<FragmentClip> {
    // Custom has no vertex clip so wrap every rotated parallelogram; built-ins wrap extras only.
    let keep_innermost = matches!(
        kind,
        ScenePrimitiveKind::Custom(custom) if custom.renderer.as_ref() != "nana.host-texture"
    );
    if keep_innermost {
        let mut dest = clip::rotated_fragment_clips(clips, origin);
        dest.extend(clip::polygon_fragment_clips(clips, origin));
        dest
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
) {
    if clips.is_empty() || commands.len() == start {
        return;
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
}

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
) {
    let common = stack
        .iter()
        .zip(needed.iter())
        .take_while(|(open, want)| open.node == want.node)
        .count();
    while stack.len() > common {
        pop_opacity_group(commands, stack, depth, slots, uniforms);
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
    }
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
            .node_bounds(group.node)
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
) {
    stack.pop();
    *depth = depth.saturating_sub(1);
    if matches!(commands.last(), Some(DrawCommand::PushGroup { .. })) {
        commands.pop();
        uniforms.pop();
        *slots = slots.saturating_sub(1);
        return;
    }
    commands.push(DrawCommand::PopGroup);
}

fn push_quad(commands: &mut Vec<DrawCommand>, index: u32, scissor: PhysicalRect) {
    if let Some(DrawCommand::Quads {
        range,
        scissor: last,
    }) = commands.last_mut()
        && *last == scissor
        && range.end == index
    {
        range.end = index + 1;
        return;
    }
    commands.push(DrawCommand::Quads {
        range: index..index + 1,
        scissor,
    });
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
                        DrawCommand::Icon { prepared, scissor } => {
                            pipelines.icons.draw(
                                &mut pass,
                                prepared,
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
                            if !node.custom.dedicated_pass
                                && renderer.draw_in_pass(
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
                                )
                            {
                                restore_dest_viewport(&mut pass, dest_physical);
                                index += 1;
                                continue;
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
mod tests {
    use std::sync::Arc;

    use nana_ui_core::{ButtonKind, LengthSpec, OverflowSpec, PaintTransform, SemanticColorRole};
    use nana_ui_runtime::{
        AppContext, Button as RuntimeButton, ComponentGeometry, ComputedStyle, CustomRenderNode,
        DocumentId, ExtractedNode, GpuTextureView, LayoutBox, MutationQueue, NodeKind, NodeStyle,
        StableNodeId, StandardVisual, TextContent,
    };
    use nana_ui_scene::{
        AffineTransform, ClipRegion, ScenePrimitiveKind, SceneRect, StrokeCap, StrokePattern,
        UiScene,
    };

    use super::*;
    use crate::HostTextureRegistry;

    #[derive(Debug)]
    struct FillClipRenderer {
        pipeline: wgpu::RenderPipeline,
    }

    impl FillClipRenderer {
        fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("nana-ui.test.fill.shader"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                    r#"
                    @vertex
                    fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
                        var positions = array<vec2<f32>, 3>(
                            vec2<f32>(-1.0, -1.0),
                            vec2<f32>(3.0, -1.0),
                            vec2<f32>(-1.0, 3.0),
                        );
                        return vec4<f32>(positions[index], 0.0, 1.0);
                    }

                    @fragment
                    fn fs_main() -> @location(0) vec4<f32> {
                        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
                    }
                    "#,
                )),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("nana-ui.test.fill.pipeline"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("nana-ui.test.fill.pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
            Self { pipeline }
        }
    }

    impl SceneGpuRenderer for FillClipRenderer {
        fn prepare(&self, _node: &SceneGpuNode, _context: SceneGpuPrepareContext<'_>) {}

        fn render(&self, node: &SceneGpuNode, context: SceneGpuRenderContext<'_>) {
            if context.bounds.width == 0 || context.bounds.height == 0 {
                return;
            }
            let mut pass = context
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("nana-ui.test.fill"),
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
                &mut pass,
                SceneGpuPassContext {
                    device: context.device,
                    queue: context.queue,
                    bounds: context.bounds,
                    clip: context.clip,
                    dest_size: [
                        context.clip.x.saturating_add(context.clip.width).max(1),
                        context.clip.y.saturating_add(context.clip.height).max(1),
                    ],
                    gpu_work: context.gpu_work,
                },
            );
        }

        fn draw_in_pass(
            &self,
            _node: &SceneGpuNode,
            pass: &mut wgpu::RenderPass<'_>,
            context: SceneGpuPassContext<'_>,
        ) -> bool {
            if context.clip.width == 0 || context.clip.height == 0 {
                return false;
            }
            pass.set_pipeline(&self.pipeline);
            pass.set_scissor_rect(
                context.clip.x,
                context.clip.y,
                context.clip.width,
                context.clip.height,
            );
            pass.draw(0..3, 0..1);
            true
        }
    }

    #[test]
    fn empty_scene_validates() {
        let operations = validate_scene(&UiScene::new(), None, None).unwrap();
        assert!(operations.is_empty());
    }

    #[test]
    fn validate_scene_rejects_unregistered_host_texture() {
        let mut context = AppContext::new();
        let button = context
            .create_component(DocumentId::new(1).unwrap(), RuntimeButton::new("Preview"))
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
        layout.set_custom_render(
            button.stable_id(),
            Some(CustomRenderNode::new("nana.host-texture", "preview", 1)),
        );
        context.commit_mutations(layout).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        let mut scene = UiScene::new();
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
    fn paint_records_gpu_work_only_after_encode_and_submit() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        assert!(painter.last_gpu_work().is_none());

        let skipped = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [0, 0],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let target = test_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui gpu work skip"),
        });
        painter
            .paint(&UiScene::new(), &mut encoder, &target, skipped, None, None)
            .unwrap();
        assert!(painter.last_gpu_work().is_none());
        assert!(painter.last_gpu_timings().is_none());

        let (scene, registry) = hosted_preview_scene(&device);
        let viewport = ScenePaintViewport {
            logical_size: [128.0, 96.0],
            physical_size: [128, 96],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.1, 0.1, 0.1, 1.0],
            clear: true,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui gpu work encode"),
        });
        painter
            .paint(
                &scene,
                &mut encoder,
                &target,
                viewport,
                Some(&registry),
                None,
            )
            .unwrap();
        let observed = painter
            .last_gpu_work()
            .expect("encoded frame records GPU work");
        assert!(
            observed.gpu_upload_bytes > 0,
            "host texture / quad write_buffer must be counted"
        );
        assert!(observed.draw_calls > 0);
        assert!(observed.draw_batches > 0);
        let timings = painter
            .last_gpu_timings()
            .expect("encoded frame times GPU stages");
        assert!(timings.submit.is_zero());
        let submit_started = std::time::Instant::now();
        queue.submit([encoder.finish()]);
        painter.record_submit(submit_started.elapsed());
        let submitted = painter.last_gpu_timings().unwrap();
        assert!(
            !submitted.encode.is_zero()
                || !submitted.gpu_upload.is_zero()
                || !submitted.batch.is_zero()
        );
        assert_eq!(
            painter.last_gpu_work().unwrap().gpu_upload_bytes,
            observed.gpu_upload_bytes
        );
    }

    #[test]
    fn text_shape_cache_hits_on_repaint_with_identical_pixels() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let scene = labeled_selected_button_scene();
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);

        let paint_once = |painter: &mut SceneWgpuPainter, scene: &UiScene| -> Vec<u8> {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nana-ui shape cache test"),
            });
            painter
                .paint(scene, &mut encoder, &view, viewport, None, None)
                .unwrap();
            queue.submit([encoder.finish()]);
            readback_rgba(
                &device,
                &queue,
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nana-ui shape cache readback"),
                }),
                &texture,
                64,
                64,
            )
        };

        let first = paint_once(&mut painter, &scene);
        let (hits_after_first, misses_after_first, _) = painter.text_shape_cache_stats();
        assert!(
            misses_after_first > 0,
            "labeled button scene contains text that must shape once"
        );
        assert_eq!(hits_after_first, 0);

        // A clone carries the same text under a fresh scene instance, so the
        // frame is rebatched and the shaping cache is the only thing that can
        // save the work.
        let second = paint_once(&mut painter, &scene.clone());
        let (hits_after_second, misses_after_second, _) = painter.text_shape_cache_stats();
        assert_eq!(
            misses_after_second, misses_after_first,
            "repaint of an unchanged scene must not reshape any paragraph"
        );
        assert!(
            hits_after_second >= misses_after_first,
            "every paragraph from the first frame must be a cache hit on repaint"
        );
        assert_eq!(
            first, second,
            "cached shaping must produce identical pixels"
        );
    }

    #[test]
    fn repaint_of_an_unchanged_scene_reblits_dest_without_rebatching() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let scene = labeled_selected_button_scene();
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);

        let paint_once = |painter: &mut SceneWgpuPainter| -> Vec<u8> {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nana-ui unchanged repaint"),
            });
            painter
                .paint(&scene, &mut encoder, &view, viewport, None, None)
                .unwrap();
            queue.submit([encoder.finish()]);
            readback_rgba(
                &device,
                &queue,
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nana-ui unchanged repaint readback"),
                }),
                &texture,
                64,
                64,
            )
        };

        let first = paint_once(&mut painter);
        let encoded = painter
            .last_dest_pass_counts
            .expect("first frame encodes the scene");
        assert!(
            encoded.msaa > 0 && encoded.blit == 1,
            "first frame must paint the scene into dest, got {encoded:?}"
        );
        assert!(
            painter.last_gpu_work().unwrap().draw_calls > 0,
            "first frame must issue draws"
        );

        let second = paint_once(&mut painter);
        let reused = painter
            .last_dest_pass_counts
            .expect("repaint still encodes the blit");
        assert_eq!(
            (reused.msaa, reused.color, reused.blit),
            (0, 0, 1),
            "unchanged scene and viewport must only re-blit dest, got {reused:?}"
        );
        let work = painter.last_gpu_work().unwrap();
        assert_eq!(
            work.gpu_upload_bytes, 0,
            "reused frame must not re-upload quads or meshes"
        );
        assert_eq!(
            first, second,
            "re-blitting dest must produce the same pixels as encoding it"
        );

        // A scene change gives up the reuse and paints again.
        let mut changed = scene.clone();
        changed.apply_delta(
            [colored_quad_node(
                99,
                0.0,
                0.0,
                64.0,
                64.0,
                [0.0, 1.0, 0.0, 1.0],
            )],
            [],
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui changed repaint"),
        });
        painter
            .paint(&changed, &mut encoder, &view, viewport, None, None)
            .unwrap();
        queue.submit([encoder.finish()]);
        let repainted = painter
            .last_dest_pass_counts
            .expect("changed scene encodes again");
        assert!(
            repainted.msaa > 0,
            "a changed scene must give up the reuse and repaint dest, got {repainted:?}"
        );
        drop(texture);
    }

    #[test]
    fn paint_draws_node_inserted_by_in_place_apply_delta() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [colored_quad_node(
                1,
                0.0,
                0.0,
                32.0,
                64.0,
                [1.0, 0.0, 0.0, 1.0],
            )],
            [],
        );
        let first_instance = scene.instance_id();
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);

        let paint = |painter: &mut SceneWgpuPainter, scene: &UiScene| -> Vec<u8> {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nana-ui in-place delta paint"),
            });
            painter
                .paint(scene, &mut encoder, &view, viewport, None, None)
                .unwrap();
            readback_rgba(&device, &queue, encoder, &texture, 64, 64)
        };

        let first = paint(&mut painter, &scene);
        let left = pixel(&first, 64, 8, 32);
        let right = pixel(&first, 64, 48, 32);
        assert!(
            is_red_slot(left),
            "first paint must draw the existing quad, got {left:?}"
        );
        assert!(
            right[0] < 40 && right[1] < 40 && right[2] < 40,
            "right half must stay clear before the inserted node, got {right:?}"
        );

        scene.apply_delta(
            [colored_quad_node(
                2,
                32.0,
                0.0,
                32.0,
                64.0,
                [0.0, 1.0, 0.0, 1.0],
            )],
            [],
        );
        assert_ne!(
            scene.instance_id(),
            first_instance,
            "in-place apply_delta must refresh instance so the painter cannot reuse the stale op stream"
        );

        let second = paint(&mut painter, &scene);
        let left = pixel(&second, 64, 8, 32);
        let right = pixel(&second, 64, 48, 32);
        assert!(
            is_red_slot(left),
            "existing quad must still draw after the in-place insert, got {left:?}"
        );
        assert!(
            is_green_slot(right),
            "new primitive must be encoded after in-place apply_delta, got {right:?}"
        );
    }

    fn graph_canvas_stroke_node(
        value: u64,
        edges: Vec<(Vec<[f32; 2]>, [f32; 4])>,
        background: [f32; 4],
    ) -> ExtractedNode {
        let mut node = extracted_div(
            value,
            &[],
            0.0,
            0.0,
            64.0,
            64.0,
            nana_ui_core::LayoutStyle::default(),
            Some(background),
        );
        node.standard_visual = Some(StandardVisual::GraphCanvas {
            nodes: Arc::from([]),
            ports: Arc::from([]),
            edges: Arc::from([]),
            connecting: None,
            grid_spacing: 24.0,
            viewport_offset_x: 0.0,
            viewport_offset_y: 0.0,
            viewport_zoom: 1.0,
        });
        node.component_geometry = Some(ComponentGeometry::GraphCanvas {
            nodes: Vec::new(),
            separators: Vec::new(),
            ports: Vec::new(),
            port_labels: Vec::new(),
            edges,
            edge_labels: Vec::new(),
            grid: Vec::new(),
            background,
            grid_color: [0.0, 0.0, 0.0, 0.0],
            separator_color: [0.0, 0.0, 0.0, 0.0],
        });
        node
    }

    #[test]
    fn graph_canvas_stroke_paints_capsule_coverage_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let background = [0.0, 0.0, 1.0, 1.0];
        scene.apply_delta(
            [graph_canvas_stroke_node(
                1,
                vec![(
                    vec![[8.0, 32.0], [56.0, 32.0], [56.0, 12.0]],
                    [1.0, 0.0, 0.0, 1.0],
                )],
                background,
            )],
            [],
        );
        assert!(
            scene
                .primitives()
                .any(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Stroke { .. })),
            "GraphCanvas edges must extract as Stroke, not TimeSeriesChart quads"
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui articulated stroke"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let midline = pixel(&pixels, 64, 32, 32);
        assert!(
            is_red_slot(midline),
            "horizontal capsule midline must ink, got {midline:?}"
        );
        let join = pixel(&pixels, 64, 56, 32);
        assert!(
            join[0] > 120,
            "articulated join must keep the shared endpoint disc, got {join:?}"
        );
        let far = pixel(&pixels, 64, 32, 8);
        assert!(
            is_blue_slot(far),
            "pixels outside the 1.6px capsule must stay GraphCanvas fill, got {far:?}"
        );
        let covering_corner = pixel(&pixels, 64, 32, 28);
        assert!(
            covering_corner[2] > 120 && covering_corner[0] < 80,
            "covering-quad corners 4px off the 0.8px radius must be discarded, got {covering_corner:?}"
        );
        drop(texture);
    }

    #[test]
    fn graph_canvas_stroke_paints_round_end_caps_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [graph_canvas_stroke_node(
                1,
                vec![(vec![[16.0, 32.0], [48.0, 32.0]], [1.0, 0.0, 0.0, 1.0])],
                [0.0, 0.0, 1.0, 1.0],
            )],
            [],
        );
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [256, 256],
            4.0,
        );
        let midline = pixel(&pixels, 256, 128, 128);
        assert!(
            is_red_slot(midline),
            "4× physical midline must ink the 1.6px capsule, got {midline:?}"
        );
        let start_cap = pixel(&pixels, 256, 62, 128);
        assert!(
            start_cap[0] > 120,
            "round cap 0.5 logical px before the start must stay inside the disc, got {start_cap:?}"
        );
        let end_cap = pixel(&pixels, 256, 194, 128);
        assert!(
            end_cap[0] > 120,
            "round cap 0.5 logical px past the end must stay inside the disc, got {end_cap:?}"
        );
        let beyond_cap = pixel(&pixels, 256, 48, 128);
        assert!(
            is_blue_slot(beyond_cap),
            "4 logical px before the start must stay GraphCanvas fill, got {beyond_cap:?}"
        );
        let far_normal = pixel(&pixels, 256, 128, 112);
        assert!(
            is_blue_slot(far_normal),
            "4 logical px off the 0.8px radius must stay fill, got {far_normal:?}"
        );
    }

    #[test]
    fn tapered_stroke_paints_uneven_capsule_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [graph_canvas_stroke_node(
                1,
                vec![(vec![[8.0, 32.0], [56.0, 32.0]], [1.0, 0.0, 0.0, 1.0])],
                [0.0, 0.0, 1.0, 1.0],
            )],
            [],
        );
        let primitive = scene
            .primitives()
            .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Stroke { .. }))
            .expect("extracted stroke");
        assert!(scene.replace_primitive_kind(
            primitive.id,
            ScenePrimitiveKind::Stroke {
                points: vec![[8.0, 32.0], [56.0, 32.0]],
                width: 2.0,
                color: [1.0, 0.0, 0.0, 1.0],
                widths: vec![12.0, 4.0],
                cap: StrokeCap::Round,
                pattern: None,
            },
        ));
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [64, 64],
            1.0,
        );
        let thick_end = pixel(&pixels, 64, 8, 32);
        assert!(
            is_red_slot(thick_end),
            "6px radius at the start must ink, got {thick_end:?}"
        );
        let thick_side = pixel(&pixels, 64, 8, 28);
        assert!(
            thick_side[0] > 120,
            "4px off the start must stay inside the 6px disc, got {thick_side:?}"
        );
        let thin_side = pixel(&pixels, 64, 56, 28);
        assert!(
            is_blue_slot(thin_side),
            "4px off the 2px end must stay fill, got {thin_side:?}"
        );
        let thin_mid = pixel(&pixels, 64, 56, 32);
        assert!(
            thin_mid[0] > 120,
            "2px radius end midline must ink, got {thin_mid:?}"
        );
    }

    #[test]
    fn butt_stroke_cuts_round_end_caps_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [graph_canvas_stroke_node(
                1,
                vec![(vec![[16.0, 32.0], [48.0, 32.0]], [1.0, 0.0, 0.0, 1.0])],
                [0.0, 0.0, 1.0, 1.0],
            )],
            [],
        );
        let primitive = scene
            .primitives()
            .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Stroke { .. }))
            .expect("extracted stroke");
        assert!(scene.replace_primitive_kind(
            primitive.id,
            ScenePrimitiveKind::Stroke {
                points: vec![[16.0, 32.0], [48.0, 32.0]],
                width: 8.0,
                color: [1.0, 0.0, 0.0, 1.0],
                widths: Vec::new(),
                cap: StrokeCap::Butt,
                pattern: None,
            },
        ));
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [256, 256],
            4.0,
        );
        let midline = pixel(&pixels, 256, 128, 128);
        assert!(
            is_red_slot(midline),
            "butt midline must ink the 8px stroke, got {midline:?}"
        );
        let past_start = pixel(&pixels, 256, 58, 128);
        assert!(
            is_blue_slot(past_start),
            "1.5 logical px before a butt start must stay fill, got {past_start:?}"
        );
        let start_edge = pixel(&pixels, 256, 64, 128);
        assert!(
            start_edge[0] > 80,
            "the butt start cut must still ink on the endpoint, got {start_edge:?}"
        );
    }

    #[test]
    fn square_stroke_extends_flat_caps_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [graph_canvas_stroke_node(
                1,
                vec![(vec![[16.0, 32.0], [48.0, 32.0]], [1.0, 0.0, 0.0, 1.0])],
                [0.0, 0.0, 1.0, 1.0],
            )],
            [],
        );
        let primitive = scene
            .primitives()
            .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Stroke { .. }))
            .expect("extracted stroke");
        assert!(scene.replace_primitive_kind(
            primitive.id,
            ScenePrimitiveKind::Stroke {
                points: vec![[16.0, 32.0], [48.0, 32.0]],
                width: 8.0,
                color: [1.0, 0.0, 0.0, 1.0],
                widths: Vec::new(),
                cap: StrokeCap::Square,
                pattern: None,
            },
        ));
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [256, 256],
            4.0,
        );
        let outside_round = pixel(&pixels, 256, 52, 116);
        assert!(
            is_red_slot(outside_round),
            "square cap must ink the half-width box past the start, got {outside_round:?}"
        );
        let past_square = pixel(&pixels, 256, 44, 128);
        assert!(
            is_blue_slot(past_square),
            "beyond the square extension must stay fill, got {past_square:?}"
        );
    }

    #[test]
    fn dashed_stroke_skips_gaps_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [graph_canvas_stroke_node(
                1,
                vec![(vec![[8.0, 32.0], [56.0, 32.0]], [1.0, 0.0, 0.0, 1.0])],
                [0.0, 0.0, 1.0, 1.0],
            )],
            [],
        );
        let primitive = scene
            .primitives()
            .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Stroke { .. }))
            .expect("extracted stroke");
        assert!(scene.replace_primitive_kind(
            primitive.id,
            ScenePrimitiveKind::Stroke {
                points: vec![[8.0, 32.0], [56.0, 32.0]],
                width: 8.0,
                color: [1.0, 0.0, 0.0, 1.0],
                widths: Vec::new(),
                cap: StrokeCap::Butt,
                pattern: Some(Box::new(StrokePattern {
                    dash: vec![8.0, 8.0],
                    dash_offset: 0.0,
                    colors: Vec::new(),
                })),
            },
        ));
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [64, 64],
            1.0,
        );
        let dash = pixel(&pixels, 64, 12, 32);
        assert!(
            is_red_slot(dash),
            "first 8px on-dash must ink, got {dash:?}"
        );
        let gap = pixel(&pixels, 64, 20, 32);
        assert!(
            is_blue_slot(gap),
            "8px off-dash gap must stay fill, got {gap:?}"
        );
        let next = pixel(&pixels, 64, 28, 32);
        assert!(is_red_slot(next), "second on-dash must ink, got {next:?}");
    }

    #[test]
    fn sdf_dashed_border_skips_gaps_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [extracted_div(
                1,
                &[],
                0.0,
                0.0,
                80.0,
                80.0,
                nana_ui_core::LayoutStyle {
                    background: Some([1.0, 1.0, 1.0, 1.0]),
                    border_width: Some(4.0),
                    border_color: Some([1.0, 0.0, 0.0, 1.0]),
                    border_style: Some(nana_ui_core::BorderStyle::Dashed),
                    ..nana_ui_core::LayoutStyle::default()
                },
                Some([1.0, 1.0, 1.0, 1.0]),
            )],
            [],
        );
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [80.0, 80.0],
            [80, 80],
            1.0,
        );
        // w=4 → dash 12, gap 8, period 20. Sample the top ring, away from corners.
        let dash = pixel(&pixels, 80, 10, 2);
        assert!(
            is_red_slot(dash),
            "dashed on-segment must stroke, got {dash:?}"
        );
        let gap = pixel(&pixels, 80, 16, 2);
        assert!(
            gap[0] > 200 && gap[1] > 200 && gap[2] > 200,
            "dashed gap must show fill, got {gap:?}"
        );
    }

    #[test]
    fn sdf_dotted_border_skips_gaps_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [extracted_div(
                1,
                &[],
                0.0,
                0.0,
                80.0,
                80.0,
                nana_ui_core::LayoutStyle {
                    background: Some([1.0, 1.0, 1.0, 1.0]),
                    border_width: Some(4.0),
                    border_color: Some([1.0, 0.0, 0.0, 1.0]),
                    border_style: Some(nana_ui_core::BorderStyle::Dotted),
                    ..nana_ui_core::LayoutStyle::default()
                },
                Some([1.0, 1.0, 1.0, 1.0]),
            )],
            [],
        );
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [80.0, 80.0],
            [80, 80],
            1.0,
        );
        // w=4 → round dots period 8, radius 2, on the top centerline.
        let on = pixel(&pixels, 80, 40, 2);
        assert!(is_red_slot(on), "dotted center must stroke, got {on:?}");
        let off = pixel(&pixels, 80, 44, 2);
        assert!(
            off[0] > 200 && off[1] > 200 && off[2] > 200,
            "dotted gap must show fill, got {off:?}"
        );
    }

    #[test]
    fn per_point_stroke_colors_paint_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [graph_canvas_stroke_node(
                1,
                vec![(
                    vec![[8.0, 32.0], [32.0, 32.0], [56.0, 32.0]],
                    [1.0, 0.0, 0.0, 1.0],
                )],
                [0.0, 0.0, 1.0, 1.0],
            )],
            [],
        );
        let primitive = scene
            .primitives()
            .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Stroke { .. }))
            .expect("extracted stroke");
        assert!(scene.replace_primitive_kind(
            primitive.id,
            ScenePrimitiveKind::Stroke {
                points: vec![[8.0, 32.0], [32.0, 32.0], [56.0, 32.0]],
                width: 8.0,
                color: [1.0, 0.0, 0.0, 1.0],
                widths: Vec::new(),
                cap: StrokeCap::Butt,
                pattern: Some(Box::new(StrokePattern {
                    dash: Vec::new(),
                    dash_offset: 0.0,
                    colors: vec![
                        [1.0, 0.0, 0.0, 1.0],
                        [0.0, 1.0, 0.0, 1.0],
                        [0.0, 1.0, 0.0, 1.0],
                    ],
                })),
            },
        ));
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [64, 64],
            1.0,
        );
        let red = pixel(&pixels, 64, 16, 32);
        assert!(
            is_red_slot(red),
            "first segment midpoint color must stay red, got {red:?}"
        );
        let green = pixel(&pixels, 64, 48, 32);
        assert!(
            is_green_slot(green),
            "second segment must use the green start color, got {green:?}"
        );
    }

    #[test]
    fn graph_canvas_diagonal_stroke_paints_capsule_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [graph_canvas_stroke_node(
                1,
                vec![(vec![[8.0, 8.0], [56.0, 56.0]], [1.0, 0.0, 0.0, 1.0])],
                [0.0, 0.0, 1.0, 1.0],
            )],
            [],
        );
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [64, 64],
            1.0,
        );
        let midline = pixel(&pixels, 64, 32, 32);
        assert!(
            is_red_slot(midline),
            "diagonal capsule midline must ink, got {midline:?}"
        );
        let far = pixel(&pixels, 64, 8, 56);
        assert!(
            is_blue_slot(far),
            "pixels far from the diagonal must stay GraphCanvas fill, got {far:?}"
        );
        let off_normal = pixel(&pixels, 64, 32, 28);
        assert!(
            off_normal[2] > 120 && off_normal[0] < 80,
            "4px off the diagonal must discard covering-quad corners, got {off_normal:?}"
        );
    }

    #[test]
    fn graph_canvas_stroke_respects_ancestor_overflow_clip_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let mut canvas = graph_canvas_stroke_node(
            3,
            vec![(vec![[8.0, 32.0], [56.0, 32.0]], [1.0, 0.0, 0.0, 1.0])],
            [0.0, 1.0, 0.0, 1.0],
        );
        canvas.parent = Some(StableNodeId::new(2).unwrap());
        scene.apply_delta(
            [
                colored_quad_node(1, 0.0, 0.0, 64.0, 64.0, [0.0, 0.0, 1.0, 1.0]),
                overflow_parent(2, &[3], 20.0, 20.0, 24.0, 24.0, None),
                canvas,
            ],
            [],
        );
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [64, 64],
            1.0,
        );
        let inside_stroke = pixel(&pixels, 64, 32, 32);
        assert!(
            is_red_slot(inside_stroke),
            "stroke inside the overflow clip must ink, got {inside_stroke:?}"
        );
        let inside_fill = pixel(&pixels, 64, 32, 26);
        assert!(
            is_green_slot(inside_fill),
            "GraphCanvas fill inside the clip and off the 1.6px stroke must stay green, got {inside_fill:?}"
        );
        let leaked_start = pixel(&pixels, 64, 8, 32);
        assert!(
            is_blue_slot(leaked_start),
            "stroke past the ancestor clip must not ink, got {leaked_start:?}"
        );
        let leaked_end = pixel(&pixels, 64, 56, 32);
        assert!(
            is_blue_slot(leaked_end),
            "stroke past the far clip edge must not ink, got {leaked_end:?}"
        );
    }

    #[test]
    fn spinner_ticks_paint_capsule_coverage_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let mut spinner = extracted_div(
            2,
            &[],
            0.0,
            0.0,
            64.0,
            64.0,
            nana_ui_core::LayoutStyle::default(),
            None,
        );
        spinner.standard_visual = Some(StandardVisual::Spinner {
            label: Arc::from(""),
            size: 24.0,
            phase: 0.0,
        });
        spinner.standard_visual_foreground = Some([1.0, 0.0, 0.0, 1.0]);
        scene.apply_delta(
            [
                colored_quad_node(1, 0.0, 0.0, 64.0, 64.0, [0.0, 0.0, 1.0, 1.0]),
                spinner,
            ],
            [],
        );
        assert!(
            scene
                .primitives()
                .any(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Spinner { .. })),
            "Spinner visual must extract as Spinner capsules"
        );
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [64, 64],
            1.0,
        );
        let tick = pixel(&pixels, 64, 20, 32);
        assert!(
            tick[0] > 120,
            "phase-0 east tick midline (20,32) must ink, got {tick:?}"
        );
        let hub = pixel(&pixels, 64, 12, 32);
        assert!(
            is_blue_slot(hub),
            "spinner hub is inside the ring, not a tick, got {hub:?}"
        );
        let far = pixel(&pixels, 64, 48, 16);
        assert!(
            is_blue_slot(far),
            "pixels outside the 24px spinner must stay the sibling fill, got {far:?}"
        );
    }

    #[test]
    fn time_series_line_paints_capsule_coverage_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                colored_quad_node(1, 0.0, 0.0, 64.0, 64.0, [0.0, 0.0, 1.0, 1.0]),
                time_series_stroke_node(
                    2,
                    vec![[8.0, 32.0], [56.0, 32.0], [56.0, 12.0]],
                    [1.0, 0.0, 0.0, 1.0],
                ),
            ],
            [],
        );
        assert!(
            scene
                .primitives()
                .any(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Stroke { .. })),
            "TimeSeriesChart line must extract as Stroke, not tiled QuadBatch"
        );
        let pixels = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [64, 64],
            1.0,
        );
        let midline = pixel(&pixels, 64, 32, 32);
        assert!(
            is_red_slot(midline),
            "chart line midline must ink the 2px capsule, got {midline:?}"
        );
        let join = pixel(&pixels, 64, 56, 32);
        assert!(
            join[0] > 120,
            "articulated join must keep the shared endpoint disc, got {join:?}"
        );
        let far = pixel(&pixels, 64, 32, 8);
        assert!(
            is_blue_slot(far),
            "pixels outside the 2px capsule must stay the sibling fill, got {far:?}"
        );
        let covering_corner = pixel(&pixels, 64, 32, 28);
        assert!(
            covering_corner[2] > 120 && covering_corner[0] < 80,
            "covering-quad corners 4px off the 1px radius must be discarded, got {covering_corner:?}"
        );
    }

    fn time_series_stroke_node(
        value: u64,
        points: Vec<[f32; 2]>,
        color: [f32; 4],
    ) -> ExtractedNode {
        let mut node = extracted_div(
            value,
            &[],
            0.0,
            0.0,
            64.0,
            64.0,
            nana_ui_core::LayoutStyle::default(),
            None,
        );
        node.standard_visual = Some(StandardVisual::TimeSeriesChart {
            values: Arc::from([0.0, 1.0]),
        });
        node.component_geometry = Some(ComponentGeometry::TimeSeriesChart {
            grid: Vec::new(),
            area: Vec::new(),
            line: points,
            grid_color: [0.0, 0.0, 0.0, 0.0],
            area_color: [0.0, 0.0, 0.0, 0.0],
            line_color: color,
        });
        node
    }

    #[test]
    fn graph_canvas_stroke_gpu_upload_scales_with_segment_count() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let fill = encode_scene_gpu_work(
            &device,
            &queue,
            &mut painter,
            &graph_canvas_scene(Vec::new()),
        );
        let work_32 = encode_scene_gpu_work(
            &device,
            &queue,
            &mut painter,
            &graph_canvas_scene(l_stroke_edges(32)),
        );
        let work_64 = encode_scene_gpu_work(
            &device,
            &queue,
            &mut painter,
            &graph_canvas_scene(l_stroke_edges(64)),
        );
        let mesh_32 = work_32
            .gpu_upload_bytes
            .saturating_sub(fill.gpu_upload_bytes);
        let mesh_64 = work_64
            .gpu_upload_bytes
            .saturating_sub(fill.gpu_upload_bytes);
        assert!(mesh_32 > 0, "strokes must add mesh upload bytes");
        assert_eq!(
            mesh_64,
            mesh_32 * 2,
            "doubling GraphCanvas edges must double isolated mesh upload bytes"
        );
        assert_eq!(
            work_32.draw_calls.saturating_sub(fill.draw_calls),
            1,
            "consecutive same-scissor Stroke primitives batch into one mesh draw"
        );
        assert_eq!(
            work_64.draw_calls.saturating_sub(fill.draw_calls),
            1,
            "64 batched GraphCanvas edges still issue one mesh draw"
        );
    }

    #[test]
    fn graph_canvas_stroke_skips_identical_instance_upload() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let scene = graph_canvas_scene(l_stroke_edges(16));
        let first = encode_scene_gpu_work(&device, &queue, &mut painter, &scene);
        let second = encode_scene_gpu_work(&device, &queue, &mut painter, &scene.clone());
        assert!(
            second.gpu_upload_bytes < first.gpu_upload_bytes,
            "rebuilding the same stroke instances must skip the mesh upload, first {} second {}",
            first.gpu_upload_bytes,
            second.gpu_upload_bytes
        );
    }

    fn graph_canvas_scene(edges: Vec<(Vec<[f32; 2]>, [f32; 4])>) -> UiScene {
        let mut scene = UiScene::new();
        scene.apply_delta(
            [graph_canvas_stroke_node(1, edges, [0.0, 0.0, 1.0, 1.0])],
            [],
        );
        scene
    }

    fn l_stroke_edges(count: usize) -> Vec<(Vec<[f32; 2]>, [f32; 4])> {
        (0..count)
            .map(|index| {
                let x = 8.0 + (index % 8) as f32 * 6.0;
                let y = 8.0 + (index / 8) as f32 * 6.0;
                (
                    vec![[x, y], [x + 4.0, y], [x + 4.0, y + 4.0]],
                    [1.0, 0.0, 0.0, 1.0],
                )
            })
            .collect()
    }

    fn paint_scene_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        painter: &mut SceneWgpuPainter,
        scene: &UiScene,
        logical_size: [f32; 2],
        physical_size: [u32; 2],
        scale_factor: f32,
    ) -> Vec<u8> {
        let viewport = ScenePaintViewport {
            logical_size,
            physical_size,
            scale_factor,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(
            device,
            wgpu::TextureFormat::Rgba8Unorm,
            physical_size[0],
            physical_size[1],
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui articulated stroke probe"),
        });
        painter
            .paint(scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(
            device,
            queue,
            encoder,
            &texture,
            physical_size[0],
            physical_size[1],
        );
        drop(texture);
        pixels
    }

    fn encode_scene_gpu_work(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        painter: &mut SceneWgpuPainter,
        scene: &UiScene,
    ) -> nana_ui_core::GpuWorkObservation {
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let target = test_target(device, wgpu::TextureFormat::Rgba8Unorm, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui articulated stroke work"),
        });
        painter
            .paint(scene, &mut encoder, &target, viewport, None, None)
            .unwrap();
        queue.submit([encoder.finish()]);
        painter
            .last_gpu_work()
            .expect("encoded GraphCanvas frame records GPU work")
    }

    #[test]
    fn rotated_clip_does_not_paint_sibling_in_aabb_outside_rect() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                colored_quad_node(1, 8.0, 8.0, 16.0, 16.0, [0.0, 0.0, 1.0, 1.0]),
                rotated_overflow_parent(2, &[3], 16.0, 16.0, 32.0, 32.0),
                colored_quad_child(3, 2, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
            ],
            [],
        );
        let (probe_x, probe_y) = aabb_outside_rotated_overflow_probe();

        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui rotated clip sibling"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let sibling = pixel(&pixels, 64, probe_x, probe_y);
        let inside = pixel(&pixels, 64, 32, 32);
        assert!(
            is_blue_slot(sibling),
            "rotated overflow must not cover sibling in AABB-outside-rect, pixel ({probe_x},{probe_y})={sibling:?}"
        );
        assert!(
            is_red_slot(inside),
            "rotated clip interior must still paint the overflowing child, got {inside:?}"
        );
        drop(texture);
    }

    #[test]
    fn rotated_clip_does_not_paint_text_sibling_in_aabb_outside_rect() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                colored_quad_node(1, 8.0, 8.0, 16.0, 16.0, [0.0, 0.0, 1.0, 1.0]),
                rotated_overflow_parent(2, &[3], 16.0, 16.0, 32.0, 32.0),
                overflowing_text_child(3, 2, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
            ],
            [],
        );
        let (probe_x, probe_y) = aabb_outside_rotated_overflow_probe();
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui rotated clip text sibling"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let sibling = pixel(&pixels, 64, probe_x, probe_y);
        let inside = pixel(&pixels, 64, 32, 32);
        assert!(
            is_blue_slot(sibling),
            "rotated overflow must not cover sibling with text in AABB-outside-rect, pixel ({probe_x},{probe_y})={sibling:?}"
        );
        assert!(
            inside[0] > 40,
            "rotated clip interior must still paint overflowing text, got {inside:?}"
        );
        drop(texture);
    }

    #[test]
    fn rotated_clip_does_not_paint_host_texture_sibling_in_aabb_outside_rect() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                colored_quad_node(1, 8.0, 8.0, 16.0, 16.0, [0.0, 0.0, 1.0, 1.0]),
                rotated_overflow_parent(2, &[3], 16.0, 16.0, 32.0, 32.0),
                host_texture_child(3, 2, 0.0, 0.0, 64.0, 64.0, "layer"),
            ],
            [],
        );
        let view = solid_texture_view(&device, &queue, format, 64, 64, wgpu::Color::RED);
        let registry = register_host_texture("layer", &view, 64, 64);
        let (probe_x, probe_y) = aabb_outside_rotated_overflow_probe();
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, target_view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui rotated clip host texture sibling"),
        });
        painter
            .paint(
                &scene,
                &mut encoder,
                &target_view,
                viewport,
                Some(&registry),
                None,
            )
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let sibling = pixel(&pixels, 64, probe_x, probe_y);
        let inside = pixel(&pixels, 64, 32, 32);
        assert!(
            is_blue_slot(sibling),
            "rotated overflow must not cover sibling with HostTexture in AABB-outside-rect, pixel ({probe_x},{probe_y})={sibling:?}"
        );
        assert!(
            is_red_slot(inside),
            "rotated clip interior must still sample HostTexture, got {inside:?}"
        );
        drop(view);
        drop(texture);
    }

    #[test]
    fn nested_rotated_clips_reject_quad_inside_inner_outside_outer() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let mut inner = overflow_parent(3, &[4], 0.0, 0.0, 64.0, 64.0, None);
        inner.parent = Some(StableNodeId::new(2).unwrap());
        scene.apply_delta(
            [
                colored_quad_node(1, 8.0, 8.0, 16.0, 16.0, [0.0, 0.0, 1.0, 1.0]),
                rotated_overflow_parent(2, &[3], 16.0, 16.0, 32.0, 32.0),
                inner,
                colored_quad_child(4, 3, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
            ],
            [],
        );
        let child = scene
            .primitives()
            .find(|primitive| {
                matches!(
                    primitive.kind,
                    ScenePrimitiveKind::Quad {
                        background: Some([1.0, 0.0, 0.0, 1.0]),
                        ..
                    }
                )
            })
            .expect("overflowing child quad");
        assert!(
            super::clip::rotated_fragment_clips(
                &child.clips,
                super::clip::paint_origin([0.0, 0.0], [0.0, 0.0])
            )
            .len()
                >= 2,
            "child must carry two 45° parallelograms, got {:?}",
            child.clips
        );
        let (probe_x, probe_y) = nested_rotated_overflow_probe(&child.clips);

        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui nested rotated clip"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let counts = painter
            .last_dest_pass_counts
            .expect("encoded frame records dest passes");
        assert!(
            counts.group >= 1,
            "two rotated clips must dest-composite the extra parallelogram, got {counts:?}"
        );
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let probe = pixel(&pixels, 64, probe_x, probe_y);
        let inside = pixel(&pixels, 64, 32, 32);
        assert!(
            !is_red_slot(probe),
            "nested extra clip must reject inner-inside/outer-outside, pixel ({probe_x},{probe_y})={probe:?}"
        );
        assert!(
            is_red_slot(inside),
            "intersection of both 45° clips must still paint, got {inside:?}"
        );
        drop(texture);
    }

    #[test]
    fn rotated_clip_does_not_paint_custom_in_aabb_outside_rect() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                colored_quad_node(1, 8.0, 8.0, 16.0, 16.0, [0.0, 0.0, 1.0, 1.0]),
                rotated_overflow_parent(2, &[3], 16.0, 16.0, 32.0, 32.0),
                custom_render_child(3, 2, 0.0, 0.0, 64.0, 64.0, "test.fill"),
            ],
            [],
        );
        let mut renderers = SceneGpuRendererRegistry::new();
        renderers.insert(
            "test.fill",
            Arc::new(FillClipRenderer::new(&device, format)),
        );
        let (probe_x, probe_y) = aabb_outside_rotated_overflow_probe();
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui rotated clip custom"),
        });
        painter
            .paint(
                &scene,
                &mut encoder,
                &view,
                viewport,
                None,
                Some(&renderers),
            )
            .unwrap();
        let counts = painter
            .last_dest_pass_counts
            .expect("encoded frame records dest passes");
        assert!(
            counts.group >= 1,
            "rotated Custom must dest-wrap fragment clip, got {counts:?}"
        );
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let sibling = pixel(&pixels, 64, probe_x, probe_y);
        let inside = pixel(&pixels, 64, 32, 32);
        assert!(
            is_blue_slot(sibling),
            "Custom AABB scissor must not leak rotated overflow, pixel ({probe_x},{probe_y})={sibling:?}"
        );
        assert!(
            is_red_slot(inside),
            "rotated clip interior must still paint Custom, got {inside:?}"
        );
        drop(texture);
    }

    #[test]
    fn axis_aligned_clips_do_not_dest_wrap() {
        let origin = super::clip::paint_origin([0.0, 0.0], [0.0, 0.0]);
        let aligned = [ClipRegion {
            bounds: SceneRect {
                x: 10.0,
                y: 10.0,
                width: 20.0,
                height: 20.0,
            },
            transform: AffineTransform::IDENTITY,
            corner_radius: 0.0,
            polygon_clip: None,
        }];
        let rotated = {
            let k = std::f32::consts::FRAC_1_SQRT_2;
            [ClipRegion {
                bounds: SceneRect {
                    x: 16.0,
                    y: 16.0,
                    width: 32.0,
                    height: 32.0,
                },
                transform: AffineTransform::from_matrix(
                    PaintTransform {
                        a: k,
                        b: k,
                        c: -k,
                        d: k,
                        ..PaintTransform::default()
                    }
                    .around_center(16.0, 16.0, 32.0, 32.0),
                ),
                corner_radius: 0.0,
                polygon_clip: None,
            }]
        };
        let custom = ScenePrimitiveKind::Custom(CustomRenderNode::new("test.fill", "slot", 1));
        let quad = ScenePrimitiveKind::Quad {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            border_color: None,
            border_width: 0.0,
            corner_radius: [0.0; 4],
            shadow: None,
            surface: nana_ui_scene::QuadSurfacePaint::default(),
        };
        assert!(clip_dests_for(&custom, &aligned, origin).is_empty());
        assert!(clip_dests_for(&quad, &aligned, origin).is_empty());
        assert_eq!(clip_dests_for(&custom, &rotated, origin).len(), 1);
        assert!(
            clip_dests_for(&quad, &rotated, origin).is_empty(),
            "single rotated clip stays in Quad vertex attrs"
        );
        assert_eq!(
            super::clip::fragment_clip(&aligned, origin),
            super::clip::FragmentClip::PASS
        );

        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                overflow_parent(1, &[2], 0.0, 0.0, 64.0, 64.0, None),
                colored_quad_child(2, 1, 8.0, 8.0, 48.0, 48.0, [1.0, 0.0, 0.0, 1.0]),
            ],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let target = test_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui axis-aligned clip no dest wrap"),
        });
        painter
            .paint(&scene, &mut encoder, &target, viewport, None, None)
            .unwrap();
        let counts = painter
            .last_dest_pass_counts
            .expect("encoded frame records dest passes");
        assert_eq!(
            counts.group, 0,
            "axis-aligned overflow must stay PASS fragment_clip, got {counts:?}"
        );
        assert_eq!(
            counts.msaa, 1,
            "axis-aligned clip must not force dest-group interleave, got {counts:?}"
        );
        queue.submit([encoder.finish()]);
    }

    #[test]
    fn translucent_parent_composites_overlapping_children_as_a_group() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                translucent_parent(1, &[2, 3], 0.0, 0.0, 64.0, 32.0, 0.5),
                colored_quad_child(2, 1, 0.0, 0.0, 40.0, 32.0, [1.0, 0.0, 0.0, 1.0]),
                colored_quad_child(3, 1, 24.0, 0.0, 40.0, 32.0, [1.0, 0.0, 0.0, 1.0]),
            ],
            [],
        );
        assert_eq!(
            scene.opacity_groups(nana_ui_runtime::StableNodeId::new(2).unwrap()),
            vec![nana_ui_scene::OpacityGroup {
                node: nana_ui_runtime::StableNodeId::new(1).unwrap(),
                opacity: 0.5,
                filter: nana_ui_core::ColorFilter::default(),
                mix_blend: nana_ui_core::MixBlendMode::Normal,
            }]
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 32.0],
            physical_size: [64, 32],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 32);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui group opacity paint"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let counts = painter
            .last_dest_pass_counts
            .expect("encoded frame records dest passes");
        assert!(
            counts.group >= 1,
            "overlapping translucent children must use a group layer, got {counts:?}"
        );
        assert_eq!(
            counts.msaa, 0,
            "group frames share sample_count=1 with GPU-interleaved dest, got {counts:?}"
        );
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 32);
        let left = pixel(&pixels, 64, 8, 16);
        let overlap = pixel(&pixels, 64, 32, 16);
        let right = pixel(&pixels, 64, 56, 16);
        assert!(
            left[0] > 90 && left[0] < 160 && left[1] < 40 && left[2] < 40,
            "non-overlap must be parent opacity over black, got {left:?}"
        );
        assert!(
            right[0] > 90 && right[0] < 160 && right[1] < 40 && right[2] < 40,
            "non-overlap must be parent opacity over black, got {right:?}"
        );
        let overlap_delta = (overlap[0] as i16 - left[0] as i16).unsigned_abs();
        assert!(
            overlap_delta < 20 && overlap[1] < 40 && overlap[2] < 40,
            "group opacity must not darken overlapping children, left={left:?} overlap={overlap:?}"
        );
        drop(texture);
    }

    #[test]
    fn drop_shadow_samples_dest_group_alpha_not_box_shadow_quads() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let shadow_srgb = [0.5, 0.5, 0.5, 1.0];
        scene.apply_delta(
            [extracted_div(
                1,
                &[],
                8.0,
                8.0,
                16.0,
                16.0,
                nana_ui_core::LayoutStyle {
                    background: Some([1.0, 0.0, 0.0, 1.0]),
                    paint: nana_ui_core::PaintStyle {
                        filter: Some(nana_ui_core::ColorFilter {
                            drop_shadow: Some(nana_ui_core::FilterDropShadow {
                                offset_x: 16.0,
                                offset_y: 0.0,
                                blur_radius: 0.0,
                                color: shadow_srgb,
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Some([1.0, 0.0, 0.0, 1.0]),
            )],
            [],
        );
        assert_eq!(
            scene
                .opacity_groups(nana_ui_runtime::StableNodeId::new(1).unwrap())
                .len(),
            1,
            "drop-shadow must isolate a dest-group"
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui drop-shadow dest-group paint"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let counts = painter
            .last_dest_pass_counts
            .expect("encoded frame records dest passes");
        assert!(
            counts.group >= 1,
            "drop-shadow must composite via dest-group, got {counts:?}"
        );
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let fill = pixel(&pixels, 64, 12, 16);
        let shadow = pixel(&pixels, 64, 32, 16);
        let left = pixel(&pixels, 64, 4, 16);
        let linearized = super::color::to_rgba8(super::color::pack_linear(shadow_srgb));
        let raw_srgb = super::color::to_rgba8(shadow_srgb);
        assert!(
            fill[0] > 200 && fill[1] < 40 && fill[2] < 40,
            "source silhouette must stay red, got {fill:?}"
        );
        assert!(
            (shadow[0] as i16 - linearized[0] as i16).unsigned_abs() < 20
                && (shadow[1] as i16 - linearized[1] as i16).unsigned_abs() < 20
                && (shadow[2] as i16 - linearized[2] as i16).unsigned_abs() < 20,
            "mid-channel drop-shadow must pack_linear like box-shadow, got {shadow:?} expected {linearized:?}"
        );
        assert!(
            (shadow[0] as i16 - raw_srgb[0] as i16).unsigned_abs() > 40,
            "raw sRGB gray must not pass as linearized, got {shadow:?} raw {raw_srgb:?}"
        );
        assert!(
            left[0] < 20 && left[1] < 20 && left[2] < 20,
            "no box-shadow geometry left of the silhouette, got {left:?}"
        );
        drop(texture);
    }

    #[test]
    fn outline_and_shadow_spread_stay_css_px_at_hidpi() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [extracted_div(
                1,
                &[],
                24.0,
                24.0,
                16.0,
                16.0,
                nana_ui_core::LayoutStyle {
                    background: Some([1.0, 0.0, 0.0, 1.0]),
                    paint: nana_ui_core::PaintStyle {
                        outline: nana_ui_core::OutlineSpec {
                            width: 2.0,
                            color: Some([0.0, 1.0, 0.0, 1.0]),
                            style: nana_ui_core::OutlineStyle::Solid,
                        },
                        box_shadows: vec![nana_ui_core::BoxShadowSpec {
                            offset_x: 0.0,
                            offset_y: 0.0,
                            blur_radius: 0.0,
                            spread_radius: 4.0,
                            color: [0.0, 0.0, 1.0, 1.0],
                            inset: false,
                        }],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Some([1.0, 0.0, 0.0, 1.0]),
            )],
            [],
        );

        let assert_band = |label: &str, pixels: &[u8], width: u32, scale: f32| {
            let sx = |logical: f32| (logical * scale).floor() as u32;
            let fill = pixel(pixels, width, sx(32.0), sx(32.0));
            let outline = pixel(pixels, width, sx(23.0), sx(32.0));
            let spread = pixel(pixels, width, sx(21.0), sx(32.0));
            let beyond = pixel(pixels, width, sx(19.5), sx(32.0));
            assert!(
                is_red_slot(fill),
                "{label} fill must stay red, got {fill:?}"
            );
            assert!(
                is_green_slot(outline),
                "{label} 2px outline must cover 1 CSS px outside the box, got {outline:?}"
            );
            assert!(
                is_blue_slot(spread),
                "{label} spread 4px must cover 3 CSS px outside the box, got {spread:?}"
            );
            assert!(
                beyond[0] < 20 && beyond[1] < 20 && beyond[2] < 20,
                "{label} must not enlarge spread at HiDPI (4.5 CSS px outside), got {beyond:?}"
            );
        };

        let one_x = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [64, 64],
            1.0,
        );
        assert_band("1x", &one_x, 64, 1.0);
        let two_x = paint_scene_rgba(
            &device,
            &queue,
            &mut painter,
            &scene,
            [64.0, 64.0],
            [128, 128],
            2.0,
        );
        assert_band("2x", &two_x, 128, 2.0);
    }

    #[test]
    fn host_texture_paints_in_document_order_with_runtime_button() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let (scene, registry, texture, overlay_fill) =
            runtime_button_over_host_texture_scene(&device, &queue, format);
        assert!(
            overlay_fill[3] > 0.99,
            "Selected Button theme fill must be opaque, got {overlay_fill:?}"
        );
        let (target, target_view) = test_copy_target(&device, format, 64, 64);
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui runtime button order paint"),
        });
        painter
            .paint(
                &scene,
                &mut encoder,
                &target_view,
                viewport,
                Some(&registry),
                None,
            )
            .unwrap();
        assert_mixed_gpu_pass_counts(&painter);
        let order = scene
            .primitives()
            .map(|primitive| match &primitive.kind {
                ScenePrimitiveKind::Quad { .. } => "quad",
                ScenePrimitiveKind::Custom(_) => "custom",
                ScenePrimitiveKind::Text { .. } => "text",
                _ => "other",
            })
            .collect::<Vec<_>>();
        assert!(
            order.iter().filter(|kind| **kind == "custom").count() == 1 && order.contains(&"quad"),
            "runtime tree must emit background/overlay quads and one host texture, got {order:?}"
        );
        let pixels = readback_rgba(&device, &queue, encoder, &target, 64, 64);
        let background = pixel(&pixels, 64, 32, 8);
        let texture_only = pixel(&pixels, 64, 32, 18);
        let foreground = pixel(&pixels, 64, 32, 36);
        assert!(
            is_danger_fill(background),
            "background quad below the texture must stay the Danger fill, got {background:?} order={order:?}"
        );
        assert!(
            is_green_slot(texture_only),
            "texture-only band should stay green, got {texture_only:?} order={order:?}"
        );
        assert!(
            !is_green_slot(foreground) && !is_danger_fill(foreground),
            "Selected Button fill must paint over the host texture, got {foreground:?} order={order:?} fill={overlay_fill:?}"
        );
        drop(texture);
    }

    #[test]
    fn two_host_texture_layers_paint_with_opaque_chrome_between() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let (scene, registry, layers) =
            two_host_texture_layers_with_chrome_scene(&device, &queue, format);
        let (target, target_view) = test_copy_target(&device, format, 64, 64);
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui layered host texture paint"),
        });
        painter
            .paint(
                &scene,
                &mut encoder,
                &target_view,
                viewport,
                Some(&registry),
                None,
            )
            .unwrap();
        assert_mixed_gpu_pass_counts(&painter);
        let custom = scene
            .primitives()
            .filter(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom(_)))
            .count();
        assert_eq!(custom, 2, "layered scene must sample two HostTexture nodes");
        let pixels = readback_rgba(&device, &queue, encoder, &target, 64, 64);
        let background_only = pixel(&pixels, 64, 32, 8);
        let chrome = pixel(&pixels, 64, 32, 20);
        let foreground = pixel(&pixels, 64, 32, 36);
        assert!(
            is_blue_slot(background_only),
            "bg-only region must stay the background texture, got {background_only:?}"
        );
        assert!(
            !is_blue_slot(chrome) && !is_red_slot(chrome),
            "chrome not covered by the foreground texture must stay the Button fill, got {chrome:?}"
        );
        assert!(
            is_red_slot(foreground),
            "foreground HostTexture must cover chrome where they overlap, got {foreground:?}"
        );
        drop(layers);
    }

    #[test]
    fn geometry_and_text_keep_msaa_then_text_after_resolve() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let scene = labeled_selected_button_scene();
        let has_text = scene
            .primitives()
            .any(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Text { .. }));
        let has_quad = scene
            .primitives()
            .any(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Quad { .. }));
        assert!(
            has_text && has_quad,
            "Selected Button must emit fill + label"
        );
        let has_gpu = scene
            .primitives()
            .any(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom(_)));
        assert!(!has_gpu);
        let target = test_target(&device, format, 64, 64);
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui msaa then text"),
        });
        painter
            .paint(&scene, &mut encoder, &target, viewport, None, None)
            .unwrap();
        let counts = painter
            .last_dest_pass_counts
            .expect("encoded frame records dest passes");
        assert_eq!(
            counts.msaa, 1,
            "pure UI may keep 4x MSAA for Quad/Mesh, got {counts:?}"
        );
        assert!(
            counts.msaa_allocated,
            "pure UI dest must allocate 4x MSAA, got {counts:?}"
        );
        assert_eq!(
            counts.color, 1,
            "Text must paint after MSAA resolve with Load, got {counts:?}"
        );
        assert_eq!(
            counts.blit, 1,
            "dest→window blit is one extra pass, got {counts:?}"
        );
        queue.submit([encoder.finish()]);
    }

    #[test]
    fn paint_accepts_rotation_and_letter_spacing() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(
                document,
                RuntimeButton::new("Hi")
                    .kind(ButtonKind::Selected)
                    .layout(square_button_layout()),
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        write_box(&mut layout, button.stable_id(), 8.0, 16.0, 48.0, 32.0);
        let mut style = nana_ui_runtime::NodeStyle {
            layout: square_button_layout(),
            ..Default::default()
        };
        Arc::make_mut(&mut style.layout).transform = Some(nana_ui_core::PaintTransform {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        });
        Arc::make_mut(&mut style.layout).letter_spacing = Some(0.5);
        Arc::make_mut(&mut style.layout).font_family = Some("Noto Sans SC".into());
        layout.set_style(button.stable_id(), style);
        context.commit_mutations(layout).unwrap();
        let scene = commit_scene(&mut context);
        validate_scene(&scene, None, None).expect("rotation and tracking must validate");
        let target = test_target(&device, format, 64, 64);
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui affine tracking paint"),
        });
        painter
            .paint(&scene, &mut encoder, &target, viewport, None, None)
            .expect("supported affine and tracking must paint");
        queue.submit([encoder.finish()]);
    }

    #[test]
    fn letter_spacing_changes_painted_pixels() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let tight = labeled_selected_button_scene();
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(
                document,
                RuntimeButton::new("Hi")
                    .kind(ButtonKind::Selected)
                    .layout(square_button_layout()),
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        write_box(&mut layout, button.stable_id(), 8.0, 16.0, 48.0, 32.0);
        let mut style = nana_ui_runtime::NodeStyle {
            layout: square_button_layout(),
            ..Default::default()
        };
        Arc::make_mut(&mut style.layout).letter_spacing = Some(8.0);
        layout.set_style(button.stable_id(), style);
        context.commit_mutations(layout).unwrap();
        let tracked = commit_scene(&mut context);
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let paint = |painter: &mut SceneWgpuPainter, scene: &UiScene| -> Vec<u8> {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nana-ui tracking paint"),
            });
            painter
                .paint(scene, &mut encoder, &view, viewport, None, None)
                .unwrap();
            readback_rgba(&device, &queue, encoder, &texture, 64, 64)
        };
        let first = paint(&mut painter, &tight);
        let second = paint(&mut painter, &tracked);
        assert_ne!(
            first, second,
            "8px letter-spacing must change painted pixels versus default tracking"
        );
        drop(texture);
    }

    #[test]
    fn host_texture_rounded_clip_matches_sibling_quad_not_fitted_dest() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let preview = context
            .create_component(
                document,
                GpuTextureView::new("layer")
                    .with_corner_radius(32.0)
                    .contain(),
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        write_box(&mut layout, preview.stable_id(), 0.0, 0.0, 64.0, 64.0);
        context.commit_mutations(layout).unwrap();
        let scene = commit_scene(&mut context);
        let view = solid_texture_view(&device, &queue, format, 64, 32, wgpu::Color::GREEN);
        let registry = register_host_texture("layer", &view, 64, 32);
        let (target, target_view) = test_copy_target(&device, format, 64, 64);
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui host texture rounded clip"),
        });
        painter
            .paint(
                &scene,
                &mut encoder,
                &target_view,
                viewport,
                Some(&registry),
                None,
            )
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &target, 64, 64);
        let center = pixel(&pixels, 64, 32, 32);
        let dest_corner = pixel(&pixels, 64, 2, 18);
        assert!(
            is_green_slot(center),
            "contain dest center must stay inside the sibling Quad circle, got {center:?}"
        );
        assert!(
            dest_corner[1] < 40,
            "letterboxed dest corner is outside the 32px node circle and must not round the dest, got {dest_corner:?}"
        );
        drop(view);
    }

    fn hosted_preview_scene(device: &wgpu::Device) -> (UiScene, HostTextureRegistry) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, RuntimeButton::new("刷新预览"))
            .unwrap();
        let preview = context
            .create_component(document, nana_ui_runtime::GpuTextureView::new("preview"))
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 8.0,
                y: 8.0,
                width: 112.0,
                height: 28.0,
            },
        );
        layout.write_layout(
            preview.stable_id(),
            LayoutBox {
                x: 8.0,
                y: 40.0,
                width: 112.0,
                height: 48.0,
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
        let texture = crate::HostTexture::from_wgpu(
            1,
            1,
            test_target(device, wgpu::TextureFormat::Rgba8Unorm, 64, 48),
        );
        let registry = HostTextureRegistry::new();
        registry.register(
            "preview",
            texture,
            64,
            48,
            crate::HostTextureAlphaMode::Premultiplied,
        );
        (scene, registry)
    }

    fn runtime_button_over_host_texture_scene(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> (UiScene, HostTextureRegistry, wgpu::TextureView, [f32; 4]) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let background = context
            .create_component(
                document,
                RuntimeButton::new("").style(opaque_fill_style(SemanticColorRole::Danger)),
            )
            .unwrap();
        let preview = context
            .create_component(document, GpuTextureView::new("layer"))
            .unwrap();
        let overlay = context
            .create_component(
                document,
                RuntimeButton::new("")
                    .kind(ButtonKind::Selected)
                    .layout(square_button_layout()),
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        write_box(&mut layout, background.stable_id(), 0.0, 0.0, 64.0, 64.0);
        write_box(&mut layout, preview.stable_id(), 0.0, 16.0, 64.0, 32.0);
        write_box(&mut layout, overlay.stable_id(), 8.0, 28.0, 48.0, 24.0);
        context.commit_mutations(layout).unwrap();
        let scene = commit_scene(&mut context);
        let overlay_fill = selected_button_fill(&context, overlay.stable_id());
        let view = solid_texture_view(device, queue, format, 64, 32, wgpu::Color::GREEN);
        let registry = register_host_texture("layer", &view, 64, 32);
        (scene, registry, view, overlay_fill)
    }

    fn two_host_texture_layers_with_chrome_scene(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> (UiScene, HostTextureRegistry, [wgpu::TextureView; 2]) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let background = context
            .create_component(document, GpuTextureView::new("bg"))
            .unwrap();
        let chrome = context
            .create_component(
                document,
                RuntimeButton::new("")
                    .kind(ButtonKind::Selected)
                    .layout(square_button_layout()),
            )
            .unwrap();
        let foreground = context
            .create_component(document, GpuTextureView::new("fg"))
            .unwrap();
        let mut layout = MutationQueue::new();
        write_box(&mut layout, background.stable_id(), 0.0, 0.0, 64.0, 64.0);
        write_box(&mut layout, chrome.stable_id(), 8.0, 16.0, 48.0, 24.0);
        write_box(&mut layout, foreground.stable_id(), 16.0, 28.0, 32.0, 28.0);
        context.commit_mutations(layout).unwrap();
        let scene = commit_scene(&mut context);
        let fill = selected_button_fill(&context, chrome.stable_id());
        assert!(
            fill[3] > 0.99,
            "Selected chrome Button fill must be opaque, got {fill:?}"
        );
        let bg = solid_texture_view(device, queue, format, 64, 64, wgpu::Color::BLUE);
        let fg = solid_texture_view(device, queue, format, 32, 28, wgpu::Color::RED);
        let registry = HostTextureRegistry::new();
        register_into(&registry, "bg", &bg, 64, 64);
        register_into(&registry, "fg", &fg, 32, 28);
        (scene, registry, [bg, fg])
    }

    /// A labeled button under a rotation, so its text takes the affine glyph
    /// path rather than the cryoglyph atlas path.
    fn rotated_label_scene() -> UiScene {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let button = context
            .create_component(
                document,
                RuntimeButton::new("Hi")
                    .kind(ButtonKind::Selected)
                    .layout(Arc::new(nana_ui_core::LayoutStyle {
                        transform: Some(PaintTransform {
                            a: k,
                            b: k,
                            c: -k,
                            d: k,
                            ..PaintTransform::default()
                        }),
                        ..(*square_button_layout()).clone()
                    })),
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        write_box(&mut layout, button.stable_id(), 8.0, 16.0, 48.0, 32.0);
        context.commit_mutations(layout).unwrap();
        commit_scene(&mut context)
    }

    #[test]
    fn affine_text_reuses_gpu_resources_across_repaints_with_identical_pixels() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let scene = rotated_label_scene();
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);

        let paint_once = |painter: &mut SceneWgpuPainter, scene: &UiScene| -> Vec<u8> {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nana-ui affine cache test"),
            });
            painter
                .paint(scene, &mut encoder, &view, viewport, None, None)
                .unwrap();
            queue.submit([encoder.finish()]);
            readback_rgba(
                &device,
                &queue,
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nana-ui affine cache readback"),
                }),
                &texture,
                64,
                64,
            )
        };

        let first = paint_once(&mut painter, &scene);
        let (hits, misses, _) = painter.affine_text_cache_stats();
        assert!(
            misses > 0,
            "rotated label must take the affine glyph path at least once"
        );
        assert_eq!(hits, 0);

        // Each miss creates an atlas texture, a bind group and a vertex buffer.
        // Repainting an unchanged rotated label must create none of them. Clones
        // carry the same label under a fresh instance so every frame is
        // rebatched and the affine cache is what has to absorb it.
        for _ in 0..4 {
            let repaint = paint_once(&mut painter, &scene.clone());
            assert_eq!(
                first, repaint,
                "reused affine GPU resources must produce identical pixels"
            );
        }
        let (hits, misses_after, evictions) = painter.affine_text_cache_stats();
        assert_eq!(
            misses_after, misses,
            "static affine text must not recreate GPU resources per frame"
        );
        assert_eq!(hits, misses * 4);
        assert_eq!(evictions, 0);
    }

    fn labeled_selected_button_scene() -> UiScene {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(
                document,
                RuntimeButton::new("Hi")
                    .kind(ButtonKind::Selected)
                    .layout(square_button_layout()),
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        write_box(&mut layout, button.stable_id(), 8.0, 16.0, 48.0, 32.0);
        context.commit_mutations(layout).unwrap();
        commit_scene(&mut context)
    }

    fn nested_rotated_overflow_probe(clips: &[ClipRegion]) -> (u32, u32) {
        let origin = super::clip::paint_origin([0.0, 0.0], [0.0, 0.0]);
        let aabb = super::clip::intersect_clips(
            super::clip::LogicalRect::viewport([0.0, 0.0], [64.0, 64.0]),
            clips,
            origin,
        )
        .unwrap();
        let rotated = super::clip::rotated_fragment_clips(clips, origin);
        let inner = *rotated.last().expect("nested probe needs a rotated clip");
        for y in 0..64u32 {
            for x in 0..64u32 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                if px >= aabb.x
                    && py >= aabb.y
                    && px < aabb.x + aabb.width
                    && py < aabb.y + aabb.height
                    && super::clip::point_in_fragment_clip(px, py, inner)
                    && !super::clip::point_in_fragment_clips(px, py, &rotated)
                {
                    return (x, y);
                }
            }
        }
        panic!(
            "nested clips must include a pixel inside the inner parallelogram but outside the outer"
        );
    }

    fn aabb_outside_rotated_overflow_probe() -> (u32, u32) {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let clips = [ClipRegion {
            bounds: SceneRect {
                x: 16.0,
                y: 16.0,
                width: 32.0,
                height: 32.0,
            },
            transform: AffineTransform::from_matrix(
                PaintTransform {
                    a: k,
                    b: k,
                    c: -k,
                    d: k,
                    ..PaintTransform::default()
                }
                .around_center(16.0, 16.0, 32.0, 32.0),
            ),
            corner_radius: 0.0,
            polygon_clip: None,
        }];
        let origin = super::clip::paint_origin([0.0, 0.0], [0.0, 0.0]);
        let aabb = super::clip::intersect_clips(
            super::clip::LogicalRect::viewport([0.0, 0.0], [64.0, 64.0]),
            &clips,
            origin,
        )
        .unwrap();
        let frag = super::clip::fragment_clip(&clips, origin);
        for y in 8..24 {
            for x in 8..24 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                if px >= aabb.x
                    && py >= aabb.y
                    && px < aabb.x + aabb.width
                    && py < aabb.y + aabb.height
                    && !super::clip::point_in_fragment_clip(px, py, frag)
                {
                    return (x, y);
                }
            }
        }
        panic!(
            "sibling must overlap a pixel inside the rotated AABB but outside the parallelogram"
        );
    }

    fn rotated_overflow_parent(
        value: u64,
        children: &[u64],
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> ExtractedNode {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        overflow_parent(
            value,
            children,
            x,
            y,
            width,
            height,
            Some(PaintTransform {
                a: k,
                b: k,
                c: -k,
                d: k,
                ..PaintTransform::default()
            }),
        )
    }

    fn overflow_parent(
        value: u64,
        children: &[u64],
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        transform: Option<PaintTransform>,
    ) -> ExtractedNode {
        ExtractedNode {
            id: StableNodeId::new(value).unwrap(),
            kind: Arc::new(NodeKind::Element { tag: "div".into() }),
            parent: None,
            children: Arc::new(
                children
                    .iter()
                    .copied()
                    .map(|child| StableNodeId::new(child).unwrap())
                    .collect(),
            ),
            layout: LayoutBox {
                x,
                y,
                width,
                height,
            },
            scroll_offset: nana_ui_runtime::ScrollOffset::default(),
            source_style: NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    overflow_x: OverflowSpec::Hidden,
                    overflow_y: OverflowSpec::Hidden,
                    transform,
                    ..nana_ui_core::LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
            style: Arc::new(ComputedStyle::default()),
            text: None,
            text_metrics: None,
            z_index: 0,
            focused: false,
            ime: None,
            text_input: None,
            text_spans: Vec::new(),
            standard_visual: None,
            component_geometry: None,
            standard_visual_foreground: None,
            custom_render: None,
        }
    }

    fn translucent_parent(
        value: u64,
        children: &[u64],
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        opacity: f32,
    ) -> ExtractedNode {
        ExtractedNode {
            id: StableNodeId::new(value).unwrap(),
            kind: Arc::new(NodeKind::Element { tag: "div".into() }),
            parent: None,
            children: Arc::new(
                children
                    .iter()
                    .copied()
                    .map(|child| StableNodeId::new(child).unwrap())
                    .collect(),
            ),
            layout: LayoutBox {
                x,
                y,
                width,
                height,
            },
            scroll_offset: nana_ui_runtime::ScrollOffset::default(),
            source_style: NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    opacity: Some(opacity),
                    ..nana_ui_core::LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
            style: Arc::new(ComputedStyle::default()),
            text: None,
            text_metrics: None,
            z_index: 0,
            focused: false,
            ime: None,
            text_input: None,
            text_spans: Vec::new(),
            standard_visual: None,
            component_geometry: None,
            standard_visual_foreground: None,
            custom_render: None,
        }
    }

    fn overflowing_text_child(
        value: u64,
        parent: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    ) -> ExtractedNode {
        ExtractedNode {
            id: StableNodeId::new(value).unwrap(),
            kind: Arc::new(NodeKind::Text),
            parent: Some(StableNodeId::new(parent).unwrap()),
            children: Arc::new(Vec::new()),
            layout: LayoutBox {
                x,
                y,
                width,
                height,
            },
            scroll_offset: nana_ui_runtime::ScrollOffset::default(),
            source_style: NodeStyle::default(),
            style: Arc::new(ComputedStyle {
                color: Some(color),
                font_size: 64.0,
                ..ComputedStyle::default()
            }),
            text: Some(TextContent {
                value: "██".into()
            }),
            text_metrics: None,
            z_index: 0,
            focused: false,
            ime: None,
            text_input: None,
            text_spans: Vec::new(),
            standard_visual: None,
            component_geometry: None,
            standard_visual_foreground: None,
            custom_render: None,
        }
    }

    fn custom_render_child(
        value: u64,
        parent: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        renderer: &str,
    ) -> ExtractedNode {
        let mut node = host_texture_child(value, parent, x, y, width, height, "slot");
        node.custom_render = Some(CustomRenderNode::new(renderer, "slot", 1));
        node
    }

    fn host_texture_child(
        value: u64,
        parent: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        resource: &str,
    ) -> ExtractedNode {
        ExtractedNode {
            id: StableNodeId::new(value).unwrap(),
            kind: Arc::new(NodeKind::Element { tag: "div".into() }),
            parent: Some(StableNodeId::new(parent).unwrap()),
            children: Arc::new(Vec::new()),
            layout: LayoutBox {
                x,
                y,
                width,
                height,
            },
            scroll_offset: nana_ui_runtime::ScrollOffset::default(),
            source_style: NodeStyle::default(),
            style: Arc::new(ComputedStyle::default()),
            text: None,
            text_metrics: None,
            z_index: 0,
            focused: false,
            ime: None,
            text_input: None,
            text_spans: Vec::new(),
            standard_visual: None,
            component_geometry: None,
            standard_visual_foreground: None,
            custom_render: Some(CustomRenderNode::new("nana.host-texture", resource, 1)),
        }
    }

    fn colored_quad_child(
        value: u64,
        parent: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    ) -> ExtractedNode {
        let mut node = colored_quad_node(value, x, y, width, height, color);
        node.parent = Some(StableNodeId::new(parent).unwrap());
        node
    }

    fn extracted_div(
        value: u64,
        children: &[u64],
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        layout: nana_ui_core::LayoutStyle,
        background: Option<[f32; 4]>,
    ) -> ExtractedNode {
        ExtractedNode {
            id: StableNodeId::new(value).unwrap(),
            kind: Arc::new(NodeKind::Element { tag: "div".into() }),
            parent: None,
            children: Arc::new(
                children
                    .iter()
                    .copied()
                    .map(|child| StableNodeId::new(child).unwrap())
                    .collect(),
            ),
            layout: LayoutBox {
                x,
                y,
                width,
                height,
            },
            scroll_offset: nana_ui_runtime::ScrollOffset::default(),
            source_style: NodeStyle {
                layout: Arc::new(layout),
                ..NodeStyle::default()
            },
            style: Arc::new(ComputedStyle {
                background,
                ..ComputedStyle::default()
            }),
            text: None,
            text_metrics: None,
            z_index: 0,
            focused: false,
            ime: None,
            text_input: None,
            text_spans: Vec::new(),
            standard_visual: None,
            component_geometry: None,
            standard_visual_foreground: None,
            custom_render: None,
        }
    }

    fn colored_quad_node(
        value: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    ) -> ExtractedNode {
        extracted_div(
            value,
            &[],
            x,
            y,
            width,
            height,
            nana_ui_core::LayoutStyle {
                background: Some(color),
                ..nana_ui_core::LayoutStyle::default()
            },
            Some(color),
        )
    }

    fn commit_scene(context: &mut AppContext) -> UiScene {
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        context
            .shape_text(&work.text, &mut crate::NanaTextShaper::default())
            .unwrap();
        let mut scene = UiScene::new();
        scene.apply_delta(
            context.world().extract_nodes(&work.render_extraction),
            work.render_removals,
        );
        scene
    }

    fn selected_button_fill(context: &AppContext, id: nana_ui_runtime::StableNodeId) -> [f32; 4] {
        let work = context.world().extract_nodes(&[id]);
        match work[0].component_geometry.as_ref() {
            Some(ComponentGeometry::Button {
                background: Some(fill),
                ..
            }) => *fill,
            other => panic!("expected opaque Button geometry fill, got {other:?}"),
        }
    }

    fn write_box(
        queue: &mut MutationQueue,
        id: nana_ui_runtime::StableNodeId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) {
        queue.write_layout(
            id,
            LayoutBox {
                x,
                y,
                width,
                height,
            },
        );
    }

    fn square_button_layout() -> Arc<nana_ui_core::LayoutStyle> {
        Arc::new(nana_ui_core::LayoutStyle {
            border_radius: Some(0.0),
            border_width: Some(0.0),
            padding_left: Some(LengthSpec::Px(0.0)),
            padding_right: Some(LengthSpec::Px(0.0)),
            padding_top: Some(LengthSpec::Px(0.0)),
            padding_bottom: Some(LengthSpec::Px(0.0)),
            min_height: Some(LengthSpec::Px(0.0)),
            ..nana_ui_core::LayoutStyle::default()
        })
    }

    fn opaque_fill_style(background: SemanticColorRole) -> NodeStyle {
        NodeStyle {
            layout: square_button_layout(),
            background: Some(background),
            ..NodeStyle::default()
        }
    }

    fn register_host_texture(
        resource: &str,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> HostTextureRegistry {
        let registry = HostTextureRegistry::new();
        register_into(&registry, resource, view, width, height);
        registry
    }

    fn register_into(
        registry: &HostTextureRegistry,
        resource: &str,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        registry.register(
            resource,
            crate::HostTexture::from_wgpu(1, 1, view.clone()),
            width,
            height,
            crate::HostTextureAlphaMode::Premultiplied,
        );
    }

    fn assert_mixed_gpu_pass_counts(painter: &SceneWgpuPainter) {
        let counts = painter
            .last_dest_pass_counts
            .expect("encoded frame records dest passes");
        assert_eq!(
            counts.color, 1,
            "HostTexture composition must stay in one dest color pass, not 1+N textures, got {counts:?}"
        );
        assert_eq!(
            counts.msaa, 0,
            "mixed GPU frames must not open an MSAA pass, got {counts:?}"
        );
        assert!(
            !counts.msaa_allocated,
            "mixed GPU frames must not allocate unused 4x MSAA, got {counts:?}"
        );
        assert_eq!(
            counts.blit, 1,
            "dest→window blit may be one extra pass, got {counts:?}"
        );
    }

    fn solid_texture_view(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        color: wgpu::Color,
    ) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui scene order fill"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui scene order fill"),
        });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana-ui scene order fill"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        queue.submit([encoder.finish()]);
        view
    }

    fn test_copy_target(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui scene order target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn readback_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let unpadded = width as usize * 4;
        let padded = unpadded.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-ui scene order readback"),
            size: (padded * height as usize) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .expect("document-order readback poll");
        let mapped = slice
            .get_mapped_range()
            .expect("document-order readback must be mapped");
        let mut pixels = Vec::with_capacity(unpadded * height as usize);
        for row in mapped.chunks_exact(padded) {
            pixels.extend_from_slice(&row[..unpadded]);
        }
        pixels
    }

    fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * width + x) * 4) as usize;
        [
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        ]
    }

    fn is_green_slot(color: [u8; 4]) -> bool {
        color[1] > 180 && color[0] < 40 && color[2] < 40
    }

    fn is_blue_slot(color: [u8; 4]) -> bool {
        color[2] > 180 && color[0] < 40 && color[1] < 40
    }

    fn is_red_slot(color: [u8; 4]) -> bool {
        color[0] > 180 && color[1] < 40 && color[2] < 40
    }

    fn is_danger_fill(color: [u8; 4]) -> bool {
        color[0] > 150 && color[1] < 80 && color[2] < 80
    }

    fn test_target(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("nana-ui scene gpu work target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn test_device() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .expect("scene GPU work test requires a WGPU adapter");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nana-ui scene GPU work test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("scene GPU work test requires a WGPU device")
    }

    fn paint_surface_quad_node(
        value: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        background: [f32; 4],
        surface: nana_ui_scene::QuadSurfacePaint,
    ) -> ExtractedNode {
        extracted_div(
            value,
            &[],
            x,
            y,
            width,
            height,
            nana_ui_core::LayoutStyle {
                background: Some(background),
                paint: nana_ui_core::PaintStyle {
                    background_image: surface.background_image.clone(),
                    background_layers: surface.background_layers.clone(),
                    content_image: surface.content_image.clone(),
                    mask: surface.mask.clone(),
                    border_image: surface.border_image.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(background),
        )
    }

    fn frost_quad_node_with_fill(
        value: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: [f32; 4],
        filter: nana_ui_core::BackdropFilter,
    ) -> ExtractedNode {
        extracted_div(
            value,
            &[],
            x,
            y,
            width,
            height,
            nana_ui_core::LayoutStyle {
                background: Some(fill),
                paint: nana_ui_core::PaintStyle {
                    backdrop_filter: Some(filter),
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(fill),
        )
    }

    fn clip_path_inset_round_parent(
        value: u64,
        children: &[u64],
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        inset_px: f32,
        round_px: f32,
    ) -> ExtractedNode {
        extracted_div(
            value,
            children,
            x,
            y,
            width,
            height,
            nana_ui_core::LayoutStyle {
                border_radius: Some(0.0),
                paint: nana_ui_core::PaintStyle {
                    clip_path: Some(nana_ui_core::ClipPath::Inset(nana_ui_core::ClipInset {
                        top: LengthSpec::Px(inset_px),
                        right: LengthSpec::Px(inset_px),
                        bottom: LengthSpec::Px(inset_px),
                        left: LengthSpec::Px(inset_px),
                        round: Some(LengthSpec::Px(round_px)),
                    })),
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
        )
    }

    fn clip_path_circle_parent(
        value: u64,
        children: &[u64],
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> ExtractedNode {
        extracted_div(
            value,
            children,
            x,
            y,
            width,
            height,
            nana_ui_core::LayoutStyle {
                paint: nana_ui_core::PaintStyle {
                    clip_path: Some(nana_ui_core::ClipPath::Circle(nana_ui_core::ClipCircle {
                        radius: nana_ui_core::ClipShapeRadius::ClosestSide,
                        cx: LengthSpec::Percent(50.0),
                        cy: LengthSpec::Percent(50.0),
                    })),
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
        )
    }

    fn clip_path_polygon_parent(
        value: u64,
        children: &[u64],
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> ExtractedNode {
        extracted_div(
            value,
            children,
            x,
            y,
            width,
            height,
            nana_ui_core::LayoutStyle {
                paint: nana_ui_core::PaintStyle {
                    clip_path: Some(nana_ui_core::ClipPath::Polygon(vec![
                        nana_ui_core::ClipPoint {
                            x: LengthSpec::Percent(0.0),
                            y: LengthSpec::Percent(0.0),
                        },
                        nana_ui_core::ClipPoint {
                            x: LengthSpec::Percent(100.0),
                            y: LengthSpec::Percent(0.0),
                        },
                        nana_ui_core::ClipPoint {
                            x: LengthSpec::Percent(50.0),
                            y: LengthSpec::Percent(100.0),
                        },
                    ])),
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
        )
    }

    fn clip_path_inset_round_parent_rotated(
        value: u64,
        children: &[u64],
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        round_px: f32,
        transform: nana_ui_core::PaintTransform,
    ) -> ExtractedNode {
        let mut node =
            clip_path_inset_round_parent(value, children, x, y, width, height, 0.0, round_px);
        Arc::make_mut(&mut node.source_style.layout).transform = Some(transform);
        node
    }

    fn frost_quad_child(
        value: u64,
        parent: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: [f32; 4],
        filter: nana_ui_core::BackdropFilter,
    ) -> ExtractedNode {
        let mut node = frost_quad_node_with_fill(value, x, y, width, height, fill, filter);
        node.parent = Some(StableNodeId::new(parent).unwrap());
        node
    }

    fn frost_quad_node_with_fill_and_transform(
        value: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: [f32; 4],
        filter: nana_ui_core::BackdropFilter,
        transform: nana_ui_core::PaintTransform,
    ) -> ExtractedNode {
        let mut node = frost_quad_node_with_fill(value, x, y, width, height, fill, filter);
        Arc::make_mut(&mut node.source_style.layout).transform = Some(transform);
        node
    }

    #[test]
    fn backdrop_blurs_content_behind_frost_panel() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let frost = nana_ui_core::BackdropFilter {
            blur_radius: 8.0,
            saturate: 1.0,
        };
        scene.apply_delta(
            [
                colored_quad_node(1, 0.0, 0.0, 64.0, 128.0, [1.0, 0.0, 0.0, 1.0]),
                colored_quad_node(2, 64.0, 0.0, 192.0, 128.0, [0.0, 1.0, 0.0, 1.0]),
                frost_quad_node_with_fill(3, 48.0, 48.0, 32.0, 32.0, [0.0, 0.0, 0.0, 0.0], frost),
            ],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [256.0, 128.0],
            physical_size: [256, 128],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 256, 128);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui frost blur split"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 256, 128);
        let center = pixel(&pixels, 256, 64, 64);
        assert!(
            center[0] > 40 && center[1] > 40,
            "frost center on red/green split must blend both channels, got {center:?}"
        );
        let dest_center = pixel(&pixels, 256, 128, 64);
        assert!(
            dest_center[1] > 180 && dest_center[0] < 40,
            "dest-center UV without frost must stay pure green, got {dest_center:?}"
        );
        drop(texture);
    }

    #[test]
    fn backdrop_two_frost_panels_use_independent_regions() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let desaturate = nana_ui_core::BackdropFilter {
            blur_radius: 4.0,
            saturate: 0.0,
        };
        scene.apply_delta(
            [
                colored_quad_node(1, 0.0, 0.0, 128.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
                colored_quad_node(2, 128.0, 0.0, 128.0, 64.0, [0.0, 1.0, 0.0, 1.0]),
                frost_quad_node_with_fill(
                    3,
                    16.0,
                    16.0,
                    32.0,
                    32.0,
                    [0.0, 0.0, 0.0, 0.0],
                    desaturate,
                ),
                frost_quad_node_with_fill(
                    4,
                    208.0,
                    16.0,
                    32.0,
                    32.0,
                    [0.0, 0.0, 0.0, 0.0],
                    desaturate,
                ),
            ],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [256.0, 64.0],
            physical_size: [256, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 256, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui frost independent regions"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 256, 64);
        let left = pixel(&pixels, 256, 32, 32);
        let right = pixel(&pixels, 256, 224, 32);
        assert!(
            left[1] > 30 && left[2] > 30 && left[0] < 120,
            "left frost over red with saturate(0) must gray out, got {left:?}"
        );
        assert!(
            right[0] > 100 && right[2] > 100 && right[1] > 150,
            "right frost over green with saturate(0) must stay green-weighted, got {right:?}"
        );
        assert!(
            left[0] + 40 < right[0],
            "independent frost regions must not last-write-wins, left={left:?} right={right:?}"
        );
        drop(texture);
    }

    #[test]
    fn backdrop_first_dest_command_survives_transparent_quad() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let desaturate = nana_ui_core::BackdropFilter {
            blur_radius: 6.0,
            saturate: 0.0,
        };
        scene.apply_delta(
            [
                frost_quad_node_with_fill(
                    1,
                    16.0,
                    16.0,
                    32.0,
                    32.0,
                    [0.0, 0.0, 0.0, 0.0],
                    desaturate,
                ),
                colored_quad_node(2, 0.0, 0.0, 64.0, 64.0, [0.0, 0.0, 0.0, 0.0]),
            ],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 1.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui frost survives transparent quad"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let center = pixel(&pixels, 64, 32, 32);
        assert!(
            center[2] < 80 && center[0] > 8 && center[1] > 8,
            "frost over clear blue must survive a later transparent quad, got {center:?}"
        );
        let rb_delta = center[0]
            .abs_diff(center[1])
            .max(center[1].abs_diff(center[2]));
        assert!(
            rb_delta < 40,
            "saturate(0) over blue must stay near-neutral, got {center:?}"
        );
        drop(texture);
    }

    #[test]
    fn backdrop_filter_forces_sample_count_one_dest_path() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [frost_quad_node_with_fill(
                1,
                8.0,
                8.0,
                48.0,
                48.0,
                [1.0, 1.0, 1.0, 0.35],
                nana_ui_core::BackdropFilter {
                    blur_radius: 12.0,
                    saturate: 1.0,
                },
            )],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.1, 0.1, 0.1, 1.0],
            clear: true,
        };
        let view = test_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui frost dest path"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let counts = painter
            .last_dest_pass_counts
            .expect("frost frame records dest passes");
        assert_eq!(counts.msaa, 0, "backdrop frames must stay sample_count=1");
        assert!(
            counts.backdrop >= 3,
            "frost must encode copy+blur+composite passes, got {counts:?}"
        );
        drop(encoder);
    }

    #[test]
    fn inset_round_clip_corners_are_transparent_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                clip_path_inset_round_parent(1, &[2], 0.0, 0.0, 64.0, 64.0, 0.0, 32.0),
                colored_quad_child(2, 1, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
            ],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 1.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui inset round clip"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let corner = pixel(&pixels, 64, 2, 2);
        assert!(
            corner[2] > 180 && corner[0] < 80,
            "FragmentClip inset(0) round 32px must cut sharp child corners to clear blue; \
             AABB-only scissor would stay red, got {corner:?}"
        );
        let center = pixel(&pixels, 64, 32, 32);
        assert!(
            center[0] > 200 && center[1] < 40,
            "sharp red child interior must stay opaque red under rounded FragmentClip, got {center:?}"
        );
        drop(texture);
    }

    #[test]
    fn circle_clip_path_cuts_child_corners_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                clip_path_circle_parent(1, &[2], 0.0, 0.0, 64.0, 64.0),
                colored_quad_child(2, 1, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
            ],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 1.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui circle clip"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let corner = pixel(&pixels, 64, 2, 2);
        assert!(
            corner[2] > 180 && corner[0] < 80,
            "circle() must clip square corners to clear, not a silent AABB, got {corner:?}"
        );
        let center = pixel(&pixels, 64, 32, 32);
        assert!(
            center[0] > 200 && center[1] < 40,
            "circle interior must stay child-red, got {center:?}"
        );
        drop(texture);
    }

    #[test]
    fn polygon_clip_path_cuts_child_corners_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                clip_path_polygon_parent(1, &[2], 0.0, 0.0, 64.0, 64.0),
                colored_quad_child(2, 1, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
            ],
            [],
        );
        let child = scene
            .primitive(nana_ui_scene::PrimitiveId {
                node: StableNodeId::new(2).unwrap(),
                slot: 0,
            })
            .expect("child quad");
        assert!(
            child
                .clips
                .iter()
                .any(|clip| clip.polygon_clip.as_ref().is_some_and(|p| p.len() >= 3)),
            "ancestor polygon clip must reach child primitive clips"
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 1.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui polygon ancestor clip"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let lower_left = pixel(&pixels, 64, 4, 60);
        assert!(
            lower_left[0] < 80,
            "polygon clip must cut child fill outside the triangle; AABB-only would stay red at {lower_left:?}"
        );
        let center = pixel(&pixels, 64, 32, 24);
        assert!(
            center[0] > 200 && center[1] < 40,
            "triangle interior must stay opaque red, got {center:?}"
        );
        drop(texture);
    }

    #[test]
    fn perspective_rotate_y_paints_trapezoid_on_gpu() {
        let mat = nana_ui_core::PaintMat4::perspective(800.0)
            .expect("d")
            .then(nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians()));
        let x = 28.0;
        let y = 40.0;
        let width = 200.0;
        let height = 80.0;
        let corners = mat
            .around_origin(x, y, width * 0.5, height * 0.5)
            .projected_corners(x, y, width, height)
            .expect("corners");
        let mut node = colored_quad_node(1, x, y, width, height, [1.0, 0.0, 0.0, 1.0]);
        Arc::make_mut(&mut node.source_style.layout).transform_3d = Some(mat);
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta([node], []);
        let viewport = ScenePaintViewport {
            logical_size: [256.0, 160.0],
            physical_size: [256, 160],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 256, 160);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui perspective rotateY trapezoid"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .expect("planar 3D must paint in the existing quad pass");
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 256, 160);
        let pivoted = mat.around_origin(x, y, width * 0.5, height * 0.5);
        let left_pt = pivoted
            .project_xy(x + width * 0.2, y + height * 0.5)
            .expect("left interior");
        let right_pt = pivoted
            .project_xy(x + width * 0.8, y + height * 0.5)
            .expect("right interior");
        let left_x = left_pt[0].round().clamp(1.0, 254.0) as u32;
        let right_x = right_pt[0].round().clamp(1.0, 254.0) as u32;
        let left_span = painted_column_span(&pixels, 256, 160, left_x);
        let right_span = painted_column_span(&pixels, 256, 160, right_x);
        assert!(
            (left_span as i32 - right_span as i32).abs() > 4,
            "GPU rotateY+perspective must paint a trapezoid, left_span={left_span} right_span={right_span} left_x={left_x} right_x={right_x} corners={corners:?}"
        );
        drop(texture);
    }

    fn painted_column_span(pixels: &[u8], width: u32, height: u32, x: u32) -> u32 {
        let mut min_y = height;
        let mut max_y = 0;
        let mut any = false;
        for y in 0..height {
            let color = pixel(pixels, width, x, y);
            if color[0] > 80 {
                any = true;
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
        assert!(any, "expected painted pixels in column {x}");
        max_y - min_y + 1
    }

    #[test]
    fn rotated_inset_round_clip_keeps_corner_radius_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let k = std::f32::consts::FRAC_1_SQRT_2;
        scene.apply_delta(
            [
                clip_path_inset_round_parent_rotated(
                    1,
                    &[2],
                    0.0,
                    0.0,
                    64.0,
                    64.0,
                    24.0,
                    nana_ui_core::PaintTransform {
                        a: k,
                        b: k,
                        c: -k,
                        d: k,
                        ..Default::default()
                    },
                ),
                colored_quad_child(2, 1, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
            ],
            [],
        );
        // around_center(rotate 45°, box 0,0,64,64), k = 1/sqrt(2):
        //   scene_x = k*lx - k*ly + 32
        //   scene_y = k*lx + k*ly + 32(1-sqrt(2))   // ~ -13.255
        // Diamond vertices ~ (32,-13.3), (77.3,32), (32,77.3), (-13.3,32) sit
        // outside a 64x64 origin-0 viewport. dest = scene - scene_origin, so a
        // 96x96 view at origin (-16,-16) puts the top vertex at dest ~ (48, 2.7).
        //
        // Probe dest (48, 8): pixel center (48.5, 8.5) -> scene (32.5, -7.5) ->
        // local ~ (4.42, 3.72). Inside the sharp [0,64]^2 diamond (r=0 stays
        // child-red) and outside the r=24 rounded-box SDF (dist to corner
        // circle (24,24) ~ 28.2 > 24 -> dest/clear blue). Viewport (2,2)
        // inverse-maps to local ~ (-10.4, 32), already outside the diamond.
        let viewport = ScenePaintViewport {
            logical_size: [96.0, 96.0],
            physical_size: [96, 96],
            scale_factor: 1.0,
            scene_origin: [-16.0, -16.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 1.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 96, 96);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui rotated inset round clip"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 96, 96);
        let cutlet = pixel(&pixels, 96, 48, 8);
        assert!(
            cutlet[2] > 180 && cutlet[0] < 80,
            "rotated inset(round) must SDF-round in the rotated frame, not zero radius; got {cutlet:?}"
        );
        let interior = pixel(&pixels, 96, 48, 48);
        assert!(
            interior[0] > 200 && interior[1] < 40,
            "diamond interior must stay child-red under rounded FragmentClip, got {interior:?}"
        );
        drop(texture);
    }

    #[test]
    fn host_texture_under_inset_round_ancestor_clips_corners() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                clip_path_inset_round_parent(1, &[2], 0.0, 0.0, 64.0, 64.0, 0.0, 32.0),
                host_texture_child(2, 1, 0.0, 0.0, 64.0, 64.0, "layer"),
            ],
            [],
        );
        let view = solid_texture_view(&device, &queue, format, 64, 64, wgpu::Color::GREEN);
        let registry = register_host_texture("layer", &view, 64, 64);
        let (texture, target_view) = test_copy_target(&device, format, 64, 64);
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 1.0, 1.0],
            clear: true,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui host texture inset round ancestor"),
        });
        painter
            .paint(
                &scene,
                &mut encoder,
                &target_view,
                viewport,
                Some(&registry),
                None,
            )
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let corner = pixel(&pixels, 64, 2, 2);
        assert!(
            corner[2] > 180 && corner[1] < 80,
            "HostTexture overflow clip must inherit ancestor inset(round) radius; got {corner:?}"
        );
        drop(texture);
        drop(view);
    }

    #[test]
    fn rotated_backdrop_filter_mixes_dest_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let frost = nana_ui_core::BackdropFilter {
            blur_radius: 8.0,
            saturate: 1.0,
        };
        scene.apply_delta(
            [
                colored_quad_node(1, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
                colored_quad_node(2, 64.0, 0.0, 64.0, 64.0, [0.0, 0.0, 1.0, 1.0]),
                frost_quad_node_with_fill_and_transform(
                    3,
                    48.0,
                    16.0,
                    32.0,
                    32.0,
                    [0.0, 0.0, 0.0, 0.0],
                    frost,
                    nana_ui_core::PaintTransform {
                        a: k,
                        b: k,
                        c: -k,
                        d: k,
                        ..Default::default()
                    },
                ),
            ],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [128.0, 64.0],
            physical_size: [128, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 128, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui rotated frost"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let counts = painter
            .last_dest_pass_counts
            .expect("rotated frost records dest passes");
        assert!(
            counts.backdrop >= 3,
            "rotated backdrop must encode copy+blur+composite, got {counts:?}"
        );
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 128, 64);
        let center = pixel(&pixels, 128, 64, 32);
        assert!(
            center[0] > 30 && center[2] > 30,
            "rotated frost centered on the red/blue edge must mix dest samples, got {center:?}"
        );
        // 45° around (64,32): diamond tips ≈ (64, 9.37), (86.63, 32), (64, 54.63),
        // (41.37, 32). Unrotated AABB is [48,80]×[16,48]. Dest (64, 12) sits
        // inside the diamond and outside that AABB; AABB composite would skip it.
        let tip = pixel(&pixels, 128, 64, 12);
        assert!(
            tip[0] > 20 && tip[2] > 20,
            "rotated frost must cover the diamond tip outside the unrotated AABB; \
             AABB composite would leave a pure dest color, got {tip:?}"
        );
        drop(texture);
    }

    #[test]
    fn frost_under_ancestor_polygon_clips_outside_triangle_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let frost = nana_ui_core::BackdropFilter {
            blur_radius: 8.0,
            saturate: 0.0,
        };
        scene.apply_delta(
            [
                colored_quad_node(1, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
                clip_path_polygon_parent(2, &[3], 0.0, 0.0, 64.0, 64.0),
                frost_quad_child(3, 2, 0.0, 0.0, 64.0, 64.0, [0.0, 0.0, 0.0, 0.0], frost),
            ],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 1.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui frost ancestor polygon"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let inside = pixel(&pixels, 64, 32, 24);
        assert!(
            inside[0] < 180 && (inside[0] as i16 - inside[1] as i16).abs() < 50,
            "frost saturate(0) must gray dest inside the ancestor triangle, got {inside:?}"
        );
        let outside = pixel(&pixels, 64, 4, 60);
        assert!(
            outside[0] > 200 && outside[1] < 80,
            "ancestor polygon must clip frost; AABB-only composite would desaturate {outside:?}"
        );
        drop(texture);
    }

    #[test]
    fn gradient_white_to_transparent_source_over_red() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let gradient_surface = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                    angle_deg: 180.0,
                    stops: vec![
                        nana_ui_core::GradientStop {
                            position: 0.0,
                            color: [1.0, 1.0, 1.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 1.0,
                            color: [1.0, 1.0, 1.0, 0.0],
                        },
                    ],
                }),
            )),
            ..Default::default()
        };
        scene.apply_delta(
            [
                colored_quad_node(1, 0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
                paint_surface_quad_node(
                    2,
                    0.0,
                    0.0,
                    64.0,
                    64.0,
                    [0.0, 0.0, 0.0, 0.0],
                    gradient_surface,
                ),
            ],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui gradient source-over"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let center = pixel(&pixels, 64, 32, 32);
        assert!(
            center[0] > 200 && center[1] < 180 && center[2] < 180,
            "white→transparent gradient must source-over red (pink), not replace with black {center:?}"
        );
        drop(texture);
    }

    #[test]
    fn mask_linear_fade_scales_rgb_with_alpha() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        let masked_surface = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                    angle_deg: 0.0,
                    stops: vec![nana_ui_core::GradientStop {
                        position: 0.0,
                        color: [1.0, 0.0, 0.0, 1.0],
                    }],
                }),
            )),
            mask: Some(nana_ui_core::MaskImage::Gradient(
                nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                    angle_deg: 90.0,
                    stops: vec![
                        nana_ui_core::GradientStop {
                            position: 0.0,
                            color: [1.0, 1.0, 1.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 1.0,
                            color: [1.0, 1.0, 1.0, 0.0],
                        },
                    ],
                }),
            )),
            ..Default::default()
        };
        scene.apply_delta(
            [paint_surface_quad_node(
                1,
                0.0,
                0.0,
                64.0,
                64.0,
                [1.0, 0.0, 0.0, 1.0],
                masked_surface,
            )],
            [],
        );
        let primitive = scene
            .primitive(nana_ui_scene::PrimitiveId {
                node: StableNodeId::new(1).unwrap(),
                slot: 0,
            })
            .expect("masked quad primitive");
        match &primitive.kind {
            nana_ui_scene::ScenePrimitiveKind::Quad { surface, .. } => {
                assert!(
                    surface.mask.is_some(),
                    "mask must travel on quad surface {surface:?}"
                );
            }
            other => panic!("expected quad primitive, got {other:?}"),
        }
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 1.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui mask fade"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let left = pixel(&pixels, 64, 4, 32);
        let right = pixel(&pixels, 64, 56, 32);
        assert!(
            left[0] > 200 && left[1] < 40 && left[2] < 40,
            "masked quad left must stay red {left:?}"
        );
        assert!(
            right[2] > 180 && right[0] < 80,
            "mask fade must reveal clear blue, not opaque red {right:?}"
        );
        drop(texture);
    }

    #[test]
    fn mask_linear_six_stops_uses_stop_five_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let masked_surface = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                    angle_deg: 0.0,
                    stops: vec![nana_ui_core::GradientStop {
                        position: 0.0,
                        color: [1.0, 0.0, 0.0, 1.0],
                    }],
                }),
            )),
            mask: Some(nana_ui_core::MaskImage::Gradient(
                nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                    angle_deg: 180.0,
                    stops: vec![
                        nana_ui_core::GradientStop {
                            position: 0.0,
                            color: [1.0, 1.0, 1.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 0.2,
                            color: [1.0, 1.0, 1.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 0.4,
                            color: [1.0, 1.0, 1.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 0.6,
                            color: [1.0, 1.0, 1.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 0.8,
                            color: [1.0, 1.0, 1.0, 0.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 1.0,
                            color: [1.0, 1.0, 1.0, 0.0],
                        },
                    ],
                }),
            )),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta(
            [paint_surface_quad_node(
                1,
                0.0,
                0.0,
                64.0,
                64.0,
                [1.0, 0.0, 0.0, 1.0],
                masked_surface,
            )],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 1.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui six-stop mask"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let upper = pixel(&pixels, 64, 32, 8);
        let lower = pixel(&pixels, 64, 32, 56);
        assert!(
            upper[0] > 200 && upper[1] < 40 && upper[2] < 40,
            "top must stay unmasked red before stop 5, got {upper:?}"
        );
        assert!(
            lower[2] > 180 && lower[0] < 80,
            "bottom must hide via stop 5+ mask alpha, not stay red {lower:?}"
        );
        drop(texture);
    }

    #[test]
    fn radial_gradient_center_differs_from_linear_edge() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let radial = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                nana_ui_core::CssGradient::Radial(nana_ui_core::RadialGradient {
                    circle: true,
                    center: [nana_ui_core::LengthSpec::Percent(50.0); 2],
                    stops: vec![
                        nana_ui_core::GradientStop {
                            position: 0.0,
                            color: [1.0, 0.0, 0.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 1.0,
                            color: [0.0, 0.0, 1.0, 1.0],
                        },
                    ],
                }),
            )),
            ..Default::default()
        };
        let linear = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                    angle_deg: 90.0,
                    stops: vec![
                        nana_ui_core::GradientStop {
                            position: 0.0,
                            color: [1.0, 0.0, 0.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 1.0,
                            color: [0.0, 0.0, 1.0, 1.0],
                        },
                    ],
                }),
            )),
            ..Default::default()
        };
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };

        let mut scene = UiScene::new();
        scene.apply_delta(
            [paint_surface_quad_node(
                1,
                0.0,
                0.0,
                64.0,
                64.0,
                [0.0, 0.0, 0.0, 0.0],
                radial,
            )],
            [],
        );
        let (radial_tex, radial_view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui radial gradient"),
        });
        painter
            .paint(&scene, &mut encoder, &radial_view, viewport, None, None)
            .unwrap();
        let radial_pixels = readback_rgba(&device, &queue, encoder, &radial_tex, 64, 64);
        let radial_center = pixel(&radial_pixels, 64, 32, 32);
        let radial_corner = pixel(&radial_pixels, 64, 4, 4);

        let mut scene = UiScene::new();
        scene.apply_delta(
            [paint_surface_quad_node(
                2,
                0.0,
                0.0,
                64.0,
                64.0,
                [0.0, 0.0, 0.0, 0.0],
                linear,
            )],
            [],
        );
        let (linear_tex, linear_view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui linear gradient"),
        });
        painter
            .paint(&scene, &mut encoder, &linear_view, viewport, None, None)
            .unwrap();
        let linear_pixels = readback_rgba(&device, &queue, encoder, &linear_tex, 64, 64);
        let linear_left = pixel(&linear_pixels, 64, 4, 32);
        let linear_right = pixel(&linear_pixels, 64, 60, 32);

        assert!(
            radial_center[0] > 200 && radial_center[2] < 80,
            "radial center must be red, got {radial_center:?}"
        );
        assert!(
            radial_corner[2] > 200 && radial_corner[0] < 80,
            "radial corner must be blue, got {radial_corner:?}"
        );
        assert!(
            linear_left[0] > 200 && linear_left[2] < 80,
            "linear left must be red, got {linear_left:?}"
        );
        assert!(
            linear_right[2] > 200 && linear_right[0] < 80,
            "linear right must be blue, got {linear_right:?}"
        );
        drop(radial_tex);
        drop(linear_tex);
    }

    #[test]
    fn linear_gradient_five_stops_uses_stop_five_on_gpu() {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let surface = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                    angle_deg: 180.0,
                    stops: vec![
                        nana_ui_core::GradientStop {
                            position: 0.0,
                            color: [1.0, 0.0, 0.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 0.2,
                            color: [1.0, 0.0, 0.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 0.4,
                            color: [1.0, 0.0, 0.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 0.6,
                            color: [1.0, 0.0, 0.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 0.8,
                            color: [0.0, 0.0, 1.0, 1.0],
                        },
                        nana_ui_core::GradientStop {
                            position: 1.0,
                            color: [0.0, 0.0, 1.0, 1.0],
                        },
                    ],
                }),
            )),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta(
            [paint_surface_quad_node(
                1,
                0.0,
                0.0,
                64.0,
                64.0,
                [0.0, 0.0, 0.0, 0.0],
                surface,
            )],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui five-stop gradient"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let upper = pixel(&pixels, 64, 32, 8);
        let lower = pixel(&pixels, 64, 32, 56);
        assert!(
            upper[0] > 200 && upper[2] < 80,
            "top must stay red before stop 5, got {upper:?}"
        );
        assert!(
            lower[2] > 200 && lower[0] < 80,
            "bottom must reach blue from stop 5+, got {lower:?}"
        );
        drop(texture);
    }

    fn blue_tile_fixture_png() -> (std::path::PathBuf, std::path::PathBuf) {
        let fixture_dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        std::fs::create_dir_all(&fixture_dir).expect("fixture dir");
        let png_path = fixture_dir.join("blue-tile.png");
        if !png_path.exists() {
            let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 255, 255]));
            img.save(&png_path).expect("write fixture png");
        }
        (fixture_dir, png_path)
    }

    fn paint_url_quad_and_sample_center(url: String) -> [u8; 4] {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let surface = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::url_with_fit(
                url,
                nana_ui_core::BackgroundImageFit::Stretch,
            )),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta(
            [paint_surface_quad_node(
                1,
                0.0,
                0.0,
                64.0,
                64.0,
                [0.0, 0.0, 0.0, 0.0],
                surface,
            )],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: [64.0, 64.0],
            physical_size: [64, 64],
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, 64, 64);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui url png"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, 64, 64);
        let sample = pixel(&pixels, 64, 32, 32);
        drop(texture);
        sample
    }

    struct LocalPngServer {
        url: String,
        shutdown: std::sync::mpsc::Sender<()>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl LocalPngServer {
        fn serve(png: Vec<u8>) -> Self {
            let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .expect("bind 127.0.0.1");
            let addr = listener.local_addr().expect("local addr");
            listener.set_nonblocking(true).expect("nonblocking");
            let (shutdown, rx) = std::sync::mpsc::channel::<()>();
            let handle = std::thread::spawn(move || {
                loop {
                    if rx.try_recv().is_ok() {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            use std::io::{Read, Write};
                            let mut buf = [0u8; 1024];
                            let _ = stream.read(&mut buf);
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                png.len()
                            );
                            let _ = stream.write_all(header.as_bytes());
                            let _ = stream.write_all(&png);
                            let _ = stream.flush();
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                url: format!("http://{addr}/blue-tile.png"),
                shutdown,
                handle: Some(handle),
            }
        }
    }

    impl Drop for LocalPngServer {
        fn drop(&mut self) {
            let _ = self.shutdown.send(());
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    #[test]
    fn background_image_file_url_paints_fixture_png() {
        let (fixture_dir, png_path) = blue_tile_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let file_url = url::Url::from_file_path(&png_path)
            .map(|parsed| parsed.to_string())
            .unwrap_or_else(|_| {
                format!(
                    "file:///{}",
                    png_path.display().to_string().replace('\\', "/")
                )
            });
        let sample = paint_url_quad_and_sample_center(file_url);
        super::image_url::reset_test_url_base();
        assert!(
            sample[2] > 200 && sample[0] < 80,
            "file url png must paint blue tile, got {sample:?}"
        );
    }

    #[test]
    fn background_image_http_url_paints_fixture_png() {
        let (_fixture_dir, png_path) = blue_tile_fixture_png();
        let png = std::fs::read(&png_path).expect("read fixture png");
        let server = LocalPngServer::serve(png);
        let sample = paint_url_quad_and_sample_center(server.url.clone());
        assert!(
            sample[2] > 200 && sample[0] < 80,
            "http url png must paint blue tile, got {sample:?}"
        );
    }

    fn red_tile_fixture_png() -> std::path::PathBuf {
        let fixture_dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        std::fs::create_dir_all(&fixture_dir).expect("fixture dir");
        let png_path = fixture_dir.join("red-tile.png");
        if !png_path.exists() {
            let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]));
            img.save(&png_path).expect("write red fixture png");
        }
        png_path
    }

    fn stripe_tile_fixture_png() -> std::path::PathBuf {
        let fixture_dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        std::fs::create_dir_all(&fixture_dir).expect("fixture dir");
        let png_path = fixture_dir.join("stripe-tile.png");
        if !png_path.exists() {
            let mut img = image::RgbaImage::new(8, 8);
            for (x, _y, pixel) in img.enumerate_pixels_mut() {
                *pixel = if x < 4 {
                    image::Rgba([255, 0, 0, 255])
                } else {
                    image::Rgba([0, 0, 255, 255])
                };
            }
            img.save(&png_path).expect("write stripe fixture png");
        }
        png_path
    }

    fn paint_surface_sample(
        surface: nana_ui_scene::QuadSurfacePaint,
        logical: [f32; 2],
        physical: [u32; 2],
        fill: [f32; 4],
        sample_x: u32,
        sample_y: u32,
    ) -> [u8; 4] {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [paint_surface_quad_node(
                1, 0.0, 0.0, logical[0], logical[1], fill, surface,
            )],
            [],
        );
        let viewport = ScenePaintViewport {
            logical_size: logical,
            physical_size: physical,
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, physical[0], physical[1]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui image layer png"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, physical[0], physical[1]);
        let sample = pixel(&pixels, physical[0], sample_x, sample_y);
        drop(texture);
        sample
    }

    #[test]
    fn img_src_content_image_paints_fixture_png() {
        let (fixture_dir, png_path) = blue_tile_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let surface = nana_ui_scene::QuadSurfacePaint {
            content_image: Some(nana_ui_core::BackgroundImage::url_with_fit(
                png_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("blue-tile.png")
                    .to_string(),
                nana_ui_core::BackgroundImageFit::Stretch,
            )),
            ..Default::default()
        };
        let sample = paint_surface_sample(
            surface,
            [64.0, 64.0],
            [64, 64],
            [0.0, 0.0, 0.0, 1.0],
            32,
            32,
        );
        super::image_url::reset_test_url_base();
        assert!(
            sample[2] > 200 && sample[0] < 80,
            "img src must paint through content_image, got {sample:?}"
        );
    }

    #[test]
    fn inline_svg_data_url_paints_through_content_image() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"><path d=\"M0 0 H8 V8 H0 Z\" fill=\"#00ff00\"/></svg>";
        use base64::Engine as _;
        let url = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(svg.as_bytes())
        );
        let surface = nana_ui_scene::QuadSurfacePaint {
            content_image: Some(nana_ui_core::BackgroundImage::url_with_fit(
                url,
                nana_ui_core::BackgroundImageFit::Stretch,
            )),
            ..Default::default()
        };
        let sample = paint_surface_sample(surface, [8.0, 8.0], [8, 8], [0.0, 0.0, 0.0, 1.0], 4, 4);
        assert!(
            sample[1] > 200 && sample[0] < 80 && sample[2] < 80,
            "inline svg path must paint via the url texture cache, got {sample:?}"
        );
    }

    #[test]
    fn object_fit_contain_letterboxes_on_wide_box() {
        let (fixture_dir, png_path) = blue_tile_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let surface = nana_ui_scene::QuadSurfacePaint {
            content_image: Some(nana_ui_core::BackgroundImage::Url {
                url: png_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("blue-tile.png")
                    .to_string(),
                fit: nana_ui_core::BackgroundImageFit::Contain,
                size_width: None,
                size_height: None,
                position: nana_ui_core::BackgroundPosition::center(),
                repeat: nana_ui_core::BackgroundRepeat::NoRepeat,
            }),
            ..Default::default()
        };
        let fill = [1.0, 0.0, 0.0, 1.0];
        let center = paint_surface_sample(surface.clone(), [64.0, 32.0], [64, 32], fill, 32, 16);
        let edge = paint_surface_sample(surface, [64.0, 32.0], [64, 32], fill, 2, 16);
        super::image_url::reset_test_url_base();
        assert!(
            center[2] > 200 && center[0] < 80,
            "contain center must sample the image, got {center:?}"
        );
        assert!(
            edge[0] > 200 && edge[2] < 80,
            "contain letterbox must keep background, got {edge:?}"
        );
    }

    #[test]
    fn two_layer_background_paints_top_over_bottom() {
        let (fixture_dir, blue_path) = blue_tile_fixture_png();
        let red_path = red_tile_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let surface = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Url {
                url: red_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("red-tile.png")
                    .to_string(),
                fit: nana_ui_core::BackgroundImageFit::Auto,
                size_width: None,
                size_height: None,
                position: nana_ui_core::BackgroundPosition::default(),
                repeat: nana_ui_core::BackgroundRepeat::NoRepeat,
            }),
            background_layers: vec![nana_ui_core::BackgroundImage::url_with_fit(
                blue_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("blue-tile.png")
                    .to_string(),
                nana_ui_core::BackgroundImageFit::Stretch,
            )],
            ..Default::default()
        };
        let top_left = paint_surface_sample(
            surface.clone(),
            [64.0, 64.0],
            [64, 64],
            [0.0, 0.0, 0.0, 1.0],
            4,
            4,
        );
        let far = paint_surface_sample(
            surface,
            [64.0, 64.0],
            [64, 64],
            [0.0, 0.0, 0.0, 1.0],
            48,
            48,
        );
        super::image_url::reset_test_url_base();
        assert!(
            top_left[0] > 200 && top_left[2] < 80,
            "8px auto red layer must cover the top-left, got {top_left:?}"
        );
        assert!(
            far[2] > 200 && far[0] < 80,
            "stretch blue layer must show outside the red tile, got {far:?}"
        );
    }

    #[test]
    fn background_repeat_x_tiles_stripe() {
        let fixture_dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let stripe = stripe_tile_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let surface = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Url {
                url: stripe
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("stripe-tile.png")
                    .to_string(),
                fit: nana_ui_core::BackgroundImageFit::Auto,
                size_width: None,
                size_height: None,
                position: nana_ui_core::BackgroundPosition::default(),
                repeat: nana_ui_core::BackgroundRepeat::RepeatX,
            }),
            ..Default::default()
        };
        let left = paint_surface_sample(
            surface.clone(),
            [16.0, 8.0],
            [16, 8],
            [0.0, 0.0, 0.0, 1.0],
            2,
            4,
        );
        let mid = paint_surface_sample(surface, [16.0, 8.0], [16, 8], [0.0, 0.0, 0.0, 1.0], 10, 4);
        super::image_url::reset_test_url_base();
        assert!(
            left[0] > 200 && left[2] < 80,
            "first tile left must be red, got {left:?}"
        );
        assert!(
            mid[0] > 200 && mid[2] < 80,
            "second tile left must be red from repeat-x, got {mid:?}"
        );
    }

    #[test]
    fn default_background_repeat_tiles_sized_url() {
        let (fixture_dir, png_path) = blue_tile_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let surface = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Url {
                url: png_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("blue-tile.png")
                    .to_string(),
                fit: nana_ui_core::BackgroundImageFit::Length,
                size_width: Some(nana_ui_core::LengthSpec::Px(32.0)),
                size_height: None,
                position: nana_ui_core::BackgroundPosition::default(),
                repeat: nana_ui_core::BackgroundRepeat::Repeat,
            }),
            ..Default::default()
        };
        let first = paint_surface_sample(
            surface.clone(),
            [64.0, 32.0],
            [64, 32],
            [1.0, 0.0, 0.0, 1.0],
            8,
            16,
        );
        let tiled = paint_surface_sample(
            surface,
            [64.0, 32.0],
            [64, 32],
            [1.0, 0.0, 0.0, 1.0],
            40,
            16,
        );
        super::image_url::reset_test_url_base();
        assert!(
            first[2] > 200 && first[0] < 80,
            "first 32px tile must be the image, got {first:?}"
        );
        assert!(
            tiled[2] > 200 && tiled[0] < 80,
            "unspecified repeat + size 32px must tile, got {tiled:?}"
        );
    }

    #[test]
    fn background_repeat_space_does_not_paint_as_repeat() {
        let (fixture_dir, png_path) = blue_tile_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let surface = nana_ui_scene::QuadSurfacePaint {
            background_image: Some(nana_ui_core::BackgroundImage::Url {
                url: png_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("blue-tile.png")
                    .to_string(),
                fit: nana_ui_core::BackgroundImageFit::Length,
                size_width: Some(nana_ui_core::LengthSpec::Px(32.0)),
                size_height: None,
                position: nana_ui_core::BackgroundPosition::default(),
                repeat: nana_ui_core::BackgroundRepeat::Unsupported,
            }),
            ..Default::default()
        };
        let first =
            paint_surface_sample(surface, [64.0, 32.0], [64, 32], [1.0, 0.0, 0.0, 1.0], 8, 16);
        super::image_url::reset_test_url_base();
        assert!(
            first[0] > 200 && first[2] < 80,
            "space/unsupported must fail-closed the url layer (fill red), not tile blue, got {first:?}"
        );
    }

    #[test]
    fn mask_image_url_reuses_jail_texture() {
        let (fixture_dir, png_path) = blue_tile_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let surface = nana_ui_scene::QuadSurfacePaint {
            mask: Some(nana_ui_core::MaskImage::Url(
                png_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("blue-tile.png")
                    .to_string(),
            )),
            ..Default::default()
        };
        let sample =
            paint_surface_sample(surface, [16.0, 16.0], [16, 16], [1.0, 0.0, 0.0, 1.0], 8, 8);
        super::image_url::reset_test_url_base();
        assert!(
            sample[0] > 200 && sample[2] < 80,
            "opaque mask url must keep the fill, got {sample:?}"
        );
    }

    fn nine_slice_fixture_png() -> (std::path::PathBuf, std::path::PathBuf) {
        let fixture_dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        std::fs::create_dir_all(&fixture_dir).expect("fixture dir");
        let png_path = fixture_dir.join("nine-slice.png");
        if !png_path.exists() {
            let mut img = image::RgbaImage::new(8, 8);
            for (x, y, pixel) in img.enumerate_pixels_mut() {
                let region_x = if x < 2 {
                    0
                } else if x < 6 {
                    1
                } else {
                    2
                };
                let region_y = if y < 2 {
                    0
                } else if y < 6 {
                    1
                } else {
                    2
                };
                *pixel = match (region_x, region_y) {
                    (0, 0) => image::Rgba([255, 0, 0, 255]),
                    (1, 0) => image::Rgba([0, 255, 0, 255]),
                    (2, 0) => image::Rgba([0, 0, 255, 255]),
                    (0, 1) => image::Rgba([0, 255, 255, 255]),
                    (1, 1) => image::Rgba([255, 255, 255, 255]),
                    (2, 1) => image::Rgba([255, 0, 255, 255]),
                    (0, 2) => image::Rgba([255, 255, 0, 255]),
                    (1, 2) => image::Rgba([0, 0, 0, 255]),
                    _ => image::Rgba([128, 128, 128, 255]),
                };
            }
            img.save(&png_path).expect("write nine-slice png");
        }
        (fixture_dir, png_path)
    }

    fn paint_layout_sample(
        layout: nana_ui_core::LayoutStyle,
        logical: [f32; 2],
        sample_x: u32,
        sample_y: u32,
    ) -> [u8; 4] {
        let fill = layout.background.unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut painter = SceneWgpuPainter::new(&device, &queue, format);
        let mut scene = UiScene::new();
        scene.apply_delta(
            [extracted_div(
                1,
                &[],
                0.0,
                0.0,
                logical[0],
                logical[1],
                layout,
                Some(fill),
            )],
            [],
        );
        let physical = [logical[0] as u32, logical[1] as u32];
        let viewport = ScenePaintViewport {
            logical_size: logical,
            physical_size: physical,
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let (texture, view) = test_copy_target(&device, format, physical[0], physical[1]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui layout sample"),
        });
        painter
            .paint(&scene, &mut encoder, &view, viewport, None, None)
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &texture, physical[0], physical[1]);
        let sample = pixel(&pixels, physical[0], sample_x, sample_y);
        drop(texture);
        sample
    }

    #[test]
    fn four_side_solid_borders_still_paint() {
        let layout = nana_ui_core::LayoutStyle {
            background: Some([0.1, 0.1, 0.1, 1.0]),
            border_top_width: Some(6.0),
            border_right_width: Some(6.0),
            border_bottom_width: Some(6.0),
            border_left_width: Some(6.0),
            border_top_color: Some([1.0, 0.0, 0.0, 1.0]),
            border_right_color: Some([0.0, 1.0, 0.0, 1.0]),
            border_bottom_color: Some([0.0, 0.0, 1.0, 1.0]),
            border_left_color: Some([1.0, 1.0, 0.0, 1.0]),
            ..Default::default()
        };
        let top = paint_layout_sample(layout.clone(), [40.0, 40.0], 20, 2);
        let right = paint_layout_sample(layout.clone(), [40.0, 40.0], 38, 20);
        let bottom = paint_layout_sample(layout.clone(), [40.0, 40.0], 20, 38);
        let left = paint_layout_sample(layout, [40.0, 40.0], 2, 20);
        assert!(
            top[0] > 200 && top[1] < 80,
            "top must stay red, got {top:?}"
        );
        assert!(
            right[1] > 200 && right[0] < 80,
            "right must stay green, got {right:?}"
        );
        assert!(
            bottom[2] > 200 && bottom[0] < 80,
            "bottom must stay blue, got {bottom:?}"
        );
        assert!(
            left[0] > 200 && left[1] > 200 && left[2] < 80,
            "left must stay yellow, got {left:?}"
        );
    }

    #[test]
    fn border_image_url_nine_slice_paints_corners_and_fill() {
        let (fixture_dir, png_path) = nine_slice_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let url = png_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("nine-slice.png")
            .to_string();
        let surface = nana_ui_scene::QuadSurfacePaint {
            border_image: Some(nana_ui_core::BorderImageSpec {
                source: nana_ui_core::BackgroundImage::url(url),
                slice: [nana_ui_core::BorderImageSlice::Number(2.0); 4],
                fill: true,
            }),
            ..Default::default()
        };
        let tl = paint_surface_sample(
            surface.clone(),
            [40.0, 40.0],
            [40, 40],
            [0.1, 0.1, 0.1, 1.0],
            1,
            1,
        );
        let top = paint_surface_sample(
            surface.clone(),
            [40.0, 40.0],
            [40, 40],
            [0.1, 0.1, 0.1, 1.0],
            20,
            1,
        );
        let left = paint_surface_sample(
            surface.clone(),
            [40.0, 40.0],
            [40, 40],
            [0.1, 0.1, 0.1, 1.0],
            1,
            20,
        );
        let center = paint_surface_sample(
            surface.clone(),
            [40.0, 40.0],
            [40, 40],
            [0.1, 0.1, 0.1, 1.0],
            20,
            20,
        );
        let br = paint_surface_sample(
            surface,
            [40.0, 40.0],
            [40, 40],
            [0.1, 0.1, 0.1, 1.0],
            38,
            38,
        );
        super::image_url::reset_test_url_base();
        assert!(
            tl[0] > 200 && tl[1] < 80,
            "tl slice must be red, got {tl:?}"
        );
        assert!(
            top[1] > 200 && top[0] < 80,
            "top slice must be green, got {top:?}"
        );
        assert!(
            left[1] > 200 && left[2] > 200 && left[0] < 80,
            "left slice must be cyan, got {left:?}"
        );
        assert!(
            center[0] > 200 && center[1] > 200 && center[2] > 200,
            "fill center must be white, got {center:?}"
        );
        assert!(
            br[0] > 80 && br[0] < 180 && br[1] > 80 && br[1] < 180,
            "br slice must be gray, got {br:?}"
        );
    }

    #[test]
    fn unsupported_border_image_does_not_paint_nine_slice() {
        let (fixture_dir, png_path) = nine_slice_fixture_png();
        super::set_background_image_url_base(fixture_dir);
        let url = png_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("nine-slice.png")
            .to_string();
        let layout = nana_ui_core::LayoutStyle {
            background: Some([0.1, 0.1, 0.1, 1.0]),
            paint: nana_ui_core::PaintStyle {
                unsupported_border_image: true,
                border_image: Some(nana_ui_core::BorderImageSpec {
                    source: nana_ui_core::BackgroundImage::url(url),
                    slice: [nana_ui_core::BorderImageSlice::Number(2.0); 4],
                    fill: true,
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let center = paint_layout_sample(layout, [40.0, 40.0], 20, 20);
        super::image_url::reset_test_url_base();
        assert!(
            center[0] < 80 && center[1] < 80 && center[2] < 80,
            "sticky unsupported must not 9-slice-fill white, got {center:?}"
        );
    }

    #[test]
    fn border_image_linear_gradient_nine_slice_paints() {
        let surface = nana_ui_scene::QuadSurfacePaint {
            border_image: Some(nana_ui_core::BorderImageSpec {
                source: nana_ui_core::BackgroundImage::Gradient(nana_ui_core::CssGradient::Linear(
                    nana_ui_core::LinearGradient {
                        angle_deg: 180.0,
                        stops: vec![
                            nana_ui_core::GradientStop {
                                position: 0.0,
                                color: [1.0, 0.0, 0.0, 1.0],
                            },
                            nana_ui_core::GradientStop {
                                position: 1.0,
                                color: [0.0, 0.0, 1.0, 1.0],
                            },
                        ],
                    },
                )),
                slice: [nana_ui_core::BorderImageSlice::Percent(25.0); 4],
                fill: true,
            }),
            ..Default::default()
        };
        let top = paint_surface_sample(
            surface.clone(),
            [40.0, 40.0],
            [40, 40],
            [0.0, 0.0, 0.0, 1.0],
            20,
            4,
        );
        let bottom = paint_surface_sample(
            surface,
            [40.0, 40.0],
            [40, 40],
            [0.0, 0.0, 0.0, 1.0],
            20,
            36,
        );
        assert!(
            top[0] > 180 && top[2] < 80,
            "gradient top slice must stay red, got {top:?}"
        );
        assert!(
            bottom[2] > 180 && bottom[0] < 80,
            "gradient bottom slice must stay blue, got {bottom:?}"
        );
    }
}
