//! DX12 shared-texture export, read back on a D3D11 device of the same
//! adapter the way an in-process Spout sender would.
//!
//! The D3D11 staging read is test-only verification of the exported bytes;
//! the export path itself never reads pixels back to the CPU.

#![cfg(all(windows, feature = "native-export"))]

use std::os::windows::io::AsRawHandle;
use std::time::{Duration, Instant};

use nana_gpu::__framework;
use nana_gpu::{
    GpuCapability, GpuContext, GpuTexture, GpuTextureDescriptor, GpuTextureFormat,
    GpuTextureRegion, GpuTextureUsages, NativeExportDeferral, NativeExportError,
    NativeExportOutcome, NativeExportPool, NativeFrameToken,
};
use windows::Win32::Foundation::{GetHandleInformation, HANDLE, HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11Device1, ID3D11Device5, ID3D11DeviceContext,
    ID3D11DeviceContext4, ID3D11Fence, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter, IDXGIFactory4,
};
use windows::core::Interface;

const EXTENT: [u32; 2] = [64, 32];

fn dx12() -> Option<GpuContext> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .ok()?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some(__framework::adopt(adapter, device, queue))
}

macro_rules! dx12_or_skip {
    () => {
        match dx12() {
            Some(gpu) => gpu,
            None => {
                eprintln!("skipped: no DX12 adapter");
                return;
            }
        }
    };
}

/// BGRA bytes where every texel encodes its own position.
fn pattern() -> Vec<u8> {
    let [width, height] = EXTENT;
    let mut bytes = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            bytes.extend_from_slice(&[
                x as u8 * 3,
                y as u8 * 5,
                (x ^ y) as u8,
                128 + (x % 64) as u8,
            ]);
        }
    }
    bytes
}

fn source(gpu: &GpuContext, bytes: &[u8]) -> GpuTexture {
    let texture = gpu
        .create_texture(&GpuTextureDescriptor {
            label: Some("native export source"),
            width: EXTENT[0],
            height: EXTENT[1],
            format: GpuTextureFormat::BGRA8_UNORM,
            usage: GpuTextureUsages::COPY_SRC | GpuTextureUsages::COPY_DST,
        })
        .unwrap();
    gpu.write_texture(
        &texture,
        GpuTextureRegion::full(EXTENT[0], EXTENT[1]),
        bytes,
        EXTENT[0] * 4,
    )
    .unwrap();
    texture
}

fn export(pool: &mut NativeExportPool, gpu: &GpuContext, source: &GpuTexture) -> NativeFrameToken {
    let mut frame = gpu.begin_frame("native export test");
    let staged = match pool.stage(&mut frame, source).unwrap() {
        NativeExportOutcome::Staged(staged) => staged,
        NativeExportOutcome::Deferred(deferral) => panic!("deferred: {deferral:?}"),
    };
    frame.submit();
    pool.finish(staged).unwrap()
}

fn wait_completed(pool: &NativeExportPool, value: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while pool.completed_value().unwrap() < value {
        pool.gpu().poll();
        assert!(Instant::now() < deadline, "fence never reached {value}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The consumer: a D3D11 device on the exporting adapter.
struct Consumer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

impl Consumer {
    fn on_luid(luid: i64) -> Self {
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).unwrap();
            let adapter: IDXGIAdapter = factory
                .EnumAdapterByLuid(LUID {
                    LowPart: luid as u32,
                    HighPart: (luid >> 32) as i32,
                })
                .unwrap();
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .unwrap();
            Self {
                device: device.unwrap(),
                context: context.unwrap(),
            }
        }
    }

    /// Wait for ready, copy the slot into a staging texture, signal release,
    /// then (test-only) map the staging copy.
    fn read(&self, token: &mut NativeFrameToken) -> Vec<u8> {
        unsafe {
            let device1: ID3D11Device1 = self.device.cast().unwrap();
            let device5: ID3D11Device5 = self.device.cast().unwrap();
            let context4: ID3D11DeviceContext4 = self.context.cast().unwrap();
            let fence: ID3D11Fence = {
                let mut fence: Option<ID3D11Fence> = None;
                device5
                    .OpenSharedFence(HANDLE(token.fence_handle().as_raw_handle()), &mut fence)
                    .unwrap();
                fence.unwrap()
            };
            let shared: ID3D11Texture2D = device1
                .OpenSharedResource1(HANDLE(
                    token.texture_handle(token.slot()).unwrap().as_raw_handle(),
                ))
                .unwrap();
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            shared.GetDesc(&mut desc);
            assert_eq!([desc.Width, desc.Height], token.extent());
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
                ..desc
            };
            let mut staging = None;
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            token.accept_release();
            context4.Wait(&fence, token.ready_value()).unwrap();
            self.context.CopyResource(&staging, &shared);
            context4.Signal(&fence, token.release_value()).unwrap();
            self.context.Flush();
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let [width, height] = token.extent();
            let mut bytes = Vec::with_capacity((width * height * 4) as usize);
            for row in 0..height {
                let start = (mapped.pData as *const u8).add((row * mapped.RowPitch) as usize);
                bytes.extend_from_slice(std::slice::from_raw_parts(start, (width * 4) as usize));
            }
            self.context.Unmap(&staging, 0);
            bytes
        }
    }
}

#[test]
fn a_dx12_device_advertises_native_export() {
    let gpu = dx12_or_skip!();
    assert!(
        gpu.capabilities()
            .supports(GpuCapability::NativeTextureExport)
    );
}

#[test]
fn exported_frames_reach_a_d3d11_device_and_the_fence_completes() {
    let gpu = dx12_or_skip!();
    let bytes = pattern();
    let source = source(&gpu, &bytes);
    let mut pool = NativeExportPool::new(&gpu, EXTENT).unwrap();
    let consumer = Consumer::on_luid(pool.adapter_luid().unwrap());
    for frame in 0..4u64 {
        let mut token = export(&mut pool, &gpu, &source);
        assert_eq!(token.ready_value(), 2 * frame + 1);
        assert_eq!(token.release_value(), 2 * frame + 2);
        assert_eq!(
            token.slot(),
            (frame as usize) % nana_gpu::NATIVE_EXPORT_SLOTS
        );
        assert_eq!(consumer.read(&mut token), bytes, "frame {frame}");
        drop(token);
        wait_completed(&pool, 2 * frame + 2);
    }
    assert_eq!(pool.issued(), 4);
}

#[test]
fn an_unreleased_frame_defers_the_next_and_a_dropped_token_releases_itself() {
    let gpu = dx12_or_skip!();
    let source = source(&gpu, &pattern());
    let mut pool = NativeExportPool::new(&gpu, EXTENT).unwrap();
    let token = export(&mut pool, &gpu, &source);
    wait_completed(&pool, token.ready_value());
    let mut frame = gpu.begin_frame("busy");
    match pool.stage(&mut frame, &source).unwrap() {
        NativeExportOutcome::Deferred(NativeExportDeferral::ConsumerBusy { .. }) => {}
        other => panic!("expected a deferral, got {other:?}"),
    }
    drop(frame);
    // Nobody accepted it: dropping releases the slot on the producer queue.
    drop(token);
    wait_completed(&pool, 2);
    let next = export(&mut pool, &gpu, &source);
    assert_eq!(next.ready_value(), 3);
}

#[test]
fn a_discarded_frame_leaves_the_pool_where_it_was() {
    let gpu = dx12_or_skip!();
    let source = source(&gpu, &pattern());
    let mut pool = NativeExportPool::new(&gpu, EXTENT).unwrap();
    let mut frame = gpu.begin_frame("discarded");
    let NativeExportOutcome::Staged(staged) = pool.stage(&mut frame, &source).unwrap() else {
        panic!("first frame cannot defer");
    };
    drop(frame);
    assert_eq!(
        pool.finish(staged).unwrap_err(),
        NativeExportError::NotSubmitted
    );
    let token = export(&mut pool, &gpu, &source);
    assert_eq!(token.ready_value(), 1);
    assert_eq!(token.slot(), 0);
}

#[test]
fn retiring_closes_the_handles_once_the_last_token_is_gone() {
    let gpu = dx12_or_skip!();
    let source = source(&gpu, &pattern());
    let mut pool = NativeExportPool::new(&gpu, EXTENT).unwrap();
    let token = export(&mut pool, &gpu, &source);
    let handles: Vec<HANDLE> = (0..nana_gpu::NATIVE_EXPORT_SLOTS)
        .map(|slot| HANDLE(token.texture_handle(slot).unwrap().as_raw_handle()))
        .chain(std::iter::once(HANDLE(
            token.fence_handle().as_raw_handle(),
        )))
        .collect();
    let open = |handle: HANDLE| unsafe {
        let mut flags = 0;
        GetHandleInformation(handle, &mut flags).is_ok()
    };
    pool.retire();
    assert!(pool.is_retired());
    let mut frame = gpu.begin_frame("retired");
    assert_eq!(
        pool.stage(&mut frame, &source).unwrap_err(),
        NativeExportError::Retired
    );
    drop(frame);
    assert!(
        handles.iter().all(|handle| open(*handle)),
        "a live token keeps them open"
    );
    drop(token);
    assert!(
        handles.iter().all(|handle| !open(*handle)),
        "handles outlived the pool"
    );
}

#[test]
fn incompatible_sources_are_refused_before_recording() {
    let gpu = dx12_or_skip!();
    let mut pool = NativeExportPool::new(&gpu, EXTENT).unwrap();
    let wrong = gpu
        .create_texture(&GpuTextureDescriptor {
            label: Some("rgba source"),
            width: EXTENT[0],
            height: EXTENT[1],
            format: GpuTextureFormat::RGBA8_UNORM,
            usage: GpuTextureUsages::COPY_SRC,
        })
        .unwrap();
    let mut frame = gpu.begin_frame("incompatible");
    assert_eq!(
        pool.stage(&mut frame, &wrong).unwrap_err(),
        NativeExportError::IncompatibleSource
    );
    assert_eq!(
        NativeExportPool::new(&gpu, [0, 4]).unwrap_err(),
        NativeExportError::InvalidExtent {
            width: 0,
            height: 4
        }
    );
}
