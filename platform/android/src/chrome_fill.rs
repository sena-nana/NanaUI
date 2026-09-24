//! Shell chrome fill helpers (scissor clamp + band plan).
//!
//! Host wgpu paints solid colors into [`crate::shell::ShellChromeBand`] rects.
//! Full Nana `DesktopShell` remains deferred.

use nana_ui_core::PhysicalRect;

use crate::shell::ShellChromeBand;

/// Clamp a physical band into the framebuffer; `None` if empty after clamp.
pub fn clamp_scissor(
    rect: PhysicalRect,
    frame_w: u32,
    frame_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    if frame_w == 0 || frame_h == 0 || rect.width == 0 || rect.height == 0 {
        return None;
    }
    if rect.x >= frame_w || rect.y >= frame_h {
        return None;
    }
    let x = rect.x;
    let y = rect.y;
    let w = rect.width.min(frame_w.saturating_sub(x));
    let h = rect.height.min(frame_h.saturating_sub(y));
    if w == 0 || h == 0 {
        None
    } else {
        Some((x, y, w, h))
    }
}

/// Convert chrome bands into scissor + premultiplied-ready RGBA clears.
pub fn band_draw_list(
    bands: &[ShellChromeBand],
    frame_w: u32,
    frame_h: u32,
) -> Vec<(u32, u32, u32, u32, [f64; 4])> {
    let mut out = Vec::with_capacity(bands.len());
    for band in bands {
        if let Some((x, y, w, h)) = clamp_scissor(band.rect, frame_w, frame_h) {
            out.push((x, y, w, h, band.color));
        }
    }
    out
}

/// Bytes of one band's `vec4<f32>` fill color.
pub(crate) const FILL_COLOR_SIZE: u32 = 16;

/// Per-band slot in the dynamic-offset color buffer.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub(crate) fn fill_color_stride(min_uniform_offset_alignment: u32) -> u32 {
    FILL_COLOR_SIZE.next_multiple_of(min_uniform_offset_alignment.max(1))
}

/// Dynamic offset of band `band`'s color.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub(crate) fn fill_color_offset(band: usize, stride: u32) -> u32 {
    u32::try_from(band)
        .ok()
        .and_then(|band| band.checked_mul(stride))
        .expect("chrome fill color offset exceeds u32")
}

/// Pack colors one slot each (little-endian, as uniform buffers are) so a
/// single queue write lands them all before the frame executes.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub(crate) fn pack_fill_colors(
    out: &mut Vec<u8>,
    colors: impl IntoIterator<Item = [f64; 4]>,
    stride: u32,
) {
    out.clear();
    for color in colors {
        let slot = out.len();
        out.resize(slot + stride as usize, 0);
        for (c, v) in color.into_iter().enumerate() {
            out[slot + c * 4..][..4].copy_from_slice(&(v as f32).to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::AndroidShellStub;

    #[test]
    fn clamp_scissor_rejects_empty_and_oob() {
        assert!(
            clamp_scissor(
                PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 10
                },
                100,
                100
            )
            .is_none()
        );
        assert!(
            clamp_scissor(
                PhysicalRect {
                    x: 200,
                    y: 0,
                    width: 10,
                    height: 10
                },
                100,
                100
            )
            .is_none()
        );
    }

    #[test]
    fn clamp_scissor_clips_to_frame() {
        let (x, y, w, h) = clamp_scissor(
            PhysicalRect {
                x: 80,
                y: 90,
                width: 50,
                height: 50,
            },
            100,
            100,
        )
        .expect("clipped");
        assert_eq!((x, y, w, h), (80, 90, 20, 10));
    }

    #[test]
    fn band_draw_list_from_shell_has_distinct_regions() {
        let mut shell = AndroidShellStub::new();
        shell.resize(1080, 1920, 2.0);
        let bands = shell.chrome_bands();
        let draws = band_draw_list(&bands, 1080, 1920);
        assert!(
            draws.len() >= 2,
            "expected multiple chrome fills, got {}",
            draws.len()
        );
        // Title band starts at top-left.
        assert_eq!(draws[0].0, 0);
        assert_eq!(draws[0].1, 0);
        // Colors must differ between title and body (visual chrome signal).
        let colors: Vec<_> = draws.iter().map(|d| d.4).collect();
        assert!(
            colors.windows(2).any(|w| w[0] != w[1]),
            "chrome bands should use distinct fill colors"
        );
    }

    #[test]
    fn fill_color_stride_rounds_up_to_offset_alignment() {
        assert_eq!(fill_color_stride(256), 256);
        assert_eq!(fill_color_stride(4), 16);
        assert_eq!(fill_color_stride(0), 16);
    }

    #[test]
    #[should_panic(expected = "exceeds u32")]
    fn fill_color_offset_rejects_offsets_past_u32() {
        fill_color_offset(1 << 24, 256);
    }

    #[test]
    fn each_band_offset_reads_back_its_own_color() {
        let colors = [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 0.5],
            [0.25, 0.5, 0.75, 1.0],
        ];
        let stride = fill_color_stride(256);
        let mut bytes = vec![0xAA; 7];
        pack_fill_colors(&mut bytes, colors, stride);
        assert_eq!(bytes.len(), colors.len() * stride as usize);
        for (band, color) in colors.iter().enumerate() {
            let at = fill_color_offset(band, stride) as usize;
            let read: [f32; 4] = std::array::from_fn(|c| {
                f32::from_le_bytes(bytes[at + c * 4..][..4].try_into().unwrap())
            });
            assert_eq!(read, color.map(|v| v as f32), "band {band}");
        }
    }
}
