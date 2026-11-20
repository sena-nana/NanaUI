//! `.deb`: written by the packager itself, on any host. An `ar` archive of
//! `debian-binary`, `control.tar.gz` and `data.tar.gz`, the data in FHS form
//! under `/usr`. The result is parsed back before it is reported built, and
//! checked with `dpkg-deb` where that tool exists.

use std::path::Path;

use super::archive::{self, Tar};
use super::{CheckStatus, InstallerInput, Meta, Outcome, ToolCheck};
use crate::sign::tool::ToolCommand;
use crate::util::io;

/// Rust target architectures → Debian architectures.
const ARCHES: [(&str, &str); 9] = [
    ("x86_64", "amd64"),
    ("aarch64", "arm64"),
    ("i686", "i386"),
    ("i586", "i386"),
    ("armv7", "armhf"),
    ("riscv64gc", "riscv64"),
    ("powerpc64le", "ppc64el"),
    ("s390x", "s390x"),
    ("loongarch64", "loong64"),
];

/// The application version as a Debian upstream version: a prerelease
/// sorts before its release (`1.0.0-rc.1` → `1.0.0~rc.1`).
pub(crate) fn deb_version(version: &str) -> Result<String, String> {
    let (core, build) = match version.split_once('+') {
        Some((core, build)) => (core, Some(build)),
        None => (version, None),
    };
    let mut text = core.replacen('-', "~", 1);
    if let Some(build) = build {
        text.push('+');
        text.push_str(build);
    }
    let valid = text.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+.~-".contains(&b));
    if valid {
        Ok(text)
    } else {
        Err(format!(
            "application version `{version}` cannot be a Debian version (it must start with a \
             digit and use letters, digits, `.`, `+`, `-`)"
        ))
    }
}

fn mode_of(path: &Path, relative: &str, meta: &Meta) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = (relative, meta);
        std::fs::metadata(path).map_or(0o644, |m| {
            if m.permissions().mode() & 0o111 != 0 {
                0o755
            } else {
                0o644
            }
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        let lib = format!("usr/lib/{}/", meta.package);
        let executable = relative == format!("{lib}{}", meta.exe)
            || relative.starts_with(&format!("{lib}runtime/tools/"));
        if executable { 0o755 } else { 0o644 }
    }
}

/// Every entry under `root` into `tar`, directories before their contents,
/// siblings sorted. Returns the installed size in KiB.
fn add_tree(tar: &mut Tar, root: &Path, meta: &Meta) -> Result<u64, String> {
    fn walk(
        tar: &mut Tar,
        root: &Path,
        dir: &Path,
        meta: &Meta,
        kib: &mut u64,
    ) -> Result<(), String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(io("list staging"))?
            .collect::<Result<_, _>>()
            .map_err(io("list staging"))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = crate::util::relative_path(root, &path)?;
            let name = format!("./{relative}");
            let kind = entry.file_type().map_err(io("stat staging"))?;
            *kib += 1;
            if kind.is_symlink() {
                let target = std::fs::read_link(&path).map_err(io("read link"))?;
                let target = target
                    .to_str()
                    .ok_or_else(|| format!("{} links to a non-UTF-8 path", path.display()))?
                    .replace('\\', "/");
                tar.symlink(&name, &target)?;
            } else if kind.is_dir() {
                tar.dir(&name, 0o755)?;
                walk(tar, root, &path, meta, kib)?;
            } else {
                let bytes = std::fs::read(&path).map_err(io("read staged file"))?;
                *kib += (bytes.len() as u64).div_ceil(1024).saturating_sub(1);
                tar.file(&name, mode_of(&path, &relative, meta), &bytes)?;
            }
        }
        Ok(())
    }
    tar.dir(".", 0o755)?;
    let mut kib = 0;
    walk(tar, root, root, meta, &mut kib)?;
    Ok(kib)
}

fn control(input: &InstallerInput<'_>, meta: &Meta, version: &str, arch: &str, kib: u64) -> String {
    let mut text = format!(
        "Package: {}\nVersion: {version}\nArchitecture: {arch}\nMaintainer: {}\n\
         Installed-Size: {kib}\nSection: misc\nPriority: optional\n",
        meta.package, meta.maintainer
    );
    if !input.installer.deb_depends.is_empty() {
        text.push_str(&format!(
            "Depends: {}\n",
            input.installer.deb_depends.join(", ")
        ));
    }
    if let Some(homepage) = &meta.homepage {
        text.push_str(&format!("Homepage: {homepage}\n"));
    }
    text.push_str(&format!("Description: {}\n", meta.summary));
    if let Some(description) = &meta.description {
        for line in description.trim().lines() {
            let line = line.trim_end();
            if line.is_empty() {
                text.push_str(" .\n");
            } else {
                text.push_str(&format!(" {line}\n"));
            }
        }
    }
    text
}

pub(super) fn build(
    input: &InstallerInput<'_>,
    meta: &Meta,
    dir: &Path,
    staging: &Path,
) -> Result<Outcome, String> {
    let arch = super::arch(input.target, &ARCHES)?;
    let version = deb_version(&meta.version)?;
    let root = staging.join("root");
    let icon = super::linux_icon(input)?;
    let links = super::stage_fhs(input, meta, &root.join("usr"), &icon, true)?;

    let mtime = super::source_date(input.env);
    let mut data = Tar::new(mtime);
    let kib = add_tree(&mut data, &root, meta)?;
    // Hosts without symlinks staged none: add them to the archive directly.
    for (link, target) in &links {
        if std::fs::symlink_metadata(root.join("usr").join(link)).is_err() {
            data.symlink(&format!("./usr/{link}"), target)?;
        }
    }
    let data = archive::gzip(&data.finish())?;
    let mut control_tar = Tar::new(mtime);
    control_tar.dir(".", 0o755)?;
    control_tar.file(
        "./control",
        0o644,
        control(input, meta, &version, arch, kib).as_bytes(),
    )?;
    let control_tar = archive::gzip(&control_tar.finish())?;
    let deb = archive::ar(
        &[
            ("debian-binary", b"2.0\n"),
            ("control.tar.gz", &control_tar),
            ("data.tar.gz", &data),
        ],
        mtime,
    );
    let path = dir.join(format!("{}_{version}_{arch}.deb", meta.package));
    std::fs::write(&path, &deb).map_err(io("write .deb"))?;

    let mut checks = vec![self_parse(&deb, meta)?];
    checks.push(match super::find_tool(input, "dpkg-deb", None) {
        Some(dpkg) => {
            super::run(
                input.runner,
                &ToolCommand::new(&dpkg).arg("--info").arg(&path),
            )?;
            super::run(
                input.runner,
                &ToolCommand::new(&dpkg).arg("--contents").arg(&path),
            )?;
            ToolCheck {
                name: "dpkg-deb".into(),
                status: CheckStatus::Pass,
            }
        }
        None => ToolCheck {
            name: "dpkg-deb".into(),
            status: CheckStatus::NotExecuted("dpkg-deb is not on PATH".into()),
        },
    });
    Ok(Outcome::Built { path, checks })
}

/// Parse the archive back: the members dpkg expects, in order, and the
/// files the runtime needs where `ApplicationPaths` looks for them.
fn self_parse(deb: &[u8], meta: &Meta) -> Result<ToolCheck, String> {
    let members = archive::read_ar(deb)?;
    let names: Vec<&str> = members.iter().map(|(name, _)| name.as_str()).collect();
    if names != ["debian-binary", "control.tar.gz", "data.tar.gz"] {
        return Err(format!("the built .deb has members {names:?}"));
    }
    let data = archive::list_tar(&archive::gunzip(&members[2].1)?)?;
    for required in [
        format!("./usr/lib/{}/{}", meta.package, meta.exe),
        format!(
            "./usr/share/{}/manifest/{}",
            meta.package,
            nana_package::MANIFEST_FILE
        ),
        format!("./usr/bin/{}", meta.exe),
        format!("./usr/share/applications/{}.desktop", meta.id),
    ] {
        if !data.iter().any(|entry| entry.path == required) {
            return Err(format!("the built .deb lacks {required}"));
        }
    }
    let control = archive::list_tar(&archive::gunzip(&members[1].1)?)?;
    if !control.iter().any(|entry| entry.path == "./control") {
        return Err("the built .deb has no control file".into());
    }
    Ok(ToolCheck {
        name: "deb.self-parse".into(),
        status: CheckStatus::Pass,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_map_to_debian_order() {
        assert_eq!(deb_version("1.2.3").unwrap(), "1.2.3");
        assert_eq!(deb_version("1.2.3-rc.1").unwrap(), "1.2.3~rc.1");
        assert_eq!(deb_version("1.2.3-rc.1+git5").unwrap(), "1.2.3~rc.1+git5");
        assert!(deb_version("v1.0").is_err());
        assert_eq!(
            super::super::arch("aarch64-unknown-linux-gnu", &ARCHES).unwrap(),
            "arm64"
        );
        assert_eq!(
            super::super::arch("armv7-unknown-linux-gnueabihf", &ARCHES).unwrap(),
            "armhf"
        );
        assert!(super::super::arch("wasm32-unknown-unknown", &ARCHES).is_err());
    }
}
