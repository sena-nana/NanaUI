use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use wgpu;

use crate::geometry::{LogicalRect, PhysicalRect};
use crate::gpu_view::{RenderSlot, intersect_physical, slot_for_bounds};

const SOURCE: &str = r#"
@group(0) @binding(0)
var source: texture_2d<f32>;

@group(0) @binding(1)
var source_sampler: sampler;

struct LayerUniform {
    // opacity, corner radius (logical px), dest width, dest height
    params: vec4<f32>,
    // opaque flag, scale factor, dest tex width, dest tex height
    source: vec4<f32>,
    // CSS matrix a, b, c, d
    affine: vec4<f32>,
    // e, f, dest.x, dest.y
    origin: vec4<f32>,
    // rounded clip in the same pre-affine logical space as dest (sibling Quad)
    clip: vec4<f32>,
    // overflow parallelogram: local rect + inverse CSS matrix
    clip_rect: vec4<f32>,
    clip_inv_abcd: vec4<f32>,
    clip_inv_ef: vec4<f32>,
}

@group(0) @binding(2)
var<uniform> layer: LayerUniform;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) world: vec2<f32>,
}

// Same SDF as scene quad solids so HostTexture and the sibling Quad share a clip.
fn rounded_box_sdf(p: vec2<f32>, size: vec2<f32>, corners: vec4<f32>) -> f32 {
    var box_half = select(corners.yz, corners.xw, p.x > 0.0);
    var corner = select(box_half.y, box_half.x, p.y > 0.0);
    var q = abs(p) - size + corner;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - corner;
}

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var units = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let uv = units[index];
    let local = vec2<f32>(
        layer.origin.z + uv.x * layer.params.z,
        layer.origin.w + uv.y * layer.params.w,
    );
    let world = vec2<f32>(
        layer.affine.x * local.x + layer.affine.z * local.y + layer.origin.x,
        layer.affine.y * local.x + layer.affine.w * local.y + layer.origin.y,
    );
    let physical = world * layer.source.y;
    let dest = vec2<f32>(max(layer.source.z, 1.0), max(layer.source.w, 1.0));
    var output: VertexOutput;
    output.position = vec4<f32>(
        2.0 * physical.x / dest.x - 1.0,
        1.0 - 2.0 * physical.y / dest.y,
        0.0,
        1.0,
    );
    output.uv = uv;
    output.local = local;
    output.world = world;
    return output;
}

fn overflow_clip_local(world: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        layer.clip_inv_abcd.x * world.x + layer.clip_inv_abcd.z * world.y + layer.clip_inv_ef.x,
        layer.clip_inv_abcd.y * world.x + layer.clip_inv_abcd.w * world.y + layer.clip_inv_ef.y,
    );
}

fn inside_overflow_clip(world: vec2<f32>) -> bool {
    let local = overflow_clip_local(world);
    if (any(local < layer.clip_rect.xy)) || (any(local > layer.clip_rect.xy + layer.clip_rect.zw)) {
        return false;
    }
    let radius = layer.clip_inv_ef.z;
    if (radius <= 0.0) {
        return true;
    }
    let rel = local - layer.clip_rect.xy;
    let half = layer.clip_rect.zw * 0.5;
    let center = rel - half;
    let corner = min(radius, min(half.x, half.y));
    let q = abs(center) - half + corner;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - corner <= 0.0;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if !inside_overflow_clip(input.world) {
        discard;
    }
    let sampled = textureSample(source, source_sampler, input.uv);
    let source_alpha = select(sampled.a, 1.0, layer.source.x > 0.5);
    let color = vec4<f32>(sampled.rgb, source_alpha) * layer.params.x;
    let has_clip = layer.clip.z > 0.0 && layer.clip.w > 0.0;
    let box_pos = select(layer.origin.zw, layer.clip.xy, has_clip);
    let box_size = select(layer.params.zw, layer.clip.zw, has_clip);
    let radius = min(layer.params.y, min(box_size.x, box_size.y) * 0.5);
    if radius <= 0.0 {
        return color;
    }
    let scale = max(layer.source.y, 0.0001);
    let pos = box_pos * scale;
    let size = box_size * scale;
    let local_pos = input.local * scale;
    let scaled_radius = radius * scale;
    let dist = rounded_box_sdf(
        -(local_pos - pos - size * 0.5) * 2.0,
        size,
        vec4<f32>(scaled_radius * 2.0)
    ) / 2.0;
    return color * clamp(0.5 - dist, 0.0, 1.0);
}
"#;

static NEXT_HOST_TEXTURE_INSTANCE_ID: AtomicU64 = AtomicU64::new(1);

/// A stable handle to a host-owned, filterable 2D WGPU texture.
///
/// The handle remains valid while the host updates the content in place or
/// replaces the underlying view. `generation` changes only when the view is
/// replaced, while `version` changes for every content invalidation.
#[derive(Debug, Clone)]
pub struct HostTexture {
    state: Arc<HostTextureState>,
}

#[derive(Debug)]
struct HostTextureState {
    id: u64,
    instance_identity: u64,
    version: AtomicU64,
    view: VersionedResource<wgpu::TextureView>,
}

#[derive(Debug)]
struct HostTextureSnapshot {
    id: u64,
    generation: u64,
    instance_identity: u64,
    view: wgpu::TextureView,
}

#[derive(Debug)]
struct VersionedResource<T> {
    state: RwLock<VersionedResourceState<T>>,
}

#[derive(Debug)]
struct VersionedResourceState<T> {
    generation: u64,
    resource: T,
}

impl<T> VersionedResource<T> {
    fn new(generation: u64, resource: T) -> Self {
        Self {
            state: RwLock::new(VersionedResourceState {
                generation,
                resource,
            }),
        }
    }

    fn generation(&self) -> u64 {
        self.state
            .read()
            .expect("versioned resource lock")
            .generation
    }

    fn replace(&self, resource: T) -> u64 {
        self.replace_with(|_| resource)
    }

    fn replace_with(&self, resource: impl FnOnce(u64) -> T) -> u64 {
        let mut state = self.state.write().expect("versioned resource lock");
        let generation = state.generation.saturating_add(1);
        state.resource = resource(generation);
        state.generation = generation;
        generation
    }
}

impl<T: Clone> VersionedResource<T> {
    fn snapshot(&self) -> (u64, T) {
        let state = self.state.read().expect("versioned resource lock");
        (state.generation, state.resource.clone())
    }
}

impl HostTexture {
    pub fn from_wgpu(id: u64, generation: u64, view: wgpu::TextureView) -> Self {
        Self {
            state: Arc::new(HostTextureState {
                id,
                instance_identity: next_host_texture_instance_id(),
                version: AtomicU64::new(generation),
                view: VersionedResource::new(generation, view),
            }),
        }
    }

    pub fn id(&self) -> u64 {
        self.state.id
    }

    pub fn generation(&self) -> u64 {
        self.state.view.generation()
    }

    /// Returns the monotonically increasing content version.
    pub fn version(&self) -> u64 {
        self.state.version.load(Ordering::Acquire)
    }

    /// Marks the current view as containing new host-rendered content.
    pub fn invalidate(&self) -> u64 {
        self.state.version.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Replaces the sampled view and invalidates the content in one operation.
    ///
    /// The stable handle is intentionally retained so the Scene painter
    /// observes the replacement without rebuilding layout.
    pub fn replace_view(&self, view: wgpu::TextureView) -> u64 {
        self.state.view.replace(view);
        self.invalidate()
    }

    fn snapshot(&self) -> HostTextureSnapshot {
        let (generation, view) = self.state.view.snapshot();
        HostTextureSnapshot {
            id: self.id(),
            generation,
            instance_identity: self.state.instance_identity,
            view,
        }
    }

    fn same_handle(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HostTextureAlphaMode {
    #[default]
    Premultiplied,
    Opaque,
}

impl HostTextureAlphaMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Premultiplied => "premultiplied",
            Self::Opaque => "opaque",
        }
    }
}

/// JS-visible metadata plus the host-owned WGPU view used by `<nana-gpu>`.
#[derive(Debug, Clone)]
pub struct HostTextureBinding {
    pub slot: String,
    pub texture: HostTexture,
    pub width: u32,
    pub height: u32,
    pub alpha_mode: HostTextureAlphaMode,
}

impl HostTextureBinding {
    pub fn aspect_ratio(&self) -> f32 {
        if self.width == 0 || self.height == 0 {
            1.0
        } else {
            self.width as f32 / self.height as f32
        }
    }
}

/// Shared slot registry. It stores texture views only; Device/Queue ownership
/// remains with the hosted renderer.
#[derive(Debug, Clone, Default)]
pub struct HostTextureRegistry {
    bindings: Arc<RwLock<HashMap<String, RegisteredHostTextureBinding>>>,
    revision: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
struct RegisteredHostTextureBinding {
    binding: HostTextureBinding,
    generation: u64,
    version: u64,
}

impl RegisteredHostTextureBinding {
    fn new(binding: HostTextureBinding) -> Self {
        let generation = binding.texture.generation();
        let version = binding.texture.version();
        Self {
            binding,
            generation,
            version,
        }
    }

    fn matches(&self, binding: &HostTextureBinding) -> bool {
        self.binding.texture.same_handle(&binding.texture)
            && self.generation == binding.texture.generation()
            && self.version == binding.texture.version()
            && self.binding.width == binding.width
            && self.binding.height == binding.height
            && self.binding.alpha_mode == binding.alpha_mode
    }
}

impl HostTextureRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        slot: impl Into<String>,
        texture: HostTexture,
        width: u32,
        height: u32,
        alpha_mode: HostTextureAlphaMode,
    ) -> HostTextureBinding {
        let slot = slot.into();
        let binding = HostTextureBinding {
            slot: slot.clone(),
            texture,
            width: width.max(1),
            height: height.max(1),
            alpha_mode,
        };
        let mut bindings = self.bindings.write().expect("host texture registry");
        if let Some(current) = bindings.get(&slot)
            && current.matches(&binding)
        {
            return current.binding.clone();
        }
        bindings.insert(slot, RegisteredHostTextureBinding::new(binding.clone()));
        self.revision.fetch_add(1, Ordering::AcqRel);
        binding
    }

    pub fn get(&self, slot: &str) -> Option<HostTextureBinding> {
        self.bindings
            .read()
            .ok()
            .and_then(|bindings| bindings.get(slot).map(|entry| entry.binding.clone()))
    }

    pub fn remove(&self, slot: &str) -> Option<HostTextureBinding> {
        let removed = self
            .bindings
            .write()
            .ok()?
            .remove(slot)
            .map(|entry| entry.binding);
        if removed.is_some() {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        removed
    }

    /// Marks a slot's content as changed while retaining its texture view.
    /// Consumers can compare [`Self::revision`] to schedule the next frame.
    pub fn invalidate(&self, slot: &str) -> Option<u64> {
        let mut bindings = self.bindings.write().ok()?;
        let binding = bindings.get_mut(slot)?;
        let version = binding.binding.texture.invalidate();
        binding.generation = binding.binding.texture.generation();
        binding.version = version;
        self.revision.fetch_add(1, Ordering::AcqRel);
        Some(version)
    }

    /// Device-loss boundary: every prior JS texture handle becomes unresolved.
    pub fn invalidate_all(&self) -> usize {
        let Ok(mut bindings) = self.bindings.write() else {
            return 0;
        };
        let count = bindings.len();
        bindings.clear();
        if count > 0 {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        count
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub fn len(&self) -> usize {
        self.bindings
            .read()
            .map(|bindings| bindings.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Presentation properties for one host texture layer.
///
/// Layer order follows Scene primitive order, so composition remains owned
/// by the UI tree rather than by a second renderer.
#[derive(Debug, Clone)]
pub struct HostTextureLayer {
    texture: HostTexture,
    opacity: f32,
    corner_radius: f32,
    clip: Option<LogicalRect>,
    fragment_clip_rect: [f32; 4],
    fragment_clip_inv_abcd: [f32; 4],
    fragment_clip_inv_ef: [f32; 2],
    fragment_clip_corner_radius: f32,
    alpha_mode: Option<HostTextureAlphaMode>,
}

impl HostTextureLayer {
    const PASS_CLIP_RECT: [f32; 4] = [-1.0e7, -1.0e7, 2.0e7, 2.0e7];
    const PASS_CLIP_INV_ABCD: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const PASS_CLIP_INV_EF: [f32; 2] = [0.0, 0.0];

    pub const fn new(texture: HostTexture) -> Self {
        Self {
            texture,
            opacity: 1.0,
            corner_radius: 0.0,
            clip: None,
            fragment_clip_rect: Self::PASS_CLIP_RECT,
            fragment_clip_inv_abcd: Self::PASS_CLIP_INV_ABCD,
            fragment_clip_inv_ef: Self::PASS_CLIP_INV_EF,
            fragment_clip_corner_radius: 0.0,
            alpha_mode: None,
        }
    }

    pub fn from_binding(binding: HostTextureBinding) -> Self {
        Self {
            texture: binding.texture,
            opacity: 1.0,
            corner_radius: 0.0,
            clip: None,
            fragment_clip_rect: Self::PASS_CLIP_RECT,
            fragment_clip_inv_abcd: Self::PASS_CLIP_INV_ABCD,
            fragment_clip_inv_ef: Self::PASS_CLIP_INV_EF,
            fragment_clip_corner_radius: 0.0,
            alpha_mode: Some(binding.alpha_mode),
        }
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = finite_opacity(opacity);
        self
    }

    pub fn with_clip(mut self, clip: LogicalRect) -> Self {
        // Pre-affine logical clip, same space as Scene bounds / the sibling
        // Quad. Combined with `corner_radius` this is the rounded clip; the
        // AABB is still scissored at draw.
        self.clip = Some(clip);
        self
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = if radius.is_finite() {
            radius.max(0.0)
        } else {
            0.0
        };
        self
    }

    /// Overflow parallelogram in paint space. Independent of rounded sibling
    /// Quad SDF (`clip` + `corner_radius`).
    pub const fn with_fragment_clip(
        mut self,
        rect: [f32; 4],
        inv_abcd: [f32; 4],
        inv_ef: [f32; 2],
        corner_radius: f32,
    ) -> Self {
        self.fragment_clip_rect = rect;
        self.fragment_clip_inv_abcd = inv_abcd;
        self.fragment_clip_inv_ef = inv_ef;
        self.fragment_clip_corner_radius = if corner_radius.is_finite() {
            corner_radius.max(0.0)
        } else {
            0.0
        };
        self
    }

    pub const fn with_alpha_mode(mut self, alpha_mode: HostTextureAlphaMode) -> Self {
        self.alpha_mode = Some(alpha_mode);
        self
    }

    pub const fn texture(&self) -> &HostTexture {
        &self.texture
    }

    pub const fn opacity(&self) -> f32 {
        self.opacity
    }

    pub const fn clip(&self) -> Option<LogicalRect> {
        self.clip
    }

    pub const fn corner_radius(&self) -> f32 {
        self.corner_radius
    }

    pub fn alpha_mode(&self) -> HostTextureAlphaMode {
        self.alpha_mode
            .unwrap_or(HostTextureAlphaMode::Premultiplied)
    }
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct GpuTexturePrimitive {
    layer: HostTextureLayer,
    presentation: PresentationIdentity,
}

impl GpuTexturePrimitive {
    pub(crate) fn from_scene(node: u64, slot: u8, layer: HostTextureLayer) -> Self {
        Self {
            layer,
            presentation: PresentationIdentity::Scene { node, slot },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &self,
        pipeline: &mut GpuTexturePipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: LogicalRect,
        affine: [f32; 6],
        scale_factor: f32,
        dest_size: [u32; 2],
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let texture = self.layer.texture.snapshot();
        let key = TextureKey::new(self.presentation, texture.id);
        let (slot, viewport_rect) = slot_for_bounds(texture.id, bounds, scale_factor);
        let clip = self
            .layer
            .clip
            .map(|clip| RenderSlot::new(texture.id, clip, scale_factor).physical);
        let needs_rebind = texture_needs_rebind(
            pipeline
                .textures
                .get(&key)
                .map(PreparedTexture::fingerprint),
            &texture,
        );

        if needs_rebind {
            let layer_uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-ui host texture layer uniform"),
                size: std::mem::size_of::<LayerUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let layer_uniform_value =
                make_layer_uniform(&self.layer, bounds, affine, scale_factor, dest_size);
            let uniform_bytes = bytemuck::bytes_of(&layer_uniform_value);
            queue.write_buffer(&layer_uniform, 0, uniform_bytes);
            if let Some(work) = gpu_work {
                work.record_upload(uniform_bytes.len());
                work.record_realloc();
            }
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nana-ui host texture bind group"),
                layout: &pipeline.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: layer_uniform.as_entire_binding(),
                    },
                ],
            });
            pipeline.textures.insert(
                key,
                PreparedTexture {
                    instance_identity: texture.instance_identity,
                    generation: texture.generation,
                    bind_group,
                    slot,
                    viewport: viewport_rect,
                    clip,
                    layer_uniform,
                    used: true,
                },
            );
        } else if let Some(prepared) = pipeline.textures.get_mut(&key) {
            let layer_uniform_value =
                make_layer_uniform(&self.layer, bounds, affine, scale_factor, dest_size);
            let uniform_bytes = bytemuck::bytes_of(&layer_uniform_value);
            queue.write_buffer(&prepared.layer_uniform, 0, uniform_bytes);
            if let Some(work) = gpu_work {
                work.record_upload(uniform_bytes.len());
            }
            prepared.slot = slot;
            prepared.viewport = viewport_rect;
            prepared.clip = clip;
            prepared.used = true;
        }
    }

    pub(crate) fn draw_in_pass(
        &self,
        pipeline: &GpuTexturePipeline,
        pass: &mut wgpu::RenderPass<'_>,
        clip_bounds: PhysicalRect,
        dest_size: [u32; 2],
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let key = TextureKey::new(self.presentation, self.layer.texture.id());
        let Some(texture) = pipeline.textures.get(&key) else {
            return;
        };
        let layer_bounds = texture
            .clip
            .map_or(clip_bounds, |clip| intersect_physical(clip_bounds, clip));
        if layer_bounds.width == 0 || layer_bounds.height == 0 {
            return;
        }
        encode_host_texture(pipeline, texture, pass, layer_bounds, dest_size, gpu_work);
    }
}

fn encode_host_texture(
    pipeline: &GpuTexturePipeline,
    texture: &PreparedTexture,
    pass: &mut wgpu::RenderPass<'_>,
    bounds: PhysicalRect,
    dest_size: [u32; 2],
    gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
) {
    let viewport = texture.viewport;
    let _ = viewport;
    pass.set_viewport(
        0.0,
        0.0,
        dest_size[0].max(1) as f32,
        dest_size[1].max(1) as f32,
        0.0,
        1.0,
    );
    pass.set_scissor_rect(bounds.x, bounds.y, bounds.width, bounds.height);
    pass.set_pipeline(&pipeline.pipeline);
    pass.set_bind_group(0, &texture.bind_group, &[]);
    pass.draw(0..6, 0..1);
    pass.set_viewport(
        0.0,
        0.0,
        dest_size[0].max(1) as f32,
        dest_size[1].max(1) as f32,
        0.0,
        1.0,
    );
    if let Some(work) = gpu_work {
        work.record_draw_batch();
        work.record_draw_call();
    }
}

#[doc(hidden)]
pub struct GpuTexturePipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: HashMap<TextureKey, PreparedTexture>,
}

impl GpuTexturePipeline {
    pub(crate) fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-ui host texture shader"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui host texture bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-ui host texture pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-ui host texture pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nana-ui host texture sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
            textures: HashMap::new(),
        }
    }

    pub(crate) fn trim(&mut self) {
        trim_unused(&mut self.textures, |texture| &mut texture.used);
    }
}

struct PreparedTexture {
    instance_identity: u64,
    generation: u64,
    bind_group: wgpu::BindGroup,
    slot: RenderSlot,
    viewport: [f32; 4],
    clip: Option<crate::geometry::PhysicalRect>,
    layer_uniform: wgpu::Buffer,
    used: bool,
}

impl PreparedTexture {
    const fn fingerprint(&self) -> TextureFingerprint {
        TextureFingerprint {
            instance_identity: self.instance_identity,
            generation: self.generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextureFingerprint {
    instance_identity: u64,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PresentationIdentity {
    Scene { node: u64, slot: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TextureKey {
    presentation: PresentationIdentity,
    texture_id: u64,
}

impl TextureKey {
    const fn new(presentation: PresentationIdentity, texture_id: u64) -> Self {
        Self {
            presentation,
            texture_id,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LayerUniform {
    params: [f32; 4],
    source: [f32; 4],
    affine: [f32; 4],
    origin: [f32; 4],
    clip: [f32; 4],
    clip_rect: [f32; 4],
    clip_inv_abcd: [f32; 4],
    clip_inv_ef: [f32; 4],
}

fn make_layer_uniform(
    layer: &HostTextureLayer,
    bounds: LogicalRect,
    affine: [f32; 6],
    scale_factor: f32,
    dest_size: [u32; 2],
) -> LayerUniform {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let clip = layer.clip.unwrap_or(bounds);
    LayerUniform {
        params: [
            layer.opacity,
            layer.corner_radius,
            bounds.width.max(0.0),
            bounds.height.max(0.0),
        ],
        source: [
            if layer.alpha_mode() == HostTextureAlphaMode::Opaque {
                1.0
            } else {
                0.0
            },
            scale,
            dest_size[0].max(1) as f32,
            dest_size[1].max(1) as f32,
        ],
        affine: [affine[0], affine[1], affine[2], affine[3]],
        origin: [affine[4], affine[5], bounds.x, bounds.y],
        clip: [clip.x, clip.y, clip.width.max(0.0), clip.height.max(0.0)],
        clip_rect: layer.fragment_clip_rect,
        clip_inv_abcd: layer.fragment_clip_inv_abcd,
        clip_inv_ef: [
            layer.fragment_clip_inv_ef[0],
            layer.fragment_clip_inv_ef[1],
            layer.fragment_clip_corner_radius,
            0.0,
        ],
    }
}

fn next_host_texture_instance_id() -> u64 {
    // Zero is never issued, and exhaustion fails closed instead of reusing an
    // identity that may still exist in a renderer cache.
    NEXT_HOST_TEXTURE_INSTANCE_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("host texture instance identity space exhausted")
}

fn texture_needs_rebind(
    prepared: Option<TextureFingerprint>,
    texture: &HostTextureSnapshot,
) -> bool {
    prepared.is_none_or(|prepared| {
        prepared.instance_identity != texture.instance_identity
            || prepared.generation != texture.generation
    })
}

fn trim_unused<K: Eq + std::hash::Hash, V>(
    entries: &mut HashMap<K, V>,
    mut used: impl FnMut(&mut V) -> &mut bool,
) {
    entries.retain(|_, entry| {
        let used = used(entry);
        let retain = *used;
        *used = false;
        retain
    });
}

fn finite_opacity(opacity: f32) -> f32 {
    if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc, thread};

    use pollster::block_on;
    use wgpu;

    use super::{
        GpuTexturePipeline, HostTexture, HostTextureAlphaMode, HostTextureLayer,
        HostTextureRegistry, PresentationIdentity, TextureFingerprint, TextureKey,
        VersionedResource, make_layer_uniform, texture_needs_rebind, trim_unused,
    };
    use crate::geometry::LogicalRect;

    #[test]
    fn scene_identity_is_stable_across_frames_and_distinct_across_primitives() {
        let first_frame = TextureKey::new(PresentationIdentity::Scene { node: 9, slot: 2 }, 7);
        let next_frame = TextureKey::new(PresentationIdentity::Scene { node: 9, slot: 2 }, 7);
        let sibling = TextureKey::new(PresentationIdentity::Scene { node: 10, slot: 2 }, 7);

        assert_eq!(first_frame, next_frame);
        assert_ne!(first_frame, sibling);
    }

    #[test]
    fn replacing_a_dropped_handle_with_the_same_public_identity_always_rebinds() {
        let (device, _) = test_device();
        let old = HostTexture::from_wgpu(7, 3, test_texture_view(&device));
        let old_snapshot = old.snapshot();
        let old_fingerprint = TextureFingerprint {
            instance_identity: old_snapshot.instance_identity,
            generation: old_snapshot.generation,
        };
        drop(old);

        let replacement = HostTexture::from_wgpu(7, 3, test_texture_view(&device));
        let replacement_snapshot = replacement.snapshot();
        assert_ne!(
            old_snapshot.instance_identity,
            replacement_snapshot.instance_identity
        );
        assert_eq!(old_snapshot.id, replacement_snapshot.id);
        assert_eq!(old_snapshot.generation, replacement_snapshot.generation);

        let presentation = PresentationIdentity::Scene { node: 9, slot: 2 };
        assert_eq!(
            TextureKey::new(presentation, old_snapshot.id),
            TextureKey::new(presentation, replacement_snapshot.id)
        );
        assert!(texture_needs_rebind(
            Some(old_fingerprint),
            &replacement_snapshot
        ));
    }

    #[test]
    fn versioned_resource_snapshots_generation_and_resource_atomically() {
        let resource = Arc::new(VersionedResource::new(0, 0));
        let writer = Arc::clone(&resource);
        let writer = thread::spawn(move || {
            for _ in 0..1_000 {
                writer.replace_with(|generation| generation);
            }
        });
        for _ in 0..1_000 {
            let (generation, value) = resource.snapshot();
            assert_eq!(generation, value);
        }
        writer.join().unwrap();
        assert_eq!(resource.snapshot(), (1_000, 1_000));
    }

    #[test]
    fn replacing_a_host_view_advances_generation_and_version_together() {
        let (device, _) = test_device();
        let texture = HostTexture::from_wgpu(7, 3, test_texture_view(&device));

        assert_eq!(texture.replace_view(test_texture_view(&device)), 4);
        let snapshot = texture.snapshot();
        assert_eq!(snapshot.generation, 4);
        assert_eq!(texture.version(), 4);
    }

    #[test]
    fn trim_keeps_only_entries_used_in_the_previous_frame_and_resets_them() {
        let mut entries = HashMap::from([(1, true), (2, false)]);
        trim_unused(&mut entries, |used| used);
        assert_eq!(entries, HashMap::from([(1, false)]));
        trim_unused(&mut entries, |used| used);
        assert!(entries.is_empty());
    }

    #[test]
    fn alpha_mode_is_per_binding_and_exact_reregistration_is_a_noop() {
        let texture = test_host_texture(7, 3);
        let registry = HostTextureRegistry::new();
        let premultiplied = registry.register(
            "premultiplied",
            texture.clone(),
            320,
            180,
            HostTextureAlphaMode::Premultiplied,
        );
        let opaque = registry.register(
            "opaque",
            texture.clone(),
            320,
            180,
            HostTextureAlphaMode::Opaque,
        );
        let revision = registry.revision();
        registry.register(
            "premultiplied",
            texture.clone(),
            320,
            180,
            HostTextureAlphaMode::Premultiplied,
        );
        assert_eq!(registry.revision(), revision);

        let bounds = LogicalRect::new(0.0, 0.0, 320.0, 180.0);
        let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let premultiplied_uniform = make_layer_uniform(
            &HostTextureLayer::from_binding(premultiplied.clone()),
            bounds,
            identity,
            1.0,
            [320, 180],
        );
        let opaque_uniform = make_layer_uniform(
            &HostTextureLayer::from_binding(opaque),
            LogicalRect::new(0.0, 0.0, 320.0, 180.0),
            identity,
            1.0,
            [320, 180],
        );
        assert_eq!(premultiplied_uniform.source[0], 0.0);
        assert_eq!(opaque_uniform.source[0], 1.0);

        let rounded = make_layer_uniform(
            &HostTextureLayer::from_binding(premultiplied.clone()).with_corner_radius(8.0),
            bounds,
            identity,
            1.0,
            [320, 180],
        );
        assert_eq!(rounded.params[1], 8.0);
        assert_eq!(rounded.clip, [0.0, 0.0, 320.0, 180.0]);

        let contain_dest = LogicalRect::new(0.0, 40.0, 320.0, 100.0);
        let rounded_clip = make_layer_uniform(
            &HostTextureLayer::from_binding(premultiplied)
                .with_corner_radius(32.0)
                .with_clip(bounds),
            contain_dest,
            identity,
            1.0,
            [320, 180],
        );
        assert_eq!(rounded_clip.params[1], 32.0);
        assert_eq!(rounded_clip.params[2], 320.0);
        assert_eq!(rounded_clip.params[3], 100.0);
        assert_eq!(rounded_clip.origin[2], 0.0);
        assert_eq!(rounded_clip.origin[3], 40.0);
        assert_eq!(rounded_clip.clip, [0.0, 0.0, 320.0, 180.0]);
    }

    #[test]
    fn opaque_shader_pipeline_builds_on_the_host_device() {
        let (device, queue) = test_device();
        let _pipeline = GpuTexturePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    }

    fn test_host_texture(id: u64, generation: u64) -> HostTexture {
        let (device, _) = test_device();
        HostTexture::from_wgpu(id, generation, test_texture_view(&device))
    }

    fn test_texture_view(device: &wgpu::Device) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("NanaUI GPU texture lifecycle test texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn test_device() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .expect("GPU texture lifecycle test requires a WGPU adapter");
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("NanaUI GPU texture lifecycle test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("GPU texture lifecycle test requires a WGPU device");
        (device, queue)
    }
}
