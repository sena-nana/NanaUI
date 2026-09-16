//! The migration corpus: input cases and their committed golden layouts.
//!
//! Cases and goldens live in this crate rather than beside the reference
//! engine on purpose. The golden is the baseline a native engine has to
//! satisfy, so it has to survive the deletion of the engine it was first
//! recorded from.

use crate::constraints::TextConstraints;
use crate::counters::TextWorkCounters;
use crate::edit::{CaretGeometry, CaretPosition, HitTestResult};
use crate::layout::TextLayout;
use crate::source::TextSpan;
use crate::style::{TextKind, TextStyle};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Bumped when the golden envelope changes shape, so a mass re-bless is a
/// deliberate, reviewable act rather than a silent drift.
pub const GOLDEN_SCHEMA_VERSION: u32 = 1;

/// Every font fixture a case may name.
///
/// This is the contract between the corpus and whatever supplies font bytes:
/// the corpus asserts each case names a known id, and the supplier asserts it
/// can produce bytes for every id here. Neither side can drift alone.
pub const KNOWN_FONT_FIXTURES: &[&str] = &[
    // The product UI face: Latin, combining marks, f-ligatures, kerning, kana,
    // and CJK ideographs.
    "noto-sans-sc",
    // Two glyphs, `wdth` plus a custom `BEVL` axis, cmap covers only U+0041.
    // Doubles as the missing-glyph and fallback fixture.
    "nana-test-vf",
    "noto-sans-arabic",
    "noto-sans-kr",
    "noto-emoji",
];

/// Whether a case is expected to match its golden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaseStatus {
    #[default]
    Pass,
    /// A known gap. Requires `gap` and `gap_note`.
    Ignore,
}

/// A pointer position to hit-test.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CorpusHitTest {
    pub x_px: f32,
    pub y_px: f32,
}

/// A caret to resolve geometry for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CorpusCaret {
    pub byte: usize,
    #[serde(default)]
    pub affinity: crate::edit::Affinity,
    #[serde(default)]
    pub line: u32,
}

impl From<CorpusCaret> for CaretPosition {
    fn from(caret: CorpusCaret) -> Self {
        Self {
            byte: caret.byte,
            affinity: caret.affinity,
            line: caret.line,
        }
    }
}

/// One corpus input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusCase {
    pub id: String,
    /// A sentence describing the behaviour, used as the test name.
    pub title: String,
    #[serde(default)]
    pub status: CaseStatus,
    /// Gap identifier, required when `status` is `Ignore`.
    #[serde(default)]
    pub gap: Option<String>,
    #[serde(default)]
    pub gap_note: Option<String>,
    /// Which corpus category this row covers. Must be one of [`CATEGORIES`].
    pub category: String,
    #[serde(default)]
    pub kind: TextKind,
    /// Font fixture ids. Registration order is fallback order.
    pub fonts: Vec<String>,
    pub text: String,
    #[serde(default)]
    pub spans: Vec<TextSpan>,
    #[serde(default)]
    pub style: TextStyle,
    #[serde(default)]
    pub constraints: TextConstraints,
    #[serde(default)]
    pub hit_tests: Vec<CorpusHitTest>,
    #[serde(default)]
    pub carets: Vec<CorpusCaret>,
    /// `None` uses [`DEFAULT_TOLERANCES`](super::DEFAULT_TOLERANCES).
    #[serde(default)]
    pub tolerances: Option<super::Tolerances>,
}

/// The categories #89 requires the corpus to cover.
///
/// A category with no case is a silently dropped requirement, so the corpus
/// well-formedness test asserts each one is populated.
pub const CATEGORIES: &[&str] = &[
    "latin",
    "cjk",
    "hangul",
    "emoji",
    "combining",
    "ligature",
    "rtl",
    "bidi",
    "fallback",
    "missing-glyph",
    "variable-font",
    "wrap",
    "dpi",
    "ime",
];

/// A recorded layout plus the caret answers derived from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Golden {
    pub schema_version: u32,
    pub case: String,
    pub layout: TextLayout,
    #[serde(default)]
    pub hit_tests: Vec<HitTestResult>,
    #[serde(default)]
    pub carets: Vec<Option<CaretGeometry>>,
    /// Counters the producing pass reported. Recorded so a counter change is a
    /// reviewable diff rather than an invisible one.
    #[serde(default)]
    pub counters: TextWorkCounters,
}

/// Root of the checked-in corpus.
pub fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus")
}

pub fn corpus_case_dir() -> PathBuf {
    corpus_dir().join("cases")
}

pub fn corpus_golden_dir() -> PathBuf {
    corpus_dir().join("golden")
}

pub fn case_path(id: &str) -> PathBuf {
    corpus_case_dir().join(format!("{id}.json"))
}

pub fn golden_path(id: &str) -> PathBuf {
    corpus_golden_dir().join(format!("{id}.layout.json"))
}

/// Reads one case by id.
pub fn load_case(id: &str) -> Result<CorpusCase, CorpusError> {
    let path = case_path(id);
    let bytes = std::fs::read(&path).map_err(|source| CorpusError::Io {
        path: path.clone(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| CorpusError::Parse { path, source })
}

/// Reads one golden by case id.
pub fn load_golden(id: &str) -> Result<Golden, CorpusError> {
    let path = golden_path(id);
    let bytes = std::fs::read(&path).map_err(|source| CorpusError::Io {
        path: path.clone(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| CorpusError::Parse { path, source })
}

/// Reads every case, sorted by id.
pub fn load_cases() -> Result<Vec<CorpusCase>, CorpusError> {
    let dir = corpus_case_dir();
    let mut ids = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|source| CorpusError::Io {
        path: dir.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| CorpusError::Io {
            path: dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            ids.push(stem.to_string());
        }
    }
    ids.sort();
    ids.iter().map(|id| load_case(id)).collect()
}

/// Ids of every golden on disk, sorted.
pub fn golden_ids() -> Result<Vec<String>, CorpusError> {
    let dir = corpus_golden_dir();
    let mut ids = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|source| CorpusError::Io {
        path: dir.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| CorpusError::Io {
            path: dir.clone(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(stem) = name.strip_suffix(".layout.json") {
            ids.push(stem.to_string());
        }
    }
    ids.sort();
    Ok(ids)
}

/// Writes a golden, pretty-printed with a trailing newline so the file is a
/// readable diff.
pub fn write_golden(golden: &Golden) -> Result<(), CorpusError> {
    let path = golden_path(&golden.case);
    let mut text = serde_json::to_string_pretty(golden).map_err(|source| CorpusError::Parse {
        path: path.clone(),
        source,
    })?;
    text.push('\n');
    std::fs::write(&path, text).map_err(|source| CorpusError::Io { path, source })
}

#[derive(Debug)]
pub enum CorpusError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for CorpusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Parse { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for CorpusError {}
