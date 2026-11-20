//! `linux-package`: detached, ASCII-armored GPG signatures (`<file>.asc`)
//! for the Linux installer artifacts (`.deb`, `.AppImage`, `.flatpak`).
//! Linux has no in-place executable signature; distributions and users
//! verify the artifact.
//!
//! - `NANA_GPG_KEY_ID`: the signing key's fingerprint.
//! - `NANA_GPG_PASSPHRASE` (optional): fed to gpg on stdin
//!   (`--pinentry-mode loopback --passphrase-fd 0`), never on its command
//!   line; without it, gpg-agent supplies the key.
//! - `NANA_GPG_HOMEDIR` (optional), `NANA_GPG` (optional binary path).
//!
//! gpg runs on every host, so this adapter is not host-bound.

use std::path::{Path, PathBuf};

use nana_package::manifest::SignStatus;

use super::{PlatformSigner, SignContext, SignStage, ToolCommand, not_executed};
use crate::layout::TargetPlatform;

const KEY_ID: &str = "NANA_GPG_KEY_ID";
const PASSPHRASE: &str = "NANA_GPG_PASSPHRASE";
const HOMEDIR: &str = "NANA_GPG_HOMEDIR";
const GPG: &str = "NANA_GPG";

pub(super) struct LinuxPackage;

fn signature_of(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".asc");
    PathBuf::from(name)
}

impl LinuxPackage {
    fn targets<'a>(context: &SignContext<'a>) -> Result<Vec<&'a Path>, SignStatus> {
        let files = context.artifacts_with(&["deb", "appimage", "flatpak", "rpm"]);
        if files.is_empty() {
            Err(not_executed(
                "no Linux installer artifact (.deb, .AppImage, .flatpak) to sign",
            ))
        } else {
            Ok(files)
        }
    }

    fn gpg(context: &SignContext<'_>) -> Result<ToolCommand, SignStatus> {
        let gpg = context.tool("gpg", Some(GPG))?;
        let mut command = ToolCommand::new(gpg).arg("--batch");
        if let Some(home) = context.env.var(HOMEDIR) {
            command = command.arg("--homedir").arg(home);
        }
        Ok(command)
    }
}

impl PlatformSigner for LinuxPackage {
    fn id(&self) -> &'static str {
        "linux-package"
    }

    fn platform(&self) -> Option<TargetPlatform> {
        Some(TargetPlatform::Linux)
    }

    fn stages(&self) -> &'static [SignStage] {
        &[SignStage::Artifacts]
    }

    fn sign(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            let targets = Self::targets(context)?;
            let key = context.require(KEY_ID, "the fingerprint of the GPG signing key")?;
            for file in targets {
                let mut command = Self::gpg(context)?
                    .args(["--yes", "--local-user"])
                    .arg(&key)
                    .args(["--armor", "--detach-sign", "--output"])
                    .arg(signature_of(file));
                if let Some(passphrase) = context.env.var(PASSPHRASE) {
                    command = command
                        .args(["--pinentry-mode", "loopback", "--passphrase-fd", "0"])
                        .stdin(format!("{passphrase}\n").into_bytes());
                }
                context.run(&command.arg(file))?;
            }
            Ok(SignStatus::Signed)
        })();
        result.unwrap_or_else(|status| status)
    }

    fn verify(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            for file in Self::targets(context)? {
                context.run(
                    &Self::gpg(context)?
                        .arg("--verify")
                        .arg(signature_of(file))
                        .arg(file),
                )?;
            }
            Ok(SignStatus::Verified)
        })();
        result.unwrap_or_else(|status| status)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{context, env};
    use super::super::tool::testing::Recorder;
    use super::super::{Host, SignStage, run_stage};
    use super::*;

    #[test]
    fn every_linux_artifact_gets_a_detached_signature() {
        let env = env(&[(KEY_ID, "FPR"), (PASSPHRASE, "secret words")]);
        let runner = Recorder::with_tools(&["gpg"]);
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("nana-sign-gpg-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let artifacts = vec![
            dir.join("fixture_1.0_amd64.deb"),
            dir.join("Fixture-x86_64.AppImage"),
            dir.join("notes.txt"),
        ];
        // Any host: gpg is not host-bound.
        let mut cx = context(Host::Macos, &env, &runner, &artifacts);
        let results = run_stage(&["linux-package"], SignStage::Artifacts, &mut cx).unwrap();
        assert_eq!(results[0].status, SignStatus::Verified);
        let calls = runner.calls.borrow();
        let signs: Vec<_> = calls
            .iter()
            .filter(|(_, args, _, _)| args.contains(&"--detach-sign".to_owned()))
            .collect();
        assert_eq!(signs.len(), 2);
        for (_, args, _, stdin) in &signs {
            assert!(*stdin, "the passphrase goes through stdin");
            assert!(!args.iter().any(|a| a.contains("secret")));
            assert!(args.contains(&"loopback".to_owned()));
        }
        assert!(dir.join("fixture_1.0_amd64.deb.asc").is_file());
        assert_eq!(
            calls
                .iter()
                .filter(|(_, args, _, _)| args.contains(&"--verify".to_owned()))
                .count(),
            2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_to_sign_is_not_executed() {
        let env = env(&[(KEY_ID, "FPR")]);
        let runner = Recorder::with_tools(&["gpg"]);
        let mut cx = context(Host::Linux, &env, &runner, &[]);
        let results = run_stage(&["linux-package"], SignStage::Artifacts, &mut cx).unwrap();
        assert!(matches!(results[0].status, SignStatus::NotExecuted { .. }));
    }
}
