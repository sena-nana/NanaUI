//! Raw backend access for NanaUI's own crates.
//!
//! Not part of the contract and not for applications: renderers and hosts
//! outside the framework use the `wgpu-interop` feature, which the boundary
//! check keeps auditable. This module exists because Cargo features unify —
//! if the framework reached WGPU through `wgpu-interop`, every consumer would
//! silently get the escape hatch too.

use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use crate::{
    FrameContext, GpuContext, GpuDeviceLost, GpuRenderTarget, GpuSubmission, GpuTexture,
    GpuTextureFormat, GpuTextureUsages,
};

/// Framework-only realization helper for renderers that already own the raw
/// device. Public consumers must use `GpuContext::create_resource_layout`.
pub fn logical_layout(
    device: &wgpu::Device,
    generation: crate::DeviceGeneration,
    table: &crate::ResourceTable,
) -> Result<crate::GpuResourceLayout, crate::GpuError> {
    crate::realization::create_resource_layout_raw(device, generation, table)
}

/// Adopt a device the framework requested. Installs the device-lost callback
/// that feeds [`GpuContext::is_lost`].
pub fn adopt_tracking_loss(
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
) -> GpuContext {
    GpuContext::adopt(adapter, device, queue, true)
}

/// Adopt a device someone else owns, including its loss callback.
pub fn adopt(adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> GpuContext {
    GpuContext::adopt(adapter, device, queue, false)
}

pub fn adapter(gpu: &GpuContext) -> &wgpu::Adapter {
    &gpu.inner.adapter
}

pub fn adapter_info(gpu: &GpuContext) -> &wgpu::AdapterInfo {
    &gpu.inner.adapter_info
}

pub fn device(gpu: &GpuContext) -> &wgpu::Device {
    &gpu.inner.device
}

pub fn queue(gpu: &GpuContext) -> &wgpu::Queue {
    &gpu.inner.queue
}

/// Hold across a raw `submit` / `write_*` from any thread. Never across
/// `poll(Wait)` or surface work, and never around a contract call that submits
/// itself: the guard is not reentrant.
pub fn lock_submission(gpu: &GpuContext) -> RwLockReadGuard<'_, ()> {
    gpu.lock_submission()
}

/// Hold across `Surface::configure`.
pub fn lock_reconfigure(gpu: &GpuContext) -> RwLockWriteGuard<'_, ()> {
    gpu.lock_reconfigure()
}

/// Record a loss reported outside WGPU's callback (an embedder's own
/// notification).
pub fn mark_lost(gpu: &GpuContext, report: GpuDeviceLost) {
    gpu.mark_lost(report);
}

pub fn encoder(frame: &mut FrameContext) -> &mut wgpu::CommandEncoder {
    frame.encoder_mut()
}

pub fn transient_registry(
    frame: &FrameContext,
) -> std::sync::Arc<std::sync::Mutex<Vec<(crate::TransientResourceKey, wgpu::Buffer)>>> {
    frame.transient_registry()
}

pub fn submission_index(submission: &GpuSubmission) -> &wgpu::SubmissionIndex {
    &submission.index
}

pub fn texture(texture: &GpuTexture) -> &wgpu::Texture {
    texture.raw()
}

pub fn texture_view(texture: &GpuTexture) -> &wgpu::TextureView {
    texture.raw_view()
}

pub fn resource_layout(layout: &crate::GpuResourceLayout) -> &wgpu::BindGroupLayout {
    layout.raw()
}

pub fn resource_group(group: &crate::GpuResourceGroup) -> &wgpu::BindGroup {
    &group.bind_group
}

pub fn resource_group_dynamic_offsets(group: &crate::GpuResourceGroup) -> &[u32] {
    group.dynamic_offsets()
}

pub fn wrap_buffer(
    gpu: &GpuContext,
    buffer: wgpu::Buffer,
    size: u64,
    usage: crate::GpuBufferUsages,
) -> crate::GpuBuffer {
    crate::GpuBuffer::wrap_raw(gpu, buffer, size, usage)
}

pub fn wrap_sampler(gpu: &GpuContext, sampler: wgpu::Sampler) -> crate::GpuSampler {
    crate::GpuSampler::wrap_raw(gpu, sampler)
}

pub fn texture_from_wgpu(gpu: &GpuContext, texture: wgpu::Texture) -> GpuTexture {
    GpuTexture::wrap(gpu, texture)
}

pub fn target_view(target: &GpuRenderTarget) -> &wgpu::TextureView {
    &target.view
}

pub fn render_target(
    gpu: &GpuContext,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    size: [u32; 2],
) -> GpuRenderTarget {
    GpuRenderTarget::from_view(gpu, view, format, size)
}

pub const fn format_from_wgpu(format: wgpu::TextureFormat) -> GpuTextureFormat {
    GpuTextureFormat(format)
}

pub const fn format_to_wgpu(format: GpuTextureFormat) -> wgpu::TextureFormat {
    format.to_wgpu()
}

/// Return the format capabilities that WGPU will use for this context. The
/// framework uses this for internal render targets whose format is not tied to
/// the host surface (for example the motion evaluator). Keeping the query here
/// matters when an adapter-specific format table rejects a WebGPU-guaranteed
/// format.
pub fn texture_format_features(
    gpu: &GpuContext,
    format: wgpu::TextureFormat,
) -> Option<wgpu::TextureFormatFeatures> {
    gpu.format_features(format)
}

pub fn usages_to_wgpu(usage: GpuTextureUsages) -> wgpu::TextureUsages {
    usage.to_wgpu()
}

/// Share a real render pipeline across renderers on this context. Keys must
/// identify the complete immutable recipe (format, sample count, shader,
/// layout, material, primitive, blend/depth state and vertex layout).
/// The factory runs only on a miss and must not reenter the policy registry.
pub fn render_pipeline(
    gpu: &GpuContext,
    key: crate::PipelineKey,
    create: impl FnOnce() -> wgpu::RenderPipeline,
) -> Result<wgpu::RenderPipeline, crate::GpuError> {
    gpu.policy().pipeline(key, create)
}

pub fn render_pipeline_state(
    policy: &crate::GpuDeviceState,
    key: crate::PipelineKey,
    create: impl FnOnce() -> wgpu::RenderPipeline,
) -> Result<wgpu::RenderPipeline, crate::GpuError> {
    policy.pipeline(key, create)
}

/// The writes a frame submits ahead of its own commands. Renderers record
/// into it instead of calling `queue.write_*`: the frame stages them all
/// through one mapped chunk and copies them in one upload command buffer.
#[derive(Clone)]
pub struct FrameUploadHandle(crate::upload::FrameUploads);

impl std::fmt::Debug for FrameUploadHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrameUploadHandle")
            .finish_non_exhaustive()
    }
}

impl FrameUploadHandle {
    /// Like `queue.write_buffer`: `offset` 4-aligned, a length that is not
    /// is padded with zeros.
    pub fn write_buffer(&self, target: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_buffer(target, offset, bytes);
    }

    /// Like `queue.write_texture` for mip `mip_level` of a 2D texture:
    /// `rows_per_image` rows of `row_bytes` each, `bytes_per_row` apart.
    #[allow(clippy::too_many_arguments)]
    pub fn write_texture(
        &self,
        target: &wgpu::Texture,
        mip_level: u32,
        origin: wgpu::Origin3d,
        extent: wgpu::Extent3d,
        bytes: &[u8],
        bytes_per_row: u32,
        rows_per_image: u32,
        row_bytes: u32,
    ) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_texture(
                target,
                mip_level,
                origin,
                extent,
                bytes,
                bytes_per_row,
                rows_per_image,
                row_bytes,
            );
    }

    /// A region whose `row_bytes`-long rows `fill(row, dst)` writes in place.
    pub fn write_texture_rows(
        &self,
        target: &wgpu::Texture,
        origin: wgpu::Origin3d,
        extent: wgpu::Extent3d,
        row_bytes: u32,
        fill: impl FnMut(usize, &mut [u8]),
    ) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_texture_rows(target, 0, origin, extent, row_bytes, fill);
    }
}

pub fn frame_uploads(frame: &FrameContext) -> FrameUploadHandle {
    FrameUploadHandle(frame.uploads())
}

pub fn acquire_transient_buffer(
    policy: &crate::GpuDeviceState,
    key: &crate::TransientResourceKey,
    device: &wgpu::Device,
    label: &'static str,
    usage: wgpu::BufferUsages,
) -> Result<wgpu::Buffer, crate::GpuError> {
    policy.acquire_transient_buffer(*key, || {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: key.byte_size,
            usage,
            mapped_at_creation: false,
        })
    })
}
