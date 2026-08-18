//! Host-owned wgpu Surface on top of `ANativeWindow` (Vulkan).
//!
//! Includes a minimal solid-color fill pipeline for shell chrome bands
//! (scissor + fullscreen triangle). Not Nana DesktopShell.

use std::sync::Arc;

use android_activity::AndroidApp;
use raw_window_handle::{DisplayHandle, HasWindowHandle};
use wgpu::{
    BindGroup, BindGroupLayout, Buffer, BufferUsages, ColorTargetState, CompositeAlphaMode,
    CurrentSurfaceTexture, Device, FragmentState, Instance, MultisampleState, PipelineLayout,
    PresentMode, PrimitiveState, Queue, RenderPipeline, ShaderModule, Surface,
    SurfaceConfiguration, SurfaceTargetUnsafe, TextureFormat, TextureUsages, VertexState,
};

use crate::chrome_fill::band_draw_list;
use crate::shell::ShellChromeBand;

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
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub config: SurfaceConfiguration,
    pub format: TextureFormat,
    _instance: Instance,
    fill: SolidFillPipeline,
}

struct SolidFillPipeline {
    pipeline: RenderPipeline,
    bind_group: BindGroup,
    uniform: Buffer,
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
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-android fill color"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-android fill bg"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
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
            bind_group,
            uniform,
        }
    }

    fn write_color(&self, queue: &Queue, color: [f64; 4]) {
        let rgba = [
            color[0] as f32,
            color[1] as f32,
            color[2] as f32,
            color[3] as f32,
        ];
        let mut bytes = [0u8; 16];
        for (i, v) in rgba.iter().enumerate() {
            bytes[i * 4..(i + 1) * 4].copy_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
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
            let window = native
                .window_handle()
                .map_err(|e| format!("window handle: {e}"))?;
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
            required_limits:
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
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
            device: Arc::new(device),
            queue: Arc::new(queue),
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
        self.surface.configure(&self.device, &self.config);
    }

    /// Chrome fill, then the NanaUI Scene slot, in one encoder submit.
    pub fn present_chrome_bands_with_overlay(
        &self,
        bands: &[ShellChromeBand],
        mut overlay: impl FnMut(&wgpu::TextureView, &mut wgpu::CommandEncoder) -> Result<(), String>,
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
                self.surface.configure(&self.device, &self.config);
                return Err("surface outdated (reconfigured)".into());
            }
            CurrentSurfaceTexture::Lost => return Err("surface lost".into()),
            CurrentSurfaceTexture::Validation => return Err("surface validation".into()),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("nana-android chrome+slot"),
            });

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
            // One pass per band so uniform color writes land before each draw.
            for (i, &(x, y, w, h, color)) in draws.iter().enumerate() {
                self.fill.write_color(&self.queue, color);
                let load = if i == 0 {
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.08,
                        g: 0.09,
                        b: 0.11,
                        a: 1.0,
                    })
                } else {
                    wgpu::LoadOp::Load
                };
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("nana-android chrome band"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load,
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
                    pass.set_bind_group(0, &self.fill.bind_group, &[]);
                    pass.set_scissor_rect(x, y, w, h);
                    pass.draw(0..3, 0..1);
                }
            }
        }

        overlay(&view, &mut encoder)?;
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }
}
