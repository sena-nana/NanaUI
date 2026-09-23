//! Platform signing adapters (Issue #226 §9).
//!
//! Nana does not reimplement codesign, notarization, Authenticode or MSIX
//! signing. An adapter wraps the platform tool; this module defines the
//! contract and reports, per adapter, whether it actually ran. The built-in
//! adapters are not implemented yet and say so (`NotExecuted`) instead of
//! pretending: an unsigned development build and a signed distribution build
//! are never confused.
//!
//! Credentials (certificates, notarization API keys) are the adapter's to
//! obtain from the CI environment or keychain; they never pass through
//! `nana-package.toml` or the package manifest.

use std::path::Path;

use nana_package::manifest::{PlatformSigning, SignStatus};

use crate::layout::TargetPlatform;

pub struct SignContext<'a> {
    pub platform: TargetPlatform,
    /// The application root (`App/` or `Name.app`).
    pub app_root: &'a Path,
    pub executable: &'a Path,
}

pub trait PlatformSigner {
    fn id(&self) -> &'static str;
    fn applies_to(&self, platform: TargetPlatform) -> bool;
    fn sign(&self, context: &SignContext<'_>) -> SignStatus;
    fn verify(&self, context: &SignContext<'_>) -> SignStatus;
}

/// An adapter whose platform integration is not written yet.
struct Pending {
    id: &'static str,
    platform: TargetPlatform,
}

impl PlatformSigner for Pending {
    fn id(&self) -> &'static str {
        self.id
    }

    fn applies_to(&self, platform: TargetPlatform) -> bool {
        platform == self.platform
    }

    fn sign(&self, _: &SignContext<'_>) -> SignStatus {
        SignStatus::NotExecuted {
            reason: format!(
                "the `{}` adapter is not implemented yet (Issue #226 workstream F); \
                 sign with the platform tool after packaging",
                self.id
            ),
        }
    }

    fn verify(&self, context: &SignContext<'_>) -> SignStatus {
        self.sign(context)
    }
}

pub const KNOWN_ADAPTERS: [&str; 5] = [
    "macos-codesign",
    "macos-notarize",
    "windows-authenticode",
    "msix",
    "linux-package",
];

pub fn adapter(id: &str) -> Option<Box<dyn PlatformSigner>> {
    let platform = match id {
        "macos-codesign" | "macos-notarize" => TargetPlatform::Macos,
        "windows-authenticode" | "msix" => TargetPlatform::Windows,
        "linux-package" => TargetPlatform::Linux,
        _ => return None,
    };
    let id = KNOWN_ADAPTERS.into_iter().find(|known| *known == id)?;
    Some(Box::new(Pending { id, platform }))
}

/// The configured adapters that apply to `platform`, rejecting unknown ids.
pub fn configured(configured: &[String], platform: TargetPlatform) -> Result<Vec<&str>, String> {
    let mut ids = Vec::new();
    for id in configured {
        let signer = adapter(id).ok_or_else(|| {
            format!(
                "unknown signing adapter `{id}` (known: {})",
                KNOWN_ADAPTERS.join(", ")
            )
        })?;
        if signer.applies_to(platform) {
            ids.push(id.as_str());
        }
    }
    Ok(ids)
}

/// Run the configured adapters that apply to `context.platform`.
pub fn run_platform_signing(
    configured: &[String],
    context: &SignContext<'_>,
) -> Result<Vec<PlatformSigning>, String> {
    let mut results = Vec::new();
    for id in self::configured(configured, context.platform)? {
        let signer = adapter(id).expect("configured() checked the id");
        let status = match signer.sign(context) {
            SignStatus::Signed => signer.verify(context),
            other => other,
        };
        results.push(PlatformSigning {
            adapter: id.to_owned(),
            status,
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_adapters_report_not_executed() {
        let context = SignContext {
            platform: TargetPlatform::Macos,
            app_root: Path::new("/x.app"),
            executable: Path::new("/x.app/Contents/MacOS/x"),
        };
        let results = run_platform_signing(
            &["macos-codesign".into(), "windows-authenticode".into()],
            &context,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].status, SignStatus::NotExecuted { .. }));
        assert!(!results[0].status.is_signed());
        assert!(run_platform_signing(&["gpg".into()], &context).is_err());
    }
}
