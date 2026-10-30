//! Load `@font-face` `url(...)` srcs into the process FontSystem.
//!
//! A host calls it with the faces it wants loaded; nothing calls it on
//! parse. Reuses [`resolve_background_image_url`] / the given URL base.
//! Local, packaged (`nana://res/`) and `data:` only: remote srcs never reach
//! the network.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use crate::nana_text::{HostFontStyle, register_host_font_face};
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

static LOADED_FONT_URLS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn loaded_urls() -> &'static Mutex<HashSet<String>> {
    LOADED_FONT_URLS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn cache_key(resolved: &str) -> String {
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

fn already_loaded(key: &str) -> bool {
    loaded_urls()
        .lock()
        .map(|guard| guard.contains(key))
        .unwrap_or(false)
}

fn mark_loaded(key: String) {
    if let Ok(mut guard) = loaded_urls().lock() {
        guard.insert(key);
    }
}

/// Load each face's first usable `url(...)` and register it with CSS aliases.
///
/// The face is registered over its whole `weight`..`weight_end` range.
/// Already-loaded keys are skipped. Failed srcs drop that face (no system-font
/// stand-in). Remote `http(s)` srcs are refused — `@font-face` has no network
/// transport. `data:` and jailed filesystem
/// srcs read up to 8 MiB; `data:` is decoded here, since the CSS engine's
/// file loader (`nana_ui_css::css_at_rule::load_font_face_bytes`) reads only files.
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
    for url in &spec.urls {
        let Some(resolved) = resolve_background_image_url(url) else {
            continue;
        };
        if !resolved_resource_is_allowed(url, &resolved) {
            continue;
        }
        let key = cache_key(&resolved);
        if already_loaded(&key) {
            return true;
        }
        let Some(bytes) = load_font_src_bytes(&resolved) else {
            continue;
        };
        let Some(family) = spec.family.as_deref().filter(|family| !family.is_empty()) else {
            continue;
        };
        if register_host_font_face(family, bytes, spec.weight, spec.weight_end) == 0 {
            continue;
        }
        mark_loaded(key);
        return true;
    }
    false
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
}
