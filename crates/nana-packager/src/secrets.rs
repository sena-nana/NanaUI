//! Content keys and the publisher signing key: where they come from, and
//! proof that none of them leaked into the output.
//!
//! Keys are read from environment variables or key files named on the
//! command line, never from `nana-package.toml`. A key file inside the
//! config's repository is refused (it would be one `git add` away from
//! being committed) unless explicitly allowed for tests.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use nana_package::{ContentKey, KeyId, PublisherKey};
use zeroize::Zeroizing;

pub const PUBLISHER_ENV: &str = "NANA_PUBLISHER_SIGNING_KEY";

/// Environment variable carrying content key `name`:
/// `NANA_CONTENT_KEY_<NAME>` with `-` and `.` mapped to `_`, uppercased.
pub fn content_key_env(name: &str) -> String {
    let suffix: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("NANA_CONTENT_KEY_{suffix}")
}

/// Where secrets come from, as given on the command line.
#[derive(Debug, Default, Clone)]
pub struct SecretSources {
    /// `--content-key-file name=PATH`.
    pub content_key_files: BTreeMap<String, PathBuf>,
    /// `--signing-key-file PATH`.
    pub signing_key_file: Option<PathBuf>,
    /// `--allow-in-tree-key` (tests only).
    pub allow_in_tree: bool,
}

/// Secrets loaded for one packaging run.
pub struct LoadedSecrets {
    /// Key name → (id, generation, key).
    pub content: BTreeMap<String, (KeyId, u32, ContentKey)>,
    pub publisher: Option<(SigningKey, PublisherKey)>,
    /// Every secret byte string, for [`scan_for_leaks`].
    raw: Vec<Zeroizing<Vec<u8>>>,
}

impl std::fmt::Debug for LoadedSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedSecrets")
            .field("content", &self.content.keys().collect::<Vec<_>>())
            .field(
                "publisher",
                &self.publisher.as_ref().map(|(_, p)| p.to_text()),
            )
            .finish()
    }
}

impl LoadedSecrets {
    pub fn empty() -> Self {
        Self {
            content: BTreeMap::new(),
            publisher: None,
            raw: Vec::new(),
        }
    }

    pub fn add_content_key(&mut self, name: &str, generation: u32, key: ContentKey) {
        self.raw.push(Zeroizing::new(key.as_bytes().to_vec()));
        self.content
            .insert(name.to_owned(), (KeyId::from_name(name), generation, key));
    }

    pub fn set_publisher(&mut self, seed: [u8; 32]) -> PublisherKey {
        let signing = SigningKey::from_bytes(&seed);
        let public = PublisherKey::from_bytes(signing.verifying_key().as_bytes())
            .expect("a derived Ed25519 public key is valid");
        self.raw.push(Zeroizing::new(seed.to_vec()));
        self.raw
            .push(Zeroizing::new(signing.to_keypair_bytes().to_vec()));
        self.publisher = Some((signing, public.clone()));
        public
    }

    /// Load the content keys `needed` (name → generation) and, when
    /// `publisher` is configured, the signing key.
    pub fn load(
        needed: &BTreeMap<String, u32>,
        want_publisher: bool,
        sources: &SecretSources,
        repo_root: Option<&Path>,
    ) -> Result<Self, String> {
        let mut secrets = Self::empty();
        for (name, generation) in needed {
            let text = match sources.content_key_files.get(name) {
                Some(path) => read_key_file(path, sources.allow_in_tree, repo_root)?,
                None => {
                    let var = content_key_env(name);
                    Zeroizing::new(std::env::var(&var).map_err(|_| {
                        format!(
                            "content key `{name}` is not available: set {var} (64 hex digits) or pass \
                             --content-key-file {name}=PATH"
                        )
                    })?)
                }
            };
            let key = ContentKey::from_hex(&text)
                .ok_or_else(|| format!("content key `{name}` must be 64 hex digits"))?;
            secrets.add_content_key(name, *generation, key);
        }
        if want_publisher {
            let text = match &sources.signing_key_file {
                Some(path) => read_key_file(path, sources.allow_in_tree, repo_root)?,
                None => Zeroizing::new(std::env::var(PUBLISHER_ENV).map_err(|_| {
                    format!(
                        "the publisher signing key is not available: set {PUBLISHER_ENV} (64 hex \
                         digits, the Ed25519 seed) or pass --signing-key-file PATH"
                    )
                })?),
            };
            let seed = nana_package::from_hex::<32>(text.trim())
                .ok_or("the publisher signing key must be 64 hex digits")?;
            secrets.set_publisher(seed);
        }
        Ok(secrets)
    }
}

fn read_key_file(
    path: &Path,
    allow_in_tree: bool,
    repo_root: Option<&Path>,
) -> Result<Zeroizing<String>, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("cannot read key file {}: {error}", path.display()))?;
    if !allow_in_tree
        && let Some(root) = repo_root.and_then(|root| std::fs::canonicalize(root).ok())
        && canonical.starts_with(&root)
    {
        return Err(format!(
            "key file {} is inside the repository at {}; keep keys outside the source tree \
             (CI secrets, a keychain, or a directory outside the checkout)",
            path.display(),
            root.display()
        ));
    }
    std::fs::read_to_string(&canonical)
        .map(Zeroizing::new)
        .map_err(|error| format!("cannot read key file {}: {error}", path.display()))
}

/// The enclosing git work tree of `dir`, if any.
pub fn repository_root(dir: &Path) -> Option<PathBuf> {
    let dir = std::fs::canonicalize(dir).ok()?;
    dir.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Every encoding a secret could be written in by mistake. Base64 is
/// matched at all three alignments (a key embedded after 1 or 2 other bytes,
/// as in a PEM / PKCS#8 blob, encodes differently), using only the
/// characters the secret's bytes fully determine.
fn encodings(secret: &[u8]) -> Vec<Vec<u8>> {
    let hex = nana_package::to_hex(secret);
    let mut out = vec![
        secret.to_vec(),
        hex.clone().into_bytes(),
        hex.to_ascii_uppercase().into_bytes(),
    ];
    for url in [false, true] {
        for shift in 0..3usize {
            let mut shifted = vec![0u8; shift];
            shifted.extend_from_slice(secret);
            let text = base64(&shifted, url);
            // Character i covers bits [6i, 6i + 6); keep those inside the
            // secret's bits [8 * shift, 8 * (shift + len)).
            let (start, end) = (8 * shift, 8 * (shift + secret.len()));
            let first = start.div_ceil(6);
            let last = end / 6;
            if last > first {
                out.push(text.as_bytes()[first..last].to_vec());
            }
        }
    }
    out
}

/// Unpadded base64 (a padded encoding contains the unpadded one).
fn base64(bytes: &[u8], url: bool) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
    if url {
        URL_SAFE_NO_PAD.encode(bytes)
    } else {
        STANDARD_NO_PAD.encode(bytes)
    }
}

/// Scan every file under `root` for any loaded secret in any encoding.
/// Returns the offending relative paths (never the secret).
pub fn scan_for_leaks(root: &Path, secrets: &LoadedSecrets) -> Result<Vec<String>, String> {
    let needles: Vec<Vec<u8>> = secrets.raw.iter().flat_map(|s| encodings(s)).collect();
    let mut leaks = Vec::new();
    if needles.is_empty() {
        return Ok(leaks);
    }
    for path in crate::util::walk_files(root)? {
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("cannot scan {}: {error}", path.display()))?;
        if needles
            .iter()
            .any(|needle| crate::util::contains(&bytes, needle))
        {
            leaks.push(crate::util::relative_path(root, &path)?);
        }
    }
    Ok(leaks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_names() {
        assert_eq!(
            content_key_env("content-main"),
            "NANA_CONTENT_KEY_CONTENT_MAIN"
        );
        assert_eq!(content_key_env("dlc.1"), "NANA_CONTENT_KEY_DLC_1");
    }

    #[test]
    fn leak_scan_finds_every_encoding() {
        let dir = std::env::temp_dir().join(format!("nana-packager-leak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let mut secrets = LoadedSecrets::empty();
        let key = [0x5a; 32];
        secrets.add_content_key("main", 1, ContentKey::from_bytes(key));
        std::fs::write(dir.join("clean.json"), b"{\"ok\":true}").unwrap();
        assert!(scan_for_leaks(&dir, &secrets).unwrap().is_empty());
        for (name, body) in [
            ("raw.bin", key.to_vec()),
            ("hex.txt", nana_package::to_hex(&key).into_bytes()),
            (
                "upper.txt",
                nana_package::to_hex(&key).to_uppercase().into_bytes(),
            ),
            ("b64.txt", base64(&key, false).into_bytes()),
            ("sub/url.txt", base64(&key, true).into_bytes()),
        ] {
            std::fs::write(dir.join(name), body).unwrap();
        }
        // Embedded in a longer base64 stream at an unaligned offset.
        let mut pem_like = vec![0x30, 0x2e, 0x02, 0x01];
        pem_like.extend_from_slice(&key);
        std::fs::write(dir.join("sub/pem.txt"), base64(&pem_like, false)).unwrap();
        let leaks = scan_for_leaks(&dir, &secrets).unwrap();
        assert_eq!(leaks.len(), 6, "{leaks:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_tree_key_files_are_refused() {
        let root = std::env::temp_dir().join(format!("nana-packager-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let file = root.join("key.hex");
        std::fs::write(&file, "00".repeat(32)).unwrap();
        let repo = repository_root(&root).unwrap();
        assert!(
            read_key_file(&file, false, Some(&repo))
                .unwrap_err()
                .contains("inside the repository")
        );
        assert!(read_key_file(&file, true, Some(&repo)).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }
}
