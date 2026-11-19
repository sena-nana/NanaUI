//! NanaUI's GPU backend contract.
//!
//! Renderers, producers and hosts talk to the GPU through Nana-owned types:
//!
//! - [`GpuContext`] is the one device a process renders with: its
//!   [`DeviceGeneration`], [`GpuCapabilities`], loss state, texture creation
//!   and upload. Queue work needs no guard against surface reconfiguration:
//!   NanaUI's wgpu fork waits only for the work that used that surface.
//! - [`FrameContext`] owns one command encoder from [`GpuContext::begin_frame`]
//!   until it is submitted or dropped. Dropping it without submitting is the
//!   discard, and rolls back the retained writes painters recorded into it.
//! - [`GpuTexture`] / [`GpuRenderTarget`] carry the device generation they
//!   were created on, so a resource from a replaced device is refused instead
//!   of reaching the backend.
//! - The device's shader and pipeline cache is one per [`GpuContext`]; the
//!   host persists it through [`GpuContext::pipeline_cache_key`],
//!   [`GpuContext::load_pipeline_cache`] and [`GpuContext::pipeline_cache_data`].
//! - [`GpuDeviceState`] is the shared per-device policy authority for bounded
//!   uploads, frame slots, transient keys, pipeline/realization identities and
//!   submission retirement. It contains no public WGPU types.
//!
//! WGPU is the only backend. Its objects stay behind the contract; the
//! `wgpu-interop` feature is the explicit escape hatch for hosts that bring
//! their own device and renderers that record their own pipelines.

#![recursion_limit = "256"]

mod abi;
mod budget;
mod context;
mod error;
mod frame;
mod pipeline_cache;
mod policy;
mod realization;
mod texture;
mod upload;

#[cfg(test)]
mod test_gpu;

#[doc(hidden)]
pub mod __framework;

#[cfg(feature = "wgpu-interop")]
mod wgpu_interop;

#[cfg(all(windows, feature = "native-export"))]
mod export;

#[cfg(all(windows, feature = "native-import"))]
mod import;

pub use abi::{
    LogicalBinding, LogicalBindingType, LogicalResource, ResourceBinding, ResourceClass,
    ResourceSet, ResourceTable, ShaderInterface, ShaderStage, VertexAttribute,
};
pub use context::{
    DeviceGeneration, GpuBackend, GpuCapabilities, GpuCapability, GpuCapabilityOutcome, GpuContext,
    GpuDeviceLost, GpuDeviceType, GpuFeatureSet, GpuLimits, GpuLossReason,
};
pub use error::GpuError;
pub use frame::{
    FrameAcquisition, FrameContext, FrameId, FrameLane, GpuSubmission, RetainedWrites,
};
pub use pipeline_cache::{
    PipelineCacheKey, PipelineCacheLoad, PipelineCacheRefusal, PipelineCacheSaver,
    PipelineCacheStore,
};
pub use policy::{FrameSlotId, GpuDeviceState, GpuPolicyStats, PipelineKey, TransientResourceKey};
pub use realization::{GpuBuffer, GpuBufferDescriptor, GpuBufferUsages, GpuSampler};
pub use realization::{GpuResourceGroup, GpuResourceLayout};
pub use texture::{
    GpuRenderTarget, GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages,
};

#[cfg(feature = "wgpu-interop")]
pub use wgpu_interop::{WgpuInterop, wgpu};

#[cfg(all(windows, feature = "native-export"))]
pub use export::{
    NATIVE_EXPORT_FORMAT, NATIVE_EXPORT_SLOTS, NativeExportDeferral, NativeExportError,
    NativeExportOutcome, NativeExportPool, NativeExportRetirement, NativeFrameToken,
    StagedNativeFrame,
};

#[cfg(all(windows, feature = "native-import"))]
pub use import::{
    NATIVE_IMPORT_SLOTS, NativeImportError, NativeImportFrame, NativeImportPool, NativeImportTarget,
};
