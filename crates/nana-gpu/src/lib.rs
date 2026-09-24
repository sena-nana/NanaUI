//! NanaUI's GPU backend contract.
//!
//! Renderers, producers and hosts talk to the GPU through Nana-owned types:
//!
//! - [`GpuContext`] is the one device a process renders with: its
//!   [`DeviceGeneration`], [`GpuCapabilities`], loss state, texture creation
//!   and upload, and the submission guard that keeps off-thread work away from
//!   surface reconfiguration.
//! - [`FrameContext`] owns one command encoder from [`GpuContext::begin_frame`]
//!   until it is submitted or dropped. Dropping it without submitting is the
//!   discard, and rolls back the retained writes painters recorded into it.
//! - [`GpuTexture`] / [`GpuRenderTarget`] carry the device generation they
//!   were created on, so a resource from a replaced device is refused instead
//!   of reaching the backend.
//!
//! WGPU is the only backend. Its objects stay behind the contract; the
//! `wgpu-interop` feature is the explicit escape hatch for hosts that bring
//! their own device and renderers that record their own pipelines.

mod context;
mod error;
mod frame;
mod texture;

#[doc(hidden)]
pub mod __framework;

#[cfg(feature = "wgpu-interop")]
mod wgpu_interop;

pub use context::{
    DeviceGeneration, GpuBackend, GpuCapabilities, GpuContext, GpuDeviceLost, GpuDeviceType,
    GpuFeatureSet, GpuLossReason,
};
pub use error::GpuError;
pub use frame::{FrameContext, FrameId, GpuSubmission, RetainedWrites};
pub use texture::{
    GpuRenderTarget, GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages,
};

#[cfg(feature = "wgpu-interop")]
pub use wgpu_interop::{WgpuInterop, wgpu};
