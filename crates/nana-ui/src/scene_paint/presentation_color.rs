//! Transfer functions used at the Scene presentation boundary.
//!
//! Scene composition is always linear scRGB.  These helpers intentionally do
//! not perform alpha handling or gamut clipping; callers must unpremultiply,
//! transfer/map the colour, and premultiply again at the final boundary.
//! Keeping the scalar curves here makes the CPU clear path and the shader
//! implementation easy to compare and gives hosts a small, pure reference
//! implementation for capability and numerical tests.

// BT.2100 publishes these constants with more decimal places than f32's
// shortest-round-trip spelling; retain the reference values for CPU/GPU
// parity and numerical vectors.
#![allow(clippy::excessive_precision)]

const PQ_M1: f32 = 0.159_301_757_812_5;
const PQ_M2: f32 = 78.843_75;
const PQ_C1: f32 = 0.835_937_5;
const PQ_C2: f32 = 18.851_562_5;
const PQ_C3: f32 = 18.687_5;

const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 0.284_668_92;
const HLG_C: f32 = 0.559_910_73;
const HLG_KNEE: f32 = 1.0 / 12.0;

/// Runtime display parameters for a presentation transfer.
///
/// These values intentionally do not belong to [`ScenePresentationProfile`]
/// (which is a pipeline-cache key). A display may change EDR headroom while a
/// window remains on the same surface; hosts update this small parameter block
/// without rebuilding scene pipelines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScenePresentationParameters {
    /// Multiplicative range above SDR reference white. `1.0` is an SDR
    /// fallback; the default `16.0` is a conservative HDR budget when the
    /// platform has no live metadata.
    pub headroom: f32,
    /// Luminance represented by one linear scRGB unit, in nits.
    pub reference_white_nits: f32,
}

impl Default for ScenePresentationParameters {
    fn default() -> Self {
        Self {
            headroom: 16.0,
            reference_white_nits: 80.0,
        }
    }
}

impl ScenePresentationParameters {
    pub const fn new(headroom: f32, reference_white_nits: f32) -> Self {
        Self {
            headroom,
            reference_white_nits,
        }
    }

    /// Return a copy with a new runtime display headroom value.
    ///
    /// The value is normalized when handed to the painter/uniform, so this
    /// builder remains const and cheap for hosts that receive metadata in
    /// separate callbacks.
    #[must_use]
    pub const fn with_headroom(self, headroom: f32) -> Self {
        Self { headroom, ..self }
    }

    /// Return a copy with a new SDR reference-white luminance in nits.
    #[must_use]
    pub const fn with_reference_white_nits(self, reference_white_nits: f32) -> Self {
        Self {
            reference_white_nits,
            ..self
        }
    }

    /// Normalize untrusted display metadata to finite, conservative bounds.
    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            headroom: finite_bound(self.headroom, 1.0, 125.0, 16.0),
            reference_white_nits: finite_bound(self.reference_white_nits, 1.0, 10_000.0, 80.0),
        }
    }

    #[must_use]
    pub fn headroom(self) -> f32 {
        self.normalized().headroom
    }

    #[must_use]
    pub fn reference_white_nits(self) -> f32 {
        self.normalized().reference_white_nits
    }

    /// Layout for the destination blit uniform (`headroom`, reference white,
    /// followed by two reserved lanes for future display metadata).
    #[must_use]
    pub fn to_uniform(self) -> [f32; 4] {
        let normalized = self.normalized();
        [
            normalized.headroom,
            normalized.reference_white_nits,
            0.0,
            0.0,
        ]
    }
}

/// BT.709/sRGB linear RGB to BT.2020 linear RGB matrix (D65).
///
/// The matrix is represented in row-major form in this CPU helper.  The
/// corresponding WGSL constant is column-major, as required by WGSL matrix
/// multiplication.
pub const BT709_TO_BT2020: [[f32; 3]; 3] = [
    [0.627_402, 0.329_292, 0.043_306],
    [0.069_095, 0.919_544, 0.011_360],
    [0.016_394, 0.088_028, 0.895_578],
];

/// Encode normalized absolute luminance with SMPTE ST 2084 (PQ).
///
/// `normalized_luminance` is luminance divided by 10,000 nits.  Inputs are
/// finite-clamped to `[0, 1]`, and the return value is in `[0, 1]`.
#[must_use]
pub fn pq_encode(normalized_luminance: f32) -> f32 {
    let y = finite_clamp(normalized_luminance);
    if y <= 0.0 {
        return 0.0;
    }
    let yp = y.powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * yp) / (1.0 + PQ_C3 * yp)).powf(PQ_M2)
}

/// Alias using the transfer-function terminology used by BT.2100 and wgpu.
#[must_use]
pub fn pq_oetf(normalized_luminance: f32) -> f32 {
    pq_encode(normalized_luminance)
}

/// Decode a normalized ST 2084 code value into luminance divided by 10,000
/// nits.  Inputs and output are finite-clamped to `[0, 1]`.
#[must_use]
pub fn pq_decode(encoded: f32) -> f32 {
    let n = finite_clamp(encoded).powf(1.0 / PQ_M2);
    let numerator = (n - PQ_C1).max(0.0);
    let denominator = (PQ_C2 - PQ_C3 * n).max(f32::MIN_POSITIVE);
    (numerator / denominator).powf(1.0 / PQ_M1).clamp(0.0, 1.0)
}

/// Alias using the transfer-function terminology used by BT.2100 and wgpu.
#[must_use]
pub fn pq_eotf(encoded: f32) -> f32 {
    pq_decode(encoded)
}

/// Encode absolute luminance in nits with PQ (10000 nits is code value 1).
#[must_use]
pub fn pq_encode_nits(nits: f32) -> f32 {
    pq_encode(nits / 10_000.0)
}

/// Decode PQ into absolute luminance in nits (code value 1 is 10000 nits).
#[must_use]
pub fn pq_decode_nits(encoded: f32) -> f32 {
    pq_decode(encoded) * 10_000.0
}

/// Encode normalized scene luminance with BT.2100 HLG OETF.
///
/// HLG is a relative signal: `1.0` denotes the nominal 1000-nit peak.  The
/// input is therefore scene luminance divided by that nominal peak.
#[must_use]
pub fn hlg_encode(scene_luminance: f32) -> f32 {
    let y = finite_clamp(scene_luminance);
    if y <= HLG_KNEE {
        (3.0 * y).sqrt()
    } else {
        HLG_A * (12.0 * y - HLG_B).ln() + HLG_C
    }
}

/// Alias using the transfer-function terminology used by BT.2100 and wgpu.
#[must_use]
pub fn hlg_oetf(scene_luminance: f32) -> f32 {
    hlg_encode(scene_luminance)
}

/// Decode a normalized BT.2100 HLG signal into normalized scene luminance.
#[must_use]
pub fn hlg_decode(encoded: f32) -> f32 {
    let e = finite_clamp(encoded);
    if e <= 0.5 {
        e * e / 3.0
    } else {
        (((e - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
    .clamp(0.0, 1.0)
}

/// Alias using the transfer-function terminology used by BT.2100 and wgpu.
#[must_use]
pub fn hlg_eotf(encoded: f32) -> f32 {
    hlg_decode(encoded)
}

/// Convert linear BT.709/scRGB channels to linear BT.2020 channels.
#[must_use]
pub fn linear_sc_rgb_to_bt2020(rgb: [f32; 3]) -> [f32; 3] {
    BT709_TO_BT2020.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2])
}

/// Apply a bounded highlight shoulder while leaving SDR UI values unchanged.
///
/// A shared scale is applied only to channels above SDR reference white. This
/// keeps neutral highlights and their chroma direction stable without lifting
/// unrelated midtone channels when one highlight is present. `headroom == 1`
/// is the SDR limit; larger finite values roll highlights toward the
/// available peak.
#[must_use]
pub fn tone_map_headroom_rgb(mut rgb: [f32; 3], headroom: f32) -> [f32; 3] {
    // Custom GPU nodes and host supplied clear colours can contain NaN or
    // infinity.  Sanitise before finding the shared peak so an infinite
    // channel cannot turn the scale into `0 * infinity = NaN`.
    for channel in &mut rgb {
        // Match the presentation shader's finite-value guard.  Besides NaN
        // and infinity, reject magnitudes large enough to overflow the
        // exponential/scale arithmetic on a backend with relaxed FP rules.
        if !channel.is_finite() || channel.abs() >= 1.0e20 {
            *channel = 0.0;
        }
    }
    let peak = rgb
        .into_iter()
        .fold(0.0_f32, |peak, channel| peak.max(channel));
    if peak <= 1.0 {
        return rgb;
    }
    let headroom = if headroom.is_finite() {
        headroom.clamp(1.0, 125.0)
    } else {
        1.0
    };
    let mapped_peak = if headroom <= 1.0 {
        1.0
    } else {
        1.0 + (headroom - 1.0) * (1.0 - (-(peak - 1.0) / (headroom - 1.0)).exp())
    };
    // Scale the excursion above white, not the whole channel. This anchors
    // every channel at 1.0 even when another channel has a much larger peak,
    // avoiding a discontinuity as a channel crosses the SDR/HDR boundary.
    let scale = (mapped_peak - 1.0) / (peak - 1.0);
    for channel in &mut rgb {
        if *channel > 1.0 {
            *channel = 1.0 + (*channel - 1.0) * scale;
        }
    }
    rgb
}

#[inline]
fn finite_clamp(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else if value.is_nan() {
        0.0
    } else if value.is_sign_positive() {
        1.0
    } else {
        0.0
    }
}

#[inline]
fn finite_bound(value: f32, low: f32, high: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_parameters_normalize_metadata_without_cache_identity() {
        let params = ScenePresentationParameters::new(f32::NAN, f32::INFINITY).normalized();
        assert_eq!(params, ScenePresentationParameters::default());
        let clamped = ScenePresentationParameters::new(0.25, 20_000.0).normalized();
        assert_eq!(clamped.headroom(), 1.0);
        assert_eq!(clamped.reference_white_nits(), 10_000.0);
        assert_eq!(
            ScenePresentationParameters::default()
                .with_headroom(4.0)
                .with_reference_white_nits(100.0)
                .to_uniform(),
            [4.0, 100.0, 0.0, 0.0]
        );
        assert_eq!(
            ScenePresentationParameters::new(4.0, 80.0).to_uniform(),
            [4.0, 80.0, 0.0, 0.0]
        );
    }

    #[test]
    fn pq_reference_vectors_and_round_trip() {
        // SMPTE ST 2084 reference values, with tolerance for f32/libm.
        for (nits, expected) in [
            (0.0, 0.0),
            (100.0, 0.508_078_4),
            (1_000.0, 0.751_827_1),
            (10_000.0, 1.0),
        ] {
            let encoded = pq_encode_nits(nits);
            assert!(
                (encoded - expected).abs() < 2.0e-4,
                "{nits}: {encoded} vs {expected}"
            );
            assert!((pq_decode_nits(encoded) - nits).abs() < nits.max(1.0) * 3.0e-4);
        }
        assert_eq!(pq_encode_nits(f32::NAN), 0.0);
        assert_eq!(pq_encode_nits(f32::INFINITY), 1.0);
    }

    #[test]
    fn hlg_reference_vectors_and_round_trip() {
        for (scene, expected) in [
            (0.0, 0.0),
            (1.0 / 12.0, 0.5),
            (0.18, 0.672_358_3),
            (1.0, 1.0),
        ] {
            let encoded = hlg_encode(scene);
            assert!(
                (encoded - expected).abs() < 3.0e-4,
                "{scene}: {encoded} vs {expected}"
            );
            assert!((hlg_decode(encoded) - scene).abs() < 3.0e-4);
        }
        assert_eq!(hlg_encode(f32::NAN), 0.0);
        assert_eq!(hlg_decode(f32::INFINITY), 1.0);
    }

    #[test]
    fn bt2020_matrix_preserves_white_and_maps_red() {
        let white = linear_sc_rgb_to_bt2020([1.0, 1.0, 1.0]);
        assert!(
            white
                .into_iter()
                .all(|channel| (channel - 1.0).abs() < 1.0e-5)
        );
        let red = linear_sc_rgb_to_bt2020([1.0, 0.0, 0.0]);
        assert!((red[0] - 0.627_402).abs() < 1.0e-6);
        assert!((red[1] - 0.069_095).abs() < 1.0e-6);
        assert!((red[2] - 0.016_394).abs() < 1.0e-6);
    }

    #[test]
    fn headroom_shoulder_keeps_reference_white_and_chroma() {
        assert_eq!(
            tone_map_headroom_rgb([1.0, 0.5, 0.0], 16.0),
            [1.0, 0.5, 0.0]
        );
        let mapped = tone_map_headroom_rgb([4.0, 2.0, 1.0], 4.0);
        assert!(mapped[0] < 4.0 && mapped[0] > 1.0);
        let excess_ratio = (mapped[1] - 1.0) / (mapped[0] - 1.0);
        assert!((excess_ratio - 1.0 / 3.0).abs() < 1.0e-6);
        assert_eq!(tone_map_headroom_rgb([2.0, 1.0, 0.0], 1.0), [1.0, 1.0, 0.0]);
        assert_eq!(
            tone_map_headroom_rgb([f32::INFINITY, 0.5, f32::NAN], 16.0),
            [0.0, 0.5, 0.0]
        );
        assert_eq!(
            tone_map_headroom_rgb([2.0, 0.18, 0.18], 4.0)[1..],
            [0.18, 0.18]
        );
        assert_eq!(
            tone_map_headroom_rgb([f32::MAX, 0.5, -f32::MAX], 16.0),
            [0.0, 0.5, 0.0]
        );
        let at_white = tone_map_headroom_rgb([100.0, 1.0, 0.5], 4.0);
        let above_white = tone_map_headroom_rgb([100.0, 1.000_01, 0.5], 4.0);
        assert!((above_white[1] - at_white[1]).abs() < 1.0e-5);
        assert!(above_white[1] >= 1.0);
    }
}
