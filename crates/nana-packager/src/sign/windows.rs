//! `windows-authenticode` and `msix`, both through `signtool`.
//!
//! - The certificate: `NANA_AUTHENTICODE_CERT_SHA1` (thumbprint of a
//!   certificate in the Windows store, the hardware-token / HSM route), or
//!   `NANA_AUTHENTICODE_PFX` plus `NANA_AUTHENTICODE_PFX_PASSWORD`. signtool
//!   takes a PFX password only on its command line; prefer the store.
//! - A timestamp server: `[platform.windows] timestamp_url` or
//!   `NANA_AUTHENTICODE_TIMESTAMP_URL`. Without one a signature dies with
//!   its certificate, so the adapter refuses to sign.
//! - `NANA_SIGNTOOL` (optional): signtool's path when it is not on `PATH`.
//!
//! In the application stage the main executable is always signed; other
//! PE files are signed only when they carry no valid signature yet, so a
//! vendor's signed DLL (`steam_api64.dll`) keeps its own.

use std::path::{Path, PathBuf};

use nana_package::manifest::SignStatus;

use super::{Host, PlatformSigner, SignContext, SignStage, ToolCommand, not_executed};
use crate::layout::TargetPlatform;

const CERT_SHA1: &str = "NANA_AUTHENTICODE_CERT_SHA1";
const PFX: &str = "NANA_AUTHENTICODE_PFX";
const PFX_PASSWORD: &str = "NANA_AUTHENTICODE_PFX_PASSWORD";
const TIMESTAMP: &str = "NANA_AUTHENTICODE_TIMESTAMP_URL";
const SIGNTOOL: &str = "NANA_SIGNTOOL";

fn is_pe(path: &Path) -> bool {
    use std::io::Read;
    let named = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["exe", "dll", "sys"]
                .iter()
                .any(|e| ext.eq_ignore_ascii_case(e))
        });
    let mut magic = [0u8; 2];
    named
        && std::fs::File::open(path)
            .and_then(|mut file| file.read_exact(&mut magic))
            .is_ok()
        && magic == *b"MZ"
}

/// Every PE file under `root`, sorted.
fn pe_files(root: &Path) -> Vec<PathBuf> {
    crate::util::walk_files(root)
        .unwrap_or_default()
        .into_iter()
        .filter(|path| is_pe(path))
        .collect()
}

/// `signtool sign` with the configured certificate and timestamp server.
fn sign_command(context: &SignContext<'_>) -> Result<ToolCommand, SignStatus> {
    context.on_host(Host::Windows, "signtool")?;
    let timestamp = context
        .options
        .windows_timestamp_url
        .clone()
        .or_else(|| context.env.var(TIMESTAMP))
        .ok_or_else(|| {
            not_executed(format!(
                "no timestamp server: set [platform.windows] timestamp_url or {TIMESTAMP}"
            ))
        })?;
    let signtool = context.tool("signtool", Some(SIGNTOOL))?;
    let command = ToolCommand::new(signtool)
        .args(["sign", "/fd", "sha256", "/td", "sha256", "/tr"])
        .arg(timestamp);
    if let Some(thumbprint) = context.env.var(CERT_SHA1) {
        return Ok(command.arg("/sha1").arg(thumbprint));
    }
    if let Some(pfx) = context.env.var(PFX) {
        let command = command.arg("/f").arg(pfx);
        return Ok(match context.env.var(PFX_PASSWORD) {
            Some(password) => command.arg("/p").secret_arg(password),
            None => command,
        });
    }
    Err(not_executed(format!(
        "no code signing certificate: set {CERT_SHA1} or {PFX}"
    )))
}

fn verify_command(context: &SignContext<'_>) -> Result<ToolCommand, SignStatus> {
    context.on_host(Host::Windows, "signtool")?;
    let signtool = context.tool("signtool", Some(SIGNTOOL))?;
    Ok(ToolCommand::new(signtool).args(["verify", "/pa", "/all"]))
}

fn sign_files(context: &SignContext<'_>, files: &[PathBuf]) -> Result<(), SignStatus> {
    if files.is_empty() {
        return Ok(());
    }
    let mut command = sign_command(context)?;
    for file in files {
        command = command.arg(file);
    }
    context.run(&command).map(drop)
}

fn verify_files(context: &SignContext<'_>, files: &[PathBuf]) -> Result<SignStatus, SignStatus> {
    for file in files {
        context.run(&verify_command(context)?.arg(file))?;
    }
    Ok(SignStatus::Verified)
}

pub(super) struct Authenticode;

impl Authenticode {
    /// What this stage signs.
    fn targets(stage: SignStage, context: &SignContext<'_>) -> Result<Vec<PathBuf>, SignStatus> {
        match stage {
            SignStage::Application => {
                let (Some(root), Some(main)) = (context.app_root, context.executable) else {
                    return Err(not_executed("no application to sign"));
                };
                let mut files = vec![main.to_path_buf()];
                for file in pe_files(root) {
                    if file == main {
                        continue;
                    }
                    let already = verify_command(context)?.arg("/q").arg(&file);
                    if !context.runner.run(&already).is_ok_and(|out| out.success) {
                        files.push(file);
                    }
                }
                Ok(files)
            }
            SignStage::Artifacts => {
                let files: Vec<PathBuf> = context
                    .artifacts_with(&["msi", "exe"])
                    .into_iter()
                    .map(Path::to_path_buf)
                    .collect();
                if files.is_empty() {
                    Err(not_executed("no .msi or installer .exe to sign"))
                } else {
                    Ok(files)
                }
            }
        }
    }
}

impl PlatformSigner for Authenticode {
    fn id(&self) -> &'static str {
        "windows-authenticode"
    }

    fn platform(&self) -> Option<TargetPlatform> {
        Some(TargetPlatform::Windows)
    }

    fn stages(&self) -> &'static [SignStage] {
        &[SignStage::Application, SignStage::Artifacts]
    }

    fn sign(&self, stage: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            // Credentials and host first: listing targets runs signtool.
            sign_command(context)?;
            let files = Self::targets(stage, context)?;
            sign_files(context, &files)?;
            Ok(SignStatus::Signed)
        })();
        result.unwrap_or_else(|status| status)
    }

    fn verify(&self, stage: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            verify_command(context)?;
            let files = match stage {
                SignStage::Application => {
                    let root = context
                        .app_root
                        .ok_or_else(|| not_executed("no application to verify"))?;
                    pe_files(root)
                }
                SignStage::Artifacts => Self::targets(stage, context)?,
            };
            verify_files(context, &files)
        })();
        result.unwrap_or_else(|status| status)
    }
}

pub(super) struct Msix;

impl Msix {
    fn targets(context: &SignContext<'_>) -> Result<Vec<PathBuf>, SignStatus> {
        let files: Vec<PathBuf> = context
            .artifacts_with(&["msix", "msixbundle", "appx", "appxbundle"])
            .into_iter()
            .map(Path::to_path_buf)
            .collect();
        if files.is_empty() {
            Err(not_executed(
                "no .msix artifact: the packager builds msi and nsis installers; pass an .msix \
                 built elsewhere to `nana-packager sign --adapter msix`",
            ))
        } else {
            Ok(files)
        }
    }
}

impl PlatformSigner for Msix {
    fn id(&self) -> &'static str {
        "msix"
    }

    fn platform(&self) -> Option<TargetPlatform> {
        Some(TargetPlatform::Windows)
    }

    fn stages(&self) -> &'static [SignStage] {
        &[SignStage::Artifacts]
    }

    fn sign(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            let files = Self::targets(context)?;
            sign_files(context, &files)?;
            Ok(SignStatus::Signed)
        })();
        result.unwrap_or_else(|status| status)
    }

    fn verify(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            let files = Self::targets(context)?;
            verify_files(context, &files)
        })();
        result.unwrap_or_else(|status| status)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{context, env};
    use super::super::tool::testing::{Recorder, fail};
    use super::super::{Host, SignStage, run_stage};
    use super::*;

    fn scratch() -> PathBuf {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("nana-sign-windows-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("runtime/bin")).unwrap();
        dir
    }

    #[test]
    fn authenticode_signs_the_exe_and_unsigned_libraries_only() {
        let root = scratch();
        let main = root.join("Fixture.exe");
        for path in [
            &main,
            &root.join("steam_api64.dll"),
            &root.join("runtime/bin/foo.dll"),
        ] {
            std::fs::write(path, b"MZ\x90\x00").unwrap();
        }
        std::fs::write(root.join("runtime/bin/notes.dll.txt"), b"MZ").unwrap();
        let env = env(&[(CERT_SHA1, "ABCDEF"), (TIMESTAMP, "http://ts.example")]);
        // The vendor DLL already verifies; ours does not.
        let runner = Recorder::with_tools(&["signtool"])
            .answer(
                &format!("/q {}", root.join("steam_api64.dll").display()),
                super::super::tool::testing::ok(""),
            )
            .answer("/q", fail("No signature found"));
        let mut cx = context(Host::Windows, &env, &runner, &[]);
        cx.app_root = Some(&root);
        cx.executable = Some(&main);
        let results =
            run_stage(&["windows-authenticode"], SignStage::Application, &mut cx).unwrap();
        assert_eq!(results[0].status, SignStatus::Verified, "{results:?}");
        let lines = runner.lines();
        let sign = lines.iter().find(|l| l.contains(" sign ")).unwrap();
        assert!(sign.contains("/fd sha256 /td sha256 /tr http://ts.example /sha1 ABCDEF"));
        assert!(sign.contains("Fixture.exe"));
        assert!(sign.contains("foo.dll"));
        assert!(!sign.contains("steam_api64.dll"), "{sign}");
        assert!(!sign.contains("notes.dll.txt"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pfx_password_never_reaches_a_description() {
        let env = env(&[
            (PFX, "C:/ci/cert.pfx"),
            (PFX_PASSWORD, "hunter2"),
            (TIMESTAMP, "http://ts"),
        ]);
        let runner = Recorder::with_tools(&["signtool"]).answer(" sign ", fail("bad pfx"));
        let msi = vec![PathBuf::from("C:/out/Fixture.msi")];
        let mut cx = context(Host::Windows, &env, &runner, &msi);
        let results = run_stage(&["windows-authenticode"], SignStage::Artifacts, &mut cx).unwrap();
        let SignStatus::Failed { detail } = &results[0].status else {
            panic!("{:?}", results[0].status);
        };
        assert!(!detail.contains("hunter2"), "{detail}");
        assert!(detail.contains("/p ***"), "{detail}");
    }

    #[test]
    fn no_timestamp_server_or_foreign_host_is_not_executed() {
        let env = env(&[(CERT_SHA1, "AB")]);
        let runner = Recorder::with_tools(&["signtool"]);
        let msi = vec![PathBuf::from("/out/F.msi")];
        let mut cx = context(Host::Windows, &env, &runner, &msi);
        let results = run_stage(&["windows-authenticode"], SignStage::Artifacts, &mut cx).unwrap();
        let SignStatus::NotExecuted { reason } = &results[0].status else {
            panic!("{:?}", results[0].status);
        };
        assert!(reason.contains("timestamp"), "{reason}");
        cx.host = Host::Macos;
        cx.options.windows_timestamp_url = Some("http://ts".into());
        let results = run_stage(
            &["windows-authenticode", "msix"],
            SignStage::Artifacts,
            &mut cx,
        )
        .unwrap();
        assert!(
            results
                .iter()
                .all(|r| matches!(r.status, SignStatus::NotExecuted { .. }))
        );
        assert!(runner.lines().is_empty());
    }
}
