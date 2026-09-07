//! Pixel verdicts, so "did anything paint" is a number rather than a judgement.
//!
//! The headless skill used to tell an Agent to open the PNG and treat a flat
//! clear as a failure. That is a human judgement an Agent cannot make reliably,
//! and it was recomputed inline in three tests. It belongs in one place and in
//! every screenshot reply.

use super::protocol::{PixelDiff, PixelStats, RectDump};
use crate::offscreen::Size;

/// RGBA8, row-major, `width * height * 4` bytes.
pub fn pixel_stats(size: Size<u32>, pixels: &[u8], clear: [f32; 4]) -> PixelStats {
    let clear8 = [
        channel_u8(clear[0]),
        channel_u8(clear[1]),
        channel_u8(clear[2]),
    ];
    let mut seen = std::collections::HashSet::new();
    let mut nonclear = 0u64;
    let mut luma = 0f64;
    let (rgba, _) = pixels.as_chunks::<4>();
    for pixel in rgba {
        seen.insert(u32::from_be_bytes([0, pixel[0], pixel[1], pixel[2]]));
        // A tolerance of 2 absorbs sRGB round-tripping through the clear colour
        // without absorbing anything a real surface would paint.
        if pixel[..3]
            .iter()
            .zip(clear8)
            .any(|(value, base)| value.abs_diff(base) > 2)
        {
            nonclear += 1;
        }
        luma += 0.2126 * f64::from(pixel[0])
            + 0.7152 * f64::from(pixel[1])
            + 0.0722 * f64::from(pixel[2]);
    }
    let total = rgba.len().max(1) as f64;
    PixelStats {
        width: size.width,
        height: size.height,
        unique_colors: seen.len(),
        nonclear_ratio: (nonclear as f64 / total) as f32,
        mean_luma: (luma / total / 255.0) as f32,
    }
}

/// Exact-pixel difference. `tolerance` is a per-channel allowance.
///
/// In-process and dependency-free, unlike `scripts/pixel_ssim_compare.sh`,
/// which needs ImageMagick and reports perceptual similarity instead.
pub fn pixel_diff(
    size: Size<u32>,
    baseline: &[u8],
    candidate: &[u8],
    tolerance: u8,
) -> Result<PixelDiff, String> {
    if baseline.len() != candidate.len() {
        return Err(format!(
            "frame sizes differ: {} vs {} bytes",
            baseline.len(),
            candidate.len()
        ));
    }
    let (base, _) = baseline.as_chunks::<4>();
    let (cand, _) = candidate.as_chunks::<4>();
    let mut changed = 0u64;
    let mut max_delta = 0u8;
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (index, (a, b)) in base.iter().zip(cand).enumerate() {
        let delta = a
            .iter()
            .zip(b)
            .map(|(left, right)| left.abs_diff(*right))
            .max()
            .unwrap_or(0);
        if delta <= tolerance {
            continue;
        }
        changed += 1;
        max_delta = max_delta.max(delta);
        let x = index as u32 % size.width.max(1);
        let y = index as u32 / size.width.max(1);
        bounds = Some(match bounds {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }
    let total = base.len().max(1) as f64;
    Ok(PixelDiff {
        changed_pixels: changed,
        changed_ratio: (changed as f64 / total) as f32,
        max_channel_delta: max_delta,
        bbox: bounds.map(|(x0, y0, x1, y1)| RectDump {
            x: x0 as f32,
            y: y0 as f32,
            width: (x1 - x0 + 1) as f32,
            height: (y1 - y0 + 1) as f32,
        }),
    })
}

fn channel_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_clear_frame_reports_one_colour_and_no_coverage() {
        let clear = [0.5, 0.5, 0.5, 1.0];
        let pixels = [128, 128, 128, 255].repeat(16);
        let stats = pixel_stats(Size::new(4, 4), &pixels, clear);
        assert_eq!(stats.unique_colors, 1);
        assert_eq!(stats.nonclear_ratio, 0.0);
    }

    #[test]
    fn one_painted_pixel_is_visible_in_both_the_count_and_the_ratio() {
        let clear = [0.0, 0.0, 0.0, 1.0];
        let mut pixels = [0, 0, 0, 255].repeat(16);
        pixels[0..4].copy_from_slice(&[255, 0, 0, 255]);
        let stats = pixel_stats(Size::new(4, 4), &pixels, clear);
        assert_eq!(stats.unique_colors, 2);
        assert!((stats.nonclear_ratio - 1.0 / 16.0).abs() < 1e-6);
    }

    #[test]
    fn diff_locates_the_changed_region() {
        let base = [0, 0, 0, 255].repeat(16);
        let mut candidate = base.clone();
        // (2, 1) in a 4x4 frame.
        let offset = (4 + 2) * 4;
        candidate[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
        let diff = pixel_diff(Size::new(4, 4), &base, &candidate, 0).expect("same size");
        assert_eq!(diff.changed_pixels, 1);
        assert_eq!(diff.max_channel_delta, 255);
        let bbox = diff.bbox.expect("a changed pixel has a bounding box");
        assert_eq!(
            (bbox.x, bbox.y, bbox.width, bbox.height),
            (2.0, 1.0, 1.0, 1.0)
        );
    }

    #[test]
    fn diff_refuses_mismatched_frames_instead_of_reporting_a_number() {
        let error = pixel_diff(Size::new(4, 4), &[0; 64], &[0; 16], 0).unwrap_err();
        assert!(error.contains("differ"), "{error}");
    }
}
