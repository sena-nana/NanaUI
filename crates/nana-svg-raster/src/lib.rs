//! CPU SVG raster (resvg → pixmap). Not a painter and not an SVG DOM.

use std::sync::Arc;

/// Premultiplied RGBA8 pixmap, or straight alpha from [`rasterize_document`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterizedSvg {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

/// Embedded font for SVG `<text>`.
#[derive(Debug, Clone, Copy)]
pub struct SvgFont<'a> {
    pub bytes: &'a [u8],
    pub family: &'a str,
}

/// Vue generic `<svg>`: stretch independently to `width`×`height`.
pub fn rasterize_stretch(
    markup: &str,
    width: u32,
    height: u32,
    font: Option<SvgFont<'_>>,
    max_edge: u32,
) -> Option<RasterizedSvg> {
    let width = width.clamp(1, max_edge.max(1));
    let height = height.clamp(1, max_edge.max(1));
    render(
        Source::Str(markup),
        Some((width as f32, height as f32)),
        font,
        None,
        Fit::Stretch { width, height },
        None,
        false,
    )
}

/// Icon atlas: contain in a square, `currentColor` → white mask.
pub fn rasterize_white_mask(svg: &str, pixel_size: u32, max_edge: u32) -> Option<Vec<u8>> {
    let raster = render(
        Source::Str(svg),
        Some((24.0, 24.0)),
        None,
        Some("#ffffff"),
        Fit::Contain {
            pixel_size: pixel_size.min(max_edge.max(1)),
        },
        Some([255, 255, 255]),
        false,
    )?;
    Some(raster.rgba.to_vec())
}

/// Canvas Image: document size, straight alpha.
pub fn rasterize_document(bytes: &[u8]) -> Option<RasterizedSvg> {
    render(
        Source::Bytes(bytes),
        None,
        None,
        None,
        Fit::Document,
        None,
        true,
    )
}

/// URL image decode: document size, straight alpha, contained within `max_edge`.
pub fn rasterize_document_capped(bytes: &[u8], max_edge: u32) -> Option<RasterizedSvg> {
    render(
        Source::Bytes(bytes),
        None,
        None,
        None,
        Fit::DocumentCapped { max_edge },
        None,
        true,
    )
}

enum Source<'a> {
    Str(&'a str),
    Bytes(&'a [u8]),
}

enum Fit {
    Document,
    DocumentCapped { max_edge: u32 },
    Stretch { width: u32, height: u32 },
    Contain { pixel_size: u32 },
}

fn render(
    source: Source<'_>,
    default_size: Option<(f32, f32)>,
    font: Option<SvgFont<'_>>,
    replace_current_color: Option<&str>,
    fit: Fit,
    force_rgb: Option<[u8; 3]>,
    unpremultiply: bool,
) -> Option<RasterizedSvg> {
    let mut options = resvg::usvg::Options::default();
    // The default string resolver `fs::read`s absolute/cwd paths. Keep data: only.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    if let Some((width, height)) = default_size {
        options.default_size = resvg::usvg::Size::from_wh(width, height)?;
    }
    if let Some(font) = font {
        let mut fontdb = resvg::usvg::fontdb::Database::new();
        fontdb.load_font_data(font.bytes.to_vec());
        fontdb.set_sans_serif_family(font.family);
        options.font_family = font.family.into();
        options.fontdb = Arc::new(fontdb);
    }
    let tree = match (source, replace_current_color) {
        (Source::Str(markup), Some(color)) => {
            resvg::usvg::Tree::from_str(&markup.replace("currentColor", color), &options).ok()?
        }
        (Source::Str(markup), None) => resvg::usvg::Tree::from_str(markup, &options).ok()?,
        (Source::Bytes(bytes), Some(color)) => {
            let markup = std::str::from_utf8(bytes).ok()?;
            resvg::usvg::Tree::from_str(&markup.replace("currentColor", color), &options).ok()?
        }
        (Source::Bytes(bytes), None) => resvg::usvg::Tree::from_data(bytes, &options).ok()?,
    };
    let size = tree.size();
    let (width, height, transform) = match fit {
        Fit::Document => {
            let int = size.to_int_size();
            if int.width() == 0 || int.height() == 0 {
                return None;
            }
            (int.width(), int.height(), tiny_skia::Transform::identity())
        }
        Fit::DocumentCapped { max_edge } => {
            let src_w = size.width().max(f32::EPSILON);
            let src_h = size.height().max(f32::EPSILON);
            let max_edge = max_edge.max(1) as f32;
            let scale = (max_edge / src_w).min(max_edge / src_h).min(1.0);
            let width = (src_w * scale).round().clamp(1.0, max_edge) as u32;
            let height = (src_h * scale).round().clamp(1.0, max_edge) as u32;
            (
                width,
                height,
                tiny_skia::Transform::from_scale(width as f32 / src_w, height as f32 / src_h),
            )
        }
        Fit::Stretch { width, height } => {
            let sx = width as f32 / size.width().max(f32::EPSILON);
            let sy = height as f32 / size.height().max(f32::EPSILON);
            (width, height, tiny_skia::Transform::from_scale(sx, sy))
        }
        Fit::Contain { pixel_size } => {
            if pixel_size == 0 {
                return None;
            }
            let fit = size.width().max(size.height()).max(f32::EPSILON);
            let scale = pixel_size as f32 / fit;
            let dx = (pixel_size as f32 - size.width() * scale) * 0.5;
            let dy = (pixel_size as f32 - size.height() * scale) * 0.5;
            (
                pixel_size,
                pixel_size,
                tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy),
            )
        }
    };
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut rgba = pixmap.data().to_vec();
    if let Some([r, g, b]) = force_rgb {
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel[0] = r;
            pixel[1] = g;
            pixel[2] = b;
        }
    }
    if unpremultiply {
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let a = pixel[3] as u16;
            if a > 0 {
                for channel in &mut pixel[..3] {
                    let scaled = (*channel as u16).saturating_mul(255).saturating_add(a / 2);
                    *channel = scaled.checked_div(a).unwrap_or(255).min(255) as u8;
                }
            }
        }
    }
    Some(RasterizedSvg {
        width,
        height,
        rgba: rgba.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_fills_target() {
        let markup = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="32" height="32"><rect x="4" y="4" width="24" height="24" fill="#ff0000"/></svg>"##;
        let raster = rasterize_stretch(markup, 32, 32, None, 2048).expect("rect");
        let center = &raster.rgba[(16 * 32 + 16) * 4..][..4];
        assert!(center[0] > 200 && center[3] > 200, "{center:?}");
    }

    #[test]
    fn document_is_straight_alpha() {
        let raster = rasterize_document(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="3" height="2"><rect width="3" height="2" fill="#60a5fa"/></svg>"##,
        )
        .expect("doc");
        assert_eq!((raster.width, raster.height), (3, 2));
        assert_eq!(&raster.rgba[..4], &[0x60, 0xa5, 0xfa, 0xff]);
    }

    #[test]
    fn white_mask_tints_rgb() {
        let rgba = rasterize_white_mask(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="currentColor"/></svg>"##,
            16,
            256,
        )
        .expect("mask");
        assert!(
            rgba.chunks(4)
                .any(|p| p[3] > 16 && p[..3] == [255, 255, 255])
        );
    }
}
