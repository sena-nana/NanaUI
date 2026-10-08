//! Native texture export: DX12 shared textures and a shared fence another
//! D3D device opens by handle (feature `native-export`, Windows).
//!
//! A [`NativeExportPool`] owns [`NATIVE_EXPORT_SLOTS`] committed BGRA8
//! textures created with `D3D12_HEAP_FLAG_SHARED`, exported as NT handles,
//! and one shared fence. Inside the producer's own [`FrameContext`] the pool
//! copies a rendered texture into the next slot; [`FrameContext::submit`]
//! signals the frame's *ready* value on the device queue right after the
//! submit, under the submission guard. The consumer waits for that value on
//! its own GPU timeline, reads the slot and signals the frame's *release*
//! value. Frame `k` (from 0) is ready at `2k + 1` and released at `2k + 2`.
//!
//! Nobody waits on the CPU. A frame the consumer has not released yet makes
//! the next [`NativeExportPool::stage`] answer [`NativeExportOutcome::Deferred`].
//! Both sides signal the one fence, so the producer only issues frame `k + 1`
//! once release `2k + 2` completed: the fence value never goes backwards.
//!
//! A [`NativeFrameToken`] dropped without [`NativeFrameToken::accept_release`]
//! is released by the producer itself, with a signal on its own queue that
//! is ordered after the ready one.

use std::os::windows::io::{BorrowedHandle, RawHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_FENCE_FLAG_SHARED, D3D12_HEAP_FLAG_SHARED, D3D12_HEAP_PROPERTIES,
    D3D12_HEAP_TYPE_DEFAULT, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
    D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, D3D12_RESOURCE_STATE_COPY_DEST,
    D3D12_TEXTURE_LAYOUT_UNKNOWN, ID3D12CommandQueue, ID3D12Fence, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::PCWSTR;

use crate::{DeviceGeneration, FrameContext, GpuBackend, GpuContext, GpuTexture, GpuTextureFormat};

/// Shared textures in one pool.
pub const NATIVE_EXPORT_SLOTS: usize = 3;

/// The texel format of every exported texture: `DXGI_FORMAT_B8G8R8A8_UNORM`.
pub const NATIVE_EXPORT_FORMAT: GpuTextureFormat = GpuTextureFormat::BGRA8_UNORM;

/// Why the pool cannot be created or a frame cannot be exported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeExportError {
    /// The device is not on DX12, or this build cannot reach it.
    Unsupported(&'static str),
    /// Zero, or beyond the device's 2D limit.
    InvalidExtent { width: u32, height: u32 },
    /// The source texture or frame belongs to another device.
    DeviceMismatch {
        expected: DeviceGeneration,
        found: DeviceGeneration,
    },
    /// The source is not a BGRA8 texture of the pool's extent with `COPY_SRC`.
    IncompatibleSource,
    /// The frame holding a staged copy was dropped instead of submitted.
    NotSubmitted,
    /// The pool was retired (device switch or reconfiguration).
    Retired,
    /// A native call failed: which step, and its `HRESULT`.
    Native { step: &'static str, code: i32 },
}

impl std::fmt::Display for NativeExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(formatter, "native export unsupported: {reason}"),
            Self::InvalidExtent { width, height } => {
                write!(
                    formatter,
                    "native export extent {width}x{height} is invalid"
                )
            }
            Self::DeviceMismatch { expected, found } => write!(
                formatter,
                "native export resource belongs to device {found}, expected {expected}"
            ),
            Self::IncompatibleSource => formatter.write_str(
                "native export source must be a BGRA8 COPY_SRC texture of the pool's extent",
            ),
            Self::NotSubmitted => {
                formatter.write_str("the frame carrying the export copy was not submitted")
            }
            Self::Retired => formatter.write_str("the native export pool was retired"),
            Self::Native { step, code } => {
                write!(
                    formatter,
                    "native export {step} failed: HRESULT {code:#010x}"
                )
            }
        }
    }
}

impl std::error::Error for NativeExportError {}

fn native(step: &'static str) -> impl FnOnce(windows::core::Error) -> NativeExportError {
    move |error| NativeExportError::Native {
        step,
        code: error.code().0,
    }
}

/// Why a frame was not exported this time. The previous frame stays the
/// consumer's; nothing waited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeExportDeferral {
    /// The consumer has not released the previous frame yet. `waiting` is how
    /// long the producer has been waiting for that release.
    ConsumerBusy { waiting: Duration },
}

/// What [`NativeExportPool::stage`] did.
#[derive(Debug)]
pub enum NativeExportOutcome {
    /// The copy was recorded into the frame and its ready signal attached to
    /// the frame's submit. Hand it to [`NativeExportPool::finish`] after
    /// [`FrameContext::submit`].
    Staged(StagedNativeFrame),
    Deferred(NativeExportDeferral),
}

/// A copy recorded into a frame that has not been submitted yet.
#[derive(Debug)]
#[must_use = "finish the staged export after submitting its frame"]
pub struct StagedNativeFrame {
    pool: u64,
    slot: usize,
    ready: u64,
    signal: Arc<AtomicU8>,
}

const SIGNAL_PENDING: u8 = 0;
const SIGNAL_DONE: u8 = 1;
const SIGNAL_FAILED: u8 = 2;

/// COM objects moved into a submit hook. D3D12 command queues and fences are
/// free-threaded.
struct SendQueue(ID3D12CommandQueue, ID3D12Fence);
// SAFETY: `ID3D12CommandQueue::Signal` and the fence are documented as
// free-threaded; the hook runs once, under the device's submission guard.
unsafe impl Send for SendQueue {}

/// The exported handles. Closed when the pool and every token are gone.
struct SharedHandles {
    pool: u64,
    adapter_luid: i64,
    extent: [u32; 2],
    textures: [HANDLE; NATIVE_EXPORT_SLOTS],
    fence_handle: HANDLE,
    fence: ID3D12Fence,
    queue: ID3D12CommandQueue,
    gpu: GpuContext,
}

// SAFETY: NT handles are process-wide values and the COM objects are
// free-threaded D3D12 interfaces; nothing here is thread-affine.
unsafe impl Send for SharedHandles {}
unsafe impl Sync for SharedHandles {}

impl Drop for SharedHandles {
    fn drop(&mut self) {
        for handle in self
            .textures
            .iter()
            .chain(std::iter::once(&self.fence_handle))
        {
            if !handle.is_invalid() {
                // SAFETY: each handle was returned by `CreateSharedHandle` and
                // is closed exactly once, here.
                unsafe {
                    let _ = CloseHandle(*handle);
                }
            }
        }
    }
}

impl SharedHandles {
    /// Signal `value` on the device queue, under the submission guard: the
    /// queue order puts it after everything already submitted.
    fn signal(&self, value: u64) -> Result<(), NativeExportError> {
        let _guard = self.gpu.lock_submission();
        // SAFETY: the fence and queue belong to the live device `gpu` holds.
        unsafe { self.queue.Signal(&self.fence, value) }.map_err(native("queue signal"))
    }
}

/// One exported frame: which slot holds it and the fence values that bracket
/// the consumer's access.
///
/// The consumer opens the pool's handles once per [`Self::pool_generation`]
/// (`OpenSharedResource1` for every slot, `OpenSharedFence`), then for each
/// token: GPU-waits for [`Self::ready_value`], reads [`Self::slot`], and
/// GPU-signals [`Self::release_value`] after the read, on the same context.
/// Call [`Self::accept_release`] before doing so. A token dropped without it
/// is released by the producer.
pub struct NativeFrameToken {
    shared: Arc<SharedHandles>,
    slot: usize,
    ready: u64,
    accepted: bool,
}

impl std::fmt::Debug for NativeFrameToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeFrameToken")
            .field("pool", &self.shared.pool)
            .field("slot", &self.slot)
            .field("ready", &self.ready)
            .field("accepted", &self.accepted)
            .finish()
    }
}

impl NativeFrameToken {
    /// Changes whenever the handles change (new pool, resize, device switch).
    pub fn pool_generation(&self) -> u64 {
        self.shared.pool
    }

    /// `LUID` of the adapter the textures live on, as `(HighPart << 32) | LowPart`.
    pub fn adapter_luid(&self) -> i64 {
        self.shared.adapter_luid
    }

    pub fn extent(&self) -> [u32; 2] {
        self.shared.extent
    }

    pub fn format(&self) -> GpuTextureFormat {
        NATIVE_EXPORT_FORMAT
    }

    /// Which shared texture holds this frame.
    pub fn slot(&self) -> usize {
        self.slot
    }

    /// The NT handle of shared texture `slot`, valid while this token lives.
    pub fn texture_handle(&self, slot: usize) -> Option<BorrowedHandle<'_>> {
        let handle = self.shared.textures.get(slot)?;
        // SAFETY: the handle stays open until `shared` is dropped, which
        // cannot happen while `self` is borrowed.
        Some(unsafe { BorrowedHandle::borrow_raw(handle.0 as RawHandle) })
    }

    /// The NT handle of the shared fence, valid while this token lives.
    pub fn fence_handle(&self) -> BorrowedHandle<'_> {
        // SAFETY: as for `texture_handle`.
        unsafe { BorrowedHandle::borrow_raw(self.shared.fence_handle.0 as RawHandle) }
    }

    /// Fence value at which the slot holds this frame.
    pub fn ready_value(&self) -> u64 {
        self.ready
    }

    /// Fence value the consumer signals once it no longer reads the slot.
    pub fn release_value(&self) -> u64 {
        self.ready + 1
    }

    /// The consumer takes over signalling [`Self::release_value`]. It must do
    /// so on its GPU timeline after waiting for [`Self::ready_value`]; a value
    /// signalled before the producer's ready signal ran would move the fence
    /// backwards when that one lands.
    pub fn accept_release(&mut self) {
        self.accepted = true;
    }
}

impl Drop for NativeFrameToken {
    fn drop(&mut self) {
        if !self.accepted {
            // Nobody will read the slot: release it on the producer's queue,
            // where it is ordered after the ready signal.
            let _ = self.shared.signal(self.release_value());
        }
    }
}

struct Slot {
    texture: wgpu::Texture,
    /// Release value that frees it; 0 when never used.
    release: u64,
}

/// A pool of DX12 shared textures and a shared fence on the [`GpuContext`]'s
/// own device and queue.
pub struct NativeExportPool {
    gpu: GpuContext,
    shared: Option<Arc<SharedHandles>>,
    slots: Vec<Slot>,
    next_slot: usize,
    /// Frames issued so far: the next one is ready at `2 * issued + 1`.
    issued: u64,
    /// Release value of the last issued frame, until it completed.
    outstanding: Option<(u64, Instant)>,
}

impl std::fmt::Debug for NativeExportPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeExportPool")
            .field("generation", &self.pool_generation())
            .field("extent", &self.extent())
            .field("issued", &self.issued)
            .field("retired", &self.shared.is_none())
            .finish()
    }
}

fn next_pool_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl NativeExportPool {
    /// Creates [`NATIVE_EXPORT_SLOTS`] shared textures of `extent` and the
    /// shared fence on `gpu`'s device. Requires a DX12 device
    /// ([`crate::GpuCapability::NativeTextureExport`]).
    pub fn new(gpu: &GpuContext, extent: [u32; 2]) -> Result<Self, NativeExportError> {
        let [width, height] = extent;
        let max = gpu.capabilities().max_texture_dimension_2d();
        if width == 0 || height == 0 || width > max || height > max {
            return Err(NativeExportError::InvalidExtent { width, height });
        }
        if gpu.capabilities().backend() != GpuBackend::Dx12 {
            return Err(NativeExportError::Unsupported("the device is not on DX12"));
        }
        if gpu.is_lost() {
            return Err(NativeExportError::Unsupported("the device is lost"));
        }
        // SAFETY: `gpu` is a live DX12 device; the HAL objects are only used
        // for creation calls that D3D12 allows from any thread.
        unsafe { create_pool(gpu, extent) }
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    /// `None` once retired.
    pub fn pool_generation(&self) -> Option<u64> {
        self.shared.as_ref().map(|shared| shared.pool)
    }

    pub fn extent(&self) -> Option<[u32; 2]> {
        self.shared.as_ref().map(|shared| shared.extent)
    }

    pub fn adapter_luid(&self) -> Option<i64> {
        self.shared.as_ref().map(|shared| shared.adapter_luid)
    }

    pub fn is_retired(&self) -> bool {
        self.shared.is_none()
    }

    /// Frames handed out so far.
    pub fn issued(&self) -> u64 {
        self.issued
    }

    /// The fence value both sides have completed.
    pub fn completed_value(&self) -> Option<u64> {
        let shared = self.shared.as_ref()?;
        // SAFETY: the fence belongs to the live pool.
        Some(unsafe { shared.fence.GetCompletedValue() })
    }

    /// Stops exporting. The textures stay alive for tokens still held; their
    /// handles close when the last one is dropped. The consumer reopens on
    /// the next pool's generation.
    pub fn retire(&mut self) {
        self.shared = None;
        self.slots.clear();
        self.outstanding = None;
    }

    /// Records a copy of `source` into the next free slot of `frame` and
    /// attaches the frame's ready signal to its submit. Never waits: when the
    /// consumer still holds the previous frame this answers `Deferred` and
    /// records nothing.
    pub fn stage(
        &mut self,
        frame: &mut FrameContext,
        source: &GpuTexture,
    ) -> Result<NativeExportOutcome, NativeExportError> {
        let shared = Arc::clone(self.shared.as_ref().ok_or(NativeExportError::Retired)?);
        for found in [frame.generation(), source.generation()] {
            if found != self.gpu.generation() {
                return Err(NativeExportError::DeviceMismatch {
                    expected: self.gpu.generation(),
                    found,
                });
            }
        }
        let (width, height) = source.size();
        if source.format() != NATIVE_EXPORT_FORMAT
            || [width, height] != shared.extent
            || !source.usage().contains(crate::GpuTextureUsages::COPY_SRC)
        {
            return Err(NativeExportError::IncompatibleSource);
        }
        // SAFETY: the fence belongs to the live pool.
        let completed = unsafe { shared.fence.GetCompletedValue() };
        if completed == u64::MAX {
            // A removed device reports every fence as complete.
            return Err(NativeExportError::Native {
                step: "fence (device removed)",
                code: 0x887A_0005_u32 as i32,
            });
        }
        if let Some((release, since)) = self.outstanding {
            if completed < release {
                return Ok(NativeExportOutcome::Deferred(
                    NativeExportDeferral::ConsumerBusy {
                        waiting: since.elapsed(),
                    },
                ));
            }
            self.outstanding = None;
        }
        let slot = self.next_slot;
        debug_assert!(self.slots[slot].release <= completed);
        let ready = 2 * self.issued + 1;
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        frame.encoder_mut().copy_texture_to_texture(
            source.raw().as_image_copy(),
            self.slots[slot].texture.as_image_copy(),
            extent,
        );
        let signal = Arc::new(AtomicU8::new(SIGNAL_PENDING));
        let hook_signal = Arc::clone(&signal);
        let target = SendQueue(shared.queue.clone(), shared.fence.clone());
        frame.after_submit(Box::new(move || {
            let target = target;
            // SAFETY: runs right after `queue.submit`, under the submission
            // guard, on the queue that executed the copy.
            let result = unsafe { target.0.Signal(&target.1, ready) };
            hook_signal.store(
                if result.is_ok() {
                    SIGNAL_DONE
                } else {
                    SIGNAL_FAILED
                },
                Ordering::Release,
            );
        }));
        Ok(NativeExportOutcome::Staged(StagedNativeFrame {
            pool: shared.pool,
            slot,
            ready,
            signal,
        }))
    }

    /// Turns a staged copy into a token once its frame was submitted. A frame
    /// that was dropped instead leaves the pool as it was.
    pub fn finish(
        &mut self,
        staged: StagedNativeFrame,
    ) -> Result<NativeFrameToken, NativeExportError> {
        let shared = Arc::clone(self.shared.as_ref().ok_or(NativeExportError::Retired)?);
        if staged.pool != shared.pool {
            return Err(NativeExportError::Retired);
        }
        match staged.signal.load(Ordering::Acquire) {
            SIGNAL_DONE => {}
            SIGNAL_FAILED => {
                return Err(NativeExportError::Native {
                    step: "queue signal",
                    code: 0,
                });
            }
            _ => return Err(NativeExportError::NotSubmitted),
        }
        let release = staged.ready + 1;
        self.issued += 1;
        self.slots[staged.slot].release = release;
        self.next_slot = (staged.slot + 1) % self.slots.len();
        self.outstanding = Some((release, Instant::now()));
        Ok(NativeFrameToken {
            shared,
            slot: staged.slot,
            ready: staged.ready,
            accepted: false,
        })
    }
}

unsafe fn create_pool(
    gpu: &GpuContext,
    extent: [u32; 2],
) -> Result<NativeExportPool, NativeExportError> {
    let [width, height] = extent;
    let device = &gpu.inner.device;
    let queue = &gpu.inner.queue;
    let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }.ok_or(
        NativeExportError::Unsupported("the device is not a DX12 device"),
    )?;
    let hal_queue = unsafe { queue.as_hal::<wgpu::hal::api::Dx12>() }.ok_or(
        NativeExportError::Unsupported("the queue is not a DX12 queue"),
    )?;
    let raw_device = hal_device.raw_device().clone();
    let raw_queue = hal_queue.as_raw().clone();
    drop(hal_queue);
    let heap = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        ..Default::default()
    };
    let resource_desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: u64::from(width),
        Height: height,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        // D3D11's `OpenSharedResource1` refuses a shared D3D12 texture without
        // render-target use (E_INVALIDARG); with it, the D3D11 view also
        // carries the render-target bind flag a Spout sender checks for.
        Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
    };
    let fence: ID3D12Fence = unsafe { raw_device.CreateFence(0, D3D12_FENCE_FLAG_SHARED) }
        .map_err(native("CreateFence"))?;
    let mut handles = [HANDLE::default(); NATIVE_EXPORT_SLOTS];
    let mut resources = Vec::with_capacity(NATIVE_EXPORT_SLOTS);
    let close = |handles: &[HANDLE]| {
        for handle in handles.iter().filter(|handle| !handle.is_invalid()) {
            unsafe {
                let _ = CloseHandle(*handle);
            }
        }
    };
    for handle in &mut handles {
        let mut resource: Option<ID3D12Resource> = None;
        if let Err(error) = unsafe {
            raw_device.CreateCommittedResource(
                &heap,
                D3D12_HEAP_FLAG_SHARED,
                &resource_desc,
                D3D12_RESOURCE_STATE_COPY_DEST,
                None,
                &mut resource,
            )
        } {
            close(&handles);
            return Err(native("CreateCommittedResource")(error));
        }
        let Some(resource) = resource else {
            close(&handles);
            return Err(NativeExportError::Native {
                step: "CreateCommittedResource",
                code: 0,
            });
        };
        match unsafe {
            raw_device.CreateSharedHandle(&resource, None, GENERIC_ALL.0, PCWSTR::null())
        } {
            Ok(shared) => *handle = shared,
            Err(error) => {
                close(&handles);
                return Err(native("CreateSharedHandle(texture)")(error));
            }
        }
        resources.push(resource);
    }
    let fence_handle =
        match unsafe { raw_device.CreateSharedHandle(&fence, None, GENERIC_ALL.0, PCWSTR::null()) }
        {
            Ok(handle) => handle,
            Err(error) => {
                close(&handles);
                return Err(native("CreateSharedHandle(fence)")(error));
            }
        };
    let luid = unsafe { raw_device.GetAdapterLuid() };
    let adapter_luid = (i64::from(luid.HighPart) << 32) | i64::from(luid.LowPart);
    drop(hal_device);
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let slots = resources
        .into_iter()
        .map(|resource| {
            let hal_texture = unsafe {
                wgpu::hal::dx12::Device::texture_from_raw(
                    resource,
                    wgpu::TextureFormat::Bgra8Unorm,
                    wgpu::TextureDimension::D2,
                    size,
                    1,
                    1,
                )
            };
            let texture = unsafe {
                device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                    hal_texture,
                    &wgpu::TextureDescriptor {
                        label: Some("nana-gpu.native-export"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Bgra8Unorm,
                        usage: wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::COPY_DST,
                )
            };
            Slot {
                texture,
                release: 0,
            }
        })
        .collect();
    Ok(NativeExportPool {
        gpu: gpu.clone(),
        shared: Some(Arc::new(SharedHandles {
            pool: next_pool_generation(),
            adapter_luid,
            extent,
            textures: handles,
            fence_handle,
            fence,
            queue: raw_queue,
            gpu: gpu.clone(),
        })),
        slots,
        next_slot: 0,
        issued: 0,
        outstanding: None,
    })
}
