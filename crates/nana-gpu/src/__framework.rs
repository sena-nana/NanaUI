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
/// `poll(Wait)` or surface work.
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

pub fn submission_index(submission: &GpuSubmission) -> &wgpu::SubmissionIndex {
    &submission.index
}

pub fn texture(texture: &GpuTexture) -> &wgpu::Texture {
    texture.raw()
}

pub fn texture_view(texture: &GpuTexture) -> &wgpu::TextureView {
    texture.raw_view()
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

pub fn usages_to_wgpu(usage: GpuTextureUsages) -> wgpu::TextureUsages {
    usage.to_wgpu()
}
