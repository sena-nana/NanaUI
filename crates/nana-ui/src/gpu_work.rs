//! Observed GPU work for Issue #8 counters.
//!
//! Values are recorded only on a real encode/submit path. CPU-only Runtime
//! drains never construct this sink, so WorkCounters GPU fields stay `None`.

use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, RwLockReadGuard};
use std::time::Duration;

use nana_gpu::{GpuContext, GpuDeviceState, TransientResourceKey};
use nana_ui_core::{FrameStage, GpuWorkObservation};
use nana_ui_runtime::FrameProfiler;

pub(crate) type TransientBufferRegistry = Arc<Mutex<Vec<(TransientResourceKey, wgpu::Buffer)>>>;

#[derive(Debug)]
pub(crate) struct ManagedBuffer(Option<wgpu::Buffer>);

impl ManagedBuffer {
    pub(crate) fn new(buffer: wgpu::Buffer) -> Self {
        Self(Some(buffer))
    }

    fn take(&mut self) -> wgpu::Buffer {
        self.0.take().expect("managed buffer slot is populated")
    }

    fn put(&mut self, buffer: wgpu::Buffer) {
        debug_assert!(self.0.is_none());
        self.0 = Some(buffer);
    }
}

impl Deref for ManagedBuffer {
    type Target = wgpu::Buffer;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("managed buffer slot is populated")
    }
}

impl DerefMut for ManagedBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("managed buffer slot is populated")
    }
}

/// Shared accumulator for one Scene/WGPU frame.
#[derive(Debug, Default)]
pub struct GpuWorkSink {
    work: RefCell<GpuWorkObservation>,
    policy: Option<GpuDeviceState>,
    gpu: Option<GpuContext>,
    transient_registry: Option<TransientBufferRegistry>,
}

impl GpuWorkSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_policy(policy: &GpuDeviceState) -> Self {
        Self {
            work: RefCell::new(GpuWorkObservation::default()),
            policy: Some(policy.clone()),
            gpu: None,
            transient_registry: None,
        }
    }

    pub fn with_gpu(gpu: &GpuContext) -> Self {
        Self {
            work: RefCell::new(GpuWorkObservation::default()),
            policy: Some(gpu.policy().clone()),
            gpu: Some(gpu.clone()),
            transient_registry: None,
        }
    }

    pub(crate) fn with_gpu_registry(
        gpu: &GpuContext,
        transient_registry: Option<TransientBufferRegistry>,
    ) -> Self {
        Self {
            work: RefCell::new(GpuWorkObservation::default()),
            policy: Some(gpu.policy().clone()),
            gpu: Some(gpu.clone()),
            transient_registry,
        }
    }

    /// Hold the host submission lock while a renderer maps a queue staging
    /// view and records the copy that consumes it. Surface reconfiguration
    /// takes the write side of the same lock.
    pub(crate) fn lock_submission(&self) -> Option<RwLockReadGuard<'_, ()>> {
        self.gpu
            .as_ref()
            .map(nana_gpu::__framework::lock_submission)
    }

    /// Replace a persistent renderer buffer through the policy pool. The old
    /// buffer is held by the current frame and returned only after submission
    /// completion (or immediately when the frame is discarded).
    pub(crate) fn replace_buffer(
        &self,
        device: &wgpu::Device,
        slot: &mut ManagedBuffer,
        size: u64,
        usage: wgpu::BufferUsages,
        label: &'static str,
    ) {
        // The slot owns the buffer through Option, so a failed allocation or
        // unwind can never leave an uninitialized WGPU object behind.
        let old = slot.take();
        let Some(gpu) = &self.gpu else {
            let next = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            });
            slot.put(next);
            drop(old);
            return;
        };
        let Some(registry) = &self.transient_registry else {
            // A sink not bound to a FrameContext cannot prove queue
            // completion, so it must not acquire or recycle a pooled buffer.
            let next = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            });
            slot.put(next);
            drop(old);
            return;
        };
        let key = TransientResourceKey::buffer(gpu.generation(), usage.bits(), size);
        let next = nana_gpu::__framework::acquire_transient_buffer(
            gpu.policy(),
            &key,
            device,
            label,
            usage,
        );
        let next = if let Ok(next) = next {
            registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((key, old));
            next
        } else {
            let next = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            });
            drop(old);
            next
        };
        slot.put(next);
    }

    /// Write a dynamic buffer through the per-device staging policy while
    /// preserving the renderer's target buffer contract. The target write is
    /// still a compatibility copy until every renderer consumes arena offsets.
    pub(crate) fn write_buffer(
        &self,
        queue: &wgpu::Queue,
        target: &wgpu::Buffer,
        offset: u64,
        bytes: &[u8],
    ) {
        let padded_len = (bytes.len() + 3) & !3;
        let mut padded = Vec::new();
        let write_bytes = if padded_len == bytes.len() {
            bytes
        } else {
            padded.resize(padded_len, 0);
            padded[..bytes.len()].copy_from_slice(bytes);
            &padded
        };
        if let Some(gpu) = &self.gpu {
            let _submission = nana_gpu::__framework::lock_submission(gpu);
            if !nana_gpu::__framework::stage_upload(
                gpu,
                queue,
                write_bytes,
                wgpu::COPY_BUFFER_ALIGNMENT,
            ) {
                gpu.policy().record_upload(write_bytes.len() as u64);
            }
            queue.write_buffer(target, offset, write_bytes);
            self.work.borrow_mut().record_upload(bytes.len());
        } else {
            queue.write_buffer(target, offset, write_bytes);
            self.work.borrow_mut().record_upload(bytes.len());
        }
    }

    /// Upload texture bytes while sharing the policy submission guard and
    /// staging accounting. The texture copy remains an explicit WGPU copy
    /// because texture layouts are renderer-specific.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_texture(
        &self,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        origin: [u32; 3],
        bytes: &[u8],
        bytes_per_row: u32,
        rows_per_image: u32,
        extent: [u32; 3],
    ) {
        let padded_len = (bytes.len() + 3) & !3;
        let mut padded = Vec::new();
        let stage_bytes = if padded_len == bytes.len() {
            bytes
        } else {
            padded.resize(padded_len, 0);
            padded[..bytes.len()].copy_from_slice(bytes);
            &padded
        };
        if let Some(gpu) = &self.gpu {
            let _submission = nana_gpu::__framework::lock_submission(gpu);
            if !nana_gpu::__framework::stage_upload(
                gpu,
                queue,
                stage_bytes,
                wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64,
            ) {
                gpu.policy().record_upload(stage_bytes.len() as u64);
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: origin[0],
                        y: origin[1],
                        z: origin[2],
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(rows_per_image),
                },
                wgpu::Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: extent[2],
                },
            );
            self.work.borrow_mut().record_upload(bytes.len());
        } else {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: origin[0],
                        y: origin[1],
                        z: origin[2],
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(rows_per_image),
                },
                wgpu::Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: extent[2],
                },
            );
            self.work.borrow_mut().record_upload(bytes.len());
        }
    }

    pub fn record_upload(&self, bytes: usize) {
        self.work.borrow_mut().record_upload(bytes);
        if let Some(policy) = &self.policy
            && policy.reserve_upload(bytes as u64, 256).is_none()
        {
            policy.record_upload(bytes as u64);
        }
    }

    pub fn record_realloc(&self) {
        self.work.borrow_mut().record_realloc();
        if let Some(policy) = &self.policy {
            policy.record_reallocation();
        }
    }

    pub fn record_batch_rebuild(&self) {
        self.work.borrow_mut().record_batch_rebuild();
    }

    pub fn record_draw_batch(&self) {
        self.work.borrow_mut().record_draw_batch();
    }

    pub fn record_draw_call(&self) {
        self.work.borrow_mut().record_draw_call();
    }

    pub fn snapshot(&self) -> GpuWorkObservation {
        *self.work.borrow()
    }
}

/// Stage timings a GPU host measured while encoding/submitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuStageTimings {
    pub batch: Duration,
    pub gpu_upload: Duration,
    pub encode: Duration,
    pub submit: Duration,
}

impl GpuStageTimings {
    /// Fold host-measured GPU stages onto a FrameProfiler.
    ///
    /// Runtime-only hosts must call [`FrameProfiler::mark_runtime_unsupported`]
    /// instead and never invent these durations.
    pub fn record_on(self, profiler: &mut FrameProfiler) {
        profiler.record(FrameStage::Batch, self.batch);
        profiler.record(FrameStage::GpuUpload, self.gpu_upload);
        profiler.record(FrameStage::Encode, self.encode);
        profiler.record(FrameStage::Submit, self.submit);
    }
}
