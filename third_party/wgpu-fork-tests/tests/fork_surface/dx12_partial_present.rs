//! DX12 `DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL` + `Present1` dirty-rect tracking
//! and buffer preservation (`SurfaceTexture::buffer_index` / `preserved` and
//! `Queue::present_with_damage`).
use crate::support;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nana_wgpu_fork_tests::no_adapter;
use support::{clear, create_window, pump, resize, surface};
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

#[test]
fn dx12_flip_sequential_preserves_backbuffers_and_presents_with_damage() {
    let _gpu_guard = nana_wgpu_fork_tests::gpu_lock();
    let window = create_window();
    pump();

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = surface(&instance, window);
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    })) else {
        unsafe { DestroyWindow(window).unwrap() };
        no_adapter(
            "dx12_flip_sequential_preserves_backbuffers_and_presents_with_damage",
            wgpu::Backends::DX12,
        );
        return;
    };
    let Ok((device, queue)) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    else {
        unsafe { DestroyWindow(window).unwrap() };
        no_adapter(
            "dx12_flip_sequential_preserves_backbuffers_and_presents_with_damage",
            wgpu::Backends::DX12,
        );
        return;
    };

    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    device.on_uncaptured_error({
        let errors = errors.clone();
        Arc::new(move |error: wgpu::Error| errors.lock().unwrap().push(error.to_string()))
    });

    let capabilities = surface.get_capabilities(&adapter);
    let mut configuration = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: capabilities.formats[0],
        color_space: Default::default(),
        width: 160,
        height: 120,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: capabilities.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &configuration);

    let mut seen_buffers = HashSet::new();
    let mut saw_preserved = false;

    for step in 0..8u32 {
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                let index = texture
                    .buffer_index()
                    .expect("DX12 surface exposes backbuffer index");
                let expected_preserved = !seen_buffers.insert(index);
                assert_eq!(
                    texture.preserved(),
                    expected_preserved,
                    "step {step} backbuffer {index}: preserved mismatch"
                );
                let view = texture.texture.create_view(&Default::default());
                let mut encoder = device.create_command_encoder(&Default::default());
                if expected_preserved {
                    saw_preserved = true;
                    // Render with LoadOp::Load into a scissored dirty rect on a preserved buffer.
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("partial_present_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        occlusion_query_set: None,
                        timestamp_writes: None,
                        multiview_mask: None,
                    });
                    pass.set_scissor_rect(8, 8, 32, 24);
                    drop(pass);
                    queue.submit([encoder.finish()]);
                    queue.present_with_damage(texture, &[[8, 8, 32, 24]]);
                } else {
                    clear(&mut encoder, &view, 0.25);
                    queue.submit([encoder.finish()]);
                    queue.present(texture);
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Outdated => {}
            other => panic!("DX12 acquire failed: {other:?}"),
        }
    }

    assert!(
        saw_preserved,
        "expected FLIP_SEQUENTIAL swapchain to cycle back to a previously presented backbuffer"
    );

    // Resize must reset preserved tracking on every backbuffer.
    configuration.width = 192;
    configuration.height = 144;
    resize(window, configuration.width, configuration.height);
    surface.configure(&device, &configuration);
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
            assert!(
                !texture.preserved(),
                "resized swapchain backbuffer must start unpreserved"
            );
            let view = texture.texture.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            clear(&mut encoder, &view, 0.5);
            queue.submit([encoder.finish()]);
            queue.present(texture);
        }
        wgpu::CurrentSurfaceTexture::Timeout
        | wgpu::CurrentSurfaceTexture::Occluded
        | wgpu::CurrentSurfaceTexture::Outdated => {}
        other => panic!("DX12 resize acquire failed: {other:?}"),
    }

    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(5)),
    });
    drop(surface);
    unsafe { DestroyWindow(window).unwrap() };

    let errors = errors.lock().unwrap();
    assert!(
        errors.is_empty(),
        "DX12 partial present raised uncaptured errors: {errors:#?}"
    );
}
