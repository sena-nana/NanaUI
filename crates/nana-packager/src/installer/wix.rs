//! `.msi`: a WiX v4+ source (`.wxs`) generated from the application tree
//! and built with `wix build` on a Windows host (`NANA_WIX` names the tool
//! when it is not on `PATH`). Elsewhere the `.wxs` is kept under
//! `installer/staging/msi/` and the format reports `not-executed`.
//!
//! The package installs per machine into `Program Files\<Name>`, with a
//! Start menu shortcut, and upgrades in place: its upgrade code is stable
//! (`msi_upgrade_code`, else derived from the application id), so a newer
//! version replaces an older one and a downgrade is refused.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::{InstallerInput, Meta, Outcome};
use crate::sign::tool::{Host, ToolCommand};
use crate::util::io;

const ARCHES: [(&str, &str); 4] = [
    ("x86_64", "x64"),
    ("aarch64", "arm64"),
    ("i686", "x86"),
    ("i586", "x86"),
];

/// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` (braces optional) → canonical
/// upper-case form with braces.
pub(crate) fn parse_guid(text: &str) -> Option<String> {
    let inner = text
        .strip_prefix('{')
        .and_then(|t| t.strip_suffix('}'))
        .unwrap_or(text);
    let groups: Vec<&str> = inner.split('-').collect();
    let lengths = [8, 4, 4, 4, 12];
    (groups.len() == 5
        && groups
            .iter()
            .zip(lengths)
            .all(|(group, len)| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit())))
    .then(|| format!("{{{}}}", inner.to_ascii_uppercase()))
}

/// A stable upgrade code for `application_id`: a name-based (version 5
/// layout) GUID, so every build of the application agrees on it.
pub(crate) fn derived_upgrade_code(application_id: &str) -> String {
    let mut bytes: [u8; 16] =
        nana_package::hash::derive_prefix("nana.msi.upgrade-code.v1", &[application_id.as_bytes()]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = nana_package::to_hex(&bytes).to_ascii_uppercase();
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// MSI's `major.minor.build` (255.255.65535). A prerelease or build suffix
/// cannot be expressed and is dropped.
pub(crate) fn msi_version(version: &str) -> Result<String, String> {
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<u64> = core
        .split('.')
        .map(|part| part.parse::<u64>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("application version `{version}` is not dotted numeric"))?;
    let get = |index: usize| parts.get(index).copied().unwrap_or(0);
    let (major, minor, build) = (get(0), get(1), get(2));
    if parts.is_empty() || parts.len() > 3 || major > 255 || minor > 255 || build > 65535 {
        return Err(format!(
            "application version `{version}` does not fit an MSI version \
             (major.minor.build up to 255.255.65535)"
        ));
    }
    Ok(format!("{major}.{minor}.{build}"))
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The `.wxs` for the application tree at `app_root`.
pub(crate) fn wxs(input: &InstallerInput<'_>, meta: &Meta) -> Result<String, String> {
    let version = msi_version(&meta.version)?;
    let upgrade_code = match &input.installer.msi_upgrade_code {
        Some(code) => parse_guid(code).ok_or("msi_upgrade_code is not a GUID")?,
        None => derived_upgrade_code(&meta.id),
    };
    let mut components = Vec::new();
    let mut body = String::new();
    let mut counter = 0usize;
    fn tree(
        dir: &Path,
        depth: usize,
        body: &mut String,
        components: &mut Vec<String>,
        counter: &mut usize,
    ) -> Result<(), String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(io("list application"))?
            .collect::<Result<_, _>>()
            .map_err(io("list application"))?;
        entries.sort_by_key(|entry| entry.file_name());
        let indent = "  ".repeat(depth);
        for entry in entries {
            let path = entry.path();
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| format!("{} is not UTF-8", path.display()))?;
            *counter += 1;
            let id = *counter;
            if path.is_dir() {
                writeln!(
                    body,
                    "{indent}<Directory Id=\"d{id}\" Name=\"{}\">",
                    xml(&name)
                )
                .unwrap();
                tree(&path, depth + 1, body, components, counter)?;
                writeln!(body, "{indent}</Directory>").unwrap();
            } else {
                let source = std::path::absolute(&path).map_err(io("resolve source"))?;
                writeln!(
                    body,
                    "{indent}<Component Id=\"c{id}\"><File Id=\"f{id}\" Source=\"{}\" KeyPath=\"yes\" /></Component>",
                    xml(&source.display().to_string())
                )
                .unwrap();
                components.push(format!("c{id}"));
            }
        }
        Ok(())
    }
    tree(input.app_root, 4, &mut body, &mut components, &mut counter)?;
    let exe = &input.layout.executable;
    let mut refs = String::new();
    for component in &components {
        writeln!(refs, "      <ComponentRef Id=\"{component}\" />").unwrap();
    }
    let registry_key = format!("Software\\{}\\{}", meta.vendor, meta.name);
    Ok(format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!-- Generated by nana-packager from the packaged application tree. -->
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="{name}" Manufacturer="{vendor}" Version="{version}" UpgradeCode="{upgrade_code}" Scope="perMachine" Compressed="yes">
    <MajorUpgrade DowngradeErrorMessage="A newer version of [ProductName] is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <StandardDirectory Id="ProgramFiles64Folder">
      <Directory Id="INSTALLFOLDER" Name="{name}">
{body}      </Directory>
    </StandardDirectory>
    <StandardDirectory Id="ProgramMenuFolder">
      <Component Id="StartMenuShortcut">
        <Shortcut Id="AppShortcut" Name="{name}" Target="[INSTALLFOLDER]{exe}" WorkingDirectory="INSTALLFOLDER" />
        <RegistryValue Root="HKLM" Key="{registry_key}" Name="shortcut" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </StandardDirectory>
    <Feature Id="Main">
{refs}      <ComponentRef Id="StartMenuShortcut" />
    </Feature>
  </Package>
</Wix>
"#,
        name = xml(&meta.name),
        vendor = xml(&meta.vendor),
        exe = xml(&exe.replace('/', "\\")),
        registry_key = xml(&registry_key),
    ))
}

pub(super) fn build(
    input: &InstallerInput<'_>,
    meta: &Meta,
    dir: &Path,
    staging: &Path,
) -> Result<Outcome, String> {
    let arch = super::arch(input.target, &ARCHES)?;
    let source = wxs(input, meta)?;
    std::fs::create_dir_all(staging).map_err(io("create msi staging"))?;
    let wxs_path: PathBuf = staging.join(format!("{}.wxs", meta.stem));
    std::fs::write(&wxs_path, source).map_err(io("write .wxs"))?;
    let not_executed = |reason: String| {
        Ok(Outcome::NotExecuted {
            reason,
            staged: Some(wxs_path.clone()),
        })
    };
    if input.host != Host::Windows {
        return not_executed(format!(
            "wix builds MSI packages on a windows host; this host is {} (the .wxs is staged)",
            input.host.as_str()
        ));
    }
    let Some(wix) = super::find_tool(input, "wix", Some("NANA_WIX")) else {
        return not_executed("`wix` (WiX Toolset v4+) is not on PATH (or set NANA_WIX)".into());
    };
    let path = dir.join(format!("{}-{}-{arch}.msi", meta.stem, meta.version));
    super::run(
        input.runner,
        &ToolCommand::new(wix)
            .args(["build", "-arch", arch, "-o"])
            .arg(&path)
            .arg(&wxs_path),
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
    fn guids_and_versions() {
        assert_eq!(
            parse_guid("12345678-abcd-ef01-2345-6789abcdef01").as_deref(),
            Some("{12345678-ABCD-EF01-2345-6789ABCDEF01}")
        );
        assert!(parse_guid("{1234}").is_none());
        let derived = derived_upgrade_code("dev.nana.fixture");
        assert_eq!(derived, derived_upgrade_code("dev.nana.fixture"));
        assert_ne!(derived, derived_upgrade_code("dev.nana.other"));
        assert_eq!(parse_guid(&derived).as_deref(), Some(derived.as_str()));
        assert_eq!(&derived[15..16], "5");
        assert_eq!(msi_version("1.2.3-rc.1").unwrap(), "1.2.3");
        assert_eq!(msi_version("2").unwrap(), "2.0.0");
        assert!(msi_version("256.0.0").is_err());
        assert!(msi_version("1.2.3.4").is_err());
    }
}
