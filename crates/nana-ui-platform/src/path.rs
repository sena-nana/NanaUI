//! File-system paths and their user-facing spelling.
//!
//! Keep the real path for I/O, identity, persistence and clipboard actions.
//! Display strings may omit an extended-length prefix and contain optional
//! line-break opportunities; never turn them back into file-system paths.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// Remove a Windows verbatim prefix only when ordinary Win32 spelling has
/// the same meaning. This performs no I/O and leaves other platforms alone.
/// Long paths, special names, device paths and non-Unicode paths are preserved.
pub fn normalize_path(path: &Path) -> Cow<'_, Path> {
    #[cfg(windows)]
    if let Some(text) = path.to_str()
        && let Some(plain) = ordinary_windows_path(text)
        // Leave room for legacy directory APIs (MAX_PATH - 12), including NUL.
        && plain.encode_utf16().count() < 248
    {
        return match plain {
            Cow::Borrowed(text) => Cow::Borrowed(Path::new(text)),
            Cow::Owned(text) => Cow::Owned(PathBuf::from(text)),
        };
    }
    Cow::Borrowed(path)
}

/// Resolve a path on disk, then apply [`normalize_path`]. Errors are those of
/// [`std::fs::canonicalize`]. Run this on a worker, never to format UI text.
pub fn canonicalize_path(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)?;
    Ok(normalize_path(&canonical).into_owned())
}

/// Format a path for read-only UI. Ordinary Windows disk and UNC paths omit
/// the verbatim prefix even when long. Special names retain their spelling.
/// `wrap_separators` adds U+200B after separators, for narrow labels only.
/// This is lossy text, not a path to save, copy, edit, or open. No I/O occurs.
pub fn display_path(path: impl AsRef<Path>, wrap_separators: bool) -> String {
    let path = path.as_ref();
    let text = path.to_string_lossy();
    #[cfg(windows)]
    let text = match path.to_str().and_then(ordinary_windows_path) {
        Some(plain) => plain,
        None => text,
    };
    if !wrap_separators {
        return text.into_owned();
    }
    let mut shown = String::with_capacity(text.len() + text.len() / 8);
    for character in text.chars() {
        shown.push(character);
        if matches!(character, '\\' | '/') {
            shown.push('\u{200B}');
        }
    }
    shown
}

#[cfg(any(windows, test))]
fn ordinary_windows_path(text: &str) -> Option<Cow<'_, str>> {
    let rest = text.strip_prefix(r"\\?\")?;
    let (plain, components) = if rest
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"UNC\"))
    {
        let unc = &rest[4..];
        let mut root = unc.split('\\');
        if root.next()?.is_empty() || root.next()?.is_empty() {
            return None;
        }
        (Cow::Owned(format!(r"\\{unc}")), unc)
    } else if rest.as_bytes().first()?.is_ascii_alphabetic()
        && rest.as_bytes().get(1..3) == Some(b":\\")
    {
        (Cow::Borrowed(rest), &rest[3..])
    } else {
        return None;
    };
    let components = components.strip_suffix('\\').unwrap_or(components);
    if !components.is_empty() && !components.split('\\').all(ordinary_component) {
        return None;
    }
    Some(plain)
}

#[cfg(any(windows, test))]
fn ordinary_component(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name.encode_utf16().count() > 255
        || name
            .chars()
            .any(|c| c <= '\u{1f}' || "<>:\"/|?*".contains(c))
    {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return false;
    }
    let upper = stem.to_ascii_uppercase();
    !["COM", "LPT"].iter().any(|prefix| {
        upper.strip_prefix(prefix).is_some_and(|number| {
            matches!(
                number,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_spellings_distinguish_paths_from_devices_and_special_names() {
        for (raw, plain) in [
            (r"\\?\C:\模型\Nana Live.json", r"C:\模型\Nana Live.json"),
            (r"\\?\UNC\nas\share\模型", r"\\nas\share\模型"),
            (r"\\?\unc\nas\share\模型", r"\\nas\share\模型"),
            (r"\\?\C:\", r"C:\"),
        ] {
            assert_eq!(ordinary_windows_path(raw).as_deref(), Some(plain));
        }
        for raw in [
            r"\\?\C:\name.",
            r"\\?\C:\name ",
            r"\\?\C:\a\..\b",
            r"\\?\C:\a/child",
            r"\\?\C:\con.txt",
            r"\\?\C:\COM¹.txt",
            r"\\?\C:\con .txt",
            r"\\?\C:\CONIN$",
            r"\\?\C:\a:b",
            r"\\?\UNC\nas\share\name.",
            r"\\?\UNC\nas",
            r"\\?\C:relative",
            r"\\?\Volume{1234}\file",
            r"\\.\PhysicalDrive0",
            r"\\?\GLOBALROOT\Device\x",
        ] {
            assert_eq!(ordinary_windows_path(raw), None, "{raw}");
        }
    }

    #[test]
    fn display_wrapping_is_opt_in_and_does_not_change_the_path() {
        let raw = Path::new("/模型/Nana Live.json");
        assert_eq!(display_path(raw, false), "/模型/Nana Live.json");
        assert_eq!(
            display_path(raw, true),
            "/\u{200B}模型/\u{200B}Nana Live.json"
        );
        assert_eq!(raw, Path::new("/模型/Nana Live.json"));
        assert_eq!(normalize_path(raw).as_ref(), raw);
    }

    #[cfg(windows)]
    #[test]
    fn normalizing_is_idempotent_and_display_handles_long_disk_and_unc_paths() {
        for raw in [r"\\?\C:\模型\a b.json", r"\\?\UNC\nas\share\a b.json"] {
            let raw = Path::new(raw);
            let plain = normalize_path(raw);
            assert!(!plain.to_string_lossy().starts_with(r"\\?\"));
            assert_eq!(normalize_path(&plain).as_ref(), plain.as_ref());
            assert_eq!(display_path(raw, false), plain.to_string_lossy());
        }
        for prefix in [r"\\?\C:\", r"\\?\UNC\nas\share\"] {
            let long = format!("{prefix}{}\\{}", "模".repeat(150), "x".repeat(150));
            assert_eq!(normalize_path(Path::new(&long)).as_ref(), Path::new(&long));
            assert!(!display_path(&long, false).starts_with(r"\\?\"));
        }
        let unicode_short = format!(r"\\?\C:\{}", "模".repeat(100));
        assert!(
            !normalize_path(Path::new(&unicode_short))
                .to_string_lossy()
                .starts_with(r"\\?\")
        );
        let wide_long = format!(r"\\?\C:\{}", "😀".repeat(125));
        assert_eq!(
            normalize_path(Path::new(&wide_long)).as_ref(),
            Path::new(&wide_long)
        );
        for raw in [
            r"\\?\C:\name.",
            r"\\?\UNC\nas\share\NUL.txt",
            r"\\?\Volume{1234}\x",
        ] {
            assert_eq!(normalize_path(Path::new(raw)).as_ref(), Path::new(raw));
            assert_eq!(display_path(raw, false), raw);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn windows_looking_names_on_other_platforms_are_untouched() {
        let raw = Path::new(r"\\?\C:\foo");
        assert_eq!(normalize_path(raw).as_ref(), raw);
        assert_eq!(display_path(raw, false), raw.to_string_lossy());
    }

    #[test]
    fn canonicalization_preserves_identity_special_names_and_errors() {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("nana-path-{}-{seq}", std::process::id(),));
        std::fs::create_dir(&root).unwrap();
        let file = root.join("Nana 模型.json");
        std::fs::write(&file, b"model").unwrap();
        let canonical = canonicalize_path(&file).unwrap();
        assert_eq!(
            std::fs::canonicalize(&canonical).unwrap(),
            std::fs::canonicalize(&file).unwrap()
        );
        assert_eq!(std::fs::read(&canonical).unwrap(), b"model");
        assert_eq!(
            canonicalize_path(root.join("missing")).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            let invalid = PathBuf::from(std::ffi::OsString::from_wide(&[
                92, 92, 63, 92, 67, 58, 92, 0xd800,
            ]));
            assert_eq!(normalize_path(&invalid).as_ref(), invalid.as_path());
            let special = std::fs::canonicalize(&root).unwrap().join("name.");
            std::fs::write(&special, b"special").unwrap();
            let actual = canonicalize_path(&special).unwrap();
            assert!(actual.to_string_lossy().starts_with(r"\\?\"));
            assert_eq!(std::fs::read(&actual).unwrap(), b"special");
            std::fs::remove_file(special).unwrap();
            let long_directory = std::fs::canonicalize(&root)
                .unwrap()
                .join("a".repeat(120))
                .join("b".repeat(120));
            std::fs::create_dir_all(&long_directory).unwrap();
            let long_file = long_directory.join("model.json");
            std::fs::write(&long_file, b"long").unwrap();
            let actual = canonicalize_path(&long_file).unwrap();
            assert!(actual.to_string_lossy().starts_with(r"\\?\"));
            assert!(!display_path(&actual, false).starts_with(r"\\?\"));
            assert_eq!(std::fs::read(&actual).unwrap(), b"long");
            std::fs::remove_file(long_file).unwrap();
            std::fs::remove_dir(&long_directory).unwrap();
            std::fs::remove_dir(long_directory.parent().unwrap()).unwrap();
        }
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
