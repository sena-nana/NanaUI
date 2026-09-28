//! Shared per-device GPU policy bookkeeping.
//!
//! This module deliberately contains no WGPU types in its public surface.  It
//! gives renderers one authority for frame slots, upload accounting,
//! transient-resource keys, pipeline identities and retirement.  Backend
//! objects remain owned by the host and are attached to these records by the
//! renderer that created them.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::{DeviceGeneration, GpuTexture, GpuTextureFormat};

/// A bounded map whose hits cost one stamp write. When full it evicts the
/// least recently used eighth at once, found with a linear selection, so
/// eviction is amortized O(1) per insert instead of an O(n) reorder on every
/// hit.
#[derive(Debug)]
struct StampedCache<K, V> {
    entries: HashMap<K, (V, u64)>,
    clock: u64,
    capacity: usize,
}

impl<K: std::hash::Hash + Eq + Clone, V> StampedCache<K, V> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            capacity: capacity.max(8),
        }
    }

    fn get(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(key).map(|(value, stamp)| {
            *stamp = clock;
            &*value
        })
    }

    /// Insert, handing back whatever was evicted to make room.
    fn insert(&mut self, key: K, value: V, evicted: &mut Vec<V>) {
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            let mut stamps: Vec<u64> = self.entries.values().map(|(_, stamp)| *stamp).collect();
            let cut = (self.capacity / 8).max(1) - 1;
            let (_, threshold, _) = stamps.select_nth_unstable(cut);
            let threshold = *threshold;
            let doomed: Vec<K> = self
                .entries
                .iter()
                .filter(|(_, (_, stamp))| *stamp <= threshold)
                .map(|(key, _)| key.clone())
                .collect();
            for key in doomed {
                if let Some((value, _)) = self.entries.remove(&key) {
                    evicted.push(value);
                }
            }
        }
        self.clock += 1;
        self.entries.insert(key, (value, self.clock));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuPolicyStats {
    pub upload_bytes: u64,
    pub buffer_reallocations: u64,
    pub transient_pool_hits: u64,
    pub transient_pool_misses: u64,
    pub pipeline_registry_hits: u64,
    pub pipeline_registry_misses: u64,
    /// `begin_frame` found every slot held by an unsubmitted recording and
    /// went ahead without one.
    pub frame_slot_stalls: u64,
    /// `begin_frame` blocked on the oldest in-flight frame.
    pub frame_slot_waits: u64,
    pub retired_resources: u64,
    /// Writes queued through frame uploads.
    pub upload_writes: u64,
    /// Copies those writes became after merging.
    pub upload_copies: u64,
    /// Upload command buffers submitted.
    pub upload_flushes: u64,
    pub upload_ring_allocations: u64,
    pub upload_ring_waits: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransientResourceKey {
    pub generation: DeviceGeneration,
    pub format: GpuTextureFormat,
    pub usage: u32,
    pub width: u32,
    pub height: u32,
    pub depth_or_layers: u32,
    pub mip_levels: u32,
    pub sample_count: u32,
    pub byte_size: u64,
}

impl TransientResourceKey {
    pub const fn new(
        generation: DeviceGeneration,
        format: GpuTextureFormat,
        usage: u32,
        width: u32,
        height: u32,
        sample_count: u32,
    ) -> Self {
        Self {
            generation,
            format,
            usage,
            width,
            height,
            depth_or_layers: 1,
            mip_levels: 1,
            sample_count,
            byte_size: 0,
        }
    }

    pub const fn buffer(generation: DeviceGeneration, usage: u32, byte_size: u64) -> Self {
        Self {
            generation,
            format: GpuTextureFormat::R8_UNORM,
            usage,
            width: 1,
            height: 1,
            depth_or_layers: 1,
            mip_levels: 1,
            sample_count: 1,
            byte_size,
        }
    }

    pub const fn with_layers(mut self, depth_or_layers: u32, mip_levels: u32) -> Self {
        self.depth_or_layers = depth_or_layers;
        self.mip_levels = mip_levels;
        self
    }

    pub fn bytes_hint(self) -> u64 {
        if self.byte_size != 0 {
            return self.byte_size;
        }
        let bpp = self.format.bytes_per_pixel().unwrap_or(4) as u64;
        let samples = u64::from(self.sample_count.max(1));
        let layers = u64::from(self.depth_or_layers.max(1));
        let mips = u64::from(self.mip_levels.max(1));
        (0..mips.min(32)).fold(0u64, |total, mip| {
            total.saturating_add(
                (u64::from(self.width.max(1)) >> mip).max(1)
                    * (u64::from(self.height.max(1)) >> mip).max(1)
                    * layers
                    * samples
                    * bpp,
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineKey {
    pub generation: DeviceGeneration,
    pub target_format: crate::GpuTextureFormat,
    pub sample_count: u32,
    pub shader: u64,
    pub layout: u64,
    pub material: u64,
    pub primitive: u64,
    pub blend: u64,
    pub depth: u64,
    pub vertex_layout: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameSlotId(pub u32);

/// Which frame holds a slot and, once it is submitted, the submission to
/// wait on for the slot.
#[derive(Debug)]
struct FrameSlot {
    token: u64,
    submission: Option<wgpu::SubmissionIndex>,
}

/// Why no slot was free.
pub(crate) enum SlotWait {
    /// Wait for this submission; its completion frees a slot.
    Submitted(wgpu::SubmissionIndex),
    /// Every slot is held by a recording that has not been submitted.
    Unsubmitted,
}

#[derive(Debug)]
struct RetiredResource {
    submission: u64,
    bytes: u64,
}

#[derive(Debug)]
struct PolicyState {
    transient_budget: u64,
    transient_bytes: u64,
    transient: HashMap<TransientResourceKey, VecDeque<u64>>,
    transient_buffers: HashMap<TransientResourceKey, VecDeque<wgpu::Buffer>>,
    transient_textures: HashMap<TransientResourceKey, VecDeque<GpuTexture>>,
    pipelines: StampedCache<PipelineKey, wgpu::RenderPipeline>,
    retired_pipelines: Vec<(u64, wgpu::RenderPipeline)>,
    layouts: StampedCache<u64, Arc<wgpu::BindGroupLayout>>,
    retired_layouts: Vec<(u64, Arc<wgpu::BindGroupLayout>)>,
    retired: Vec<RetiredResource>,
    frame_slots: Vec<Option<FrameSlot>>,
    stats: GpuPolicyStats,
}

/// Shared policy state for one `DeviceGeneration`.
#[derive(Clone, Debug)]
pub struct GpuDeviceState {
    generation: DeviceGeneration,
    inner: Arc<Mutex<PolicyState>>,
}

impl GpuDeviceState {
    pub(crate) fn new(generation: DeviceGeneration) -> Self {
        Self {
            generation,
            inner: Arc::new(Mutex::new(PolicyState {
                transient_budget: 64 * 1024 * 1024,
                transient_bytes: 0,
                transient: HashMap::new(),
                transient_buffers: HashMap::new(),
                transient_textures: HashMap::new(),
                pipelines: StampedCache::new(MAX_PIPELINES),
                retired_pipelines: Vec::new(),
                layouts: StampedCache::new(MAX_LAYOUTS),
                retired_layouts: Vec::new(),
                retired: Vec::new(),
                frame_slots: vec![None, None, None],
                stats: GpuPolicyStats::default(),
            })),
        }
    }

    pub fn generation(&self) -> DeviceGeneration {
        self.generation
    }

    pub fn set_transient_budget(&self, bytes: u64) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.transient_budget = bytes;
        evict_transient(&mut state);
    }

    /// Configure the number of in-flight slots. Existing reservations are
    /// discarded only when the new capacity is smaller than their index;
    /// callers should do this after a device/surface generation change.
    pub fn set_frame_slot_count(&self, count: usize) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let occupied = state
            .frame_slots
            .iter()
            .filter(|slot| slot.is_some())
            .count();
        let last_occupied = state
            .frame_slots
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |index| index + 1);
        state
            .frame_slots
            .resize_with(count.max(1).max(occupied).max(last_occupied), || None);
    }

    /// Take a free slot for the frame `token`, or `None` when all are held.
    pub fn acquire_frame_slot(&self, token: u64) -> Option<FrameSlotId> {
        self.take_frame_slot(token).ok()
    }

    /// Take a free slot, or say what to wait for.
    pub(crate) fn take_frame_slot(&self, token: u64) -> Result<FrameSlotId, SlotWait> {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((index, slot)) = state
            .frame_slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            *slot = Some(FrameSlot {
                token,
                submission: None,
            });
            return Ok(FrameSlotId(index as u32));
        }
        // Tokens increase with each frame, so the smallest submitted one is
        // the oldest submission holding a slot.
        Err(state
            .frame_slots
            .iter()
            .flatten()
            .filter_map(|slot| slot.submission.clone().map(|index| (slot.token, index)))
            .min_by_key(|(token, _)| *token)
            .map_or(SlotWait::Unsubmitted, |(_, index)| {
                SlotWait::Submitted(index)
            }))
    }

    /// The frame holding `slot` was submitted as `submission`.
    pub(crate) fn mark_frame_slot_submitted(
        &self,
        slot: FrameSlotId,
        token: u64,
        submission: wgpu::SubmissionIndex,
    ) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(Some(held)) = state.frame_slots.get_mut(slot.0 as usize)
            && held.token == token
        {
            held.submission = Some(submission);
        }
    }

    pub(crate) fn record_slot_wait(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stats
            .frame_slot_waits += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::FRAME_SLOT_WAITS);
    }

    pub(crate) fn record_slot_stall(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stats
            .frame_slot_stalls += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::FRAME_SLOT_STALLS);
        nana_diagnostics::event!(nana_diagnostics::framework::gpu::FRAME_SLOTS_EXHAUSTED);
    }

    /// Free `slot` if the frame holding it has token `completed` or older.
    pub fn release_frame_slot(&self, slot: FrameSlotId, completed: u64) -> bool {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(owner) = state.frame_slots.get_mut(slot.0 as usize) else {
            return false;
        };
        if owner.as_ref().is_some_and(|held| held.token <= completed) {
            *owner = None;
            true
        } else {
            false
        }
    }

    /// Release a slot by its unique frame token. This is used for completion
    /// callbacks because frame creation order and queue submission order may
    /// differ.
    pub fn release_frame_slot_exact(&self, slot: FrameSlotId, token: u64) -> bool {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(owner) = state.frame_slots.get_mut(slot.0 as usize) else {
            return false;
        };
        if owner.as_ref().is_some_and(|held| held.token == token) {
            *owner = None;
            true
        } else {
            false
        }
    }

    pub fn record_upload(&self, bytes: u64) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.stats.upload_bytes = state.stats.upload_bytes.saturating_add(bytes);
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::UPLOAD_BYTES, bytes);
    }

    pub(crate) fn record_ring_allocation(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stats
            .upload_ring_allocations += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::UPLOAD_RING_ALLOCATIONS);
    }

    pub(crate) fn record_ring_wait(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stats
            .upload_ring_waits += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::UPLOAD_RING_WAITS);
    }

    /// One upload command buffer: `writes` queued writes became `copies`
    /// copies.
    pub(crate) fn record_flush(&self, writes: u64, copies: u64) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.stats.upload_writes += writes;
        state.stats.upload_copies += copies;
        state.stats.upload_flushes += 1;
        drop(state);
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::UPLOAD_WRITES, writes);
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::UPLOAD_COPIES, copies);
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::UPLOAD_FLUSHES);
    }

    pub fn record_reallocation(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.stats.buffer_reallocations = state.stats.buffer_reallocations.saturating_add(1);
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::BUFFER_REALLOCATIONS);
    }

    pub fn acquire_transient(&self, key: TransientResourceKey) -> Option<u64> {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if key.generation != self.generation {
            return None;
        }
        if let Some(ids) = state.transient.get_mut(&key)
            && let Some(id) = ids.pop_front()
        {
            let empty = ids.is_empty();
            state.transient_bytes = state.transient_bytes.saturating_sub(key.bytes_hint());
            state.stats.transient_pool_hits += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::TRANSIENT_POOL_HITS);
            if empty {
                state.transient.remove(&key);
            }
            return Some(id);
        }
        state.stats.transient_pool_misses += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::TRANSIENT_POOL_MISSES);
        None
    }

    pub fn release_transient(&self, key: TransientResourceKey, id: u64) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if key.generation != self.generation {
            return;
        }
        state.transient_bytes = state.transient_bytes.saturating_add(key.bytes_hint());
        state.transient.entry(key).or_default().push_back(id);
        evict_transient(&mut state);
    }

    /// Acquire a real buffer from the bounded, generation-local transient
    /// pool. The factory runs under the policy lock, so concurrent cold
    /// requests cannot accidentally share one mutable buffer.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn acquire_transient_buffer(
        &self,
        key: TransientResourceKey,
        create: impl FnOnce() -> wgpu::Buffer,
    ) -> Result<wgpu::Buffer, crate::GpuError> {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if key.generation != self.generation {
            return Err(crate::GpuError::DeviceMismatch {
                expected: self.generation,
                found: key.generation,
            });
        }
        let pooled = state.transient_buffers.get_mut(&key).and_then(|buffers| {
            let buffer = buffers.pop_front();
            let empty = buffers.is_empty();
            buffer.map(|buffer| (buffer, empty))
        });
        if let Some((buffer, empty)) = pooled {
            state.transient_bytes = state.transient_bytes.saturating_sub(key.bytes_hint());
            state.stats.transient_pool_hits += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::TRANSIENT_POOL_HITS);
            if empty {
                state.transient_buffers.remove(&key);
            }
            return Ok(buffer);
        }
        state.stats.transient_pool_misses += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::TRANSIENT_POOL_MISSES);
        let buffer = create();
        if buffer.size() != key.byte_size || buffer.usage().bits() != key.usage {
            return Err(crate::GpuError::TransientDescriptorMismatch);
        }
        Ok(buffer)
    }

    /// Return a transient buffer after the last submission using it has
    /// completed. Descriptor and generation mismatches are dropped rather
    /// than allowing an alias across resource semantics or device epochs.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn release_transient_buffer(&self, key: TransientResourceKey, buffer: wgpu::Buffer) {
        if key.generation != self.generation
            || buffer.size() != key.byte_size
            || buffer.usage().bits() != key.usage
        {
            return;
        }
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.transient_bytes = state.transient_bytes.saturating_add(key.bytes_hint());
        state
            .transient_buffers
            .entry(key)
            .or_default()
            .push_back(buffer);
        evict_transient(&mut state);
    }

    /// Acquire a real texture from the bounded transient pool, creating it on
    /// a miss. The factory runs while the policy lock is held so concurrent
    /// cold requests cannot create duplicate resources for the same key.
    pub fn acquire_transient_texture(
        &self,
        key: TransientResourceKey,
        create: impl FnOnce() -> Result<GpuTexture, crate::GpuError>,
    ) -> Result<GpuTexture, crate::GpuError> {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if key.generation != self.generation {
            return Err(crate::GpuError::DeviceMismatch {
                expected: self.generation,
                found: key.generation,
            });
        }
        let pooled = state.transient_textures.get_mut(&key).and_then(|textures| {
            let texture = textures.pop_front();
            let empty = textures.is_empty();
            texture.map(|texture| (texture, empty))
        });
        if let Some((texture, empty)) = pooled {
            state.transient_bytes = state.transient_bytes.saturating_sub(key.bytes_hint());
            state.stats.transient_pool_hits += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::TRANSIENT_POOL_HITS);
            if empty {
                state.transient_textures.remove(&key);
            }
            return Ok(texture);
        }
        state.stats.transient_pool_misses += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::TRANSIENT_POOL_MISSES);
        let texture = create()?;
        if texture.generation() != self.generation {
            return Err(crate::GpuError::DeviceMismatch {
                expected: self.generation,
                found: texture.generation(),
            });
        }
        if !texture_matches_key(&texture, key) {
            return Err(crate::GpuError::TransientDescriptorMismatch);
        }
        Ok(texture)
    }

    /// Return a real texture after its last lease/submission has completed.
    /// Generation mismatches are dropped instead of crossing device epochs.
    pub fn release_transient_texture(&self, key: TransientResourceKey, texture: GpuTexture) {
        if key.generation != self.generation || !texture_matches_key(&texture, key) {
            return;
        }
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.transient_bytes = state.transient_bytes.saturating_add(key.bytes_hint());
        state
            .transient_textures
            .entry(key)
            .or_default()
            .push_back(texture);
        evict_transient(&mut state);
    }

    pub(crate) fn pipeline(
        &self,
        key: PipelineKey,
        create: impl FnOnce() -> wgpu::RenderPipeline,
    ) -> Result<wgpu::RenderPipeline, crate::GpuError> {
        if key.generation != self.generation {
            return Err(crate::GpuError::DeviceMismatch {
                expected: self.generation,
                found: key.generation,
            });
        }
        // Creation is serialized: concurrent cold requests must not compile
        // the same pipeline twice. The factory must not reenter this registry.
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pipeline) = state.pipelines.get(&key).cloned() {
            state.stats.pipeline_registry_hits += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::PIPELINE_REGISTRY_HITS);
            return Ok(pipeline);
        }
        let pipeline = create();
        let mut evicted = Vec::new();
        state.pipelines.insert(key, pipeline.clone(), &mut evicted);
        // Keep evicted backend objects alive until a queue completion
        // callback drains this list: a command buffer may still use them.
        state
            .retired_pipelines
            .extend(evicted.into_iter().map(|pipeline| (0, pipeline)));
        state.stats.pipeline_registry_misses += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::PIPELINE_REGISTRY_MISSES);
        Ok(pipeline)
    }

    pub(crate) fn resource_layout(
        &self,
        key: u64,
        create: impl FnOnce() -> Arc<wgpu::BindGroupLayout>,
    ) -> Arc<wgpu::BindGroupLayout> {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(layout) = state.layouts.get(&key).cloned() {
            return layout;
        }
        let layout = create();
        let mut evicted = Vec::new();
        state.layouts.insert(key, layout.clone(), &mut evicted);
        state
            .retired_layouts
            .extend(evicted.into_iter().map(|layout| (0, layout)));
        layout
    }

    pub(crate) fn bind_pipeline_retirement(&self, submission: u64) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        for (retired_at, _) in &mut state.retired_pipelines {
            if *retired_at == 0 {
                *retired_at = submission;
            }
        }
        for (retired_at, _) in &mut state.retired_layouts {
            if *retired_at == 0 {
                *retired_at = submission;
            }
        }
    }

    pub fn retire(&self, submission: u64, bytes: u64) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retired
            .push(RetiredResource { submission, bytes });
    }

    pub fn collect_retired(&self, completed_submission: u64) -> u64 {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut bytes: u64 = 0;
        let retired = std::mem::take(&mut state.retired);
        let mut pending = Vec::with_capacity(retired.len());
        for item in retired {
            if item.submission <= completed_submission {
                bytes = bytes.saturating_add(item.bytes);
                state.stats.retired_resources += 1;
                nana_diagnostics::metric!(nana_diagnostics::framework::gpu::RETIRED_RESOURCES);
            } else {
                pending.push(item);
            }
        }
        state.retired = pending;
        state
            .retired_pipelines
            .retain(|(submission, _)| *submission == 0 || *submission > completed_submission);
        state
            .retired_layouts
            .retain(|(submission, _)| *submission == 0 || *submission > completed_submission);
        bytes
    }

    pub fn stats(&self) -> GpuPolicyStats {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).stats
    }
}

const MAX_PIPELINES: usize = 256;
const MAX_LAYOUTS: usize = 256;

fn texture_matches_key(texture: &GpuTexture, key: TransientResourceKey) -> bool {
    let raw = texture.raw();
    texture.generation() == key.generation
        && texture.format() == key.format
        && texture.size() == (key.width, key.height)
        && texture.usage().bits() == key.usage
        && raw.usage() == texture.usage().to_wgpu()
        && raw.dimension() == wgpu::TextureDimension::D2
        && raw.depth_or_array_layers() == key.depth_or_layers
        && raw.mip_level_count() == key.mip_levels
        && raw.sample_count() == key.sample_count
        && key.byte_size == 0
}

fn evict_transient(state: &mut PolicyState) {
    while state.transient_bytes > state.transient_budget {
        // HashMap iteration is deliberately unordered. Pick a stable key so
        // budget pressure produces reproducible evictions across runs while
        // still evicting only free pooled entries.
        let key = state
            .transient
            .keys()
            .chain(state.transient_buffers.keys())
            .chain(state.transient_textures.keys())
            .min_by_key(|key| transient_eviction_rank(**key))
            .copied();
        let Some(key) = key else { break };
        let (removed, empty) = if state.transient.contains_key(&key) {
            let ids = state.transient.get_mut(&key).expect("key exists");
            (ids.pop_front().is_some(), ids.is_empty())
        } else if let Some(buffers) = state.transient_buffers.get_mut(&key) {
            (buffers.pop_front().is_some(), buffers.is_empty())
        } else if let Some(textures) = state.transient_textures.get_mut(&key) {
            (textures.pop_front().is_some(), textures.is_empty())
        } else {
            (false, true)
        };
        if removed {
            state.transient_bytes = state.transient_bytes.saturating_sub(key.bytes_hint());
        }
        if empty {
            state.transient.remove(&key);
            state.transient_buffers.remove(&key);
            state.transient_textures.remove(&key);
        }
    }
}

fn transient_eviction_rank(key: TransientResourceKey) -> (u64, u64, u64, u64, u64, u64, u64, u64) {
    (
        key.generation.get(),
        key.byte_size,
        key.width as u64,
        key.height as u64,
        key.depth_or_layers as u64,
        key.mip_levels as u64,
        key.sample_count as u64,
        key.usage as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> GpuDeviceState {
        GpuDeviceState::new(DeviceGeneration::next())
    }

    #[test]
    fn stamped_cache_evicts_the_least_recently_used_eighth() {
        let mut cache = StampedCache::new(64);
        let mut evicted = Vec::new();
        for key in 0..64u32 {
            cache.insert(key, key, &mut evicted);
        }
        // Touch the oldest eight: the next eight become the oldest.
        for key in 0..8 {
            assert_eq!(cache.get(&key), Some(&key));
        }
        cache.insert(64, 64, &mut evicted);
        evicted.sort_unstable();
        assert_eq!(evicted, (8..16).collect::<Vec<_>>());
        assert_eq!(cache.len(), 57);
        for key in 0..8 {
            assert!(cache.get(&key).is_some());
        }
        // Re-inserting a present key never evicts.
        evicted.clear();
        for key in 57..64 {
            cache.insert(key, key, &mut evicted);
        }
        assert!(evicted.is_empty());
    }

    #[test]
    fn pools_and_registries_are_generation_scoped_and_counted() {
        let state = state();
        let key = TransientResourceKey::new(
            state.generation(),
            crate::GpuTextureFormat::RGBA8_UNORM,
            2,
            4,
            4,
            1,
        );
        assert!(state.acquire_transient(key).is_none());
        state.release_transient(key, 7);
        assert_eq!(state.acquire_transient(key), Some(7));
        let stats = state.stats();
        assert_eq!(stats.transient_pool_hits, 1);
        assert_eq!(stats.transient_pool_misses, 1);
    }

    #[test]
    fn retirement_waits_for_submission() {
        let state = state();
        state.retire(4, 10);
        state.retire(2, 7);
        assert_eq!(state.collect_retired(3), 7);
        assert_eq!(state.collect_retired(4), 10);
        assert_eq!(state.stats().retired_resources, 2);
    }

    #[test]
    fn transient_budget_evicts_whole_entries_and_keeps_keys_isolated() {
        let state = state();
        state.set_transient_budget(16);
        let small = TransientResourceKey::new(
            state.generation(),
            crate::GpuTextureFormat::RGBA8_UNORM,
            1,
            2,
            2,
            1,
        );
        let other_format = TransientResourceKey::new(
            state.generation(),
            crate::GpuTextureFormat::R8_UNORM,
            1,
            2,
            2,
            1,
        );
        state.release_transient(small, 1);
        state.release_transient(other_format, 2);
        let small_hit = state.acquire_transient(small).is_some();
        let other_hit = state.acquire_transient(other_format).is_some();
        assert_ne!(small_hit, other_hit);
    }

    #[test]
    fn frame_slots_stall_until_completion() {
        let state = state();
        state.set_frame_slot_count(1);
        let slot = state.acquire_frame_slot(4).unwrap();
        assert!(state.acquire_frame_slot(5).is_none());
        assert!(!state.release_frame_slot(slot, 3));
        assert!(state.release_frame_slot(slot, 4));
        assert!(state.acquire_frame_slot(5).is_some());
    }

    #[test]
    fn shrinking_slots_preserves_sparse_in_flight_slot() {
        let state = state();
        state.set_frame_slot_count(3);
        let first = state.acquire_frame_slot(1).unwrap();
        let _second = state.acquire_frame_slot(2).unwrap();
        let third = state.acquire_frame_slot(3).unwrap();
        assert!(state.release_frame_slot(first, 1));
        state.set_frame_slot_count(1);
        assert!(state.release_frame_slot(third, 3));
    }

    #[test]
    fn real_transient_buffer_pool_reuses_matching_descriptors() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .expect("GPU policy tests require a WGPU adapter");
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("GPU policy tests require a WGPU device");
        let state = GpuDeviceState::new(DeviceGeneration::next());
        let usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST;
        let key = TransientResourceKey::buffer(state.generation(), usage.bits(), 256);
        let buffer = state
            .acquire_transient_buffer(key, || {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("policy transient buffer"),
                    size: 256,
                    usage,
                    mapped_at_creation: false,
                })
            })
            .expect("matching buffer");
        state.release_transient_buffer(key, buffer);
        let reused = state
            .acquire_transient_buffer(key, || {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("unexpected cold buffer"),
                    size: 256,
                    usage,
                    mapped_at_creation: false,
                })
            })
            .expect("pooled buffer");
        drop(reused);
        assert_eq!(state.stats().transient_pool_hits, 1);
        assert_eq!(state.stats().transient_pool_misses, 1);
        let wrong = TransientResourceKey::buffer(state.generation(), usage.bits(), 128);
        assert_eq!(
            state
                .acquire_transient_buffer(wrong, || {
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("wrong-size buffer"),
                        size: 256,
                        usage,
                        mapped_at_creation: false,
                    })
                })
                .expect_err("descriptor mismatch"),
            crate::GpuError::TransientDescriptorMismatch
        );
    }
}
