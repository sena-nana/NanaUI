//! Installer backends end to end (Issue #274): every format either builds
//! and checks out, or says why it did not run here — never both, never a
//! silent pass. The `.deb` is built on every host, unpacked, and its FHS
//! tree resolved by `ApplicationPaths` exactly as an installed copy would
//! be, down to reading a pack entry through the resolved locations.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nana_package::manifest::{PackageManifest, SignStatus};
use nana_package::{NoKeys, PackReader, ReadStats, TrustPolicy};
use nana_packager::installer::{CheckStatus, InstallerReport, InstallerStatus, archive};
use nana_packager::package::{PackageOptions, PackageReport, package};
use nana_packager::secrets::SecretSources;
use nana_packager::sign::{self, Host, SignContext, SignStage, SystemRunner};
use nana_ui_platform::{
    ApplicationIdentity, ApplicationPaths, PathEnvironment, PathPlatform, RuntimeLayout,
};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Short: gpg-agent's socket lives under it on macOS (104-byte limit).
    let dir = std::env::temp_dir().join(format!("nni-{name}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

const CONFIG: &str = r#"
schema_version = 1

[application]
id = "dev.nana.installer-test"
name = "Installer Test"
version = "3.1.0-rc.2"
vendor = "Nana"

[executable]
file_name = "InstallerTest"

[distribution]
backend = "installer"

[distribution.installer]
formats = ["dmg", "msi", "nsis", "deb", "appimage", "flatpak"]
summary = "Installer backend fixture"
description = "First paragraph.\n\nSecond paragraph."
homepage = "https://example.invalid/nana"
categories = ["Utility", "Development"]
deb_depends = ["libc6 (>= 2.31)"]

[runtime]
tools = [{ source = "vendor/tool.bin" }]

[resources]
root = "assets"

[[resources.packs]]
name = "ui"
class = "protected"
include = ["ui/**"]

[signing]
platform = ["linux-package", "macos-codesign", "windows-authenticode"]
"#;

fn project(dir: &Path) -> (PathBuf, PathBuf) {
    let write = |relative: &str, bytes: &[u8]| {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    write("nana-package.toml", CONFIG.as_bytes());
    write("assets/ui/app.css", b".a { color: red }");
    write("assets/ui/img/a.bin", &[7u8; 50_000]);
    write("vendor/tool.bin", b"tool");
    let mut exe = b"\x7fnot-a-real-binary\0".to_vec();
    exe.extend_from_slice(
        b"NANA-IDENTITY-V1\x00dev.nana.installer-test\x00Installer Test\x003.1.0-rc.2\x00Nana\x00END\x00",
    );
    write("bin/installer-test", &exe);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.join("bin/installer-test"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    (
        dir.join("nana-package.toml"),
        dir.join("bin/installer-test"),
    )
}

fn run(dir: &Path, target: &str) -> PackageReport {
    let (config, exe) = project(dir);
    package(&PackageOptions {
        config,
        executable: exe,
        target: target.into(),
        out: dir.join("out"),
        baseline: None,
        cache: None,
        profile: "dist".into(),
        build_id: None,
        compact: false,
        secrets: SecretSources::default(),
    })
    .unwrap()
}

fn installer<'a>(report: &'a PackageReport, format: &str) -> &'a InstallerReport {
    report
        .installers
        .iter()
        .find(|installer| installer.format == format)
        .unwrap_or_else(|| panic!("no {format} installer: {:#?}", report.installers))
}

fn built(report: &PackageReport, format: &str) -> PathBuf {
    match &installer(report, format).status {
        InstallerStatus::Built { file, size, .. } => {
            assert!(*size > 0);
            PathBuf::from(file)
        }
        other => panic!("{format} was not built: {other:?}"),
    }
}

/// Unpack `data.tar.gz` of a `.deb` under `root`.
fn unpack_deb(deb: &[u8], root: &Path) {
    let members = archive::read_ar(deb).unwrap();
    let data = archive::list_tar(&archive::gunzip(&members[2].1).unwrap()).unwrap();
    for entry in data {
        let target = root.join(entry.path.trim_start_matches("./"));
        match entry.kind {
            b'5' => std::fs::create_dir_all(&target).unwrap(),
            b'2' => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&entry.link, &target).unwrap();
            }
            _ => std::fs::write(&target, entry.data).unwrap(),
        }
    }
}

#[test]
fn linux_installers_build_a_deb_whose_fhs_tree_the_runtime_resolves() {
    let dir = scratch("linux");
    let report = run(&dir.0, "x86_64-unknown-linux-gnu");
    // Only the Linux formats, in config order.
    let formats: Vec<_> = report.installers.iter().map(|i| i.format).collect();
    assert_eq!(formats, ["deb", "appimage", "flatpak"]);

    let deb_file = built(&report, "deb");
    assert_eq!(
        deb_file,
        Path::new("installer/installer-test_3.1.0~rc.2_amd64.deb")
    );
    let InstallerStatus::Built { checks, .. } = &installer(&report, "deb").status else {
        unreachable!()
    };
    assert_eq!(checks[0].status, CheckStatus::Pass, "{checks:?}");
    let dpkg = checks.iter().find(|c| c.name == "dpkg-deb").unwrap();
    assert_eq!(
        matches!(dpkg.status, CheckStatus::Pass),
        host_has("dpkg-deb")
    );

    let deb = std::fs::read(dir.0.join("out").join(&deb_file)).unwrap();
    let members = archive::read_ar(&deb).unwrap();
    let control = archive::list_tar(&archive::gunzip(&members[1].1).unwrap()).unwrap();
    let control = String::from_utf8(control[1].data.clone()).unwrap();
    assert!(control.contains("Package: installer-test\n"), "{control}");
    assert!(control.contains("Version: 3.1.0~rc.2\n"));
    assert!(control.contains("Architecture: amd64\n"));
    assert!(control.contains("Maintainer: Nana\n"));
    assert!(control.contains("Depends: libc6 (>= 2.31)\n"));
    assert!(control.contains(
        "Description: Installer backend fixture\n First paragraph.\n .\n Second paragraph.\n"
    ));

    // Install it under a scratch root and resolve it as the runtime would.
    let root = dir.0.join("root");
    unpack_deb(&deb, &root);
    let exe = root.join("usr/lib/installer-test/InstallerTest");
    assert!(exe.is_file());
    let env = PathEnvironment {
        platform: PathPlatform::Linux,
        executable: exe.clone(),
        vars: [("HOME", "/home/u"), ("XDG_DATA_DIRS", "/nonexistent-nana")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.into()))
            .collect(),
        android_files_dir: None,
        android_cache_dir: None,
        portable_marker: false,
    };
    let identity =
        ApplicationIdentity::new("dev.nana.installer-test", "Installer Test", "3.1.0-rc.2");
    let paths = ApplicationPaths::resolve_with(&identity, &env).unwrap();
    assert_eq!(paths.layout(), RuntimeLayout::Installed);
    assert_eq!(
        paths.runtime_manifest(),
        root.join("usr/share/installer-test/manifest")
    );
    assert_eq!(
        paths.runtime_tools().join("tool.bin"),
        root.join("usr/lib/installer-test/runtime/tools/tool.bin")
    );
    assert!(paths.runtime_tools().join("tool.bin").is_file());
    let (manifest, _) =
        PackageManifest::read(paths.runtime_manifest(), &TrustPolicy::AllowUnsigned).unwrap();
    let pack = &manifest.resource_packs[0];
    let reader = PackReader::open(
        &paths.runtime_resources().join(&pack.file),
        &pack.name,
        &NoKeys,
        &TrustPolicy::AllowUnsigned,
        pack.expected().as_ref(),
    )
    .unwrap();
    assert_eq!(
        reader
            .read("ui/app.css", u64::MAX, &mut ReadStats::default())
            .unwrap(),
        b".a { color: red }"
    );
    assert!(
        root.join("usr/share/applications/dev.nana.installer-test.desktop")
            .is_file()
    );
    assert!(
        root.join("usr/share/icons/hicolor/256x256/apps/dev.nana.installer-test.png")
            .is_file()
    );
    #[cfg(unix)]
    assert_eq!(
        std::fs::canonicalize(root.join("usr/bin/InstallerTest")).unwrap(),
        std::fs::canonicalize(&exe).unwrap()
    );

    // AppImage and Flatpak need their Linux tools: not executed elsewhere,
    // with what was generated kept for the build on the right host.
    for format in ["appimage", "flatpak"] {
        match &installer(&report, format).status {
            InstallerStatus::Built { .. } => {
                if !cfg!(target_os = "linux") {
                    panic!("{format} built off Linux");
                }
            }
            InstallerStatus::NotExecuted { reason, staged } => {
                assert!(!reason.is_empty());
                if format == "flatpak" {
                    let staged = dir.0.join("out").join(staged.as_ref().unwrap());
                    let manifest: serde_json::Value = serde_json::from_slice(
                        &std::fs::read(staged.join("dev.nana.installer-test.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(manifest["command"], "InstallerTest");
                    assert_eq!(manifest["runtime"], "org.freedesktop.Platform");
                    assert!(
                        staged
                            .join("tree/share/installer-test/manifest/package.json")
                            .is_file()
                    );
                    assert!(
                        staged
                            .join("tree/lib/installer-test/InstallerTest")
                            .is_file()
                    );
                }
            }
        }
    }
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("installer flatpak not built"))
            || cfg!(target_os = "linux")
    );

    // Artifact signing ran on the .deb and, without a key, said so.
    let linux = report
        .artifact_signing
        .iter()
        .find(|s| s.adapter == "linux-package")
        .unwrap();
    let SignStatus::NotExecuted { reason } = &linux.status else {
        panic!("{:?}", linux.status);
    };
    assert!(reason.contains("NANA_GPG_KEY_ID"), "{reason}");
}

fn host_has(tool: &str) -> bool {
    use nana_packager::sign::ToolRunner;
    SystemRunner.find(tool).is_some()
}

/// A real detached GPG signature of the built `.deb`, made and verified by
/// the host's gpg with a throwaway key, where gpg exists.
#[test]
fn linux_package_signs_the_deb_with_real_gpg() {
    if !host_has("gpg") {
        return;
    }
    let dir = scratch("gpg");
    let report = run(&dir.0, "x86_64-unknown-linux-gnu");
    let deb = dir.0.join("out").join(built(&report, "deb"));
    let home = dir.0.join("g");
    std::fs::create_dir_all(&home).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let generated = std::process::Command::new("gpg")
        .arg("--homedir")
        .arg(&home)
        .args([
            "--batch",
            "--passphrase",
            "",
            "--quick-gen-key",
            "Nana Test <test@nana.invalid>",
            "ed25519",
            "sign",
            "never",
        ])
        .output()
        .unwrap();
    let kill_agent = || {
        let _ = std::process::Command::new("gpgconf")
            .arg("--homedir")
            .arg(&home)
            .args(["--kill", "gpg-agent"])
            .output();
    };
    if !generated.status.success() {
        kill_agent();
        panic!(
            "gpg key generation failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
    }
    let env: BTreeMap<String, String> = [
        ("NANA_GPG_KEY_ID", "test@nana.invalid".to_owned()),
        ("NANA_GPG_HOMEDIR", home.display().to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    let artifacts = vec![deb.clone()];
    let mut context = SignContext::artifacts(&artifacts);
    context.env = &env;
    let results = sign::run_stage(&["linux-package"], SignStage::Artifacts, &mut context).unwrap();
    // A tampered artifact no longer verifies.
    let mut bytes = std::fs::read(&deb).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&deb, bytes).unwrap();
    let tampered = sign::adapter("linux-package")
        .unwrap()
        .verify(SignStage::Artifacts, &context);
    kill_agent();
    assert_eq!(results[0].status, SignStatus::Verified, "{results:?}");
    assert!(PathBuf::from(format!("{}.asc", deb.display())).is_file());
    assert!(
        matches!(tampered, SignStatus::Failed { .. }),
        "{tampered:?}"
    );
}

#[test]
fn macos_builds_a_verified_dmg_on_a_mac_host() {
    let dir = scratch("dmg");
    let report = run(&dir.0, "aarch64-apple-darwin");
    let formats: Vec<_> = report.installers.iter().map(|i| i.format).collect();
    assert_eq!(formats, ["dmg"]);
    match &installer(&report, "dmg").status {
        InstallerStatus::Built { file, checks, .. } => {
            assert!(Host::current() == Host::Macos);
            assert_eq!(file, "installer/Installer-Test-3.1.0-rc.2.dmg");
            assert_eq!(checks[0].status, CheckStatus::Pass);
        }
        InstallerStatus::NotExecuted { reason, .. } => {
            assert!(Host::current() != Host::Macos, "{reason}");
        }
    }
    // No signing identity in the test environment: not executed, not signed.
    let codesign = &report.platform_signing[0];
    assert_eq!(codesign.adapter, "macos-codesign");
    assert!(!codesign.status.is_signed());
}

#[test]
fn windows_installer_sources_are_generated_on_any_host() {
    let dir = scratch("windows");
    let report = run(&dir.0, "x86_64-pc-windows-msvc");
    let formats: Vec<_> = report.installers.iter().map(|i| i.format).collect();
    assert_eq!(formats, ["msi", "nsis"]);
    let staged = |format: &str| match &installer(&report, format).status {
        InstallerStatus::NotExecuted { staged, .. } => {
            Some(dir.0.join("out").join(staged.as_ref().unwrap()))
        }
        InstallerStatus::Built { .. } => None,
    };
    if let Some(wxs) = staged("msi") {
        let wxs = std::fs::read_to_string(wxs).unwrap();
        assert!(wxs.contains("Version=\"3.1.0\""), "{wxs}");
        assert!(wxs.contains("Manufacturer=\"Nana\""));
        assert!(wxs.contains("Name=\"runtime\""));
        assert!(wxs.contains("InstallerTest.exe"));
        assert!(wxs.contains("ui.nrpack"));
        assert!(wxs.contains("<MajorUpgrade"));
        // Every file has exactly one component, referenced once.
        assert_eq!(
            wxs.matches("<Component Id=\"c").count(),
            wxs.matches("<ComponentRef Id=\"c").count()
        );
    }
    if let Some(nsi) = staged("nsis") {
        let nsi = std::fs::read_to_string(nsi).unwrap();
        assert!(
            nsi.contains("InstallDir \"$PROGRAMFILES64\\Installer Test\""),
            "{nsi}"
        );
        assert!(nsi.contains("SetOutPath \"$INSTDIR\\runtime\\tools\""));
        assert!(nsi.contains("Delete \"$INSTDIR\\runtime\\resources\\ui.nrpack\""));
        assert!(nsi.contains("RMDir \"$INSTDIR\\runtime\\resources\""));
        assert!(!nsi.contains("RMDir /r"));
    }
    // The Authenticode adapter cannot run without a certificate (or off
    // Windows): reported, not passed.
    let authenticode = &report.platform_signing[0];
    assert_eq!(authenticode.adapter, "windows-authenticode");
    assert!(matches!(
        authenticode.status,
        SignStatus::NotExecuted { .. }
    ));
}

/// An update manifest for the built `.deb`, signed with the publisher key,
/// that an updater on the previous release accepts and one on this release
/// refuses (nothing newer to install).
#[test]
fn update_manifests_are_signed_and_bound_to_the_package() {
    use nana_package::update::{ArtifactFormat, UpdateError, UpdateManifest, UpdatePolicy};
    use nana_packager::release::{UpdateManifestOptions, update_manifest};

    let dir = scratch("update");
    let report = run(&dir.0, "x86_64-unknown-linux-gnu");
    let deb = dir.0.join("out").join(built(&report, "deb"));
    let key_file = dir.0.join("publisher.key");
    let seed = [0x5au8; 32];
    std::fs::write(&key_file, nana_package::to_hex(&seed)).unwrap();
    let public = nana_package::PublisherKey::from_bytes(
        ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .as_bytes(),
    )
    .unwrap();
    let app_root = dir.0.join("out/app/InstallerTest");
    let out = dir.0.join("update");
    let options = UpdateManifestOptions {
        app_root: app_root.clone(),
        channel: "beta".into(),
        artifacts: vec![deb.clone()],
        minimum_from_version: Some("3.0.0".into()),
        out: out.clone(),
        secrets: SecretSources {
            signing_key_file: Some(key_file),
            ..SecretSources::default()
        },
    };
    let update = update_manifest(&options).unwrap();
    assert_eq!(update.artifacts[0].format, ArtifactFormat::Deb);

    let bytes = std::fs::read(out.join("update.json")).unwrap();
    let signature = std::fs::read(out.join("update.json.sig")).unwrap();
    let (current, _) = PackageManifest::read(
        &app_root.join("runtime/manifest"),
        &TrustPolicy::AllowUnsigned,
    )
    .unwrap();
    let mut previous = current.clone();
    previous.application.version = "3.0.1".into();
    let policy = UpdatePolicy {
        channel: "beta".into(),
        publisher: public,
    };
    let accepted = UpdateManifest::verify(&bytes, &signature, &previous, &policy).unwrap();
    accepted
        .check_artifact(
            deb.file_name().unwrap().to_str().unwrap(),
            std::fs::File::open(&deb).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        UpdateManifest::verify(&bytes, &signature, &current, &policy),
        Err(UpdateError::NotNewer { .. })
    ));
    // Unsigned updates do not exist: no key, no manifest.
    let unsigned = update_manifest(&UpdateManifestOptions {
        secrets: SecretSources::default(),
        out: dir.0.join("unsigned"),
        ..options
    });
    if std::env::var_os(nana_packager::secrets::PUBLISHER_ENV).is_none() {
        assert!(unsigned.is_err());
    }
}

#[test]
fn android_assets_carry_the_manifest_and_packs() {
    let dir = scratch("android");
    run(&dir.0, "x86_64-unknown-linux-gnu");
    let assets = dir.0.join("assets");
    let written =
        nana_packager::release::android_assets(&dir.0.join("out/app/InstallerTest"), &assets)
            .unwrap();
    assert_eq!(written.len(), 2, "{written:?}");
    let manifest = assets.join("nana/manifest/package.json");
    let (manifest, _) =
        PackageManifest::read(manifest.parent().unwrap(), &TrustPolicy::AllowUnsigned).unwrap();
    for pack in &manifest.resource_packs {
        assert!(assets.join("nana/resources").join(&pack.file).is_file());
    }
}
