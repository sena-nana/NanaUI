//! Bounded latest-frame exchange on a host's existing GPU.
//!
//! A producer thread copies finished frames into a small pool of textures on
//! the shared [`GpuContext`], or renders directly into a pool slot acquired via
//! [`FrameExchange::try_begin_write`], and a consumer — usually a UI window —
//! samples the newest completed frame. Neither side waits for the other or for
//! GPU completion: a full pool drops producer work, and a lease frees its slot
//! only after the consumer submission that sampled it has completed.
//!
//! `copy_from*` performs a GPU-side `copy_texture_to_texture` (never CPU
//! readback), while `try_begin_write` / `commit_write*` lets the producer render
//! into the pooled texture directly on the same submission without a copy.

#![recursion_limit = "256"]

use std::{
    num::NonZeroU8,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use nana_gpu::{__framework, GpuTextureDescriptor, GpuTextureUsages, TransientResourceKey};
/// The contract types an exchange is built from, so a producer crate needs no
/// other GPU dependency.
pub use nana_gpu::{DeviceGeneration, GpuContext, GpuTexture, GpuTextureFormat};

/// The RGB primaries declared for a frame's texels.
///
/// `Unknown` is used by the backwards-compatible [`FrameExchange::copy_from`]
/// entry point.  Consumers must not treat it as sRGB; a producer that knows
/// the encoding should use [`FrameExchange::copy_from_with_metadata`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrameColorPrimaries {
    /// The producer did not declare the primaries.
    #[default]
    Unknown,
    /// Rec.709 / sRGB primaries.
    Srgb,
    /// Display-P3 primaries.
    DisplayP3,
    /// Rec.2020 primaries (used by BT.2100 PQ and HLG).
    Bt2020,
}

/// The transfer function applied to the RGB channels in a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrameTransferFunction {
    /// The producer did not declare a transfer function.
    #[default]
    Unknown,
    /// Linear-light values, including the linear scRGB working convention.
    Linear,
    /// The sRGB OETF.
    Srgb,
    /// BT.2100 perceptual quantizer.
    Pq,
    /// BT.2100 hybrid log-gamma.
    Hlg,
}

/// The sample range of the encoded RGB channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrameColorRange {
    /// The producer did not declare a range. Consumers must not infer one
    /// from the texture format alone.
    #[default]
    Unknown,
    /// Full-range RGB (for example, 0..1 in normalized texture samples).
    Full,
    /// Studio/video range (for example, nominal 16..235 in 8-bit samples).
    Limited,
}

/// Alpha interpretation for the copied pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrameAlphaMode {
    /// The producer did not declare alpha semantics.
    #[default]
    Unknown,
    /// Alpha is always one; RGB is opaque.
    Opaque,
    /// RGB is stored independently of alpha.
    Straight,
    /// RGB is already multiplied by alpha.
    Premultiplied,
}

/// Absolute luminance convention for linear RGB values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrameLinearUnit {
    /// The producer has not declared what a linear value of one means.
    #[default]
    Unknown,
    /// Linear scRGB: RGB value one represents 80 cd/m² (nit).
    Scrgb80Nits,
}

/// Source color metadata carried with one exchanged frame.
///
/// [`FrameExchange`] copies texels byte-for-byte and does not perform a color
/// conversion. This value is therefore a declaration of the source pixels,
/// not a promise about the destination surface. A consumer must inspect it
/// before sampling or exporting a frame and perform an explicit conversion
/// when its destination contract differs. The legacy [`FrameExchange::copy_from`]
/// method attaches [`Self::unknown`], so it never silently labels an unknown
/// source as sRGB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FrameColorMetadata {
    primaries: FrameColorPrimaries,
    transfer: FrameTransferFunction,
    range: FrameColorRange,
    alpha: FrameAlphaMode,
    linear_unit: FrameLinearUnit,
}

impl FrameColorMetadata {
    /// Construct an explicit source declaration.
    pub const fn new(
        primaries: FrameColorPrimaries,
        transfer: FrameTransferFunction,
        range: FrameColorRange,
        alpha: FrameAlphaMode,
    ) -> Self {
        Self {
            primaries,
            transfer,
            range,
            alpha,
            linear_unit: FrameLinearUnit::Unknown,
        }
    }

    /// Metadata used by the backwards-compatible copy methods.
    pub const fn unknown() -> Self {
        Self::new(
            FrameColorPrimaries::Unknown,
            FrameTransferFunction::Unknown,
            FrameColorRange::Unknown,
            FrameAlphaMode::Unknown,
        )
    }

    /// Common full-range sRGB declaration.
    pub const fn srgb(alpha: FrameAlphaMode) -> Self {
        Self::new(
            FrameColorPrimaries::Srgb,
            FrameTransferFunction::Srgb,
            FrameColorRange::Full,
            alpha,
        )
    }

    /// Relative linear Rec.709 declaration with an unspecified luminance unit.
    /// A consumer cannot automatically treat it as linear scRGB.
    pub const fn linear_srgb(alpha: FrameAlphaMode) -> Self {
        Self::new(
            FrameColorPrimaries::Srgb,
            FrameTransferFunction::Linear,
            FrameColorRange::Full,
            alpha,
        )
    }

    /// Linear scRGB declaration, with RGB value one fixed at 80 nit.
    pub const fn linear_scrgb(alpha: FrameAlphaMode) -> Self {
        Self {
            linear_unit: FrameLinearUnit::Scrgb80Nits,
            ..Self::linear_srgb(alpha)
        }
    }

    /// Full-range BT.2100 PQ declaration.
    pub const fn bt2020_pq(alpha: FrameAlphaMode) -> Self {
        Self::new(
            FrameColorPrimaries::Bt2020,
            FrameTransferFunction::Pq,
            FrameColorRange::Full,
            alpha,
        )
    }

    /// Full-range BT.2100 HLG declaration.
    pub const fn bt2020_hlg(alpha: FrameAlphaMode) -> Self {
        Self::new(
            FrameColorPrimaries::Bt2020,
            FrameTransferFunction::Hlg,
            FrameColorRange::Full,
            alpha,
        )
    }

    pub const fn primaries(self) -> FrameColorPrimaries {
        self.primaries
    }

    pub const fn transfer(self) -> FrameTransferFunction {
        self.transfer
    }

    pub const fn range(self) -> FrameColorRange {
        self.range
    }

    pub const fn alpha(self) -> FrameAlphaMode {
        self.alpha
    }

    pub const fn linear_unit(self) -> FrameLinearUnit {
        self.linear_unit
    }

    /// Whether primaries, transfer, range, and alpha are all declared. Linear
    /// luminance units are checked separately with [`Self::linear_unit`].
    pub const fn is_explicit(self) -> bool {
        !matches!(self.primaries, FrameColorPrimaries::Unknown)
            && !matches!(self.transfer, FrameTransferFunction::Unknown)
            && !matches!(self.range, FrameColorRange::Unknown)
            && !matches!(self.alpha, FrameAlphaMode::Unknown)
    }

    pub const fn is_hdr(self) -> bool {
        matches!(
            self.transfer,
            FrameTransferFunction::Pq | FrameTransferFunction::Hlg
        )
    }
}

/// One in-flight copy, the frame a consumer samples, and the frame it retired
/// but has not presented yet. Add two slots for every additional consumer.
pub const DEFAULT_CAPACITY: NonZeroU8 = NonZeroU8::new(3).unwrap();

static NEXT_EXCHANGE: AtomicU64 = AtomicU64::new(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// Identity of one copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameToken<E = u64> {
    exchange: u64,
    device_generation: DeviceGeneration,
    epoch: E,
    slot: u8,
    sequence: u64,
}

impl<E: Copy> FrameToken<E> {
    /// Stable id of the exchange that produced this frame. Useful for
    /// correlating low-volume diagnostics across a model switch.
    pub fn exchange_id(&self) -> u64 {
        self.exchange
    }

    /// Application epoch the frame was produced under.
    pub fn epoch(&self) -> E {
        self.epoch
    }

    /// Increases with every copy of one exchange.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn device_generation(&self) -> DeviceGeneration {
        self.device_generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyOutcome {
    /// The copy is queued and becomes visible on a later [`FrameExchange::poll`].
    Submitted,
    /// Every slot is copying or leased, or the device's producer submission
    /// budget is used up. The frame is dropped; nothing waits.
    PoolFull,
    /// The source has zero width or height.
    EmptySource,
    /// The source cannot be copied into a sampled 2D texture: it lacks
    /// `COPY_SRC`, is multisampled, is not a single-layer 2D texture, or has a
    /// depth/stencil format, or its format cannot back a sampled copy on this
    /// device.
    IncompatibleSource,
    /// The source lives on another device than the exchange.
    DeviceMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteAcquireError {
    /// Requested extent has zero width or height.
    EmptyExtent,
    /// Requested format cannot back a renderable, sampleable 2D slot texture
    /// (for example depth/stencil, compressed, or unsupported usage).
    IncompatibleFormat,
    /// Every slot in the pool is currently held by an uncommitted write, an
    /// in-flight frame, or a consumer lease.
    PoolFull,
}

/// A free pool slot reserved for direct producer rendering.
///
/// The producer renders directly into [`Self::texture`] (or [`Self::wgpu_texture`]
/// / [`Self::wgpu_view`] with `wgpu-interop`), submits that work to the shared
/// device queue, and hands the slot back to [`FrameExchange::commit_write`] (or
/// [`FrameExchange::commit_write_with_payload`]). Dropping an uncommitted slot
/// immediately releases it back to the pool.
pub struct WriteSlot {
    exchange: u64,
    slot_index: usize,
    sequence: u64,
    texture: GpuTexture,
    ownership: Arc<SlotOwnership>,
    committed: bool,
}

impl WriteSlot {
    pub fn texture(&self) -> &GpuTexture {
        &self.texture
    }

    pub fn size(&self) -> (u32, u32) {
        self.texture.size()
    }

    pub fn format(&self) -> GpuTextureFormat {
        self.texture.format()
    }

    pub fn slot(&self) -> u8 {
        self.ownership.slot
    }

    #[cfg(feature = "wgpu-interop")]
    pub fn wgpu_texture(&self) -> &wgpu::Texture {
        __framework::texture(&self.texture)
    }

    #[cfg(feature = "wgpu-interop")]
    pub fn wgpu_view(&self) -> &wgpu::TextureView {
        __framework::texture_view(&self.texture)
    }
}

impl Drop for WriteSlot {
    fn drop(&mut self) {
        if !self.committed {
            self.ownership.release(self.sequence);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameExchangeStats {
    pub submitted: u64,
    pub published: u64,
    /// Completed copies overtaken by a newer completed copy.
    pub superseded: u64,
    pub pool_full: u64,
    /// Completed copies whose epoch was no longer current.
    pub stale_epoch: u64,
    pub occupied: u8,
    pub occupied_high_water: u8,
}

struct SlotOwnership {
    slot: u8,
    /// Sequence holding the slot; 0 while free.
    sequence: AtomicU64,
    /// Sequence handed back by a dropped lease; 0 when none is waiting.
    returned: AtomicU64,
}

impl SlotOwnership {
    fn new(slot: u8) -> Self {
        Self {
            slot,
            sequence: AtomicU64::new(0),
            returned: AtomicU64::new(0),
        }
    }

    fn is_free(&self) -> bool {
        self.sequence.load(Ordering::Acquire) == 0
    }

    fn reserve(&self, sequence: u64) -> bool {
        sequence != 0
            && self
                .sequence
                .compare_exchange(0, sequence, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }

    /// Sequences are unique within an exchange, so a delayed or duplicate
    /// completion cannot free a slot that has been reserved again.
    fn release(&self, sequence: u64) {
        let _ = self
            .sequence
            .compare_exchange(sequence, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}

/// Keeps the device and pool textures alive while any lease remains. The last
/// owner hands destruction to a background thread, because the final device
/// drop may wait for the GPU to go idle.
struct Retirement {
    gpu: Option<GpuContext>,
    textures: Mutex<Vec<GpuTexture>>,
}

impl Drop for Retirement {
    fn drop(&mut self) {
        let Some(gpu) = self.gpu.take() else {
            return;
        };
        let textures = std::mem::take(
            self.textures
                .get_mut()
                .unwrap_or_else(|error| error.into_inner()),
        );
        // If the thread cannot be spawned the closure is dropped here, which
        // destroys the resources on this thread instead.
        #[cfg(test)]
        let retire_track = tests::enter_retirement();
        let _ = std::thread::Builder::new()
            .name("nana-frame-exchange-retire".into())
            .spawn(move || {
                #[cfg(test)]
                let _retire_track = retire_track;
                let _ = __framework::poll(
                    &gpu,
                    wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(std::time::Duration::from_secs(15)),
                    },
                );
                drop(textures);
                drop(gpu);
            });
    }
}

/// A completed copy the consumer may sample until it drops the lease.
pub struct FrameLease<E = u64, M = ()> {
    token: FrameToken<E>,
    texture: GpuTexture,
    color_metadata: FrameColorMetadata,
    payload: M,
    ownership: Arc<SlotOwnership>,
    _retirement: Arc<Retirement>,
}

impl<E: Copy, M> FrameLease<E, M> {
    pub fn token(&self) -> FrameToken<E> {
        self.token
    }

    /// The copied frame. Sample it only while this lease is held.
    pub fn texture(&self) -> &GpuTexture {
        &self.texture
    }

    pub fn size(&self) -> (u32, u32) {
        self.texture.size()
    }

    pub fn format(&self) -> GpuTextureFormat {
        self.texture.format()
    }

    /// Source color metadata supplied by the producer for this copy.
    ///
    /// The exchange never converts pixels. A consumer must use this value to
    /// choose the correct sampling/export path instead of assuming the
    /// texture is sRGB.
    pub fn color_metadata(&self) -> FrameColorMetadata {
        self.color_metadata
    }

    /// Immutable application data copied with this frame, independent of its epoch.
    pub fn payload(&self) -> &M {
        &self.payload
    }
}

impl<E, M> Drop for FrameLease<E, M> {
    fn drop(&mut self) {
        // Only record the return. Registering the queue callback that frees
        // the slot can enter the backend, which belongs to the producer lane.
        self.ownership
            .returned
            .store(self.token.sequence, Ordering::Release);
    }
}

struct Shared<E, M> {
    exchange: u64,
    device_generation: DeviceGeneration,
    latest: Mutex<Option<Arc<FrameLease<E, M>>>>,
    epoch: Mutex<E>,
    notified: AtomicBool,
    active: AtomicBool,
}

/// Consumer side. Never waits for the producer or for GPU completion.
pub struct FrameInbox<E = u64, M = ()> {
    shared: Arc<Shared<E, M>>,
}

impl<E, M> Clone for FrameInbox<E, M> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<E: Copy + Eq, M> FrameInbox<E, M> {
    pub fn device_generation(&self) -> DeviceGeneration {
        self.shared.device_generation
    }

    /// Acknowledge the pending wake and acquire the latest frame. Repeated
    /// calls can return the same frame; compare tokens.
    pub fn try_take_latest(&self) -> Option<Arc<FrameLease<E, M>>> {
        // Acknowledge before loading: a racing publication either appears in
        // this load or raises a new wake.
        self.shared.notified.store(false, Ordering::Release);
        self.latest()
    }

    /// Inspect the latest frame without acknowledging the wake.
    pub fn latest(&self) -> Option<Arc<FrameLease<E, M>>> {
        if !self.shared.active.load(Ordering::Acquire) {
            return None;
        }
        let frame = lock(&self.shared.latest).clone()?;
        self.is_current(&frame.token).then_some(frame)
    }

    /// Whether a frame still belongs to this live exchange and its epoch.
    pub fn is_current(&self, token: &FrameToken<E>) -> bool {
        token.exchange == self.shared.exchange
            && self.shared.active.load(Ordering::Acquire)
            && token.epoch == *lock(&self.shared.epoch)
    }

    /// The exchange is still the one that produced `token`, its epoch has
    /// moved, and it has not published the next frame. The picture already on
    /// screen stays: unbinding here presents a transparent window.
    pub fn awaits_replacement(&self, token: &FrameToken<E>) -> bool {
        token.exchange == self.shared.exchange
            && token.device_generation == self.shared.device_generation
            && self.shared.active.load(Ordering::Acquire)
            && token.epoch != *lock(&self.shared.epoch)
            && lock(&self.shared.latest).is_none()
    }
}

struct PendingCopy<E, M> {
    token: FrameToken<E>,
    color_metadata: FrameColorMetadata,
    payload: M,
    completed: Arc<AtomicBool>,
}

struct Slot<E, M> {
    ownership: Arc<SlotOwnership>,
    texture: Option<GpuTexture>,
    pending: Option<PendingCopy<E, M>>,
}

/// Producer side, owned by one thread.
pub struct FrameExchange<E = u64, M = ()> {
    shared: Arc<Shared<E, M>>,
    gpu: GpuContext,
    retirement: Arc<Retirement>,
    slots: Box<[Slot<E, M>]>,
    epoch: E,
    notify: Arc<dyn Fn() + Send + Sync>,
    sequence: u64,
    last_published: u64,
    stats: FrameExchangeStats,
}

impl<E: Copy + Eq + Send + Sync + 'static> FrameExchange<E> {
    /// Construct an exchange without application payloads.
    pub fn new(
        gpu: &GpuContext,
        capacity: NonZeroU8,
        epoch: E,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self::new_with_payload(gpu, capacity, epoch, notify)
    }
}

impl<E: Copy + Eq + Send + Sync + 'static, M: Send + Sync + 'static> FrameExchange<E, M> {
    /// `notify` runs on the producer thread once per acknowledged wake; it
    /// should only schedule the consumer, never wait for it.
    pub fn new_with_payload(
        gpu: &GpuContext,
        capacity: NonZeroU8,
        epoch: E,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                exchange: NEXT_EXCHANGE.fetch_add(1, Ordering::Relaxed),
                device_generation: gpu.generation(),
                latest: Mutex::new(None),
                epoch: Mutex::new(epoch),
                notified: AtomicBool::new(false),
                active: AtomicBool::new(true),
            }),
            retirement: Arc::new(Retirement {
                gpu: Some(gpu.clone()),
                textures: Mutex::new(Vec::with_capacity(usize::from(capacity.get()))),
            }),
            gpu: gpu.clone(),
            slots: (0..capacity.get())
                .map(|slot| Slot {
                    ownership: Arc::new(SlotOwnership::new(slot)),
                    texture: None,
                    pending: None,
                })
                .collect(),
            epoch,
            notify,
            sequence: 0,
            last_published: 0,
            stats: FrameExchangeStats::default(),
        }
    }

    pub fn inbox(&self) -> FrameInbox<E, M> {
        FrameInbox {
            shared: Arc::clone(&self.shared),
        }
    }

    /// A new epoch hides the latest frame immediately; copies started under
    /// an older epoch are never published.
    pub fn set_epoch(&mut self, epoch: E) {
        if self.epoch == epoch {
            return;
        }
        self.epoch = epoch;
        *lock(&self.shared.epoch) = epoch;
        let had_frame = lock(&self.shared.latest).take().is_some();
        if had_frame {
            self.wake();
        }
    }

    /// Copy `source` into a free slot on the shared device. Returns without
    /// waiting when the pool is full.
    ///
    /// Safe from any thread, including while a window reconfigures its surface.
    pub fn copy_from(&mut self, source: &GpuTexture, epoch: E) -> CopyOutcome
    where
        M: Default,
    {
        self.copy_from_with_metadata(source, epoch, FrameColorMetadata::unknown())
    }

    /// Copy `source` while carrying an explicit source color declaration.
    ///
    /// The declaration is attached to the resulting [`FrameLease`]; texels
    /// are still copied byte-for-byte and no conversion is performed.
    pub fn copy_from_with_metadata(
        &mut self,
        source: &GpuTexture,
        epoch: E,
        color_metadata: FrameColorMetadata,
    ) -> CopyOutcome
    where
        M: Default,
    {
        self.copy_from_with_payload(source, epoch, color_metadata, M::default())
    }

    /// Copy texels and immutable application data as one completed publication.
    pub fn copy_from_with_payload(
        &mut self,
        source: &GpuTexture,
        epoch: E,
        color_metadata: FrameColorMetadata,
        payload: M,
    ) -> CopyOutcome {
        if source.generation() != self.gpu.generation() {
            self.set_epoch(epoch);
            return CopyOutcome::DeviceMismatch;
        }
        self.copy_raw(__framework::texture(source), epoch, color_metadata, payload)
    }

    /// [`Self::copy_from`] for a raw WGPU texture created on the exchange's
    /// device.
    #[cfg(feature = "wgpu-interop")]
    pub fn copy_from_wgpu(&mut self, source: &wgpu::Texture, epoch: E) -> CopyOutcome
    where
        M: Default,
    {
        self.copy_from_wgpu_with_metadata(source, epoch, FrameColorMetadata::unknown())
    }

    /// [`Self::copy_from_with_metadata`] for a raw WGPU texture created on the
    /// exchange's device.
    #[cfg(feature = "wgpu-interop")]
    pub fn copy_from_wgpu_with_metadata(
        &mut self,
        source: &wgpu::Texture,
        epoch: E,
        color_metadata: FrameColorMetadata,
    ) -> CopyOutcome
    where
        M: Default,
    {
        self.copy_from_wgpu_with_payload(source, epoch, color_metadata, M::default())
    }

    /// Raw WGPU variant of [`Self::copy_from_with_payload`].
    #[cfg(feature = "wgpu-interop")]
    pub fn copy_from_wgpu_with_payload(
        &mut self,
        source: &wgpu::Texture,
        epoch: E,
        color_metadata: FrameColorMetadata,
        payload: M,
    ) -> CopyOutcome {
        self.copy_raw(source, epoch, color_metadata, payload)
    }

    fn copy_raw(
        &mut self,
        source: &wgpu::Texture,
        epoch: E,
        color_metadata: FrameColorMetadata,
        payload: M,
    ) -> CopyOutcome {
        self.set_epoch(epoch);
        let size = source.size();
        if size.width == 0 || size.height == 0 {
            return CopyOutcome::EmptySource;
        }
        if !source.usage().contains(wgpu::TextureUsages::COPY_SRC)
            || source.sample_count() != 1
            || source.dimension() != wgpu::TextureDimension::D2
            || size.depth_or_array_layers != 1
            || source.format().is_depth_stencil_format()
        {
            return CopyOutcome::IncompatibleSource;
        }
        let format = __framework::format_from_wgpu(source.format());
        let reserved = self.sequence.checked_add(1).and_then(|sequence| {
            let index = self
                .slots
                .iter()
                .position(|slot| slot.ownership.reserve(sequence))?;
            Some((sequence, index))
        });
        let Some((sequence, index)) = reserved else {
            self.stats.pool_full += 1;
            return CopyOutcome::PoolFull;
        };
        self.sequence = sequence;
        let slot = &mut self.slots[index];
        let Some(mut frame) = self
            .gpu
            .try_begin_producer_frame("NanaUI frame exchange copy")
        else {
            slot.ownership.release(sequence);
            self.stats.pool_full += 1;
            return CopyOutcome::PoolFull;
        };
        // A reserved slot was free before this copy, so its previous texture
        // is no longer sampled by a consumer. Return it to the shared policy
        // and reacquire by the complete resource key. This makes resize and
        // steady-size exchange copies use the same bounded pool as renderers.
        let usage = GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST;
        if let Some(old) = slot.texture.take() {
            let old_key = TransientResourceKey::new(
                self.gpu.generation(),
                old.format(),
                old.usage().bits(),
                old.size().0,
                old.size().1,
                1,
            );
            self.gpu.policy().release_transient_texture(old_key, old);
        }
        let key = TransientResourceKey::new(
            self.gpu.generation(),
            format,
            usage.bits(),
            size.width,
            size.height,
            1,
        );
        let gpu = self.gpu.clone();
        match self.gpu.policy().acquire_transient_texture(key, || {
            gpu.create_texture(&GpuTextureDescriptor {
                label: Some("NanaUI frame exchange slot"),
                width: size.width,
                height: size.height,
                format,
                usage,
            })
        }) {
            Ok(texture) => slot.texture = Some(texture),
            Err(_) => {
                slot.ownership.release(sequence);
                return CopyOutcome::IncompatibleSource;
            }
        }
        let texture = __framework::texture(slot.texture.as_ref().expect("frame exchange slot"));
        let queue = __framework::queue(&self.gpu);
        __framework::encoder(&mut frame).copy_texture_to_texture(
            source.as_image_copy(),
            texture.as_image_copy(),
            size,
        );
        let _submission = frame.submit();
        let completed = Arc::new(AtomicBool::new(false));
        let completion = Arc::clone(&completed);
        queue.on_submitted_work_done(move || completion.store(true, Ordering::Release));
        slot.pending = Some(PendingCopy {
            token: FrameToken {
                exchange: self.shared.exchange,
                device_generation: self.shared.device_generation,
                epoch,
                slot: slot.ownership.slot,
                sequence,
            },
            color_metadata,
            payload,
            completed,
        });
        self.stats.submitted += 1;
        self.stats.occupied_high_water = self.stats.occupied_high_water.max(self.occupied());
        CopyOutcome::Submitted
    }

    /// Reserve a free slot texture sized `(width, height)` in `format` with
    /// `RENDER_TARGET | SAMPLED | COPY_SRC | COPY_DST` usages so a producer can
    /// render directly into it without an extra `copy_texture_to_texture`.
    ///
    /// Returns [`WriteAcquireError::PoolFull`] without waiting when every slot is
    /// in use. Dropping the returned [`WriteSlot`] without committing releases
    /// the slot immediately.
    pub fn try_begin_write(
        &mut self,
        width: u32,
        height: u32,
        format: GpuTextureFormat,
    ) -> Result<WriteSlot, WriteAcquireError> {
        if width == 0 || height == 0 {
            return Err(WriteAcquireError::EmptyExtent);
        }
        let wgpu_format = __framework::format_to_wgpu(format);
        if wgpu_format.is_depth_stencil_format() || wgpu_format.is_compressed() {
            return Err(WriteAcquireError::IncompatibleFormat);
        }
        let reserved = self.sequence.checked_add(1).and_then(|sequence| {
            let index = self
                .slots
                .iter()
                .position(|slot| slot.ownership.reserve(sequence))?;
            Some((sequence, index))
        });
        let Some((sequence, index)) = reserved else {
            self.stats.pool_full += 1;
            return Err(WriteAcquireError::PoolFull);
        };
        self.sequence = sequence;
        let slot = &mut self.slots[index];
        let usage = GpuTextureUsages::RENDER_TARGET
            | GpuTextureUsages::SAMPLED
            | GpuTextureUsages::COPY_SRC
            | GpuTextureUsages::COPY_DST;
        let reuse_existing = slot.texture.as_ref().is_some_and(|existing| {
            existing.size() == (width, height)
                && existing.format() == format
                && existing.usage() == usage
        });
        if !reuse_existing {
            if let Some(old) = slot.texture.take() {
                let old_key = TransientResourceKey::new(
                    self.gpu.generation(),
                    old.format(),
                    old.usage().bits(),
                    old.size().0,
                    old.size().1,
                    1,
                );
                self.gpu.policy().release_transient_texture(old_key, old);
            }
            let key = TransientResourceKey::new(
                self.gpu.generation(),
                format,
                usage.bits(),
                width,
                height,
                1,
            );
            let gpu = self.gpu.clone();
            match self.gpu.policy().acquire_transient_texture(key, || {
                gpu.create_texture(&GpuTextureDescriptor {
                    label: Some("NanaUI frame exchange write slot"),
                    width,
                    height,
                    format,
                    usage,
                })
            }) {
                Ok(texture) => slot.texture = Some(texture),
                Err(_) => {
                    slot.ownership.release(sequence);
                    return Err(WriteAcquireError::IncompatibleFormat);
                }
            }
        }
        let texture = slot
            .texture
            .as_ref()
            .expect("write slot texture must be allocated")
            .clone();
        let ownership = Arc::clone(&slot.ownership);
        self.stats.occupied_high_water = self.stats.occupied_high_water.max(self.occupied());
        Ok(WriteSlot {
            exchange: self.shared.exchange,
            slot_index: index,
            sequence,
            texture,
            ownership,
            committed: false,
        })
    }

    /// Commit a [`WriteSlot`] whose rendering work has been submitted on the
    /// shared queue.
    pub fn commit_write(&mut self, slot: WriteSlot, epoch: E) -> CopyOutcome
    where
        M: Default,
    {
        self.commit_write_with_metadata(slot, epoch, FrameColorMetadata::unknown())
    }

    /// Commit a [`WriteSlot`] together with explicit color metadata.
    pub fn commit_write_with_metadata(
        &mut self,
        slot: WriteSlot,
        epoch: E,
        color_metadata: FrameColorMetadata,
    ) -> CopyOutcome
    where
        M: Default,
    {
        self.commit_write_with_payload(slot, epoch, color_metadata, M::default())
    }

    /// Commit a [`WriteSlot`] together with color metadata and an immutable
    /// application payload.
    ///
    /// The caller must have already submitted any GPU commands writing into
    /// `slot` on [`FrameExchange`]'s device queue; completion is tracked via
    /// the next queue fence before [`FrameExchange::poll`] publishes the lease.
    pub fn commit_write_with_payload(
        &mut self,
        mut write_slot: WriteSlot,
        epoch: E,
        color_metadata: FrameColorMetadata,
        payload: M,
    ) -> CopyOutcome {
        self.set_epoch(epoch);
        if write_slot.exchange != self.shared.exchange {
            return CopyOutcome::DeviceMismatch;
        }
        write_slot.committed = true;
        let completed = Arc::new(AtomicBool::new(false));
        let completion = Arc::clone(&completed);
        __framework::queue(&self.gpu)
            .on_submitted_work_done(move || completion.store(true, Ordering::Release));
        let slot = &mut self.slots[write_slot.slot_index];
        slot.pending = Some(PendingCopy {
            token: FrameToken {
                exchange: self.shared.exchange,
                device_generation: self.shared.device_generation,
                epoch,
                slot: write_slot.ownership.slot,
                sequence: write_slot.sequence,
            },
            color_metadata,
            payload,
            completed,
        });
        self.stats.submitted += 1;
        self.stats.occupied_high_water = self.stats.occupied_high_water.max(self.occupied());
        CopyOutcome::Submitted
    }

    /// Run on every producer tick, including ticks that copy nothing: it frees
    /// returned slots and publishes the newest completed copy. Returns whether
    /// a frame was published.
    pub fn poll(&mut self) -> bool {
        for slot in self.slots.iter() {
            let sequence = slot.ownership.returned.swap(0, Ordering::AcqRel);
            if sequence == 0 {
                continue;
            }
            let ownership = Arc::clone(&slot.ownership);
            // The consumer may have sampled this texture in a submission that
            // is still queued; the slot is reusable once that work completes.
            __framework::queue(&self.gpu)
                .on_submitted_work_done(move || ownership.release(sequence));
        }
        self.gpu.poll();
        let epoch = self.epoch;
        let completed = |pending: &PendingCopy<E, M>| pending.completed.load(Ordering::Acquire);
        let newest = self
            .slots
            .iter()
            .filter_map(|slot| slot.pending.as_ref())
            .filter(|pending| pending.token.epoch == epoch && completed(pending))
            .map(|pending| pending.token.sequence)
            .max();
        // Completion is set by whichever thread's poll runs the callback, so a
        // copy can complete after `newest` was chosen. It is not superseded:
        // it stays pending for the next poll, which publishes it.
        let settled = |pending: &PendingCopy<E, M>| {
            completed(pending)
                && (pending.token.epoch != epoch
                    || newest.is_some_and(|newest| pending.token.sequence <= newest))
        };
        let mut published = None;
        for slot in self.slots.iter_mut() {
            if !slot.pending.as_ref().is_some_and(settled) {
                continue;
            }
            let pending = slot.pending.take().expect("completed frame copy");
            let sequence = pending.token.sequence;
            // No consumer has seen an unpublished copy, so its slot is free now.
            if pending.token.epoch != epoch {
                self.stats.stale_epoch += 1;
                slot.ownership.release(sequence);
                continue;
            }
            if Some(sequence) != newest || sequence <= self.last_published {
                self.stats.superseded += 1;
                slot.ownership.release(sequence);
                continue;
            }
            let texture = slot.texture.as_ref().expect("copied slot texture");
            published = Some(FrameLease {
                token: pending.token,
                texture: texture.clone(),
                color_metadata: pending.color_metadata,
                payload: pending.payload,
                ownership: Arc::clone(&slot.ownership),
                _retirement: Arc::clone(&self.retirement),
            });
        }
        let Some(frame) = published else {
            return false;
        };
        self.last_published = frame.token.sequence;
        *lock(&self.shared.latest) = Some(Arc::new(frame));
        self.stats.published += 1;
        self.wake();
        true
    }

    pub fn stats(&self) -> FrameExchangeStats {
        FrameExchangeStats {
            occupied: self.occupied(),
            ..self.stats
        }
    }

    fn occupied(&self) -> u8 {
        self.slots
            .iter()
            .filter(|slot| !slot.ownership.is_free())
            .count() as u8
    }

    fn wake(&self) {
        if !self.shared.notified.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }
}

impl<E, M> Drop for FrameExchange<E, M> {
    fn drop(&mut self) {
        self.shared.active.store(false, Ordering::Release);
        // Held leases stay valid; the textures retire with their last owner.
        lock(&self.shared.latest).take();
        lock(&self.retirement.textures)
            .extend(self.slots.iter_mut().filter_map(|slot| slot.texture.take()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_is_held_by_one_sequence_until_that_sequence_releases_it() {
        let slot = SlotOwnership::new(0);
        assert!(!slot.reserve(0), "sequence 0 means free");
        assert!(slot.reserve(1));
        assert!(!slot.reserve(2));
        slot.release(2);
        assert!(!slot.is_free(), "another sequence cannot free the slot");
        slot.release(1);
        assert!(slot.reserve(2));
        slot.release(1);
        assert!(
            !slot.is_free(),
            "a delayed duplicate cannot free a reused slot"
        );
        slot.release(2);
        assert!(slot.is_free());
    }

    #[test]
    fn payloads_follow_completed_copies_without_invalidating_the_epoch() {
        let gpu = test_gpu();
        let mut exchange = FrameExchange::<_, String>::new_with_payload(
            &gpu,
            DEFAULT_CAPACITY,
            7u64,
            Arc::new(|| {}),
        );
        let inbox = exchange.inbox();
        let texture = source(&gpu, 4, 1, wgpu::TextureUsages::COPY_SRC);
        let color = FrameColorMetadata::unknown();
        assert_eq!(
            exchange.copy_from_with_payload(&texture, 7, color, "old crop".into()),
            CopyOutcome::Submitted
        );
        wait(&gpu);
        assert!(exchange.poll());
        let old = inbox.latest().unwrap();
        assert_eq!(old.payload(), "old crop");
        assert_eq!(
            exchange.copy_from_with_payload(&texture, 7, color, "new crop, same pixels".into()),
            CopyOutcome::Submitted
        );
        let pending = exchange
            .slots
            .iter()
            .find_map(|slot| slot.pending.as_ref())
            .unwrap();
        let completed = Arc::clone(&pending.completed);
        wait(&gpu);
        completed.store(false, Ordering::Release);
        assert!(!exchange.poll());
        assert_eq!(inbox.latest().unwrap().token(), old.token());
        assert_eq!(inbox.latest().unwrap().payload(), "old crop");
        completed.store(true, Ordering::Release);
        assert!(exchange.poll());
        let new = inbox.latest().unwrap();
        assert_eq!(new.payload(), "new crop, same pixels");
        assert!(inbox.is_current(&old.token()));
        assert_eq!(old.payload(), "old crop");
        assert!(new.token().sequence() > old.token().sequence());
        assert_eq!(new.size(), old.size());
        // All three slots are leased or the latest publication; excess work is dropped.
        assert_eq!(
            exchange.copy_from_with_payload(&texture, 7, color, "third".into()),
            CopyOutcome::Submitted
        );
        wait(&gpu);
        assert!(exchange.poll());
        let third = inbox.latest().unwrap();
        assert_eq!(
            exchange.copy_from_with_payload(&texture, 7, color, "dropped".into()),
            CopyOutcome::PoolFull
        );
        assert_eq!(inbox.latest().unwrap().payload(), "third");
        assert_eq!(exchange.stats().occupied, 3);
        drop(third);
    }

    #[test]
    fn a_late_copy_never_publishes_its_payload_after_an_epoch_change() {
        let gpu = test_gpu();
        let mut exchange = FrameExchange::<_, String>::new_with_payload(
            &gpu,
            DEFAULT_CAPACITY,
            1u64,
            Arc::new(|| {}),
        );
        let texture = source(&gpu, 4, 1, wgpu::TextureUsages::COPY_SRC);
        assert_eq!(
            exchange.copy_from_with_payload(
                &texture,
                1,
                FrameColorMetadata::unknown(),
                "stale".into()
            ),
            CopyOutcome::Submitted
        );
        exchange.set_epoch(2);
        wait(&gpu);
        assert!(!exchange.poll());
        assert!(exchange.inbox().latest().is_none());
        assert_eq!(exchange.stats().stale_epoch, 1);
        assert_eq!(exchange.stats().occupied, 0);
    }

    #[test]
    fn color_metadata_is_explicit_and_unknown_does_not_mean_srgb() {
        let pq = FrameColorMetadata::bt2020_pq(FrameAlphaMode::Premultiplied);
        assert!(pq.is_explicit());
        assert!(pq.is_hdr());
        assert_eq!(pq.primaries(), FrameColorPrimaries::Bt2020);
        assert_eq!(pq.transfer(), FrameTransferFunction::Pq);
        assert_eq!(pq.range(), FrameColorRange::Full);
        assert_eq!(pq.alpha(), FrameAlphaMode::Premultiplied);

        let unknown = FrameColorMetadata::unknown();
        assert!(!unknown.is_explicit());
        assert!(!unknown.is_hdr());
        assert_eq!(unknown.primaries(), FrameColorPrimaries::Unknown);
        assert_ne!(unknown, FrameColorMetadata::srgb(FrameAlphaMode::Opaque));
    }

    struct GpuTestGuard;

    static GPU_TEST_STATE: std::sync::Mutex<(u64, usize)> = std::sync::Mutex::new((0, 0));
    static GPU_TEST_FREE: std::sync::Condvar = std::sync::Condvar::new();

    thread_local! {
        static GPU_THREAD_TOKEN: u64 = {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        };
    }

    impl Drop for GpuTestGuard {
        fn drop(&mut self) {
            let mut state = GPU_TEST_STATE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.1 = state.1.saturating_sub(1);
            if state.1 == 0 {
                state.0 = 0;
                GPU_TEST_FREE.notify_one();
            }
        }
    }

    fn lock_test_gpu() -> GpuTestGuard {
        let token = GPU_THREAD_TOKEN.with(|token| *token);
        let mut state = GPU_TEST_STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.1 > 0 && state.0 != token {
            state = GPU_TEST_FREE
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.0 = token;
        state.1 += 1;
        GpuTestGuard
    }

    static ACTIVE_RETIREMENTS: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
    static RETIREMENTS_IDLE: std::sync::Condvar = std::sync::Condvar::new();

    pub(super) struct RetirementTracker;

    impl Drop for RetirementTracker {
        fn drop(&mut self) {
            let mut count = ACTIVE_RETIREMENTS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *count = count.saturating_sub(1);
            if *count == 0 {
                RETIREMENTS_IDLE.notify_all();
            }
        }
    }

    pub(super) fn enter_retirement() -> RetirementTracker {
        let mut count = ACTIVE_RETIREMENTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *count += 1;
        RetirementTracker
    }

    fn wait_retirements() {
        let mut count = ACTIVE_RETIREMENTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while *count > 0 {
            let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
                break;
            };
            let (next, result) = RETIREMENTS_IDLE
                .wait_timeout(count, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            count = next;
            if result.timed_out() {
                break;
            }
        }
    }

    struct TestGpu {
        gpu: Option<GpuContext>,
        _guard: GpuTestGuard,
    }

    impl Drop for TestGpu {
        fn drop(&mut self) {
            drop(self.gpu.take());
            wait_retirements();
        }
    }

    impl std::ops::Deref for TestGpu {
        type Target = GpuContext;

        fn deref(&self) -> &Self::Target {
            self.gpu.as_ref().expect("TestGpu live until drop")
        }
    }

    fn test_gpu() -> TestGpu {
        static DEVICE: std::sync::OnceLock<(
            wgpu::Instance,
            wgpu::Adapter,
            wgpu::Device,
            wgpu::Queue,
        )> = std::sync::OnceLock::new();
        let guard = lock_test_gpu();
        let (_instance, adapter, device, queue) = DEVICE.get_or_init(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::from_env().unwrap_or_default(),
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
                &instance, None,
            ))
            .expect("frame exchange test requires a WGPU adapter");
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                    .expect("frame exchange test requires a WGPU device");
            (instance, adapter, device, queue)
        });
        TestGpu {
            gpu: Some(__framework::adopt(
                adapter.clone(),
                device.clone(),
                queue.clone(),
            )),
            _guard: guard,
        }
    }

    #[test]
    fn window_frame_slots_do_not_hold_back_producer_copies() {
        let gpu = test_gpu();
        gpu.policy().set_frame_slot_count(1);
        let recording = gpu.try_begin_frame("unsubmitted window frame").unwrap();
        let mut exchange = FrameExchange::new(&gpu, DEFAULT_CAPACITY, 0u64, Arc::new(|| {}));
        let texture = source(&gpu, 4, 1, wgpu::TextureUsages::COPY_SRC);
        assert_eq!(exchange.copy_from(&texture, 0), CopyOutcome::Submitted);
        drop(recording);
        wait(&gpu);
        assert!(exchange.poll());
    }

    #[test]
    fn steady_exchange_reuses_real_transient_texture() {
        let gpu = test_gpu();
        let mut exchange =
            FrameExchange::new(&gpu, NonZeroU8::new(1).unwrap(), 0u64, Arc::new(|| {}));
        let texture = source(&gpu, 4, 1, wgpu::TextureUsages::COPY_SRC);
        assert_eq!(exchange.copy_from(&texture, 0), CopyOutcome::Submitted);
        wait(&gpu);
        assert!(exchange.poll());
        exchange.set_epoch(1);
        wait(&gpu);
        let _ = exchange.poll();
        assert_eq!(exchange.copy_from(&texture, 1), CopyOutcome::Submitted);
        let stats = gpu.policy().stats();
        assert!(stats.transient_pool_hits >= 1);
    }

    #[test]
    fn lease_carries_the_producer_color_declaration_without_conversion() {
        let gpu = test_gpu();
        let mut exchange =
            FrameExchange::new(&gpu, NonZeroU8::new(1).unwrap(), 0u64, Arc::new(|| {}));
        let inbox = exchange.inbox();
        let texture = source(&gpu, 4, 1, wgpu::TextureUsages::COPY_SRC);
        let metadata = FrameColorMetadata::bt2020_hlg(FrameAlphaMode::Opaque);
        assert_eq!(
            exchange.copy_from_with_metadata(&texture, 0, metadata),
            CopyOutcome::Submitted
        );
        wait(&gpu);
        assert!(exchange.poll());
        let lease = inbox.latest().expect("published frame");
        assert_eq!(lease.color_metadata(), metadata);
        assert_eq!(lease.format(), GpuTextureFormat::RGBA8_UNORM);
    }

    fn source(gpu: &GpuContext, size: u32, layers: u32, usage: wgpu::TextureUsages) -> GpuTexture {
        let texture = __framework::device(gpu).create_texture(&wgpu::TextureDescriptor {
            label: Some("frame exchange test source"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage,
            view_formats: &[],
        });
        __framework::texture_from_wgpu(gpu, texture)
    }

    fn wait(gpu: &GpuContext) {
        __framework::device(gpu)
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(15)),
            })
            .unwrap();
    }

    #[test]
    fn sources_that_cannot_be_copied_are_refused_without_taking_a_slot() {
        let gpu = test_gpu();
        let mut exchange = FrameExchange::new(&gpu, DEFAULT_CAPACITY, 0, Arc::new(|| {}));
        let unreadable = source(&gpu, 4, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        let layered = source(&gpu, 4, 2, wgpu::TextureUsages::COPY_SRC);
        let foreign = test_gpu();
        let elsewhere = source(&foreign, 4, 1, wgpu::TextureUsages::COPY_SRC);
        assert_eq!(
            exchange.copy_from(&elsewhere, 0),
            CopyOutcome::DeviceMismatch
        );
        assert_eq!(
            exchange.copy_from(&unreadable, 0),
            CopyOutcome::IncompatibleSource
        );
        assert_eq!(
            exchange.copy_from(&layered, 0),
            CopyOutcome::IncompatibleSource
        );
        assert_eq!(exchange.stats(), FrameExchangeStats::default());
    }

    #[test]
    fn leases_bound_the_pool_survive_resize_and_outlive_the_exchange() {
        let gpu = test_gpu();
        let wakes = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&wakes);
        let mut exchange = FrameExchange::new(
            &gpu,
            DEFAULT_CAPACITY,
            (1u64, 1u64),
            Arc::new(move || {
                observed.fetch_add(1, Ordering::AcqRel);
            }),
        );
        let inbox = exchange.inbox();
        let copyable = |size| source(&gpu, size, 1, wgpu::TextureUsages::COPY_SRC);
        let original = copyable(4);

        // Without a consumer there is one outstanding wake and one latest frame,
        // however many frames complete.
        for _ in 0..32 {
            assert_eq!(
                exchange.copy_from(&original, (1, 1)),
                CopyOutcome::Submitted
            );
            wait(&gpu);
            exchange.poll();
        }
        assert_eq!(inbox.latest().unwrap().token().sequence(), 32);
        assert_eq!(wakes.load(Ordering::Acquire), 1);
        assert_eq!(exchange.stats().published, 32);
        drop(inbox.try_take_latest());
        wakes.store(0, Ordering::Release);

        let capacity = usize::from(DEFAULT_CAPACITY.get());
        let mut held = Vec::new();
        for _ in 0..capacity {
            assert_eq!(
                exchange.copy_from(&original, (1, 1)),
                CopyOutcome::Submitted
            );
            wait(&gpu);
            assert!(exchange.poll());
            held.push(inbox.try_take_latest().expect("completed frame"));
        }
        assert_eq!(wakes.load(Ordering::Acquire), 3);
        let full_before = exchange.stats().pool_full;
        for _ in 0..100 {
            assert!(!exchange.poll());
            assert_eq!(exchange.copy_from(&original, (1, 1)), CopyOutcome::PoolFull);
        }
        assert_eq!(
            wakes.load(Ordering::Acquire),
            3,
            "a full pool must not wake the consumer"
        );
        assert_eq!(exchange.stats().pool_full, full_before + 100);
        assert_eq!(usize::from(exchange.stats().occupied), capacity);

        let resized = copyable(8);
        drop(held.remove(0));
        assert_eq!(
            exchange.copy_from(&resized, (2, 1)),
            CopyOutcome::PoolFull,
            "returning a lease is not completion"
        );
        exchange.poll();
        wait(&gpu);
        assert_eq!(exchange.copy_from(&resized, (2, 1)), CopyOutcome::Submitted);
        wait(&gpu);
        exchange.poll();
        let newest = inbox.latest().unwrap();
        assert_eq!(newest.size(), (8, 8));
        assert_eq!(newest.token().epoch(), (2, 1));
        assert_eq!(held[0].size(), (4, 4), "a resize keeps leased dimensions");
        drop(held);
        exchange.poll();
        wait(&gpu);

        assert_eq!(
            exchange.copy_from(&original, (3, 1)),
            CopyOutcome::Submitted
        );
        // Epochs with equal sums are still different incarnations.
        exchange.set_epoch((2, 2));
        assert!(!inbox.is_current(&newest.token()));
        wait(&gpu);
        let stale_before = exchange.stats().stale_epoch;
        assert!(!exchange.poll());
        assert_eq!(exchange.stats().stale_epoch, stale_before + 1);
        assert!(inbox.latest().is_none());

        for generation in 10..110 {
            exchange.set_epoch((generation, 2));
            exchange.poll();
            let size = if generation % 2 == 0 { 4 } else { 8 };
            let changing = copyable(size);
            assert_eq!(
                exchange.copy_from(&changing, (generation, 2)),
                CopyOutcome::Submitted
            );
            wait(&gpu);
            assert!(exchange.poll());
            assert_eq!(inbox.latest().unwrap().size(), (size, size));
            assert!(
                exchange
                    .slots
                    .iter()
                    .filter(|slot| slot.texture.is_some())
                    .count()
                    <= capacity
            );
        }
        assert!(usize::from(exchange.stats().occupied_high_water) <= capacity);

        let producer = std::thread::current().id();
        let (completed, completion) = std::sync::mpsc::channel();
        __framework::queue(&exchange.gpu).on_submitted_work_done(move || {
            completed.send(std::thread::current().id()).unwrap();
        });
        drop(exchange);
        assert!(inbox.latest().is_none());
        assert!(inbox.try_take_latest().is_none());
        assert_eq!(
            newest.size(),
            (8, 8),
            "retiring the exchange keeps held leases valid"
        );
        assert!(
            completion.try_recv().is_err(),
            "the device retires only with its last lease"
        );
        drop(newest);
        assert_ne!(
            completion
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            producer,
            "the last queue completion runs on the retirement thread"
        );
    }

    #[test]
    fn write_slot_renders_and_publishes_without_copy_and_reuses_slot_texture() {
        let gpu = test_gpu();
        let mut exchange = FrameExchange::<_, String>::new_with_payload(
            &gpu,
            NonZeroU8::new(1).unwrap(),
            5u64,
            Arc::new(|| {}),
        );
        let inbox = exchange.inbox();
        let slot = exchange
            .try_begin_write(4, 4, GpuTextureFormat::RGBA8_UNORM)
            .expect("free slot");
        let first_ptr = slot.texture().clone();

        let device = __framework::device(&gpu);
        let queue = __framework::queue(&gpu);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("write slot clear"),
        });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("write slot pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: __framework::texture_view(slot.texture()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.2,
                            g: 0.4,
                            b: 0.6,
                            a: 0.8,
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
        queue.submit([encoder.finish()]);
        let metadata = FrameColorMetadata::srgb(FrameAlphaMode::Premultiplied);
        assert_eq!(
            exchange.commit_write_with_payload(slot, 5, metadata, "direct".into()),
            CopyOutcome::Submitted
        );
        wait(&gpu);
        assert!(exchange.poll());

        let lease = inbox.try_take_latest().expect("published lease");
        assert_eq!(lease.payload(), "direct");
        assert_eq!(lease.color_metadata(), metadata);
        assert!(lease.texture().ptr_eq(&first_ptr));

        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("write slot readback"),
            size: 256 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut copy_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("write slot readback copy"),
        });
        copy_enc.copy_texture_to_buffer(
            __framework::texture(lease.texture()).as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(4),
                },
            },
            wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([copy_enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| {
                let _ = tx.send(res);
            });
        wait(&gpu);
        rx.recv().unwrap().unwrap();
        let data = readback.slice(..).get_mapped_range().unwrap();
        let expected = [51u8, 102, 153, 204];
        for actual in data[0..16].chunks_exact(4) {
            for (a, e) in actual.iter().zip(expected.iter()) {
                assert!(
                    a.abs_diff(*e) <= 1,
                    "expected ~{expected:?}, got {actual:?}"
                );
            }
        }
        drop(data);
        readback.unmap();

        drop(lease);
        exchange.set_epoch(6);
        exchange.poll();
        wait(&gpu);

        let second = exchange
            .try_begin_write(4, 4, GpuTextureFormat::RGBA8_UNORM)
            .expect("reused slot");
        assert!(
            second.texture().ptr_eq(&first_ptr),
            "re-acquiring the same size and format must reuse the slot texture without reallocation"
        );
    }

    #[test]
    fn dropped_write_slot_returns_to_pool_immediately() {
        let gpu = test_gpu();
        let mut exchange =
            FrameExchange::<_, ()>::new_with_payload(&gpu, DEFAULT_CAPACITY, 1u64, Arc::new(|| {}));
        assert_eq!(
            exchange
                .try_begin_write(0, 4, GpuTextureFormat::RGBA8_UNORM)
                .err(),
            Some(WriteAcquireError::EmptyExtent)
        );
        let mut held = Vec::new();
        for _ in 0..DEFAULT_CAPACITY.get() {
            held.push(
                exchange
                    .try_begin_write(4, 4, GpuTextureFormat::RGBA8_UNORM)
                    .expect("free slot"),
            );
        }
        assert_eq!(
            exchange
                .try_begin_write(4, 4, GpuTextureFormat::RGBA8_UNORM)
                .err(),
            Some(WriteAcquireError::PoolFull)
        );
        held.pop();
        let reacquired = exchange
            .try_begin_write(4, 4, GpuTextureFormat::RGBA8_UNORM)
            .expect("dropped write slot immediately returns to pool");
        assert_eq!(reacquired.size(), (4, 4));
    }

    #[test]
    fn write_slot_commit_respects_epoch_change() {
        let gpu = test_gpu();
        let mut exchange = FrameExchange::<_, String>::new_with_payload(
            &gpu,
            NonZeroU8::new(1).unwrap(),
            10u64,
            Arc::new(|| {}),
        );
        let inbox = exchange.inbox();
        let slot = exchange
            .try_begin_write(4, 4, GpuTextureFormat::RGBA8_UNORM)
            .expect("free slot");
        assert_eq!(
            exchange.commit_write_with_payload(
                slot,
                10,
                FrameColorMetadata::unknown(),
                "stale".into()
            ),
            CopyOutcome::Submitted
        );
        exchange.set_epoch(11);
        wait(&gpu);
        assert!(!exchange.poll());
        assert_eq!(exchange.stats().stale_epoch, 1);
        assert!(inbox.latest().is_none());
    }
}
