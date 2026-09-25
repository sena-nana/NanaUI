//! Host-owned wgpu Surface on top of `ANativeWindow` (Vulkan).
//!
//! Includes a minimal solid-color fill pipeline for shell chrome bands
//! (scissor + fullscreen triangle). Not Nana DesktopShell.

use android_activity::AndroidApp;
use raw_window_handle::{AndroidNdkWindowHandle, DisplayHandle, RawWindowHandle, WindowHandle};
use wgpu::{
    BindGroup, BindGroupLayout, Buffer, BufferUsages, ColorTargetState, CompositeAlphaMode,
    CurrentSurfaceTexture, Device, FragmentState, Instance, MultisampleState, PipelineLayout,
    PresentMode, PrimitiveState, Queue, RenderPipeline, ShaderModule, Surface,
    SurfaceConfiguration, SurfaceTargetUnsafe, TextureFormat, TextureUsages, VertexState,
};

use crate::chrome_fill::{
    FILL_COLOR_SIZE, band_draw_list, fill_color_offset, fill_color_stride, pack_fill_colors,
};
use crate::shell::ShellChromeBand;
use nana_ui::{FrameContext, GpuContext, GpuRenderTarget};

const FILL_SHADER: &str = r#"
struct Uniforms {
    color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> @builtin(position) vec4<f32> {
    // Fullscreen triangle covering clip space.
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    return vec4<f32>(pos[idx], 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return uniforms.color;
}
"#;

pub struct GpuSurface {
    pub surface: Surface<'static>,
    pub gpu: GpuContext,
    pub config: SurfaceConfiguration,
    pub format: TextureFormat,
    _instance: Instance,
    fill: SolidFillPipeline,
}

/// One color slot per band, selected by dynamic offset. Queue writes land
/// before the frame executes, so all colors go up in one write, not one
/// rewritten slot per band.
struct SolidFillPipeline {
    pipeline: RenderPipeline,
    bind_layout: BindGroupLayout,
    bind_group: BindGroup,
    uniform: Buffer,
    stride: u32,
    packed: Vec<u8>,
    uploaded: Vec<u8>,
}

impl SolidFillPipeline {
    fn new(device: &Device, format: TextureFormat) -> Self {
        let shader: ShaderModule = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-android chrome fill"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(FILL_SHADER)),
        });
        let bind_layout: BindGroupLayout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("nana-android fill bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(FILL_COLOR_SIZE.into()),
                    },
                    count: None,
                }],
            });
        let stride = fill_color_stride(device.limits().min_uniform_buffer_offset_alignment);
        let (uniform, bind_group) =
            Self::create_colors(device, &bind_layout, 4 * u64::from(stride));
        let pipeline_layout: PipelineLayout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("nana-android fill pl"),
                bind_group_layouts: &[Some(&bind_layout)],
                immediate_size: 0,
            });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-android fill pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_layout,
            bind_group,
            uniform,
            stride,
            packed: Vec::new(),
            uploaded: Vec::new(),
        }
    }

    fn create_colors(device: &Device, layout: &BindGroupLayout, size: u64) -> (Buffer, BindGroup) {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-android fill colors"),
            size,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-android fill bg"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform,
                    offset: 0,
                    size: wgpu::BufferSize::new(FILL_COLOR_SIZE.into()),
                }),
            }],
        });
        (uniform, bind_group)
    }

    /// Upload all band colors, skipping the write when they are unchanged.
    fn write_colors(
        &mut self,
        device: &Device,
        queue: &Queue,
        colors: impl IntoIterator<Item = [f64; 4]>,
    ) {
        pack_fill_colors(&mut self.packed, colors, self.stride);
        let needed = self.packed.len() as u64;
        if needed > self.uniform.size() {
            (self.uniform, self.bind_group) =
                Self::create_colors(device, &self.bind_layout, needed.next_power_of_two());
            self.uploaded.clear();
        }
        if self.packed != self.uploaded {
            queue.write_buffer(&self.uniform, 0, &self.packed);
            std::mem::swap(&mut self.packed, &mut self.uploaded);
        }
    }
}

impl GpuSurface {
    /// Create a Vulkan-capable surface from the current native window.
    pub fn new(app: &AndroidApp, width: u32, height: u32) -> Result<Self, String> {
        let width = width.max(1);
        let height = height.max(1);

        let native = app
            .native_window()
            .ok_or_else(|| "ANativeWindow missing".to_string())?;

        let instance = Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        // SAFETY: Native window is alive for the duration of this Surface while
        // the activity holds the window; runtime drops Surface on Destroyed.
        let surface = unsafe {
            let display = DisplayHandle::android();
            let window = AndroidNdkWindowHandle::new(native.ptr().cast());
            // SAFETY: the ANativeWindow is retained by `native` for this
            // surface creation call and remains alive for the Surface.
            let window = WindowHandle::borrow_raw(RawWindowHandle::AndroidNdk(window));
            instance.create_surface_unsafe(SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display.as_raw()),
                raw_window_handle: window.as_raw(),
            })
        }
        .map_err(|e| format!("create_surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("request_adapter: {e}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nana-android-host"),
            required_features: wgpu::Features::empty(),
            // SceneWgpuPainter exceeds downlevel limits (fragment storage buffers,
            // 16 inter-stage variables); ask for what the adapter has, never more.
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .map_err(|e| format!("request_device: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    TextureFormat::Rgba8Unorm
                        | TextureFormat::Bgra8Unorm
                        | TextureFormat::Rgba8UnormSrgb
                )
            })
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| "no surface formats".to_string())?;

        let alpha = if caps.alpha_modes.contains(&CompositeAlphaMode::Opaque) {
            CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(CompositeAlphaMode::Auto)
        };

        let present = if caps.present_modes.contains(&PresentMode::Fifo) {
            PresentMode::Fifo
        } else {
            caps.present_modes
                .first()
                .copied()
                .unwrap_or(PresentMode::Fifo)
        };

        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Srgb,
            width,
            height,
            present_mode: present,
            desired_maximum_frame_latency: 2,
            alpha_mode: alpha,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let fill = SolidFillPipeline::new(&device, format);

        Ok(Self {
            surface,
            gpu: GpuContext::from_wgpu(adapter, device, queue),
            config,
            format,
            _instance: instance,
            fill,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.config.width == width && self.config.height == height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface
            .configure(self.gpu.wgpu().device(), &self.config);
    }

    /// Chrome fill, then the NanaUI Scene slot, in one encoder submit.
    pub fn present_chrome_bands_with_overlay(
        &mut self,
        bands: &[ShellChromeBand],
        mut overlay: impl FnMut(&GpuRenderTarget, &mut FrameContext) -> Result<(), String>,
    ) -> Result<(), String> {
        let draws = band_draw_list(bands, self.config.width, self.config.height);

        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) | CurrentSurfaceTexture::Suboptimal(frame) => {
                frame
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            CurrentSurfaceTexture::Outdated => {
                self.surface
                    .configure(self.gpu.wgpu().device(), &self.config);
                return Err("surface outdated (reconfigured)".into());
            }
            CurrentSurfaceTexture::Lost => return Err("surface lost".into()),
            CurrentSurfaceTexture::Validation => return Err("surface validation".into()),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let target = GpuRenderTarget::from_wgpu(
            &self.gpu,
            view.clone(),
            self.format,
            [frame.texture.width(), frame.texture.height()],
        );
        let mut recording = self.gpu.begin_frame("nana-android chrome+slot");
        let encoder = recording.wgpu_encoder();

        if draws.is_empty() {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nana-android chrome clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.10,
                            g: 0.12,
                            b: 0.16,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        } else {
            self.fill.write_colors(
                self.gpu.wgpu().device(),
                self.gpu.wgpu().queue(),
                draws.iter().map(|d| d.4),
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nana-android chrome bands"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.08,
                            g: 0.09,
                            b: 0.11,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.fill.pipeline);
            for (i, &(x, y, w, h, _)) in draws.iter().enumerate() {
                let offset = fill_color_offset(i, self.fill.stride);
                pass.set_bind_group(0, &self.fill.bind_group, &[offset]);
                pass.set_scissor_rect(x, y, w, h);
                pass.draw(0..3, 0..1);
            }
        }

        // The frame is dropped before the surface texture on failure.
        overlay(&target, &mut recording)?;
        recording.submit();
        self.gpu.wgpu().queue().present(frame);
        Ok(())
    }
}
