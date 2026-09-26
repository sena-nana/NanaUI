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
//! - [`GpuDeviceState`] is the shared per-device policy authority for bounded
//!   uploads, frame slots, transient keys, pipeline/realization identities and
//!   submission retirement. It contains no public WGPU types.
//!
//! WGPU is the only backend. Its objects stay behind the contract; the
//! `wgpu-interop` feature is the explicit escape hatch for hosts that bring
//! their own device and renderers that record their own pipelines.

#![recursion_limit = "256"]

mod abi;
mod context;
mod error;
mod frame;
mod policy;
mod realization;
mod texture;

#[doc(hidden)]
pub mod __framework;

#[cfg(feature = "wgpu-interop")]
mod wgpu_interop;

pub use abi::{
    LogicalBinding, LogicalBindingType, LogicalResource, ResourceBinding, ResourceClass,
    ResourceSet, ResourceTable, ShaderInterface, ShaderStage, VertexAttribute,
};
pub use context::{
    DeviceGeneration, GpuBackend, GpuCapabilities, GpuCapability, GpuCapabilityOutcome, GpuContext,
    GpuDeviceLost, GpuDeviceType, GpuFeatureSet, GpuLimits, GpuLossReason,
};
pub use error::GpuError;
pub use frame::{FrameContext, FrameId, GpuSubmission, RetainedWrites};
pub use policy::{
    FrameSlotId, GpuDeviceState, GpuPolicyStats, PipelineKey, TransientResourceKey,
    UploadReservation,
};
pub use realization::{GpuBuffer, GpuBufferDescriptor, GpuBufferUsages, GpuSampler};
pub use realization::{GpuResourceGroup, GpuResourceLayout};
pub use texture::{
    GpuRenderTarget, GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages,
};

#[cfg(feature = "wgpu-interop")]
pub use wgpu_interop::{WgpuInterop, wgpu};
