//! Committed-baseline gate for the Gallery snapshot suite.
//!
//! The suite used to write a missing baseline from the frame it was supposed to
//! judge, keep it in `target/`, and never look at the difference it computed. A
//! real regression therefore recorded itself as the new truth and reported
//! success. Recording a baseline is now an explicit act ([`Mode::Bless`]) and a
//! mismatch is a non-zero exit.
//!
//! Comparison is exact. Rendering the whole suite twice on one adapter produced
//! byte-identical PNGs, and the offscreen target is single-sampled, so there is
//! no run-to-run noise for a tolerance to absorb. A tolerance would only buy
//! slack across adapters, and no honest threshold separates a software
//! rasteriser's edge noise from a one-step colour-token regression: both are a
//! small delta over a large area. Baselines are therefore keyed by adapter and
//! an unknown adapter fails loudly instead of comparing against a foreign one.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use nana_ui_devtools::agent::PixelDiff;
use nana_ui_devtools::agent::pixels::{pixel_diff, pixel_stats};

use crate::write::{self, Size};

/// Per-channel allowance. See the module note on why it is zero.
pub const TOLERANCE: u8 = 0;

/// How many failing snapshots the report lists before it summarises the rest.
const REPORT_LIMIT: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Compare against the committed baseline; disagreement is a failure.
    Verify,
    /// Record baselines. Empty `prefixes` blesses everything.
    Bless { prefixes: Vec<String> },
}

impl Mode {
    fn blesses(&self, key: &str) -> bool {
        match self {
            Self::Verify => false,
            Self::Bless { prefixes } => {
                prefixes.is_empty() || prefixes.iter().any(|prefix| key.starts_with(prefix))
            }
        }
    }
}

/// What one snapshot did against its baseline.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Matched,
    Recorded,
    Updated,
    Unchanged,
    Missing,
    Unreadable(String),
    SizeChanged { baseline: Size<u32> },
    Changed(PixelDiff),
}

impl Outcome {
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            Self::Missing | Self::Unreadable(_) | Self::SizeChanged { .. } | Self::Changed(_)
        )
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Matched => "MATCH",
            Self::Recorded => "RECORD",
            Self::Updated => "UPDATE",
            Self::Unchanged => "MATCH",
            Self::Missing => "MISSING",
            Self::Unreadable(_) => "UNREADABLE",
            Self::SizeChanged { .. } => "RESIZED",
            Self::Changed(_) => "CHANGED",
        }
    }

    fn detail(&self, size: Size<u32>) -> String {
        match self {
            Self::Missing => "no baseline recorded for this adapter".to_owned(),
            Self::Unreadable(error) => format!("baseline PNG could not be decoded: {error}"),
            Self::SizeChanged { baseline } => format!(
                "{}x{} baseline vs {}x{} render",
                baseline.width, baseline.height, size.width, size.height
            ),
            Self::Changed(diff) => {
                let bbox = diff
                    .bbox
                    .map(|rect| {
                        format!(
                            " bbox=({},{} {}x{})",
                            rect.x as u32, rect.y as u32, rect.width as u32, rect.height as u32
                        )
                    })
                    .unwrap_or_default();
                format!(
                    "changed_pixels={} ({:.4}%) max_channel_delta={}{bbox}",
                    diff.changed_pixels,
                    diff.changed_ratio * 100.0,
                    diff.max_channel_delta
                )
            }
            _ => String::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    key: String,
    outcome: Outcome,
    size: Size<u32>,
    /// `true` when the frame is nothing but the clear colour. Such a snapshot
    /// agrees with its baseline while proving nothing about rendering.
    flat: bool,
}

/// Where the suite renders to, where it compares against, and in which mode.
#[derive(Debug, Clone)]
pub struct Options {
    pub mode: Mode,
    pub output: PathBuf,
    /// Root of the committed baseline tree; the adapter key is a child of it.
    pub baseline_root: PathBuf,
    pub platform: String,
    /// Adapter identity as reported by wgpu, for the report header.
    pub adapter: String,
}

pub struct Recorder {
    options: Options,
    baseline_dir: PathBuf,
    /// Whether the adapter had any baseline at all before this run.
    baseline_existed: bool,
    entries: Vec<Entry>,
    seen: BTreeSet<String>,
    contract_failures: Vec<String>,
}

/// The suite's verdict. `failures` decides the process exit code.
#[derive(Debug, Clone)]
pub struct Report {
    pub summary: String,
    pub failures: usize,
}

impl Recorder {
    pub fn new(options: Options) -> Self {
        let baseline_dir = options.baseline_root.join(&options.platform);
        Self {
            baseline_existed: baseline_dir.is_dir(),
            baseline_dir,
            options,
            entries: Vec::new(),
            seen: BTreeSet::new(),
            contract_failures: Vec::new(),
        }
    }

    /// A fixture whose own `machine_verdict` is `fail`. Reported, not gated:
    /// the backlog predates this gate, and a check that is red regardless of
    /// the change under review teaches people to ignore it.
    pub fn note_contract_failure(&mut self, key: &str) {
        self.contract_failures.push(key.to_owned());
    }

    /// Sibling evidence path for `key`: `a/b/c.png` plus `evidence.txt` becomes
    /// `a/b/c.evidence.txt`, so one snapshot is one name in every tree.
    pub fn sibling(&self, key: &str, suffix: &str) -> PathBuf {
        self.options.output.join(sibling_key(key, suffix))
    }

    /// Write `pixels` as the run's `key` image and judge it against the
    /// baseline. Diagnostic images are written only when the two disagree, so
    /// any `*.difference.png` in the output tree marks a real failure.
    pub fn record(
        &mut self,
        key: &str,
        size: Size<u32>,
        pixels: &[u8],
        clear: [f32; 4],
    ) -> Result<(), Box<dyn Error>> {
        let runtime_path = self.options.output.join(key);
        write::png(&runtime_path, size, pixels)?;
        self.seen.insert(key.to_owned());

        let baseline_path = self.baseline_dir.join(key);
        let baseline = load_baseline(&baseline_path);
        let outcome = if self.options.mode.blesses(key) {
            match &baseline {
                Loaded::Present(baseline_size, baseline_pixels)
                    if *baseline_size == size && baseline_pixels == pixels =>
                {
                    Outcome::Unchanged
                }
                Loaded::Missing => {
                    write::png(&baseline_path, size, pixels)?;
                    Outcome::Recorded
                }
                _ => {
                    write::png(&baseline_path, size, pixels)?;
                    Outcome::Updated
                }
            }
        } else {
            self.compare(key, size, pixels, baseline)?
        };

        let flat = pixel_stats(size, pixels, clear).nonclear_ratio == 0.0;
        self.entries.push(Entry {
            key: key.to_owned(),
            outcome,
            size,
            flat,
        });
        Ok(())
    }

    fn compare(
        &self,
        key: &str,
        size: Size<u32>,
        pixels: &[u8],
        baseline: Loaded,
    ) -> Result<Outcome, Box<dyn Error>> {
        let (baseline_size, baseline_pixels) = match baseline {
            Loaded::Missing => return Ok(Outcome::Missing),
            Loaded::Unreadable(error) => return Ok(Outcome::Unreadable(error)),
            Loaded::Present(size, pixels) => (size, pixels),
        };
        if baseline_size != size {
            self.write_baseline_copy(key, baseline_size, &baseline_pixels)?;
            return Ok(Outcome::SizeChanged {
                baseline: baseline_size,
            });
        }
        let diff = pixel_diff(size, &baseline_pixels, pixels, TOLERANCE)?;
        if diff.changed_pixels == 0 {
            return Ok(Outcome::Matched);
        }
        self.write_baseline_copy(key, size, &baseline_pixels)?;
        write::png(
            &self
                .options
                .output
                .join(sibling_key(key, "side-by-side.png")),
            Size::new(size.width * 2 + SIDE_BY_SIDE_GAP, size.height),
            &crate::render::side_by_side(&baseline_pixels, pixels, size, SIDE_BY_SIDE_GAP),
        )?;
        write::png(
            &self.options.output.join(sibling_key(key, "difference.png")),
            size,
            &crate::render::pixel_difference(&baseline_pixels, pixels),
        )?;
        Ok(Outcome::Changed(diff))
    }

    fn write_baseline_copy(
        &self,
        key: &str,
        size: Size<u32>,
        pixels: &[u8],
    ) -> Result<(), Box<dyn Error>> {
        write::png(
            &self.options.output.join(sibling_key(key, "baseline.png")),
            size,
            pixels,
        )
    }

    /// Baselines under the adapter key that this run did not render. They are a
    /// failure in [`Mode::Verify`] — a stale baseline is a snapshot nobody is
    /// checking any more — and are deleted when blessing.
    fn stale(&self) -> Vec<String> {
        let mut stale = Vec::new();
        collect_pngs(&self.baseline_dir, &self.baseline_dir, &mut stale);
        stale.retain(|key| !self.seen.contains(key));
        stale.sort();
        stale
    }

    pub fn finish(self) -> Result<Report, Box<dyn Error>> {
        let stale = self.stale();
        let blessing = matches!(self.options.mode, Mode::Bless { .. });
        if blessing {
            for key in &stale {
                std::fs::remove_file(self.baseline_dir.join(key))?;
            }
        }

        let mut failures = self
            .entries
            .iter()
            .filter(|entry| entry.outcome.is_failure())
            .count();
        if !blessing {
            failures += stale.len();
        }

        let mut summary = String::new();
        writeln!(summary, "adapter:  {}", self.options.adapter)?;
        writeln!(summary, "platform: {}", self.options.platform)?;
        writeln!(summary, "baseline: {}", self.baseline_dir.display())?;
        writeln!(summary, "output:   {}", self.options.output.display())?;
        writeln!(
            summary,
            "mode:     {}",
            if blessing { "bless" } else { "verify" }
        )?;
        writeln!(summary, "tolerance: {TOLERANCE} (exact)")?;
        writeln!(summary)?;

        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        for entry in &self.entries {
            *counts.entry(entry.outcome.label()).or_default() += 1;
        }
        writeln!(summary, "snapshots: {}", self.entries.len())?;
        for (label, count) in &counts {
            writeln!(summary, "  {label}: {count}")?;
        }
        if !stale.is_empty() {
            writeln!(
                summary,
                "  {}: {}",
                if blessing { "REMOVED" } else { "STALE" },
                stale.len()
            )?;
        }

        let flat = self
            .entries
            .iter()
            .filter(|entry| entry.flat)
            .collect::<Vec<_>>();
        if !flat.is_empty() {
            writeln!(summary)?;
            writeln!(
                summary,
                "{} snapshot(s) painted nothing but the clear colour; they agree with any \
                 baseline recorded from them and prove nothing:",
                flat.len()
            )?;
            for entry in flat.iter().take(REPORT_LIMIT) {
                writeln!(summary, "  FLAT  {}", entry.key)?;
            }
        }

        if !self.contract_failures.is_empty() {
            writeln!(summary)?;
            writeln!(
                summary,
                "{} fixture(s) record `machine_verdict: fail` in their own evidence. That \
                 backlog predates the baseline gate and does not affect this exit code; see \
                 the listed `*.evidence.txt`:",
                self.contract_failures.len()
            )?;
            for key in self.contract_failures.iter().take(REPORT_LIMIT) {
                writeln!(summary, "  CONTRACT  {key}")?;
            }
            if self.contract_failures.len() > REPORT_LIMIT {
                writeln!(
                    summary,
                    "  ... and {} more",
                    self.contract_failures.len() - REPORT_LIMIT
                )?;
            }
        }

        if !self.baseline_existed && !blessing {
            writeln!(summary)?;
            writeln!(
                summary,
                "No baseline exists for adapter `{}`. Baselines are per-adapter because the \
                 comparison is exact; another adapter's PNGs are not a valid reference. Record \
                 this adapter's baseline with `--bless` and commit it, or run on an adapter that \
                 already has one.",
                self.options.platform
            )?;
        } else {
            let mut listed = 0;
            let mut elided = 0;
            let mut body = String::new();
            for entry in self.entries.iter().filter(|e| e.outcome.is_failure()) {
                if listed == REPORT_LIMIT {
                    elided += 1;
                    continue;
                }
                listed += 1;
                writeln!(
                    body,
                    "  {:<10} {}  {}",
                    entry.outcome.label(),
                    entry.key,
                    entry.outcome.detail(entry.size)
                )?;
            }
            for key in &stale {
                if listed == REPORT_LIMIT {
                    elided += 1;
                    continue;
                }
                listed += 1;
                writeln!(
                    body,
                    "  {:<10} {key}  baseline has no matching snapshot",
                    if blessing { "REMOVED" } else { "STALE" }
                )?;
            }
            if !body.is_empty() {
                writeln!(summary)?;
                summary.push_str(&body);
                if elided > 0 {
                    writeln!(summary, "  ... and {elided} more")?;
                }
            }
        }

        if failures > 0 {
            writeln!(summary)?;
            writeln!(
                summary,
                "{failures} snapshot(s) disagree with the baseline. Inspect the \
                 `*.difference.png` and `*.side-by-side.png` files under {}; re-record with \
                 `--bless [PREFIX ...]` once the change is intended.",
                self.options.output.display()
            )?;
        }

        let report_path = self.options.output.join("snapshot-report.txt");
        if let Some(parent) = report_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&report_path, &summary)?;

        Ok(Report { summary, failures })
    }
}

const SIDE_BY_SIDE_GAP: u32 = 8;

enum Loaded {
    Missing,
    Unreadable(String),
    Present(Size<u32>, Vec<u8>),
}

fn load_baseline(path: &Path) -> Loaded {
    if !path.exists() {
        return Loaded::Missing;
    }
    match write::read_png(path) {
        Some((size, pixels)) => Loaded::Present(size, pixels),
        None => Loaded::Unreadable(path.display().to_string()),
    }
}

fn sibling_key(key: &str, suffix: &str) -> String {
    format!("{}.{suffix}", key.strip_suffix(".png").unwrap_or(key))
}

fn collect_pngs(root: &Path, directory: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_pngs(root, &path, found);
        } else if path.extension().is_some_and(|extension| extension == "png")
            && let Ok(relative) = path.strip_prefix(root)
            && let Some(key) = relative.to_str()
        {
            found.push(key.replace('\\', "/"));
        }
    }
}

/// Filesystem-safe adapter identity. Driver and device names are kept in full
/// rather than truncated: a software rasteriser encodes its LLVM version in its
/// name, and a rasteriser upgrade genuinely produces different pixels, so the
/// key changing to one with no baseline is the correct, self-explaining outcome.
pub fn platform_key(backend: &str, adapter: &str) -> String {
    let mut key = String::new();
    for character in format!("{backend}-{adapter}").chars() {
        if character.is_ascii_alphanumeric() {
            key.push(character.to_ascii_lowercase());
        } else if !key.ends_with('-') {
            key.push('-');
        }
    }
    key.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const SIZE: Size<u32> = Size::new(4, 4);

    fn scratch(name: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nana-ui-snapshot-baseline-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn options(root: &Path, mode: Mode) -> Options {
        Options {
            mode,
            output: root.join("out"),
            baseline_root: root.join("snapshots"),
            platform: "test-adapter".to_owned(),
            adapter: "test adapter".to_owned(),
        }
    }

    fn frame(marked: usize) -> Vec<u8> {
        let mut pixels = [0, 0, 0, 255].repeat(16);
        for index in 0..marked {
            pixels[index * 4..index * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        pixels
    }

    #[test]
    fn a_snapshot_without_a_baseline_fails_instead_of_recording_itself() {
        let root = scratch("missing");
        let mut recorder = Recorder::new(options(&root, Mode::Verify));
        recorder
            .record("panel.png", SIZE, &frame(1), CLEAR)
            .expect("record");
        let report = recorder.finish().expect("finish");

        assert_eq!(report.failures, 1);
        assert!(
            !root.join("snapshots/test-adapter/panel.png").exists(),
            "verify must not write a baseline it was supposed to judge against"
        );
    }

    #[test]
    fn blessing_records_a_baseline_that_a_later_run_matches() {
        let root = scratch("bless");
        let mut recorder = Recorder::new(options(
            &root,
            Mode::Bless {
                prefixes: Vec::new(),
            },
        ));
        recorder
            .record("panel.png", SIZE, &frame(1), CLEAR)
            .expect("record");
        assert_eq!(recorder.finish().expect("finish").failures, 0);
        assert!(root.join("snapshots/test-adapter/panel.png").exists());

        let mut recorder = Recorder::new(options(&root, Mode::Verify));
        recorder
            .record("panel.png", SIZE, &frame(1), CLEAR)
            .expect("record");
        let report = recorder.finish().expect("finish");
        assert_eq!(report.failures, 0);
        assert!(
            !root.join("out/panel.difference.png").exists(),
            "a match must not leave diagnostic images behind"
        );
    }

    #[test]
    fn a_single_changed_pixel_fails_and_leaves_a_difference_image() {
        let root = scratch("changed");
        let mut recorder = Recorder::new(options(
            &root,
            Mode::Bless {
                prefixes: Vec::new(),
            },
        ));
        recorder
            .record("panel.png", SIZE, &frame(1), CLEAR)
            .expect("record");
        recorder.finish().expect("finish");

        let mut recorder = Recorder::new(options(&root, Mode::Verify));
        recorder
            .record("panel.png", SIZE, &frame(2), CLEAR)
            .expect("record");
        let report = recorder.finish().expect("finish");

        assert_eq!(report.failures, 1);
        assert!(root.join("out/panel.difference.png").exists());
        assert!(root.join("out/panel.side-by-side.png").exists());
        assert!(root.join("out/panel.baseline.png").exists());
    }

    #[test]
    fn a_resized_snapshot_fails_rather_than_comparing_different_geometry() {
        let root = scratch("resized");
        let mut recorder = Recorder::new(options(
            &root,
            Mode::Bless {
                prefixes: Vec::new(),
            },
        ));
        recorder
            .record("panel.png", SIZE, &frame(1), CLEAR)
            .expect("record");
        recorder.finish().expect("finish");

        let mut recorder = Recorder::new(options(&root, Mode::Verify));
        recorder
            .record("panel.png", Size::new(2, 2), &frame(1)[..16], CLEAR)
            .expect("record");
        assert_eq!(recorder.finish().expect("finish").failures, 1);
    }

    #[test]
    fn a_baseline_no_snapshot_renders_any_more_fails_and_bless_removes_it() {
        let root = scratch("stale");
        let mut recorder = Recorder::new(options(
            &root,
            Mode::Bless {
                prefixes: Vec::new(),
            },
        ));
        recorder
            .record("panel.png", SIZE, &frame(1), CLEAR)
            .expect("record");
        recorder
            .record("nested/gone.png", SIZE, &frame(1), CLEAR)
            .expect("record");
        recorder.finish().expect("finish");

        let mut recorder = Recorder::new(options(&root, Mode::Verify));
        recorder
            .record("panel.png", SIZE, &frame(1), CLEAR)
            .expect("record");
        assert_eq!(recorder.finish().expect("finish").failures, 1);

        let mut recorder = Recorder::new(options(
            &root,
            Mode::Bless {
                prefixes: Vec::new(),
            },
        ));
        recorder
            .record("panel.png", SIZE, &frame(1), CLEAR)
            .expect("record");
        assert_eq!(recorder.finish().expect("finish").failures, 0);
        assert!(!root.join("snapshots/test-adapter/nested/gone.png").exists());
    }

    #[test]
    fn a_bless_prefix_leaves_snapshots_outside_it_under_verification() {
        let root = scratch("prefix");
        let mut recorder = Recorder::new(options(
            &root,
            Mode::Bless {
                prefixes: Vec::new(),
            },
        ));
        for key in ["kept.png", "moved/panel.png"] {
            recorder
                .record(key, SIZE, &frame(1), CLEAR)
                .expect("record");
        }
        recorder.finish().expect("finish");

        let mut recorder = Recorder::new(options(
            &root,
            Mode::Bless {
                prefixes: vec!["moved/".to_owned()],
            },
        ));
        recorder
            .record("kept.png", SIZE, &frame(3), CLEAR)
            .expect("record");
        recorder
            .record("moved/panel.png", SIZE, &frame(3), CLEAR)
            .expect("record");
        let report = recorder.finish().expect("finish");

        assert_eq!(
            report.failures, 1,
            "a filtered bless must still verify everything outside the filter"
        );
        let (_, kept) = write::read_png(&root.join("snapshots/test-adapter/kept.png")).unwrap();
        assert_eq!(kept, frame(1), "kept.png was outside the bless prefix");
        let (_, moved) =
            write::read_png(&root.join("snapshots/test-adapter/moved/panel.png")).unwrap();
        assert_eq!(moved, frame(3));
    }

    #[test]
    fn a_frame_of_nothing_but_the_clear_colour_is_reported_as_flat() {
        let root = scratch("flat");
        let mut recorder = Recorder::new(options(
            &root,
            Mode::Bless {
                prefixes: Vec::new(),
            },
        ));
        recorder
            .record("empty.png", SIZE, &frame(0), CLEAR)
            .expect("record");
        assert!(recorder.finish().expect("finish").summary.contains("FLAT"));
    }

    #[test]
    fn adapter_keys_are_filesystem_safe_and_keep_the_driver_version() {
        assert_eq!(platform_key("Metal", "Apple M4 Pro"), "metal-apple-m4-pro");
        assert_eq!(
            platform_key("Vulkan", "llvmpipe (LLVM 19.1.7, 256 bits)"),
            "vulkan-llvmpipe-llvm-19-1-7-256-bits"
        );
    }
}
