use std::sync::Arc;
#[cfg(feature = "hosted")]
use std::sync::Mutex;

#[cfg(feature = "hosted")]
use crate::scene_gpu::{
    SceneGpuNode, SceneGpuPrepareContext, SceneGpuRenderContext, SceneGpuRenderer,
};
use nana_ui_runtime::{NATIVE_CONTENT_RENDERER, StableNodeId};
use nana_ui_scene::{AffineTransform, ScenePrimitiveKind, SceneRect, UiScene};

#[derive(Clone, Debug, PartialEq)]
pub struct NativeContentRegion {
    pub node: StableNodeId,
    pub resource: Arc<str>,
    pub generation: u64,
    pub bounds: SceneRect,
    pub clip: SceneRect,
}

/// Resolves the same retained transforms and clip chain used by the painter.
/// Nonrectangular and offscreen composition require a platform capability that
/// this backend does not provide, so they fail before a native visual is shown.
pub fn native_content_regions(
    scene: &UiScene,
    viewport: SceneRect,
) -> Result<Vec<NativeContentRegion>, String> {
    let plan = scene.frame_plan().map_err(|error| error.to_string())?;
    let mut regions = Vec::new();
    let mut resources = std::collections::HashSet::new();
    for id in plan.custom_nodes.iter() {
        let Some(primitive) = scene.draw_primitive(*id) else {
            continue;
        };
        let ScenePrimitiveKind::Custom { node, mask } = &primitive.kind else {
            continue;
        };
        if node.renderer.as_ref() != NATIVE_CONTENT_RENDERER {
            continue;
        }
        if primitive.opacity != 1.0
            || mask.is_some()
            || !scene.opacity_groups(primitive.node).is_empty()
            || !scene.filter_groups(primitive.node).is_empty()
        {
            return Err("native content requires an opaque, unfiltered composition parent".into());
        }
        let bounds = translated_rect(primitive.bounds, primitive.transform)?;
        let mut clip = intersection(bounds, viewport);
        for parent in primitive.clips.iter() {
            if parent.corner_radius != 0.0 || parent.polygon_clip.is_some() {
                return Err("native content currently requires rectangular clipping".into());
            }
            clip = intersection(clip, translated_rect(parent.bounds, parent.transform)?);
        }
        if clip.width <= 0.0 || clip.height <= 0.0 {
            continue;
        }
        if !resources.insert(Arc::clone(&node.resource)) {
            return Err("a native resource cannot occupy two regions in one window".into());
        }
        regions.push(NativeContentRegion {
            node: primitive.node,
            resource: Arc::clone(&node.resource),
            generation: node.revision,
            bounds,
            clip,
        });
    }
    Ok(regions)
}

fn translated_rect(rect: SceneRect, transform: AffineTransform) -> Result<SceneRect, String> {
    let [a, b, c, d, x, y] = transform.0;
    if transform.is_projective()
        || [a, b, c, d] != [1.0, 0.0, 0.0, 1.0]
        || ![rect.x, rect.y, rect.width, rect.height, x, y]
            .iter()
            .all(|value| value.is_finite())
    {
        return Err("native content requires a finite translation-only transform".into());
    }
    Ok(SceneRect {
        x: rect.x + x,
        y: rect.y + y,
        ..rect
    })
}

fn intersection(a: SceneRect, b: SceneRect) -> SceneRect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    SceneRect {
        x,
        y,
        width: (a.x + a.width).min(b.x + b.width).max(x) - x,
        height: (a.y + a.height).min(b.y + b.height).max(y) - y,
    }
}

#[cfg(feature = "hosted")]
#[derive(Debug, Default)]
pub(crate) struct NativeContentRenderer {
    pipeline: Mutex<Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>>,
}

#[cfg(feature = "hosted")]
impl SceneGpuRenderer for NativeContentRenderer {
    fn prepare(&self, _: &SceneGpuNode, context: SceneGpuPrepareContext<'_>) {
        let mut cached = self.pipeline.lock().expect("native content pipeline");
        if cached
            .as_ref()
            .is_some_and(|(format, _)| *format == context.target_format)
        {
            return;
        }
        let shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("native content opening"),
                source: wgpu::ShaderSource::Wgsl(
                    r#"
@vertex fn vertex(@builtin(vertex_index) i:u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>,3>(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));
    return vec4(p[i],0.0,1.0);
}
@fragment fn fragment() -> @location(0) vec4<f32> { return vec4(0.0); }
"#
                    .into(),
                ),
            });
        let pipeline = context
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("native content opening"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: context.target_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        *cached = Some((context.target_format, pipeline));
    }

    fn render(&self, node: &SceneGpuNode, context: SceneGpuRenderContext<'_>) {
        if node.custom.param(0) == Some(0.0) {
            return;
        }
        let cached = self.pipeline.lock().expect("native content pipeline");
        let Some((_, pipeline)) = cached.as_ref() else {
            return;
        };
        let bounds = crate::gpu_view::intersect_physical(context.bounds, context.clip);
        if bounds.width == 0 || bounds.height == 0 {
            return;
        }
        let mut pass = context
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native content opening"),
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
        pass.set_pipeline(pipeline);
        pass.set_viewport(
            bounds.x as f32,
            bounds.y as f32,
            bounds.width as f32,
            bounds.height as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(bounds.x, bounds.y, bounds.width, bounds.height);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scene(opacity: f32, duplicate: bool) -> UiScene {
        let mut context = nana_ui_runtime::AppContext::new();
        let document = nana_ui_runtime::DocumentId::new(1).unwrap();
        let mut view = nana_ui_runtime::NativeContent::new("browser").generation(3);
        Arc::make_mut(&mut view.style.layout).opacity = Some(opacity);
        let first = context.create_component(document, view.clone()).unwrap();
        let second = duplicate.then(|| context.create_component(document, view).unwrap());
        let mut mutations = nana_ui_runtime::MutationQueue::new();
        for entity in [Some(first), second].into_iter().flatten() {
            mutations.write_layout(
                entity.stable_id(),
                nana_ui_runtime::LayoutBox {
                    x: 10.0,
                    y: 20.0,
                    width: 80.0,
                    height: 40.0,
                },
            );
        }
        context.commit_mutations(mutations).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        let mut scene = UiScene::new();
        scene.apply_delta(
            context.world().extract_nodes(&work.render_extraction),
            work.render_removals,
        );
        scene
    }
    #[test]
    fn native_regions_clip_to_window_and_reject_unsupported_composition() {
        let viewport = SceneRect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
        };
        let regions = native_content_regions(&scene(1.0, false), viewport).unwrap();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].generation, 3);
        assert_eq!(
            regions[0].clip,
            SceneRect {
                x: 10.0,
                y: 20.0,
                width: 40.0,
                height: 30.0
            }
        );
        assert!(native_content_regions(&scene(0.5, false), viewport).is_err());
        assert!(native_content_regions(&scene(1.0, true), viewport).is_err());
    }
}
