//! HarmonyOS / OpenHarmony system fonts (`docs/reference/ohos.md`).
//!
//! The system describes its fonts in `/system/etc/fontconfig.json`, the
//! schema of OpenHarmony's Skia port: `fontdir` (directories), `generic`
//! (family aliases), `fallback` (BCP 47 tag → family, in order) and
//! `font_file_map`. The first three are read. The file map is not: its names
//! have not always matched the files on a device, and a face's family comes
//! from its own name table anyway. A missing or unparseable file, or one that
//! names no directory that exists, leaves `/system/fonts` and the built-in
//! HarmonyOS chain ([`OHOS_FAMILIES`]).
//!
//! Every path is taken under an injected root, so all of this runs on any
//! host; the device passes `/`.

use super::fallback::{FallbackPolicy, OHOS_FAMILIES};
use super::query::GenericFamily;
use crate::shape::ScriptTag;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

pub(super) const CONFIG_PATH: &str = "system/etc/fontconfig.json";
pub(super) const FONTS_DIR: &str = "system/fonts";

const HANI: ScriptTag = ScriptTag(*b"Hani");
const HIRA: ScriptTag = ScriptTag(*b"Hira");
const KANA: ScriptTag = ScriptTag(*b"Kana");
const HANG: ScriptTag = ScriptTag(*b"Hang");

/// What `fontconfig.json` says, before any of it is checked against the disk.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct OhosFontConfig {
    /// `fontdir`, as the file spells them (absolute device paths).
    pub font_dirs: Vec<PathBuf>,
    /// `generic[].alias` keys that are CSS generics, with their family.
    pub generics: Vec<(GenericFamily, String)>,
    /// The default (`""`) `fallback` set: (tag, family) in file order, a
    /// comma-separated key split into one pair per tag.
    pub fallback: Vec<(String, String)>,
}

impl OhosFontConfig {
    /// `None` when `root/system/etc/fontconfig.json` is missing or is not a
    /// JSON object.
    pub fn read(root: &Path) -> Option<Self> {
        Self::parse(&std::fs::read_to_string(root.join(CONFIG_PATH)).ok()?)
    }

    /// Unknown keys and entries of the wrong shape are skipped, not errors:
    /// the schema has grown keys between releases.
    pub fn parse(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let object = value.as_object()?;
        let array = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        };

        let font_dirs = array("fontdir")
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect();

        let mut generics = Vec::new();
        for entry in array("generic") {
            let Some(family) = entry.get("family").and_then(Value::as_str) else {
                continue;
            };
            let aliases = entry.get("alias").and_then(Value::as_array);
            for alias in aliases.into_iter().flatten().filter_map(Value::as_object) {
                for key in alias.keys() {
                    if let Some(generic) = GenericFamily::from_css_keyword(key) {
                        generics.push((generic, family.to_owned()));
                    }
                }
            }
        }

        let mut fallback = Vec::new();
        for set in array("fallback").filter_map(Value::as_object) {
            let Some(entries) = set.get("").and_then(Value::as_array) else {
                continue;
            };
            for entry in entries.iter().filter_map(Value::as_object) {
                for (key, family) in entry {
                    // Sibling keys such as `font-variations` are not families.
                    let Some(family) = family.as_str() else {
                        continue;
                    };
                    for tag in key.split(',') {
                        fallback.push((tag.trim().to_owned(), family.to_owned()));
                    }
                }
            }
        }

        Some(Self {
            font_dirs,
            generics,
            fallback,
        })
    }
}

/// `dir` (an absolute device path) under `root`. Anything but plain
/// components is refused, so the file cannot name a place outside `root`.
fn under_root(root: &Path, dir: &Path) -> Option<PathBuf> {
    let mut components = dir.components();
    if components.next() != Some(Component::RootDir) {
        return None;
    }
    let mut path = root.to_path_buf();
    for component in components {
        match component {
            Component::Normal(part) => path.push(part),
            _ => return None,
        }
    }
    Some(path)
}

/// Directories to scan: the configured ones that exist under `root`, in file
/// order, else `root/system/fonts`.
pub(super) fn font_dirs(root: &Path, config: Option<&OhosFontConfig>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in config.into_iter().flat_map(|config| &config.font_dirs) {
        let Some(dir) = under_root(root, dir) else {
            continue;
        };
        if dir.is_dir() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    if dirs.is_empty() {
        dirs.push(root.join(FONTS_DIR));
    }
    dirs
}

/// Where one `fallback` tag's family goes in the policy.
enum Slot {
    LastResort,
    Emoji,
    Symbol,
    Math,
    Scripts(Vec<(ScriptTag, Option<&'static str>)>),
    /// A language tag that names a script: its rule is narrowed to it.
    LanguageScript(ScriptTag, String),
}

fn script_subtag(subtag: &str) -> Option<ScriptTag> {
    let bytes = subtag.as_bytes();
    if bytes.len() != 4 || !bytes.iter().all(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut tag = [0; 4];
    for (index, byte) in bytes.iter().enumerate() {
        tag[index] = if index == 0 {
            byte.to_ascii_uppercase()
        } else {
            byte.to_ascii_lowercase()
        };
    }
    Some(ScriptTag(tag))
}

fn slot(tag: &str) -> Option<Slot> {
    let tag = tag.trim().replace('_', "-").to_ascii_lowercase();
    let slot = match tag.as_str() {
        "" => Slot::LastResort,
        "und-zsye" => Slot::Emoji,
        "und-zsym" => Slot::Symbol,
        "und-math" | "und-zmth" => Slot::Math,
        "zh" | "zh-hans" | "zh-cn" | "zh-sg" => Slot::Scripts(vec![(HANI, None)]),
        "zh-hant" => Slot::Scripts(vec![
            (HANI, Some("zh-hant")),
            (HANI, Some("zh-tw")),
            (HANI, Some("zh-hk")),
            (HANI, Some("zh-mo")),
        ]),
        "ja" => Slot::Scripts(vec![(HANI, Some("ja")), (HIRA, None), (KANA, None)]),
        "ko" => Slot::Scripts(vec![(HANG, None), (HANI, Some("ko"))]),
        _ => {
            let (language, script) = tag.split_once('-')?;
            let script = script_subtag(script.split('-').next()?)?;
            if language == "und" {
                Slot::Scripts(vec![(script, None)])
            } else {
                Slot::LanguageScript(script, language.to_owned())
            }
        }
    };
    Some(slot)
}

/// `first` then `then`, without repeating a family (ASCII case-insensitive).
fn merged<'a>(
    first: impl IntoIterator<Item = &'a str>,
    then: impl IntoIterator<Item = &'a str>,
) -> Vec<&'a str> {
    let mut families: Vec<&str> = Vec::new();
    for family in first.into_iter().chain(then) {
        if !families
            .iter()
            .any(|known| known.eq_ignore_ascii_case(family))
        {
            families.push(family);
        }
    }
    families
}

/// The OHOS fallback policy: what `config` says first, the built-in
/// HarmonyOS chain after it. `None` is the built-in chain alone.
pub(super) fn policy(config: Option<&OhosFontConfig>) -> FallbackPolicy {
    let mut policy = FallbackPolicy::empty();
    let mut last_resort: Vec<&str> = Vec::new();
    let mut emoji: Vec<&str> = Vec::new();
    let mut symbol: Vec<&str> = Vec::new();
    let mut math: Vec<&str> = Vec::new();
    // Script rules are tried in push order, so the file's go in first.
    for (tag, family) in config.into_iter().flat_map(|config| &config.fallback) {
        match slot(tag) {
            Some(Slot::LastResort) => last_resort.push(family),
            Some(Slot::Emoji) => emoji.push(family),
            Some(Slot::Symbol) => symbol.push(family),
            Some(Slot::Math) => math.push(family),
            Some(Slot::Scripts(rules)) => {
                for (script, language) in rules {
                    policy.push_script_rule(script, language, [family.as_str()]);
                }
            }
            Some(Slot::LanguageScript(script, language)) => {
                policy.push_script_rule(script, Some(&language), [family.as_str()]);
            }
            None => {}
        }
    }
    OHOS_FAMILIES.apply(&mut policy);

    let generics = [
        GenericFamily::Serif,
        GenericFamily::SansSerif,
        GenericFamily::Monospace,
        GenericFamily::Cursive,
        GenericFamily::Fantasy,
        GenericFamily::SystemUi,
        GenericFamily::Math,
    ];
    for generic in generics {
        let configured: Vec<&str> = config
            .into_iter()
            .flat_map(|config| &config.generics)
            .filter(|(known, _)| *known == generic)
            .map(|(_, family)| family.as_str())
            .chain(if generic == GenericFamily::Math {
                math.clone()
            } else {
                Vec::new()
            })
            .collect();
        if configured.is_empty() {
            continue;
        }
        let built_in: Vec<String> = policy
            .generic(generic)
            .iter()
            .map(|family| family.to_string())
            .collect();
        let families = merged(configured, built_in.iter().map(String::as_str));
        policy.set_generic(generic, families);
    }

    let built_in_emoji: Vec<String> = policy
        .emoji_families()
        .iter()
        .map(|f| f.to_string())
        .collect();
    let built_in_symbol: Vec<String> = policy
        .symbol_families()
        .iter()
        .map(|f| f.to_string())
        .collect();
    let built_in_last: Vec<String> = policy
        .last_resort_families()
        .iter()
        .map(|f| f.to_string())
        .collect();
    let emoji = merged(emoji, built_in_emoji.iter().map(String::as_str));
    let symbol = merged(symbol, built_in_symbol.iter().map(String::as_str));
    // As the built-in chain builds it: symbols and emoji are last resorts too.
    let last_resort = merged(
        last_resort
            .into_iter()
            .chain(symbol.iter().copied())
            .chain(emoji.iter().copied()),
        built_in_last.iter().map(String::as_str),
    );
    policy
        .set_emoji_families(emoji)
        .set_symbol_families(symbol)
        .set_last_resort_families(last_resort);
    policy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::LanguageTag;
    use std::sync::Arc;

    /// The shape of OpenHarmony's shipped `fontconfig.json`, trimmed: a
    /// sibling `font-variations` key, a comma-separated tag and the file map.
    const CONFIG: &str = r#"{
      "fontdir": ["/system/fonts/", "/vendor/fonts/"],
      "generic": [
        { "family": "HarmonyOS Sans",
          "alias": [{ "HarmonyOS-Sans": 0 }, { "HarmonyOS-Sans-Bold": 900 }] },
        { "family": "Noto Serif", "alias": [{ "serif": 0 }] },
        { "family": "Noto Sans Mono", "alias": [{ "monospace": 400 }] }
      ],
      "fallback": [
        { "": [
          { "und-Zsye": "HMOS Color Emoji" },
          { "": "Noto Sans" },
          { "font-variations": [{ "weight": 400, "wght": 400 }],
            "und-Arab": "HarmonyOS Sans Naskh Arabic UI" },
          { "und-Geor,und-Geok": "Noto Sans Georgian" },
          { "und-Gujr": "Noto Sans Gujarati" },
          { "zh-Hans": "HarmonyOS Sans SC" },
          { "zh-Hant": "HarmonyOS Sans TC" },
          { "ja": "Noto Sans CJK JP" },
          { "ko": "Noto Sans CJK JP" },
          { "und-Zsym": "Noto Sans Symbols" },
          { "und-Math": "Noto Sans Math" }
        ] },
        { "other-set": [{ "und-Latn": "Not The Default Set" }] }
      ],
      "font_file_map": [{ "HarmonyOS-Sans": "HarmonyOS_Sans.ttf" }]
    }"#;

    fn names(families: &[Arc<str>]) -> Vec<&str> {
        families.iter().map(|family| &**family).collect()
    }

    fn script(policy: &FallbackPolicy, tag: &[u8; 4], language: Option<&str>) -> Vec<String> {
        let language = language.and_then(LanguageTag::new);
        policy
            .script_families(ScriptTag(*tag), language.as_ref())
            .iter()
            .map(|family| family.to_string())
            .collect()
    }

    /// A scratch root removed on drop.
    struct Root(PathBuf);

    impl Root {
        fn new(name: &str) -> Self {
            static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "nana-text-ohos-{name}-{}-{seq}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn dir(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn config(&self, text: &str) {
            let path = self.0.join(CONFIG_PATH);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_shipped_schema_parses_into_dirs_generics_and_the_default_fallback_set() {
        let config = OhosFontConfig::parse(CONFIG).unwrap();
        assert_eq!(
            config.font_dirs,
            [
                PathBuf::from("/system/fonts/"),
                PathBuf::from("/vendor/fonts/")
            ]
        );
        assert_eq!(
            config.generics,
            [
                (GenericFamily::Serif, "Noto Serif".to_owned()),
                (GenericFamily::Monospace, "Noto Sans Mono".to_owned()),
            ]
        );
        let tags: Vec<&str> = config
            .fallback
            .iter()
            .map(|(tag, _)| tag.as_str())
            .collect();
        assert_eq!(
            tags,
            [
                "und-Zsye", "", "und-Arab", "und-Geor", "und-Geok", "und-Gujr", "zh-Hans",
                "zh-Hant", "ja", "ko", "und-Zsym", "und-Math",
            ]
        );
        assert!(OhosFontConfig::parse("not json").is_none());
        assert!(OhosFontConfig::parse("[]").is_none());
        assert_eq!(OhosFontConfig::parse("{}"), Some(OhosFontConfig::default()));
    }

    #[test]
    fn without_a_config_the_built_in_harmonyos_chain_applies() {
        let policy = policy(None);
        let mut built_in = FallbackPolicy::empty();
        OHOS_FAMILIES.apply(&mut built_in);
        assert_eq!(policy, built_in);
        assert_eq!(
            names(policy.generic(GenericFamily::SansSerif))[..2],
            ["HarmonyOS Sans", "HarmonyOS Sans SC"]
        );
        assert_eq!(script(&policy, b"Hani", None)[0], "HarmonyOS Sans SC");
        assert_eq!(
            script(&policy, b"Hani", Some("zh-TW"))[0],
            "HarmonyOS Sans TC"
        );
        assert_eq!(
            script(&policy, b"Hani", Some("zh-HK"))[0],
            "HarmonyOS Sans TC"
        );
        assert_eq!(script(&policy, b"Hira", None)[0], "Noto Sans CJK JP");
        assert_eq!(
            names(policy.emoji_families()),
            ["HMOS Color Emoji", "HMOS Color Emoji Flags"]
        );
    }

    #[test]
    fn config_entries_go_ahead_of_the_built_in_chain() {
        let mut config = OhosFontConfig::parse(CONFIG).unwrap();
        config
            .fallback
            .insert(0, ("zh-Hans".into(), "Vendor SC".into()));
        config
            .fallback
            .push(("und-Zsye".into(), "Vendor Emoji".into()));
        config
            .generics
            .push((GenericFamily::SansSerif, "Vendor Sans".into()));
        let policy = policy(Some(&config));

        assert_eq!(
            script(&policy, b"Hani", None),
            ["Vendor SC", "HarmonyOS Sans SC", "Noto Sans CJK JP"]
        );
        assert_eq!(
            script(&policy, b"Hani", Some("zh-Hant-TW"))[0],
            "HarmonyOS Sans TC"
        );
        assert_eq!(script(&policy, b"Hani", Some("ja"))[0], "Noto Sans CJK JP");
        assert_eq!(script(&policy, b"Hang", None)[0], "Noto Sans CJK JP");
        // Scripts the built-in chain does not name come from the file alone.
        assert_eq!(script(&policy, b"Gujr", None), ["Noto Sans Gujarati"]);
        assert_eq!(script(&policy, b"Geok", None), ["Noto Sans Georgian"]);
        assert!(
            script(&policy, b"Latn", None).is_empty(),
            "only the default set is read"
        );
        assert_eq!(
            script(&policy, b"Arab", None),
            [
                "HarmonyOS Sans Naskh Arabic UI",
                "HarmonyOS Sans Naskh Arabic"
            ]
        );

        assert_eq!(
            names(policy.generic(GenericFamily::SansSerif)),
            [
                "Vendor Sans",
                "HarmonyOS Sans",
                "HarmonyOS Sans SC",
                "Noto Sans"
            ]
        );
        assert_eq!(
            names(policy.generic(GenericFamily::Monospace)),
            ["Noto Sans Mono", "HarmonyOS Sans"]
        );
        assert_eq!(
            names(policy.generic(GenericFamily::Math)),
            ["Noto Sans Math"]
        );
        assert_eq!(
            names(policy.emoji_families()),
            ["HMOS Color Emoji", "Vendor Emoji", "HMOS Color Emoji Flags"]
        );
        assert_eq!(names(policy.symbol_families())[0], "Noto Sans Symbols");
        assert_eq!(names(policy.last_resort_families())[0], "Noto Sans");
    }

    #[test]
    fn language_tags_with_a_script_narrow_their_rule_and_unknown_tags_are_dropped() {
        let config = OhosFontConfig {
            fallback: vec![
                ("sr-Cyrl".into(), "Serbian Cyrillic".into()),
                ("en".into(), "Plain Language".into()),
                ("und-Toolong".into(), "Bad Script".into()),
            ],
            ..OhosFontConfig::default()
        };
        let policy = policy(Some(&config));
        assert_eq!(
            script(&policy, b"Cyrl", Some("sr-RS")),
            ["Serbian Cyrillic"]
        );
        assert!(script(&policy, b"Cyrl", None).is_empty());
        assert_eq!(policy, {
            let mut expected = FallbackPolicy::empty();
            expected.push_script_rule(ScriptTag(*b"Cyrl"), Some("sr"), ["Serbian Cyrillic"]);
            OHOS_FAMILIES.apply(&mut expected);
            expected
        });
    }

    #[test]
    fn configured_dirs_that_exist_are_scanned_and_system_fonts_is_the_fallback() {
        let root = Root::new("dirs");
        // No config, no directory: still the one place OHOS keeps fonts.
        assert_eq!(font_dirs(&root.0, None), [root.0.join(FONTS_DIR)]);

        let system = root.dir("system/fonts");
        let config = OhosFontConfig::parse(CONFIG).unwrap();
        // `/vendor/fonts/` does not exist under this root and is skipped.
        assert_eq!(
            font_dirs(&root.0, Some(&config)),
            std::slice::from_ref(&system)
        );

        let vendor = root.dir("vendor/fonts");
        assert_eq!(font_dirs(&root.0, Some(&config)), [system.clone(), vendor]);

        let escaping = OhosFontConfig {
            font_dirs: vec![
                PathBuf::from("/system/../../outside"),
                PathBuf::from("system/fonts"),
                PathBuf::from("/missing"),
            ],
            ..OhosFontConfig::default()
        };
        assert_eq!(font_dirs(&root.0, Some(&escaping)), [system]);
    }

    #[test]
    fn the_config_is_read_from_under_the_injected_root() {
        let root = Root::new("read");
        assert_eq!(OhosFontConfig::read(&root.0), None);
        root.config("{ broken");
        assert_eq!(OhosFontConfig::read(&root.0), None);
        root.config(CONFIG);
        assert_eq!(OhosFontConfig::read(&root.0), OhosFontConfig::parse(CONFIG));
    }
}
