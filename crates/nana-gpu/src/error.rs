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
    /// The format cannot be created with the requested usages on this adapter,
    /// or has no fixed texel size for a CPU upload.
    UnsupportedFormat(GpuTextureFormat),
    /// The region leaves the texture.
    RegionOutOfBounds,
    /// A row is shorter than the region is wide.
    RowTooShort { needed: u32, provided: u32 },
    /// The bytes end before the region does.
    DataTooShort { needed: usize, provided: usize },
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
        }
    }
}

impl std::error::Error for GpuError {}
