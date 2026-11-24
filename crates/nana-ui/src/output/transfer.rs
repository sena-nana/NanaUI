//! Final, same-size transfer or alpha conversion of an already rendered
//! presentation. This never traverses UiScene and does not create a second
//! painter or device.

use nana_gpu::__framework as raw;
use nana_gpu::{
    FrameContext, GpuContext, GpuError, GpuRenderTarget, GpuTexture, GpuTextureFormat,
    LogicalBinding, LogicalBindingType, ResourceTable, ShaderStage,
};

const TRANSFER_SHADER_KEY: u64 = 0x7472_616e_7366_6573;
const STRAIGHT_SHADER_KEY: u64 = 0x7374_7261_6967_6874;
const MAX_CACHED_BINDINGS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TransferOp {
    Blit {
        source_srgb: bool,
    },
    #[cfg_attr(not(feature = "hosted"), allow(dead_code))]
    StraightenAlpha,
}

impl TransferOp {
    fn shader_identity(self, format: GpuTextureFormat) -> (u64, u64) {
        match self {
            Self::Blit { source_srgb } => {
                let variant = match (source_srgb, format.is_srgb()) {
                    (false, true) => 1,
                    (true, false) => 2,
                    _ => 0,
                };
                (TRANSFER_SHADER_KEY, variant)
            }
            Self::StraightenAlpha => (STRAIGHT_SHADER_KEY, 0),
        }
    }
}

pub(crate) struct OutputTransfer {
    generation: nana_gpu::DeviceGeneration,
    format: GpuTextureFormat,
    op: TransferOp,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    bindings: Vec<(GpuTexture, GpuTextureFormat, wgpu::BindGroup)>,
}

impl OutputTransfer {
    #[cfg(test)]
    pub(crate) fn new(gpu: &GpuContext, format: GpuTextureFormat, op: TransferOp) -> Self {
        Self::try_new(gpu, format, op).expect("output transfer test pipeline")
    }

    pub(crate) fn try_new(
        gpu: &GpuContext,
        format: GpuTextureFormat,
        op: TransferOp,
    ) -> Result<Self, crate::scene_paint::ScenePaintError> {
        let device = raw::device(gpu);
        let policy = gpu.policy();
        let table = ResourceTable::new(vec![LogicalBinding::new(
            0,
            LogicalBindingType::SampledTexture,
            &[ShaderStage::Fragment],
        )])
        .map_err(
            |error| crate::scene_paint::ScenePaintError::ShaderUnavailable {
                label: "nana.output.transfer",
                detail: error.to_string(),
            },
        )?
        .for_generation(gpu.generation());
        let layout_key = table.layout_key();
        let logical_layout = gpu.create_resource_layout(&table).map_err(|error| {
            crate::scene_paint::ScenePaintError::ShaderUnavailable {
                label: "nana.output.transfer",
                detail: error.to_string(),
            }
        })?;
        let layout = raw::resource_layout(&logical_layout).clone();
        let label = match op {
            TransferOp::Blit { .. } => "nana.output.transfer",
            TransferOp::StraightenAlpha => "nana-ui.window-output.straight",
        };
        let (shader_key, variant) = op.shader_identity(format);
        let pipeline_key = nana_gpu::PipelineKey {
            generation: policy.generation(),
            target_format: format,
            sample_count: 1,
            shader: shader_key,
            layout: layout_key,
            material: 0,
            primitive: variant,
            blend: 0,
            depth: 0,
            vertex_layout: 0,
        };
        let build_pipeline = |vs_module: &wgpu::ShaderModule,
                              vs_ep: &str,
                              fs_module: &wgpu::ShaderModule,
                              fs_ep: &str,
                              _: &[(&str, f64)]| {
            let layout_pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
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
        };
        let pipeline = match op {
            TransferOp::Blit { source_srgb } => {
                // A hardware sRGB view decodes on load and encodes on store. Untyped
                // export textures contain the very same encoded channel values.
                let fragment_entry = match (source_srgb, format.is_srgb()) {
                    (false, true) => "fs_decode",
                    (true, false) => "fs_encode",
                    _ => "fs_passthrough",
                };
                let shader =
                    crate::scene_paint::wgsl::program!("nana.output.transfer", TRANSFER_WGSL);
                shader.try_cached_pipeline(
                    Some(policy),
                    |_| pipeline_key,
                    device,
                    "vs",
                    fragment_entry,
                    &[],
                    build_pipeline,
                )?
            }
            TransferOp::StraightenAlpha => {
                let shader = crate::scene_paint::wgsl::program!(
                    "nana-ui.window-output.straight",
                    STRAIGHT_ALPHA_WGSL
                );
                shader.try_cached_pipeline(
                    Some(policy),
                    |_| pipeline_key,
                    device,
                    "vs",
                    "fs",
                    &[],
                    build_pipeline,
                )?
            }
        };
        Ok(Self {
            generation: gpu.generation(),
            format,
            op,
            layout,
            pipeline,
            bindings: Vec::new(),
        })
    }

    pub(crate) fn matches(
        &self,
        gpu: &GpuContext,
        format: GpuTextureFormat,
        op: TransferOp,
    ) -> bool {
        self.generation == gpu.generation() && self.format == format && self.op == op
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
        let source_format = source.format();
        let index = match self
            .bindings
            .iter()
            .position(|(texture, fmt, _)| texture.ptr_eq(source) && *fmt == source_format)
        {
            Some(index) => {
                let last = self.bindings.len() - 1;
                if index != last {
                    self.bindings.swap(index, last);
                }
                last
            }
            None => {
                let label = match self.op {
                    TransferOp::Blit { .. } => "nana.output.transfer",
                    TransferOp::StraightenAlpha => "nana-ui.window-output.straight",
                };
                let bind = raw::device(frame.gpu()).create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &self.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(raw::texture_view(source)),
                    }],
                });
                if self.bindings.len() == MAX_CACHED_BINDINGS {
                    self.bindings.remove(0);
                }
                self.bindings.push((source.clone(), source_format, bind));
                self.bindings.len() - 1
            }
        };
        let label = match self.op {
            TransferOp::Blit { .. } => "nana.output.transfer",
            TransferOp::StraightenAlpha => "nana-ui.window-output.straight",
        };
        let mut pass = raw::encoder(frame).begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
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
        pass.set_bind_group(0, &self.bindings[index].2, &[]);
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

pub(crate) const STRAIGHT_ALPHA_WGSL: &str = r"
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let color = textureLoad(source, vec2<i32>(position.xy), 0);
    if (color.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(min(color.rgb / color.a, vec3<f32>(1.0)), color.a);
}
";
