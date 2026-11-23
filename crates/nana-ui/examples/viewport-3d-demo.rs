//! Canonical offscreen 3D viewport example using `HostTexture` + `GpuTextureView`.
//!
//! Demonstrates:
//! - Offscreen 4x MSAA `Rgba16Float` + `Depth32Float` rendering in linear scRGB
//! - Resolving into a `GpuTexture::from_wgpu` wrapped in `HostTexture`
//! - Reading `HostTextureRegistry::painted_extent` (with physical-size fallback)
//! - `GpuContext::begin_producer_frame` (does not consume presentation frame slots)
//! - `rebuild_gpu` recovery across `DeviceGeneration` changes
//! - Orbit / zoom input scoped to `GpuTextureView` (`with_pointer_events(true)`)
//!
//! Run with:
//! `cargo run -p nana-ui --example viewport-3d-demo --features hosted,bundled-fonts,wgpu-interop,runtime-wgsl,view-macro`

use std::num::NonZeroU32;
use std::time::Instant;

use nana_ui::runtime::view::{Mount, Signal, css, signal, text, widget, with_refs};
use nana_ui::runtime::{
    Activate, Button, DocumentId, Entity, FrameworkError, GpuTextureView, LengthSpec, MountedView,
    RuntimeDocument, Stack,
};
use nana_ui::wgpu::{self, util::DeviceExt};
use nana_ui::{
    DeviceGeneration, FrameDemand, GpuContext, GpuTexture, HostTexture, HostTextureAlphaMode,
    HostTextureRegistry, RoutedInput, RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate,
    TextureSlot, WindowDescriptor, run_runtime,
};
use nana_ui_platform::{InputPayload, PointerPhase, WindowEvent, WindowId};

const VIEWPORT_SLOT: &str = "viewport-3d";
const SAMPLES: u32 = 4;
const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const SHADER_WGSL: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) world: vec3<f32>,
}

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = uniforms.view_proj * vec4<f32>(input.position, 1.0);
    out.normal = input.normal;
    out.color = input.color;
    out.world = input.position;
    return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(input.normal);
    let l = normalize(vec3<f32>(0.55, 0.85, 0.45));
    let v = normalize(uniforms.eye.xyz - input.world);
    let ndotl = max(dot(n, l), 0.0);
    let ambient = mix(vec3<f32>(0.16, 0.14, 0.13), vec3<f32>(0.34, 0.42, 0.54), n.y * 0.5 + 0.5);
    let rim = pow(1.0 - clamp(abs(dot(n, v)), 0.0, 1.0), 3.0) * 0.08;
    let rgb = input.color * (ambient + vec3<f32>(1.0, 0.96, 0.90) * ndotl * 0.85) + vec3<f32>(rim);
    return vec4<f32>(rgb, 1.0);
}
"#;

#[derive(Debug, Clone)]
enum Message {
    ResetCamera,
    ToggleSpin,
}

struct OffscreenTargets {
    size: [u32; 2],
    msaa_color: wgpu::TextureView,
    msaa_depth: wgpu::TextureView,
    resolve_view: wgpu::TextureView,
    host_texture: HostTexture,
}

struct Gpu3dState {
    generation: DeviceGeneration,
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
    targets: Option<OffscreenTargets>,
}

struct Viewport3dDemo {
    document: RuntimeDocument,
    viewport: Entity<GpuTextureView>,
    textures: HostTextureRegistry,
    slot: TextureSlot,
    gpu_state: Option<Gpu3dState>,
    yaw: f32,
    pitch: f32,
    distance: f32,
    spinning: bool,
    dragging: bool,
    last_pos: Option<(f32, f32)>,
    status: Signal<String>,
    last_wake: Instant,
}

impl Viewport3dDemo {
    fn hud(status: Signal<String>) -> impl Mount<Output = (MountedView, Entity<GpuTextureView>)> {
        let viewport = nana_ui::runtime::view::entity_ref::<GpuTextureView>();
        let root = widget(
            Stack::layers()
                .width(LengthSpec::Fill)
                .height(LengthSpec::Fill),
        )
        .children((
            widget(GpuTextureView::new(VIEWPORT_SLOT).with_pointer_events(true))
                .entity_ref(viewport),
            widget(Stack::column(8.0))
                .css(css! {
                    width: shrink;
                    padding: 12px 16px;
                    margin: 16px;
                    border-radius: 12px;
                    background-color: rgba(20, 22, 28, 0.86);
                })
                .children((
                    text(move || status.get()).css(css! { font-size: 13px; color: #f2f4f8; }),
                    widget(Stack::row(8.0)).children((
                        widget(Button::new("Reset View")).on_cx(|_, _: &Activate, cx| {
                            cx.dispatch_program(Message::ResetCamera);
                        }),
                        widget(Button::new("Toggle Spin")).on_cx(|_, _: &Activate, cx| {
                            cx.dispatch_program(Message::ToggleSpin);
                        }),
                    )),
                )),
        ));
        with_refs(root, viewport)
    }

    fn render_viewport(&mut self, gpu: &GpuContext, fallback_size: [u32; 2]) {
        let max_dim = gpu.wgpu().device().limits().max_texture_dimension_2d.max(1);
        let raw = self
            .textures
            .painted_extent(VIEWPORT_SLOT)
            .unwrap_or(fallback_size);
        let size = [raw[0].clamp(1, max_dim), raw[1].clamp(1, max_dim)];

        let generation = gpu.generation();
        let mut replaced = false;
        if self
            .gpu_state
            .as_ref()
            .is_none_or(|s| s.generation != generation)
        {
            self.gpu_state = Some(build_gpu_state(gpu));
            replaced = true;
        }
        let state = self.gpu_state.as_mut().expect("gpu state");
        if state.targets.as_ref().is_none_or(|t| t.size != size) {
            state.targets = Some(build_targets(gpu, size));
            replaced = true;
        }

        let aspect = size[0] as f32 / size[1] as f32;
        let (vp, eye) = camera_matrices(self.yaw, self.pitch, self.distance, aspect);
        let mut uniform_floats = [0.0f32; 20];
        uniform_floats[..16].copy_from_slice(&vp);
        uniform_floats[16..20].copy_from_slice(&[eye[0], eye[1], eye[2], 1.0]);
        gpu.wgpu()
            .queue()
            .write_buffer(&state.uniform_buf, 0, f32_bytes(&uniform_floats));

        let targets = state.targets.as_ref().expect("targets");
        let mut frame = gpu.begin_producer_frame("viewport-3d-demo");
        {
            let encoder = frame.wgpu_encoder();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport3d msaa pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa_color,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.025,
                            g: 0.032,
                            b: 0.045,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.msaa_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&state.pipeline);
            pass.set_bind_group(0, &state.uniform_group, &[]);
            pass.set_vertex_buffer(0, state.vertex_buf.slice(..));
            pass.set_index_buffer(state.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..state.index_count, 0, 0..1);
        }
        let _ = frame.submit();

        if replaced {
            self.slot.replace(
                targets.host_texture.clone(),
                size[0],
                size[1],
                HostTextureAlphaMode::Opaque,
            );
        } else {
            self.slot.invalidate();
        }
        self.status.set(format!(
            "HostTexture 3D ({}x{}) · Drag to orbit, wheel to zoom",
            size[0], size[1]
        ));
    }
}

impl RuntimeProgram for Viewport3dDemo {
    type Message = Message;
    type Error = FrameworkError;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let status = signal("HostTexture 3D · Drag to orbit, wheel to zoom".to_string());
        let doc_id = DocumentId::allocate();
        let mut document = RuntimeDocument::new(doc_id);
        let (_, viewport) = document
            .context_mut()
            .mount_view_root(doc_id, || Self::hud(status))?;
        let textures = HostTextureRegistry::new();
        let slot = textures.slot(VIEWPORT_SLOT);
        let phys = context.content_geometry().physical_size;
        let mut app = Self {
            document,
            viewport,
            textures,
            slot,
            gpu_state: None,
            yaw: 0.65,
            pitch: 0.42,
            distance: 3.4,
            spinning: true,
            dragging: false,
            last_pos: None,
            status,
            last_wake: Instant::now(),
        };
        app.render_viewport(context.gpu(), [phys.0.max(1), phys.1.max(1)]);
        Ok((app, Vec::new()))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&self.document)))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&mut self.document)))
    }

    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        Some(self.textures.clone())
    }

    fn prepare_window_frame(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if id != WindowId::PRIMARY {
            return;
        }
        let phys = context.content_geometry().physical_size;
        self.render_viewport(context.gpu(), [phys.0.max(1), phys.1.max(1)]);
    }

    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        self.gpu_state = None;
        let phys = context.content_geometry().physical_size;
        self.render_viewport(context.gpu(), [phys.0.max(1), phys.1.max(1)]);
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match message {
            Message::ResetCamera => {
                self.yaw = 0.65;
                self.pitch = 0.42;
                self.distance = 3.4;
            }
            Message::ToggleSpin => {
                self.spinning = !self.spinning;
                self.last_wake = Instant::now();
            }
        }
        RuntimeProgramUpdate::redraw(WindowId::PRIMARY)
    }

    fn frame_demand(&self, _id: WindowId) -> FrameDemand {
        if self.spinning {
            FrameDemand::Continuous(NonZeroU32::new(60).expect("non-zero"))
        } else {
            FrameDemand::OnDemand
        }
    }

    fn wake(
        &mut self,
        now: Instant,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        let dt = (now - self.last_wake).as_secs_f32().clamp(0.0, 0.1);
        self.last_wake = now;
        if self.spinning {
            self.yaw += dt * 0.7;
            return RuntimeProgramUpdate::redraw(WindowId::PRIMARY);
        }
        RuntimeProgramUpdate::default()
    }

    fn input_event(
        &mut self,
        id: WindowId,
        input: RoutedInput<'_>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let on_view = input.pointer_hit == Some(self.viewport.stable_id());
        match &input.event.payload {
            InputPayload::Pointer(p) => match p.phase {
                PointerPhase::Down if on_view => {
                    self.dragging = true;
                    self.last_pos = Some((p.x, p.y));
                }
                PointerPhase::Move if self.dragging => {
                    if let Some((lx, ly)) = self.last_pos {
                        self.yaw -= (p.x - lx) * 0.008;
                        self.pitch = (self.pitch + (p.y - ly) * 0.008).clamp(-1.45, 1.45);
                    }
                    self.last_pos = Some((p.x, p.y));
                    return Ok(RuntimeProgramUpdate::redraw(id));
                }
                PointerPhase::Up => {
                    self.dragging = false;
                    self.last_pos = None;
                }
                _ => {}
            },
            InputPayload::Wheel(w) if on_view => {
                self.distance = (self.distance * (-w.delta_y * 0.1).exp()).clamp(1.2, 20.0);
                return Ok(RuntimeProgramUpdate::redraw(id));
            }
            _ => {}
        }
        Ok(RuntimeProgramUpdate::default())
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::Ready { id, .. } | WindowEvent::Resized { id, .. } => {
                RuntimeProgramUpdate::redraw(id)
            }
            _ => RuntimeProgramUpdate::default(),
        }
    }
}

fn build_gpu_state(gpu: &GpuContext) -> Gpu3dState {
    let device = gpu.wgpu().device();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("viewport3d demo shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
    });
    let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport3d demo uniforms"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("viewport3d demo pipe layout"),
        bind_group_layouts: &[Some(&uniform_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("viewport3d demo pipeline"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 36,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 12,
                        shader_location: 1,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 24,
                        shader_location: 2,
                    },
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: SCENE_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: SAMPLES,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    });
    let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport3d demo uniform buf"),
        size: 80,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport3d demo uniform group"),
        layout: &uniform_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buf.as_entire_binding(),
        }],
    });
    let (verts, indices) = cube_and_pedestal();
    let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("viewport3d demo vb"),
        contents: f32_bytes(&verts),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("viewport3d demo ib"),
        contents: u32_bytes(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    Gpu3dState {
        generation: gpu.generation(),
        pipeline,
        uniform_buf,
        uniform_group,
        vertex_buf,
        index_buf,
        index_count: indices.len() as u32,
        targets: None,
    }
}

fn build_targets(gpu: &GpuContext, size: [u32; 2]) -> OffscreenTargets {
    let device = gpu.wgpu().device();
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    let msaa_color = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("viewport3d demo msaa"),
            size: extent,
            mip_level_count: 1,
            sample_count: SAMPLES,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default());
    let msaa_depth = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("viewport3d demo depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: SAMPLES,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default());
    let resolve_raw = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport3d demo resolve"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SCENE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let resolve_view = resolve_raw.create_view(&wgpu::TextureViewDescriptor::default());
    let output = GpuTexture::from_wgpu(gpu, resolve_raw);
    let host_texture = HostTexture::new(&output);
    OffscreenTargets {
        size,
        msaa_color,
        msaa_depth,
        resolve_view,
        host_texture,
    }
}

fn cube_and_pedestal() -> (Vec<f32>, Vec<u32>) {
    let mut verts = Vec::new();
    let mut indices = Vec::new();
    let mut push_box = |center: [f32; 3], half: [f32; 3], color: [f32; 3]| {
        let [cx, cy, cz] = center;
        let [hx, hy, hz] = half;
        let faces = [
            (
                [0.0, 0.0, 1.0],
                [
                    [cx - hx, cy - hy, cz + hz],
                    [cx + hx, cy - hy, cz + hz],
                    [cx + hx, cy + hy, cz + hz],
                    [cx - hx, cy + hy, cz + hz],
                ],
            ),
            (
                [0.0, 0.0, -1.0],
                [
                    [cx + hx, cy - hy, cz - hz],
                    [cx - hx, cy - hy, cz - hz],
                    [cx - hx, cy + hy, cz - hz],
                    [cx + hx, cy + hy, cz - hz],
                ],
            ),
            (
                [1.0, 0.0, 0.0],
                [
                    [cx + hx, cy - hy, cz + hz],
                    [cx + hx, cy - hy, cz - hz],
                    [cx + hx, cy + hy, cz - hz],
                    [cx + hx, cy + hy, cz + hz],
                ],
            ),
            (
                [-1.0, 0.0, 0.0],
                [
                    [cx - hx, cy - hy, cz - hz],
                    [cx - hx, cy - hy, cz + hz],
                    [cx - hx, cy + hy, cz + hz],
                    [cx - hx, cy + hy, cz - hz],
                ],
            ),
            (
                [0.0, 1.0, 0.0],
                [
                    [cx - hx, cy + hy, cz + hz],
                    [cx + hx, cy + hy, cz + hz],
                    [cx + hx, cy + hy, cz - hz],
                    [cx - hx, cy + hy, cz - hz],
                ],
            ),
            (
                [0.0, -1.0, 0.0],
                [
                    [cx - hx, cy - hy, cz - hz],
                    [cx + hx, cy - hy, cz - hz],
                    [cx + hx, cy - hy, cz + hz],
                    [cx - hx, cy - hy, cz + hz],
                ],
            ),
        ];
        for (n, corners) in faces {
            let base = (verts.len() / 9) as u32;
            for p in corners {
                verts.extend_from_slice(&[
                    p[0], p[1], p[2], n[0], n[1], n[2], color[0], color[1], color[2],
                ]);
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    };
    push_box([0.0, -0.58, 0.0], [1.1, 0.08, 1.1], [0.24, 0.28, 0.36]);
    push_box([0.0, 0.0, 0.0], [0.5, 0.5, 0.5], [0.92, 0.45, 0.34]);
    (verts, indices)
}

fn camera_matrices(yaw: f32, pitch: f32, distance: f32, aspect: f32) -> ([f32; 16], [f32; 3]) {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let eye = [cp * sy * distance, sp * distance, cp * cy * distance];
    let f = normalize3([-eye[0], -eye[1], -eye[2]]);
    let s = normalize3(cross3(f, [0.0, 1.0, 0.0]));
    let u = cross3(s, f);
    let view = [
        s[0],
        u[0],
        -f[0],
        0.0,
        s[1],
        u[1],
        -f[1],
        0.0,
        s[2],
        u[2],
        -f[2],
        0.0,
        -dot3(s, eye),
        -dot3(u, eye),
        dot3(f, eye),
        1.0,
    ];
    let fov = 42.0f32.to_radians();
    let t = (fov * 0.5).tan();
    let near = 0.05f32;
    let far = 100.0f32;
    let proj = [
        1.0 / (aspect.max(1e-3) * t),
        0.0,
        0.0,
        0.0,
        0.0,
        1.0 / t,
        0.0,
        0.0,
        0.0,
        0.0,
        far / (near - far),
        -1.0,
        0.0,
        0.0,
        (near * far) / (near - far),
        0.0,
    ];
    (mul4(proj, view), eye)
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = dot3(v, v).sqrt().max(1e-6);
    [v[0] / len, v[1] / len, v[2] / len]
}

fn mul4(a: [f32; 16], b: [f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = a[row] * b[col * 4]
                + a[4 + row] * b[col * 4 + 1]
                + a[8 + row] * b[col * 4 + 2]
                + a[12 + row] * b[col * 4 + 3];
        }
    }
    out
}

fn f32_bytes(values: &[f32]) -> &[u8] {
    let ptr = values.as_ptr().cast::<u8>();
    // SAFETY: `f32` has no padding and valid alignment for byte slices.
    unsafe { std::slice::from_raw_parts(ptr, std::mem::size_of_val(values)) }
}

fn u32_bytes(values: &[u32]) -> &[u8] {
    let ptr = values.as_ptr().cast::<u8>();
    // SAFETY: `u32` has no padding and valid alignment for byte slices.
    unsafe { std::slice::from_raw_parts(ptr, std::mem::size_of_val(values)) }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    run_runtime::<Viewport3dDemo>(
        WindowDescriptor::new("NanaUI 3D Viewport Demo (HostTexture)").initial_size(1024.0, 720.0),
    )
}
