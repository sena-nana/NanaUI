//! Host-neutral surface colour-profile negotiation.
//!
//! Desktop (`hosted`), mobile (`nana-mobile-host`), and custom `wgpu-interop`
//! embedders share the same deterministic capability resolver so wide-gamut,
//! HDR, and SDR fallback selection behave identically everywhere.

use crate::{ScenePresentationColorSpace, ScenePresentationProfile};
pub use nana_ui_platform::SurfacePresentationPolicy;

/// Why a requested surface profile was not selected exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfaceProfileFallback {
    /// The requested colour space/format was not advertised by the surface.
    CapabilityUnavailable,
    /// The requested colour space was advertised only on a format whose alpha
    /// channel cannot support transparent composition (for example, 2-bit
    /// alpha `Rgb10a2Unorm` on a transparent BT.2100 window).
    TransparencyIncompatible,
}

/// The concrete format and colour space a host configured for one surface.
///
/// This is the single profile identity carried across resize, recovery and
/// device migration. Equal profiles are a no-op and must not trigger a
/// swapchain rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResolvedSurfaceProfile {
    pub requested: SurfacePresentationPolicy,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) color_space: wgpu::SurfaceColorSpace,
    pub fallback: Option<SurfaceProfileFallback>,
}

impl ResolvedSurfaceProfile {
    pub const fn is_hdr(self) -> bool {
        self.color_space.is_hdr()
    }

    pub const fn is_wide_gamut(self) -> bool {
        matches!(
            self.color_space,
            wgpu::SurfaceColorSpace::DisplayP3
                | wgpu::SurfaceColorSpace::ExtendedDisplayP3
                | wgpu::SurfaceColorSpace::Bt2100Pq
                | wgpu::SurfaceColorSpace::Bt2100Hlg
        )
    }

    pub const fn fallback(self) -> Option<SurfaceProfileFallback> {
        self.fallback
    }

    /// Raw surface format for hosts that explicitly opt into WGPU interop.
    #[cfg(feature = "wgpu-interop")]
    pub const fn format(self) -> wgpu::TextureFormat {
        self.format
    }

    /// Raw surface colour space for hosts that explicitly opt into WGPU
    /// interop.
    #[cfg(feature = "wgpu-interop")]
    pub const fn color_space(self) -> wgpu::SurfaceColorSpace {
        self.color_space
    }

    /// Translate the host surface contract into the Scene painter contract.
    /// The painter must see the color space as well as the format: a P3
    /// `Bgra8Unorm` surface cannot share an sRGB painter's direct path.
    pub fn scene_profile(self) -> ScenePresentationProfile {
        let color_space = match self.color_space {
            wgpu::SurfaceColorSpace::DisplayP3 => ScenePresentationColorSpace::DisplayP3,
            wgpu::SurfaceColorSpace::ExtendedSrgbLinear => {
                ScenePresentationColorSpace::ExtendedSrgbLinear
            }
            wgpu::SurfaceColorSpace::ExtendedSrgb => ScenePresentationColorSpace::ExtendedSrgb,
            wgpu::SurfaceColorSpace::ExtendedDisplayP3 => {
                ScenePresentationColorSpace::ExtendedDisplayP3
            }
            wgpu::SurfaceColorSpace::Bt2100Pq => ScenePresentationColorSpace::Bt2100Pq,
            wgpu::SurfaceColorSpace::Bt2100Hlg => ScenePresentationColorSpace::Bt2100Hlg,
            _ => ScenePresentationColorSpace::Srgb,
        };
        let profile = ScenePresentationProfile::new(
            nana_gpu::__framework::format_from_wgpu(self.format),
            color_space,
        );
        if (matches!(
            self.requested,
            SurfacePresentationPolicy::Hdr
                | SurfacePresentationPolicy::HdrPq
                | SurfacePresentationPolicy::HdrHlg
        ) && !self.is_hdr())
            || (matches!(self.requested, SurfacePresentationPolicy::WideGamut)
                && matches!(color_space, ScenePresentationColorSpace::Srgb))
        {
            profile.with_force_float_working()
        } else {
            profile
        }
    }
}

/// Resolve a host presentation policy against the surface's advertised
/// `(format, color_space)` capability pairs.
///
/// `format_capabilities` is the authoritative selector because HDR and
/// wide-gamut contracts depend on both format and colour space. PQ/HLG pairs
/// are only selected when explicitly requested by [`SurfacePresentationPolicy`];
/// the two-alpha-bit `Rgb10a2Unorm` target is excluded for transparent windows,
/// while fp16 PQ/HLG remains eligible when its exact capability pair is
/// advertised. `formats` is retained as a fallback for adapters that do not
/// populate the extended table.
#[cfg(feature = "wgpu-interop")]
pub fn resolve_surface_profile(
    requested: SurfacePresentationPolicy,
    capabilities: &wgpu::SurfaceCapabilities,
    transparent: bool,
) -> Option<ResolvedSurfaceProfile> {
    resolve_surface_profile_internal(requested, capabilities, transparent)
}

pub(crate) fn resolve_surface_profile_internal(
    requested: SurfacePresentationPolicy,
    capabilities: &wgpu::SurfaceCapabilities,
    transparent: bool,
) -> Option<ResolvedSurfaceProfile> {
    use wgpu::{SurfaceColorSpace as C, SurfaceColorSpaces as S, TextureFormat as F};

    let srgb_formats = [
        F::Rgba8UnormSrgb,
        F::Bgra8UnormSrgb,
        F::Rgba8Unorm,
        F::Bgra8Unorm,
    ];
    let mut rows: Vec<(F, wgpu::SurfaceColorSpaces)> = capabilities
        .format_capabilities
        .iter()
        .map(|row| (row.format, row.color_spaces))
        .collect();
    if rows.is_empty() {
        rows = capabilities
            .formats
            .iter()
            .copied()
            .filter(|format| srgb_formats.contains(format))
            .map(|format| (format, S::SRGB))
            .collect();
    }
    if rows.is_empty() {
        return None;
    }

    // Display-P3 may be advertised with either an untyped UNORM format (the
    // shader owns the OETF), an sRGB-typed format (the shader writes linear
    // values and the attachment applies the OETF), or fp16 (the shader owns
    // the transfer and preserves the extended working range). Keep the
    // 8-bit choices first for WideGamut's SDR preference, but retain fp16 as
    // a capability fallback for HDR-capable P3 surfaces.
    let p3_formats = [
        F::Bgra8Unorm,
        F::Rgba8Unorm,
        F::Bgra8UnormSrgb,
        F::Rgba8UnormSrgb,
        F::Rgba16Float,
    ];
    let hdr_formats = [F::Rgba16Float];
    let broadcast_hdr_formats = [F::Rgb10a2Unorm, F::Rgba16Float];
    let mut candidates: Vec<(C, &[F])> = match requested {
        SurfacePresentationPolicy::Hdr => vec![
            (C::ExtendedSrgb, &hdr_formats),
            (C::ExtendedSrgbLinear, &hdr_formats),
            (C::ExtendedDisplayP3, &hdr_formats),
            (C::DisplayP3, &p3_formats),
            (C::Srgb, &srgb_formats),
        ],
        SurfacePresentationPolicy::HdrPq | SurfacePresentationPolicy::HdrHlg => {
            let preferred = match requested {
                SurfacePresentationPolicy::HdrPq => C::Bt2100Pq,
                _ => C::Bt2100Hlg,
            };
            vec![
                (preferred, &broadcast_hdr_formats),
                (C::ExtendedSrgbLinear, &hdr_formats),
                (C::ExtendedSrgb, &hdr_formats),
                (C::ExtendedDisplayP3, &hdr_formats),
                (C::DisplayP3, &p3_formats),
                (C::Srgb, &srgb_formats),
            ]
        }
        SurfacePresentationPolicy::WideGamut => {
            vec![(C::DisplayP3, &p3_formats), (C::Srgb, &srgb_formats)]
        }
        SurfacePresentationPolicy::Sdr | SurfacePresentationPolicy::Auto => {
            vec![(C::Srgb, &srgb_formats)]
        }
    };
    let mut rejected_for_transparency = false;
    for (index, (space, formats)) in candidates.drain(..).enumerate() {
        let flag = space.to_color_spaces()?;
        if let Some((format, _)) = formats.iter().find_map(|wanted| {
            // Providers should emit one row per format, but a few native
            // adapters have historically split the color-space flags across
            // duplicate rows. Union rows before matching so a valid pair is
            // not lost merely because the table was partitioned.
            let spaces = rows
                .iter()
                .filter(|(format, _)| format == wanted)
                .fold(S::empty(), |acc, (_, spaces)| acc | *spaces);
            if !spaces.contains(flag) {
                return None;
            }
            // `Rgb10a2Unorm` has only two alpha bits and is not a stable
            // premultiplied-alpha contract for transparent composition. An
            // fp16 PQ/HLG target carries full alpha on platforms that
            // advertise that exact pair, so keep it eligible here and let the
            // surface's negotiated alpha mode remain the authority.
            if transparent
                && matches!(space, C::Bt2100Pq | C::Bt2100Hlg)
                && *wanted == F::Rgb10a2Unorm
            {
                if index == 0 {
                    rejected_for_transparency = true;
                }
                return None;
            }
            Some((wanted, spaces))
        }) {
            let fallback_reason = if rejected_for_transparency {
                SurfaceProfileFallback::TransparencyIncompatible
            } else {
                SurfaceProfileFallback::CapabilityUnavailable
            };
            return Some(ResolvedSurfaceProfile {
                requested,
                format: *format,
                color_space: space,
                fallback: (index != 0).then_some(fallback_reason),
            });
        }
    }
    // Old/native providers may expose another sRGB format. Keep that valid
    // pair, but never manufacture an sRGB contract for a PQ/HLG-only format.
    // A surface without any supported pair is incompatible with this policy.
    let format = rows
        .iter()
        .find(|(format, spaces)| srgb_formats.contains(format) && spaces.contains(S::SRGB))
        .map(|(format, _)| *format)
        // A malformed native table may omit the default-SDR rows while still
        // exposing them through `formats`. Only derive sRGB for the known
        // 8-bit surface formats; never relabel fp16 or 10-bit HDR formats.
        .or_else(|| {
            capabilities
                .formats
                .iter()
                .copied()
                .find(|format| srgb_formats.contains(format))
        })?;
    Some(ResolvedSurfaceProfile {
        requested,
        format,
        color_space: C::Srgb,
        fallback: Some(if rejected_for_transparency {
            SurfaceProfileFallback::TransparencyIncompatible
        } else {
            SurfaceProfileFallback::CapabilityUnavailable
        }),
    })
}

/// Whether a profile transition requires reconfiguring the swapchain.
/// Resizes and present-mode policy remain separate concerns; an identical
/// profile is explicitly a no-op.
pub fn surface_profile_changed(
    current: ResolvedSurfaceProfile,
    next: ResolvedSurfaceProfile,
) -> bool {
    current != next
}
