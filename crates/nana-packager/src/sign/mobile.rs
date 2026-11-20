//! `android-apksigner` and `ohos-hapsigner`: sign an APK or HAP built
//! outside the packager (Gradle, DevEco/hvigor), through `nana-packager
//! sign`. Neither tool is host-bound.
//!
//! Android (`apksigner`, APK Signature Scheme v2+):
//! - `NANA_ANDROID_KEYSTORE`, `NANA_ANDROID_KEY_ALIAS`,
//!   `NANA_ANDROID_KEYSTORE_PASSWORD`, optional `NANA_ANDROID_KEY_PASSWORD`.
//!   Passwords reach apksigner as `env:` references, never on its command
//!   line.
//! - `NANA_APKSIGNER`, else `apksigner` on `PATH`, else the newest
//!   `build-tools` of `ANDROID_HOME` / `ANDROID_SDK_ROOT`.
//!
//! OpenHarmony (`hap-sign-tool.jar sign-app`, local keystore mode):
//! - `NANA_OHOS_HAP_SIGN_TOOL` (the jar), `NANA_OHOS_KEYSTORE`,
//!   `NANA_OHOS_KEYSTORE_PASSWORD`, `NANA_OHOS_KEY_ALIAS`,
//!   `NANA_OHOS_KEY_PASSWORD`, `NANA_OHOS_APP_CERT` (`.cer`),
//!   `NANA_OHOS_PROFILE` (`.p7b`), optional `NANA_OHOS_SIGN_ALG` (default
//!   `SHA256withECDSA`) and `NANA_JAVA`. hap-sign-tool takes its passwords
//!   on its command line only; they are masked in every report.

use std::path::{Path, PathBuf};

use nana_package::manifest::SignStatus;

use super::{PlatformSigner, SignContext, SignStage, ToolCommand, not_executed};
use crate::layout::TargetPlatform;

const APKSIGNER: &str = "NANA_APKSIGNER";
const KEYSTORE: &str = "NANA_ANDROID_KEYSTORE";
const KEY_ALIAS: &str = "NANA_ANDROID_KEY_ALIAS";
const KEYSTORE_PASSWORD: &str = "NANA_ANDROID_KEYSTORE_PASSWORD";
const KEY_PASSWORD: &str = "NANA_ANDROID_KEY_PASSWORD";

pub(super) struct ApkSigner;

impl ApkSigner {
    fn targets<'a>(context: &SignContext<'a>) -> Result<Vec<&'a Path>, SignStatus> {
        let files = context.artifacts_with(&["apk"]);
        if files.is_empty() {
            Err(not_executed("no .apk to sign"))
        } else {
            Ok(files)
        }
    }

    fn tool(context: &SignContext<'_>) -> Result<PathBuf, SignStatus> {
        if let Ok(path) = context.tool("apksigner", Some(APKSIGNER)) {
            return Ok(path);
        }
        // The newest build-tools of the SDK.
        let sdk = context
            .env
            .var("ANDROID_HOME")
            .or_else(|| context.env.var("ANDROID_SDK_ROOT"));
        let name = if cfg!(windows) {
            "apksigner.bat"
        } else {
            "apksigner"
        };
        sdk.and_then(|sdk| {
            let mut versions: Vec<PathBuf> = std::fs::read_dir(Path::new(&sdk).join("build-tools"))
                .ok()?
                .filter_map(|entry| entry.ok().map(|entry| entry.path().join(name)))
                .filter(|path| path.is_file())
                .collect();
            versions.sort();
            versions.pop()
        })
        .ok_or_else(|| {
            not_executed(format!(
                "`apksigner` is not on PATH (set {APKSIGNER} or ANDROID_HOME)"
            ))
        })
    }
}

impl PlatformSigner for ApkSigner {
    fn id(&self) -> &'static str {
        "android-apksigner"
    }

    fn platform(&self) -> Option<TargetPlatform> {
        None
    }

    fn stages(&self) -> &'static [SignStage] {
        &[SignStage::Artifacts]
    }

    fn sign(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            let targets = Self::targets(context)?;
            let keystore = context.require(KEYSTORE, "the signing keystore")?;
            let alias = context.require(KEY_ALIAS, "the key alias in the keystore")?;
            let password = context.require(KEYSTORE_PASSWORD, "the keystore password")?;
            let tool = Self::tool(context)?;
            for file in targets {
                let mut command = ToolCommand::new(&tool)
                    .args(["sign", "--ks"])
                    .arg(&keystore)
                    .arg("--ks-key-alias")
                    .arg(&alias)
                    .arg("--ks-pass")
                    .arg(format!("env:{KEYSTORE_PASSWORD}"))
                    .env(KEYSTORE_PASSWORD, password.clone());
                if let Some(key_password) = context.env.var(KEY_PASSWORD) {
                    command = command
                        .arg("--key-pass")
                        .arg(format!("env:{KEY_PASSWORD}"))
                        .env(KEY_PASSWORD, key_password);
                }
                context.run(&command.arg(file))?;
            }
            Ok(SignStatus::Signed)
        })();
        result.unwrap_or_else(|status| status)
    }

    fn verify(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            let tool = Self::tool(context)?;
            for file in Self::targets(context)? {
                context.run(
                    &ToolCommand::new(&tool)
                        .args(["verify", "--verbose"])
                        .arg(file),
                )?;
            }
            Ok(SignStatus::Verified)
        })();
        result.unwrap_or_else(|status| status)
    }
}

const HAP_TOOL: &str = "NANA_OHOS_HAP_SIGN_TOOL";
const OHOS_KEYSTORE: &str = "NANA_OHOS_KEYSTORE";
const OHOS_KEYSTORE_PASSWORD: &str = "NANA_OHOS_KEYSTORE_PASSWORD";
const OHOS_KEY_ALIAS: &str = "NANA_OHOS_KEY_ALIAS";
const OHOS_KEY_PASSWORD: &str = "NANA_OHOS_KEY_PASSWORD";
const OHOS_APP_CERT: &str = "NANA_OHOS_APP_CERT";
const OHOS_PROFILE: &str = "NANA_OHOS_PROFILE";
const OHOS_SIGN_ALG: &str = "NANA_OHOS_SIGN_ALG";
const JAVA: &str = "NANA_JAVA";

pub(super) struct HapSigner;

impl HapSigner {
    fn targets<'a>(context: &SignContext<'a>) -> Result<Vec<&'a Path>, SignStatus> {
        let files = context.artifacts_with(&["hap", "hsp"]);
        if files.is_empty() {
            Err(not_executed("no .hap to sign"))
        } else {
            Ok(files)
        }
    }

    fn java(context: &SignContext<'_>) -> Result<ToolCommand, SignStatus> {
        let jar = context.require(HAP_TOOL, "the path of hap-sign-tool.jar")?;
        let java = context.tool("java", Some(JAVA))?;
        Ok(ToolCommand::new(java).arg("-jar").arg(jar))
    }
}

impl PlatformSigner for HapSigner {
    fn id(&self) -> &'static str {
        "ohos-hapsigner"
    }

    fn platform(&self) -> Option<TargetPlatform> {
        None
    }

    fn stages(&self) -> &'static [SignStage] {
        &[SignStage::Artifacts]
    }

    fn sign(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            let targets = Self::targets(context)?;
            let keystore = context.require(OHOS_KEYSTORE, "the signing keystore (.p12)")?;
            let keystore_password =
                context.require(OHOS_KEYSTORE_PASSWORD, "the keystore password")?;
            let alias = context.require(OHOS_KEY_ALIAS, "the key alias")?;
            let key_password = context.require(OHOS_KEY_PASSWORD, "the key password")?;
            let cert = context.require(OHOS_APP_CERT, "the application certificate (.cer)")?;
            let profile = context.require(OHOS_PROFILE, "the signed provision profile (.p7b)")?;
            let algorithm = context
                .env
                .var(OHOS_SIGN_ALG)
                .unwrap_or_else(|| "SHA256withECDSA".into());
            for file in targets {
                let mut signed = file.as_os_str().to_owned();
                signed.push(".signed");
                let signed = PathBuf::from(signed);
                let command = Self::java(context)?
                    .args(["sign-app", "-mode", "localSign", "-keyAlias"])
                    .arg(&alias)
                    .arg("-keyPwd")
                    .secret_arg(&key_password)
                    .arg("-appCertFile")
                    .arg(&cert)
                    .arg("-profileFile")
                    .arg(&profile)
                    .arg("-signAlg")
                    .arg(&algorithm)
                    .arg("-keystoreFile")
                    .arg(&keystore)
                    .arg("-keystorePwd")
                    .secret_arg(&keystore_password)
                    .arg("-inFile")
                    .arg(file)
                    .arg("-outFile")
                    .arg(&signed);
                let ran = context.run(&command);
                if ran.is_err() {
                    let _ = std::fs::remove_file(&signed);
                }
                ran?;
                std::fs::rename(&signed, file).map_err(|error| SignStatus::Failed {
                    detail: format!(
                        "cannot replace {} with its signed copy: {error}",
                        file.display()
                    ),
                })?;
            }
            Ok(SignStatus::Signed)
        })();
        result.unwrap_or_else(|status| status)
    }

    fn verify(&self, _: SignStage, context: &SignContext<'_>) -> SignStatus {
        let result = (|| {
            let work = context
                .work_dir
                .map_or_else(std::env::temp_dir, Path::to_path_buf);
            for file in Self::targets(context)? {
                static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let stem = format!("nana-hap-verify-{}-{seq}", std::process::id());
                let chain = work.join(format!("{stem}.cer"));
                let profile = work.join(format!("{stem}.p7b"));
                let ran = context.run(
                    &Self::java(context)?
                        .args(["verify-app", "-inFile"])
                        .arg(file)
                        .arg("-outCertChain")
                        .arg(&chain)
                        .arg("-outProfile")
                        .arg(&profile),
                );
                let _ = std::fs::remove_file(&chain);
                let _ = std::fs::remove_file(&profile);
                ran?;
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
    fn apksigner_passes_passwords_by_environment_reference() {
        let env = env(&[
            (KEYSTORE, "/ci/release.jks"),
            (KEY_ALIAS, "release"),
            (KEYSTORE_PASSWORD, "storepass"),
            (KEY_PASSWORD, "keypass"),
        ]);
        let runner = Recorder::with_tools(&["apksigner"]);
        let apk = vec![PathBuf::from("/out/app-release.apk")];
        let mut cx = context(Host::Linux, &env, &runner, &apk);
        let results = run_stage(&["android-apksigner"], SignStage::Artifacts, &mut cx).unwrap();
        assert_eq!(results[0].status, SignStatus::Verified);
        let calls = runner.calls.borrow();
        let (_, args, envs, _) = &calls[0];
        let line = args.join(" ");
        assert_eq!(
            line,
            "sign --ks /ci/release.jks --ks-key-alias release --ks-pass env:NANA_ANDROID_KEYSTORE_PASSWORD \
             --key-pass env:NANA_ANDROID_KEY_PASSWORD /out/app-release.apk"
        );
        assert!(!line.contains("storepass") && !line.contains("keypass"));
        assert_eq!(envs, &[KEYSTORE_PASSWORD, KEY_PASSWORD]);
        assert_eq!(
            calls[1].1.join(" "),
            "verify --verbose /out/app-release.apk"
        );
    }

    #[test]
    fn hap_signing_masks_passwords_and_replaces_the_input() {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("nana-sign-hap-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let hap = dir.join("entry-default-unsigned.hap");
        std::fs::write(&hap, b"unsigned").unwrap();
        let env = env(&[
            (HAP_TOOL, "/sdk/hap-sign-tool.jar"),
            (OHOS_KEYSTORE, "/ci/ohos.p12"),
            (OHOS_KEYSTORE_PASSWORD, "kspw"),
            (OHOS_KEY_ALIAS, "release"),
            (OHOS_KEY_PASSWORD, "keypw"),
            (OHOS_APP_CERT, "/ci/app.cer"),
            (OHOS_PROFILE, "/ci/app.p7b"),
        ]);
        let runner = Recorder::with_tools(&["java"]);
        let artifacts = vec![hap.clone()];
        let mut cx = context(Host::Windows, &env, &runner, &artifacts);
        let results = run_stage(&["ohos-hapsigner"], SignStage::Artifacts, &mut cx).unwrap();
        assert_eq!(results[0].status, SignStatus::Verified, "{results:?}");
        let lines = runner.lines().join("\n");
        assert!(lines.contains("-jar /sdk/hap-sign-tool.jar sign-app -mode localSign"));
        assert!(lines.contains("-keyPwd *** "));
        assert!(lines.contains("-keystorePwd *** "));
        assert!(!lines.contains("kspw") && !lines.contains("keypw"));
        assert!(lines.contains("-signAlg SHA256withECDSA"));
        assert!(lines.contains("verify-app -inFile"));
        // The recorder "signed" into -outFile; the input was replaced.
        assert_eq!(std::fs::read(&hap).unwrap(), b"signed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_credentials_are_not_executed() {
        let env = env(&[]);
        let runner = Recorder::with_tools(&["apksigner", "java"]);
        let files = vec![PathBuf::from("/x.apk"), PathBuf::from("/x.hap")];
        let mut cx = context(Host::Linux, &env, &runner, &files);
        let results = run_stage(
            &["android-apksigner", "ohos-hapsigner"],
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
