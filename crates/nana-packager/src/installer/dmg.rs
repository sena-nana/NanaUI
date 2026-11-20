//! `.dmg`: the `.app` and an `/Applications` link on a compressed (UDZO)
//! disk image, built with `hdiutil` on a macOS host. The bundle is copied
//! with `ditto`, which keeps the code signature, extended attributes and
//! framework symlinks intact. `macos-notarize` signs, notarizes and staples
//! the image in the artifact stage.

use std::path::Path;

use super::{CheckStatus, InstallerInput, Meta, Outcome, ToolCheck};
use crate::sign::tool::{Host, ToolCommand};
use crate::util::io;

pub(super) fn build(
    input: &InstallerInput<'_>,
    meta: &Meta,
    dir: &Path,
    staging: &Path,
) -> Result<Outcome, String> {
    if input.host != Host::Macos {
        return Ok(Outcome::NotExecuted {
            reason: format!(
                "hdiutil runs on a macos host; this host is {}",
                input.host.as_str()
            ),
            staged: None,
        });
    }
    let (Some(hdiutil), Some(ditto)) = (
        super::find_tool(input, "hdiutil", None),
        super::find_tool(input, "ditto", None),
    ) else {
        return Ok(Outcome::NotExecuted {
            reason: "hdiutil or ditto is not on PATH".into(),
            staged: None,
        });
    };
    let bundle = input
        .app_root
        .file_name()
        .ok_or("the application root has no name")?;
    std::fs::create_dir_all(staging).map_err(io("create dmg staging"))?;
    super::run(
        input.runner,
        &ToolCommand::new(&ditto)
            .arg(input.app_root)
            .arg(staging.join(bundle)),
    )?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))
        .map_err(io("link /Applications"))?;
    let path = dir.join(format!("{}-{}.dmg", meta.stem, meta.version));
    super::run(
        input.runner,
        &ToolCommand::new(&hdiutil)
            .args(["create", "-quiet", "-ov", "-volname"])
            .arg(&meta.name)
            .arg("-srcfolder")
            .arg(staging)
            .args(["-fs", "HFS+", "-format", "UDZO"])
            .arg(&path),
    )?;
    super::run(
        input.runner,
        &ToolCommand::new(&hdiutil)
            .args(["verify", "-quiet"])
            .arg(&path),
    )?;
    Ok(Outcome::Built {
        path,
        checks: vec![ToolCheck {
            name: "hdiutil verify".into(),
            status: CheckStatus::Pass,
        }],
    })
}
