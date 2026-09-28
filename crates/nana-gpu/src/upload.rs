//! Frame uploads through a per-device staging ring.
//!
//! Renderers used to hand every small write to `queue.write_buffer` /
//! `write_texture`. Each call pays for a staging allocation of its own inside
//! WGPU — tens of thousands of instructions on Metal, far more than the bytes
//! — so a frame that touches a hundred buffers paid a hundred times.
//!
//! Here writes are appended to a CPU batch owned by the [`crate::FrameContext`]
//! (or, outside a frame, to the device's pending batch). When the frame is
//! submitted, both batches are copied into one mapped chunk from the
//! [`UploadRing`] in a single `memcpy`, an upload command buffer records one
//! copy per destination run, and that command buffer is submitted in the same
//! `queue.submit` ahead of the frame's own. The semantics are the queue's:
//! every write lands before the frame's commands, and in program order.
//!
//! A mapped buffer cannot be used by a submission, so the ring is a pool of
//! whole chunks rather than one buffer with a moving head: a chunk is used by
//! exactly one submission and comes back — mapped again — through the map
//! callback that submission registers.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::GpuDeviceState;

/// Row pitch a buffer-to-texture copy requires.
const ROW_ALIGNMENT: u64 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
/// Texture copies start at a multiple of this in the chunk; D3D12's placement
/// alignment, and a multiple of every block size.
const TEXTURE_OFFSET_ALIGNMENT: u64 = 512;
/// Smallest chunk the ring creates.
const MIN_CHUNK: u64 = 1 << 20;
/// Chunks kept pooled at most, in bytes. Chunks returned beyond it are freed.
const RING_BUDGET: u64 = 32 << 20;
/// How long a flush waits for an in-flight chunk before allocating past the
/// budget instead.
const RING_WAIT: Duration = Duration::from_millis(250);

struct BufferCopy {
    source: u64,
    target: wgpu::Buffer,
    offset: u64,
    size: u64,
}

struct TextureCopy {
    source: u64,
    bytes_per_row: u32,
    rows_per_image: u32,
    target: wgpu::Texture,
    mip_level: u32,
    origin: wgpu::Origin3d,
    extent: wgpu::Extent3d,
}

/// Writes waiting for a submission, with their bytes packed in order.
#[derive(Default)]
pub(crate) struct UploadBatch {
    bytes: Vec<u8>,
    buffers: Vec<BufferCopy>,
    textures: Vec<TextureCopy>,
    writes: u64,
}

impl UploadBatch {
    fn is_empty(&self) -> bool {
        self.buffers.is_empty() && self.textures.is_empty()
    }

    fn align_to(&mut self, alignment: u64) -> u64 {
        let at = (self.bytes.len() as u64).next_multiple_of(alignment);
        self.bytes.resize(at as usize, 0);
        at
    }

    /// Queue `bytes` for `target` at `offset`. Like `queue.write_buffer`,
    /// `offset` must be 4-aligned; a length that is not is padded with zeros.
    pub(crate) fn write_buffer(&mut self, target: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let source = self.align_to(wgpu::COPY_BUFFER_ALIGNMENT);
        self.bytes.extend_from_slice(bytes);
        let size = (bytes.len() as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        self.bytes.resize((source + size) as usize, 0);
        self.writes += 1;
        // Consecutive writes to consecutive ranges of one buffer are one copy.
        if let Some(last) = self.buffers.last_mut()
            && last.target == *target
            && last.offset + last.size == offset
            && last.source + last.size == source
        {
            last.size += size;
            return;
        }
        self.buffers.push(BufferCopy {
            source,
            target: target.clone(),
            offset,
            size,
        });
    }

    /// Queue a 2D region of `target`, `rows` rows of `row_bytes` each, read
    /// with `bytes_per_row` stride from `bytes`. Rows are repacked to the
    /// copy's 256-byte pitch here.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_texture(
        &mut self,
        target: &wgpu::Texture,
        mip_level: u32,
        origin: wgpu::Origin3d,
        extent: wgpu::Extent3d,
        bytes: &[u8],
        bytes_per_row: u32,
        rows_per_image: u32,
        row_bytes: u32,
    ) {
        let height = extent.height.max(1) as usize;
        let image_rows = rows_per_image.max(extent.height) as usize;
        self.write_texture_rows(target, mip_level, origin, extent, row_bytes, |row, out| {
            // Layer by layer, each `rows_per_image` rows apart in `bytes`.
            let source_row = (row / height) * image_rows + row % height;
            let start = source_row * bytes_per_row as usize;
            out.copy_from_slice(&bytes[start..start + out.len()]);
        });
    }

    /// Queue a 2D region of `target` whose rows `fill(row, dst)` writes
    /// straight into the batch, so callers that compose rows (glyph gutters)
    /// need no scratch of their own.
    pub(crate) fn write_texture_rows(
        &mut self,
        target: &wgpu::Texture,
        mip_level: u32,
        origin: wgpu::Origin3d,
        extent: wgpu::Extent3d,
        row_bytes: u32,
        mut fill: impl FnMut(usize, &mut [u8]),
    ) {
        if extent.width == 0 || extent.height == 0 || extent.depth_or_array_layers == 0 {
            return;
        }
        let pitch = u64::from(row_bytes).next_multiple_of(ROW_ALIGNMENT);
        let rows = extent.height as usize * extent.depth_or_array_layers as usize;
        let source = self.align_to(TEXTURE_OFFSET_ALIGNMENT);
        self.bytes
            .resize((source + pitch * rows as u64) as usize, 0);
        for row in 0..rows {
            let start = (source + pitch * row as u64) as usize;
            fill(row, &mut self.bytes[start..start + row_bytes as usize]);
        }
        self.writes += 1;
        self.textures.push(TextureCopy {
            source,
            bytes_per_row: pitch as u32,
            rows_per_image: extent.height,
            target: target.clone(),
            mip_level,
            origin,
            extent,
        });
    }

    /// Move everything in `other` behind what this batch already holds.
    fn append(&mut self, other: &mut UploadBatch) {
        if other.is_empty() {
            return;
        }
        let base = self.align_to(TEXTURE_OFFSET_ALIGNMENT);
        self.bytes.extend_from_slice(&other.bytes);
        self.buffers.extend(other.buffers.drain(..).map(|mut copy| {
            copy.source += base;
            copy
        }));
        self.textures
            .extend(other.textures.drain(..).map(|mut copy| {
                copy.source += base;
                copy
            }));
        self.writes += other.writes;
        other.clear();
    }

    fn clear(&mut self) {
        self.bytes.clear();
        self.buffers.clear();
        self.textures.clear();
        self.writes = 0;
    }
}

/// A frame's upload batch, shared with the renderers recording into it.
pub type FrameUploads = Arc<Mutex<UploadBatch>>;

/// Mapped staging chunks for one device.
pub(crate) struct UploadRing {
    state: Mutex<RingState>,
}

#[derive(Default)]
struct RingState {
    /// Mapped and ready.
    free: Vec<wgpu::Buffer>,
    /// Bytes of every chunk the ring owns, free or in flight.
    owned: u64,
    /// Emptied batches kept for their capacity.
    spare: Vec<UploadBatch>,
}

impl UploadRing {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RingState::default()),
        })
    }

    /// An empty batch, reusing the capacity of an earlier frame's.
    pub(crate) fn batch(&self) -> UploadBatch {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .spare
            .pop()
            .unwrap_or_default()
    }

    fn recycle(&self, mut batch: UploadBatch) {
        batch.clear();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.spare.len() < 4 {
            state.spare.push(batch);
        }
    }

    fn take_free(&self, size: u64) -> Option<wgpu::Buffer> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let index = state
            .free
            .iter()
            .enumerate()
            .filter(|(_, chunk)| chunk.size() >= size)
            .min_by_key(|(_, chunk)| chunk.size())
            .map(|(index, _)| index)?;
        Some(state.free.swap_remove(index))
    }

    /// A mapped chunk of at least `size` bytes.
    fn acquire(
        self: &Arc<Self>,
        device: &wgpu::Device,
        policy: &GpuDeviceState,
        latest: Option<wgpu::SubmissionIndex>,
        size: u64,
    ) -> wgpu::Buffer {
        if let Some(chunk) = self.take_free(size) {
            return chunk;
        }
        // Chunks come back through map callbacks, which run in poll.
        let _ = device.poll(wgpu::PollType::Poll);
        if let Some(chunk) = self.take_free(size) {
            return chunk;
        }
        let chunk_size = size.max(MIN_CHUNK).next_power_of_two();
        let over_budget = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.owned + chunk_size > RING_BUDGET
        };
        if over_budget && let Some(latest) = latest {
            policy.record_ring_wait();
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: Some(latest),
                timeout: Some(RING_WAIT),
            });
            if let Some(chunk) = self.take_free(size) {
                return chunk;
            }
        }
        policy.record_ring_allocation();
        self.state.lock().unwrap_or_else(|e| e.into_inner()).owned += chunk_size;
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-gpu upload chunk"),
            size: chunk_size,
            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        })
    }

    /// A chunk that came back mapped. Kept while the pool is within budget.
    fn release(&self, chunk: wgpu::Buffer, mapped: bool) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let pooled: u64 = state.free.iter().map(wgpu::Buffer::size).sum();
        if mapped && pooled + chunk.size() <= RING_BUDGET {
            state.free.push(chunk);
        } else {
            state.owned = state.owned.saturating_sub(chunk.size());
        }
    }
}

/// Build the upload command buffer for `batch`, or `None` when it is empty.
/// The chunk returns to `ring` when the submission that carries the command
/// buffer completes.
pub(crate) fn record(
    ring: &Arc<UploadRing>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    policy: &GpuDeviceState,
    latest: Option<wgpu::SubmissionIndex>,
    batch: UploadBatch,
) -> Option<wgpu::CommandBuffer> {
    if batch.is_empty() {
        ring.recycle(batch);
        return None;
    }
    let size = (batch.bytes.len() as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
    let chunk = ring.acquire(device, policy, latest, size);
    match chunk.slice(..size).get_mapped_range_mut() {
        Ok(mut view) => view
            .slice(..batch.bytes.len())
            .copy_from_slice(&batch.bytes),
        Err(_) => {
            // A chunk that is not mapped (a lost device, or a mapping that
            // failed) cannot stage anything. The writes still land, one
            // queue call each.
            ring.release(chunk, false);
            write_directly(queue, &batch);
            ring.recycle(batch);
            return None;
        }
    }
    chunk.unmap();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-gpu frame uploads"),
    });
    for copy in &batch.buffers {
        encoder.copy_buffer_to_buffer(&chunk, copy.source, &copy.target, copy.offset, copy.size);
    }
    for copy in &batch.textures {
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &chunk,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: copy.source,
                    bytes_per_row: Some(copy.bytes_per_row),
                    rows_per_image: Some(copy.rows_per_image),
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture: &copy.target,
                mip_level: copy.mip_level,
                origin: copy.origin,
                aspect: wgpu::TextureAspect::All,
            },
            copy.extent,
        );
    }
    policy.record_flush(
        batch.writes,
        (batch.buffers.len() + batch.textures.len()) as u64,
    );
    let returning = Arc::clone(ring);
    let mapped = chunk.clone();
    encoder.map_buffer_on_submit(&chunk, wgpu::MapMode::Write, .., move |result| {
        returning.release(mapped, result.is_ok());
    });
    ring.recycle(batch);
    Some(encoder.finish())
}

fn write_directly(queue: &wgpu::Queue, batch: &UploadBatch) {
    for copy in &batch.buffers {
        let start = copy.source as usize;
        queue.write_buffer(
            &copy.target,
            copy.offset,
            &batch.bytes[start..start + copy.size as usize],
        );
    }
    for copy in &batch.textures {
        let rows = copy.extent.height as usize * copy.extent.depth_or_array_layers as usize;
        let start = copy.source as usize;
        let end = start + copy.bytes_per_row as usize * rows;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &copy.target,
                mip_level: copy.mip_level,
                origin: copy.origin,
                aspect: wgpu::TextureAspect::All,
            },
            &batch.bytes[start..end.min(batch.bytes.len())],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(copy.bytes_per_row),
                rows_per_image: Some(copy.rows_per_image),
            },
            copy.extent,
        );
    }
}

/// Append `from` to `into`, leaving `from` empty.
pub(crate) fn merge(into: &mut UploadBatch, from: &mut UploadBatch) {
    into.append(from);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> wgpu::Device {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .expect("upload tests require a WGPU adapter");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("upload tests require a WGPU device")
            .0
    }

    fn buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    #[test]
    fn adjacent_writes_to_one_buffer_are_one_copy_and_lengths_pad() {
        let device = device();
        let target = buffer(&device, 64);
        let other = buffer(&device, 64);
        let mut batch = UploadBatch::default();
        batch.write_buffer(&target, 0, &[1, 2, 3, 4]);
        batch.write_buffer(&target, 4, &[5, 6, 7, 8]);
        batch.write_buffer(&target, 16, &[9]);
        batch.write_buffer(&other, 0, &[1; 8]);
        assert_eq!(batch.writes, 4);
        assert_eq!(batch.buffers.len(), 3);
        assert_eq!(batch.buffers[0].size, 8);
        assert_eq!(batch.buffers[1].size, 4, "a 1-byte write pads to 4");
        assert_eq!(batch.bytes.len() % 4, 0);
    }

    #[test]
    fn texture_rows_repack_to_the_copy_pitch_at_an_aligned_offset() {
        let device = device();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 3,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut batch = UploadBatch::default();
        batch.write_buffer(&buffer(&device, 16), 0, &[0; 4]);
        batch.write_texture(
            &texture,
            0,
            wgpu::Origin3d::ZERO,
            wgpu::Extent3d {
                width: 3,
                height: 2,
                depth_or_array_layers: 1,
            },
            &[1, 2, 3, 0xAA, 4, 5, 6],
            4,
            2,
            3,
        );
        let copy = &batch.textures[0];
        assert_eq!(copy.source % TEXTURE_OFFSET_ALIGNMENT, 0);
        assert_eq!(copy.bytes_per_row, 256);
        let at = copy.source as usize;
        assert_eq!(&batch.bytes[at..at + 3], &[1, 2, 3]);
        assert_eq!(&batch.bytes[at + 256..at + 259], &[4, 5, 6]);
    }

    #[test]
    fn appending_rebases_the_source_offsets() {
        let device = device();
        let target = buffer(&device, 64);
        let mut first = UploadBatch::default();
        first.write_buffer(&target, 0, &[1; 4]);
        let mut second = UploadBatch::default();
        second.write_buffer(&target, 32, &[2; 4]);
        merge(&mut first, &mut second);
        assert!(second.is_empty());
        assert_eq!(first.buffers.len(), 2);
        let rebased = &first.buffers[1];
        assert_eq!(rebased.source % TEXTURE_OFFSET_ALIGNMENT, 0);
        assert_eq!(
            &first.bytes[rebased.source as usize..rebased.source as usize + 4],
            &[2; 4]
        );
        assert_eq!(first.writes, 2);
    }
}
