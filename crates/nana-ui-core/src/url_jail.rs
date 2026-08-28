//! Filesystem jail shared by stylesheets, `@font-face`, and `url()` images.
//!
//! Not a CSS parser: only path/host checks and canonicalize-under-root.

use std::path::{Path, PathBuf};

/// Same cap as `@font-face` file reads (`nana-ui-vue::MAX_FONT_FACE_BYTES`).
pub const MAX_LOCAL_URL_BYTES: u64 = 8 * 1024 * 1024;

/// `path` must exist and canonicalize under `jail`.
pub fn canonicalize_within_jail(path: &Path, jail: &Path) -> Option<PathBuf> {
    if path_looks_network(path) {
        return None;
    }
    let jail_canon = std::fs::canonicalize(jail).ok()?;
    let path_canon = std::fs::canonicalize(path).ok()?;
    if path_canon.starts_with(&jail_canon) {
        Some(path_canon)
    } else {
        None
    }
}

/// Resolve a relative / `file://` href against `from` or `base`.
///
/// Does not read the file. Callers must [`canonicalize_within_jail`] before use.
/// Remote schemes (`http`, `https`, `data`) and non-local `file` hosts are refused.
pub fn resolve_filesystem_href(href: &str, from: Option<&str>, base: &Path) -> Option<PathBuf> {
    let trimmed = href.trim();
    if trimmed.is_empty()
        || is_remote_or_data_href(trimmed)
        || href_is_protocol_relative_or_unc(trimmed)
    {
        return None;
    }
    if trimmed.to_ascii_lowercase().starts_with("file:") {
        return file_url_to_path(trimmed).filter(|path| !path_looks_network(path));
    }
    let decoded = percent_decode_if_needed(trimmed)?;
    if href_is_protocol_relative_or_unc(&decoded) {
        return None;
    }
    let path = Path::new(&decoded);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let origin = from.map(Path::new).and_then(|p| p.parent()).unwrap_or(base);
        origin.join(decoded)
    };
    if path_looks_network(&resolved) {
        None
    } else {
        Some(resolved)
    }
}

/// Read a local image/font-like URL only if it stays inside `jail` and under `max_bytes`.
pub fn read_bytes_within_jail(href: &str, jail: &Path, max_bytes: u64) -> Option<Vec<u8>> {
    let path = resolve_filesystem_href(href, None, jail)?;
    let canonical = canonicalize_within_jail(&path, jail)?;
    let meta = std::fs::metadata(&canonical).ok()?;
    if meta.len() > max_bytes {
        return None;
    }
    std::fs::read(&canonical).ok()
}

/// Local directory jail for a Vue document / SFC / stylesheet href.
///
/// Remote schemes, protocol-relative `//`, UNC `\\`, `nana:` / `blob:`, bare
/// filenames, and `.` return `None` so relative `@import` skips instead of
/// scanning cwd or a network share.
pub fn stylesheet_base_from_href(href: &str) -> Option<PathBuf> {
    let trimmed = href.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if is_remote_or_data_href(trimmed)
        || href_is_protocol_relative_or_unc(trimmed)
        || lower.starts_with("nana:")
        || lower.starts_with("blob:")
    {
        return None;
    }
    let path = if lower.starts_with("file:") {
        file_url_to_path(trimmed).filter(|path| !path_looks_network(path))?
    } else {
        PathBuf::from(trimmed)
    };
    if path_looks_network(&path) {
        return None;
    }
    let dir = if path.is_dir() {
        path
    } else {
        let parent = path.parent()?;
        if parent.as_os_str().is_empty() {
            return None;
        }
        parent.to_path_buf()
    };
    if dir.as_os_str().is_empty() || dir == Path::new(".") || path_looks_network(&dir) {
        return None;
    }
    Some(dir)
}

/// `http(s)` / `data:` — not a filesystem href.
pub fn is_remote_or_data_href(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:")
}

/// `//host/...`, UNC, and percent-decoded forms of those.
pub fn href_is_protocol_relative_or_unc(href: &str) -> bool {
    let trimmed = href.trim();
    if looks_protocol_relative_or_unc(trimmed) {
        return true;
    }
    percent_decode_if_needed(trimmed)
        .is_some_and(|decoded| looks_protocol_relative_or_unc(decoded.trim()))
}

/// `file://` → path. Non-local hosts (`file://evil.example/...`) are refused.
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.get(5..)?; // after "file:"
    let (host, path_part) = if let Some(after) = rest.strip_prefix("//") {
        if after.starts_with('/') {
            ("", after)
        } else if is_windows_drive_path(after) {
            // `file://C:\foo` / `file://C:/foo` (missing third slash).
            ("", after)
        } else {
            let slash = after.find(['/', '\\'])?;
            (&after[..slash], &after[slash..])
        }
    } else {
        ("", rest)
    };
    if !host.is_empty() && !is_local_file_host(host) {
        return None;
    }
    let decoded = percent_decode_if_needed(path_part)?;
    if looks_protocol_relative_or_unc(&decoded) {
        return None;
    }
    #[cfg(windows)]
    {
        let trimmed = decoded.trim_start_matches('/');
        Some(PathBuf::from(trimmed.replace('/', "\\")))
    }
    #[cfg(not(windows))]
    {
        if decoded.starts_with('/') {
            Some(PathBuf::from(decoded))
        } else {
            Some(PathBuf::from(format!("/{decoded}")))
        }
    }
}

fn is_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_local_file_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host.eq_ignore_ascii_case("[::1]")
}

fn looks_protocol_relative_or_unc(href: &str) -> bool {
    let t = href.trim();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("%2f%2f") || lower.starts_with("%5c%5c") {
        return true;
    }
    let path = Path::new(t);
    if is_windows_local_disk_path(path) {
        return false;
    }
    if is_windows_unc_path(path) {
        return true;
    }
    // `\\?\C:\...` (canonical VerbatimDisk) becomes `//?/C:/...` after slash
    // normalize. Strip that local-drive prefix before the `//host` check so
    // it is not confused with protocol-relative `//cdn...` or UNC `\\server`.
    let normalized = t.replace('\\', "/");
    let candidate = strip_verbatim_disk_prefix(&normalized).unwrap_or(normalized.as_str());
    candidate.starts_with("//")
}

/// Windows UNC prefix, or a path string that is protocol-relative / UNC.
///
/// `Prefix::VerbatimDisk` (`\\?\C:\...`) is a local drive, not UNC.
pub fn path_looks_network(path: &Path) -> bool {
    if is_windows_unc_path(path) {
        return true;
    }
    if is_windows_local_disk_path(path) {
        return false;
    }
    href_is_protocol_relative_or_unc(&path.to_string_lossy())
}

fn is_windows_unc_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _))
        )
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn is_windows_local_disk_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::VerbatimDisk(_) | Prefix::Disk(_))
        )
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Remainder after a Windows `\\?\X:` / `//?/X:` VerbatimDisk prefix, if any.
fn strip_verbatim_disk_prefix(normalized_fwd: &str) -> Option<&str> {
    let rest = normalized_fwd.strip_prefix("//?")?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => Some(rest),
        _ => None,
    }
}

fn percent_decode_if_needed(input: &str) -> Option<String> {
    if !input.as_bytes().contains(&b'%') {
        return Some(input.to_string());
    }
    percent_decode(input)
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hi = from_hex(bytes[index + 1])?;
            let lo = from_hex(bytes[index + 2])?;
            out.push((hi << 4) | lo);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_rejects_remote_host() {
        assert!(file_url_to_path("file://evil.example/etc/passwd").is_none());
        assert!(
            resolve_filesystem_href("file://evil.example/etc/passwd", None, Path::new("."))
                .is_none()
        );
    }

    #[test]
    fn file_localhost_is_local() {
        let path = file_url_to_path("file://localhost/tmp/test.png").expect("localhost");
        assert!(
            path.to_string_lossy()
                .replace('\\', "/")
                .ends_with("tmp/test.png"),
            "got {}",
            path.display()
        );
    }

    #[test]
    fn file_url_windows_drive_without_third_slash_is_local() {
        let slash = file_url_to_path("file://C:/fonts/ok.ttf").expect("drive slash");
        assert!(
            slash
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("fonts/ok.ttf"),
            "got {}",
            slash.display()
        );
        let backslash = file_url_to_path(r"file://C:\fonts\ok.ttf").expect("drive backslash");
        assert!(
            backslash
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("fonts/ok.ttf"),
            "got {}",
            backslash.display()
        );
    }

    #[test]
    fn jail_skips_path_outside_root() {
        let jail =
            std::env::temp_dir().join(format!("nanaui-url-jail-{}-{}", std::process::id(), "in"));
        let outside =
            std::env::temp_dir().join(format!("nanaui-url-jail-{}-{}", std::process::id(), "out"));
        let _ = std::fs::remove_dir_all(&jail);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&jail).expect("jail");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(outside.join("secret.png"), b"nope").expect("secret");
        assert!(
            read_bytes_within_jail(
                outside.join("secret.png").to_string_lossy().as_ref(),
                &jail,
                MAX_LOCAL_URL_BYTES
            )
            .is_none()
        );
        assert!(read_bytes_within_jail("../secret.png", &jail, MAX_LOCAL_URL_BYTES).is_none());
        let _ = std::fs::remove_dir_all(&jail);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn protocol_relative_and_unc_are_blocked() {
        assert!(href_is_protocol_relative_or_unc("//evil.example/a.css"));
        assert!(href_is_protocol_relative_or_unc(
            r"\\evil.example\share\a.css"
        ));
        assert!(href_is_protocol_relative_or_unc("%2f%2fevil.example/a.css"));
        assert!(!href_is_protocol_relative_or_unc("/local/a.css"));
        assert!(resolve_filesystem_href("//evil.example/a.css", None, Path::new(".")).is_none());
        assert!(
            resolve_filesystem_href(r"\\evil.example\share\a.css", None, Path::new(".")).is_none()
        );
        assert!(
            read_bytes_within_jail("//evil.example/a.css", Path::new("."), MAX_LOCAL_URL_BYTES)
                .is_none()
        );
        assert!(file_url_to_path("file://///server/share/a.css").is_none());
        let decoded = file_url_to_path("file:///C:/fonts/My%20Font.ttf").expect("local file");
        assert!(
            decoded.to_string_lossy().contains("My Font.ttf"),
            "percent-decode must survive: {}",
            decoded.display()
        );
        assert!(
            href_is_protocol_relative_or_unc("//cdn.example.com/x.css"),
            "protocol-relative CDN href must stay blocked"
        );
        assert!(!href_is_protocol_relative_or_unc(r"\\?\C:\jail\ok.ttf"));
        assert!(!href_is_protocol_relative_or_unc(r"\\?\c:\jail\ok.ttf"));
        assert!(!path_looks_network(Path::new(r"\\?\C:\jail\ok.ttf")));
        assert!(href_is_protocol_relative_or_unc(
            r"\\?\UNC\server\share\a.css"
        ));
        assert!(path_looks_network(Path::new(r"\\server\share\a.css")));
    }

    #[test]
    fn windows_verbatim_disk_canonical_path_is_allowed_inside_jail() {
        let jail =
            std::env::temp_dir().join(format!("nanaui-url-jail-{}-verbatim", std::process::id()));
        let _ = std::fs::remove_dir_all(&jail);
        std::fs::create_dir_all(&jail).expect("jail");
        std::fs::write(jail.join("ok.ttf"), b"dummy-font-bytes").expect("ttf");
        let jail_canon = std::fs::canonicalize(&jail).expect("canon");
        #[cfg(windows)]
        {
            let lossy = jail_canon.to_string_lossy();
            assert!(
                lossy.starts_with(r"\\?\"),
                "Windows canonicalize should yield VerbatimDisk, got {lossy}"
            );
            assert!(
                !href_is_protocol_relative_or_unc(&lossy),
                "VerbatimDisk jail must not look like UNC"
            );
            assert!(!path_looks_network(&jail_canon));
        }
        let font = jail_canon.join("ok.ttf");
        assert!(
            !href_is_protocol_relative_or_unc(&font.to_string_lossy()),
            r"\\?\C:\jail\ok.ttf must be allowed inside jail"
        );
        let bytes = read_bytes_within_jail(
            font.to_string_lossy().as_ref(),
            &jail_canon,
            MAX_LOCAL_URL_BYTES,
        )
        .expect("canonical VerbatimDisk font inside jail");
        assert_eq!(bytes, b"dummy-font-bytes");
        assert!(resolve_filesystem_href("./ok.ttf", None, &jail_canon).is_some());
        assert!(canonicalize_within_jail(&jail_canon.join("ok.ttf"), &jail_canon).is_some());
        assert!(href_is_protocol_relative_or_unc("//cdn.example.com/x.css"));
        let _ = std::fs::remove_dir_all(&jail);
    }

    #[test]
    fn stylesheet_base_from_href_skips_protocol_relative_and_unc() {
        assert!(stylesheet_base_from_href("//cdn.example.com/npm/pkg/theme.css").is_none());
        assert!(stylesheet_base_from_href(r"\\cdn.example.com\npm\pkg\theme.css").is_none());
        assert!(stylesheet_base_from_href("%2f%2fcdn.example.com/npm/pkg/theme.css").is_none());
        assert!(stylesheet_base_from_href("file://evil.example/fonts/a.css").is_none());
        assert!(stylesheet_base_from_href("https://example.com/a.css").is_none());
        assert!(stylesheet_base_from_href("theme.css").is_none());
        assert!(stylesheet_base_from_href(".").is_none());
        assert!(stylesheet_base_from_href("//cdn.example.com/x.css").is_none());
        #[cfg(windows)]
        {
            assert_eq!(
                stylesheet_base_from_href(r"\\?\C:\jail\src\theme.css"),
                Some(PathBuf::from(r"\\?\C:\jail\src")),
                "VerbatimDisk is a local jail, not UNC"
            );
        }
        let dir = stylesheet_base_from_href("src/App.vue").expect("relative dir");
        assert_eq!(dir, PathBuf::from("src"));
    }
}
