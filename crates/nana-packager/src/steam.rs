//! Steam backend output (Issue #226 §8).
//!
//! SteamPipe owns version deployment, rollback and branches, so a Steam
//! package carries no Nana updater and no old versions. This writes a depot
//! build script whose content root is the packaged application, plus a file
//! list for audits. Upload credentials belong to `steamcmd` in CI; nothing
//! here reads or writes them.

use std::path::Path;

use serde::Serialize;

use crate::config::PackageConfig;
use crate::layout::{PackageLayout, TargetPlatform};
use crate::util::{content_hex, io, relative_path, walk_files};

pub const STEAM_BUILD_FILE: &str = "nana-steam-build.json";

#[derive(Debug, Serialize)]
struct SteamBuild<'a> {
    app_id: u32,
    depot_id: u32,
    platform: &'a str,
    /// Relative to this file.
    content_root: String,
    self_update: bool,
    files: Vec<SteamFile>,
}

#[derive(Debug, Serialize)]
struct SteamFile {
    path: String,
    size: u64,
    blake3: String,
}

pub fn write_steam_output(
    config: &PackageConfig,
    platform: TargetPlatform,
    out: &Path,
    app_parent: &Path,
    layout: &PackageLayout,
) -> Result<(), String> {
    let steam = config
        .distribution
        .steam
        .as_ref()
        .ok_or("distribution.backend = \"steam\" needs [distribution.steam]")?;
    let depot_id = *steam.depots.get(platform.as_str()).ok_or_else(|| {
        format!(
            "[distribution.steam].depots has no `{}` depot",
            platform.as_str()
        )
    })?;
    let dir = out.join("steam");
    std::fs::create_dir_all(&dir).map_err(io("create steam directory"))?;

    // The depot root is the directory that holds the application: App.exe
    // and runtime/ on Windows and Linux, Name.app on macOS.
    let content = match platform {
        TargetPlatform::Macos => app_parent.to_path_buf(),
        _ => app_parent.join(&layout.root),
    };
    let mut files = Vec::new();
    for path in walk_files(&content)? {
        let bytes = std::fs::read(&path).map_err(io("hash depot file"))?;
        files.push(SteamFile {
            path: relative_path(&content, &path)?,
            size: bytes.len() as u64,
            blake3: content_hex(&bytes),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let build = SteamBuild {
        app_id: steam.app_id,
        depot_id,
        platform: platform.as_str(),
        content_root: content_root(layout, platform),
        self_update: false,
        files,
    };
    std::fs::write(
        dir.join(STEAM_BUILD_FILE),
        serde_json::to_string_pretty(&build).expect("steam build serializes"),
    )
    .map_err(io("write steam build"))?;
    std::fs::write(
        dir.join(format!("depot_build_{depot_id}.vdf")),
        format!(
            "\"DepotBuildConfig\"\n{{\n\t\"DepotID\" \"{depot_id}\"\n\t\"ContentRoot\" \"{}\"\n\t\"FileMapping\"\n\t{{\n\t\t\"LocalPath\" \"*\"\n\t\t\"DepotPath\" \".\"\n\t\t\"recursive\" \"1\"\n\t}}\n}}\n",
            content_root(layout, platform)
        ),
    )
    .map_err(io("write depot build script"))?;
    Ok(())
}

fn content_root(layout: &PackageLayout, platform: TargetPlatform) -> String {
    match platform {
        // The depot holds `Name.app` itself.
        TargetPlatform::Macos => "../app".into(),
        _ => format!("../app/{}", layout.root),
    }
}
