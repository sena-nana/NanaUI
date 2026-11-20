//! The playfield presentation: one `GpuView` anchor in the UI tree, drawn by
//! an application [`SceneGpuRenderer`] as one instanced draw over every body.
//!
//! The renderer never reads the application's object model. The program
//! publishes a flattened copy ([`Playfield::publish`]) after each fixed step;
//! the renderer uploads it once per published revision, inside the painter's
//! pass, on the host's device and encoder. Preparation (pipeline, sprite,
//! bindings) is built once per device generation and target format and then
//! reused: [`SceneGpuRenderer::preparation_version`] stays constant while
//! [`SceneGpuRenderer::content_version`] says the pixels are live.
//!
//! Recording the draw needs `wgpu-interop` ([`ScenePass::wgpu`]): NanaUI has a
//! logical resource ABI but no backend-neutral pipeline or draw call for
//! custom renderers yet.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nana_ui::runtime::CustomRenderNode;
use nana_ui::wgpu;
use nana_ui::{
    DeviceGeneration, GpuContext, GpuError, GpuTexture, GpuTextureDescriptor, GpuTextureFormat,
    GpuTextureRegion, GpuTextureUsages, SceneGpuNode, SceneGpuPassContext, SceneGpuPrepareContext,
    SceneGpuRenderContext, SceneGpuRenderer, ScenePass,
};

use crate::sim::Instance;

/// The `GpuView` slot the playfield occupies.
pub const PLAYFIELD_SLOT: u64 = 1;

/// Edge of the sprite every body is drawn with.
pub const SPRITE_EXTENT: u32 = 32;

/// The one image resource: a soft disc, premultiplied linear RGBA8. Decoded
/// (here: generated) once on the CPU; realized once per device generation and
/// shared by the overlay's `GpuTextureView` and the playfield renderer.
#[derive(Debug)]
pub struct SpriteAsset {
    pixels: Vec<u8>,
}

impl SpriteAsset {
    pub fn load() -> Self {
        let extent = SPRITE_EXTENT as usize;
        let mut pixels = Vec::with_capacity(extent * extent * 4);
        let center = SPRITE_EXTENT as f32 / 2.0;
        for y in 0..extent {
            for x in 0..extent {
                let distance = ((x as f32 + 0.5 - center).hypot(y as f32 + 0.5 - center)) / center;
                let alpha = (1.0 - distance).clamp(0.0, 1.0).powf(0.6);
                let value = (alpha * 255.0).round() as u8;
                // Premultiplied white: colour equals coverage.
                pixels.extend_from_slice(&[value, value, value, value]);
            }
        }
        Self { pixels }
    }

    fn realize(&self, gpu: &GpuContext) -> Result<GpuTexture, GpuError> {
        let texture = gpu.create_texture(&GpuTextureDescriptor {
            label: Some("realtime fixture sprite"),
            width: SPRITE_EXTENT,
            height: SPRITE_EXTENT,
            format: GpuTextureFormat::RGBA8_UNORM,
            usage: GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST,
        })?;
        gpu.write_texture(
            &texture,
            GpuTextureRegion::full(SPRITE_EXTENT, SPRITE_EXTENT),
            &self.pixels,
            SPRITE_EXTENT * 4,
        )?;
        Ok(texture)
    }
}

/// Cumulative renderer work, for the fixture's evidence.
#[derive(Debug, Default)]
struct Counters {
    prepares: AtomicU64,
    draws: AtomicU64,
    instance_uploads: AtomicU64,
    pipeline_builds: AtomicU64,
    sprite_realizations: AtomicU64,
}

/// A copy of [`Counters`] at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderStats {
    /// [`SceneGpuRenderer::prepare`] calls. Stays at one per device and
    /// target format while the scene stands still.
    pub prepares: u64,
    /// Instanced draws recorded: one per painted frame.
    pub draws: u64,
    /// Instance-buffer uploads: one per published revision a frame painted.
    pub instance_uploads: u64,
    /// Pipelines built (device generation x target format).
    pub pipeline_builds: u64,
    /// GPU realizations of the sprite (one per device generation).
    pub sprite_realizations: u64,
}

impl Counters {
    fn snapshot(&self) -> RenderStats {
        RenderStats {
            prepares: self.prepares.load(Ordering::Relaxed),
            draws: self.draws.load(Ordering::Relaxed),
            instance_uploads: self.instance_uploads.load(Ordering::Relaxed),
            pipeline_builds: self.pipeline_builds.load(Ordering::Relaxed),
            sprite_realizations: self.sprite_realizations.load(Ordering::Relaxed),
        }
    }
}

/// The latest published state, as the renderer reads it.
#[derive(Debug, Default)]
struct Published {
    revision: u64,
    count: u32,
    bytes: Vec<u8>,
}

/// What the program and the renderer share: the latest instance copy and
/// the sprite's realization. Not part of the UI tree.
#[derive(Debug)]
pub struct Playfield {
    published: Mutex<Published>,
    sprite: SpriteAsset,
    sprite_texture: Mutex<Option<GpuTexture>>,
    counters: Counters,
}

impl Playfield {
    pub fn new(sprite: SpriteAsset) -> Arc<Self> {
        Arc::new(Self {
            published: Mutex::new(Published::default()),
            sprite,
            sprite_texture: Mutex::new(None),
            counters: Counters::default(),
        })
    }

    /// Publish the state after a fixed step. Cheap: a copy into a reused
    /// buffer; the GPU upload happens on the next painted frame, once.
    pub fn publish(&self, instances: &[Instance]) {
        let mut published = self.published.lock().expect("playfield state");
        published.revision += 1;
        published.count = instances.len() as u32;
        published.bytes.clear();
        for instance in instances {
            instance.write_bytes(&mut published.bytes);
        }
    }

    pub fn revision(&self) -> u64 {
        self.published.lock().expect("playfield state").revision
    }

    pub fn stats(&self) -> RenderStats {
        self.counters.snapshot()
    }

    /// The sprite on `gpu`, realized on first use per device generation.
    pub fn sprite_texture(&self, gpu: &GpuContext) -> Result<GpuTexture, GpuError> {
        let mut slot = self.sprite_texture.lock().expect("sprite texture");
        if let Some(texture) = slot
            .as_ref()
            .filter(|texture| texture.generation() == gpu.generation())
        {
            return Ok(texture.clone());
        }
        let texture = self.sprite.realize(gpu)?;
        self.counters
            .sprite_realizations
            .fetch_add(1, Ordering::Relaxed);
        *slot = Some(texture.clone());
        Ok(texture)
    }
}

/// Device objects for one (device generation, target format).
struct Prepared {
    generation: DeviceGeneration,
    format: GpuTextureFormat,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    view: wgpu::Buffer,
    view_written: Option<[f32; 8]>,
    instances: wgpu::Buffer,
    capacity: u64,
    uploaded: Option<u64>,
    count: u32,
    /// Keeps the sampled sprite alive as long as the bind group.
    _sprite: GpuTexture,
}

/// The playfield's [`SceneGpuRenderer`], registered for `"gpu-view"`.
pub struct PlayfieldRenderer {
    playfield: Arc<Playfield>,
    prepared: Mutex<Vec<Prepared>>,
}

impl std::fmt::Debug for PlayfieldRenderer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlayfieldRenderer")
            .finish_non_exhaustive()
    }
}

impl PlayfieldRenderer {
    pub fn new(playfield: Arc<Playfield>) -> Self {
        Self {
            playfield,
            prepared: Mutex::new(Vec::new()),
        }
    }

    fn ours(node: &CustomRenderNode) -> bool {
        node.resource.as_ref() == PLAYFIELD_SLOT.to_string()
    }

    /// The entry for `gpu` and `format`, built on first use. Entries of a
    /// replaced device are dropped with it.
    fn prepared<'a>(
        &self,
        prepared: &'a mut Vec<Prepared>,
        gpu: &GpuContext,
        format: GpuTextureFormat,
    ) -> Option<&'a mut Prepared> {
        let generation = gpu.generation();
        prepared.retain(|entry| entry.generation == generation);
        let index = match prepared.iter().position(|entry| entry.format == format) {
            Some(index) => index,
            None => {
                let built = self.build(gpu, format)?;
                prepared.push(built);
                prepared.len() - 1
            }
        };
        prepared.get_mut(index)
    }

    fn build(&self, gpu: &GpuContext, format: GpuTextureFormat) -> Option<Prepared> {
        let sprite = self.playfield.sprite_texture(gpu).ok()?;
        let device = gpu.wgpu().device();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("realtime fixture playfield"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("realtime fixture playfield"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("realtime fixture playfield"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("realtime fixture playfield"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: Instance::STRIDE as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: format.wgpu(),
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
        let view = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("realtime fixture playfield view"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("realtime fixture sprite"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("realtime fixture playfield"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(sprite.wgpu_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let capacity = 128 * Instance::STRIDE as u64;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("realtime fixture instances"),
            size: capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.playfield
            .counters
            .pipeline_builds
            .fetch_add(1, Ordering::Relaxed);
        Some(Prepared {
            generation: gpu.generation(),
            format,
            pipeline,
            bind_group,
            view,
            view_written: None,
            instances,
            capacity,
            uploaded: None,
            count: 0,
            _sprite: sprite,
        })
    }

    /// Bring the instance buffer up to the published revision. Runs at most
    /// once per revision however many frames present it.
    fn sync_instances(&self, gpu: &GpuContext, prepared: &mut Prepared) {
        let published = self.playfield.published.lock().expect("playfield state");
        if prepared.uploaded == Some(published.revision) {
            return;
        }
        let needed = published.bytes.len() as u64;
        if needed > prepared.capacity {
            prepared.capacity = needed.next_power_of_two();
            prepared.instances = gpu.wgpu().device().create_buffer(&wgpu::BufferDescriptor {
                label: Some("realtime fixture instances"),
                size: prepared.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if needed > 0 {
            gpu.wgpu()
                .queue()
                .write_buffer(&prepared.instances, 0, &published.bytes);
        }
        prepared.count = published.count;
        prepared.uploaded = Some(published.revision);
        self.playfield
            .counters
            .instance_uploads
            .fetch_add(1, Ordering::Relaxed);
    }
}

/// Nothing the renderer prepares depends on the node beyond its slot, so
/// preparation is reused until the device or target format changes.
const PREPARATION: u64 = 1;

impl SceneGpuRenderer for PlayfieldRenderer {
    fn preparation_version(&self, _node: &CustomRenderNode) -> Option<u64> {
        Some(PREPARATION)
    }

    /// The pixels follow the application's published state, not the node.
    fn content_version(&self, _node: &CustomRenderNode) -> Option<u64> {
        None
    }

    fn prepare(&self, node: &SceneGpuNode, context: SceneGpuPrepareContext<'_>) {
        if !Self::ours(&node.custom) {
            return;
        }
        self.playfield
            .counters
            .prepares
            .fetch_add(1, Ordering::Relaxed);
        let mut prepared = self.prepared.lock().expect("playfield pipelines");
        let _ = self.prepared(&mut prepared, context.gpu, context.target_format);
    }

    fn render(&self, node: &SceneGpuNode, mut context: SceneGpuRenderContext<'_>) {
        let pass_context = SceneGpuPassContext {
            gpu: context.gpu,
            target_format: context.target_format,
            presentation: context.presentation,
            bounds: context.bounds,
            clip: context.clip,
            dest_size: context.dest_size,
            gpu_work: context.gpu_work,
        };
        context.with_pass("realtime fixture playfield", |pass| {
            self.draw_in_pass(node, pass, pass_context);
        });
    }

    fn draw_in_pass(
        &self,
        node: &SceneGpuNode,
        pass: &mut ScenePass<'_, '_>,
        context: SceneGpuPassContext<'_>,
    ) -> bool {
        if !Self::ours(&node.custom) || context.bounds.width == 0 || context.bounds.height == 0 {
            return false;
        }
        let mut prepared = self.prepared.lock().expect("playfield pipelines");
        let Some(prepared) = self.prepared(&mut prepared, context.gpu, context.target_format)
        else {
            return false;
        };
        self.sync_instances(context.gpu, prepared);
        let view = [
            context.bounds.x as f32,
            context.bounds.y as f32,
            context.bounds.width as f32,
            context.bounds.height as f32,
            context.dest_size[0].max(1) as f32,
            context.dest_size[1].max(1) as f32,
            node.opacity,
            0.0,
        ];
        if prepared.view_written != Some(view) {
            let bytes: Vec<u8> = view.iter().flat_map(|word| word.to_le_bytes()).collect();
            context
                .gpu
                .wgpu()
                .queue()
                .write_buffer(&prepared.view, 0, &bytes);
            prepared.view_written = Some(view);
        }
        pass.set_scissor(context.clip);
        if prepared.count > 0 {
            let raw = pass.wgpu();
            raw.set_pipeline(&prepared.pipeline);
            raw.set_bind_group(0, &prepared.bind_group, &[]);
            raw.set_vertex_buffer(0, prepared.instances.slice(..));
            raw.draw(0..6, 0..prepared.count);
        }
        self.playfield
            .counters
            .draws
            .fetch_add(1, Ordering::Relaxed);
        true
    }
}

const SHADER: &str = r#"
struct View {
    // Playfield rect in destination pixels, then destination size and opacity.
    rect: vec4<f32>,
    dest: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var sprite: texture_2d<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vertex_main(
    @builtin(vertex_index) vertex: u32,
    @location(0) body: vec4<f32>,
    @location(1) color: vec4<f32>,
) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex];
    let radius = body.z * min(view.rect.z, view.rect.w);
    let center = view.rect.xy + body.xy * view.rect.zw;
    let pixel = center + (corner * 2.0 - vec2<f32>(1.0)) * radius;
    var output: VertexOutput;
    output.position = vec4<f32>(
        pixel.x / view.dest.x * 2.0 - 1.0,
        1.0 - pixel.y / view.dest.y * 2.0,
        0.0,
        1.0,
    );
    output.uv = corner;
    output.color = vec4<f32>(color.rgb, color.a * view.dest.z);
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // The sprite is premultiplied linear; tint it and keep it premultiplied.
    let texel = textureSample(sprite, sprite_sampler, input.uv);
    return texel * vec4<f32>(input.color.rgb * input.color.a, input.color.a);
}
"#;
