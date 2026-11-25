//! CPU SVG raster (resvg → pixmap). Not a painter and not an SVG DOM.

use std::sync::Arc;

/// Premultiplied RGBA8 pixmap, or straight alpha from [`rasterize_document`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterizedSvg {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

/// One font file's bytes, shared with whoever owns them.
pub type SvgFontData = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// The faces SVG `<text>` may be drawn with.
///
/// This crate finds no fonts itself: the caller's font system says which
/// files to read and what the generic families are called, so SVG text
/// follows the same sources and fallback as the rest of the application.
/// The files are shared, never copied. A family the document names that is
/// not among `faces` is drawn in the sans-serif one.
#[derive(Clone, Default)]
pub struct SvgFonts {
    /// Font files. Every face of a collection is available.
    pub faces: Vec<SvgFontData>,
    /// What `sans-serif` is called, and the default for unnamed text.
    pub sans_serif: Option<String>,
    pub serif: Option<String>,
    pub monospace: Option<String>,
}

/// Generic `<svg>`: stretch independently to `width`×`height`.
pub fn rasterize_stretch(
    markup: &str,
    width: u32,
    height: u32,
    fonts: Option<&SvgFonts>,
    max_edge: u32,
) -> Option<RasterizedSvg> {
    let width = width.clamp(1, max_edge.max(1));
    let height = height.clamp(1, max_edge.max(1));
    render(
        Source::Str(markup),
        Some((width as f32, height as f32)),
        fonts,
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
    rasterize_document_capped_with_fonts(bytes, max_edge, None)
}

/// URL SVG decode with the faces SVG text is drawn with. Without them text
/// is not drawn; glyphs embedded as paths need no font.
pub fn rasterize_document_capped_with_fonts(
    bytes: &[u8],
    max_edge: u32,
    fonts: Option<&SvgFonts>,
) -> Option<RasterizedSvg> {
    render(
        Source::Bytes(bytes),
        None,
        fonts,
        None,
        Fit::DocumentCapped { max_edge },
        None,
        true,
    )
}

/// OpenType `SVG ` table glyph rasterization: renders `#glyph<id>` (or the
/// whole document when `allow_root_fallback` is true) scaled by `size_px / upem`
/// into straight RGBA8 with `(left, top, width, height)` in y-up baseline-relative
/// raster pixels.
pub fn rasterize_opentype_svg_glyph(
    bytes: &[u8],
    glyph_id: u16,
    allow_root_fallback: bool,
    size_px: f32,
    upem: u16,
    max_edge: u32,
) -> Option<(i32, i32, RasterizedSvg)> {
    if !size_px.is_finite() || size_px <= 0.0 || upem == 0 {
        return None;
    }
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    options.default_size = resvg::usvg::Size::from_wh(f32::from(upem), f32::from(upem))?;
    let tree = resvg::usvg::Tree::from_data(bytes, &options).ok()?;
    let target_id = format!("glyph{glyph_id}");
    let node = tree.node_by_id(&target_id);
    if node.is_none() && !allow_root_fallback {
        return None;
    }
    let bbox = match node {
        Some(node) => node.abs_layer_bounding_box()?.to_rect(),
        None => tree.root().abs_bounding_box(),
    };
    let scale = size_px / f32::from(upem);
    let min_x = (bbox.left() * scale).floor();
    let min_y = (bbox.top() * scale).floor();
    let max_x = (bbox.right() * scale).ceil();
    let max_y = (bbox.bottom() * scale).ceil();
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }
    let width = (max_x - min_x).max(0.0) as u32;
    let height = (max_y - min_y).max(0.0) as u32;
    if width == 0 || height == 0 {
        return Some((
            0,
            0,
            RasterizedSvg {
                width: 0,
                height: 0,
                rgba: Arc::from([]),
            },
        ));
    }
    if width > max_edge || height > max_edge {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    match node {
        Some(node) => {
            // `resvg::render_node` pre-translates by `-bbox.xy()` in root-canvas space
            // and renders the node without ancestor transforms (`viewBox`, parent `<g>`),
            // so feed ancestor transforms back in and cancel its internal `-bbox.xy()`.
            let parent_transform = match node {
                resvg::usvg::Node::Group(group) => group
                    .abs_transform()
                    .pre_concat(group.transform().invert().unwrap_or_default()),
                resvg::usvg::Node::Path(path) => path.abs_transform(),
                resvg::usvg::Node::Image(image) => image.abs_transform(),
                resvg::usvg::Node::Text(text) => text
                    .flattened()
                    .abs_transform()
                    .pre_concat(text.flattened().transform().invert().unwrap_or_default()),
            };
            let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, -min_x, -min_y)
                .pre_concat(parent_transform)
                .pre_translate(bbox.x(), bbox.y());
            resvg::render_node(node, transform, &mut pixmap.as_mut())?;
        }
        None => {
            let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, -min_x, -min_y);
            resvg::render(&tree, transform, &mut pixmap.as_mut());
        }
    }
    let mut rgba = pixmap.data().to_vec();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let a = pixel[3] as u16;
        if a > 0 && a < 255 {
            for channel in &mut pixel[..3] {
                let scaled = (*channel as u16).saturating_mul(255).saturating_add(a / 2);
                *channel = scaled.checked_div(a).unwrap_or(255).min(255) as u8;
            }
        }
    }
    Some((
        min_x as i32,
        -(min_y as i32),
        RasterizedSvg {
            width,
            height,
            rgba: rgba.into(),
        },
    ))
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
    fonts: Option<&SvgFonts>,
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
    if let Some(fonts) = fonts.filter(|fonts| !fonts.faces.is_empty()) {
        let mut fontdb = resvg::usvg::fontdb::Database::new();
        for face in &fonts.faces {
            fontdb.load_font_source(resvg::usvg::fontdb::Source::Binary(Arc::clone(face)));
        }
        if let Some(family) = &fonts.sans_serif {
            fontdb.set_sans_serif_family(family);
            options.font_family = family.clone();
        }
        if let Some(family) = &fonts.serif {
            fontdb.set_serif_family(family);
        }
        if let Some(family) = &fonts.monospace {
            fontdb.set_monospace_family(family);
        }
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
