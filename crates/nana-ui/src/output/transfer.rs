//! Final, same-size transfer of an already rendered presentation. This never
//! traverses UiScene and does not create a second painter or device.

use nana_gpu::__framework as raw;
use nana_gpu::{FrameContext, GpuContext, GpuError, GpuRenderTarget, GpuTexture};

pub(crate) struct OutputTransfer {
    generation: nana_gpu::DeviceGeneration,
    format: nana_gpu::GpuTextureFormat,
    source_srgb: bool,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    bindings: Vec<(GpuTexture, wgpu::BindGroup)>,
}

impl OutputTransfer {
    pub(crate) fn new(
        gpu: &GpuContext,
        format: nana_gpu::GpuTextureFormat,
        source_srgb: bool,
    ) -> Self {
        let device = raw::device(gpu);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana.output.transfer"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let layout_pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana.output.transfer"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // A hardware sRGB view decodes on load and encodes on store. Untyped
        // export textures contain the very same encoded channel values.
        let fragment_entry = match (source_srgb, format.is_srgb()) {
            (false, true) => "fs_decode",
            (true, false) => "fs_encode",
            _ => "fs_passthrough",
        };
        let shader = crate::scene_paint::wgsl::program!("nana.output.transfer", TRANSFER_WGSL);
        let pipeline = shader.pipeline(
            device,
            "vs",
            fragment_entry,
            &[],
            |vs_module, vs_ep, fs_module, fs_ep, _| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("nana.output.transfer"),
                    layout: Some(&layout_pipeline),
                    vertex: wgpu::VertexState {
                        module: vs_module,
                        entry_point: Some(vs_ep),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: fs_module,
                        entry_point: Some(fs_ep),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: raw::format_to_wgpu(format),
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            },
        );
        Self {
            generation: gpu.generation(),
            format,
            source_srgb,
            layout,
            pipeline,
            bindings: Vec::new(),
        }
    }

    pub(crate) fn matches(
        &self,
        gpu: &GpuContext,
        format: nana_gpu::GpuTextureFormat,
        source_srgb: bool,
    ) -> bool {
        self.generation == gpu.generation()
            && self.format == format
            && self.source_srgb == source_srgb
    }

    pub(crate) fn record(
        &mut self,
        frame: &mut FrameContext,
        source: &GpuTexture,
        target: &GpuRenderTarget,
    ) -> Result<(), GpuError> {
        for found in [source.generation(), target.generation(), frame.generation()] {
            if found != self.generation {
                return Err(GpuError::DeviceMismatch {
                    expected: self.generation,
                    found,
                });
            }
        }
        if target.format() != self.format {
            return Err(GpuError::UnsupportedFormat(target.format()));
        }
        if raw::target_view(target).texture() == raw::texture(source) {
            return Err(GpuError::UnsupportedCapability(
                "presentation source and destination must be distinct",
            ));
        }
        if source.size() != (target.size()[0], target.size()[1]) {
            return Err(GpuError::RegionOutOfBounds);
        }
        let index = match self
            .bindings
            .iter()
            .position(|(texture, _)| texture.ptr_eq(source))
        {
            Some(index) => index,
            None => {
                let bind = raw::device(frame.gpu()).create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("nana.output.transfer"),
                    layout: &self.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(raw::texture_view(source)),
                    }],
                });
                if self.bindings.len() == 8 {
                    self.bindings.remove(0);
                }
                self.bindings.push((source.clone(), bind));
                self.bindings.len() - 1
            }
        };
        let mut pass = raw::encoder(frame).begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nana.output.transfer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: raw::target_view(target),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bindings[index].1, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

pub(crate) const TRANSFER_WGSL: &str = r"
@group(0) @binding(0) var image: texture_2d<f32>;
fn decode(c: vec3<f32>) -> vec3<f32> { return select(pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045)); }
fn encode(c: vec3<f32>) -> vec3<f32> { return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), c * 12.92, c <= vec3<f32>(0.0031308)); }
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let c = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(c * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs_passthrough(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(image, vec2<i32>(p.xy), 0);
}
@fragment fn fs_decode(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(image, vec2<i32>(p.xy), 0);
    return vec4<f32>(decode(c.rgb), c.a);
}
@fragment fn fs_encode(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(image, vec2<i32>(p.xy), 0);
    return vec4<f32>(encode(c.rgb), c.a);
}
";
