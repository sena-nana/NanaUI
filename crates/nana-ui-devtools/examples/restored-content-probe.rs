//! Reproducible one-tree image, Markdown and resize acceptance.
use nana_ui::{
    HostTexture, HostTextureAlphaMode, HostTextureRegistry, RuntimeInputAdapter, runtime::*,
};
use nana_ui_devtools::{
    agent::RuntimeAgentSession,
    offscreen::{self, OffscreenSnapshots, Size},
};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};
use std::path::Path;

fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputEvent {
    InputEvent::Pointer {
        phase,
        pointer_id: 1,
        pointer_type: PointerType::Mouse,
        x,
        y,
        screen_x: x,
        screen_y: y,
        button: 0,
        buttons: u16::from(phase != PointerPhase::Up),
        pressure: 1.0,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: true,
        activation_click: false,
        modifiers: InputModifiers::default(),
    }
}

fn capture(
    gpu: &mut OffscreenSnapshots,
    session: &mut RuntimeAgentSession,
    registry: &HostTextureRegistry,
    path: &Path,
    viewport: (u32, u32, f32),
) -> Result<(), Box<dyn std::error::Error>> {
    session.flush()?;
    let renderers = gpu.default_gpu_renderers();
    let (width, height, scale) = viewport;
    let size = Size::new(
        (width as f32 * scale) as u32,
        (height as f32 * scale) as u32,
    );
    let pixels = gpu.paint_layers_scaled(
        &[(session.document().scene(), true)],
        size,
        scale,
        session.clear_color(),
        Some(registry),
        Some(&renderers),
    )?;
    for text in session.document().scene().primitives().filter(|p| matches!(&p.kind, nana_ui_scene::ScenePrimitiveKind::Text { content, .. } if content == "deleted" || content == "link")) {
        let decoration = session.document().scene().primitives().find(|p| p.node == text.node && matches!(&p.kind, nana_ui_scene::ScenePrimitiveKind::Stroke { .. }) && (p.bounds.x - text.bounds.x).abs() < 0.1 && (p.bounds.width - text.bounds.width).abs() < 0.1).expect("text decoration must enter Scene");
        let x = ((decoration.bounds.x + decoration.bounds.width * 0.5) * scale) as usize;
        let y = ((decoration.bounds.y + decoration.bounds.height * 0.5) * scale) as usize;
        let clear = session.clear_color();
        assert!((y.saturating_sub(1)..=y+1).any(|row| {
            let offset = (row * size.width as usize + x) * 4;
            pixels.get(offset..offset+3).is_some_and(|rgb| (0..3).any(|i| (f32::from(rgb[i])/255.0-clear[i]).abs() > 0.2))
        }), "decoration must paint real pixels");
    }
    offscreen::write_png(path, size, &pixels)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/restored-content-probe".into());
    let output = Path::new(&output);
    std::fs::create_dir_all(output)?;
    let mut gpu = OffscreenSnapshots::new()?;
    let textures = HostTextureRegistry::new();
    let extent = wgpu::Extent3d {
        width: 120,
        height: 72,
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("intrinsic-size fixture"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels = (0..72)
        .flat_map(|y| {
            (0..120).flat_map(move |x| {
                [
                    30 + x as u8,
                    70 + (y * 2) as u8,
                    if (x / 12 + y / 12) % 2 == 0 { 220 } else { 150 },
                    255,
                ]
            })
        })
        .collect::<Vec<_>>();
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(480),
            rows_per_image: Some(72),
        },
        extent,
    );
    textures.register(
        "fixture",
        HostTexture::from_wgpu(501, 1, texture.create_view(&Default::default())),
        120,
        72,
        HostTextureAlphaMode::Opaque,
    );
    for viewport in [(760, 680, 1.0), (380, 720, 2.0)] {
        let (width, height, scale) = viewport;
        for (theme_name, theme) in [
            ("light", nana_ui::ThemeMode::Light),
            ("dark", nana_ui::ThemeMode::Dark),
        ] {
            let name = format!("{theme_name}-{width}x{height}-{scale}x");
            let id = DocumentId::new(91).unwrap();
            let mut document = RuntimeDocument::new(id);
            let cx = document.context_mut();
            cx.set_theme(theme)?;
            let root = cx.create_component(id, Stack::column(18.0).padding(24.0))?;
            let mut markdown = NativeMarkdown::from_source(
                "# Markdown 图片与公式\n\n**Bold** *italic* ~~deleted~~ `code` [link](https://example.test)\n\n行内公式 $E=mc^2$ 与中文。\n\n$$\\frac{1}{\\sqrt{x^2+1}}$$\n\n```mermaid\nflowchart LR\nA[输入 Input] --> B[绘制 Scene]\n```\n\n![resolved preview](fixture.svg)",
            );
            assert!(markdown.resolve_image("fixture.svg", "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='180'%20height='64'%3E%3Crect%20width='180'%20height='64'%20rx='8'%20fill='%232f8bbb'/%3E%3Ccircle%20cx='145'%20cy='32'%20r='20'%20fill='%23f3c568'/%3E%3C/svg%3E", 180, 64));
            let body = cx.create_detached_component(id, markdown)?;
            cx.append_child(root, body)?;
            let area = cx.create_detached_component(
                id,
                TextArea::new("拖动右下角调整高度 / Drag the corner")
                    .height(90.0)
                    .resize_vertical(true),
            )?;
            cx.append_child(root, area)?;
            let mut session = RuntimeAgentSession::new_scaled(document, width, height, scale)?;
            capture(
                &mut gpu,
                &mut session,
                &textures,
                &output.join(format!("markdown-resize-{name}-before.png")),
                viewport,
            )?;
            let world = session.document().context().world();
            let initial = world.layout_box(area.stable_id()).unwrap().height;
            let ComponentGeometry::TextInput {
                resize_grip: Some(grip),
                ..
            } = world.component_geometry(area.stable_id()).unwrap()
            else {
                panic!("resize grip");
            };
            let (x, y) = world
                .layout_pointer_position(area.stable_id(), grip.x + 7.0, grip.y + 7.0)
                .unwrap();
            let mut adapter = RuntimeInputAdapter::default();
            for (phase, offset) in [
                (PointerPhase::Down, 0.0),
                (PointerPhase::Move, 55.0),
                (PointerPhase::Up, 55.0),
            ] {
                adapter.dispatch(
                    session.document_mut().context_mut(),
                    id,
                    &pointer(phase, x, y + offset),
                )?;
            }
            session.flush()?;
            assert!(
                (session
                    .document()
                    .context()
                    .world()
                    .layout_box(area.stable_id())
                    .unwrap()
                    .height
                    - initial
                    - 55.0)
                    .abs()
                    < 0.1
            );
            capture(
                &mut gpu,
                &mut session,
                &textures,
                &output.join(format!("markdown-resize-{name}-after.png")),
                viewport,
            )?;

            let mut document = RuntimeDocument::new(id);
            document.context_mut().set_theme(theme)?;
            let viewer = document.context_mut().create_component(
                id,
                ImageViewer::new(ImageViewerContent::host_texture("fixture"))
                    .intrinsic_size(120, 72)
                    .name("120 × 72 intrinsic texture")
                    .metadata("Native size • zoom and pan"),
            )?;
            let mut session = RuntimeAgentSession::new_scaled(document, width, height, scale)?;
            capture(
                &mut gpu,
                &mut session,
                &textures,
                &output.join(format!("viewer-{name}-native.png")),
                viewport,
            )?;
            session
                .document_mut()
                .context_mut()
                .update_component(viewer, |view, _| view.zoom = 3.0)?;
            capture(
                &mut gpu,
                &mut session,
                &textures,
                &output.join(format!("viewer-{name}-zoom.png")),
                viewport,
            )?;

            // The consumer's long review message exercises the same bounded
            // user bubble and following action as LiliaCode's conversation.
            let mut document = RuntimeDocument::new(id);
            let cx = document.context_mut();
            cx.set_theme(theme)?;
            let root = cx.create_component(id, Stack::fill_column(18.0).padding(24.0))?;
            let bubble = cx.create_detached_component(
                id,
                Stack::column(6.0)
                    .padding_xy(14.0, 10.0)
                    .surface(SemanticColorRole::AccentSoft)
                    .radius(12.0)
                    .width(LengthSpec::FitContent)
                    .with_layout(|layout| {
                        layout.max_width = Some(LengthSpec::Min2(
                            nana_ui_core::LengthAtom::Percent(76.0),
                            nana_ui_core::LengthAtom::Px(620.0),
                        ));
                        layout.margin_left = Some(LengthSpec::Auto);
                    }),
            )?;
            cx.append_child(root, bubble)?;
            let mut markdown = NativeMarkdown::parse(
                "Review the changes introduced by commit \"0123456789abcdef0123456789abcdef01234567\" compared with its parent. Inspect the actual diff and enough surrounding code to verify each issue. Do not modify files. Prioritize actionable correctness, regression, security, and missing-test findings; include severity, file and line, trigger, and impact. If no actionable issues are found, say so and describe relevant validation limits. Report the findings in this conversation.\n\nAdditional user input:\n验证审批与草稿恢复",
            );
            let layout = std::sync::Arc::make_mut(&mut markdown.style.layout);
            layout.width = Some(LengthSpec::FitContent);
            layout.max_width = Some(LengthSpec::Percent(100.0));
            let markdown = cx.create_detached_component(id, markdown)?;
            cx.assemble_markdown(markdown)?;
            cx.append_child(bubble, markdown)?;
            let action = cx.create_detached_component(id, Button::new("复制"))?;
            cx.append_child(bubble, action)?;
            let mut session = RuntimeAgentSession::new_scaled(document, width, height, scale)?;
            session.flush()?;
            let cx = session.document().context();
            let world = cx.world();
            let bubble_bounds = world.layout_box(bubble.stable_id()).unwrap();
            let text_bounds = world.layout_box(markdown.stable_id()).unwrap();
            let action_bounds = world.layout_box(action.stable_id()).unwrap();
            let command_bottom = session
                .document()
                .scene()
                .primitives()
                .filter(|primitive| primitive.node == markdown.stable_id())
                .map(|primitive| primitive.bounds.y + primitive.bounds.height)
                .reduce(f32::max)
                .expect("Markdown must produce real Scene content");
            capture(
                &mut gpu,
                &mut session,
                &textures,
                &output.join(format!("constrained-markdown-{name}.png")),
                viewport,
            )?;
            assert!(
                command_bottom - text_bounds.y > 60.0,
                "long message must wrap"
            );
            // Scene text bounds include conservative line extents; require even those
            // extents to stay above the action, then verify the whole bubble contains it.
            assert!(action_bounds.y >= command_bottom);
            assert!(
                action_bounds.y + action_bounds.height + 10.0
                    <= bubble_bounds.y + bubble_bounds.height + 0.1
            );
            assert!(bubble_bounds.width <= ((width as f32 - 48.0) * 0.76).min(620.0) + 0.1);
            assert!(
                bubble_bounds.y + bubble_bounds.height <= height as f32 - 24.0 + 0.1,
                "complete bubble and action must be visible"
            );
        }
    }
    Ok(())
}
