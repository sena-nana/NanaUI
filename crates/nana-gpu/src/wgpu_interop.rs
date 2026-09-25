//! The explicit WGPU escape hatch (feature `wgpu-interop`).
//!
//! For hosts that bring their own device and for renderers that record their
//! own pipelines until Nana has a logical shader ABI. Everything here hands
//! out the same objects the contract wraps; it never creates a second device.

use std::sync::RwLockReadGuard;

use crate::{
    FrameContext, GpuContext, GpuRenderTarget, GpuSubmission, GpuTexture, GpuTextureFormat,
    GpuTextureUsages,
};

pub use wgpu;

/// Borrowed view of the WGPU objects behind a [`GpuContext`].
#[derive(Clone, Copy)]
pub struct WgpuInterop<'a> {
    gpu: &'a GpuContext,
}

impl<'a> WgpuInterop<'a> {
    pub fn adapter(self) -> &'a wgpu::Adapter {
        &self.gpu.inner.adapter
    }

    pub fn adapter_info(self) -> &'a wgpu::AdapterInfo {
        &self.gpu.inner.adapter_info
    }

    pub fn device(self) -> &'a wgpu::Device {
        &self.gpu.inner.device
    }

    pub fn queue(self) -> &'a wgpu::Queue {
        &self.gpu.inner.queue
    }

    /// Hold across every raw `queue.submit` / `write_texture` / `write_buffer`,
    /// from any thread, including the window thread. Never hold it across
    /// `device.poll(Wait)`, a sleep, or surface work: reconfiguration waits
    /// for it. It is not reentrant: do not call [`FrameContext::submit`],
    /// [`GpuContext::write_texture`] or anything else that submits for you
    /// while holding it.
    pub fn lock_submission(self) -> RwLockReadGuard<'a, ()> {
        self.gpu.lock_submission()
    }
}

impl GpuContext {
    /// Adopt a device the caller created. The caller keeps the device-lost
    /// callback: NanaUI installs none, and learns of a loss from the host
    /// (`EmbeddedRuntime::notify_device_lost`).
    pub fn from_wgpu(adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self::adopt(adapter, device, queue, false)
    }

    pub fn wgpu(&self) -> WgpuInterop<'_> {
        WgpuInterop { gpu: self }
    }
}

impl GpuTexture {
    /// Wrap a texture created on `gpu`'s device.
    pub fn from_wgpu(gpu: &GpuContext, texture: wgpu::Texture) -> Self {
        Self::wrap(gpu, texture)
    }

    pub fn wgpu(&self) -> &wgpu::Texture {
        self.raw()
    }

    /// The default full view.
    pub fn wgpu_view(&self) -> &wgpu::TextureView {
        self.raw_view()
    }
}

impl GpuTextureFormat {
    pub const fn from_wgpu(format: wgpu::TextureFormat) -> Self {
        Self(format)
    }

    pub const fn wgpu(self) -> wgpu::TextureFormat {
        self.to_wgpu()
    }
}

impl GpuTextureUsages {
    pub fn wgpu(self) -> wgpu::TextureUsages {
        self.to_wgpu()
    }
}

impl GpuRenderTarget {
    /// A view on `gpu`'s device, with its format and physical size.
    pub fn from_wgpu(
        gpu: &GpuContext,
        view: wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [u32; 2],
    ) -> Self {
        Self::from_view(gpu, view, format, size)
    }

    pub fn wgpu_view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

impl FrameContext {
    /// The frame's encoder. Record into it; never finish or submit it.
    pub fn wgpu_encoder(&mut self) -> &mut wgpu::CommandEncoder {
        self.encoder_mut()
    }
}

impl GpuSubmission {
    pub fn wgpu_index(&self) -> &wgpu::SubmissionIndex {
        &self.index
    }
}
