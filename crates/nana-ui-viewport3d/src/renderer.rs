//! Offscreen 4x MSAA + Depth32Float renderer publishing into a NanaUI [`TextureSlot`].

use std::fmt;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4};
use nana_ui::wgpu::{self, util::DeviceExt};
use nana_ui::{
    DeviceGeneration, GpuContext, GpuTexture, HostTexture, HostTextureAlphaMode,
    HostTextureRegistry, TextureSlot,
};

use crate::camera::OrbitCamera;
use crate::mesh::{MeshData, Vertex3d, ground_grid};
use crate::scene::{Material3d, Scene3d};

const SAMPLES: u32 = 4;
const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Errors returned by [`Viewport3dRenderer::render_to_slot`].
#[derive(Debug, Clone)]
pub enum Viewport3dError {
    ZeroExtent,
    DeviceMismatch,
}

impl fmt::Display for Viewport3dError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroExtent => f.write_str("viewport3d extent must be non-zero"),
            Self::DeviceMismatch => f.write_str("rendered texture device generation mismatch"),
        }
    }
}

impl std::error::Error for Viewport3dError {}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalsUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_color: [f32; 4],
    ground_color: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceUniform {
    model: [[f32; 4]; 4],
    normal_0: [f32; 4],
    normal_1: [f32; 4],
    normal_2: [f32; 4],
    color: [f32; 4],
    emissive_roughness: [f32; 4],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    material_group: wgpu::BindGroup,
    base_color: [f32; 4],
}

struct Targets {
    size: [u32; 2],
    msaa_color: wgpu::TextureView,
    msaa_depth: wgpu::TextureView,
    resolve_view: wgpu::TextureView,
    host_texture: HostTexture,
}

struct GpuState {
    generation: DeviceGeneration,
    bg_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    material_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    meshes: Vec<GpuMesh>,
    uploaded_mesh_rev: u64,
    grid_mesh: Option<(f32, f32, GpuMesh)>,
    instance_pool: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    targets: Option<Targets>,
}

/// Reusable 3D renderer targeting a NanaUI [`TextureSlot`] (`HostTexture`).
pub struct Viewport3dRenderer {
    state: Option<GpuState>,
}

impl Default for Viewport3dRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Viewport3dRenderer {
    pub fn new() -> Self {
        Self { state: None }
    }

    /// Drops cached GPU pipelines and textures (they rebuild automatically on next render).
    pub fn invalidate_gpu(&mut self) {
        self.state = None;
    }

    /// Renders `scene` from `camera` into a 4x MSAA offscreen `Rgba16Float` target and
    /// publishes it to `slot`. Uses `registry.painted_extent(slot.name())` when available,
    /// otherwise `fallback_size`.
    pub fn render_to_slot(
        &mut self,
        gpu: &GpuContext,
        registry: &HostTextureRegistry,
        slot: &TextureSlot,
        fallback_size: [u32; 2],
        camera: &OrbitCamera,
        scene: &Scene3d,
    ) -> Result<[u32; 2], Viewport3dError> {
        let max_dim = gpu.wgpu().device().limits().max_texture_dimension_2d.max(1);
        let raw = registry
            .painted_extent(slot.name())
            .unwrap_or(fallback_size);
        let size = [raw[0].clamp(1, max_dim), raw[1].clamp(1, max_dim)];
        if size[0] == 0 || size[1] == 0 {
            return Err(Viewport3dError::ZeroExtent);
        }

        let generation = gpu.generation();
        let mut replaced = false;
        if self
            .state
            .as_ref()
            .is_none_or(|s| s.generation != generation)
        {
            self.state = Some(build_gpu_state(gpu));
            replaced = true;
        }
        let state = self.state.as_mut().expect("gpu state");

        if state.uploaded_mesh_rev != scene.mesh_revision() {
            state.meshes = scene
                .meshes()
                .iter()
                .map(|m| upload_mesh(gpu, &state.material_layout, &state.sampler, m))
                .collect();
            state.uploaded_mesh_rev = scene.mesh_revision();
        }

        let grid_active = scene.show_grid;
        if grid_active {
            let needs_grid = state
                .grid_mesh
                .as_ref()
                .is_none_or(|(ext, step, _)| *ext != scene.grid_extent || *step != scene.grid_step);
            if needs_grid {
                let gm = ground_grid(scene.grid_extent, scene.grid_step);
                let gpu_gm = upload_mesh(gpu, &state.material_layout, &state.sampler, &gm);
                state.grid_mesh = Some((scene.grid_extent, scene.grid_step, gpu_gm));
            }
        }

        let targets_rebuilt = state.targets.as_ref().is_none_or(|t| t.size != size);
        if targets_rebuilt {
            state.targets = Some(build_targets(gpu, size)?);
            replaced = true;
        }

        let aspect = size[0] as f32 / size[1] as f32;
        let lights = &scene.lights;
        let globals = GlobalsUniform {
            view_proj: camera.view_proj(aspect).to_cols_array_2d(),
            eye: [
                camera.eye_position().x,
                camera.eye_position().y,
                camera.eye_position().z,
                1.0,
            ],
            sun_dir: [lights.sun_dir[0], lights.sun_dir[1], lights.sun_dir[2], 0.0],
            sun_color: [
                lights.sun_color[0],
                lights.sun_color[1],
                lights.sun_color[2],
                1.0,
            ],
            sky_color: [
                lights.sky_color[0],
                lights.sky_color[1],
                lights.sky_color[2],
                1.0,
            ],
            ground_color: [
                lights.ground_color[0],
                lights.ground_color[1],
                lights.ground_color[2],
                1.0,
            ],
            bg_top: [
                lights.background_top[0],
                lights.background_top[1],
                lights.background_top[2],
                1.0,
            ],
            bg_bottom: [
                lights.background_bottom[0],
                lights.background_bottom[1],
                lights.background_bottom[2],
                1.0,
            ],
        };
        let queue = gpu.wgpu().queue();
        queue.write_buffer(&state.globals_buf, 0, bytemuck::bytes_of(&globals));

        let draw_count = scene.instances().len() + usize::from(grid_active);
        ensure_instance_slots(gpu, state, draw_count);

        let mut draw_items: Vec<(usize, InstanceUniform, bool)> = Vec::with_capacity(draw_count);
        if grid_active {
            draw_items.push((
                usize::MAX,
                make_instance_uniform(
                    Mat4::IDENTITY,
                    [1.0; 4],
                    Material3d {
                        color: [0.42, 0.46, 0.54, 1.0],
                        roughness: 0.9,
                        emissive: [0.0; 3],
                    },
                ),
                true,
            ));
        }
        for inst in scene.instances() {
            let Some(mesh) = state.meshes.get(inst.mesh_index) else {
                continue;
            };
            draw_items.push((
                inst.mesh_index,
                make_instance_uniform(inst.transform, mesh.base_color, inst.material),
                false,
            ));
        }

        for (slot_idx, (_, uniform, _)) in draw_items.iter().enumerate() {
            queue.write_buffer(
                &state.instance_pool[slot_idx].0,
                0,
                bytemuck::bytes_of(uniform),
            );
        }

        let targets = state.targets.as_ref().expect("targets");
        let mut frame = gpu.begin_producer_frame("nana-viewport3d");
        {
            let encoder = frame.wgpu_encoder();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport3d scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa_color,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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

            pass.set_pipeline(&state.bg_pipeline);
            pass.set_bind_group(0, &state.globals_group, &[]);
            pass.draw(0..3, 0..1);

            pass.set_pipeline(&state.mesh_pipeline);
            pass.set_bind_group(0, &state.globals_group, &[]);
            for (slot_idx, (mesh_idx, _, is_grid)) in draw_items.iter().enumerate() {
                let gpu_mesh = if *is_grid {
                    &state.grid_mesh.as_ref().expect("grid mesh").2
                } else {
                    &state.meshes[*mesh_idx]
                };
                pass.set_bind_group(1, &gpu_mesh.material_group, &[]);
                pass.set_bind_group(2, &state.instance_pool[slot_idx].1, &[]);
                pass.set_vertex_buffer(0, gpu_mesh.vertices.slice(..));
                pass.set_index_buffer(gpu_mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..gpu_mesh.index_count, 0, 0..1);
            }
        }
        let _submission = frame.submit();

        if replaced {
            slot.replace(
                targets.host_texture.clone(),
                size[0],
                size[1],
                HostTextureAlphaMode::Opaque,
            );
        } else {
            slot.invalidate();
        }
        Ok(size)
    }
}

fn make_instance_uniform(model: Mat4, base_color: [f32; 4], mat: Material3d) -> InstanceUniform {
    let normal_mat = Mat3::from_mat4(model).inverse().transpose();
    let cols = normal_mat.to_cols_array_2d();
    InstanceUniform {
        model: model.to_cols_array_2d(),
        normal_0: [cols[0][0], cols[0][1], cols[0][2], 0.0],
        normal_1: [cols[1][0], cols[1][1], cols[1][2], 0.0],
        normal_2: [cols[2][0], cols[2][1], cols[2][2], 0.0],
        color: [
            base_color[0] * mat.color[0],
            base_color[1] * mat.color[1],
            base_color[2] * mat.color[2],
            base_color[3] * mat.color[3],
        ],
        emissive_roughness: [
            mat.emissive[0],
            mat.emissive[1],
            mat.emissive[2],
            mat.roughness,
        ],
    }
}

fn build_gpu_state(gpu: &GpuContext) -> GpuState {
    let device = gpu.wgpu().device();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("viewport3d shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
    });

    let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport3d globals layout"),
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
    let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport3d material layout"),
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
        ],
    });
    let instance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport3d instance layout"),
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

    let bg_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("viewport3d bg layout"),
        bind_group_layouts: &[Some(&globals_layout)],
        immediate_size: 0,
    });
    let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("viewport3d bg pipeline"),
        layout: Some(&bg_pipe_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("bg_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("bg_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: SCENE_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
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

    let mesh_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("viewport3d mesh layout"),
        bind_group_layouts: &[
            Some(&globals_layout),
            Some(&material_layout),
            Some(&instance_layout),
        ],
        immediate_size: 0,
    });
    let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("viewport3d mesh pipeline"),
        layout: Some(&mesh_pipe_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("mesh_vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex3d>() as u64,
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
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 24,
                        shader_location: 2,
                    },
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("mesh_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: SCENE_FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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

    let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport3d globals"),
        size: std::mem::size_of::<GlobalsUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport3d globals group"),
        layout: &globals_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: globals_buf.as_entire_binding(),
        }],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("viewport3d sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        ..wgpu::SamplerDescriptor::default()
    });

    GpuState {
        generation: gpu.generation(),
        bg_pipeline,
        mesh_pipeline,
        globals_buf,
        globals_group,
        material_layout,
        instance_layout,
        sampler,
        meshes: Vec::new(),
        uploaded_mesh_rev: 0,
        grid_mesh: None,
        instance_pool: Vec::new(),
        targets: None,
    }
}

fn ensure_instance_slots(gpu: &GpuContext, state: &mut GpuState, count: usize) {
    let device = gpu.wgpu().device();
    while state.instance_pool.len() < count {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport3d instance"),
            size: std::mem::size_of::<InstanceUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport3d instance group"),
            layout: &state.instance_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buf.as_entire_binding(),
            }],
        });
        state.instance_pool.push((buf, group));
    }
}

fn upload_mesh(
    gpu: &GpuContext,
    material_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    mesh: &MeshData,
) -> GpuMesh {
    let device = gpu.wgpu().device();
    let queue = gpu.wgpu().queue();
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("viewport3d vb"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("viewport3d ib"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    let (tex_w, tex_h, rgba): (u32, u32, &[u8]) = match &mesh.albedo_rgba {
        Some((w, h, bytes)) => (*w, *h, bytes.as_slice()),
        None => (1, 1, &[255, 255, 255, 255]),
    };
    let extent = wgpu::Extent3d {
        width: tex_w,
        height: tex_h,
        depth_or_array_layers: 1,
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport3d albedo"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(tex_w * 4),
            rows_per_image: Some(tex_h),
        },
        extent,
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let material_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport3d material group"),
        layout: material_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    GpuMesh {
        vertices,
        indices,
        index_count: mesh.indices.len() as u32,
        material_group,
        base_color: mesh.base_color,
    }
}

fn build_targets(gpu: &GpuContext, size: [u32; 2]) -> Result<Targets, Viewport3dError> {
    let device = gpu.wgpu().device();
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    let msaa_color = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("viewport3d msaa color"),
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
            label: Some("viewport3d msaa depth"),
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
        label: Some("viewport3d resolve"),
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
    if output.generation() != gpu.generation() {
        return Err(Viewport3dError::DeviceMismatch);
    }
    let host_texture = HostTexture::new(&output);
    Ok(Targets {
        size,
        msaa_color,
        msaa_depth,
        resolve_view,
        host_texture,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use glam::Vec3;
    use nana_ui::runtime::view::widget;
    use nana_ui::runtime::{
        DocumentId, GpuTextureView, LayoutViewport, LengthSpec, MeasureTextShaper, RuntimeDocument,
        Stack,
    };
    use nana_ui::{
        GpuTextureDescriptor, GpuTextureFormat, GpuTextureUsages, ScenePaintViewport,
        SceneWgpuPainter,
    };

    use super::*;
    use crate::mesh::cube;
    use crate::scene::Instance3d;

    fn test_gpu() -> GpuContext {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("GPU device");
        GpuContext::from_wgpu(adapter, device, queue)
    }

    fn readback_rgba(gpu: &GpuContext, target: &GpuTexture, width: u32, height: u32) -> Vec<u8> {
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let device = gpu.wgpu().device();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport3d readback"),
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut frame = gpu.begin_frame("viewport3d readback");
        frame.wgpu_encoder().copy_texture_to_buffer(
            target.wgpu().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = frame.submit();
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission.wgpu_index().clone()),
                timeout: Some(Duration::from_secs(15)),
            })
            .expect("poll readback");
        rx.recv_timeout(Duration::from_secs(15)).unwrap().unwrap();
        let mapped = slice.get_mapped_range().expect("mapped");
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for line in mapped.chunks_exact(row as usize) {
            for bgra in line[..(width * 4) as usize].chunks_exact(4) {
                pixels.extend_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
            }
        }
        drop(mapped);
        buffer.unmap();
        pixels
    }

    #[test]
    fn renders_3d_scene_into_host_texture_and_composites_with_nana_ui() {
        let gpu = test_gpu();
        let format = GpuTextureFormat::BGRA8_UNORM_SRGB;
        let mut painter = SceneWgpuPainter::try_new(&gpu, format).expect("painter");
        let registry = HostTextureRegistry::new();
        let slot = registry.slot("viewport3d");

        let mut scene3d = Scene3d::new();
        scene3d.show_grid = false;
        let box_mesh = scene3d.add_mesh(cube([1.4, 1.4, 1.4]));
        scene3d.add_instance(Instance3d {
            id: 1,
            mesh_index: box_mesh,
            transform: glam::Mat4::IDENTITY,
            material: Material3d {
                color: [0.95, 0.35, 0.25, 1.0],
                roughness: 0.4,
                emissive: [0.0; 3],
            },
        });
        let camera = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.5,
            pitch: 0.35,
            distance: 3.0,
            ..Default::default()
        };
        let mut renderer = Viewport3dRenderer::new();
        let first_size = renderer
            .render_to_slot(&gpu, &registry, &slot, [128, 96], &camera, &scene3d)
            .expect("first render");
        assert_eq!(first_size, [128, 96]);

        let doc_id = DocumentId::allocate();
        let mut doc = RuntimeDocument::new(doc_id);
        doc.context_mut()
            .mount_view_root(doc_id, || {
                widget(
                    Stack::layers()
                        .width(LengthSpec::Fill)
                        .height(LengthSpec::Fill),
                )
                .children(widget(
                    GpuTextureView::new("viewport3d").with_pointer_events(true),
                ))
            })
            .expect("mount");
        doc.flush(LayoutViewport::new(256.0, 192.0), &mut MeasureTextShaper)
            .expect("flush");
        let ui_scene = doc.shared_scene();

        let target = gpu
            .create_texture(&GpuTextureDescriptor {
                label: Some("viewport3d test target"),
                width: 256,
                height: 192,
                format,
                usage: GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC,
            })
            .expect("target");
        let mut frame = gpu.begin_frame("viewport3d test");
        painter
            .paint(
                &ui_scene,
                &mut frame,
                &target.render_target().expect("rt"),
                ScenePaintViewport {
                    logical_size: [256.0, 192.0],
                    physical_size: [256, 192],
                    scale_factor: 1.0,
                    scene_origin: [0.0, 0.0],
                    target_origin: [0.0, 0.0],
                    clear_color: [0.0, 0.0, 0.0, 1.0],
                    clear: true,
                },
                Some(&registry),
                None,
            )
            .expect("paint");
        frame.submit();

        assert_eq!(registry.painted_extent("viewport3d"), Some([256, 192]));
        let pixels = readback_rgba(&gpu, &target, 256, 192);
        let center = (96 * 256 + 128) * 4;
        assert!(
            pixels[center] > pixels[center + 2] + 30,
            "center pixel should show warm red/orange cube face, got {:?}",
            &pixels[center..center + 4]
        );

        // Second render picks up `painted_extent` [256, 192] automatically.
        let resized = renderer
            .render_to_slot(&gpu, &registry, &slot, [128, 96], &camera, &scene3d)
            .expect("resized render");
        assert_eq!(resized, [256, 192]);
    }
}
