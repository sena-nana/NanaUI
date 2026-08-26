//! Resolve `background-image: url(...)` for GPU texture upload.

use std::path::PathBuf;
use std::sync::OnceLock;

use url::Url;

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
pub fn resolve_background_image_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("data:")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        return Some(trimmed.to_string());
    }
    if trimmed.starts_with("file:") {
        return file_url_to_filesystem_path(trimmed);
    }
    Some(join_relative(trimmed))
}

fn file_url_to_filesystem_path(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    parsed
        .to_file_path()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

fn join_relative(rel: &str) -> String {
    let base = url_base();
    if let Ok(base_url) = Url::from_directory_path(&base) {
        if let Ok(joined) = base_url.join(rel) {
            if let Ok(path) = joined.to_file_path() {
                return path.to_string_lossy().into_owned();
            }
        }
    }
    base.join(rel).to_string_lossy().into_owned()
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
        assert_eq!(resolved, "/tmp/test.png");
    }

    #[test]
    fn file_localhost_url_becomes_path() {
        let resolved = resolve_background_image_url("file://localhost/tmp/test.png").unwrap();
        assert_eq!(resolved, "/tmp/test.png");
    }
}
