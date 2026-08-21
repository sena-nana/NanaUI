//! Nana-owned WGPU painter for [`UiScene`].
//!
//! This is the product Scene backend. It paints [`UiScene`] through the host GPU.
//! The host owns Device/Queue/encoder. HostTexture is sampled in document order
//! inside one dest pass.

mod clip;
mod color;
mod dest;
mod host_texture;
mod mesh;
mod quad;
mod text;
mod validate;

use std::sync::Arc;
use std::time::Instant;

use nana_ui_core::GpuWorkObservation;
use nana_ui_scene::{RenderOperation, ScenePrimitiveKind, UiScene};

use crate::gpu_work::{GpuStageTimings, GpuWorkSink};
use crate::scene_gpu::{
    SceneGpuNode, SceneGpuPassContext, SceneGpuPrepareContext, SceneGpuRenderContext,
    SceneGpuRenderer, SceneGpuRendererRegistry,
};
use crate::{HostTextureRegistry, PhysicalRect};

pub(crate) use validate::validate_scene;
pub use validate::{HostTextureSceneResolver, ScenePaintError};

use clip::{
    LogicalRect, intersect_clips, paint_origin, physical_bounds, physical_scissor, translated_rect,
};
use dest::{DestPassCounts, DestTarget};
use host_texture::{HostTexturePipeline, PreparedHostTexture};
use mesh::{MeshPipeline, MeshRange};
use quad::QuadPipeline;
use text::{PreparedText, TextPipeline};

#[derive(Clone, Copy)]
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
    text: TextPipeline,
    host_textures: HostTexturePipeline,
    dest: Option<DestTarget>,
    last_gpu_work: Option<GpuWorkObservation>,
    last_gpu_timings: Option<GpuStageTimings>,
    last_dest_pass_counts: Option<DestPassCounts>,
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
    HostTexture(PreparedHostTexture),
    Custom {
        node: SceneGpuNode,
        renderer: Arc<dyn SceneGpuRenderer>,
        bounds: PhysicalRect,
        clip: PhysicalRect,
    },
}

impl SceneWgpuPainter {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            format,
            quads: QuadPipeline::new(device, format),
            meshes: MeshPipeline::new(device, format),
            text: TextPipeline::new(device, queue, format),
            host_textures: HostTexturePipeline::new(device, queue, format),
            dest: None,
            last_gpu_work: None,
            last_gpu_timings: None,
            last_dest_pass_counts: None,
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
        let operations = validate_scene(scene, host_textures, gpu_renderers)?;
        if viewport.physical_size[0] == 0 || viewport.physical_size[1] == 0 {
            self.last_gpu_work = None;
            self.last_gpu_timings = None;
            self.last_dest_pass_counts = None;
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

        let batch_started = Instant::now();
        self.quads.begin_frame();
        self.meshes.begin_frame();
        self.text.begin_frame(&self.queue, dest_physical);

        let mut commands = Vec::new();
        for operation in operations.iter() {
            let id = match operation {
                RenderOperation::PrepareExternal(_) => continue,
                RenderOperation::Draw(id) | RenderOperation::InvokeCustom(id) => *id,
            };
            let Some(primitive) = scene.primitive(id) else {
                continue;
            };
            let Some(clip) = intersect_clips(viewport_clip, &primitive.clips, origin) else {
                continue;
            };
            let Some(scissor) = physical_scissor(clip, scale, dest_physical) else {
                continue;
            };
            let bounds = translated_rect(primitive.bounds, primitive.transform.0, origin);
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
                        let item_bounds = translated_rect(*item, primitive.transform.0, origin);
                        if let Some(index) = self.quads.push(
                            item_bounds,
                            clip,
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
                    letter_spacing: _,
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
                        primitive.opacity,
                    ) {
                        commands.push(DrawCommand::Text { prepared, scissor });
                    }
                }
                ScenePrimitiveKind::Icon { icon, color } => {
                    if let Some(range) = self.meshes.push_icon(
                        bounds,
                        *icon,
                        color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                        primitive.opacity,
                    ) {
                        commands.push(DrawCommand::Mesh { range, scissor });
                    }
                }
                ScenePrimitiveKind::Spinner { phase, color } => {
                    if let Some(range) = self.meshes.push_spinner(
                        bounds,
                        *phase,
                        color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                        primitive.opacity,
                    ) {
                        commands.push(DrawCommand::Mesh { range, scissor });
                    }
                }
                ScenePrimitiveKind::Stroke {
                    points,
                    width,
                    color,
                } => {
                    let mapped = points
                        .iter()
                        .map(|point| {
                            [
                                origin[0] + point[0] + primitive.transform.0[4],
                                origin[1] + point[1] + primitive.transform.0[5],
                            ]
                        })
                        .collect::<Vec<_>>();
                    if let Some(range) =
                        self.meshes
                            .push_stroke(&mapped, *width, *color, primitive.opacity)
                    {
                        commands.push(DrawCommand::Mesh { range, scissor });
                    }
                }
                ScenePrimitiveKind::Custom(custom) => {
                    if custom.renderer.as_ref() == "nana.host-texture" {
                        let binding = host_textures
                            .and_then(|registry| registry.get(custom.resource.as_ref()))
                            .expect("validated host texture remains registered");
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
                        commands.push(DrawCommand::HostTexture(self.host_textures.prepare(
                            &self.device,
                            &self.queue,
                            binding,
                            primitive.id.node.get(),
                            primitive.id.slot,
                            LogicalRect::from_xywh(dest.x, dest.y, dest.width, dest.height),
                            scissor,
                            primitive.opacity,
                            dest_physical,
                            scale,
                            Some(&gpu_work),
                        )));
                    } else {
                        let renderer = gpu_renderers
                            .and_then(|registry| registry.get(custom.renderer.as_ref()))
                            .expect("validated scene GPU renderer remains registered");
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
                                bounds: bounds.to_core(),
                                scale_factor: scale,
                                gpu_work: Some(&gpu_work),
                            },
                        );
                        commands.push(DrawCommand::Custom {
                            node,
                            renderer,
                            bounds: physical_bounds(bounds, scale, scissor),
                            clip: scissor,
                        });
                    }
                }
            }
        }
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
        let gpu_upload = upload_started.elapsed();

        let encode_started = Instant::now();
        DestTarget::ensure(
            &mut self.dest,
            &self.device,
            self.format,
            dest_physical[0],
            dest_physical[1],
        );
        let dest = self.dest.as_ref().expect("dest target");
        let clear = wgpu::Color {
            r: viewport.clear_color[0] as f64,
            g: viewport.clear_color[1] as f64,
            b: viewport.clear_color[2] as f64,
            a: viewport.clear_color[3] as f64,
        };
        let gpu_interleaved = commands.iter().any(|command| {
            matches!(
                command,
                DrawCommand::HostTexture(_) | DrawCommand::Custom { .. }
            )
        });
        let sample_count = if gpu_interleaved { 1 } else { 4 };
        let initial_load = if viewport.clear {
            wgpu::LoadOp::Clear(clear)
        } else {
            wgpu::LoadOp::Load
        };
        let mut dest_passes = DestPassCounts::default();
        if gpu_interleaved {
            encode_ordered(
                &EncodeOrdered {
                    quads: &self.quads,
                    meshes: &self.meshes,
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
                    | DrawCommand::HostTexture(_)
                    | DrawCommand::Custom { .. } => {}
                }
            }
            drop(pass);
            if commands
                .iter()
                .any(|command| matches!(command, DrawCommand::Text { .. }))
            {
                let mut pass = dest.begin_color_pass(encoder, wgpu::LoadOp::Load, &mut dest_passes);
                restore_dest_viewport(&mut pass, dest_physical);
                for command in &commands {
                    if let DrawCommand::Text { prepared, scissor } = command {
                        self.text
                            .draw(&mut pass, prepared, *scissor, Some(&gpu_work));
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
        self.last_gpu_work = Some(gpu_work.snapshot());
        self.last_gpu_timings = Some(GpuStageTimings {
            batch,
            gpu_upload,
            encode,
            submit: std::time::Duration::ZERO,
        });
        self.last_dest_pass_counts = Some(dest_passes);
        Ok(())
    }
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
    text: &'a TextPipeline,
    host_textures: &'a HostTexturePipeline,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    gpu_work: &'a GpuWorkSink,
}

fn encode_ordered(
    pipelines: &EncodeOrdered<'_>,
    encoder: &mut wgpu::CommandEncoder,
    dest: &DestTarget,
    dest_physical: [u32; 2],
    mut load: wgpu::LoadOp<wgpu::Color>,
    commands: &[DrawCommand],
    dest_passes: &mut DestPassCounts,
) {
    const SAMPLE_COUNT: u32 = 1;
    let mut index = 0;
    while index < commands.len() {
        let mut pass = dest.begin_color_pass(encoder, load, dest_passes);
        restore_dest_viewport(&mut pass, dest_physical);
        while index < commands.len() {
            match &commands[index] {
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
                    pipelines
                        .text
                        .draw(&mut pass, prepared, *scissor, Some(pipelines.gpu_work));
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
                renderer.render(
                    node,
                    SceneGpuRenderContext {
                        device: pipelines.device,
                        queue: pipelines.queue,
                        encoder,
                        target: dest.color_view(),
                        bounds: *bounds,
                        clip: *clip,
                        gpu_work: Some(pipelines.gpu_work),
                    },
                );
            }
            index += 1;
            load = wgpu::LoadOp::Load;
        }
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
    use nana_ui_core::{ButtonKind, LengthSpec, SemanticColorRole};
    use nana_ui_runtime::{
        AppContext, Button as RuntimeButton, ComponentGeometry, CustomRenderNode, DocumentId,
        GpuTextureView, LayoutBox, MutationQueue, NodeStyle,
    };
    use nana_ui_scene::{ScenePrimitiveKind, UiScene};

    use super::*;
    use crate::HostTextureRegistry;

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

    fn square_button_layout() -> std::sync::Arc<nana_ui_core::LayoutStyle> {
        std::sync::Arc::new(nana_ui_core::LayoutStyle {
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
