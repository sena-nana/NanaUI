use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::budget::Admission;
use crate::frame::{FrameContext, FrameLane};
use crate::policy::GpuDeviceState;
use crate::texture::{
    GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion, GpuTextureUsages,
};
use crate::{FrameId, GpuError};

static NEXT_DEVICE_GENERATION: AtomicU64 = AtomicU64::new(1);

/// How long one wait for a lane's oldest submission blocks before the device
/// is checked for loss again.
const BUDGET_POLL: std::time::Duration = std::time::Duration::from_millis(250);
/// How long a submitter waits for room before going ahead over budget. Past
/// Windows' 2 s GPU timeout a job that has not finished has either reset the
/// device (then waiting is pointless) or the driver let it run on; either way
/// the submitter is not held forever.
const BUDGET_WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

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
    non_uniform_indexing: bool,
    rasterizer_ordered_storage: bool,
    shader_f16: bool,
    subgroup: bool,
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
    /// Non-uniform dynamic indexing into sampled texture and storage buffer arrays.
    pub const fn non_uniform_indexing(self) -> bool {
        self.non_uniform_indexing
    }
    /// Fragment-stage rasterizer-ordered storage textures (DX12 ROV, Vulkan
    /// fragment shader interlock, Metal raster order groups).
    pub const fn rasterizer_ordered_storage(self) -> bool {
        self.rasterizer_ordered_storage
    }
    /// Native 16-bit floating-point (`f16`) shader arithmetic.
    pub const fn shader_f16(self) -> bool {
        self.shader_f16
    }
    /// Subgroup (wave / SIMD-group) shader operations.
    pub const fn subgroup(self) -> bool {
        self.subgroup
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
    NonUniformIndexing,
    RasterizerOrderedStorage,
    ShaderF16,
    Subgroup,
    Indirect,
    MultiDraw,
    Timestamps,
    ExternalTexture,
    TransientHint,
    /// DX12 shared textures plus a shared fence another D3D device opens by
    /// handle (`NativeExportPool`, feature `native-export`, Windows).
    NativeTextureExport,
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
            GpuCapability::NonUniformIndexing => {
                features.resource_arrays() && features.non_uniform_indexing()
            }
            GpuCapability::RasterizerOrderedStorage => features.rasterizer_ordered_storage(),
            GpuCapability::ShaderF16 => features.shader_f16(),
            GpuCapability::Subgroup => features.subgroup(),
            GpuCapability::Indirect => features.indirect(),
            GpuCapability::MultiDraw => features.multi_draw(),
            GpuCapability::Timestamps => features.timestamp_query(),
            GpuCapability::ExternalTexture => features.external_texture(),
            GpuCapability::TransientHint => features.transient_hint(),
            GpuCapability::NativeTextureExport => {
                cfg!(all(windows, feature = "native-export"))
                    && matches!(self.backend, GpuBackend::Dx12)
            }
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
                    GpuCapability::NonUniformIndexing => {
                        "non-uniform resource array indexing is unavailable"
                    }
                    GpuCapability::RasterizerOrderedStorage => {
                        "rasterizer-ordered storage textures are unavailable"
                    }
                    GpuCapability::ShaderF16 => "16-bit floating-point shader ops are unavailable",
                    GpuCapability::Subgroup => "subgroup shader operations are unavailable",
                    GpuCapability::Indirect => "indirect first-instance is unavailable",
                    GpuCapability::MultiDraw => "multi-draw indirect count is unavailable",
                    GpuCapability::Timestamps => "timestamp queries are unavailable",
                    GpuCapability::ExternalTexture => {
                        "external memory export/semaphore ownership interop is unavailable"
                    }
                    GpuCapability::TransientHint => "transient resource hints are unavailable",
                    GpuCapability::NativeTextureExport => {
                        if cfg!(all(windows, feature = "native-export")) {
                            "native texture export needs a DX12 device"
                        } else {
                            "native texture export needs Windows and the native-export feature"
                        }
                    }
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
    supports_view_formats: bool,
    supports_rgba16_float_render: bool,
    supports_rgba32_float_render: bool,
    features: GpuFeatureSet,
    limits: GpuLimits,
}

fn format_features_with(
    adapter: &wgpu::Adapter,
    device_features: wgpu::Features,
    webgpu_format_table: bool,
    format: wgpu::TextureFormat,
) -> Option<wgpu::TextureFormatFeatures> {
    if !device_features.contains(format.required_features()) {
        return None;
    }
    Some(if webgpu_format_table {
        format.guaranteed_format_features(device_features)
    } else {
        adapter.get_texture_format_features(format)
    })
}

pub(crate) fn format_features_satisfy(
    features: Option<wgpu::TextureFormatFeatures>,
    format: wgpu::TextureFormat,
    blendable: bool,
    filterable: bool,
) -> bool {
    if format.is_depth_stencil_format() {
        return false;
    }
    let Some(features) = features else {
        return false;
    };
    if !features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
    {
        return false;
    }
    if blendable
        && !features
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
    {
        return false;
    }
    if filterable
        && (!features
            .allowed_usages
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
            || !features
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE))
    {
        return false;
    }
    true
}

impl GpuCapabilities {
    fn read(adapter: &wgpu::Adapter, device: &wgpu::Device, webgpu_format_table: bool) -> Self {
        let info = adapter.get_info();
        let features = device.features();
        let downlevel = adapter.get_downlevel_capabilities();
        let supports_view_formats = downlevel.flags.contains(wgpu::DownlevelFlags::VIEW_FORMATS);
        let supports_rgba16_float_render = format_features_satisfy(
            format_features_with(
                adapter,
                features,
                webgpu_format_table,
                wgpu::TextureFormat::Rgba16Float,
            ),
            wgpu::TextureFormat::Rgba16Float,
            true,
            true,
        );
        let supports_rgba32_float_render = format_features_satisfy(
            format_features_with(
                adapter,
                features,
                webgpu_format_table,
                wgpu::TextureFormat::Rgba32Float,
            ),
            wgpu::TextureFormat::Rgba32Float,
            false,
            false,
        );
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
            supports_view_formats,
            supports_rgba16_float_render,
            supports_rgba32_float_render,
            features: GpuFeatureSet {
                dual_source_blending: features.contains(wgpu::Features::DUAL_SOURCE_BLENDING),
                pipeline_cache: features.contains(wgpu::Features::PIPELINE_CACHE),
                timestamp_query: features.contains(wgpu::Features::TIMESTAMP_QUERY),
                // The current logical ResourceArray maps to sampled texture
                // arrays. Storage resource arrays are a separate ABI shape
                // and must not advertise support for this declaration.
                resource_arrays: features.contains(wgpu::Features::TEXTURE_BINDING_ARRAY),
                non_uniform_indexing: features.contains(
                    wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
                ),
                rasterizer_ordered_storage: features
                    .contains(wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER),
                shader_f16: features.contains(wgpu::Features::SHADER_F16),
                subgroup: features.contains(wgpu::Features::SUBGROUP),
                indirect: features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
                multi_draw: features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
                external_texture: false,
                transient_hint: false,
            },
            limits: GpuLimits {
                max_bind_groups: device.limits().max_bind_groups,
                max_bindings_per_group: device.limits().max_bindings_per_bind_group,
                max_uniform_buffer_binding_size: device.limits().max_uniform_buffer_binding_size,
                max_storage_buffer_binding_size: device.limits().max_storage_buffer_binding_size,
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

    /// Whether the adapter supports alternate-format texture views (`DownlevelFlags::VIEW_FORMATS`).
    pub const fn supports_view_formats(&self) -> bool {
        self.supports_view_formats
    }

    /// Whether `RGBA16_FLOAT` supports render attachment, blending, and filtering on this device.
    pub const fn supports_rgba16_float_render(&self) -> bool {
        self.supports_rgba16_float_render
    }

    /// Whether `RGBA32_FLOAT` supports render attachment on this device.
    pub const fn supports_rgba32_float_render(&self) -> bool {
        self.supports_rgba32_float_render
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
    /// The first report of this loss has been recorded.
    recorded: AtomicBool,
    report: Mutex<Option<GpuDeviceLost>>,
}

impl LossState {
    /// Runs on whichever thread WGPU noticed the loss on (or an embedder's).
    /// The first report is recorded as a `gpu.device_lost` fault with a
    /// `device-lost` snapshot before this returns: a driver reset is often
    /// the last thing a process does, so it must not wait for the
    /// diagnostics worker's next poll.
    fn mark(&self, report: GpuDeviceLost) {
        if !self.recorded.swap(true, Ordering::AcqRel) {
            let reason = u64::from(report.reason == GpuLossReason::Destroyed);
            let message = format!("{:?}: {}", report.reason, report.message);
            let _ = nana_diagnostics::fault_with_snapshot(
                &nana_diagnostics::framework::gpu::DEVICE_LOST,
                &[nana_diagnostics::Field::new("reason", reason)],
                Some(&message),
                "device-lost",
            );
        }
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
    loss: Arc<LossState>,
    next_frame: AtomicU64,
    pub(crate) next_submission: AtomicU64,
    pub(crate) submission_order: Mutex<()>,
    policy: GpuDeviceState,
    /// Staging chunks for frame uploads.
    pub(crate) upload_ring: Arc<crate::upload::UploadRing>,
    /// Writes made outside a frame (and those of discarded frames), landed
    /// by the next submission on this device.
    pub(crate) pending_uploads: Mutex<crate::upload::UploadBatch>,
    /// What the device's pipeline cache is saved under; `None` without one.
    pub(crate) pipeline_cache_key: Option<crate::PipelineCacheKey>,
    /// The device's default pipeline cache (`wgpu::Device::set_default_pipeline_cache`
    /// holds it weakly, so it lives here).
    pub(crate) pipeline_cache: Mutex<Option<wgpu::PipelineCache>>,
    /// What the store last received or supplied, so a later save happens
    /// only when the cache grew past it.
    pub(crate) pipeline_cache_mark: Mutex<crate::pipeline_cache::PersistMark>,
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
        let webgpu_format_table = !device
            .features()
            .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
            && adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::WEBGPU_TEXTURE_FORMAT_SUPPORT);
        let capabilities = GpuCapabilities::read(&adapter, &device, webgpu_format_table);
        let adapter_info = adapter.get_info();
        let generation = DeviceGeneration::next();
        let policy = GpuDeviceState::new(generation);
        // Every pipeline on the device, the framework's or not, goes through
        // one cache the host can persist (see `pipeline_cache`).
        let pipeline_cache_key = device
            .features()
            .contains(wgpu::Features::PIPELINE_CACHE)
            .then(|| {
                crate::PipelineCacheKey::new(
                    capabilities.backend,
                    capabilities.vendor_id,
                    capabilities.device_id,
                )
            })
            .flatten();
        let pipeline_cache = pipeline_cache_key
            .as_ref()
            .and_then(|_| crate::pipeline_cache::create(&device, None));
        let pipeline_cache_key = pipeline_cache.as_ref().and(pipeline_cache_key);
        Self {
            inner: Arc::new(GpuInner {
                generation,
                adapter,
                device,
                queue,
                adapter_info,
                capabilities,
                webgpu_format_table,
                loss,
                next_frame: AtomicU64::new(1),
                next_submission: AtomicU64::new(1),
                submission_order: Mutex::new(()),
                policy,
                upload_ring: crate::upload::UploadRing::new(),
                pending_uploads: Mutex::new(crate::upload::UploadBatch::default()),
                pipeline_cache_key,
                pipeline_cache: Mutex::new(pipeline_cache),
                pipeline_cache_mark: Mutex::new(Default::default()),
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
        // Lands with the next submission on this device, ahead of its
        // commands, as a queue write would.
        self.inner
            .pending_uploads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_texture(
                texture.raw(),
                0,
                wgpu::Origin3d {
                    x: region.x,
                    y: region.y,
                    z: 0,
                },
                wgpu::Extent3d {
                    width: region.width,
                    height: region.height,
                    depth_or_array_layers: 1,
                },
                bytes,
                bytes_per_row,
                region.height,
                row,
            );
        self.inner.policy.record_upload(needed as u64);
        Ok(())
    }

    /// Submit writes made outside a frame now. Frames submit them on their
    /// own; this is for callers that submit raw command buffers through
    /// `wgpu-interop` and read what they uploaded.
    pub fn flush_uploads(&self) {
        // In submission order, as a frame takes them: see `FrameContext::submit`.
        let _order = self
            .inner
            .submission_order
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let batch = std::mem::take(
            &mut *self
                .inner
                .pending_uploads
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        let Some(uploads) = crate::upload::record(
            &self.inner.upload_ring,
            &self.inner.device,
            &self.inner.queue,
            &self.inner.policy,
            batch,
        ) else {
            return;
        };
        let index = self.inner.queue.submit([uploads.commands]);
        self.inner.upload_ring.submitted(uploads.chunk, index);
    }

    /// Start recording one host frame ([`FrameLane::Presentation`]). The
    /// returned [`FrameContext`] owns the encoder until it is submitted or
    /// dropped. Producers use [`Self::begin_producer_frame`] instead.
    ///
    /// With every frame slot in flight this blocks until the oldest submitted
    /// frame completes. If every slot is held by a recording that was never
    /// submitted, waiting could not end: the frame runs without a slot and
    /// the fault is reported.
    pub fn begin_frame(&self, label: &'static str) -> FrameContext {
        let id = FrameId::new(self.inner.next_frame.fetch_add(1, Ordering::Relaxed));
        let mut waited = false;
        let slot = loop {
            match self.inner.policy.take_frame_slot(id.get()) {
                Ok(slot) => break Some(slot),
                Err(crate::policy::SlotWait::Submitted { index, slot, token }) if !waited => {
                    waited = true;
                    self.inner.policy.record_slot_wait();
                    // The completion callback that normally frees the slot is
                    // registered after the submit and can sit behind a
                    // producer's later submission; a finished wait frees the
                    // slot itself, so producer work never makes this frame
                    // run without one. The callback's release is then a no-op
                    // (it matches the exact token).
                    if self
                        .poll_device(wgpu::PollType::Wait {
                            submission_index: Some(index),
                            timeout: Some(std::time::Duration::from_secs(1)),
                        })
                        .is_ok()
                    {
                        self.inner.policy.release_frame_slot_exact(slot, token);
                    }
                }
                Err(_) => {
                    self.inner.policy.record_slot_stall();
                    break None;
                }
            }
        };
        self.make_frame(
            label,
            id,
            FrameLane::Presentation,
            slot,
            crate::FrameAcquisition {
                frame_slot_waits: u64::from(waited),
                frame_slot_stalls: u64::from(slot.is_none()),
            },
        )
    }

    /// Start a frame without waiting for an in-flight slot. Hosts that have
    /// their own scheduling loop can use this to turn a full pipeline into a
    /// redraw request instead of blocking the caller.
    pub fn try_begin_frame(&self, label: &'static str) -> Option<FrameContext> {
        let id = FrameId::new(self.inner.next_frame.fetch_add(1, Ordering::Relaxed));
        let slot = self.inner.policy.take_frame_slot(id.get()).ok()?;
        Some(self.make_frame(
            label,
            id,
            FrameLane::Presentation,
            Some(slot),
            crate::FrameAcquisition::default(),
        ))
    }

    /// Start recording producer work ([`FrameLane::Producer`]): content the
    /// UI shows but does not present itself, such as video, model or custom
    /// node rendering and frame exchange copies.
    ///
    /// Producer frames take no frame slot, so they never hold back a window
    /// frame. Instead the producer lane is held to
    /// [`GpuDeviceState::producer_submission_budget`](crate::GpuDeviceState::producer_submission_budget)
    /// unfinished submissions: with the budget used up this blocks until the
    /// lane's oldest submission finishes, and [`FrameContext::submit`]
    /// enforces the same bound for producers that record concurrently.
    pub fn begin_producer_frame(&self, label: &'static str) -> FrameContext {
        self.admit(FrameLane::Producer, false);
        let id = FrameId::new(self.inner.next_frame.fetch_add(1, Ordering::Relaxed));
        self.make_frame(
            label,
            id,
            FrameLane::Producer,
            None,
            crate::FrameAcquisition::default(),
        )
    }

    /// [`Self::begin_producer_frame`] without blocking: `None` while the
    /// producer lane's budget is used up (after one non-blocking poll), so a
    /// producer can drop or coalesce work instead of waiting.
    pub fn try_begin_producer_frame(&self, label: &'static str) -> Option<FrameContext> {
        let lane = &self.inner.policy.lanes.producer;
        if lane.has_room(None).is_err() {
            let _ = self.poll_device(wgpu::PollType::Poll);
            lane.has_room(None).ok()?;
        }
        let id = FrameId::new(self.inner.next_frame.fetch_add(1, Ordering::Relaxed));
        Some(self.make_frame(
            label,
            id,
            FrameLane::Producer,
            None,
            crate::FrameAcquisition::default(),
        ))
    }

    /// Wait until `lane` has room for one more submission, reserving it when
    /// `reserve`. Never called under the submission-order lock: a lane only
    /// ever waits on its own oldest submission, so no other lane's submit is
    /// held up behind it.
    pub(crate) fn admit(&self, lane: FrameLane, reserve: bool) {
        let budget = self.inner.policy.lane(lane);
        let limit = self.inner.policy.submission_limit(lane);
        let started = std::time::Instant::now();
        let mut waited = false;
        loop {
            let admission = if reserve {
                budget.reserve(limit)
            } else {
                match budget.has_room(limit) {
                    Ok(()) => return,
                    Err(admission) => admission,
                }
            };
            let wait = match admission {
                Admission::Reserved => return,
                Admission::Wait(index, done) => Some((index, done)),
                Admission::Busy => None,
            };
            // A lost device finishes nothing; one stuck job is not waited on
            // forever.
            let overdue = started.elapsed() >= BUDGET_WAIT_LIMIT;
            if self.is_lost() || overdue {
                if overdue && !self.is_lost() {
                    nana_diagnostics::event!(
                        nana_diagnostics::framework::gpu::SUBMISSION_BUDGET_OVERRUN,
                        lane = lane.diagnostic_code()
                    );
                }
                if reserve {
                    budget.force_reserve();
                }
                return;
            }
            // Entries leave on completion callbacks, which can run a poll
            // late: check the oldest without blocking before counting a wait.
            if let Some((index, done)) = &wait
                && self
                    .poll_device(wgpu::PollType::Wait {
                        submission_index: Some(index.clone()),
                        timeout: Some(std::time::Duration::ZERO),
                    })
                    .is_ok()
            {
                done.store(true, Ordering::Release);
                continue;
            }
            if !waited {
                waited = true;
                budget.record_wait();
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::gpu::SUBMISSION_BUDGET_WAITS
                );
            }
            match wait {
                Some((index, done)) => {
                    // Runs completion callbacks too, which is what normally
                    // marks entries done; marking here covers a callback that
                    // was registered late.
                    if self
                        .poll_device(wgpu::PollType::Wait {
                            submission_index: Some(index),
                            timeout: Some(BUDGET_POLL),
                        })
                        .is_ok()
                    {
                        done.store(true, Ordering::Release);
                    }
                }
                // Only reservations about to reach the queue hold the room.
                None => std::thread::yield_now(),
            }
        }
    }

    /// Run `callback` after work submitted through this queue has completed.
    ///
    /// This is the non-blocking completion seam for retained output consumers.
    /// It deliberately does not poll or wait on the calling thread. WGPU's
    /// queue callback is ordered after the submission represented by
    /// `submission` (and may conservatively run after later submissions too),
    /// which is sufficient for resource retirement and frame-slot handoff.
    pub fn on_submission_complete(
        &self,
        submission: &crate::GpuSubmission,
        callback: impl FnOnce() + Send + 'static,
    ) -> Result<(), GpuError> {
        self.check_device(submission.generation())?;
        self.inner.queue.on_submitted_work_done(callback);
        Ok(())
    }

    /// Drive completion callbacks without waiting for GPU work. Hosts that
    /// own an event loop normally call this from their tick; retained output
    /// producers can expose it through their own `poll` seam. This never uses
    /// `PollType::Wait` and therefore cannot serialize a producer with a
    /// consumer just to sample a completed frame.
    pub fn poll(&self) {
        let _ = self.poll_device(wgpu::PollType::Poll);
    }

    /// Every framework poll and wait goes through here. A poll that finds the
    /// device lost (the fork's `PollError::DeviceLost`, e.g. a Vulkan
    /// `VK_ERROR_DEVICE_LOST`) marks this context lost, whether or not it owns
    /// the device-lost callback; WGPU has already run that callback and
    /// retired every submission, so completion callbacks waiting on them ran.
    pub(crate) fn poll_device(
        &self,
        poll_type: wgpu::PollType,
    ) -> Result<wgpu::PollStatus, wgpu::PollError> {
        let result = self.inner.device.poll(poll_type);
        if let Err(wgpu::PollError::DeviceLost) = result {
            self.mark_lost(GpuDeviceLost {
                reason: GpuLossReason::Unknown,
                message: "the device was lost while polling".into(),
            });
        }
        result
    }

    fn make_frame(
        &self,
        label: &'static str,
        id: FrameId,
        lane: FrameLane,
        frame_slot: Option<crate::FrameSlotId>,
        acquisition: crate::FrameAcquisition,
    ) -> FrameContext {
        let encoder = self
            .inner
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        FrameContext::new(self.clone(), id, lane, encoder, frame_slot, acquisition)
    }

    /// What WGPU will allow for `format`, chosen the way WGPU validates it:
    /// the WebGPU table unless the device asks the adapter. `None` when the
    /// format needs a feature the device lacks.
    pub(crate) fn format_features(
        &self,
        format: wgpu::TextureFormat,
    ) -> Option<wgpu::TextureFormatFeatures> {
        format_features_with(
            &self.inner.adapter,
            self.inner.device.features(),
            self.inner.webgpu_format_table,
            format,
        )
    }

    /// Whether `format` is a non-depth color format usable as a `RENDER_ATTACHMENT` on this context.
    pub fn supports_render_format(&self, format: GpuTextureFormat) -> bool {
        let raw = format.to_wgpu();
        format_features_satisfy(self.format_features(raw), raw, false, false)
    }

    /// Whether `format` supports rendering, blending, sampling, and filtering on this context.
    pub fn supports_blendable_format(&self, format: GpuTextureFormat) -> bool {
        let raw = format.to_wgpu();
        format_features_satisfy(self.format_features(raw), raw, true, true)
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

    #[test]
    fn format_features_satisfy_checks_render_blend_filter_and_depth_formats() {
        let full = Some(wgpu::TextureFormatFeatures {
            allowed_usages: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING,
            flags: wgpu::TextureFormatFeatureFlags::BLENDABLE
                | wgpu::TextureFormatFeatureFlags::FILTERABLE,
        });
        assert!(format_features_satisfy(
            full,
            wgpu::TextureFormat::Rgba16Float,
            true,
            true
        ));
        assert!(!format_features_satisfy(
            None,
            wgpu::TextureFormat::Rgba16Float,
            false,
            false
        ));
        assert!(!format_features_satisfy(
            full,
            wgpu::TextureFormat::Depth24Plus,
            false,
            false
        ));

        let unblendable = Some(wgpu::TextureFormatFeatures {
            allowed_usages: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING,
            flags: wgpu::TextureFormatFeatureFlags::FILTERABLE,
        });
        assert!(format_features_satisfy(
            unblendable,
            wgpu::TextureFormat::Rgba32Float,
            false,
            false
        ));
        assert!(!format_features_satisfy(
            unblendable,
            wgpu::TextureFormat::Rgba32Float,
            true,
            false
        ));

        let non_render = Some(wgpu::TextureFormatFeatures {
            allowed_usages: wgpu::TextureUsages::TEXTURE_BINDING,
            flags: wgpu::TextureFormatFeatureFlags::BLENDABLE
                | wgpu::TextureFormatFeatureFlags::FILTERABLE,
        });
        assert!(!format_features_satisfy(
            non_render,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
            false
        ));
    }

    #[test]
    fn advanced_capabilities_mirror_feature_set_and_report_fallbacks() {
        let make = |features: GpuFeatureSet| GpuCapabilities {
            backend: GpuBackend::Vulkan,
            device_type: GpuDeviceType::DiscreteGpu,
            adapter_name: Arc::from("test"),
            driver: Arc::from("test"),
            driver_info: Arc::from("test"),
            vendor_id: 0,
            device_id: 0,
            max_texture_dimension_2d: 8192,
            supports_view_formats: true,
            supports_rgba16_float_render: true,
            supports_rgba32_float_render: true,
            features,
            limits: GpuLimits {
                max_bind_groups: 4,
                max_bindings_per_group: 1000,
                max_uniform_buffer_binding_size: 65536,
                max_storage_buffer_binding_size: 134_217_728,
                min_uniform_buffer_offset_alignment: 256,
                min_storage_buffer_offset_alignment: 256,
                max_buffer_size: 268_435_456,
            },
        };

        let empty = make(GpuFeatureSet::default());
        for cap in [
            GpuCapability::RasterizerOrderedStorage,
            GpuCapability::ShaderF16,
            GpuCapability::Subgroup,
            GpuCapability::NonUniformIndexing,
        ] {
            let outcome = empty.capability(cap);
            assert!(!outcome.supported);
            assert!(outcome.fallback_reason.is_some());
        }

        // NonUniformIndexing requires both resource arrays and non-uniform indexing.
        let indexing_only = make(GpuFeatureSet {
            non_uniform_indexing: true,
            ..GpuFeatureSet::default()
        });
        assert!(!indexing_only.supports(GpuCapability::NonUniformIndexing));

        let full = make(GpuFeatureSet {
            resource_arrays: true,
            non_uniform_indexing: true,
            rasterizer_ordered_storage: true,
            shader_f16: true,
            subgroup: true,
            ..GpuFeatureSet::default()
        });
        assert!(full.features().rasterizer_ordered_storage());
        assert!(full.features().shader_f16());
        assert!(full.features().subgroup());
        assert!(full.features().non_uniform_indexing());
        for cap in [
            GpuCapability::ResourceArrays,
            GpuCapability::NonUniformIndexing,
            GpuCapability::RasterizerOrderedStorage,
            GpuCapability::ShaderF16,
            GpuCapability::Subgroup,
        ] {
            let outcome = full.capability(cap);
            assert!(outcome.supported);
            assert_eq!(outcome.fallback_reason, None);
        }
    }
}
