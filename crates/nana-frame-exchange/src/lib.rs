//! Bounded latest-frame exchange on a host's existing GPU.
//!
//! A producer thread copies finished frames into a small pool of textures on
//! the shared [`GpuContext`], and a consumer — usually a UI window — samples the
//! newest completed copy. Neither side waits for the other or for GPU
//! completion: a full pool drops producer work, and a lease frees its slot
//! only after the consumer submission that sampled it has completed.
//!
//! The copy is a real GPU-side `copy_texture_to_texture`, not zero-copy. It
//! never reads pixels back to the CPU.

use std::{
    num::NonZeroU8,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use nana_gpu::{__framework, GpuTextureDescriptor, GpuTextureUsages};
/// The contract types an exchange is built from, so a producer crate needs no
/// other GPU dependency.
pub use nana_gpu::{DeviceGeneration, GpuContext, GpuTexture, GpuTextureFormat};

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
    /// Every slot is copying or leased. The frame is dropped; nothing waits.
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
        let _ = std::thread::Builder::new()
            .name("nana-frame-exchange-retire".into())
            .spawn(move || {
                let _ = __framework::device(&gpu).poll(wgpu::PollType::wait_indefinitely());
                drop(textures);
                drop(gpu);
            });
    }
}

/// A completed copy the consumer may sample until it drops the lease.
pub struct FrameLease<E = u64> {
    token: FrameToken<E>,
    texture: GpuTexture,
    ownership: Arc<SlotOwnership>,
    _retirement: Arc<Retirement>,
}

impl<E: Copy> FrameLease<E> {
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
}

impl<E> Drop for FrameLease<E> {
    fn drop(&mut self) {
        // Only record the return. Registering the queue callback that frees
        // the slot can enter the backend, which belongs to the producer lane.
        self.ownership
            .returned
            .store(self.token.sequence, Ordering::Release);
    }
}

struct Shared<E> {
    exchange: u64,
    device_generation: DeviceGeneration,
    latest: Mutex<Option<Arc<FrameLease<E>>>>,
    epoch: Mutex<E>,
    notified: AtomicBool,
    active: AtomicBool,
}

/// Consumer side. Never waits for the producer or for GPU completion.
pub struct FrameInbox<E = u64> {
    shared: Arc<Shared<E>>,
}

impl<E> Clone for FrameInbox<E> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<E: Copy + Eq> FrameInbox<E> {
    pub fn device_generation(&self) -> DeviceGeneration {
        self.shared.device_generation
    }

    /// Acknowledge the pending wake and acquire the latest frame. Repeated
    /// calls can return the same frame; compare tokens.
    pub fn try_take_latest(&self) -> Option<Arc<FrameLease<E>>> {
        // Acknowledge before loading: a racing publication either appears in
        // this load or raises a new wake.
        self.shared.notified.store(false, Ordering::Release);
        self.latest()
    }

    /// Inspect the latest frame without acknowledging the wake.
    pub fn latest(&self) -> Option<Arc<FrameLease<E>>> {
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
}

struct PendingCopy<E> {
    token: FrameToken<E>,
    completed: Arc<AtomicBool>,
}

struct Slot<E> {
    ownership: Arc<SlotOwnership>,
    texture: Option<GpuTexture>,
    pending: Option<PendingCopy<E>>,
}

/// Producer side, owned by one thread.
pub struct FrameExchange<E = u64> {
    shared: Arc<Shared<E>>,
    gpu: GpuContext,
    retirement: Arc<Retirement>,
    slots: Box<[Slot<E>]>,
    epoch: E,
    notify: Arc<dyn Fn() + Send + Sync>,
    sequence: u64,
    last_published: u64,
    stats: FrameExchangeStats,
}

impl<E: Copy + Eq + Send + Sync + 'static> FrameExchange<E> {
    /// `notify` runs on the producer thread once per acknowledged wake; it
    /// should only schedule the consumer, never wait for it.
    pub fn new(
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

    pub fn inbox(&self) -> FrameInbox<E> {
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
    /// Safe from any thread: the copy is submitted under the device's
    /// submission guard, so it never races a window's surface reconfiguration.
    pub fn copy_from(&mut self, source: &GpuTexture, epoch: E) -> CopyOutcome {
        if source.generation() != self.gpu.generation() {
            self.set_epoch(epoch);
            return CopyOutcome::DeviceMismatch;
        }
        self.copy_raw(__framework::texture(source), epoch)
    }

    /// [`Self::copy_from`] for a raw WGPU texture created on the exchange's
    /// device.
    #[cfg(feature = "wgpu-interop")]
    pub fn copy_from_wgpu(&mut self, source: &wgpu::Texture, epoch: E) -> CopyOutcome {
        self.copy_raw(source, epoch)
    }

    fn copy_raw(&mut self, source: &wgpu::Texture, epoch: E) -> CopyOutcome {
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
        // Only a free slot is resized, so a leased frame keeps its dimensions
        // and the pool never grows past its capacity.
        if slot.texture.as_ref().is_none_or(|texture| {
            texture.size() != (size.width, size.height) || texture.format() != format
        }) {
            match self.gpu.create_texture(&GpuTextureDescriptor {
                label: Some("NanaUI frame exchange slot"),
                width: size.width,
                height: size.height,
                format,
                usage: GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST,
            }) {
                Ok(texture) => slot.texture = Some(texture),
                Err(_) => {
                    slot.ownership.release(sequence);
                    return CopyOutcome::IncompatibleSource;
                }
            }
        }
        let texture = __framework::texture(slot.texture.as_ref().expect("frame exchange slot"));
        let device = __framework::device(&self.gpu);
        let queue = __framework::queue(&self.gpu);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("NanaUI frame exchange copy"),
        });
        encoder.copy_texture_to_texture(source.as_image_copy(), texture.as_image_copy(), size);
        let commands = encoder.finish();
        {
            let _submission = __framework::lock_submission(&self.gpu);
            queue.submit([commands]);
        }
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
        let _ = __framework::device(&self.gpu).poll(wgpu::PollType::Poll);
        let epoch = self.epoch;
        let completed = |pending: &PendingCopy<E>| pending.completed.load(Ordering::Acquire);
        let newest = self
            .slots
            .iter()
            .filter_map(|slot| slot.pending.as_ref())
            .filter(|pending| pending.token.epoch == epoch && completed(pending))
            .map(|pending| pending.token.sequence)
            .max();
        let mut published = None;
        for slot in self.slots.iter_mut() {
            if !slot.pending.as_ref().is_some_and(completed) {
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

impl<E> Drop for FrameExchange<E> {
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

    fn test_gpu() -> GpuContext {
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
        __framework::adopt(adapter, device, queue)
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
            .poll(wgpu::PollType::wait_indefinitely())
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
}
