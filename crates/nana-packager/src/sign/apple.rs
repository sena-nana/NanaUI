//! `macos-codesign` and `macos-notarize`.
//!
//! - `NANA_MACOS_SIGN_IDENTITY`: a `Developer ID Application: …` identity
//!   in the keychain, or `-` for an ad-hoc development signature (no
//!   hardened runtime, no timestamp, never notarizable).
//! - `NANA_MACOS_KEYCHAIN` (optional): the keychain to find it in.
//! - Notarization: `NANA_NOTARY_KEYCHAIN_PROFILE` (a `notarytool
//!   store-credentials` profile), or an App Store Connect API key:
//!   `NANA_NOTARY_API_KEY` (the `.p8` path), `NANA_NOTARY_API_KEY_ID`,
//!   `NANA_NOTARY_API_ISSUER`.

use std::path::{Path, PathBuf};

use nana_package::manifest::SignStatus;

use super::{PlatformSigner, SignContext, SignStage, ToolCommand, not_executed};
use crate::layout::TargetPlatform;

const IDENTITY: &str = "NANA_MACOS_SIGN_IDENTITY";
const KEYCHAIN: &str = "NANA_MACOS_KEYCHAIN";
const PROFILE: &str = "NANA_NOTARY_KEYCHAIN_PROFILE";
const API_KEY: &str = "NANA_NOTARY_API_KEY";
const API_KEY_ID: &str = "NANA_NOTARY_API_KEY_ID";
const API_ISSUER: &str = "NANA_NOTARY_API_ISSUER";

/// Directory extensions codesign treats as nested bundles.
const BUNDLE_EXTENSIONS: [&str; 7] = [
    "framework",
    "bundle",
    "app",
    "plugin",
    "appex",
    "xpc",
    "systemextension",
];

pub(super) struct Codesign;

fn is_macho(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && matches!(
            magic,
            [0xfe, 0xed, 0xfa, 0xce | 0xcf]
                | [0xce | 0xcf, 0xfa, 0xed, 0xfe]
                | [0xca, 0xfe, 0xba, 0xbe]
                | [0xbe, 0xba, 0xfe, 0xca]
        )
}

/// Code nested in a bundle, innermost first: Mach-O files and nested
/// bundles under `Frameworks`, `PlugIns`, `Helpers` and `MacOS` (the main
/// executable excluded: it is signed with the bundle). Symlinks are skipped;
/// what they point at is signed where it lives.
pub(super) fn nested_code(app: &Path, main: &Path) -> Vec<PathBuf> {
    fn collect(dir: &Path, main: &Path, out: &mut Vec<PathBuf>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                collect(&path, main, out);
                let bundle = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| BUNDLE_EXTENSIONS.contains(&ext));
                if bundle {
                    out.push(path);
                }
            } else if path != main && is_macho(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for dir in ["Frameworks", "PlugIns", "Helpers", "MacOS"] {
        collect(&app.join("Contents").join(dir), main, &mut out);
    }
    out
}

impl Codesign {
    fn base(context: &SignContext<'_>, codesign: &Path, identity: &str) -> ToolCommand {
        let mut command = ToolCommand::new(codesign)
            .args(["--force", "--sign"])
            .arg(identity);
        if let Some(keychain) = context.env.var(KEYCHAIN) {
            command = command.arg("--keychain").arg(keychain);
        }
        if identity == "-" {
            command.arg("--timestamp=none")
        } else {
            // Notarization requires both.
            command.args(["--timestamp", "--options", "runtime"])
        }
    }
}

impl PlatformSigner for Codesign {
    fn id(&self) -> &'static str {
        "macos-codesign"
    }

    fn platform(&self) -> Option<TargetPlatform> {
        Some(TargetPlatform::Macos)
    }

    fn stages(&self) -> &'static [SignStage] {
        &[SignStage::Application]
    }

    fn sign(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            context.on_host(super::Host::Macos, "codesign")?;
            let (Some(app), Some(main)) = (context.app_root, context.executable) else {
                return Err(not_executed("no application bundle to sign"));
            };
            let identity = context.require(
                IDENTITY,
                "a Developer ID Application identity, or `-` for an ad-hoc development signature",
            )?;
            let codesign = context.tool("codesign", None)?;
            for path in nested_code(app, main) {
                context.run(&Self::base(context, &codesign, &identity).arg(&path))?;
            }
            let mut bundle = Self::base(context, &codesign, &identity);
            if let Some(entitlements) = &context.options.macos_entitlements {
                bundle = bundle.arg("--entitlements").arg(entitlements);
            }
            context.run(&bundle.arg(app))?;
            Ok(SignStatus::Signed)
        })();
        result.unwrap_or_else(|status| status)
    }

    fn verify(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            context.on_host(super::Host::Macos, "codesign")?;
            let app = context
                .app_root
                .ok_or_else(|| not_executed("no application bundle to verify"))?;
            let codesign = context.tool("codesign", None)?;
            context.run(
                &ToolCommand::new(codesign)
                    .args(["--verify", "--strict", "--deep", "--verbose=2"])
                    .arg(app),
            )?;
            Ok(SignStatus::Verified)
        })();
        result.unwrap_or_else(|status| status)
    }
}

pub(super) struct Notarize;

/// The notarytool credential arguments, or why there are none.
fn notary_credentials(context: &SignContext<'_>) -> Result<Vec<String>, SignStatus> {
    if let Some(profile) = context.env.var(PROFILE) {
        return Ok(vec!["--keychain-profile".into(), profile]);
    }
    match (
        context.env.var(API_KEY),
        context.env.var(API_KEY_ID),
        context.env.var(API_ISSUER),
    ) {
        (Some(key), Some(id), Some(issuer)) => Ok(vec![
            "--key".into(),
            key,
            "--key-id".into(),
            id,
            "--issuer".into(),
            issuer,
        ]),
        _ => Err(not_executed(format!(
            "no notarization credentials: set {PROFILE}, or {API_KEY}, {API_KEY_ID} and \
             {API_ISSUER}"
        ))),
    }
}

impl Notarize {
    /// What gets submitted and stapled: each `.dmg` artifact, else the app
    /// itself (submitted as a zip).
    fn targets<'a>(context: &SignContext<'a>) -> Result<Vec<&'a Path>, SignStatus> {
        let dmgs = context.artifacts_with(&["dmg"]);
        if !dmgs.is_empty() {
            return Ok(dmgs);
        }
        context
            .app_root
            .map(|app| vec![app])
            .ok_or_else(|| not_executed("nothing to notarize: no .dmg and no application"))
    }

    fn submit(
        context: &SignContext<'_>,
        xcrun: &Path,
        credentials: &[String],
        file: &Path,
    ) -> Result<(), SignStatus> {
        let output = context.run(
            &ToolCommand::new(xcrun)
                .args(["notarytool", "submit"])
                .arg(file)
                .args(["--wait", "--output-format", "json"])
                .args(credentials),
        )?;
        let reply: serde_json::Value =
            serde_json::from_str(output.stdout.trim()).map_err(|error| SignStatus::Failed {
                detail: format!("notarytool printed no JSON result ({error})"),
            })?;
        let status = reply["status"].as_str().unwrap_or("unknown");
        if status != "Accepted" {
            let id = reply["id"].as_str().unwrap_or("?");
            return Err(SignStatus::Failed {
                detail: format!(
                    "notarization of {} ended `{status}` (submission {id}; \
                     `xcrun notarytool log {id}` explains why)",
                    file.display()
                ),
            });
        }
        Ok(())
    }
}

impl PlatformSigner for Notarize {
    fn id(&self) -> &'static str {
        "macos-notarize"
    }

    fn platform(&self) -> Option<TargetPlatform> {
        Some(TargetPlatform::Macos)
    }

    fn stages(&self) -> &'static [SignStage] {
        &[SignStage::Artifacts]
    }

    fn sign(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            context.on_host(super::Host::Macos, "notarytool")?;
            if context.app_root.is_some() && !context.earlier_signed("macos-codesign") {
                return Err(not_executed(
                    "notarization needs a Developer ID signature: macos-codesign did not sign \
                     this build",
                ));
            }
            let identity = context.require(IDENTITY, "the identity the build was signed with")?;
            if identity == "-" {
                return Err(not_executed(
                    "an ad-hoc signature cannot be notarized; sign with a Developer ID identity",
                ));
            }
            let credentials = notary_credentials(context)?;
            let xcrun = context.tool("xcrun", None)?;
            let targets = Self::targets(context)?;
            for target in targets {
                let is_dmg = target.extension().is_some_and(|ext| ext == "dmg");
                let submitted = if is_dmg {
                    // The disk image carries its own signature.
                    let codesign = context.tool("codesign", None)?;
                    context.run(
                        &ToolCommand::new(codesign)
                            .args(["--force", "--timestamp", "--sign"])
                            .arg(&identity)
                            .arg(target),
                    )?;
                    target.to_path_buf()
                } else {
                    let ditto = context.tool("ditto", None)?;
                    let work = match context.work_dir {
                        Some(dir) => dir.to_path_buf(),
                        None => std::env::temp_dir(),
                    };
                    let name = target
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("App.app");
                    let zip = work.join(format!("{name}.notarize.zip"));
                    context.run(
                        &ToolCommand::new(ditto)
                            .args(["-c", "-k", "--keepParent"])
                            .arg(target)
                            .arg(&zip),
                    )?;
                    zip
                };
                let result = Self::submit(context, &xcrun, &credentials, &submitted);
                if !is_dmg {
                    let _ = std::fs::remove_file(&submitted);
                }
                result?;
                context.run(
                    &ToolCommand::new(&xcrun)
                        .args(["stapler", "staple"])
                        .arg(target),
                )?;
            }
            Ok(SignStatus::Signed)
        })();
        result.unwrap_or_else(|status| status)
    }

    fn verify(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            context.on_host(super::Host::Macos, "spctl")?;
            let xcrun = context.tool("xcrun", None)?;
            let spctl = context.tool("spctl", None)?;
            for target in Self::targets(context)? {
                let is_dmg = target.extension().is_some_and(|ext| ext == "dmg");
                // A disk image carries its own stapled ticket. An app shipped
                // inside a stapled image has none of its own, so it is judged
                // by Gatekeeper alone, which looks the ticket up.
                if is_dmg {
                    context.run(
                        &ToolCommand::new(&xcrun)
                            .args(["stapler", "validate"])
                            .arg(target),
                    )?;
                }
                let assess = ToolCommand::new(&spctl).args(["--assess", "--verbose"]);
                let assess = if is_dmg {
                    assess.args(["--type", "open", "--context", "context:primary-signature"])
                } else {
                    assess.args(["--type", "execute"])
                };
                context.run(&assess.arg(target))?;
            }
            Ok(SignStatus::Verified)
        })();
        result.unwrap_or_else(|status| status)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{context, env};
    use super::super::tool::testing::{Recorder, fail, ok};
    use super::super::{Host, SignStage, run_stage};
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nana-sign-apple-{name}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A bundle with a framework (holding a Mach-O), a helper and a stray
    /// non-code file.
    fn bundle(dir: &Path) -> (PathBuf, PathBuf) {
        let app = dir.join("Fixture.app");
        let macho = [0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0];
        let main = app.join("Contents/MacOS/Fixture");
        for (path, bytes) in [
            (main.clone(), &macho[..]),
            (
                app.join("Contents/Frameworks/Foo.framework/Versions/A/Foo"),
                &macho[..],
            ),
            (app.join("Contents/Helpers/ffmpeg"), &macho[..]),
            (app.join("Contents/Helpers/README.txt"), b"text"),
        ] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        (app, main)
    }

    #[test]
    fn codesign_signs_nested_code_first_with_hardened_runtime() {
        let dir = scratch("codesign");
        let (app, main) = bundle(&dir);
        let env = env(&[(IDENTITY, "Developer ID Application: Nana (TEAM)")]);
        let runner = Recorder::with_tools(&["codesign"]);
        let mut cx = context(Host::Macos, &env, &runner, &[]);
        cx.app_root = Some(&app);
        cx.executable = Some(&main);
        cx.options.macos_entitlements = Some("/e.plist".into());
        let results = run_stage(&["macos-codesign"], SignStage::Application, &mut cx).unwrap();
        assert_eq!(results[0].status, SignStatus::Verified);
        let lines = runner.lines();
        let framework_binary = lines
            .iter()
            .position(|l| l.ends_with("Versions/A/Foo"))
            .unwrap();
        let framework = lines
            .iter()
            .position(|l| l.ends_with("Foo.framework"))
            .unwrap();
        let helper = lines.iter().position(|l| l.ends_with("ffmpeg")).unwrap();
        let bundle_line = lines
            .iter()
            .position(|l| l.ends_with("Fixture.app") && l.contains("--sign"))
            .unwrap();
        assert!(framework_binary < framework && framework < bundle_line);
        assert!(helper < bundle_line);
        assert!(!lines.iter().any(|l| l.ends_with("README.txt")));
        // The main executable is signed as the bundle, not on its own.
        assert!(!lines.iter().any(|l| l.ends_with("MacOS/Fixture")));
        assert!(lines[bundle_line].contains("--options runtime"));
        assert!(lines[bundle_line].contains("--timestamp"));
        assert!(lines[bundle_line].contains("--entitlements /e.plist"));
        assert!(lines.last().unwrap().contains("--verify --strict --deep"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notarize_submits_dmgs_and_staples_them() {
        let env = env(&[(IDENTITY, "Developer ID Application: X"), (PROFILE, "ci")]);
        let accepted = ok(r#"{"id":"abc","status":"Accepted"}"#);
        let runner = Recorder::with_tools(&["xcrun", "codesign", "spctl"])
            .answer("notarytool submit", accepted);
        let dmg = vec![PathBuf::from("/out/Fixture-1.0.dmg")];
        let mut cx = context(Host::Macos, &env, &runner, &dmg);
        let results = run_stage(&["macos-notarize"], SignStage::Artifacts, &mut cx).unwrap();
        assert_eq!(results[0].status, SignStatus::Verified);
        let lines = runner.lines().join("\n");
        assert!(
            lines.contains("--timestamp --sign Developer ID Application: X /out/Fixture-1.0.dmg")
        );
        assert!(lines.contains("notarytool submit /out/Fixture-1.0.dmg --wait --output-format json --keychain-profile ci"));
        assert!(lines.contains("stapler staple /out/Fixture-1.0.dmg"));
        assert!(lines.contains("stapler validate /out/Fixture-1.0.dmg"));
        assert!(lines.contains("--type open"));
    }

    #[test]
    fn an_app_is_judged_by_gatekeeper_not_by_its_own_ticket() {
        let env = env(&[]);
        let runner = Recorder::with_tools(&["xcrun", "spctl"])
            .answer("spctl", fail("rejected source=Unnotarized Developer ID"));
        let app = PathBuf::from("/out/F.app");
        let mut cx = context(Host::Macos, &env, &runner, &[]);
        cx.app_root = Some(&app);
        let status = Notarize.verify(SignStage::Artifacts, &cx);
        let SignStatus::Failed { detail } = &status else {
            panic!("{status:?}");
        };
        assert!(detail.contains("Unnotarized"), "{detail}");
        let lines = runner.lines().join("\n");
        assert!(!lines.contains("stapler"), "{lines}");
        assert!(lines.contains("--assess --verbose --type execute /out/F.app"));
    }

    #[test]
    fn rejected_notarization_fails_with_the_submission_id() {
        let env = env(&[
            (IDENTITY, "Developer ID Application: X"),
            (API_KEY, "/k.p8"),
            (API_KEY_ID, "KID"),
            (API_ISSUER, "ISS"),
        ]);
        let runner = Recorder::with_tools(&["xcrun", "codesign", "spctl"])
            .answer("notarytool", ok(r#"{"id":"s-1","status":"Invalid"}"#));
        let dmg = vec![PathBuf::from("/out/F.dmg")];
        let mut cx = context(Host::Macos, &env, &runner, &dmg);
        let results = run_stage(&["macos-notarize"], SignStage::Artifacts, &mut cx).unwrap();
        let SignStatus::Failed { detail } = &results[0].status else {
            panic!("{:?}", results[0].status);
        };
        assert!(
            detail.contains("Invalid") && detail.contains("s-1"),
            "{detail}"
        );
        assert!(
            runner
                .lines()
                .join("\n")
                .contains("--key /k.p8 --key-id KID --issuer ISS")
        );
        assert!(!runner.lines().join("\n").contains("stapler staple"));

        // A tool failure is a failure, with the tool's own message.
        let runner = Recorder::with_tools(&["xcrun", "codesign", "spctl"])
            .answer("codesign", fail("no identity found"));
        let mut cx = context(Host::Macos, &env, &runner, &dmg);
        let results = run_stage(&["macos-notarize"], SignStage::Artifacts, &mut cx).unwrap();
        let SignStatus::Failed { detail } = &results[0].status else {
            panic!("{:?}", results[0].status);
        };
        assert!(detail.contains("no identity found"), "{detail}");
    }

    #[test]
    fn notarize_needs_a_developer_id_signature() {
        let env = env(&[(IDENTITY, "-"), (PROFILE, "ci")]);
        let runner = Recorder::with_tools(&["xcrun", "codesign", "ditto", "spctl"]);
        let app = PathBuf::from("/out/F.app");
        let mut cx = context(Host::Macos, &env, &runner, &[]);
        cx.app_root = Some(&app);
        // codesign did not sign in this run.
        let results = run_stage(&["macos-notarize"], SignStage::Artifacts, &mut cx).unwrap();
        assert!(matches!(results[0].status, SignStatus::NotExecuted { .. }));
        cx.earlier.push(nana_package::manifest::PlatformSigning {
            adapter: "macos-codesign".into(),
            status: SignStatus::Verified,
        });
        let results = run_stage(&["macos-notarize"], SignStage::Artifacts, &mut cx).unwrap();
        let SignStatus::NotExecuted { reason } = &results[0].status else {
            panic!("{:?}", results[0].status);
        };
        assert!(reason.contains("ad-hoc"), "{reason}");
        assert!(runner.lines().is_empty());
    }

    /// The real tool on a real Mach-O: an ad-hoc signature of a bundle
    /// around this test binary, verified by `codesign --verify`.
    #[test]
    fn real_codesign_ad_hoc_signs_and_verifies_on_macos() {
        if Host::current() != Host::Macos {
            // Not executed on this host: the adapter says so itself.
            let env = env(&[(IDENTITY, "-")]);
            let runner = Recorder::with_tools(&["codesign"]);
            let app = PathBuf::from("/x.app");
            let mut cx = context(Host::current(), &env, &runner, &[]);
            cx.app_root = Some(&app);
            cx.executable = Some(&app);
            let results = run_stage(&["macos-codesign"], SignStage::Application, &mut cx).unwrap();
            assert!(matches!(results[0].status, SignStatus::NotExecuted { .. }));
            return;
        }
        let dir = scratch("real");
        let app = dir.join("Real.app");
        let main = app.join("Contents/MacOS/Real");
        std::fs::create_dir_all(main.parent().unwrap()).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), &main).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            crate::macos::info_plist(&crate::macos::PlistInput {
                application: &crate::config::ApplicationConfig {
                    id: "dev.nana.sign-test".into(),
                    name: "Real".into(),
                    version: "1.0.0".into(),
                    vendor: None,
                },
                macos: &crate::config::MacosConfig::default(),
                executable: "Real",
                build_id: None,
                extra: None,
            })
            .unwrap(),
        )
        .unwrap();
        let env = env(&[(IDENTITY, "-")]);
        let runner = super::super::SystemRunner;
        let mut cx = SignContext {
            app_root: Some(&app),
            executable: Some(&main),
            artifacts: &[],
            options: Default::default(),
            work_dir: None,
            env: &env,
            runner: &runner,
            host: Host::Macos,
            earlier: Vec::new(),
        };
        let results = run_stage(&["macos-codesign"], SignStage::Application, &mut cx).unwrap();
        assert_eq!(results[0].status, SignStatus::Verified, "{results:?}");
        // Tampering with the sealed bundle breaks verification.
        std::fs::write(app.join("Contents/Resources-extra"), b"x").unwrap();
        std::fs::write(app.join("Contents/Info.plist"), b"<plist/>").unwrap();
        let status = Codesign.verify(SignStage::Application, &cx);
        assert!(matches!(status, SignStatus::Failed { .. }), "{status:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
