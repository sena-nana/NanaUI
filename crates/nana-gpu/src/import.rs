//! Host-owned DX12 input textures for an external GPU producer.
//!
//! Each slot has its own fence. The producer waits for `release_value`, writes
//! BGRA8 pixels, and signals `ready_value`. The host GPU waits for ready, copies
//! into a private texture, restores COMMON, and signals the next release. No
//! production operation maps pixels or waits on the CPU.

use std::os::windows::io::{BorrowedHandle, RawHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_FENCE_FLAG_SHARED, D3D12_HEAP_FLAG_SHARED, D3D12_HEAP_PROPERTIES,
    D3D12_HEAP_TYPE_DEFAULT, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
    D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, D3D12_RESOURCE_STATE_COMMON,
    D3D12_TEXTURE_LAYOUT_UNKNOWN, ID3D12CommandQueue, ID3D12Fence,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::PCWSTR;

use crate::{DeviceGeneration, FrameContext, GpuBackend, GpuContext, GpuTexture};

/// Maximum number of external writes that may be outstanding.
pub const NATIVE_IMPORT_SLOTS: usize = 3;

/// A rejected input operation. A rejected ready frame is released automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeImportError {
    Unsupported(&'static str),
    InvalidExtent {
        width: u32,
        height: u32,
    },
    DeviceMismatch {
        expected: DeviceGeneration,
        found: DeviceGeneration,
    },
    Retired,
    WrongPool,
    Native {
        step: &'static str,
        code: i32,
    },
}

impl std::fmt::Display for NativeImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native input: {self:?}")
    }
}

impl std::error::Error for NativeImportError {}

fn native(step: &'static str) -> impl FnOnce(windows::core::Error) -> NativeImportError {
    move |error| NativeImportError::Native {
        step,
        code: error.code().0,
    }
}

struct Slot {
    texture: wgpu::Texture,
    texture_handle: HANDLE,
    fence_handle: HANDLE,
    fence: ID3D12Fence,
    queue: ID3D12CommandQueue,
    gpu: GpuContext,
    extent: [u32; 2],
    adapter_luid: i64,
    reserved: AtomicBool,
    next_ready: AtomicU64,
    last_release: AtomicU64,
    retired: Arc<AtomicBool>,
}

// The COM interfaces are free-threaded and NT handles are process-wide.
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

impl Drop for Slot {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.texture_handle);
            let _ = CloseHandle(self.fence_handle);
        }
    }
}

impl Slot {
    fn release(self: &Arc<Self>, ready: u64, after_stage: bool) {
        // Waiting on the GPU also makes dropping an unconsumed ready frame
        // safe: the release cannot overtake the external producer's write.
        let result = unsafe {
            if after_stage {
                // stage already queued the ready wait ahead of this submit.
                self.queue.Signal(&self.fence, ready + 1)
            } else {
                self.queue
                    .Wait(&self.fence, ready)
                    .and_then(|()| self.queue.Signal(&self.fence, ready + 1))
            }
        };
        if result.is_err() {
            self.retired.store(true, Ordering::Release);
        }
        if self.retired.load(Ordering::Acquire) {
            // The original WGPU submission's completion fence precedes the
            // native Signal above. A new queue tail is required to prove that
            // the native fence and handles can be destroyed. This allocation
            // and extra submission happen only after pool retirement.
            let retained = Arc::clone(self);
            self.gpu.inner.queue.submit(std::iter::empty());
            self.gpu
                .inner
                .queue
                .on_submitted_work_done(move || drop(retained));
        }
        self.reserved.store(false, Ordering::Release);
    }
}

/// Exclusive external write reservation. Dropping an unused reservation
/// cancels it. After enqueueing any external access, call `into_ready` before
/// dropping it; the handles and resources stay alive through that frame.
pub struct NativeImportTarget {
    slot: Option<Arc<Slot>>,
    ready: u64,
    release: u64,
}

impl NativeImportTarget {
    pub fn texture_handle(&self) -> BorrowedHandle<'_> {
        unsafe {
            BorrowedHandle::borrow_raw(self.slot.as_ref().unwrap().texture_handle.0 as RawHandle)
        }
    }

    pub fn fence_handle(&self) -> BorrowedHandle<'_> {
        unsafe {
            BorrowedHandle::borrow_raw(self.slot.as_ref().unwrap().fence_handle.0 as RawHandle)
        }
    }

    pub fn adapter_luid(&self) -> i64 {
        self.slot.as_ref().unwrap().adapter_luid
    }
    pub fn extent(&self) -> [u32; 2] {
        self.slot.as_ref().unwrap().extent
    }
    pub fn ready_value(&self) -> u64 {
        self.ready
    }
    /// Value the producer GPU waits for before writing. The host's next
    /// release is `ready_value() + 1`.
    pub fn release_value(&self) -> u64 {
        self.release
    }

    /// Publish a frame whose producer work has been enqueued.
    ///
    /// # Safety
    /// The producer must have enqueued a GPU wait for `release_value`, a
    /// complete BGRA8 write leaving the shared resource in COMMON, and a
    /// signal of `ready_value`, in that order, on the matching adapter. It
    /// must flush its context and never access the resource again without a
    /// new reservation. A signal that never executes can stall the GPU queue.
    pub unsafe fn into_ready(mut self) -> NativeImportFrame {
        let slot = self.slot.take().unwrap();
        slot.last_release.store(self.ready + 1, Ordering::Release);
        NativeImportFrame {
            slot: Some(slot),
            ready: self.ready,
        }
    }
}

impl Drop for NativeImportTarget {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            slot.reserved.store(false, Ordering::Release);
        }
    }
}

/// A producer-completed frame. Dropping it schedules GPU release without
/// copying; it never returns the slot before the producer has finished.
pub struct NativeImportFrame {
    slot: Option<Arc<Slot>>,
    ready: u64,
}

impl Drop for NativeImportFrame {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            slot.release(self.ready, false);
        }
    }
}

struct ReleaseAfterSubmit {
    slot: Option<Arc<Slot>>,
    ready: u64,
}

impl ReleaseAfterSubmit {
    fn submitted(mut self) {
        if let Some(slot) = self.slot.take() {
            slot.release(self.ready, true);
        }
    }
}

impl Drop for ReleaseAfterSubmit {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            slot.release(self.ready, false);
        }
    }
}

/// Three host-device shared BGRA8 slots. Retiring or dropping a pool stops
/// acquisition; outstanding reservations and GPU work retain their resources.
pub struct NativeImportPool {
    gpu: GpuContext,
    slots: Vec<Arc<Slot>>,
    retired: Arc<AtomicBool>,
    output: GpuTexture,
}

impl NativeImportPool {
    pub fn new(gpu: &GpuContext, extent: [u32; 2]) -> Result<Self, NativeImportError> {
        let [width, height] = extent;
        if width == 0
            || height == 0
            || width > gpu.capabilities().max_texture_dimension_2d()
            || height > gpu.capabilities().max_texture_dimension_2d()
        {
            return Err(NativeImportError::InvalidExtent { width, height });
        }
        if gpu.capabilities().backend() != GpuBackend::Dx12 || gpu.is_lost() {
            return Err(NativeImportError::Unsupported(
                "a live DX12 device is required",
            ));
        }
        let retired = Arc::new(AtomicBool::new(false));
        let mut slots = Vec::with_capacity(NATIVE_IMPORT_SLOTS);
        for _ in 0..NATIVE_IMPORT_SLOTS {
            slots.push(Arc::new(unsafe {
                create_slot(gpu, extent, retired.clone())
            }?));
        }
        let output = gpu.inner.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-gpu.native-input.private"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[wgpu::TextureFormat::Bgra8UnormSrgb],
        });
        let output = GpuTexture::wrap_as(gpu, output, wgpu::TextureFormat::Bgra8UnormSrgb);
        Ok(Self {
            gpu: gpu.clone(),
            slots,
            retired,
            output,
        })
    }

    /// Never waits. `None` means every slot is reserved or still in GPU use.
    pub fn try_acquire(&mut self) -> Result<Option<NativeImportTarget>, NativeImportError> {
        if self.retired.load(Ordering::Acquire) || self.gpu.is_lost() {
            return Err(NativeImportError::Retired);
        }
        for slot in &self.slots {
            if slot.reserved.load(Ordering::Acquire) {
                continue;
            }
            let completed = unsafe { slot.fence.GetCompletedValue() };
            if completed == u64::MAX {
                return Err(NativeImportError::Retired);
            }
            let release = slot.last_release.load(Ordering::Acquire);
            if completed < release {
                continue;
            }
            slot.reserved.store(true, Ordering::Release);
            let ready = slot.next_ready.fetch_add(2, Ordering::Relaxed);
            return Ok(Some(NativeImportTarget {
                slot: Some(slot.clone()),
                ready,
                release,
            }));
        }
        Ok(None)
    }

    /// Copy a ready frame into the pool's private sRGB-sampled BGRA8 texture.
    /// The returned clone is a mutable latest output, not an immutable frame
    /// snapshot: subsequent stages overwrite it in host GPU queue order.
    /// Publish it only after submitting `frame`, and sample it on that same
    /// host queue. Replace the pool for a new stream or extent.
    /// Dropping the frame instead releases the input without publishing it.
    pub fn stage(
        &mut self,
        frame: &mut FrameContext,
        mut input: NativeImportFrame,
    ) -> Result<GpuTexture, NativeImportError> {
        if self.retired.load(Ordering::Acquire) {
            return Err(NativeImportError::Retired);
        }
        if frame.generation() != self.gpu.generation() {
            return Err(NativeImportError::DeviceMismatch {
                expected: self.gpu.generation(),
                found: frame.generation(),
            });
        }
        let slot = input.slot.as_ref().unwrap();
        if !self
            .slots
            .iter()
            .any(|candidate| Arc::ptr_eq(candidate, slot))
        {
            return Err(NativeImportError::WrongPool);
        }
        unsafe { slot.queue.Wait(&slot.fence, input.ready) }.map_err(native("queue wait"))?;
        let [width, height] = slot.extent;
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        frame.encoder_mut().copy_texture_to_texture(
            slot.texture.as_image_copy(),
            self.output.raw().as_image_copy(),
            size,
        );
        frame.encoder_mut().transition_resources(
            std::iter::empty(),
            std::iter::once(wgpu::TextureTransition {
                texture: &slot.texture,
                selector: None,
                state: wgpu::TextureUses::PRESENT,
            }),
        );
        let release = ReleaseAfterSubmit {
            slot: input.slot.take(),
            ready: input.ready,
        };
        frame.after_submit(Box::new(move || release.submitted()));
        Ok(self.output.clone())
    }

    pub fn retire(&mut self) {
        self.retired.store(true, Ordering::Release);
    }
}

impl Drop for NativeImportPool {
    fn drop(&mut self) {
        self.retire();
        // Covers native signals already queued before retirement. Outstanding
        // targets/ready frames/recordings retain their own slot and add another
        // tail marker if they release it later, after this marker was queued.
        let retained = self.slots.clone();
        self.gpu.inner.queue.submit(std::iter::empty());
        self.gpu
            .inner
            .queue
            .on_submitted_work_done(move || drop(retained));
    }
}

unsafe fn create_slot(
    gpu: &GpuContext,
    extent: [u32; 2],
    retired: Arc<AtomicBool>,
) -> Result<Slot, NativeImportError> {
    let hal_device = unsafe { gpu.inner.device.as_hal::<wgpu::hal::api::Dx12>() }
        .ok_or(NativeImportError::Unsupported("DX12 device unavailable"))?;
    let hal_queue = unsafe { gpu.inner.queue.as_hal::<wgpu::hal::api::Dx12>() }
        .ok_or(NativeImportError::Unsupported("DX12 queue unavailable"))?;
    let device = hal_device.raw_device();
    let [width, height] = extent;
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Width: width as u64,
        Height: height,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
        ..Default::default()
    };
    let mut resource = None;
    unsafe {
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_SHARED,
            &desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            &mut resource,
        )
    }
    .map_err(native("CreateCommittedResource"))?;
    let resource: windows::Win32::Graphics::Direct3D12::ID3D12Resource =
        resource.ok_or(NativeImportError::Unsupported("no shared resource"))?;
    let fence: ID3D12Fence =
        unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_SHARED) }.map_err(native("CreateFence"))?;
    let texture_handle =
        unsafe { device.CreateSharedHandle(&resource, None, GENERIC_ALL.0, PCWSTR::null()) }
            .map_err(native("CreateSharedHandle(texture)"))?;
    let fence_handle =
        match unsafe { device.CreateSharedHandle(&fence, None, GENERIC_ALL.0, PCWSTR::null()) } {
            Ok(handle) => handle,
            Err(error) => {
                unsafe {
                    let _ = CloseHandle(texture_handle);
                }
                return Err(native("CreateSharedHandle(fence)")(error));
            }
        };
    let luid = unsafe { device.GetAdapterLuid() };
    let queue = hal_queue.as_raw().clone();
    drop(hal_queue);
    drop(hal_device);
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = unsafe {
        gpu.inner
            .device
            .create_texture_from_hal::<wgpu::hal::api::Dx12>(
                wgpu::hal::dx12::Device::texture_from_raw(
                    resource,
                    wgpu::TextureFormat::Bgra8Unorm,
                    wgpu::TextureDimension::D2,
                    size,
                    1,
                    1,
                ),
                &wgpu::TextureDescriptor {
                    label: Some("nana-gpu.native-input.shared"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
                wgpu::TextureUses::PRESENT,
            )
    };
    Ok(Slot {
        texture,
        texture_handle,
        fence_handle,
        fence,
        queue,
        gpu: gpu.clone(),
        extent,
        adapter_luid: (i64::from(luid.HighPart) << 32) | i64::from(luid.LowPart),
        reserved: AtomicBool::new(false),
        next_ready: AtomicU64::new(1),
        last_release: AtomicU64::new(0),
        retired,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{HMODULE, LUID};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
        ID3D11Device1, ID3D11Device5, ID3D11DeviceContext, ID3D11DeviceContext4, ID3D11Fence,
        ID3D11Texture2D,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter, IDXGIFactory4,
    };
    use windows::core::Interface;

    fn gpu() -> GpuContext {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("DX12 test adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        GpuContext::adopt(adapter, device, queue, false)
    }

    struct Producer {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
    }

    impl Producer {
        fn new(luid: i64) -> Self {
            unsafe {
                let factory: IDXGIFactory4 =
                    CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).unwrap();
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

        fn write(&self, target: NativeImportTarget, color: [f32; 4]) -> NativeImportFrame {
            unsafe {
                let device1: ID3D11Device1 = self.device.cast().unwrap();
                let device5: ID3D11Device5 = self.device.cast().unwrap();
                let context4: ID3D11DeviceContext4 = self.context.cast().unwrap();
                let texture: ID3D11Texture2D = device1
                    .OpenSharedResource1(HANDLE(target.texture_handle().as_raw_handle()))
                    .unwrap();
                let mut fence: Option<ID3D11Fence> = None;
                device5
                    .OpenSharedFence(HANDLE(target.fence_handle().as_raw_handle()), &mut fence)
                    .unwrap();
                let fence = fence.unwrap();
                let mut view = None;
                self.device
                    .CreateRenderTargetView(&texture, None, Some(&mut view))
                    .unwrap();
                context4.Wait(&fence, target.release_value()).unwrap();
                self.context
                    .ClearRenderTargetView(view.as_ref().unwrap(), &color);
                context4.Signal(&fence, target.ready_value()).unwrap();
                self.context.Flush();
                target.into_ready()
            }
        }
    }

    fn read(gpu: &GpuContext, texture: &GpuTexture) -> Vec<u8> {
        let (width, height) = texture.size();
        let stride = (width * 4).div_ceil(256) * 256;
        let buffer = gpu.inner.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (stride * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut frame = gpu.begin_frame("native input test readback");
        frame.encoder_mut().copy_texture_to_buffer(
            texture.raw().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        frame.submit();
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.inner
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(15)),
            })
            .unwrap();
        let mapped = buffer.slice(..).get_mapped_range().unwrap();
        let mut bytes = Vec::new();
        for row in mapped.chunks_exact(stride as usize) {
            bytes.extend_from_slice(&row[..width as usize * 4]);
        }
        bytes
    }

    fn await_slot(pool: &mut NativeImportPool) -> NativeImportTarget {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(target) = pool.try_acquire().unwrap() {
                return target;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native release did not complete"
            );
            pool.gpu.poll();
            std::thread::yield_now();
        }
    }

    #[test]
    fn d3d11_gpu_writes_reach_private_output_and_slots_recycle() {
        let gpu = gpu();
        let mut pool = NativeImportPool::new(&gpu, [64, 32]).unwrap();
        let first = pool.try_acquire().unwrap().unwrap();
        let producer = Producer::new(first.adapter_luid());
        drop(first);
        for iteration in 0..12 {
            let target = pool.try_acquire().unwrap().unwrap();
            let color = if iteration % 2 == 0 {
                [1.0, 0.0, 0.0, 1.0]
            } else {
                [0.0, 0.0, 1.0, 1.0]
            };
            let input = producer.write(target, color);
            let mut frame = gpu.begin_frame("native input copy");
            let output = pool.stage(&mut frame, input).unwrap();
            frame.submit();
            let expected = if iteration % 2 == 0 {
                [0, 0, 255, 255]
            } else {
                [255, 0, 0, 255]
            };
            assert!(
                read(&gpu, &output)
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| *pixel == expected)
            );
        }
    }

    #[test]
    fn reservations_discard_and_retirement_preserve_ownership() {
        let gpu = gpu();
        let mut pool = NativeImportPool::new(&gpu, [8, 8]).unwrap();
        let mut targets: Vec<_> = (0..NATIVE_IMPORT_SLOTS)
            .map(|_| pool.try_acquire().unwrap().unwrap())
            .collect();
        assert!(pool.try_acquire().unwrap().is_none());
        let producer = Producer::new(targets[0].adapter_luid());
        let input = producer.write(targets.remove(0), [0.0, 1.0, 0.0, 1.0]);
        let mut frame = gpu.begin_frame("discarded input copy");
        pool.stage(&mut frame, input).unwrap();
        assert!(pool.try_acquire().unwrap().is_none());
        frame.discard();
        gpu.inner
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(15)),
            })
            .unwrap();
        let recycled = await_slot(&mut pool);
        assert!(recycled.release_value() > 0);
        drop(recycled);
        let input = producer.write(pool.try_acquire().unwrap().unwrap(), [1.0; 4]);
        drop(input);
        gpu.inner
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(15)),
            })
            .unwrap();
        drop(await_slot(&mut pool));
        pool.retire();
        assert!(matches!(
            pool.try_acquire(),
            Err(NativeImportError::Retired)
        ));
        drop(targets);
    }

    #[test]
    fn cross_device_and_cross_pool_input_is_rejected() {
        let gpu = gpu();
        let other = self::gpu();
        let mut pool = NativeImportPool::new(&gpu, [8, 8]).unwrap();
        let target = pool.try_acquire().unwrap().unwrap();
        let producer = Producer::new(target.adapter_luid());
        let input = producer.write(target, [1.0; 4]);
        let mut frame = other.begin_frame("wrong device");
        assert!(matches!(
            pool.stage(&mut frame, input),
            Err(NativeImportError::DeviceMismatch { .. })
        ));
        frame.discard();
        let input = producer.write(pool.try_acquire().unwrap().unwrap(), [1.0; 4]);
        let mut other_pool = NativeImportPool::new(&gpu, [8, 8]).unwrap();
        let mut frame = gpu.begin_frame("wrong pool");
        assert!(matches!(
            other_pool.stage(&mut frame, input),
            Err(NativeImportError::WrongPool)
        ));
        frame.discard();
        gpu.inner
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(15)),
            })
            .unwrap();
    }

    #[test]
    fn retiring_before_a_pending_copy_submits_or_discards_retains_native_objects() {
        struct Gate(ID3D12Fence);
        impl Drop for Gate {
            fn drop(&mut self) {
                // Always unblock the queue, including on a failed assertion.
                unsafe {
                    let _ = self.0.Signal(1);
                }
            }
        }
        let gpu = gpu();
        for submit in [true, false] {
            let mut pool = NativeImportPool::new(&gpu, [16, 8]).unwrap();
            let target = pool.try_acquire().unwrap().unwrap();
            let weak = Arc::downgrade(target.slot.as_ref().unwrap());
            let producer = Producer::new(target.adapter_luid());
            let input = producer.write(target, [0.0, 1.0, 0.0, 1.0]);
            let gate = unsafe {
                let device = gpu.inner.device.as_hal::<wgpu::hal::api::Dx12>().unwrap();
                Gate(
                    device
                        .raw_device()
                        .CreateFence(0, D3D12_FENCE_FLAG_SHARED)
                        .unwrap(),
                )
            };
            {
                let queue = unsafe { gpu.inner.queue.as_hal::<wgpu::hal::api::Dx12>() }.unwrap();
                unsafe { queue.as_raw().Wait(&gate.0, 1) }.unwrap();
            }
            let mut frame = gpu.begin_frame("retired pending native input");
            let output = pool.stage(&mut frame, input).unwrap();
            drop(pool);
            if submit {
                frame.submit();
            } else {
                frame.discard();
            }
            gpu.poll();
            assert!(
                weak.upgrade().is_some(),
                "native objects died while queue was blocked"
            );
            unsafe { gate.0.Signal(1) }.unwrap();
            let pixels = read(&gpu, &output);
            if submit {
                assert!(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|pixel| *pixel == [0, 255, 0, 255])
                );
            } else {
                assert!(
                    pixels.iter().all(|byte| *byte == 0),
                    "discard submitted its copy"
                );
            }
            gpu.poll();
            assert!(
                weak.upgrade().is_none(),
                "native objects outlived completed retirement"
            );
        }
    }
}
