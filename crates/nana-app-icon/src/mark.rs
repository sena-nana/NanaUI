//! Shared Nana artwork for runtime icons and packaged ICO/ICNS resources.

use std::sync::OnceLock;

use image::{RgbaImage, imageops};

/// Rasterize the default mark as unpremultiplied RGBA.
pub fn rasterize(size: u32) -> Vec<u8> {
    static SOURCE: OnceLock<RgbaImage> = OnceLock::new();
    let source = SOURCE.get_or_init(|| {
        let mut source = image::load_from_memory(include_bytes!("../assets/nana.png"))
            .expect("bundled Nana icon must be a valid PNG")
            .into_rgba8();
        // Filter in premultiplied space to avoid dark fringes at transparent edges.
        for pixel in source.pixels_mut() {
            let alpha = u16::from(pixel[3]);
            for channel in &mut pixel.0[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        source
    });
    let size = size.max(1);
    // The source already has a small transparent border. A 90% image box puts
    // the visible plate at approximately 83% of the canvas, matching our grid.
    let content = ((size as f64 * 0.90).round() as u32).max(1);
    let scale = content as f64 / source.width().max(source.height()) as f64;
    let width = ((source.width() as f64 * scale).round() as u32).max(1);
    let height = ((source.height() as f64 * scale).round() as u32).max(1);
    let mut resized = imageops::resize(source, width, height, imageops::FilterType::Lanczos3);
    for pixel in resized.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = if alpha == 0 {
                0
            } else {
                ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8
            };
        }
    }
    let mut canvas = RgbaImage::new(size, size);
    imageops::overlay(
        &mut canvas,
        &resized,
        ((size - width) / 2) as i64,
        ((size - height) / 2) as i64,
    );
    canvas.into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_preserves_transparency_at_runtime_and_package_sizes() {
        for size in [16, 32, 48, 128, 256, 512] {
            let pixels = rasterize(size);
            assert_eq!(pixels.len(), (size * size * 4) as usize);
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 245));
            for i in 0..size as usize {
                let side = size as usize;
                for index in [i, (side - 1) * side + i, i * side, i * side + side - 1] {
                    assert_eq!(pixels[index * 4 + 3], 0, "transparent margin at {size}px");
                }
            }
        }
    }
}
