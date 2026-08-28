//! Resolve `background-image: url(...)` for GPU texture upload.

use std::path::PathBuf;
use std::sync::OnceLock;

use nana_ui_core::{
    MAX_LOCAL_URL_BYTES, file_url_to_path, href_is_protocol_relative_or_unc, path_looks_network,
    percent_decode_bytes, read_bytes_within_jail, resolve_filesystem_href,
};

static BACKGROUND_IMAGE_URL_BASE: OnceLock<PathBuf> = OnceLock::new();

// Per-test-thread override. Cargo libtest runs each test on one thread, so
// parallel tests cannot see each other's base. Hosts still first-set
// BACKGROUND_IMAGE_URL_BASE.
#[cfg(test)]
thread_local! {
    static TEST_URL_BASE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Document or workspace base for relative `url(...)` paths.
///
/// First call wins for hosts. When unset, relative URLs resolve against the
/// process current working directory.
pub fn set_background_image_url_base(base: PathBuf) {
    #[cfg(test)]
    {
        TEST_URL_BASE.with(|slot| *slot.borrow_mut() = Some(base.clone()));
    }
    let _ = BACKGROUND_IMAGE_URL_BASE.set(base);
}

fn fallback_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn url_base() -> PathBuf {
    #[cfg(test)]
    {
        return TEST_URL_BASE.with(|slot| slot.borrow().clone().unwrap_or_else(fallback_cwd));
    }
    #[cfg(not(test))]
    {
        BACKGROUND_IMAGE_URL_BASE
            .get()
            .cloned()
            .unwrap_or_else(fallback_cwd)
    }
}

/// Resolve a parsed CSS URL to a fetch/load key (absolute URL or filesystem path).
///
/// `http(s)` / `data:` stay fetchable. Non-local `file:` hosts, protocol-relative
/// `//`, and UNC are refused (same helper as stylesheet / `@font-face` jail).
pub fn resolve_background_image_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() || href_is_protocol_relative_or_unc(trimmed) {
        return None;
    }
    if trimmed.starts_with("data:")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        return Some(trimmed.to_string());
    }
    if trimmed.to_ascii_lowercase().starts_with("file:") {
        let path = file_url_to_path(trimmed)?;
        if path_looks_network(&path) {
            return None;
        }
        return Some(path.to_string_lossy().into_owned());
    }
    join_relative(trimmed)
}

fn join_relative(rel: &str) -> Option<String> {
    let path = resolve_filesystem_href(rel, None, &url_base())?;
    Some(path.to_string_lossy().into_owned())
}

/// Decode a resolved CSS URL into straight-alpha RGBA8. Cached by the quad
/// painter; not invoked every frame after the first hit.
///
/// SVG bytes (inline `data:image/svg+xml`, `url(.svg)`, or sniffed markup) share
/// this path with raster images so the quad URL cache keys by URL/id.
pub(super) fn decode_url_rgba(url: &str) -> Option<(u32, u32, Vec<u8>)> {
    let resolved = resolve_background_image_url(url)?;
    if resolved.starts_with("data:") {
        return decode_data_url_rgba(&resolved);
    }
    if resolved.starts_with("http://") || resolved.starts_with("https://") {
        return decode_http_rgba(&resolved);
    }
    let jail = url_base();
    let bytes = read_bytes_within_jail(url, &jail, MAX_LOCAL_URL_BYTES)?;
    decode_image_bytes_with_hint(&bytes, looks_like_svg_url(&resolved))
}

fn decode_data_url_rgba(url: &str) -> Option<(u32, u32, Vec<u8>)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let meta_l = meta.to_ascii_lowercase();
    let is_base64 = meta_l.split(';').any(|part| part.trim() == "base64");
    let bytes = if is_base64 {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .ok()?
    } else {
        percent_decode_bytes(payload)?
    };
    decode_image_bytes_with_hint(&bytes, meta_l.contains("svg"))
}

fn decode_http_rgba(url: &str) -> Option<(u32, u32, Vec<u8>)> {
    let mut response = ureq::get(url).call().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_LOCAL_URL_BYTES)
        .read_to_vec()
        .ok()?;
    if bytes.len() as u64 > MAX_LOCAL_URL_BYTES {
        return None;
    }
    decode_image_bytes_with_hint(&bytes, looks_like_svg_url(url))
}

fn decode_image_bytes(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    decode_image_bytes_with_hint(bytes, false)
}

fn decode_image_bytes_with_hint(bytes: &[u8], prefer_svg: bool) -> Option<(u32, u32, Vec<u8>)> {
    if prefer_svg || looks_like_svg(bytes) {
        match decode_svg_rgba(bytes) {
            Some(decoded) => return Some(decoded),
            None if prefer_svg => return None,
            None => {}
        }
    }
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some((width, height, rgba.into_raw()))
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let text = text.trim_start();
    text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg"))
}

fn looks_like_svg_url(url: &str) -> bool {
    let path = url
        .split_once('#')
        .map(|(path, _)| path)
        .unwrap_or(url)
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(url);
    PathExt(path).has_svg_extension()
}

struct PathExt<'a>(&'a str);

impl PathExt<'_> {
    fn has_svg_extension(self) -> bool {
        let bytes = self.0.as_bytes();
        let Some(dot) = bytes.iter().rposition(|b| *b == b'.') else {
            return false;
        };
        bytes[dot + 1..].eq_ignore_ascii_case(b"svg")
    }
}

const MAX_SVG_EDGE: u32 = 2048;

fn decode_svg_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let raster = nana_svg_raster::rasterize_document_capped(bytes, MAX_SVG_EDGE)?;
    Some((raster.width, raster.height, raster.rgba.to_vec()))
}

#[cfg(test)]
pub(super) fn reset_test_url_base() {
    TEST_URL_BASE.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn relative_url_joins_cwd_when_no_base() {
        reset_test_url_base();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let resolved = resolve_background_image_url("assets/icon.png").unwrap();
        assert_eq!(
            Path::new(&resolved),
            cwd.join("assets/icon.png").as_path(),
            "relative url must join cwd, not return the input unchanged"
        );
    }

    #[test]
    fn set_background_image_url_base_prefixes_relative() {
        let base =
            std::env::temp_dir().join(format!("nana-url-base-{}-{}", std::process::id(), "prefix"));
        set_background_image_url_base(base.clone());
        let resolved = resolve_background_image_url("assets/icon.png").unwrap();
        assert_eq!(
            Path::new(&resolved),
            base.join("assets/icon.png").as_path(),
            "set_background_image_url_base must prefix the relative path"
        );
        reset_test_url_base();
    }

    #[test]
    fn file_url_becomes_path() {
        let resolved = resolve_background_image_url("file:///tmp/test.png").unwrap();
        assert_file_url_path(&resolved);
    }

    #[test]
    fn file_localhost_url_becomes_path() {
        let resolved = resolve_background_image_url("file://localhost/tmp/test.png").unwrap();
        assert_file_url_path(&resolved);
    }

    #[test]
    fn file_remote_host_is_rejected() {
        assert!(resolve_background_image_url("file://evil.example/secret.png").is_none());
        assert!(decode_url_rgba("file://evil.example/secret.png").is_none());
        assert!(resolve_background_image_url("//cdn.example.com/x.png").is_none());
        assert!(resolve_background_image_url(r"\\cdn.example.com\share\x.png").is_none());
        assert!(resolve_background_image_url("%2f%2fcdn.example.com/x.png").is_none());
        assert!(decode_url_rgba("//cdn.example.com/x.png").is_none());
    }

    #[test]
    fn local_file_outside_jail_is_not_read() {
        let pid = std::process::id();
        let jail = std::env::temp_dir().join(format!("nanaui-img-jail-{pid}"));
        let outside = std::env::temp_dir().join(format!("nanaui-img-out-{pid}"));
        let _ = std::fs::remove_dir_all(&jail);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&jail).expect("jail");
        std::fs::create_dir_all(&outside).expect("outside");
        let secret = outside.join("secret.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]))
            .save(&secret)
            .expect("secret png");
        set_background_image_url_base(jail.clone());
        assert!(
            decode_url_rgba(&secret.to_string_lossy()).is_none(),
            "absolute path outside jail must not decode"
        );
        assert!(
            decode_url_rgba("../secret.png").is_none(),
            "relative escape must not decode"
        );
        let ok = jail.join("ok.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .save(&ok)
            .expect("ok png");
        let (w, h, rgba) = decode_url_rgba("ok.png").expect("jailed relative");
        assert_eq!((w, h), (2, 2));
        assert_eq!(&rgba[..4], &[1, 2, 3, 255]);
        let (aw, ah, argb) =
            decode_url_rgba(&ok.to_string_lossy()).expect("absolute path inside jail");
        assert_eq!((aw, ah), (2, 2));
        assert_eq!(&argb[..4], &[1, 2, 3, 255]);
        let file_url = url::Url::from_file_path(&ok)
            .map(|parsed| parsed.to_string())
            .unwrap_or_else(|_| format!("file:///{}", ok.display().to_string().replace('\\', "/")));
        let (fw, fh, frgba) = decode_url_rgba(&file_url).expect("file url inside jail");
        assert_eq!((fw, fh), (2, 2));
        assert_eq!(&frgba[..4], &[1, 2, 3, 255]);
        reset_test_url_base();
        let _ = std::fs::remove_dir_all(&jail);
        let _ = std::fs::remove_dir_all(&outside);
    }

    fn assert_file_url_path(resolved: &str) {
        let normalized = resolved.replace('\\', "/");
        assert!(
            normalized.ends_with("tmp/test.png"),
            "file url must become a filesystem path ending in tmp/test.png, got {resolved}"
        );
    }

    #[test]
    fn svg_bytes_rasterize_straight_rgba() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="#00ff00"/></svg>"##;
        let (width, height, rgba) = decode_image_bytes(svg).expect("svg decode");
        assert_eq!((width, height), (4, 2));
        assert_eq!(&rgba[..4], &[0x00, 0xff, 0x00, 0xff]);
    }

    #[test]
    fn svg_image_href_does_not_read_local_file() {
        let dir = std::env::temp_dir().join(format!("nanaui-svg-href-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let secret = dir.join("secret.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]))
            .save(&secret)
            .expect("secret");
        let href = secret.to_string_lossy().replace('\\', "/");
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"><image href=\"{href}\" width=\"4\" height=\"4\"/></svg>"
        );
        let (_, _, rgba) = decode_image_bytes(svg.as_bytes()).expect("svg");
        let red = rgba
            .chunks(4)
            .filter(|pixel| pixel[0] > 200 && pixel[3] > 16)
            .count();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(red, 0, "usvg must not fs::read image href");
    }

    #[test]
    fn svg_data_url_decodes() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#ff0000\"/></svg>";
        let url = format!("data:image/svg+xml,{svg}");
        let (width, height, rgba) = decode_url_rgba(&url).expect("svg data url");
        assert_eq!((width, height), (2, 2));
        assert_eq!(&rgba[..4], &[0xff, 0x00, 0x00, 0xff]);
    }

    #[test]
    fn inline_svg_path_rasterizes_via_resvg() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 0 H10 V10 H0 Z" fill="#ff00ff"/></svg>"##;
        let (width, height, rgba) = decode_image_bytes(svg).expect("path svg");
        assert!(width >= 1 && height >= 1, "resvg must produce a raster");
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        let inked = rgba.chunks(4).filter(|pixel| pixel[3] > 16).count();
        assert!(
            inked > 0,
            "inline svg path must ink the raster, got {inked} opaque pixels"
        );
        assert_eq!(&rgba[..4], &[0xff, 0x00, 0xff, 0xff]);
    }

    #[test]
    fn svg_raster_size_is_capped_without_aspect_distortion() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8000" height="2000"><rect width="8000" height="2000" fill="#0000ff"/></svg>"##;
        let (width, height, rgba) = decode_image_bytes(svg).expect("capped svg");
        assert_eq!((width, height), (2048, 512));
        assert_eq!(rgba.len(), (2048 * 512 * 4) as usize);
        assert_eq!(&rgba[..4], &[0x00, 0x00, 0xff, 0xff]);
    }

    #[test]
    fn svg_file_extension_hint_uses_resvg() {
        assert!(looks_like_svg_url("icons/mark.svg"));
        assert!(looks_like_svg_url("https://cdn.example/a.SVG?v=1"));
        assert!(!looks_like_svg_url("icons/mark.png"));
    }

    #[test]
    fn svg_base64_data_url_decodes() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><rect width=\"1\" height=\"1\" fill=\"#010203\"/></svg>";
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
        let url = format!("data:image/svg+xml;base64,{encoded}");
        let (width, height, rgba) = decode_url_rgba(&url).expect("base64 svg");
        assert_eq!((width, height), (1, 1));
        assert_eq!(&rgba[..4], &[0x01, 0x02, 0x03, 0xff]);
    }
}
