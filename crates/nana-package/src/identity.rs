//! The application identity marker embedded by
//! `nana_ui_platform::application_identity!`.
//!
//! Layout (UTF-8, NUL separated):
//!
//! ```text
//! NANA-IDENTITY-V1 \0 id \0 name \0 version \0 vendor \0 END \0
//! ```
//!
//! The packager reads it from the built executable and refuses to package a
//! binary whose identity disagrees with `nana-package.toml`; the validator
//! compares it with the manifest.

use std::fmt;

/// Marker prefix. Must stay in sync with `nana_ui_platform::application_identity!`.
pub const MARKER_MAGIC: &[u8] = b"NANA-IDENTITY-V1\0";
const MARKER_END: &[u8] = b"END\0";
const MAX_FIELD: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedIdentity {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Empty when the application declared none.
    pub vendor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkerError {
    /// The binary was not built with `application_identity!` (or the marker
    /// was stripped).
    Missing,
    /// More than one well-formed marker, with different contents.
    Ambiguous,
}

impl fmt::Display for MarkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str(
                "no application identity marker; declare the identity with \
                 nana_ui_platform::application_identity!",
            ),
            Self::Ambiguous => f.write_str("the binary carries conflicting identity markers"),
        }
    }
}

/// Find the one well-formed identity marker in `binary`.
///
/// The magic string also occurs wherever this parser is linked; such an
/// occurrence is not followed by five NUL-terminated fields ending in `END`
/// and is skipped. Identical duplicates (a marker referenced from two
/// codegen units) are accepted.
pub fn find_marker(binary: &[u8]) -> Result<EmbeddedIdentity, MarkerError> {
    let mut found: Option<EmbeddedIdentity> = None;
    let mut start = 0;
    while let Some(offset) = find(&binary[start..], MARKER_MAGIC) {
        let at = start + offset + MARKER_MAGIC.len();
        if let Some(identity) = parse_fields(&binary[at..]) {
            match &found {
                Some(existing) if *existing != identity => return Err(MarkerError::Ambiguous),
                Some(_) => {}
                None => found = Some(identity),
            }
        }
        start = at;
    }
    found.ok_or(MarkerError::Missing)
}

fn parse_fields(bytes: &[u8]) -> Option<EmbeddedIdentity> {
    let mut fields = Vec::with_capacity(4);
    let mut rest = bytes;
    for _ in 0..4 {
        let end = rest.iter().take(MAX_FIELD + 1).position(|&b| b == 0)?;
        let field = std::str::from_utf8(&rest[..end]).ok()?;
        if field.chars().any(char::is_control) {
            return None;
        }
        fields.push(field.to_owned());
        rest = &rest[end + 1..];
    }
    if !rest.starts_with(MARKER_END) || fields[0].is_empty() || fields[2].is_empty() {
        return None;
    }
    let vendor = fields.pop()?;
    let version = fields.pop()?;
    let name = fields.pop()?;
    let id = fields.pop()?;
    Some(EmbeddedIdentity {
        id,
        name,
        version,
        vendor,
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(id: &str, name: &str, version: &str, vendor: &str) -> Vec<u8> {
        let mut out = MARKER_MAGIC.to_vec();
        for field in [id, name, version, vendor] {
            out.extend_from_slice(field.as_bytes());
            out.push(0);
        }
        out.extend_from_slice(MARKER_END);
        out
    }

    #[test]
    fn finds_marker_among_other_bytes() {
        let mut binary = b"\x7fELF....".to_vec();
        // A bare magic (as the parser's own literal would appear) is skipped.
        binary.extend_from_slice(MARKER_MAGIC);
        binary.extend_from_slice(b"some other rodata");
        binary.extend(marker("dev.nana.app", "Nana App", "1.2.3", ""));
        binary.extend_from_slice(b"tail");
        assert_eq!(
            find_marker(&binary).unwrap(),
            EmbeddedIdentity {
                id: "dev.nana.app".into(),
                name: "Nana App".into(),
                version: "1.2.3".into(),
                vendor: String::new(),
            }
        );
    }

    #[test]
    fn identical_duplicates_are_fine_conflicts_are_not() {
        let one = marker("a.b", "A", "1", "v");
        let mut twice = one.clone();
        twice.extend_from_slice(&one);
        assert!(find_marker(&twice).is_ok());
        let mut conflict = one;
        conflict.extend(marker("a.b", "A", "2", "v"));
        assert_eq!(find_marker(&conflict), Err(MarkerError::Ambiguous));
    }

    #[test]
    fn missing_or_truncated_marker_is_missing() {
        assert_eq!(find_marker(b"nothing here"), Err(MarkerError::Missing));
        let mut truncated = marker("a.b", "A", "1", "");
        truncated.truncate(truncated.len() - 2);
        assert_eq!(find_marker(&truncated), Err(MarkerError::Missing));
    }
}
