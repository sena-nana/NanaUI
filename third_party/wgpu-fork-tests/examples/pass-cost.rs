//! CPU cost of a render pass: what one more pass in a command encoder costs
//! the recording thread, against the same draws in one pass.
//!
//! `chain` records `--passes` render passes, each drawing one triangle into
//! its own 256×256 layer while sampling the layer before it, so every pass
//! moves two textures between attachment and sampled states (a group layer,
//! a backdrop blur step). `single` records the same number of draws into one
//! pass. Per frame the thread times `create_command_encoder` through
//! `Queue::submit`, then waits for the GPU, so one submission is in flight.
//! The run stops at `--seconds` (at most 20) or `--frames`.
//!
//! ```sh
//! cargo run -j 4 --release -p nana-wgpu-fork-tests --example pass-cost -- \
//!   --backend dx12 --mode chain --passes 64 --seconds 5
//! ```
//!
//! Prints one JSON line: frame CPU µs (median, p10, p90) and the frame count.

use std::time::{Duration, Instant};

const EXTENT: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = textureLoad(source, vec2<i32>(position.xy) % vec2<i32>(8, 8), 0);
    return vec4<f32>(texel.rgb * 0.5 + vec3<f32>(0.25), 1.0);
}
"#;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Chain,
    Single,
}

struct Args {
    backend: wgpu::Backends,
    backend_name: String,
    mode: Mode,
    passes: u32,
    seconds: f64,
    frames: u64,
    discard_hal_labels: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        backend: wgpu::Backends::DX12,
        backend_name: "dx12".into(),
        mode: Mode::Chain,
        passes: 16,
        seconds: 5.0,
        frames: u64::MAX,
        discard_hal_labels: false,
    };
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        let mut value = || {
            argv.next()
                .unwrap_or_else(|| panic!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--backend" => {
                args.backend_name = value();
                args.backend = match args.backend_name.as_str() {
                    "dx12" => wgpu::Backends::DX12,
                    "vulkan" => wgpu::Backends::VULKAN,
                    other => panic!("unknown backend {other}"),
                };
            }
            "--mode" => {
                args.mode = match value().as_str() {
                    "chain" => Mode::Chain,
                    "single" => Mode::Single,
                    other => panic!("unknown mode {other}"),
                };
            }
            // Bounded: a light load by default, never more than 256 passes.
            "--passes" => args.passes = value().parse::<u32>().expect("passes").clamp(1, 256),
            "--seconds" => args.seconds = value().parse::<f64>().expect("seconds").clamp(0.5, 20.0),
            "--frames" => args.frames = value().parse().expect("frames"),
            "--discard-hal-labels" => args.discard_hal_labels = true,
            other => panic!("unknown flag {other}"),
        }
    }
    args
}

fn main() {
    let args = parse_args();
    let mut flags = wgpu::InstanceFlags::from_build_config();
    if args.discard_hal_labels {
        flags |= wgpu::InstanceFlags::DISCARD_HAL_LABELS;
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: args.backend,
        flags,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("pass-cost: no {} adapter", args.backend_name);
        std::process::exit(2);
    };
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default())).expect("device");

    // One layer per pass plus a spare, so `single` samples a texture it does
    // not draw into.
    let layers: Vec<wgpu::TextureView> = (0..=args.passes)
        .map(|_| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("pass-cost layer"),
                    size: wgpu::Extent3d {
                        width: EXTENT,
                        height: EXTENT,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        })
        .collect();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
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
    // Bind group `i` samples layer `i`.
    let sources: Vec<wgpu::BindGroup> = layers
        .iter()
        .map(|view| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                }],
            })
        })
        .collect();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(FORMAT.into())],
        }),
        multiview_mask: None,
        cache: None,
    });

    fn attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
        wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        }
    }

    let passes = args.passes as usize;
    let deadline = Instant::now() + Duration::from_secs_f64(args.seconds);
    let mut samples = Vec::new();
    let mut frame = 0u64;
    while Instant::now() < deadline && frame < args.frames {
        let started = Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        match args.mode {
            Mode::Chain => {
                for pass_index in 0..passes {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("pass-cost chain"),
                        color_attachments: &[Some(attachment(&layers[pass_index + 1]))],
                        ..Default::default()
                    });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &sources[pass_index], &[]);
                    pass.draw(0..3, 0..1);
                }
            }
            Mode::Single => {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pass-cost single"),
                    color_attachments: &[Some(attachment(&layers[0]))],
                    ..Default::default()
                });
                pass.set_pipeline(&pipeline);
                for pass_index in 0..passes {
                    pass.set_bind_group(0, &sources[pass_index + 1], &[]);
                    pass.draw(0..3, 0..1);
                }
            }
        }
        queue.submit([encoder.finish()]);
        let cpu = started.elapsed();
        // One submission in flight: wait for this frame before the next.
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(5)),
            })
            .expect("frame completes");
        // The first frames create command lists and allocators.
        if frame >= 30 {
            samples.push(cpu.as_secs_f64() * 1e6);
        }
        frame += 1;
    }
    samples.sort_by(f64::total_cmp);
    let at = |q: f64| {
        samples
            .get(((samples.len() as f64 - 1.0) * q).round() as usize)
            .copied()
            .unwrap_or(f64::NAN)
    };
    println!(
        "{{\"backend\":\"{}\",\"mode\":\"{}\",\"passes\":{},\"discard_hal_labels\":{},\
         \"frames\":{},\"cpu_us_p10\":{:.1},\"cpu_us_p50\":{:.1},\"cpu_us_p90\":{:.1}}}",
        args.backend_name,
        if args.mode == Mode::Chain {
            "chain"
        } else {
            "single"
        },
        args.passes,
        args.discard_hal_labels,
        samples.len(),
        at(0.1),
        at(0.5),
        at(0.9),
    );
}
