//! Bridge resources state and operations.

use super::*;

#[derive(Debug, Default)]
pub(super) struct State {
    /// Parsed `@font-face` rules registered with the host font system when possible.
    pub(super) font_faces: Vec<FontFaceRule>,
    /// Successfully registered faces, keyed by canonical path or `local:name` + family + weight span.
    pub(super) font_register_keys: HashSet<(String, String, u16, u16)>,
    /// Bytes of successfully registered `@font-face` files (host memory cap).
    pub(super) font_bytes_used: u64,
    /// Base directory for relative `@import` / `@font-face` `url(...)`.
    pub(super) stylesheet_base: PathBuf,
    /// Parsed imported sheets keyed by canonical href (not re-parsed every frame).
    pub(super) import_cache: HashMap<String, ParsedStylesheet>,
}

impl MessageBridge {
    pub(super) fn try_register_url_font_face(&mut self, face: &FontFaceRule, url: &str) -> bool {
        let Some((bytes, canonical)) = load_font_face_bytes(
            url,
            face.base_href.as_deref(),
            &self.resources.stylesheet_base,
        ) else {
            return false;
        };
        let key = font_face_register_key(canonical.to_string_lossy().into_owned(), face);
        if self.resources.font_register_keys.contains(&key) {
            return true;
        }
        let add = bytes.len() as u64;
        if font_registration_would_exceed_cap(self.resources.font_bytes_used, add) {
            return false;
        }
        #[cfg(feature = "scene-view")]
        {
            if nana_ui::register_host_font_face(&face.family, bytes, face.weight, face.weight_end)
                == 0
            {
                return false;
            }
        }
        #[cfg(not(feature = "scene-view"))]
        {
            let _ = bytes;
        }
        self.resources.font_register_keys.insert(key);
        self.resources.font_bytes_used = self.resources.font_bytes_used.saturating_add(add);
        self.resources.font_faces.push(face.clone());
        true
    }
}

impl MessageBridge {
    pub(super) fn try_register_local_font_face(
        &mut self,
        face: &FontFaceRule,
        local_name: &str,
    ) -> bool {
        let key = font_face_register_key(format!("local:{local_name}"), face);
        if self.resources.font_register_keys.contains(&key) {
            return true;
        }
        if !host_alias_local_font_face(&face.family, local_name, face.weight, face.weight_end) {
            return false;
        }
        self.resources.font_register_keys.insert(key);
        self.resources.font_faces.push(face.clone());
        true
    }
}

impl MessageBridge {
    /// Register a flattened `@font-face` after the first `src` entry that
    /// resolves. `local()` and `url()` are tried in CSS order: a matching
    /// system/bundled family wins without loading bytes; otherwise the next
    /// source is tried. Unmatched `local()` is fail-closed (skip, not error).
    pub(super) fn consider_font_face(&mut self, face: &FontFaceRule) {
        for src in &face.src {
            match src {
                FontFaceSrc::Local(local_name) => {
                    if self.try_register_local_font_face(face, local_name) {
                        return;
                    }
                }
                FontFaceSrc::Url(url) => {
                    if self.try_register_url_font_face(face, url) {
                        return;
                    }
                }
            }
        }
    }
}

impl MessageBridge {
    pub fn registered_font_faces(&self) -> &[FontFaceRule] {
        &self.resources.font_faces
    }
}

impl MessageBridge {
    pub fn set_stylesheet_base(&mut self, base: PathBuf) {
        if !stylesheet_base_is_set(&base) {
            self.resources.stylesheet_base = PathBuf::new();
            return;
        }
        self.resources.stylesheet_base = std::fs::canonicalize(&base).unwrap_or(base);
    }
}
