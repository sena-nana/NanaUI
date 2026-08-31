//! Rasterize a Vue `<svg>` subtree into HostTexture pixels.
//!
//! This is not an SVG DOM / D3 runtime. Lucide icons stay on the Icon atlas
//! path; this module only handles generic charts and markup.

use nana_svg_raster::SvgFont;

const FONT_BYTES: &[u8] = include_bytes!("../../nana-ui/assets/fonts/NotoSansSC-Regular.ttf");
const MAX_RASTER_EDGE: u32 = 2048;

pub use nana_svg_raster::RasterizedSvg;

/// One serialized SVG element (tag + attrs + children).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgElement {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<SvgNode>,
}

/// Serialized SVG tree node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvgNode {
    Element(SvgElement),
    Text(String),
}

/// Slots held on the GPU that are not in the current live upload set.
#[cfg_attr(not(test), allow(dead_code))]
pub fn released_svg_gpu_slots<'a>(
    held: impl IntoIterator<Item = &'a str>,
    live: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let live: std::collections::HashSet<&str> = live.into_iter().collect();
    let mut released: Vec<String> = held
        .into_iter()
        .filter(|slot| !live.contains(*slot))
        .map(str::to_string)
        .collect();
    released.sort();
    released
}

/// One dirty SVG pixmap to upload onto the host Device/Queue.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(any(test, feature = "hosted")), allow(dead_code))]
pub struct SvgHostUpload {
    pub slot: String,
    pub node: u64,
    pub raster: RasterizedSvg,
    pub version: u64,
}

/// Whether `markup` contains a `textPath` element.
#[cfg_attr(not(test), allow(dead_code))]
pub fn markup_has_text_path(markup: &str) -> bool {
    markup
        .to_ascii_lowercase()
        .split('<')
        .any(|chunk| chunk.starts_with("textpath") || chunk.starts_with("/textpath"))
}

/// Serialize an SVG element tree to XML.
pub fn serialize_svg(root: &SvgElement) -> String {
    let mut out = String::new();
    write_element(&mut out, root, true);
    out
}

/// Rasterize serialized SVG into a pixmap sized to `width`×`height`.
pub fn rasterize_svg(markup: &str, width: u32, height: u32) -> Option<RasterizedSvg> {
    nana_svg_raster::rasterize_stretch(
        markup,
        width,
        height,
        Some(SvgFont {
            bytes: FONT_BYTES,
            family: "Noto Sans SC",
        }),
        MAX_RASTER_EDGE,
    )
}

/// True when resvg produced visible coverage for `textPath` in `markup`.
///
/// Returns `false` when usvg drops `textPath` (honest skip, not a fake draw).
#[cfg_attr(not(test), allow(dead_code))]
pub fn text_path_renders(markup: &str, width: u32, height: u32) -> bool {
    if !markup_has_text_path(markup) {
        return false;
    }
    let Some(raster) = rasterize_svg(markup, width, height) else {
        return false;
    };
    raster.rgba.chunks(4).any(|pixel| pixel[3] > 16)
}

fn write_element(out: &mut String, element: &SvgElement, root: bool) {
    let tag = element.tag.trim();
    if tag.is_empty() {
        return;
    }
    out.push('<');
    out.push_str(tag);
    let mut wrote_xmlns = false;
    for (name, value) in &element.attrs {
        if name.eq_ignore_ascii_case("xmlns") {
            wrote_xmlns = true;
        }
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        push_escaped(out, value);
        out.push('"');
    }
    if root && !wrote_xmlns {
        out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
    }
    if element.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for child in &element.children {
        match child {
            SvgNode::Element(child) => write_element(out, child, false),
            SvgNode::Text(text) => push_escaped(out, text),
        }
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

fn push_escaped(out: &mut String, raw: &str) {
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_svg() -> SvgElement {
        SvgElement {
            tag: "svg".into(),
            attrs: vec![
                ("viewBox".into(), "0 0 32 32".into()),
                ("width".into(), "32".into()),
                ("height".into(), "32".into()),
            ],
            children: vec![SvgNode::Element(SvgElement {
                tag: "rect".into(),
                attrs: vec![
                    ("x".into(), "4".into()),
                    ("y".into(), "4".into()),
                    ("width".into(), "24".into()),
                    ("height".into(), "24".into()),
                    ("fill".into(), "#ff0000".into()),
                ],
                children: Vec::new(),
            })],
        }
    }

    #[test]
    fn serializes_viewbox_and_path_children() {
        let markup = serialize_svg(&rect_svg());
        assert!(markup.contains("<svg"));
        assert!(markup.contains("viewBox=\"0 0 32 32\""));
        assert!(markup.contains("xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(markup.contains("<rect"));
        assert!(markup.contains("fill=\"#ff0000\""));
    }

    #[test]
    fn rasterizes_filled_rect_with_coverage() {
        let raster = rasterize_svg(&serialize_svg(&rect_svg()), 32, 32).expect("rect svg");
        assert_eq!(raster.width, 32);
        assert_eq!(raster.height, 32);
        let ink = raster.rgba.chunks(4).filter(|pixel| pixel[3] > 16).count();
        assert!(ink > 40, "filled rect must ink the pixmap, got {ink}");
        let center = &raster.rgba[(16 * 32 + 16) * 4..][..4];
        assert!(
            center[0] > 200 && center[3] > 200,
            "rect center must be red, got {center:?}"
        );
    }

    #[test]
    fn text_path_status_is_honest() {
        let markup = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 80" width="200" height="80">
  <path id="curve" d="M10 60 Q100 10 190 60" fill="none"/>
  <text font-size="24" fill="#000000">
    <textPath href="#curve">HELLOPATH</textPath>
  </text>
</svg>"##;
        assert!(markup_has_text_path(markup));
        let renders = text_path_renders(markup, 200, 80);
        // resvg currently draws <textPath>. If that ever stops, drop the
        // `true` check and skip rather than faking a path draw.
        assert!(
            renders,
            "resvg no longer draws textPath; report skip instead of faking"
        );
        let raster = rasterize_svg(markup, 200, 80).expect("textPath pixmap");
        let ink = raster.rgba.chunks(4).filter(|pixel| pixel[3] > 16).count();
        assert!(ink > 10, "textPath claimed to render but pixmap is empty");
    }

    #[test]
    fn released_svg_gpu_slots_drops_ids_not_in_live_set() {
        let held = ["svg:1", "svg:2", "svg:3"];
        let live = ["svg:2"];
        assert_eq!(
            released_svg_gpu_slots(held, live),
            vec!["svg:1".to_string(), "svg:3".to_string()]
        );
        assert!(released_svg_gpu_slots(["svg:4"], ["svg:4"]).is_empty());
        assert_eq!(
            released_svg_gpu_slots(["svg:5"], std::iter::empty()),
            vec!["svg:5".to_string()]
        );
    }
}
