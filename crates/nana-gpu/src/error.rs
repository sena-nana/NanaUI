use std::fmt;

use crate::{DeviceGeneration, GpuTextureFormat, GpuTextureUsages};

/// A request the contract refused before it reached the backend.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    /// The resource belongs to another device than the context it was used with.
    DeviceMismatch {
        expected: DeviceGeneration,
        found: DeviceGeneration,
    },
    /// Zero, larger than the device's `max_texture_dimension_2d`, or not a
    /// whole number of blocks of a compressed format.
    InvalidExtent { width: u32, height: u32, max: u32 },
    /// The texture was not created with these usages.
    MissingUsage(GpuTextureUsages),
    /// A texture was requested with no usage at all.
    EmptyUsage,
    /// A transient texture factory did not produce the requested resource.
    TransientDescriptorMismatch,
    /// The format cannot be created with the requested usages on this adapter,
    /// or has no fixed texel size for a CPU upload.
    UnsupportedFormat(GpuTextureFormat),
    /// The region leaves the texture.
    RegionOutOfBounds,
    /// A row is shorter than the region is wide.
    RowTooShort { needed: u32, provided: u32 },
    /// The bytes end before the region does.
    DataTooShort { needed: usize, provided: usize },
    /// Two logical bindings claimed the same slot.
    DuplicateBinding,
    /// A logical resource table cannot be used with the requested capability.
    UnsupportedCapability(&'static str),
    /// A shader interface and resource table do not describe the same layout.
    ShaderInterfaceMismatch,
    /// A dynamic binding violates the backend alignment or range contract.
    InvalidBindingRange,
    /// A logical buffer has zero size, no usage, or exceeds device limits.
    InvalidBufferDescriptor,
    /// A required logical binding has no resource value.
    MissingBinding(u32),
    /// A value has the wrong kind for the declared logical binding.
    BindingTypeMismatch(u32),
}

impl fmt::Display for GpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceMismatch { expected, found } => write!(
                formatter,
                "GPU resource from device {found} used with device {expected}"
            ),
            Self::InvalidExtent { width, height, max } => write!(
                formatter,
                "texture extent {width}x{height} is empty or exceeds {max}"
            ),
            Self::MissingUsage(usage) => write!(formatter, "texture lacks usage {usage:?}"),
            Self::TransientDescriptorMismatch => {
                formatter.write_str("transient texture descriptor mismatch")
            }
            Self::EmptyUsage => formatter.write_str("texture usage must not be empty"),
            Self::UnsupportedFormat(format) => {
                write!(formatter, "texture format {format:?} is not supported here")
            }
            Self::RegionOutOfBounds => formatter.write_str("texture region is out of bounds"),
            Self::RowTooShort { needed, provided } => write!(
                formatter,
                "bytes_per_row {provided} is shorter than the region row of {needed} bytes"
            ),
            Self::DataTooShort { needed, provided } => write!(
                formatter,
                "texture upload needs {needed} bytes, got {provided}"
            ),
            Self::DuplicateBinding => {
                formatter.write_str("logical resource table has duplicate bindings")
            }
            Self::UnsupportedCapability(name) => {
                write!(formatter, "GPU capability {name} is unavailable")
            }
            Self::ShaderInterfaceMismatch => {
                formatter.write_str("shader interface and resource table do not match")
            }
            Self::InvalidBindingRange => {
                formatter.write_str("dynamic GPU binding range is invalid")
            }
            Self::InvalidBufferDescriptor => {
                formatter.write_str("GPU buffer descriptor is invalid")
            }
            Self::MissingBinding(binding) => {
                write!(formatter, "logical binding {binding} has no resource")
            }
            Self::BindingTypeMismatch(binding) => {
                write!(
                    formatter,
                    "logical binding {binding} has the wrong resource type"
                )
            }
        }
    }
}

impl std::error::Error for GpuError {}
