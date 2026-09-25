//! The JS WebGPU facade's access to the WGPU objects behind the host's
//! `GpuContext`. Framework-internal: it goes through `nana_gpu::__framework`,
//! so Vue applications do not get the `wgpu-interop` escape hatch with it.

use std::sync::RwLockReadGuard;

use nana_gpu::{__framework, GpuContext, GpuTexture};

#[derive(Clone, Copy)]
pub(crate) struct Backend<'a>(&'a GpuContext);

impl<'a> Backend<'a> {
    pub(crate) fn device(self) -> &'a wgpu::Device {
        __framework::device(self.0)
    }

    pub(crate) fn queue(self) -> &'a wgpu::Queue {
        __framework::queue(self.0)
    }

    pub(crate) fn adapter_info(self) -> &'a wgpu::AdapterInfo {
        __framework::adapter_info(self.0)
    }

    /// Held around every raw submit or queue write, never across a poll.
    pub(crate) fn lock_submission(self) -> RwLockReadGuard<'a, ()> {
        __framework::lock_submission(self.0)
    }
}

pub(crate) trait GpuBackend {
    fn backend(&self) -> Backend<'_>;
}

impl GpuBackend for GpuContext {
    fn backend(&self) -> Backend<'_> {
        Backend(self)
    }
}

pub(crate) fn texture(gpu: &GpuContext, texture: wgpu::Texture) -> GpuTexture {
    __framework::texture_from_wgpu(gpu, texture)
}
