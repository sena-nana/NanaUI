//! 富文本同一区间叠加排版、装饰和阴影效果的 GPU 回归验证。

use super::*;
use nana_text::{
    TextConstraints, TextEngine, TextKind, TextSource, TextSpan, TextStyle, TextWorkCounters,
};

/// 复现编辑器草稿中的组合，反复改变效果以覆盖保留实例扩缩容。
#[test]
fn stacked_editor_effects_rebuild_without_corrupting_instances() {
    let (device, queue) = crate::test_gpu::device();
    let mut pipeline = TextPipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let content = "1231231231231";
    let paint = nana_ui_core::PaintColor::srgb;
    let color = paint([14.0 / 255.0, 184.0 / 255.0, 134.0 / 255.0, 1.0]);
    let style = TextStyle {
        font_size_px: 20.0,
        ..TextStyle::default()
    };
    let mut source = TextSource::new(content);
    source.set_spans(vec![TextSpan {
        range: 2..content.len(),
        style: TextStyle {
            font_size_px: 40.0,
            font_weight: 950,
            italic: true,
            letter_spacing_px: 40.0,
            variations: vec![nana_ui_core::FontVariationSetting {
                tag: *b"wdth",
                value: 200.0,
            }],
            ..style.clone()
        },
        composition: None,
    }]);
    let layout = crate::text_engine::lock_engine(&pipeline.engine).layout(
        TextKind::Editable,
        &source,
        &style,
        &TextConstraints {
            max_width_px: Some(1100.0),
            preserve_lines: true,
            ..TextConstraints::default()
        },
        &mut TextWorkCounters::default(),
    );
    let handle = nana_ui_runtime::RetainedTextLayout {
        id: layout.id,
        layout,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("stacked rich effects target"),
        size: wgpu::Extent3d {
            width: 1200,
            height: 160,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    for frame in 0..48 {
        let emphasis = frame % 2 == 1;
        let effects = SceneRichPaint::new(
            SceneTextEffects::default(),
            vec![nana_ui_scene::SceneRichRun {
                range: 2..content.len(),
                effects: SceneTextEffects {
                    decoration: nana_ui_core::TextDecorationLine {
                        underline: true,
                        line_through: true,
                    },
                    emphasis,
                    shadows: Arc::from([nana_ui_core::RichTextShadow::new(
                        [0.0; 2],
                        10.0,
                        paint([0.0, 0.0, 0.0, 1.0]),
                    )]),
                    ..SceneTextEffects::default()
                },
            }],
        );
        pipeline.begin_frame([1200, 160]);
        let mut prepared = Vec::new();
        for node in 1..=frame / 2 + 2 {
            pipeline.set_effects(effects.as_ref(), None);
            prepared.push(
                pipeline
                    .prepare(
                        &device,
                        LogicalRect::from_xywh(0.0, 0.0, 1100.0, 120.0),
                        LogicalRect::from_xywh(0.0, 0.0, 1200.0, 160.0),
                        1.0,
                        content,
                        Some([1.0; 4]),
                        20.0,
                        None,
                        None,
                        None,
                        true,
                        nana_ui_core::TextWrapBreak::Word,
                        false,
                        false,
                        None,
                        TextShaping::Auto,
                        TextHorizontalAlignment::Start,
                        TextVerticalAlignment::Top,
                        &[SceneTextSpan {
                            start: 2,
                            end: content.len(),
                            color,
                        }],
                        0.0,
                        &[],
                        &SceneTextOpenType {
                            preserve_lines: true,
                            ..SceneTextOpenType::default()
                        },
                        clip::IDENTITY_AFFINE,
                        [0.0; 2],
                        clip::FragmentClip::PASS,
                        1.0,
                        [0.0; 2],
                        EntryKey {
                            node,
                            slot: 0,
                            pass: 0,
                        },
                        UNTRACKED_REVISION,
                        Some(&handle),
                        true,
                    )
                    .expect("叠加样式后仍生成文字绘制结果"),
            );
            if prepared.len() > 1 {
                let last = prepared.last().unwrap();
                assert!(pipeline.can_merge_runs(&prepared[0], last));
                pipeline.merge_runs(&prepared[0], last);
            }
        }
        pipeline.flush_runs();
        pipeline.upload(&device, &queue, None);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stacked rich effects pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for prepared in prepared.iter().take(1) {
                pipeline.draw(
                    &mut pass,
                    prepared,
                    PhysicalRect {
                        x: 0,
                        y: 0,
                        width: 1200,
                        height: 160,
                    },
                    None,
                );
            }
        }
        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("绘制队列正常完成");
        let id = pipeline
            .target
            .entries
            .lookup(EntryKey {
                node: 1,
                slot: 0,
                pass: 0,
            })
            .unwrap();
        let entry = pipeline.target.entries.get(id).unwrap();
        assert!(entry.glyphs > 0);
        assert!(entry.glyphs <= entry.capacity);
    }
}
