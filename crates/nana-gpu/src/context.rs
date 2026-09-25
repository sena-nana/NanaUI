use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::frame::FrameContext;
use crate::policy::GpuDeviceState;
use crate::texture::{
    GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion, GpuTextureUsages,
};
use crate::{FrameId, GpuError};

static NEXT_DEVICE_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Identity of one device. Every device a process adopts gets a fresh
/// generation; clones of a [`GpuContext`] share it. Resources and caches
/// created on one generation are invalid on any other.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceGeneration(NonZeroU64);

impl DeviceGeneration {
    pub(crate) fn next() -> Self {
        let value = NEXT_DEVICE_GENERATION.fetch_add(1, Ordering::Relaxed);
        Self(NonZeroU64::new(value).expect("device generation counter overflowed"))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Debug for DeviceGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "DeviceGeneration({})", self.0)
    }
}

impl fmt::Display for DeviceGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{}", self.0)
    }
}

/// Graphics API under the backend. Variant names follow WGPU's so diagnostics
/// keep their spelling.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuBackend {
    Noop,
    Vulkan,
    Metal,
    Dx12,
    Gl,
    BrowserWebGpu,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuDeviceType {
    Other,
    IntegratedGpu,
    DiscreteGpu,
    VirtualGpu,
    Cpu,
}

/// Optional device features Nana workloads choose a path by. Only features
/// the device was created with are reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct GpuFeatureSet {
    dual_source_blending: bool,
    pipeline_cache: bool,
    timestamp_query: bool,
    resource_arrays: bool,
    indirect: bool,
    multi_draw: bool,
    external_texture: bool,
    transient_hint: bool,
}

impl GpuFeatureSet {
    /// Subpixel (ClearType-style) text.
    pub const fn dual_source_blending(self) -> bool {
        self.dual_source_blending
    }

    pub const fn pipeline_cache(self) -> bool {
        self.pipeline_cache
    }

    pub const fn timestamp_query(self) -> bool {
        self.timestamp_query
    }
    pub const fn resource_arrays(self) -> bool {
        self.resource_arrays
    }
    pub const fn indirect(self) -> bool {
        self.indirect
    }
    pub const fn multi_draw(self) -> bool {
        self.multi_draw
    }
    pub const fn external_texture(self) -> bool {
        self.external_texture
    }
    pub const fn transient_hint(self) -> bool {
        self.transient_hint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuLimits {
    pub max_bind_groups: u32,
    pub max_bindings_per_group: u32,
    pub max_uniform_buffer_binding_size: u64,
    pub max_storage_buffer_binding_size: u64,
    pub min_uniform_buffer_offset_alignment: u32,
    pub min_storage_buffer_offset_alignment: u32,
    pub max_buffer_size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuCapability {
    ResourceArrays,
    Indirect,
    MultiDraw,
    Timestamps,
    ExternalTexture,
    TransientHint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuCapabilityOutcome {
    pub capability: GpuCapability,
    pub supported: bool,
    pub fallback_reason: Option<&'static str>,
}

impl GpuCapabilities {
    pub const fn supports(&self, capability: GpuCapability) -> bool {
        let features = self.features;
        match capability {
            GpuCapability::ResourceArrays => features.resource_arrays(),
            GpuCapability::Indirect => features.indirect(),
            GpuCapability::MultiDraw => features.multi_draw(),
            GpuCapability::Timestamps => features.timestamp_query(),
            GpuCapability::ExternalTexture => features.external_texture(),
            GpuCapability::TransientHint => features.transient_hint(),
        }
    }
    pub const fn capability(&self, capability: GpuCapability) -> GpuCapabilityOutcome {
        let supported = self.supports(capability);
        GpuCapabilityOutcome {
            capability,
            supported,
            fallback_reason: if supported {
                None
            } else {
                Some(match capability {
                    GpuCapability::ResourceArrays => "resource arrays are unavailable",
                    GpuCapability::Indirect => "indirect first-instance is unavailable",
                    GpuCapability::MultiDraw => "multi-draw indirect count is unavailable",
                    GpuCapability::Timestamps => "timestamp queries are unavailable",
                    GpuCapability::ExternalTexture => "external resource interop is unavailable",
                    GpuCapability::TransientHint => "transient resource hints are unavailable",
                })
            },
        }
    }
}

/// What the adopted device is and can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuCapabilities {
    backend: GpuBackend,
    device_type: GpuDeviceType,
    adapter_name: Arc<str>,
    driver: Arc<str>,
    driver_info: Arc<str>,
    vendor_id: u32,
    device_id: u32,
    max_texture_dimension_2d: u32,
    features: GpuFeatureSet,
    limits: GpuLimits,
}

impl GpuCapabilities {
    fn read(adapter: &wgpu::Adapter, device: &wgpu::Device) -> Self {
        let info = adapter.get_info();
        let features = device.features();
        Self {
            backend: match info.backend {
                wgpu::Backend::Noop => GpuBackend::Noop,
                wgpu::Backend::Vulkan => GpuBackend::Vulkan,
                wgpu::Backend::Metal => GpuBackend::Metal,
                wgpu::Backend::Dx12 => GpuBackend::Dx12,
                wgpu::Backend::Gl => GpuBackend::Gl,
                wgpu::Backend::BrowserWebGpu => GpuBackend::BrowserWebGpu,
            },
            device_type: match info.device_type {
                wgpu::DeviceType::Other => GpuDeviceType::Other,
                wgpu::DeviceType::IntegratedGpu => GpuDeviceType::IntegratedGpu,
                wgpu::DeviceType::DiscreteGpu => GpuDeviceType::DiscreteGpu,
                wgpu::DeviceType::VirtualGpu => GpuDeviceType::VirtualGpu,
                wgpu::DeviceType::Cpu => GpuDeviceType::Cpu,
            },
            adapter_name: info.name.into(),
            driver: info.driver.into(),
            driver_info: info.driver_info.into(),
            vendor_id: info.vendor,
            device_id: info.device,
            max_texture_dimension_2d: device.limits().max_texture_dimension_2d,
            features: GpuFeatureSet {
                dual_source_blending: features.contains(wgpu::Features::DUAL_SOURCE_BLENDING),
                pipeline_cache: features.contains(wgpu::Features::PIPELINE_CACHE),
                timestamp_query: features.contains(wgpu::Features::TIMESTAMP_QUERY),
                // The current logical ResourceArray maps to sampled texture
                // arrays. Storage resource arrays are a separate ABI shape
                // and must not advertise support for this declaration.
                resource_arrays: features.contains(wgpu::Features::TEXTURE_BINDING_ARRAY),
                indirect: features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
                multi_draw: features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
                external_texture: false,
                transient_hint: false,
            },
            limits: GpuLimits {
                max_bind_groups: device.limits().max_bind_groups,
                max_bindings_per_group: device.limits().max_bindings_per_bind_group,
                max_uniform_buffer_binding_size: u64::from(
                    device.limits().max_uniform_buffer_binding_size,
                ),
                max_storage_buffer_binding_size: u64::from(
                    device.limits().max_storage_buffer_binding_size,
                ),
                min_uniform_buffer_offset_alignment: device
                    .limits()
                    .min_uniform_buffer_offset_alignment,
                min_storage_buffer_offset_alignment: device
                    .limits()
                    .min_storage_buffer_offset_alignment,
                max_buffer_size: device.limits().max_buffer_size,
            },
        }
    }

    pub const fn backend(&self) -> GpuBackend {
        self.backend
    }

    pub const fn device_type(&self) -> GpuDeviceType {
        self.device_type
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    pub fn driver(&self) -> &str {
        &self.driver
    }

    pub fn driver_info(&self) -> &str {
        &self.driver_info
    }

    /// PCI vendor id, or 0 when the backend does not report one.
    pub const fn vendor_id(&self) -> u32 {
        self.vendor_id
    }

    /// PCI device id, or 0 when the backend does not report one.
    pub const fn device_id(&self) -> u32 {
        self.device_id
    }

    pub const fn max_texture_dimension_2d(&self) -> u32 {
        self.max_texture_dimension_2d
    }

    pub const fn features(&self) -> GpuFeatureSet {
        self.features
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuLossReason {
    /// Destroyed on purpose (`destroy`, or a host's own recovery probe).
    Destroyed,
    Unknown,
}

/// Why a device stopped working, as the backend reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuDeviceLost {
    pub reason: GpuLossReason,
    pub message: String,
}

#[derive(Default)]
struct LossState {
    lost: AtomicBool,
    report: Mutex<Option<GpuDeviceLost>>,
}

impl LossState {
    fn mark(&self, report: GpuDeviceLost) {
        if let Ok(mut slot) = self.report.lock() {
            slot.get_or_insert(report);
        }
        self.lost.store(true, Ordering::Release);
    }
}

pub(crate) struct GpuInner {
    generation: DeviceGeneration,
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) adapter_info: wgpu::AdapterInfo,
    capabilities: GpuCapabilities,
    /// Whether WGPU trusts the WebGPU format table on this device, rather
    /// than asking the adapter (see [`GpuContext::format_features`]).
    webgpu_format_table: bool,
    /// Serializes queue work against `Surface::configure`, which waits for the
    /// GPU to go idle: a submit racing it from another thread fails with
    /// `GpuWaitTimeout`. Every submit and upload the contract performs holds
    /// it for reading only for the duration of the call; reconfiguration takes
    /// it for writing.
    submission: RwLock<()>,
    loss: Arc<LossState>,
    next_frame: AtomicU64,
    pub(crate) next_submission: AtomicU64,
    pub(crate) submission_order: Mutex<()>,
    policy: GpuDeviceState,
}

/// The one device a process renders with. Cloning is cheap and keeps the
/// same [`DeviceGeneration`]; a replaced device is a new `GpuContext`.
#[derive(Clone)]
pub struct GpuContext {
    pub(crate) inner: Arc<GpuInner>,
}

impl fmt::Debug for GpuContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuContext")
            .field("generation", &self.inner.generation)
            .field("backend", &self.inner.capabilities.backend)
            .field("adapter", &self.inner.capabilities.adapter_name)
            .field("lost", &self.is_lost())
            .finish()
    }
}

impl GpuContext {
    pub(crate) fn adopt(
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        track_loss: bool,
    ) -> Self {
        let loss = Arc::new(LossState::default());
        if track_loss {
            let callback = Arc::clone(&loss);
            device.set_device_lost_callback(move |reason, message| {
                callback.mark(GpuDeviceLost {
                    reason: match reason {
                        wgpu::DeviceLostReason::Destroyed => GpuLossReason::Destroyed,
                        _ => GpuLossReason::Unknown,
                    },
                    message,
                });
            });
        }
        let capabilities = GpuCapabilities::read(&adapter, &device);
        let adapter_info = adapter.get_info();
        let webgpu_format_table = !device
            .features()
            .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
            && adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::WEBGPU_TEXTURE_FORMAT_SUPPORT);
        let generation = DeviceGeneration::next();
        let policy = GpuDeviceState::new_with_device(generation, &device);
        Self {
            inner: Arc::new(GpuInner {
                generation,
                adapter,
                device,
                queue,
                adapter_info,
                capabilities,
                webgpu_format_table,
                submission: RwLock::new(()),
                loss,
                next_frame: AtomicU64::new(1),
                next_submission: AtomicU64::new(1),
                submission_order: Mutex::new(()),
                policy,
            }),
        }
    }

    pub fn generation(&self) -> DeviceGeneration {
        self.inner.generation
    }

    pub fn capabilities(&self) -> &GpuCapabilities {
        &self.inner.capabilities
    }
    pub fn limits(&self) -> GpuLimits {
        self.inner.capabilities.limits
    }

    /// Shared per-device policy for uploads, transient resources, pipeline
    /// identities, realization hooks and retirement bookkeeping.
    pub fn policy(&self) -> &GpuDeviceState {
        &self.inner.policy
    }

    /// Whether the device was lost. Sticky: a lost device never recovers, the
    /// host replaces it with a new context.
    pub fn is_lost(&self) -> bool {
        self.inner.loss.lost.load(Ordering::Acquire)
    }

    pub fn lost_report(&self) -> Option<GpuDeviceLost> {
        self.inner.loss.report.lock().ok()?.clone()
    }

    /// Whether both contexts are the same device.
    pub fn same_device(&self, other: &GpuContext) -> bool {
        self.inner.generation == other.inner.generation
    }

    /// A 2D, single-mip, single-sample texture on this device.
    pub fn create_texture(
        &self,
        descriptor: &GpuTextureDescriptor<'_>,
    ) -> Result<GpuTexture, GpuError> {
        let max = self.inner.capabilities.max_texture_dimension_2d;
        if descriptor.width == 0
            || descriptor.height == 0
            || descriptor.width > max
            || descriptor.height > max
        {
            return Err(GpuError::InvalidExtent {
                width: descriptor.width,
                height: descriptor.height,
                max,
            });
        }
        if descriptor.usage == GpuTextureUsages::empty() {
            return Err(GpuError::EmptyUsage);
        }
        let usage = descriptor.usage.to_wgpu();
        let format = descriptor.format.to_wgpu();
        // Compressed formats come in whole blocks.
        let (block_width, block_height) = format.block_dimensions();
        if !descriptor.width.is_multiple_of(block_width)
            || !descriptor.height.is_multiple_of(block_height)
        {
            return Err(GpuError::InvalidExtent {
                width: descriptor.width,
                height: descriptor.height,
                max,
            });
        }
        if format.is_depth_stencil_format()
            || !self
                .format_features(format)
                .is_some_and(|features| features.allowed_usages.contains(usage))
        {
            return Err(GpuError::UnsupportedFormat(descriptor.format));
        }
        let texture = self.inner.device.create_texture(&wgpu::TextureDescriptor {
            label: descriptor.label,
            size: wgpu::Extent3d {
                width: descriptor.width,
                height: descriptor.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        Ok(GpuTexture::wrap(self, texture))
    }

    /// Upload tightly described rows into `region` of `texture`. Validated
    /// before it reaches the queue: a bad upload is an error, never a backend
    /// validation panic.
    pub fn write_texture(
        &self,
        texture: &GpuTexture,
        region: GpuTextureRegion,
        bytes: &[u8],
        bytes_per_row: u32,
    ) -> Result<(), GpuError> {
        self.check_device(texture.generation())?;
        if !texture.usage().contains(GpuTextureUsages::COPY_DST) {
            return Err(GpuError::MissingUsage(GpuTextureUsages::COPY_DST));
        }
        let (width, height) = texture.size();
        let fits = |start: u32, extent: u32, limit: u32| {
            start.checked_add(extent).is_some_and(|end| end <= limit)
        };
        if !fits(region.x, region.width, width) || !fits(region.y, region.height, height) {
            return Err(GpuError::RegionOutOfBounds);
        }
        // An empty region inside the texture is a no-op, as it is for WGPU.
        if region.width == 0 || region.height == 0 {
            return Ok(());
        }
        let format = texture.format();
        let Some(texel) = format.bytes_per_pixel() else {
            return Err(GpuError::UnsupportedFormat(format));
        };
        let row = region
            .width
            .checked_mul(texel)
            .ok_or(GpuError::RegionOutOfBounds)?;
        if bytes_per_row < row {
            return Err(GpuError::RowTooShort {
                needed: row,
                provided: bytes_per_row,
            });
        }
        let needed = (bytes_per_row as usize)
            .checked_mul(region.height as usize - 1)
            .and_then(|rows| rows.checked_add(row as usize))
            .ok_or(GpuError::RegionOutOfBounds)?;
        if bytes.len() < needed {
            return Err(GpuError::DataTooShort {
                needed,
                provided: bytes.len(),
            });
        }
        let aligned_row = bytes_per_row
            .checked_add(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT - 1)
            .map(|value| {
                value / wgpu::COPY_BYTES_PER_ROW_ALIGNMENT * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
            })
            .ok_or(GpuError::RegionOutOfBounds)?;
        let staging_len = (aligned_row as usize)
            .checked_mul(region.height as usize)
            .ok_or(GpuError::RegionOutOfBounds)?;
        let mut staging = vec![0u8; staging_len];
        for row_index in 0..region.height as usize {
            let source_start = row_index * bytes_per_row as usize;
            let destination_start = row_index * aligned_row as usize;
            staging[destination_start..destination_start + row as usize]
                .copy_from_slice(&bytes[source_start..source_start + row as usize]);
        }
        // Acquire the frame slot before reserving arena bytes. Otherwise a
        // first frame could reset the arena between staging and the copy.
        let mut frame = self.begin_frame("NanaUI texture upload");
        let staged = {
            let _submission = self.lock_submission();
            self.inner
                .policy
                .stage_upload(&self.inner.queue, &staging, 256)
        };
        let Some((reservation, backing)) = staged else {
            frame.discard();
            let _submission = self.lock_submission();
            self.inner.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: texture.raw(),
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: region.x,
                        y: region.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes[..needed],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(region.height),
                },
                wgpu::Extent3d {
                    width: region.width,
                    height: region.height,
                    depth_or_array_layers: 1,
                },
            );
            self.inner.policy.record_upload(needed as u64);
            return Ok(());
        };
        crate::frame::FrameContext::encoder_mut(&mut frame).copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &backing,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: reservation.offset,
                    bytes_per_row: Some(aligned_row),
                    rows_per_image: Some(region.height),
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture: texture.raw(),
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: region.x,
                    y: region.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: region.width,
                height: region.height,
                depth_or_array_layers: 1,
            },
        );
        let _submission = frame.submit();
        Ok(())
    }

    /// Start recording one frame. The returned [`FrameContext`] owns the
    /// encoder until it is submitted or dropped.
    pub fn begin_frame(&self, label: &'static str) -> FrameContext {
        self.inner.policy.begin_frame();
        let id = self.inner.next_frame.fetch_add(1, Ordering::Relaxed);
        let id = FrameId::new(id);
        let frame_slot = loop {
            if let Some(slot) = self.inner.policy.acquire_frame_slot(id.get()) {
                break Some(slot);
            }
            // A full policy slot means the oldest submitted frame has not
            // reached its completion callback yet. Polling makes progress
            // without creating an untracked frame that could overlap upload
            // or retirement state.
            let _ = self.inner.device.poll(wgpu::PollType::Poll);
            std::thread::yield_now();
        };
        self.make_frame(label, id, frame_slot)
    }

    /// Try to start a frame without waiting for an in-flight slot. Hosts that
    /// have their own scheduling loop can use this to turn a policy stall into
    /// a redraw request instead of blocking the caller.
    pub fn try_begin_frame(&self, label: &'static str) -> Option<FrameContext> {
        self.inner.policy.begin_frame();
        let id = FrameId::new(self.inner.next_frame.fetch_add(1, Ordering::Relaxed));
        let slot = self.inner.policy.acquire_frame_slot(id.get())?;
        Some(self.make_frame(label, id, Some(slot)))
    }

    fn make_frame(
        &self,
        label: &'static str,
        id: FrameId,
        frame_slot: Option<crate::FrameSlotId>,
    ) -> FrameContext {
        let encoder = self
            .inner
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        FrameContext::new(self.clone(), id, encoder, frame_slot)
    }

    /// What WGPU will allow for `format`, chosen the way WGPU validates it:
    /// the WebGPU table unless the device asks the adapter. `None` when the
    /// format needs a feature the device lacks.
    fn format_features(&self, format: wgpu::TextureFormat) -> Option<wgpu::TextureFormatFeatures> {
        let features = self.inner.device.features();
        if !features.contains(format.required_features()) {
            return None;
        }
        Some(if self.inner.webgpu_format_table {
            format.guaranteed_format_features(features)
        } else {
            self.inner.adapter.get_texture_format_features(format)
        })
    }

    pub(crate) fn check_device(&self, found: DeviceGeneration) -> Result<(), GpuError> {
        if found == self.inner.generation {
            Ok(())
        } else {
            Err(GpuError::DeviceMismatch {
                expected: self.inner.generation,
                found,
            })
        }
    }

    pub(crate) fn lock_submission(&self) -> RwLockReadGuard<'_, ()> {
        self.inner
            .submission
            .read()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn lock_reconfigure(&self) -> RwLockWriteGuard<'_, ()> {
        self.inner
            .submission
            .write()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn mark_lost(&self, report: GpuDeviceLost) {
        self.inner.loss.mark(report);
    }
}

impl GpuTextureFormat {
    pub(crate) const fn to_wgpu(self) -> wgpu::TextureFormat {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generations_are_unique_and_ordered() {
        let a = DeviceGeneration::next();
        let b = DeviceGeneration::next();
        assert_ne!(a, b);
        assert!(b > a);
        assert_eq!(format!("{a}"), format!("#{}", a.get()));
    }

    #[test]
    fn loss_is_sticky_and_keeps_the_first_report() {
        let loss = LossState::default();
        loss.mark(GpuDeviceLost {
            reason: GpuLossReason::Destroyed,
            message: "first".into(),
        });
        loss.mark(GpuDeviceLost {
            reason: GpuLossReason::Unknown,
            message: "second".into(),
        });
        assert!(loss.lost.load(Ordering::Acquire));
        assert_eq!(
            loss.report.lock().unwrap().as_ref().unwrap().message,
            "first"
        );
    }
}
