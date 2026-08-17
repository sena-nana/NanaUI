//! Nana-owned WGPU painter for [`UiScene`].
//!
//! This is the product Scene backend. It does not implement `iced::Widget`.
//! The host owns Device/Queue/encoder.

mod clip;
mod color;
mod dest;
mod host_texture;
mod mesh;
mod quad;
mod text;
mod validate;

use std::sync::Arc;

use nana_ui_scene::{RenderOperation, ScenePrimitiveKind, UiScene};

use crate::scene_gpu::{
    SceneGpuNode, SceneGpuPrepareContext, SceneGpuRenderContext, SceneGpuRenderer,
    SceneGpuRendererRegistry,
};
use crate::{HostTextureRegistry, PhysicalRect};

pub(crate) use validate::validate_scene;
pub use validate::{HostTextureSceneResolver, ScenePaintError};

use clip::{
    LogicalRect, intersect_clips, paint_origin, physical_bounds, physical_scissor, translated_rect,
};
use dest::DestTarget;
use host_texture::{HostTexturePipeline, PreparedHostTexture};
use mesh::{MeshPipeline, MeshRange};
use quad::QuadPipeline;
use text::{PreparedText, TextPipeline};

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
        }
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
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
                        commands.push(DrawCommand::HostTexture(self.host_textures.prepare(
                            &self.device,
                            &self.queue,
                            binding,
                            primitive.id.node.get(),
                            primitive.id.slot,
                            bounds,
                            scissor,
                            primitive.opacity,
                            dest_physical,
                            scale,
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

        self.quads
            .upload(&self.device, &self.queue, dest_physical, scale);
        self.meshes
            .upload(&self.device, &self.queue, dest_physical, scale);

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
        {
            let mut pass = dest.begin_msaa_pass(encoder, clear);
            for command in &commands {
                match command {
                    DrawCommand::Quads { range, scissor } => {
                        self.quads.draw(&mut pass, range.clone(), *scissor);
                    }
                    DrawCommand::Mesh { range, scissor } => {
                        self.meshes.draw(&mut pass, range, *scissor);
                    }
                    DrawCommand::Text { .. }
                    | DrawCommand::HostTexture(_)
                    | DrawCommand::Custom { .. } => {}
                }
            }
        }
        {
            let mut pass = begin_pass(encoder, dest.color_view(), wgpu::LoadOp::Load);
            for command in &commands {
                if let DrawCommand::Text { prepared, scissor } = command {
                    self.text.draw(&mut pass, prepared, *scissor);
                }
            }
        }
        {
            let dest_view = dest.color_view();
            for command in &commands {
                match command {
                    DrawCommand::HostTexture(prepared) => {
                        self.host_textures.render(prepared, encoder, dest_view);
                    }
                    DrawCommand::Custom {
                        node,
                        renderer,
                        bounds,
                        clip,
                    } if bounds.width > 0 && bounds.height > 0 => {
                        renderer.render(
                            node,
                            SceneGpuRenderContext {
                                device: &self.device,
                                queue: &self.queue,
                                encoder,
                                target: dest_view,
                                bounds: *bounds,
                                clip: *clip,
                            },
                        );
                    }
                    _ => {}
                }
            }
        }
        dest.blit(
            encoder,
            target,
            blit_origin[0],
            blit_origin[1],
            viewport.physical_size,
            viewport.clear.then_some(clear),
        );
        self.host_textures.trim();
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

fn begin_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    target: &'a wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("nana-ui.scene.paint"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

#[cfg(test)]
mod tests {
    use nana_ui_runtime::{
        AppContext, Button as RuntimeButton, CustomRenderNode, DocumentId, LayoutBox, MutationQueue,
    };
    use nana_ui_scene::UiScene;

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
            Some(CustomRenderNode {
                renderer: "nana.host-texture".into(),
                resource: "preview".into(),
                revision: 1,
            }),
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
}
