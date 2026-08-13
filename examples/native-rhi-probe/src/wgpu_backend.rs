use std::time::Instant;

use crate::{ProbeBackend, Sample, elapsed_ms};

pub struct WgpuProbe {
    adapter_name: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: wgpu::TextureView,
}

impl WgpuProbe {
    pub fn new(width: u32, height: u32) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("WGPU Metal probe must find an adapter");
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Nana native RHI probe WGPU device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("WGPU Metal probe must create a device");
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Nana native RHI probe WGPU target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            adapter_name,
            device,
            queue,
            target: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }
}

impl ProbeBackend for WgpuProbe {
    fn name(&self) -> &'static str {
        "wgpu-metal"
    }

    fn adapter_name(&self) -> String {
        self.adapter_name.clone()
    }

    fn sample(&mut self, pass_count: usize) -> Sample {
        let encode_started = Instant::now();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Nana native RHI probe WGPU encoder"),
            });
        for pass_index in 0..pass_count {
            let clear = 0.05 + pass_index as f64 * 0.001;
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Nana native RHI probe WGPU pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear,
                            g: 0.10,
                            b: 0.15,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let command_buffer = encoder.finish();
        let encode_ms = elapsed_ms(encode_started);

        let submit_started = Instant::now();
        let submission = self.queue.submit([command_buffer]);
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .expect("WGPU probe submission must complete");
        Sample {
            encode_ms,
            submit_wait_ms: elapsed_ms(submit_started),
        }
    }
}
