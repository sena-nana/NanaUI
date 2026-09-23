//! The pack plan: which resource goes into which pack, and whether the
//! packs can be read in startup order (Issue #226 §4, §7).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use nana_package::ResourceClass;

use crate::config::{CompressionConfig, KeyAvailability, KeyConfig, PackageConfig};

/// EarlySplash packs are read before the full runtime exists; keep them tiny.
pub const EARLY_SPLASH_MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedEntry {
    /// Logical path, `/`-separated, relative to `resources.root`.
    pub key: String,
    pub source: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct PlannedPack {
    pub name: String,
    pub class: ResourceClass,
    pub entries: Vec<PlannedEntry>,
    pub key: Option<(String, KeyConfig)>,
    pub depends_on: Vec<String>,
    pub compression: CompressionConfig,
    pub max_bytes: Option<u64>,
    /// Directory prefixes (with trailing `/`) covering every entry, or the
    /// entry key itself for top-level files. Routes a lookup to this pack.
    pub prefixes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PackPlan {
    pub packs: Vec<PlannedPack>,
    pub block_size: u32,
    pub max_free_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    Io(String),
    InvalidPath(String),
    /// Two logical paths differ only in case; they collide on Windows and
    /// macOS file systems.
    CaseCollision(String, String),
    /// A resource matches the include patterns of more than one pack.
    Overlap {
        key: String,
        packs: [String; 2],
    },
    DuplicatePack(String),
    UnknownKey {
        pack: String,
        key: String,
    },
    UnknownDependency {
        pack: String,
        dependency: String,
    },
    /// A pack depends on a pack of a later startup class.
    StageInversion {
        pack: String,
        dependency: String,
    },
    DependencyCycle(Vec<String>),
    EarlySplashEncrypted(String),
    EarlySplashTooLarge {
        pack: String,
        bytes: u64,
    },
    EarlySplashDependency(String),
    /// A BootstrapUI pack is encrypted with a key that only a flow driven by
    /// that same UI can obtain: the application would start to a blank
    /// window, forever.
    StartupKeyCycle {
        pack: String,
        key: String,
    },
    EmptyPack(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::InvalidPath(path) => {
                write!(f, "resource path `{path}` is not a valid logical path")
            }
            Self::CaseCollision(a, b) => {
                write!(f, "resource paths `{a}` and `{b}` differ only in case")
            }
            Self::Overlap { key, packs } => write!(
                f,
                "resource `{key}` matches both pack `{}` and pack `{}`",
                packs[0], packs[1]
            ),
            Self::DuplicatePack(name) => write!(f, "pack `{name}` is declared twice"),
            Self::UnknownKey { pack, key } => {
                write!(
                    f,
                    "pack `{pack}` uses key `{key}`, which [keys] does not declare"
                )
            }
            Self::UnknownDependency { pack, dependency } => {
                write!(f, "pack `{pack}` depends on unknown pack `{dependency}`")
            }
            Self::StageInversion { pack, dependency } => write!(
                f,
                "pack `{pack}` depends on `{dependency}`, which belongs to a later startup class"
            ),
            Self::DependencyCycle(path) => {
                write!(f, "pack dependency cycle: {}", path.join(" -> "))
            }
            Self::EarlySplashEncrypted(pack) => write!(
                f,
                "EarlySplash pack `{pack}` cannot be encrypted: it is read before any key exists"
            ),
            Self::EarlySplashTooLarge { pack, bytes } => write!(
                f,
                "EarlySplash pack `{pack}` holds {bytes} bytes; the limit is {EARLY_SPLASH_MAX_BYTES}"
            ),
            Self::EarlySplashDependency(pack) => {
                write!(f, "EarlySplash pack `{pack}` cannot depend on other packs")
            }
            Self::StartupKeyCycle { pack, key } => write!(
                f,
                "startup dependency cycle: BootstrapUI pack `{pack}` is encrypted with key `{key}`, \
                 which is only available after the BootstrapUI runs (availability = \
                 \"after-bootstrap-ui\"). Use an embedded or process-start key, or leave the \
                 pack unencrypted"
            ),
            Self::EmptyPack(pack) => write!(f, "pack `{pack}` matches no resources"),
        }
    }
}

impl PackPlan {
    /// Scan `resources.root` and assign every file to exactly one pack.
    pub fn build(config: &PackageConfig, base: &Path) -> Result<Option<Self>, PlanError> {
        let Some(resources) = &config.resources else {
            return Ok(None);
        };
        let root = base.join(&resources.root);
        let mut files = Vec::new();
        scan(&root, &root, &mut files)?;
        files.sort_by(|a, b| a.key.as_bytes().cmp(b.key.as_bytes()));

        let mut lowered: BTreeMap<String, String> = BTreeMap::new();
        for file in &files {
            if let Some(previous) = lowered.insert(file.key.to_lowercase(), file.key.clone()) {
                return Err(PlanError::CaseCollision(previous, file.key.clone()));
            }
        }

        let mut names = BTreeSet::new();
        for pack in &resources.packs {
            if !valid_pack_name(&pack.name) {
                return Err(PlanError::InvalidPath(pack.name.clone()));
            }
            if !names.insert(pack.name.as_str()) {
                return Err(PlanError::DuplicatePack(pack.name.clone()));
            }
        }

        let mut assigned: Vec<Vec<PlannedEntry>> = vec![Vec::new(); resources.packs.len()];
        for file in files {
            let mut owner: Option<usize> = None;
            for (index, pack) in resources.packs.iter().enumerate() {
                let included = pack.include.iter().any(|p| glob_match(p, &file.key));
                let excluded = pack.exclude.iter().any(|p| glob_match(p, &file.key));
                if included && !excluded {
                    if let Some(first) = owner {
                        return Err(PlanError::Overlap {
                            key: file.key,
                            packs: [resources.packs[first].name.clone(), pack.name.clone()],
                        });
                    }
                    owner = Some(index);
                }
            }
            // Files no pack claims are not packaged (source-only assets).
            if let Some(index) = owner {
                assigned[index].push(file);
            }
        }

        let mut packs = Vec::new();
        for (pack, entries) in resources.packs.iter().zip(assigned) {
            if entries.is_empty() {
                return Err(PlanError::EmptyPack(pack.name.clone()));
            }
            let key = match &pack.key {
                Some(name) => Some((
                    name.clone(),
                    config
                        .keys
                        .get(name)
                        .cloned()
                        .ok_or_else(|| PlanError::UnknownKey {
                            pack: pack.name.clone(),
                            key: name.clone(),
                        })?,
                )),
                None => None,
            };
            let prefixes = prefixes(&entries);
            packs.push(PlannedPack {
                name: pack.name.clone(),
                class: pack.class,
                entries,
                key,
                depends_on: pack.depends_on.clone(),
                compression: pack.compression.unwrap_or(resources.compression),
                max_bytes: pack.max_bytes,
                prefixes,
            });
        }
        let plan = Self {
            packs,
            block_size: resources.block_size,
            max_free_ratio: resources.max_free_ratio,
        };
        plan.check_startup()?;
        Ok(Some(plan))
    }

    /// Issue #226 §7: EarlySplash → BootstrapUI → Protected must be readable
    /// in that order without a cycle.
    pub fn check_startup(&self) -> Result<(), PlanError> {
        let by_name: BTreeMap<&str, &PlannedPack> =
            self.packs.iter().map(|p| (p.name.as_str(), p)).collect();
        for pack in &self.packs {
            match pack.class {
                ResourceClass::EarlySplash => {
                    if pack.key.is_some() {
                        return Err(PlanError::EarlySplashEncrypted(pack.name.clone()));
                    }
                    let bytes: u64 = pack.entries.iter().map(|e| e.size).sum();
                    if bytes > EARLY_SPLASH_MAX_BYTES {
                        return Err(PlanError::EarlySplashTooLarge {
                            pack: pack.name.clone(),
                            bytes,
                        });
                    }
                    if !pack.depends_on.is_empty() {
                        return Err(PlanError::EarlySplashDependency(pack.name.clone()));
                    }
                }
                ResourceClass::BootstrapUi => {
                    if let Some((key, config)) = &pack.key
                        && config.availability == KeyAvailability::AfterBootstrapUi
                    {
                        return Err(PlanError::StartupKeyCycle {
                            pack: pack.name.clone(),
                            key: key.clone(),
                        });
                    }
                }
                ResourceClass::Protected => {}
            }
            for dependency in &pack.depends_on {
                let target = by_name.get(dependency.as_str()).ok_or_else(|| {
                    PlanError::UnknownDependency {
                        pack: pack.name.clone(),
                        dependency: dependency.clone(),
                    }
                })?;
                if target.class > pack.class {
                    return Err(PlanError::StageInversion {
                        pack: pack.name.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }
        // Depth-first search for cycles among same-class dependencies.
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            New,
            Active,
            Done,
        }
        fn visit<'a>(
            name: &'a str,
            by_name: &BTreeMap<&'a str, &'a PlannedPack>,
            marks: &mut BTreeMap<&'a str, Mark>,
            stack: &mut Vec<&'a str>,
        ) -> Result<(), PlanError> {
            match marks.get(name).copied().unwrap_or(Mark::New) {
                Mark::Done => return Ok(()),
                Mark::Active => {
                    let start = stack.iter().position(|n| *n == name).unwrap_or(0);
                    let mut cycle: Vec<String> =
                        stack[start..].iter().map(|n| (*n).to_owned()).collect();
                    cycle.push(name.to_owned());
                    return Err(PlanError::DependencyCycle(cycle));
                }
                Mark::New => {}
            }
            marks.insert(name, Mark::Active);
            stack.push(name);
            for dependency in &by_name[name].depends_on {
                visit(dependency.as_str(), by_name, marks, stack)?;
            }
            stack.pop();
            marks.insert(name, Mark::Done);
            Ok(())
        }
        let mut marks = BTreeMap::new();
        for pack in &self.packs {
            visit(pack.name.as_str(), &by_name, &mut marks, &mut Vec::new())?;
        }
        Ok(())
    }
}

fn valid_pack_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

pub use nana_package::valid_logical_path;

fn scan(root: &Path, dir: &Path, out: &mut Vec<PlannedEntry>) -> Result<(), PlanError> {
    let read = std::fs::read_dir(dir)
        .map_err(|error| PlanError::Io(format!("cannot read {}: {error}", dir.display())))?;
    for item in read {
        let item = item.map_err(|error| PlanError::Io(error.to_string()))?;
        let path = item.path();
        let kind = item
            .file_type()
            .map_err(|error| PlanError::Io(error.to_string()))?;
        if kind.is_symlink() {
            // A symlink could pull files from outside the resource root.
            return Err(PlanError::InvalidPath(path.display().to_string()));
        }
        let name = item.file_name();
        // Operating-system litter differs between machines; packing it would
        // make builds irreproducible and add Steam delta noise.
        if matches!(
            name.to_str(),
            Some(".DS_Store" | "Thumbs.db" | "desktop.ini" | ".git" | ".svn")
        ) {
            continue;
        }
        if kind.is_dir() {
            scan(root, &path, out)?;
        } else if kind.is_file() {
            let key = crate::util::relative_path(root, &path)
                .map_err(|_| PlanError::InvalidPath(path.display().to_string()))?;
            if !valid_logical_path(&key) {
                return Err(PlanError::InvalidPath(key));
            }
            let size = item
                .metadata()
                .map_err(|error| PlanError::Io(error.to_string()))?
                .len();
            out.push(PlannedEntry {
                key,
                source: path,
                size,
            });
        }
    }
    Ok(())
}

/// Shortest set of directory prefixes that routes every entry here: the
/// top-level directory of each entry (`ui/`), or the file itself at the top
/// level. Longest-prefix routing at run time picks the pack.
fn prefixes(entries: &[PlannedEntry]) -> Vec<String> {
    let set: BTreeSet<String> = entries
        .iter()
        .map(|entry| match entry.key.split_once('/') {
            Some((dir, _)) => format!("{dir}/"),
            None => entry.key.clone(),
        })
        .collect();
    set.into_iter().collect()
}

/// Glob over `/`-separated paths: `**` matches any number of segments, `*`
/// any run within a segment, `?` one character.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    match_segments(&pattern, &path)
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| match_segments(rest, &path[skip..])),
        Some((first, rest)) => match path.split_first() {
            Some((segment, path_rest)) => {
                match_segment(first.as_bytes(), segment.as_bytes())
                    && match_segments(rest, path_rest)
            }
            None => false,
        },
    }
}

fn match_segment(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|skip| match_segment(rest, &text[skip..])),
        Some((b'?', rest)) => !text.is_empty() && match_segment(rest, &text[1..]),
        Some((c, rest)) => text.first() == Some(c) && match_segment(rest, &text[1..]),
    }
}

/// Pack plan conflicts with the routing prefixes of another pack: two packs
/// claiming the same prefix make run-time routing ambiguous.
pub fn check_prefixes(plan: &PackPlan) -> Result<(), String> {
    let mut owners: BTreeMap<&str, &str> = BTreeMap::new();
    for pack in &plan.packs {
        for prefix in &pack.prefixes {
            if let Some(other) = owners.insert(prefix.as_str(), pack.name.as_str()) {
                return Err(format!(
                    "packs `{other}` and `{}` both hold resources under `{prefix}`; split them by \
                     top-level directory so each lookup routes to one pack",
                    pack.name
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PackageConfig;
    use crate::config::tests::MINIMAL;

    #[test]
    fn globs() {
        assert!(glob_match("ui/**", "ui/a/b.png"));
        assert!(glob_match("ui/**", "ui/a.png"));
        assert!(glob_match("**/*.png", "a/b/c.png"));
        assert!(glob_match("**/*.png", "c.png"));
        assert!(!glob_match("ui/*", "ui/a/b.png"));
        assert!(glob_match("ui/?.css", "ui/a.css"));
        assert!(!glob_match("ui/**/*.psd", "ui/a.png"));
    }

    #[test]
    fn logical_paths() {
        assert!(valid_logical_path("ui/logo.png"));
        for bad in ["", "/a", "a//b", "a/../b", "./a", "a\\b", "a/\u{1}", "C:/x"] {
            assert!(!valid_logical_path(bad), "{bad:?}");
        }
    }

    struct Tree(PathBuf);
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree(name: &str, files: &[(&str, usize)]) -> Tree {
        let dir =
            std::env::temp_dir().join(format!("nana-packager-plan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, size) in files {
            let path = dir.join("assets").join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, vec![1u8; *size]).unwrap();
        }
        Tree(dir)
    }

    fn config(packs: &str, keys: &str) -> PackageConfig {
        PackageConfig::parse(&format!(
            "{MINIMAL}\n[resources]\nroot = \"assets\"\n{packs}\n{keys}"
        ))
        .unwrap()
    }

    const TWO_PACKS: &str = r#"
[[resources.packs]]
name = "bootstrap"
class = "bootstrap-ui"
include = ["boot/**"]

[[resources.packs]]
name = "ui"
class = "protected"
include = ["ui/**"]
exclude = ["ui/**/*.psd"]
key = "main"
depends_on = ["bootstrap"]
"#;

    #[test]
    fn assigns_files_and_prefixes() {
        let t = tree(
            "assign",
            &[
                ("boot/logo.png", 10),
                ("ui/a.css", 5),
                ("ui/x/b.png", 7),
                ("ui/src.psd", 3),
            ],
        );
        let config = config(
            TWO_PACKS,
            "[keys.main]\ngeneration = 1\navailability = \"process-start\"",
        );
        let plan = PackPlan::build(&config, &t.0).unwrap().unwrap();
        let ui = &plan.packs[1];
        let keys: Vec<_> = ui.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["ui/a.css", "ui/x/b.png"]);
        assert_eq!(ui.prefixes, ["ui/"]);
        assert_eq!(plan.packs[0].prefixes, ["boot/"]);
        check_prefixes(&plan).unwrap();
    }

    #[test]
    fn bootstrap_ui_with_late_key_is_a_startup_cycle() {
        let t = tree("cycle", &[("boot/logo.png", 10), ("ui/a.css", 5)]);
        let packs = TWO_PACKS.replace(
            "include = [\"boot/**\"]",
            "include = [\"boot/**\"]\nkey = \"login\"",
        );
        let config = config(
            &packs,
            "[keys.main]\ngeneration = 1\navailability = \"process-start\"\n[keys.login]\ngeneration = 1\navailability = \"after-bootstrap-ui\"",
        );
        assert_eq!(
            PackPlan::build(&config, &t.0).unwrap_err(),
            PlanError::StartupKeyCycle {
                pack: "bootstrap".into(),
                key: "login".into()
            }
        );
    }

    #[test]
    fn stage_inversion_overlap_and_splash_rules() {
        let t = tree("rules", &[("boot/logo.png", 10), ("ui/a.css", 5)]);
        let keys = "[keys.main]\ngeneration = 1\navailability = \"process-start\"";
        // bootstrap depends on a protected pack.
        let inverted = TWO_PACKS
            .replace(
                "include = [\"boot/**\"]",
                "include = [\"boot/**\"]\ndepends_on = [\"ui\"]",
            )
            .replace("depends_on = [\"bootstrap\"]", "");
        assert!(matches!(
            PackPlan::build(&config(&inverted, keys), &t.0).unwrap_err(),
            PlanError::StageInversion { .. }
        ));
        let overlap = TWO_PACKS.replace("include = [\"ui/**\"]", "include = [\"**\"]");
        assert!(matches!(
            PackPlan::build(&config(&overlap, keys), &t.0).unwrap_err(),
            PlanError::Overlap { .. }
        ));
        let splash = TWO_PACKS
            .replace("class = \"bootstrap-ui\"", "class = \"early-splash\"")
            .replace(
                "include = [\"boot/**\"]",
                "include = [\"boot/**\"]\nkey = \"main\"",
            );
        assert!(matches!(
            PackPlan::build(&config(&splash, keys), &t.0).unwrap_err(),
            PlanError::EarlySplashEncrypted(_)
        ));
    }

    #[test]
    fn dependency_cycles_are_reported() {
        let t = tree("dfs", &[("a/x", 1), ("b/y", 1)]);
        let packs = r#"
[[resources.packs]]
name = "a"
class = "protected"
include = ["a/**"]
depends_on = ["b"]

[[resources.packs]]
name = "b"
class = "protected"
include = ["b/**"]
depends_on = ["a"]
"#;
        assert!(matches!(
            PackPlan::build(&config(packs, ""), &t.0).unwrap_err(),
            PlanError::DependencyCycle(_)
        ));
    }

    #[test]
    fn case_collisions_are_rejected() {
        let t = tree("case", &[("ui/Logo.png", 1), ("ui/logo.png", 1)]);
        // Case-insensitive file systems cannot even hold both files.
        let count = std::fs::read_dir(t.0.join("assets/ui")).unwrap().count();
        if count == 2 {
            let packs =
                "[[resources.packs]]\nname = \"ui\"\nclass = \"protected\"\ninclude = [\"**\"]";
            assert!(matches!(
                PackPlan::build(&config(packs, ""), &t.0).unwrap_err(),
                PlanError::CaseCollision(..)
            ));
        }
    }
}
