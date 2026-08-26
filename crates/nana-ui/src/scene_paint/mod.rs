//! Nana-owned WGPU painter for [`UiScene`].
//!
//! This is the product Scene backend. It paints [`UiScene`] through the host GPU.
//! The host owns Device/Queue/encoder. HostTexture is sampled in document order
//! inside the current dest (or opacity-group) pass; groups do not open a pass
//! per HostTexture slot.

mod clip;
mod color;
mod dest;
mod host_texture;
mod icon;
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

pub(crate) use validate::validate_scene;
pub use validate::{HostTextureSceneResolver, ScenePaintError};

use clip::{
    FragmentClip, LogicalRect, extra_fragment_clips, fragment_clip, intersect_clips, local_rect,
    paint_affine, paint_origin, physical_bounds, physical_scissor, rotated_fragment_clips,
    transformed_aabb,
};
use dest::{DestPassCounts, DestTarget, GroupSlot};
use host_texture::{HostTexturePipeline, PreparedHostTexture};
use icon::{IconPipeline, PreparedIcon};
use mesh::{MeshPipeline, MeshRange};
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
            );
            let Some(clip) = intersect_clips(viewport_clip, &primitive.clips, origin) else {
                continue;
            };
            let Some(scissor) = physical_scissor(clip, scale, dest_physical) else {
                continue;
            };
            let frag_clip = fragment_clip(&primitive.clips, origin);
            let affine = paint_affine(primitive.transform.0, origin);
            let bounds = local_rect(primitive.bounds);
            let command_start = commands.len();
            match &primitive.kind {
                ScenePrimitiveKind::Quad {
                    background,
                    border_color,
                    border_width,
                    corner_radius,
                    shadow,
                } => {
                    if let Some(index) = self.quads.push(
                        bounds,
                        clip,
                        frag_clip,
                        affine,
                        *background,
                        *border_color,
                        *border_width,
                        *corner_radius,
                        *shadow,
                        primitive.opacity,
                    ) {
                        push_quad(&mut commands, index, scissor);
                    }
                }
                ScenePrimitiveKind::QuadBatch {
                    bounds: batch,
                    background,
                    border_color,
                    border_width,
                    corner_radius,
                    shadow,
                } => {
                    for item in batch {
                        let item_bounds = local_rect(*item);
                        if let Some(index) = self.quads.push(
                            item_bounds,
                            clip,
                            frag_clip,
                            affine,
                            *background,
                            *border_color,
                            *border_width,
                            *corner_radius,
                            *shadow,
                            primitive.opacity,
                        ) {
                            push_quad(&mut commands, index, scissor);
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
                    shaping,
                    horizontal_alignment,
                    vertical_alignment,
                    spans,
                    letter_spacing,
                } => {
                    if let Some(prepared) = self.text.prepare(
                        &self.device,
                        &self.queue,
                        encoder,
                        bounds,
                        clip,
                        scale,
                        content,
                        *color,
                        *size,
                        *weight,
                        family.as_deref(),
                        *line_height,
                        *wrap,
                        *ellipsis,
                        *shaping,
                        *horizontal_alignment,
                        *vertical_alignment,
                        spans,
                        *letter_spacing,
                        affine,
                        frag_clip,
                        primitive.opacity,
                    ) {
                        commands.push(DrawCommand::Text { prepared, scissor });
                    }
                }
                ScenePrimitiveKind::Icon { icon, color } => {
                    if let Some(prepared) = self.icons.prepare(
                        &self.device,
                        &self.queue,
                        bounds,
                        affine,
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
                        affine,
                        *phase,
                        color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                        primitive.opacity,
                        frag_clip,
                    ) {
                        commands.push(DrawCommand::Mesh { range, scissor });
                    }
                }
                ScenePrimitiveKind::Stroke {
                    points,
                    width,
                    color,
                } => {
                    if let Some(range) = self.meshes.push_stroke(
                        points,
                        affine,
                        *width,
                        *color,
                        primitive.opacity,
                        frag_clip,
                    ) {
                        commands.push(DrawCommand::Mesh { range, scissor });
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
                                    Some((local_rect(quad.bounds), *corner_radius))
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
                        renderer.prepare(
                            &node,
                            SceneGpuPrepareContext {
                                device: &self.device,
                                queue: &self.queue,
                                target_format: self.format,
                                bounds: transformed_aabb(bounds, affine).to_core(),
                                scale_factor: scale,
                                gpu_work: Some(&gpu_work),
                            },
                        );
                        commands.push(DrawCommand::Custom {
                            node,
                            renderer,
                            bounds: physical_bounds(
                                transformed_aabb(bounds, affine),
                                scale,
                                scissor,
                            ),
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
        let gpu_upload = upload_started.elapsed();

        let encode_started = Instant::now();
        let gpu_interleaved = commands.iter().any(|command| {
            matches!(
                command,
                DrawCommand::HostTexture(_)
                    | DrawCommand::Custom { .. }
                    | DrawCommand::PushGroup { .. }
            )
        });
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
                &EncodeOrdered {
                    quads: &self.quads,
                    meshes: &self.meshes,
                    icons: &self.icons,
                    text: &self.text,
                    host_textures: &self.host_textures,
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
        rotated_fragment_clips(clips, origin)
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
        uniforms.push(GroupSlot::opacity(group.opacity));
        commands.push(DrawCommand::PushGroup { layer, slot });
        stack.push(group);
    }
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

struct EncodeOrdered<'a> {
    quads: &'a QuadPipeline,
    meshes: &'a MeshPipeline,
    icons: &'a IconPipeline,
    text: &'a TextPipeline,
    host_textures: &'a HostTexturePipeline,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    gpu_work: &'a GpuWorkSink,
}

struct GroupFrame {
    layer: usize,
    slot: u32,
}

fn encode_ordered(
    pipelines: &EncodeOrdered<'_>,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nana_ui_core::{ButtonKind, LengthSpec, OverflowSpec, PaintTransform, SemanticColorRole};
    use nana_ui_runtime::{
        AppContext, Button as RuntimeButton, ComponentGeometry, ComputedStyle, CustomRenderNode,
        DocumentId, ExtractedNode, GpuTextureView, LayoutBox, MutationQueue, NodeKind, NodeStyle,
        StableNodeId, TextContent,
    };
    use nana_ui_scene::{AffineTransform, ClipRegion, ScenePrimitiveKind, SceneRect, UiScene};

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
                transform: AffineTransform(
                    PaintTransform {
                        a: k,
                        b: k,
                        c: -k,
                        d: k,
                        ..PaintTransform::default()
                    }
                    .around_center(16.0, 16.0, 32.0, 32.0),
                ),
            }]
        };
        let custom = ScenePrimitiveKind::Custom(CustomRenderNode::new("test.fill", "slot", 1));
        let quad = ScenePrimitiveKind::Quad {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            border_color: None,
            border_width: 0.0,
            corner_radius: 0.0,
            shadow: None,
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
            transform: AffineTransform(
                PaintTransform {
                    a: k,
                    b: k,
                    c: -k,
                    d: k,
                    ..PaintTransform::default()
                }
                .around_center(16.0, 16.0, 32.0, 32.0),
            ),
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

    fn colored_quad_node(
        value: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    ) -> ExtractedNode {
        ExtractedNode {
            id: StableNodeId::new(value).unwrap(),
            kind: Arc::new(NodeKind::Element { tag: "div".into() }),
            parent: None,
            children: Arc::new(Vec::new()),
            layout: LayoutBox {
                x,
                y,
                width,
                height,
            },
            scroll_offset: nana_ui_runtime::ScrollOffset::default(),
            source_style: NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    background: Some(color),
                    ..nana_ui_core::LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
            style: Arc::new(ComputedStyle {
                background: Some(color),
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
}
