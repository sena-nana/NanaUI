//! Platform signing adapters (Issue #226 §9, Issue #274 §1).
//!
//! Nana does not reimplement codesign, notarization, Authenticode, GPG,
//! apksigner or the HarmonyOS hap-sign-tool: each adapter wraps the platform
//! tool and reports, per run, what actually happened. A tool that cannot run
//! here — a foreign host, a missing tool, missing credentials, nothing to
//! sign — is reported `NotExecuted` with the reason, never as signed: an
//! unsigned development build and a signed distribution build are never
//! confused.
//!
//! Signing runs in two stages:
//!
//! 1. [`SignStage::Application`] signs the application tree in place
//!    (`macos-codesign`, `windows-authenticode`), after the package manifest
//!    is written: codesign seals the bundle with the manifest inside, and
//!    both rewrite executables, so the manifest's hashes of signable files
//!    are the pre-signing ones (`pre_platform_signing`).
//! 2. [`SignStage::Artifacts`] signs what the installer backends built
//!    (`macos-notarize`, `windows-authenticode`, `msix`, `linux-package`),
//!    and, through `nana-packager sign`, artifacts built elsewhere
//!    (`android-apksigner`, `ohos-hapsigner`).
//!
//! Credentials (certificates, keystores, notarization keys, passphrases)
//! come from the CI environment — see each adapter's `NANA_*` variables in
//! `docs/reference/packaging.md` — never from `nana-package.toml` or the
//! package manifest.

mod apple;
mod gpg;
mod mobile;
pub mod tool;
mod windows;

use std::path::{Path, PathBuf};

use nana_package::manifest::{PlatformSigning, SignStatus};

use crate::layout::TargetPlatform;
pub use tool::{Environment, Host, ProcessEnv, SystemRunner, ToolCommand, ToolOutput, ToolRunner};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignStage {
    /// The application tree, in place.
    Application,
    /// Installer and package files.
    Artifacts,
}

impl SignStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Application => "application",
            Self::Artifacts => "artifacts",
        }
    }
}

/// Non-secret signing settings from `nana-package.toml`.
#[derive(Debug, Clone, Default)]
pub struct SignOptions {
    /// `[platform.macos] entitlements`, resolved.
    pub macos_entitlements: Option<PathBuf>,
    /// `[platform.windows] timestamp_url` (or `NANA_AUTHENTICODE_TIMESTAMP_URL`).
    pub windows_timestamp_url: Option<String>,
}

pub struct SignContext<'a> {
    /// The application root (`App/` or `Name.app`), when there is one.
    pub app_root: Option<&'a Path>,
    pub executable: Option<&'a Path>,
    /// Installer or package files for the artifact stage.
    pub artifacts: &'a [PathBuf],
    pub options: SignOptions,
    /// Scratch space (notarization archives); a temporary directory when
    /// absent.
    pub work_dir: Option<&'a Path>,
    pub env: &'a dyn Environment,
    pub runner: &'a dyn ToolRunner,
    pub host: Host,
    /// What earlier adapters of this run reported.
    pub earlier: Vec<PlatformSigning>,
}

impl<'a> SignContext<'a> {
    /// The application at `app_root`, with the real tools and environment.
    pub fn application(app_root: &'a Path, executable: &'a Path) -> Self {
        Self {
            app_root: Some(app_root),
            executable: Some(executable),
            artifacts: &[],
            options: SignOptions::default(),
            work_dir: None,
            env: &ProcessEnv,
            runner: &SystemRunner,
            host: Host::current(),
            earlier: Vec::new(),
        }
    }

    /// Files to sign, with the real tools and environment.
    pub fn artifacts(artifacts: &'a [PathBuf]) -> Self {
        Self {
            app_root: None,
            executable: None,
            artifacts,
            options: SignOptions::default(),
            work_dir: None,
            env: &ProcessEnv,
            runner: &SystemRunner,
            host: Host::current(),
            earlier: Vec::new(),
        }
    }

    fn earlier_signed(&self, adapter: &str) -> bool {
        self.earlier
            .iter()
            .any(|result| result.adapter == adapter && result.status.is_signed())
    }

    /// Artifacts whose extension (case-insensitive) is one of `extensions`.
    fn artifacts_with(&self, extensions: &[&str]) -> Vec<&'a Path> {
        self.artifacts
            .iter()
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| extensions.iter().any(|e| ext.eq_ignore_ascii_case(e)))
            })
            .map(PathBuf::as_path)
            .collect()
    }

    /// `name` from the environment, or the reason it is needed.
    fn require(&self, name: &str, purpose: &str) -> Result<String, SignStatus> {
        self.env
            .var(name)
            .ok_or_else(|| not_executed(format!("{name} is not set ({purpose})")))
    }

    /// The tool `program`: `override_var` when set, else on `PATH`.
    fn tool(&self, program: &str, override_var: Option<&str>) -> Result<PathBuf, SignStatus> {
        if let Some(var) = override_var
            && let Some(path) = self.env.var(var)
        {
            return Ok(PathBuf::from(path));
        }
        self.runner.find(program).ok_or_else(|| {
            not_executed(match override_var {
                Some(var) => format!("`{program}` is not on PATH (or set {var})"),
                None => format!("`{program}` is not on PATH"),
            })
        })
    }

    /// Fail unless running on `host`.
    fn on_host(&self, host: Host, tool: &str) -> Result<(), SignStatus> {
        if self.host == host {
            Ok(())
        } else {
            Err(not_executed(format!(
                "{tool} runs on a {} host; this host is {}",
                host.as_str(),
                self.host.as_str()
            )))
        }
    }

    /// Run `command`; a failure becomes `SignStatus::Failed`.
    fn run(&self, command: &ToolCommand) -> Result<ToolOutput, SignStatus> {
        let result = self.runner.run(command);
        match result {
            Ok(output) if output.success => Ok(output),
            other => Err(SignStatus::Failed {
                detail: tool::failure(command, &other),
            }),
        }
    }
}

pub(crate) fn not_executed(reason: impl Into<String>) -> SignStatus {
    SignStatus::NotExecuted {
        reason: reason.into(),
    }
}

pub trait PlatformSigner {
    fn id(&self) -> &'static str;
    /// The packaging target this adapter serves; `None` for artifacts the
    /// packager does not build (APK, HAP), signed through `nana-packager
    /// sign` only.
    fn platform(&self) -> Option<TargetPlatform>;
    fn stages(&self) -> &'static [SignStage];
    /// Sign; `Signed` on success.
    fn sign(&self, stage: SignStage, context: &SignContext<'_>) -> SignStatus;
    /// Check with the platform tool; `Verified` when it accepts.
    fn verify(&self, stage: SignStage, context: &SignContext<'_>) -> SignStatus;

    fn applies_to(&self, platform: TargetPlatform) -> bool {
        self.platform() == Some(platform)
    }
}

pub const KNOWN_ADAPTERS: [&str; 7] = [
    "macos-codesign",
    "macos-notarize",
    "windows-authenticode",
    "msix",
    "linux-package",
    "android-apksigner",
    "ohos-hapsigner",
];

pub fn adapter(id: &str) -> Option<Box<dyn PlatformSigner>> {
    Some(match id {
        "macos-codesign" => Box::new(apple::Codesign),
        "macos-notarize" => Box::new(apple::Notarize),
        "windows-authenticode" => Box::new(windows::Authenticode),
        "msix" => Box::new(windows::Msix),
        "linux-package" => Box::new(gpg::LinuxPackage),
        "android-apksigner" => Box::new(mobile::ApkSigner),
        "ohos-hapsigner" => Box::new(mobile::HapSigner),
        _ => return None,
    })
}

/// The configured adapters that apply to `platform`, rejecting unknown ids
/// and adapters for artifacts the packager does not build.
pub fn configured(configured: &[String], platform: TargetPlatform) -> Result<Vec<&str>, String> {
    let mut ids = Vec::new();
    for id in configured {
        let signer = adapter(id).ok_or_else(|| {
            format!(
                "unknown signing adapter `{id}` (known: {})",
                KNOWN_ADAPTERS.join(", ")
            )
        })?;
        if signer.platform().is_none() {
            return Err(format!(
                "signing adapter `{id}` signs artifacts the packager does not build; \
                 run `nana-packager sign --adapter {id} FILE` on them instead"
            ));
        }
        if signer.applies_to(platform) {
            ids.push(id.as_str());
        }
    }
    Ok(ids)
}

/// Run `ids` for `stage`: sign, then verify what was signed. Adapters that
/// do not take part in `stage` are skipped.
pub fn run_stage(
    ids: &[&str],
    stage: SignStage,
    context: &mut SignContext<'_>,
) -> Result<Vec<PlatformSigning>, String> {
    let mut results = Vec::new();
    for id in ids {
        let signer = adapter(id).ok_or_else(|| format!("unknown signing adapter `{id}`"))?;
        if !signer.stages().contains(&stage) {
            continue;
        }
        let status = match signer.sign(stage, context) {
            SignStatus::Signed => signer.verify(stage, context),
            other => other,
        };
        let result = PlatformSigning {
            adapter: (*id).to_owned(),
            status,
        };
        context.earlier.push(result.clone());
        results.push(result);
    }
    Ok(results)
}

/// Run the configured adapters that apply to `platform` for `stage`.
pub fn run_platform_signing(
    configured: &[String],
    platform: TargetPlatform,
    stage: SignStage,
    context: &mut SignContext<'_>,
) -> Result<Vec<PlatformSigning>, String> {
    let ids = self::configured(configured, platform)?;
    run_stage(&ids, stage, context)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::tool::testing::Recorder;
    use super::*;
    use std::collections::BTreeMap;

    pub(crate) fn env(vars: &[(&str, &str)]) -> BTreeMap<String, String> {
        vars.iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    pub(crate) fn context<'a>(
        host: Host,
        env: &'a BTreeMap<String, String>,
        runner: &'a Recorder,
        artifacts: &'a [PathBuf],
    ) -> SignContext<'a> {
        SignContext {
            app_root: None,
            executable: None,
            artifacts,
            options: SignOptions::default(),
            work_dir: None,
            env,
            runner,
            host,
            earlier: Vec::new(),
        }
    }

    #[test]
    fn foreign_hosts_and_missing_credentials_report_not_executed() {
        let env = env(&[]);
        let runner = Recorder::with_tools(&["codesign", "signtool", "gpg"]);
        let app = PathBuf::from("/x.app");
        let mut context = context(Host::Linux, &env, &runner, &[]);
        context.app_root = Some(&app);
        context.executable = Some(&app);
        let results = run_platform_signing(
            &["macos-codesign".into(), "windows-authenticode".into()],
            TargetPlatform::Macos,
            SignStage::Application,
            &mut context,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        let SignStatus::NotExecuted { reason } = &results[0].status else {
            panic!("{:?}", results[0].status);
        };
        assert!(reason.contains("macos host"), "{reason}");
        assert!(runner.lines().is_empty());

        context.host = Host::Macos;
        let results = run_platform_signing(
            &["macos-codesign".into()],
            TargetPlatform::Macos,
            SignStage::Application,
            &mut context,
        )
        .unwrap();
        let SignStatus::NotExecuted { reason } = &results[0].status else {
            panic!("{:?}", results[0].status);
        };
        assert!(reason.contains("NANA_MACOS_SIGN_IDENTITY"), "{reason}");
        assert!(!results[0].status.is_signed());
    }

    #[test]
    fn unknown_and_mobile_adapters_are_rejected_in_config() {
        assert!(configured(&["gpg".into()], TargetPlatform::Linux).is_err());
        let error = configured(&["android-apksigner".into()], TargetPlatform::Linux).unwrap_err();
        assert!(error.contains("nana-packager sign"), "{error}");
        assert_eq!(
            configured(
                &["linux-package".into(), "msix".into()],
                TargetPlatform::Linux
            )
            .unwrap(),
            ["linux-package"]
        );
        for id in KNOWN_ADAPTERS {
            assert_eq!(adapter(id).unwrap().id(), id);
        }
    }
}
