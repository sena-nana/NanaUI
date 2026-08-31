//! Load `@font-face` `url(...)` srcs into the process FontSystem.
//!
//! Called from stylesheet **inject**, not parse. Reuses
//! [`resolve_background_image_url`] / document URL base.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::nana_text::{HostFontStyle, register_host_font_face};
use crate::scene_paint::{resolve_background_image_url, resolved_resource_is_allowed};

const FONT_FACE_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const FONT_FACE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// One parsed `@font-face` ready for host ingest (no CSSOM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFontFaceSpec {
    pub family: Option<String>,
    /// `url(...)` values only; `local(...)` is ignored.
    pub urls: Vec<String>,
    pub weight: Option<u16>,
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

/// Fetch each face's first usable `url(...)` and register it with CSS aliases.
///
/// Already-loaded keys are skipped. Failed srcs drop that face (no system-font
/// stand-in). HTTP uses a 5s timeout and 8 MiB cap.
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
        if !resolved_resource_is_allowed(&resolved) {
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
        if register_host_font_face(family, bytes, spec.weight, None) == 0 {
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
    if resolved.starts_with("http://") || resolved.starts_with("https://") {
        return fetch_http_bytes(resolved);
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

fn fetch_http_bytes(url: &str) -> Option<Vec<u8>> {
    nana_ui_platform::fetch_bytes_blocking(url, FONT_FACE_FETCH_TIMEOUT, FONT_FACE_MAX_BYTES as u64)
}
