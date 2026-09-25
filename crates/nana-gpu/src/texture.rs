use std::fmt;
use std::ops::BitOr;
use std::sync::Arc;

use crate::{DeviceGeneration, GpuContext, GpuError};

/// Texel format. Opaque so the contract never names a backend type; the
/// constants cover what Nana creates, and a surface or interop source may
/// carry any other format through unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuTextureFormat(pub(crate) wgpu::TextureFormat);

impl GpuTextureFormat {
    pub const RGBA8_UNORM: Self = Self(wgpu::TextureFormat::Rgba8Unorm);
    pub const RGBA8_UNORM_SRGB: Self = Self(wgpu::TextureFormat::Rgba8UnormSrgb);
    pub const BGRA8_UNORM: Self = Self(wgpu::TextureFormat::Bgra8Unorm);
    pub const BGRA8_UNORM_SRGB: Self = Self(wgpu::TextureFormat::Bgra8UnormSrgb);
    pub const RGBA16_FLOAT: Self = Self(wgpu::TextureFormat::Rgba16Float);
    pub const RGB10A2_UNORM: Self = Self(wgpu::TextureFormat::Rgb10a2Unorm);
    pub const R8_UNORM: Self = Self(wgpu::TextureFormat::R8Unorm);

    /// Whether sampling decodes sRGB and writing encodes it.
    pub fn is_srgb(self) -> bool {
        self.0.is_srgb()
    }

    pub fn is_depth_stencil(self) -> bool {
        self.0.is_depth_stencil_format()
    }

    /// Bytes of one texel, for formats with a fixed 1x1 block and a single
    /// aspect. `None` for compressed and depth/stencil formats.
    pub fn bytes_per_pixel(self) -> Option<u32> {
        if self.0.block_dimensions() != (1, 1) || self.0.is_depth_stencil_format() {
            return None;
        }
        self.0.block_copy_size(None)
    }
}

impl fmt::Debug for GpuTextureFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "GpuTextureFormat({:?})", self.0)
    }
}

/// How a texture may be used.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuTextureUsages(u8);

impl GpuTextureUsages {
    pub const SAMPLED: Self = Self(1);
    pub const COPY_SRC: Self = Self(1 << 1);
    pub const COPY_DST: Self = Self(1 << 2);
    pub const RENDER_TARGET: Self = Self(1 << 3);
    pub const STORAGE: Self = Self(1 << 4);

    pub const fn empty() -> Self {
        Self(0)
    }

    /// Stable Nana-level bit identity for resource-pool keys.
    pub const fn bits(self) -> u32 {
        self.0 as u32
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub(crate) fn to_wgpu(self) -> wgpu::TextureUsages {
        let mut usage = wgpu::TextureUsages::empty();
        if self.contains(Self::SAMPLED) {
            usage |= wgpu::TextureUsages::TEXTURE_BINDING;
        }
        if self.contains(Self::COPY_SRC) {
            usage |= wgpu::TextureUsages::COPY_SRC;
        }
        if self.contains(Self::COPY_DST) {
            usage |= wgpu::TextureUsages::COPY_DST;
        }
        if self.contains(Self::RENDER_TARGET) {
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        if self.contains(Self::STORAGE) {
            usage |= wgpu::TextureUsages::STORAGE_BINDING;
        }
        usage
    }

    /// The part of `usage` the contract names; other backend usages are kept
    /// on the texture but not reported.
    pub(crate) fn from_wgpu(usage: wgpu::TextureUsages) -> Self {
        let mut out = Self::empty();
        if usage.contains(wgpu::TextureUsages::TEXTURE_BINDING) {
            out = out | Self::SAMPLED;
        }
        if usage.contains(wgpu::TextureUsages::COPY_SRC) {
            out = out | Self::COPY_SRC;
        }
        if usage.contains(wgpu::TextureUsages::COPY_DST) {
            out = out | Self::COPY_DST;
        }
        if usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
            out = out | Self::RENDER_TARGET;
        }
        if usage.contains(wgpu::TextureUsages::STORAGE_BINDING) {
            out = out | Self::STORAGE;
        }
        out
    }
}

impl BitOr for GpuTextureUsages {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl fmt::Debug for GpuTextureUsages {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = [
            (Self::SAMPLED, "SAMPLED"),
            (Self::COPY_SRC, "COPY_SRC"),
            (Self::COPY_DST, "COPY_DST"),
            (Self::RENDER_TARGET, "RENDER_TARGET"),
            (Self::STORAGE, "STORAGE"),
        ];
        let mut list = formatter.debug_set();
        for (flag, name) in names {
            if self.contains(flag) {
                list.entry(&format_args!("{name}"));
            }
        }
        list.finish()
    }
}

/// A 2D, single-mip, single-sample texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTextureDescriptor<'a> {
    pub label: Option<&'a str>,
    pub width: u32,
    pub height: u32,
    pub format: GpuTextureFormat,
    pub usage: GpuTextureUsages,
}

/// Texel rectangle of an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuTextureRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl GpuTextureRegion {
    /// The whole of a `width` x `height` texture.
    pub const fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
}

struct TextureInner {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    generation: DeviceGeneration,
    format: GpuTextureFormat,
    usage: GpuTextureUsages,
    size: (u32, u32),
}

/// A texture on one device. Clones share the texture.
#[derive(Clone)]
pub struct GpuTexture {
    inner: Arc<TextureInner>,
}

impl fmt::Debug for GpuTexture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuTexture")
            .field("size", &self.inner.size)
            .field("format", &self.inner.format)
            .field("usage", &self.inner.usage)
            .field("generation", &self.inner.generation)
            .finish()
    }
}

impl GpuTexture {
    pub(crate) fn wrap(gpu: &GpuContext, texture: wgpu::Texture) -> Self {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            inner: Arc::new(TextureInner {
                generation: gpu.generation(),
                format: GpuTextureFormat(texture.format()),
                usage: GpuTextureUsages::from_wgpu(texture.usage()),
                size: (texture.width(), texture.height()),
                texture,
                view,
            }),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.inner.size
    }

    pub fn format(&self) -> GpuTextureFormat {
        self.inner.format
    }

    pub fn usage(&self) -> GpuTextureUsages {
        self.inner.usage
    }

    /// The device this texture was created on.
    pub fn generation(&self) -> DeviceGeneration {
        self.inner.generation
    }

    /// Whether both handles are the same texture.
    pub fn ptr_eq(&self, other: &GpuTexture) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// This texture as a render destination.
    pub fn render_target(&self) -> Result<GpuRenderTarget, GpuError> {
        if !self.inner.usage.contains(GpuTextureUsages::RENDER_TARGET) {
            return Err(GpuError::MissingUsage(GpuTextureUsages::RENDER_TARGET));
        }
        Ok(GpuRenderTarget {
            view: self.inner.view.clone(),
            format: self.inner.format,
            size: [self.inner.size.0, self.inner.size.1],
            generation: self.inner.generation,
        })
    }

    pub(crate) fn raw(&self) -> &wgpu::Texture {
        &self.inner.texture
    }

    pub(crate) fn raw_view(&self) -> &wgpu::TextureView {
        &self.inner.view
    }
}

/// A render destination: a texture view with its format, physical size and
/// device.
#[derive(Clone)]
pub struct GpuRenderTarget {
    pub(crate) view: wgpu::TextureView,
    pub(crate) format: GpuTextureFormat,
    pub(crate) size: [u32; 2],
    pub(crate) generation: DeviceGeneration,
}

impl fmt::Debug for GpuRenderTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuRenderTarget")
            .field("format", &self.format)
            .field("size", &self.size)
            .field("generation", &self.generation)
            .finish()
    }
}

impl GpuRenderTarget {
    pub(crate) fn from_view(
        gpu: &GpuContext,
        view: wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [u32; 2],
    ) -> Self {
        Self {
            view,
            format: GpuTextureFormat(format),
            size,
            generation: gpu.generation(),
        }
    }

    pub fn format(&self) -> GpuTextureFormat {
        self.format
    }

    /// Physical size in pixels.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn generation(&self) -> DeviceGeneration {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usages_round_trip_through_the_backend() {
        let all = GpuTextureUsages::SAMPLED
            | GpuTextureUsages::COPY_SRC
            | GpuTextureUsages::COPY_DST
            | GpuTextureUsages::RENDER_TARGET;
        assert_eq!(GpuTextureUsages::from_wgpu(all.to_wgpu()), all);
        assert!(all.contains(GpuTextureUsages::COPY_DST));
        assert!(!GpuTextureUsages::SAMPLED.contains(GpuTextureUsages::COPY_DST));
        assert_eq!(
            format!(
                "{:?}",
                GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST
            ),
            "{SAMPLED, COPY_DST}"
        );
    }

    #[test]
    fn texel_sizes_cover_uploadable_formats_only() {
        assert_eq!(GpuTextureFormat::RGBA8_UNORM.bytes_per_pixel(), Some(4));
        assert_eq!(GpuTextureFormat::R8_UNORM.bytes_per_pixel(), Some(1));
        assert_eq!(GpuTextureFormat::RGBA16_FLOAT.bytes_per_pixel(), Some(8));
        assert_eq!(
            GpuTextureFormat(wgpu::TextureFormat::Bc1RgbaUnorm).bytes_per_pixel(),
            None
        );
        assert!(GpuTextureFormat::BGRA8_UNORM_SRGB.is_srgb());
        assert!(!GpuTextureFormat::BGRA8_UNORM.is_srgb());
    }
}
