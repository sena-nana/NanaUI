//! Same-device embedding of a retained [`ExternalFrame`](super::ExternalFrame).
//!
//! An embedded node is only an output-side lease and a small amount of
//! presentation metadata.  It intentionally does not merge the child scene
//! or layout tree into the parent.  The parent samples [`Self::texture`] while
//! recording its own scene, then binds the lease to the parent's submitted
//! [`GpuSubmission`].  The lease is released from WGPU's completion callback,
//! so a child target cannot be recycled while the parent still samples it.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nana_gpu::{DeviceGeneration, GpuContext, GpuError, GpuSubmission, GpuTexture};

use super::{ExternalFrame, ExternalSurfaceError};
use crate::{HostTexture, HostTextureAlphaMode, HostTextureBinding, HostTextureRegistry};

static NEXT_EMBEDDED_TEXTURE_ID: AtomicU64 = AtomicU64::new(1);

/// Presentation metadata exported by a child to its parent compositor.
///
/// `logical_extent` describes the child scene's logical viewport. The parent
/// samples a texture with `texture_extent`; parent transforms, clips, and
/// opacity remain parent-scene concerns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmbeddedOutputMetadata {
    logical_extent: [f32; 2],
    texture_extent: [u32; 2],
    resource_generation: u64,
    content_revision: u64,
    device_generation: DeviceGeneration,
}

impl EmbeddedOutputMetadata {
    /// Build metadata after checking the child logical viewport.
    pub fn new(
        logical_extent: [f32; 2],
        texture_extent: [u32; 2],
        resource_generation: u64,
        content_revision: u64,
        device_generation: DeviceGeneration,
    ) -> Result<Self, EmbeddedMetadataError> {
        validate_extents(logical_extent, texture_extent)?;
        Ok(Self {
            logical_extent,
            texture_extent,
            resource_generation,
            content_revision,
            device_generation,
        })
    }

    pub fn from_frame(
        frame: &ExternalFrame,
        logical_extent: [f32; 2],
    ) -> Result<Self, EmbeddedMetadataError> {
        let (width, height) = frame
            .texture()
            .map_err(|_| EmbeddedMetadataError::StaleFrame)?
            .size();
        Self::new(
            logical_extent,
            [width, height],
            frame.resource_generation(),
            frame.content_revision(),
            frame.device_generation(),
        )
    }

    pub const fn logical_extent(self) -> [f32; 2] {
        self.logical_extent
    }

    pub const fn texture_extent(self) -> [u32; 2] {
        self.texture_extent
    }

    pub const fn resource_generation(self) -> u64 {
        self.resource_generation
    }

    pub const fn content_revision(self) -> u64 {
        self.content_revision
    }

    pub const fn device_generation(self) -> DeviceGeneration {
        self.device_generation
    }
}

fn validate_extents(
    logical_extent: [f32; 2],
    texture_extent: [u32; 2],
) -> Result<(), EmbeddedMetadataError> {
    if !logical_extent
        .iter()
        .all(|value| value.is_finite() && *value > 0.0)
    {
        return Err(EmbeddedMetadataError::InvalidLogicalExtent);
    }
    if texture_extent.contains(&0) {
        return Err(EmbeddedMetadataError::InvalidTextureExtent);
    }
    Ok(())
}

/// Invalid child presentation metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedMetadataError {
    InvalidLogicalExtent,
    InvalidTextureExtent,
    StaleFrame,
}

impl fmt::Display for EmbeddedMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLogicalExtent => {
                formatter.write_str("embedded logical extent must be finite and positive")
            }
            Self::InvalidTextureExtent => {
                formatter.write_str("embedded texture extent must be non-zero")
            }
            Self::StaleFrame => formatter.write_str("embedded frame is no longer current"),
        }
    }
}

impl std::error::Error for EmbeddedMetadataError {}

#[derive(Debug)]
struct LeaseState {
    frame: Mutex<Option<ExternalFrame>>,
    bound: AtomicBool,
}

impl LeaseState {
    fn new(frame: ExternalFrame) -> Self {
        Self {
            frame: Mutex::new(Some(frame)),
            bound: AtomicBool::new(false),
        }
    }

    fn retire(&self) -> bool {
        if let Ok(mut frame) = self.frame.lock()
            && let Some(frame) = frame.take()
        {
            frame.retire();
            return true;
        }
        false
    }

    fn validate(&self) -> Result<(), ExternalSurfaceError> {
        let frame = self
            .frame
            .lock()
            .map_err(|_| ExternalSurfaceError::StaleFrame)?;
        frame
            .as_ref()
            .ok_or(ExternalSurfaceError::StaleFrame)?
            .validate()
    }
}

impl Drop for LeaseState {
    fn drop(&mut self) {
        if let Ok(frame) = self.frame.get_mut()
            && let Some(frame) = frame.take()
        {
            frame.retire();
        }
    }
}

/// A parent-scene node backed by one retained child frame.
///
/// The node holds the child lease while the parent records its frame.  Call
/// [`Self::bind_to_submission`] immediately after submitting the parent frame;
/// the callback then retires the child lease after parent GPU work completes.
/// Dropping an unbound node retires its lease immediately, which is safe when
/// no parent submission was made.
#[derive(Debug)]
pub struct EmbeddedSurfaceNode {
    texture: GpuTexture,
    host_texture: HostTexture,
    metadata: EmbeddedOutputMetadata,
    lease: Arc<LeaseState>,
}

impl EmbeddedSurfaceNode {
    pub fn from_frame(
        frame: ExternalFrame,
        logical_extent: [f32; 2],
    ) -> Result<Self, EmbeddedMetadataError> {
        let metadata = EmbeddedOutputMetadata::from_frame(&frame, logical_extent)?;
        let texture = frame
            .texture()
            .map_err(|_| EmbeddedMetadataError::StaleFrame)?
            .clone();
        let host_texture = HostTexture::new(
            NEXT_EMBEDDED_TEXTURE_ID.fetch_add(1, Ordering::Relaxed),
            metadata.content_revision,
            &texture,
        );
        Ok(Self {
            texture,
            host_texture,
            metadata,
            lease: Arc::new(LeaseState::new(frame)),
        })
    }

    pub const fn metadata(&self) -> EmbeddedOutputMetadata {
        self.metadata
    }

    pub const fn resource_generation(&self) -> u64 {
        self.metadata.resource_generation
    }

    pub const fn content_revision(&self) -> u64 {
        self.metadata.content_revision
    }

    /// Validate the child generation before recording a parent pass.
    pub fn validate(&self) -> Result<(), ExternalSurfaceError> {
        self.lease.validate()
    }

    /// The child texture to sample in the parent scene.
    pub fn texture(&self) -> Result<&GpuTexture, ExternalSurfaceError> {
        self.validate()?;
        Ok(&self.texture)
    }

    /// Stable host-texture handle for a parent `SceneWgpuPainter`.
    ///
    /// Register the returned binding in the parent's [`HostTextureRegistry`]
    /// while this node is held through the parent submission.  A new child
    /// revision should create a new node/binding; parent transforms and clips
    /// do not require changing the binding.
    pub fn host_texture(&self) -> &HostTexture {
        &self.host_texture
    }

    /// Register this node as one parent host-texture slot. Re-registering the
    /// same node and dimensions is stable and does not bump the registry
    /// revision. The registry owns a clone of the handle, so the parent must
    /// replace or remove this slot when it stops using the node; keeping an
    /// old slot registered after the completion callback can sample a child
    /// target that has since been recycled.
    pub fn register_host_texture(
        &self,
        registry: &HostTextureRegistry,
        slot: impl Into<String>,
        alpha_mode: HostTextureAlphaMode,
    ) -> HostTextureBinding {
        let [width, height] = self.metadata.texture_extent;
        registry.register(slot, self.host_texture.clone(), width, height, alpha_mode)
    }

    /// Whether this node still references the child's current resource
    /// generation.
    pub fn is_current(&self) -> bool {
        self.lease.validate().is_ok()
    }

    /// Retire an unbound child lease immediately. Once it has been bound to a
    /// parent submission this returns `false`; the completion callback remains
    /// the only retirement point so the parent cannot sample freed storage.
    pub fn retire(&self) -> bool {
        if self.lease.bound.load(Ordering::Acquire) {
            return false;
        }
        self.lease.retire()
    }

    /// Bind child lifetime to a submitted parent frame.
    ///
    /// This registration is non-blocking.  `parent_gpu` must be the same
    /// device that submitted `submission`; a mismatch is rejected before the
    /// callback is installed.  Calling this method twice is rejected so one
    /// lease cannot be retired against two unrelated parent submissions.
    pub fn bind_to_submission(
        &self,
        parent_gpu: &GpuContext,
        submission: &GpuSubmission,
    ) -> Result<EmbeddedFrameBinding, EmbeddedBindError> {
        self.validate().map_err(|_| EmbeddedBindError::StaleFrame)?;
        let expected = self.metadata.device_generation;
        let found = parent_gpu.generation();
        if expected != found || submission.generation() != found {
            return Err(EmbeddedBindError::DeviceMismatch { expected, found });
        }
        if self.lease.bound.swap(true, Ordering::AcqRel) {
            return Err(EmbeddedBindError::AlreadyBound);
        }
        let lease = Arc::clone(&self.lease);
        if let Err(error) = parent_gpu.on_submission_complete(submission, move || {
            lease.retire();
        }) {
            self.lease.bound.store(false, Ordering::Release);
            self.lease.retire();
            return Err(EmbeddedBindError::Gpu(error));
        }
        Ok(EmbeddedFrameBinding {
            metadata: self.metadata,
            parent_generation: found,
            parent_submission: submission.submission(),
        })
    }
}

/// Completion token returned after an embedded lease is bound to a parent
/// submission.  The actual lease is retained by the completion callback; this
/// value carries stable metadata for diagnostics and parent bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmbeddedFrameBinding {
    metadata: EmbeddedOutputMetadata,
    parent_generation: DeviceGeneration,
    parent_submission: u64,
}

/// Alias for callers that name the resource lifetime explicitly.
impl EmbeddedFrameBinding {
    pub const fn metadata(self) -> EmbeddedOutputMetadata {
        self.metadata
    }

    pub const fn parent_generation(self) -> DeviceGeneration {
        self.parent_generation
    }

    pub const fn parent_submission(self) -> u64 {
        self.parent_submission
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddedBindError {
    StaleFrame,
    DeviceMismatch {
        expected: DeviceGeneration,
        found: DeviceGeneration,
    },
    AlreadyBound,
    Gpu(GpuError),
}

impl fmt::Display for EmbeddedBindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleFrame => {
                formatter.write_str("embedded child frame belongs to an old resource generation")
            }
            Self::DeviceMismatch { expected, found } => write!(
                formatter,
                "embedded child belongs to device {expected}, parent uses {found}"
            ),
            Self::AlreadyBound => {
                formatter.write_str("embedded child lease is already bound to a parent submission")
            }
            Self::Gpu(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for EmbeddedBindError {}

impl From<GpuError> for EmbeddedBindError {
    fn from(error: GpuError) -> Self {
        Self::Gpu(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extent_validation_is_independent_of_gpu_backend() {
        assert!(validate_extents([320.0, 180.0], [640, 360]).is_ok());
        assert_eq!(
            validate_extents([0.0, 10.0], [1, 1]),
            Err(EmbeddedMetadataError::InvalidLogicalExtent)
        );
        assert_eq!(
            validate_extents([10.0, f32::NAN], [1, 1]),
            Err(EmbeddedMetadataError::InvalidLogicalExtent)
        );
        assert_eq!(
            validate_extents([10.0, 10.0], [0, 1]),
            Err(EmbeddedMetadataError::InvalidTextureExtent)
        );
    }

    #[test]
    fn scale_validation_accepts_non_square_targets() {
        assert!(validate_extents([320.0, 180.0], [640, 360]).is_ok());
    }
}
