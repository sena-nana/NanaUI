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

type RealizationKey = (u64, u64, DeviceGeneration, GpuTextureFormat, u32, u32, u32);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuPolicyStats {
    pub upload_bytes: u64,
    pub buffer_reallocations: u64,
    pub transient_pool_hits: u64,
    pub transient_pool_misses: u64,
    pub pipeline_registry_hits: u64,
    pub pipeline_registry_misses: u64,
    pub frame_slot_stalls: u64,
    pub retired_resources: u64,
    pub realization_hits: u64,
    pub realization_misses: u64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadReservation {
    pub offset: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameSlotId(pub u32);

#[derive(Debug)]
struct UploadArena {
    capacity: u64,
    cursor: u64,
    backing: Option<wgpu::Buffer>,
}

impl UploadArena {
    fn new(capacity: u64) -> Self {
        Self {
            capacity: capacity.max(1),
            cursor: 0,
            backing: None,
        }
    }

    fn reserve(&mut self, size: u64, alignment: u64) -> Option<UploadReservation> {
        let alignment = alignment.max(1);
        let aligned =
            (self.cursor.checked_add(alignment - 1)? / alignment).checked_mul(alignment)?;
        let end = aligned.checked_add(size)?;
        if end > self.capacity {
            return None;
        }
        self.cursor = end;
        Some(UploadReservation {
            offset: aligned,
            size,
        })
    }

    fn required_end(&self, size: u64, alignment: u64) -> Option<u64> {
        let alignment = alignment.max(1);
        let aligned = self
            .cursor
            .checked_add(alignment - 1)?
            .checked_div(alignment)?
            .checked_mul(alignment)?;
        aligned.checked_add(size)
    }

    fn reset(&mut self) {
        self.cursor = 0;
    }

    fn ensure_backing(&mut self, device: Option<&wgpu::Device>) -> Option<wgpu::Buffer> {
        let device = device?;
        if self.backing.is_none() {
            self.backing = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nana-gpu upload arena"),
                size: self.capacity,
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        self.backing.clone()
    }

    fn grow_backing(&mut self, device: Option<&wgpu::Device>, needed: u64) -> Option<wgpu::Buffer> {
        let device = device?;
        let old = self.backing.take();
        while self.capacity < needed {
            self.capacity = self.capacity.saturating_mul(2).max(needed);
        }
        self.backing = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-gpu upload arena"),
            size: self.capacity,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        old
    }
}

#[derive(Debug)]
struct RetiredResource {
    submission: u64,
    bytes: u64,
}

#[derive(Debug)]
struct PolicyState {
    upload: UploadArena,
    upload_limit: u64,
    device: Option<wgpu::Device>,
    transient_budget: u64,
    transient_bytes: u64,
    transient: HashMap<TransientResourceKey, VecDeque<u64>>,
    transient_buffers: HashMap<TransientResourceKey, VecDeque<wgpu::Buffer>>,
    transient_textures: HashMap<TransientResourceKey, VecDeque<GpuTexture>>,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    pipeline_order: VecDeque<PipelineKey>,
    retired_pipelines: Vec<(u64, wgpu::RenderPipeline)>,
    layouts: HashMap<u64, Arc<wgpu::BindGroupLayout>>,
    layout_order: VecDeque<u64>,
    retired_layouts: Vec<(u64, Arc<wgpu::BindGroupLayout>)>,
    retired_realizations: Vec<(u64, GpuTexture)>,
    retired_uploads: Vec<(u64, wgpu::Buffer)>,
    realizations: HashMap<RealizationKey, GpuTexture>,
    realization_order: VecDeque<RealizationKey>,
    retired: Vec<RetiredResource>,
    frame_slots: Vec<Option<u64>>,
    stats: GpuPolicyStats,
}

/// Shared policy state for one `DeviceGeneration`.
#[derive(Clone, Debug)]
pub struct GpuDeviceState {
    generation: DeviceGeneration,
    inner: Arc<Mutex<PolicyState>>,
}

impl GpuDeviceState {
    #[cfg(test)]
    pub(crate) fn new(generation: DeviceGeneration) -> Self {
        Self::new_inner_with_limit(generation, None, u64::MAX)
    }

    pub(crate) fn new_with_device(generation: DeviceGeneration, device: &wgpu::Device) -> Self {
        Self::new_inner_with_limit(
            generation,
            Some(device.clone()),
            device.limits().max_buffer_size,
        )
    }

    fn new_inner_with_limit(
        generation: DeviceGeneration,
        device: Option<wgpu::Device>,
        upload_limit: u64,
    ) -> Self {
        Self {
            generation,
            inner: Arc::new(Mutex::new(PolicyState {
                upload: UploadArena::new((4 * 1024 * 1024).min(upload_limit).max(1)),
                upload_limit,
                device,
                transient_budget: 64 * 1024 * 1024,
                transient_bytes: 0,
                transient: HashMap::new(),
                transient_buffers: HashMap::new(),
                transient_textures: HashMap::new(),
                pipelines: HashMap::new(),
                pipeline_order: VecDeque::new(),
                retired_pipelines: Vec::new(),
                layouts: HashMap::new(),
                layout_order: VecDeque::new(),
                retired_layouts: Vec::new(),
                retired_realizations: Vec::new(),
                retired_uploads: Vec::new(),
                realizations: HashMap::new(),
                realization_order: VecDeque::new(),
                retired: Vec::new(),
                frame_slots: vec![None, None, None],
                stats: GpuPolicyStats::default(),
            })),
        }
    }

    pub fn generation(&self) -> DeviceGeneration {
        self.generation
    }

    pub fn set_upload_capacity(&self, capacity: u64) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = state.upload.backing.take() {
            state.retired_uploads.push((0, old));
        }
        state.upload.capacity = capacity.max(1);
        state.upload.reset();
    }

    pub fn set_transient_budget(&self, bytes: u64) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.transient_budget = bytes;
        evict_transient(&mut state);
    }

    /// Reset the upload arena when no frame slot is in flight. Normal
    /// [`crate::FrameContext`] recording acquires a slot automatically; this
    /// method is for explicit upload-arena users between completed frames.
    pub fn begin_frame(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if state.frame_slots.iter().all(Option::is_none) {
            state.upload.reset();
        }
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
            .resize(count.max(1).max(occupied).max(last_occupied), None);
    }

    pub fn acquire_frame_slot(&self, submission: u64) -> Option<FrameSlotId> {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((index, slot)) = state
            .frame_slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            *slot = Some(submission);
            return Some(FrameSlotId(index as u32));
        }
        state.stats.frame_slot_stalls += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::FRAME_SLOT_STALLS);
        None
    }

    pub fn release_frame_slot(&self, slot: FrameSlotId, completed_submission: u64) -> bool {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(owner) = state.frame_slots.get_mut(slot.0 as usize) else {
            return false;
        };
        if owner.is_some_and(|submission| submission <= completed_submission) {
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
        if *owner == Some(token) {
            *owner = None;
            true
        } else {
            false
        }
    }

    pub fn reserve_upload(&self, size: u64, alignment: u64) -> Option<UploadReservation> {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let device = state.device.clone();
        let _ = state.upload.ensure_backing(device.as_ref());
        let mut reservation = state.upload.reserve(size, alignment);
        if reservation.is_none()
            && let Some(needed) = state.upload.required_end(size, alignment)
            && state.device.is_some()
            && needed <= state.upload_limit
        {
            let old = state.upload.grow_backing(device.as_ref(), needed);
            if let Some(old) = old {
                state.retired_uploads.push((0, old));
            }
            state.upload.reset();
            reservation = state.upload.reserve(size, alignment);
            state.stats.buffer_reallocations = state.stats.buffer_reallocations.saturating_add(1);
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::BUFFER_REALLOCATIONS);
        }
        if reservation.is_some() {
            state.stats.upload_bytes = state.stats.upload_bytes.saturating_add(size);
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::UPLOAD_BYTES, size);
        } else {
            state.stats.buffer_reallocations = state.stats.buffer_reallocations.saturating_add(1);
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::BUFFER_REALLOCATIONS);
        }
        reservation
    }

    pub fn record_upload(&self, bytes: u64) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.stats.upload_bytes = state.stats.upload_bytes.saturating_add(bytes);
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::UPLOAD_BYTES, bytes);
    }

    /// Stage bytes into the policy-owned upload backing. Callers consume the
    /// returned reservation and buffer from their frame encoder.
    pub(crate) fn stage_upload(
        &self,
        queue: &wgpu::Queue,
        bytes: &[u8],
        alignment: u64,
    ) -> Option<(UploadReservation, wgpu::Buffer)> {
        let reservation = self.reserve_upload(bytes.len() as u64, alignment)?;
        let backing = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upload
            .backing
            .clone()?;
        queue.write_buffer(&backing, reservation.offset, bytes);
        Some((reservation, backing))
    }

    pub fn record_reallocation(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.stats.buffer_reallocations = state.stats.buffer_reallocations.saturating_add(1);
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::BUFFER_REALLOCATIONS);
    }

    pub(crate) fn upload_backing_size(&self) -> Option<u64> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upload
            .backing
            .as_ref()
            .map(wgpu::Buffer::size)
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
            state.pipeline_order.retain(|entry| *entry != key);
            state.pipeline_order.push_back(key);
            state.stats.pipeline_registry_hits += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::PIPELINE_REGISTRY_HITS);
            return Ok(pipeline);
        }
        let pipeline = create();
        const MAX_PIPELINES: usize = 256;
        if state.pipelines.len() == MAX_PIPELINES
            && let Some(oldest) = state.pipeline_order.pop_front()
            && let Some(pipeline) = state.pipelines.remove(&oldest)
        {
            // Keep the backend object alive until at least one queue
            // completion callback drains this list. A command buffer may
            // still reference an evicted pipeline.
            state.retired_pipelines.push((0, pipeline));
        }
        state.pipelines.insert(key, pipeline.clone());
        state.pipeline_order.push_back(key);
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
            state.layout_order.retain(|entry| *entry != key);
            state.layout_order.push_back(key);
            return layout;
        }
        let layout = create();
        const MAX_LAYOUTS: usize = 256;
        if state.layouts.len() == MAX_LAYOUTS
            && let Some(oldest) = state.layout_order.pop_front()
            && let Some(layout) = state.layouts.remove(&oldest)
        {
            state.retired_layouts.push((0, layout));
        }
        state.layouts.insert(key, layout.clone());
        state.layout_order.push_back(key);
        layout
    }

    pub fn realize_texture(
        &self,
        resource: u64,
        version: u64,
        texture: GpuTexture,
    ) -> Result<(GpuTexture, bool), crate::GpuError> {
        if texture.generation() != self.generation {
            return Err(crate::GpuError::DeviceMismatch {
                expected: self.generation,
                found: texture.generation(),
            });
        }
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let key = (
            resource,
            version,
            self.generation,
            texture.format(),
            texture.size().0,
            texture.size().1,
            texture.usage().bits(),
        );
        if let Some(cached) = state.realizations.get(&key).cloned() {
            state.realization_order.retain(|entry| *entry != key);
            state.realization_order.push_back(key);
            state.stats.realization_hits += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::REALIZATION_HITS);
            Ok((cached, true))
        } else {
            const MAX_REALIZATIONS: usize = 4096;
            if state.realizations.len() == MAX_REALIZATIONS
                && let Some(oldest) = state.realization_order.pop_front()
                && let Some(texture) = state.realizations.remove(&oldest)
            {
                state.retired_realizations.push((0, texture));
            }
            state.realizations.insert(key, texture.clone());
            state.realization_order.push_back(key);
            state.stats.realization_misses += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::gpu::REALIZATION_MISSES);
            Ok((texture, false))
        }
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
        for (retired_at, _) in &mut state.retired_realizations {
            if *retired_at == 0 {
                *retired_at = submission;
            }
        }
        for (retired_at, _) in &mut state.retired_uploads {
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
        state
            .retired_realizations
            .retain(|(submission, _)| *submission == 0 || *submission > completed_submission);
        state
            .retired_uploads
            .retain(|(submission, _)| *submission == 0 || *submission > completed_submission);
        bytes
    }

    pub fn stats(&self) -> GpuPolicyStats {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).stats
    }
}

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
    fn upload_arena_reuses_after_frame_reset() {
        let state = state();
        state.set_upload_capacity(64);
        assert_eq!(
            state.reserve_upload(8, 16),
            Some(UploadReservation { offset: 0, size: 8 })
        );
        assert!(state.reserve_upload(60, 1).is_none());
        state.begin_frame();
        assert_eq!(state.reserve_upload(8, 16).unwrap().offset, 0);
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
        assert_eq!(state.stats().frame_slot_stalls, 1);
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
        let state = GpuDeviceState::new_with_device(DeviceGeneration::next(), &device);
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
