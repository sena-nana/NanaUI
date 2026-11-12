//! Load `@font-face` `url(...)` srcs into the process FontSystem.
//!
//! A host calls it with the faces it wants loaded; nothing calls it on
//! parse. Reuses [`resolve_background_image_url`] / the given URL base.
//! Local, packaged (`nana://res/`) and `data:` only: remote srcs never reach
//! the network.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use nana_text::font::{FontBlob, font_blob};

use crate::nana_text::{HostFontStyle, register_host_font_face_blob};
use crate::scene_paint::{resolve_background_image_url, resolved_resource_is_allowed};

const FONT_FACE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// One parsed `@font-face` ready for host ingest (no CSSOM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFontFaceSpec {
    pub family: Option<String>,
    /// `url(...)` values only; `local(...)` is ignored.
    pub urls: Vec<String>,
    /// `font-weight` start; with [`Self::weight_end`] the inclusive range
    /// (`font-weight: 200 700`), alone a single weight.
    pub weight: Option<u16>,
    pub weight_end: Option<u16>,
    pub style: Option<HostFontStyle>,
}

/// One registered face: its src and the descriptors it was registered under.
///
/// The src alone is not the face. Two `@font-face` rules may share one file
/// under different families, weights or styles, and each is its own
/// registration; keying on the src made the second rule a silent no-op.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LoadedFaceKey {
    src: String,
    family: String,
    weight: Option<u16>,
    weight_end: Option<u16>,
    style: Option<HostFontStyle>,
}

static LOADED_FONT_FACES: OnceLock<Mutex<HashSet<LoadedFaceKey>>> = OnceLock::new();
type WeakFontBlob = Weak<dyn AsRef<[u8]> + Send + Sync>;
static LOADED_FONT_BLOBS: OnceLock<Mutex<HashMap<String, WeakFontBlob>>> = OnceLock::new();

fn loaded_faces() -> &'static Mutex<HashSet<LoadedFaceKey>> {
    LOADED_FONT_FACES.get_or_init(|| Mutex::new(HashSet::new()))
}

fn loaded_blobs() -> &'static Mutex<HashMap<String, WeakFontBlob>> {
    LOADED_FONT_BLOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn src_key(resolved: &str) -> String {
    if resolved.starts_with("data:") && resolved.len() > 128 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        resolved.hash(&mut hasher);
        format!("data-hash:{:016x}", hasher.finish())
    } else {
        resolved.to_string()
    }
}

fn already_loaded(key: &LoadedFaceKey) -> bool {
    loaded_faces()
        .lock()
        .map(|guard| guard.contains(key))
        .unwrap_or(false)
}

fn mark_loaded(key: LoadedFaceKey) {
    if let Ok(mut guard) = loaded_faces().lock() {
        guard.insert(key);
    }
}

/// Load each face's first usable `url(...)` and register it with CSS aliases.
///
/// The face is registered over its whole `weight`..`weight_end` range.
/// A face already loaded from the same src under the same descriptors is
/// skipped. Failed srcs drop that face (no system-font stand-in). Remote
/// `http(s)` srcs are refused — `@font-face` has no network transport.
/// `data:` and jailed filesystem srcs read up to 8 MiB; `data:` is decoded
/// here, since the CSS engine's file loader
/// (`nana_ui_css::css_at_rule::load_font_face_bytes`) reads only files.
pub fn ingest_host_font_faces(specs: &[HostFontFaceSpec]) -> usize {
    let mut loaded = 0usize;
    for spec in specs {
        if ingest_one(spec) {
            loaded += 1;
        }
    }
    loaded
}

fn ingest_one(spec: &HostFontFaceSpec) -> bool {
    let Some(family) = spec.family.as_deref().filter(|family| !family.is_empty()) else {
        return false;
    };
    for url in &spec.urls {
        let Some(resolved) = resolve_background_image_url(url) else {
            continue;
        };
        if !resolved_resource_is_allowed(url, &resolved) {
            continue;
        }
        let src = src_key(&resolved);
        let key = LoadedFaceKey {
            src: src.clone(),
            family: family.to_owned(),
            weight: spec.weight,
            weight_end: spec.weight_end,
            style: spec.style,
        };
        if already_loaded(&key) {
            return true;
        }
        let Some(blob) = load_or_reuse_font_blob(&src, &resolved) else {
            continue;
        };
        if register_host_font_face_blob(family, blob, spec.weight, spec.weight_end, spec.style) == 0
        {
            continue;
        }
        mark_loaded(key);
        return true;
    }
    false
}

fn load_or_reuse_font_blob(src_key: &str, resolved: &str) -> Option<FontBlob> {
    if let Ok(guard) = loaded_blobs().lock()
        && let Some(existing) = guard.get(src_key).and_then(Weak::upgrade)
    {
        return Some(existing);
    }
    let bytes = load_font_src_bytes(resolved)?;
    let blob = font_blob(bytes);
    if let Ok(mut guard) = loaded_blobs().lock() {
        if let Some(existing) = guard.get(src_key).and_then(Weak::upgrade) {
            return Some(existing);
        }
        if guard.len() >= 64 {
            guard.retain(|_, weak| weak.strong_count() > 0);
        }
        guard.insert(src_key.to_owned(), Arc::downgrade(&blob));
    }
    Some(blob)
}

fn load_font_src_bytes(resolved: &str) -> Option<Vec<u8>> {
    if resolved.starts_with("data:") {
        let bytes = decode_data_url_bytes(resolved)?;
        if bytes.len() > FONT_FACE_MAX_BYTES {
            return None;
        }
        return Some(bytes);
    }
    if nana_ui_core::is_packaged_url(resolved) {
        return nana_ui_core::read_packaged(resolved, None, FONT_FACE_MAX_BYTES as u64);
    }
    let meta = std::fs::metadata(resolved).ok()?;
    if meta.len() > FONT_FACE_MAX_BYTES as u64 {
        return None;
    }
    std::fs::read(resolved).ok()
}

fn decode_data_url_bytes(url: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    let meta = rest[..comma].to_ascii_lowercase();
    if !meta.contains("base64") {
        return None;
    }
    let payload = rest[comma + 1..].trim();
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_font_face_src_is_not_fetched() {
        let listener =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let url = format!("http://{}/a.woff2", listener.local_addr().expect("addr"));
        let spec = HostFontFaceSpec {
            family: Some("Remote".into()),
            urls: vec![url],
            weight: None,
            weight_end: None,
            style: None,
        };
        assert_eq!(
            ingest_host_font_faces(&[spec]),
            0,
            "@font-face has no network transport"
        );
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "a remote src must not open a socket"
        );
    }

    /// `font-weight: 200 700` is one registration over the whole range: a
    /// weight inside it, not only the start, lands on a face that covers it.
    #[test]
    #[cfg(feature = "bundled-fonts")]
    fn an_ingested_weight_range_covers_weights_inside_it() {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(crate::UI_FONT_REGULAR);
        let spec = HostFontFaceSpec {
            family: Some("NanaIngestRangeFace".into()),
            urls: vec![format!("data:font/ttf;base64,{encoded}")],
            weight: Some(200),
            weight_end: Some(700),
            style: None,
        };
        assert_eq!(ingest_host_font_faces(&[spec]), 1);

        let style = nana_text::TextStyle {
            font_family: Some(std::sync::Arc::from("NanaIngestRangeFace")),
            font_weight: 450,
            ..nana_text::TextStyle::default()
        };
        let engine = crate::text_engine::nana_text_engine();
        let mut engine = crate::text_engine::lock_engine(&engine);
        let layout = nana_text::TextEngine::layout(
            &mut *engine,
            nana_text::TextKind::Label,
            &nana_text::TextSource::new("H"),
            &style,
            &nana_text::TextConstraints::default(),
            &mut nana_text::TextWorkCounters::default(),
        );
        let faces: Vec<_> = layout
            .runs
            .iter()
            .filter_map(|run| engine.fonts().describe(run.font))
            .filter(|face| face.families.iter().any(|f| &**f == "NanaIngestRangeFace"))
            .map(|face| face.weight)
            .collect();
        assert_eq!(faces, [(200.0, 700.0)], "450 shapes with the 200..700 face");
    }

    /// `font-style: italic` reaches the registration: the ingested face is
    /// described as italic, not the default style.
    #[test]
    #[cfg(feature = "bundled-fonts")]
    fn an_ingested_italic_face_registers_as_italic() {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(crate::UI_FONT_REGULAR);
        let spec = HostFontFaceSpec {
            family: Some("NanaIngestItalicFace".into()),
            urls: vec![format!("data:font/ttf;base64,{encoded}")],
            weight: None,
            weight_end: None,
            style: Some(HostFontStyle::Italic),
        };
        assert_eq!(ingest_host_font_faces(&[spec]), 1);

        let style = nana_text::TextStyle {
            font_family: Some(std::sync::Arc::from("NanaIngestItalicFace")),
            italic: true,
            ..nana_text::TextStyle::default()
        };
        let engine = crate::text_engine::nana_text_engine();
        let mut engine = crate::text_engine::lock_engine(&engine);
        let layout = nana_text::TextEngine::layout(
            &mut *engine,
            nana_text::TextKind::Label,
            &nana_text::TextSource::new("H"),
            &style,
            &nana_text::TextConstraints::default(),
            &mut nana_text::TextWorkCounters::default(),
        );
        let styles: Vec<_> = layout
            .runs
            .iter()
            .filter_map(|run| engine.fonts().describe(run.font))
            .filter(|face| face.families.iter().any(|f| &**f == "NanaIngestItalicFace"))
            .map(|face| face.style)
            .collect();
        assert_eq!(styles, [nana_text::font::FontStyle::Italic]);
    }

    /// One src under a second family is a second face. Keyed on the src
    /// alone, the second rule was skipped as already loaded, so its family
    /// never existed; the tests above share one `data:` src and raced on it.
    #[test]
    #[cfg(feature = "bundled-fonts")]
    fn one_src_under_two_families_registers_both() {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(crate::UI_FONT_REGULAR);
        let spec = |family: &str| HostFontFaceSpec {
            family: Some(family.into()),
            urls: vec![format!("data:font/ttf;base64,{encoded}")],
            weight: None,
            weight_end: None,
            style: None,
        };
        assert_eq!(
            ingest_host_font_faces(&[spec("NanaIngestSharedSrcA"), spec("NanaIngestSharedSrcB")]),
            2
        );

        let engine = crate::text_engine::nana_text_engine();
        let mut engine = crate::text_engine::lock_engine(&engine);
        let mut backing_ptrs = Vec::new();
        for family in ["NanaIngestSharedSrcA", "NanaIngestSharedSrcB"] {
            let style = nana_text::TextStyle {
                font_family: Some(std::sync::Arc::from(family)),
                ..nana_text::TextStyle::default()
            };
            let layout = nana_text::TextEngine::layout(
                &mut *engine,
                nana_text::TextKind::Label,
                &nana_text::TextSource::new("H"),
                &style,
                &nana_text::TextConstraints::default(),
                &mut nana_text::TextWorkCounters::default(),
            );
            let run = layout
                .runs
                .iter()
                .find(|run| {
                    engine
                        .fonts()
                        .describe(run.font)
                        .is_some_and(|face| face.families.iter().any(|f| &**f == family))
                })
                .unwrap_or_else(|| panic!("{family} shapes with its own registration"));
            let data = engine.fonts().face_data(run.font).expect("face data");
            backing_ptrs.push((data.bytes().as_ptr(), data.bytes().len()));
        }
        assert_eq!(
            backing_ptrs[0], backing_ptrs[1],
            "multiple @font-face rules over one src must share a single FontBlob allocation"
        );
    }
}
