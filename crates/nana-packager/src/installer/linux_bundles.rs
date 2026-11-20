//! AppImage and Flatpak: the FHS tree in each format's container, built by
//! the format's own tool on a Linux host.
//!
//! - AppImage: an AppDir (`AppRun` → `usr/bin/<exe>`, `<id>.desktop`,
//!   `<id>.png`, `.DirIcon`, `usr/` in FHS form) packed by `appimagetool`
//!   (`NANA_APPIMAGETOOL` names it when it is not on `PATH`). Inside the
//!   mounted image the executable is `$APPDIR/usr/lib/<pkg>/<exe>`, so
//!   `ApplicationPaths` reads `$APPDIR/usr/share/<pkg>/`.
//! - Flatpak: a `flatpak-builder` manifest whose one module copies the
//!   staged tree into `/app`, built into a repository and exported as a
//!   single-file bundle with `flatpak build-bundle`. The runtime and SDK
//!   (`flatpak_runtime`, `flatpak_sdk`) must be installed on the build host.
//!
//! On another host, or without the tool, the AppDir / manifest is kept
//! under `installer/staging/` and the format reports `not-executed`.

use std::path::Path;

use super::{InstallerInput, Meta, Outcome};
use crate::sign::tool::{Host, ToolCommand};
use crate::util::io;

const APPIMAGE_ARCHES: [(&str, &str); 4] = [
    ("x86_64", "x86_64"),
    ("aarch64", "aarch64"),
    ("i686", "i686"),
    ("armv7", "armhf"),
];

pub(super) fn appimage(
    input: &InstallerInput<'_>,
    meta: &Meta,
    dir: &Path,
    staging: &Path,
) -> Result<Outcome, String> {
    let arch = super::arch(input.target, &APPIMAGE_ARCHES)?;
    if input.host != Host::Linux {
        return Ok(Outcome::NotExecuted {
            reason: format!(
                "appimagetool runs on a linux host; this host is {}",
                input.host.as_str()
            ),
            staged: None,
        });
    }
    let appdir = staging.join(format!("{}.AppDir", meta.stem));
    let icon = super::linux_icon(input)?;
    super::stage_fhs(input, meta, &appdir.join("usr"), &icon, true)?;
    std::fs::write(
        appdir.join(format!("{}.desktop", meta.id)),
        super::desktop_entry(meta),
    )
    .map_err(io("write AppDir desktop entry"))?;
    std::fs::write(appdir.join(format!("{}.png", meta.id)), &icon)
        .map_err(io("write AppDir icon"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(format!("usr/bin/{}", meta.exe), appdir.join("AppRun"))
            .map_err(io("link AppRun"))?;
        symlink(format!("{}.png", meta.id), appdir.join(".DirIcon"))
            .map_err(io("link .DirIcon"))?;
    }
    let Some(tool) = super::find_tool(input, "appimagetool", Some("NANA_APPIMAGETOOL")) else {
        return Ok(Outcome::NotExecuted {
            reason: "`appimagetool` is not on PATH (or set NANA_APPIMAGETOOL); the AppDir is \
                     staged"
                .into(),
            staged: Some(appdir),
        });
    };
    let path = dir.join(format!("{}-{}-{arch}.AppImage", meta.stem, meta.version));
    super::run(
        input.runner,
        &ToolCommand::new(tool)
            .arg("--no-appstream")
            .arg(&appdir)
            .arg(&path)
            .env("ARCH", arch.to_owned()),
    )?;
    Ok(Outcome::Built {
        path,
        checks: Vec::new(),
    })
}

/// Flatpak application ids: at least three dot-separated elements of
/// `[A-Za-z0-9_-]`, none starting with a digit.
pub(crate) fn valid_flatpak_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    parts.len() >= 3
        && id.len() <= 255
        && parts.iter().all(|part| {
            !part.is_empty()
                && !part.as_bytes()[0].is_ascii_digit()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

pub(crate) fn flatpak_manifest(input: &InstallerInput<'_>, meta: &Meta) -> serde_json::Value {
    let installer = input.installer;
    serde_json::json!({
        "app-id": meta.id,
        "runtime": installer.flatpak_runtime,
        "runtime-version": installer.flatpak_runtime_version,
        "sdk": installer.flatpak_sdk,
        "command": meta.exe,
        "finish-args": installer.flatpak_finish_args,
        "modules": [{
            "name": meta.package,
            "buildsystem": "simple",
            "build-commands": [
                "cp -a lib share /app/",
                "mkdir -p /app/bin",
                format!("ln -sf ../lib/{}/{} /app/bin/{}", meta.package, meta.exe, meta.exe),
            ],
            "sources": [{ "type": "dir", "path": "tree" }],
        }],
    })
}

pub(super) fn flatpak(
    input: &InstallerInput<'_>,
    meta: &Meta,
    dir: &Path,
    staging: &Path,
) -> Result<Outcome, String> {
    if !valid_flatpak_id(&meta.id) {
        return Err(format!(
            "application id `{}` is not a Flatpak application id (three or more \
             dot-separated names of letters, digits, `_` and `-`)",
            meta.id
        ));
    }
    let icon = super::linux_icon(input)?;
    // `/app/bin/<exe>` is linked by the build commands, so the staged tree
    // holds no symlink and stages on any host.
    super::stage_fhs(input, meta, &staging.join("tree"), &icon, false)?;
    let manifest = staging.join(format!("{}.json", meta.id));
    std::fs::write(
        &manifest,
        serde_json::to_string_pretty(&flatpak_manifest(input, meta)).expect("json"),
    )
    .map_err(io("write Flatpak manifest"))?;
    let not_executed = |reason: String| {
        Ok(Outcome::NotExecuted {
            reason,
            staged: Some(staging.to_path_buf()),
        })
    };
    if input.host != Host::Linux {
        return not_executed(format!(
            "flatpak-builder runs on a linux host; this host is {} (the manifest is staged)",
            input.host.as_str()
        ));
    }
    let (Some(builder), Some(flatpak)) = (
        super::find_tool(input, "flatpak-builder", None),
        super::find_tool(input, "flatpak", None),
    ) else {
        return not_executed(
            "`flatpak-builder` or `flatpak` is not on PATH; the manifest is staged".into(),
        );
    };
    let repo = staging.join("repo");
    super::run(
        input.runner,
        &ToolCommand::new(builder)
            .arg("--force-clean")
            .arg(format!("--repo={}", repo.display()))
            .arg(staging.join("build"))
            .arg(&manifest),
    )?;
    let path = dir.join(format!("{}-{}.flatpak", meta.id, meta.version));
    super::run(
        input.runner,
        &ToolCommand::new(flatpak)
            .arg("build-bundle")
            .arg(&repo)
            .arg(&path)
            .arg(&meta.id),
    )?;
    Ok(Outcome::Built {
        path,
        checks: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatpak_ids() {
        assert!(valid_flatpak_id("dev.nana.fixture"));
        assert!(valid_flatpak_id("dev.nana.layout-test"));
        assert!(!valid_flatpak_id("nana.fixture"));
        assert!(!valid_flatpak_id("dev.9nana.fixture"));
        assert!(!valid_flatpak_id("dev..fixture"));
    }
}
