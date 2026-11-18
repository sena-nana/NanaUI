use nana_ui::{
    GpuContext, GpuRenderTarget, GpuTextureFormat, NanaTextShaper, ScenePaintViewport,
    SceneWgpuPainter, runtime::*, wgpu,
};
use nana_ui_scene::UiScene;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;
const SIZE: [u32; 2] = [128, 64];

fn gpu_context() -> GpuContext {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .expect("GPU tests require a WGPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("precompiled without runtime-wgsl"),
        ..Default::default()
    }))
    .expect("GPU tests require a WGPU device");
    GpuContext::from_wgpu(adapter, device, queue)
}

fn minimal_scene() -> UiScene {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let root = cx
        .create_component(
            doc,
            Stack::column(4.0)
                .padding(8.0)
                .surface(SemanticColorRole::Surface)
                .border(SemanticColorRole::Border, 1.0)
                .radius_px(6.0),
        )
        .unwrap();
    let label = cx.create_component(doc, Text::new("Precompiled")).unwrap();
    cx.append_child(root, label).unwrap();
    let viewport = LayoutViewport::new(SIZE[0] as f32, SIZE[1] as f32);
    cx.resolve_styles(&[root.stable_id(), label.stable_id()])
        .unwrap();
    cx.shape_text(&[label.stable_id()], &mut NanaTextShaper::default())
        .unwrap();
    cx.layout_document(doc, viewport).unwrap();
    let mut scene = UiScene::new();
    scene.apply_delta(cx.world().extract_document(doc), []);
    scene
}

#[test]
fn precompiled_shaders_build_and_paint_without_runtime_wgsl() {
    #[cfg(feature = "runtime-wgsl")]
    if std::env::var_os("NANA_EXPECT_NO_RUNTIME_WGSL").is_some() {
        panic!("NANA_EXPECT_NO_RUNTIME_WGSL is set, so `runtime-wgsl` must be disabled");
    }

    let gpu = gpu_context();
    let mut painter = SceneWgpuPainter::try_new(&gpu, GpuTextureFormat::from_wgpu(FORMAT))
        .expect("SceneWgpuPainter must build from precompiled shader assets alone");

    let texture = gpu
        .wgpu()
        .device()
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("precompiled test target"),
            size: wgpu::Extent3d {
                width: SIZE[0],
                height: SIZE[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let target = GpuRenderTarget::from_wgpu(&gpu, view, FORMAT, SIZE);
    let mut frame = gpu.begin_frame("precompiled without runtime-wgsl");
    let viewport = ScenePaintViewport {
        logical_size: [SIZE[0] as f32, SIZE[1] as f32],
        physical_size: SIZE,
        scale_factor: 1.0,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: [0.0, 0.0, 0.0, 1.0],
        clear: true,
    };
    painter
        .paint(&minimal_scene(), &mut frame, &target, viewport, None, None)
        .expect("painting minimal scene without runtime-wgsl must succeed");
    frame.submit();
}
