//! Package the same inputs for Windows, macOS and Linux and validate each
//! result (without launching: the executable here is a stand-in carrying an
//! identity marker). Launch and tamper checks run against the real
//! `package-fixture` in CI.

use std::path::{Path, PathBuf};

use nana_package::manifest::{DistributionBackend, PackageManifest};
use nana_packager::package::{PackageOptions, package};
use nana_packager::secrets::SecretSources;
use nana_packager::validate::{Status, ValidateOptions, validate};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nana-package-layouts-{name}-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

const CONFIG: &str = r#"
schema_version = 1

[application]
id = "dev.nana.layout-test"
name = "Layout Test"
version = "2.0.0"

[executable]
file_name = "LayoutTest"

[distribution]
backend = "BACKEND"

[distribution.steam]
app_id = 480
depots = { windows = 481, macos = 482, linux = 483 }

[platform.windows]
root_exceptions = [{ source = "vendor/steam_api64.dll", reason = "Steamworks loads it from the executable directory" }]

[runtime]
tools = [{ source = "vendor/tool.bin" }]
plugins = [{ name = "fx", source = "vendor/fx.plugin", abi = "nana-plugin-1", version = "1.0.0" }]

[resources]
root = "assets"

[[resources.packs]]
name = "bootstrap"
class = "bootstrap-ui"
include = ["boot/**"]

[[resources.packs]]
name = "ui"
class = "protected"
include = ["ui/**"]
depends_on = ["bootstrap"]
"#;

fn project(dir: &Path, backend: &str) -> (PathBuf, PathBuf) {
    let write = |relative: &str, bytes: &[u8]| {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    write(
        "nana-package.toml",
        CONFIG.replace("BACKEND", backend).as_bytes(),
    );
    write("assets/boot/loading.txt", b"loading");
    write("assets/ui/app.css", b".a { color: red }");
    write("assets/ui/img/a.bin", &[7u8; 100_000]);
    write("vendor/steam_api64.dll", b"MZ fake");
    write("vendor/tool.bin", b"tool");
    write("vendor/fx.plugin", b"plugin");
    let mut exe = b"\x7fnot-a-real-binary\0".to_vec();
    exe.extend_from_slice(
        b"NANA-IDENTITY-V1\x00dev.nana.layout-test\x00Layout Test\x002.0.0\x00\x00END\x00",
    );
    write("bin/layout-test", &exe);
    (dir.join("nana-package.toml"), dir.join("bin/layout-test"))
}

fn options(config: PathBuf, exe: PathBuf, target: &str, out: PathBuf) -> PackageOptions {
    PackageOptions {
        config,
        executable: exe,
        target: target.into(),
        out,
        baseline: None,
        cache: None,
        profile: "dist".into(),
        build_id: Some("7".into()),
        compact: false,
        secrets: SecretSources::default(),
    }
}

fn status<'a>(report: &'a nana_packager::validate::ValidationReport, name: &str) -> &'a Status {
    &report
        .checks
        .iter()
        .find(|check| check.name == name)
        .unwrap_or_else(|| panic!("no check {name}: {report:#?}"))
        .status
}

#[test]
fn every_platform_layout_packages_and_validates() {
    for (target, root, manifest_dir) in [
        (
            "x86_64-pc-windows-msvc",
            "app/LayoutTest",
            "runtime/manifest",
        ),
        (
            "x86_64-unknown-linux-gnu",
            "app/LayoutTest",
            "runtime/manifest",
        ),
        (
            "aarch64-apple-darwin",
            "app/Layout Test.app",
            "Contents/Resources/manifest",
        ),
    ] {
        let dir = scratch(target);
        let (config, exe) = project(&dir.0, "steam");
        let out = dir.0.join("out");
        let report = package(&options(config, exe, target, out.clone())).unwrap();
        assert!(!report.signed);
        let app_root = out.join(root);
        let manifest_bytes =
            std::fs::read(app_root.join(manifest_dir).join("package.json")).unwrap();
        let manifest = PackageManifest::from_json(&manifest_bytes).unwrap();
        assert_eq!(manifest.application.build_id.as_deref(), Some("7"));
        assert_eq!(manifest.distribution.backend, DistributionBackend::Steam);
        assert!(!manifest.distribution.self_update);
        assert_eq!(manifest.resource_packs.len(), 2);
        assert_eq!(manifest.plugins[0].abi, "nana-plugin-1");

        let validation = validate(&ValidateOptions {
            app_root: app_root.clone(),
            ..ValidateOptions::default()
        })
        .unwrap();
        assert!(!validation.failed(), "{target}: {validation:#?}");
        assert_eq!(
            status(&validation, "layout.single-root-entry"),
            &Status::Pass
        );
        assert_eq!(
            status(&validation, "identity.binary-matches-manifest"),
            &Status::Pass
        );
        assert_eq!(status(&validation, "pack.ui"), &Status::Pass);
        assert!(matches!(
            status(&validation, "startup.early-splash"),
            Status::NotExecuted(_)
        ));

        let steam = std::fs::read_to_string(out.join("steam/nana-steam-build.json")).unwrap();
        assert!(steam.contains("\"self_update\": false"));
        if target.contains("windows") {
            assert!(app_root.join("LayoutTest.exe").is_file());
            assert!(app_root.join("steam_api64.dll").is_file());
            assert!(app_root.join("runtime/tools/tool.bin").is_file());
            assert!(steam.contains("\"path\": \"LayoutTest.exe\""));
            assert!(out.join("steam/depot_build_481.vdf").is_file());
        }
        if target.contains("darwin") {
            assert_eq!(
                status(&validation, "macos.info-plist"),
                &if cfg!(target_os = "macos") {
                    Status::Pass
                } else {
                    Status::NotExecuted("plutil is only available on a macOS host".into())
                }
            );
            let plist = std::fs::read_to_string(app_root.join("Contents/Info.plist")).unwrap();
            assert!(plist.contains("<string>2.0.0</string>"));
            assert!(app_root.join("Contents/Helpers/tool.bin").is_file());
            // Windows-only root exceptions do not reach other platforms.
            assert!(!app_root.join("steam_api64.dll").exists());
        }
    }
}

#[test]
fn validation_catches_a_stray_root_library_and_unlisted_files() {
    let dir = scratch("stray");
    let (config, exe) = project(&dir.0, "installer");
    let out = dir.0.join("out");
    package(&options(config, exe, "x86_64-pc-windows-msvc", out.clone())).unwrap();
    let app_root = out.join("app/LayoutTest");
    std::fs::write(app_root.join("helper.dll"), b"MZ").unwrap();
    std::fs::write(app_root.join("runtime/resources/extra.txt"), b"x").unwrap();
    let validation = validate(&ValidateOptions {
        app_root,
        ..ValidateOptions::default()
    })
    .unwrap();
    assert!(
        matches!(status(&validation, "layout.single-root-entry"), Status::Fail(detail) if detail.contains("helper.dll"))
    );
    assert!(
        matches!(status(&validation, "files.manifest-match"), Status::Fail(detail) if detail.contains("extra.txt"))
    );
}

#[test]
fn identity_mismatch_is_refused_and_dev_builds_flagged() {
    let dir = scratch("identity");
    let (config, exe) = project(&dir.0, "portable");
    let text = std::fs::read_to_string(&config)
        .unwrap()
        .replace("2.0.0", "2.0.1");
    std::fs::write(&config, text).unwrap();
    let error = package(&options(
        config.clone(),
        exe.clone(),
        "x86_64-unknown-linux-gnu",
        dir.0.join("out"),
    ))
    .unwrap_err();
    assert!(error.contains("2.0.1"), "{error}");

    let text = std::fs::read_to_string(&config)
        .unwrap()
        .replace("2.0.1", "2.0.0");
    std::fs::write(&config, text).unwrap();
    let mut bytes = std::fs::read(&exe).unwrap();
    bytes.extend_from_slice(b"attempt to add with overflow");
    std::fs::write(&exe, bytes).unwrap();
    let out = dir.0.join("out");
    let report = package(&options(
        config,
        exe,
        "x86_64-unknown-linux-gnu",
        out.clone(),
    ))
    .unwrap();
    assert!(report.warnings.iter().any(|w| w.contains("dev-profile")));
    let manifest = std::fs::read(out.join("app/LayoutTest/runtime/manifest/package.json")).unwrap();
    assert!(
        PackageManifest::from_json(&manifest)
            .unwrap()
            .build
            .debug_assertions_suspected
    );
}

#[test]
fn portable_builds_carry_the_marker_and_out_dir_is_protected() {
    let dir = scratch("portable");
    let (config, exe) = project(&dir.0, "portable");
    let out = dir.0.join("out");
    package(&options(
        config.clone(),
        exe.clone(),
        "x86_64-unknown-linux-gnu",
        out.clone(),
    ))
    .unwrap();
    assert!(
        out.join("app/LayoutTest/runtime/manifest/portable")
            .is_file()
    );
    // A directory the packager did not produce is never wiped.
    let foreign = dir.0.join("foreign");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(foreign.join("keep.txt"), b"x").unwrap();
    assert!(
        package(&options(
            config,
            exe,
            "x86_64-unknown-linux-gnu",
            foreign.clone()
        ))
        .is_err()
    );
    assert!(foreign.join("keep.txt").is_file());
}

#[test]
fn a_cache_inside_out_is_refused_before_anything_is_written() {
    let dir = scratch("cache-in-out");
    let (config, exe) = project(&dir.0, "portable");
    let out = dir.0.join("fresh-out");
    let mut opts = options(config, exe, "x86_64-unknown-linux-gnu", out.clone());
    opts.cache = Some(out.join("cache"));
    let error = package(&opts).unwrap_err();
    assert!(error.contains("--cache"), "{error}");
    assert!(!out.exists());
}

/// A macOS framework ships as the directory it is, symlinks kept, beside a
/// loose resource; validation hashes what the links lead to. A bundle whose
/// link leaves it is refused.
#[cfg(unix)]
#[test]
fn bundles_keep_their_symlinks_and_loose_resources_ship() {
    use std::os::unix::fs::symlink;
    let dir = scratch("bundle");
    let (config, exe) = project(&dir.0, "installer");
    let framework = dir.0.join("vendor/Fx.framework");
    std::fs::create_dir_all(framework.join("Versions/A/Resources")).unwrap();
    std::fs::write(framework.join("Versions/A/Fx"), b"fake dylib").unwrap();
    std::fs::write(
        framework.join("Versions/A/Resources/Info.plist"),
        b"<plist/>",
    )
    .unwrap();
    symlink("A", framework.join("Versions/Current")).unwrap();
    symlink("Versions/Current/Fx", framework.join("Fx")).unwrap();
    symlink("Versions/Current/Resources", framework.join("Resources")).unwrap();
    std::fs::write(dir.0.join("vendor/NOTICES.txt"), b"licences").unwrap();
    let text = std::fs::read_to_string(&config).unwrap().replace(
        "[runtime]\n",
        "[runtime]\nbin = [{ source = \"vendor/Fx.framework\", platforms = [\"macos\"] }]\n\
         resources = [{ source = \"vendor/NOTICES.txt\" }]\n",
    );
    std::fs::write(&config, text).unwrap();

    let out = dir.0.join("out");
    package(&options(
        config.clone(),
        exe.clone(),
        "aarch64-apple-darwin",
        out.clone(),
    ))
    .unwrap();
    let app_root = out.join("app/Layout Test.app");
    let shipped = app_root.join("Contents/Frameworks/Fx.framework");
    assert_eq!(
        std::fs::read_link(shipped.join("Versions/Current")).unwrap(),
        Path::new("A")
    );
    assert_eq!(std::fs::read(shipped.join("Fx")).unwrap(), b"fake dylib");
    assert_eq!(
        std::fs::read(app_root.join("Contents/Resources/NOTICES.txt")).unwrap(),
        b"licences"
    );
    let validation = validate(&ValidateOptions {
        app_root: app_root.clone(),
        ..ValidateOptions::default()
    })
    .unwrap();
    assert!(!validation.failed(), "{validation:#?}");
    // The framework is macOS-only; the resource ships everywhere.
    let windows = dir.0.join("out-windows");
    package(&options(
        config.clone(),
        exe.clone(),
        "x86_64-pc-windows-msvc",
        windows.clone(),
    ))
    .unwrap();
    assert!(
        !windows
            .join("app/LayoutTest/runtime/bin/Fx.framework")
            .exists()
    );
    assert!(
        windows
            .join("app/LayoutTest/runtime/resources/NOTICES.txt")
            .is_file()
    );

    // A changed file inside the bundle is caught like any other.
    std::fs::write(shipped.join("Versions/A/Fx"), b"other").unwrap();
    let validation = validate(&ValidateOptions {
        app_root,
        ..ValidateOptions::default()
    })
    .unwrap();
    assert!(
        matches!(status(&validation, "files.manifest-match"), Status::Fail(detail) if detail.contains("Fx.framework/Versions/A/Fx"))
    );

    symlink("../../../outside", framework.join("Versions/A/Leak")).unwrap();
    let error = package(&options(
        config,
        exe,
        "aarch64-apple-darwin",
        dir.0.join("out-leak"),
    ))
    .unwrap_err();
    assert!(
        error.contains("outside it") && error.contains("Leak"),
        "{error}"
    );
}
