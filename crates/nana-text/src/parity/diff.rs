//! The one structural diff. Positional, and it stops descending once the
//! counts disagree.

use crate::edit::{CaretGeometry, HitTestResult};
use crate::layout::{LineBox, TextLayout, TextRect};
use crate::shape::{ShapedGlyph, ShapedRun};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as _;

/// Where in a layout a difference sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaScope {
    Layout,
    Line(u32),
    Run { line: u32, run: u32 },
    Glyph { line: u32, run: u32, glyph: u32 },
    HitTest(u32),
    Caret(u32),
}

impl DeltaScope {
    fn label(&self) -> String {
        match *self {
            Self::Layout => "layout".to_string(),
            Self::Line(line) => format!("line {line}"),
            Self::Run { line, run } => format!("line {line} run {run}"),
            Self::Glyph { line, run, glyph } => format!("line {line} run {run} glyph {glyph}"),
            Self::HitTest(index) => format!("hit-test {index}"),
            Self::Caret(index) => format!("caret {index}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeltaValue {
    Int(i64),
    Float(f64),
    Text(String),
    /// The side did not have this item at all.
    Missing,
}

impl std::fmt::Display for DeltaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value:.2}"),
            Self::Text(value) => write!(f, "{value}"),
            Self::Missing => write!(f, "<missing>"),
        }
    }
}

/// One field that did not match.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutDelta {
    pub scope: DeltaScope,
    pub field: &'static str,
    pub expected: DeltaValue,
    pub actual: DeltaValue,
    /// `None` for non-numeric fields.
    pub delta: Option<f64>,
    /// The tolerance that was applied, when one was.
    pub tolerance: Option<f32>,
}

/// The result of comparing one corpus case.
#[derive(Debug, Clone)]
pub struct ParityReport {
    pub case_id: String,
    pub deltas: Vec<LayoutDelta>,
}

impl ParityReport {
    pub fn ok(&self) -> bool {
        self.deltas.is_empty()
    }
}

/// Per-field float tolerances, in physical px.
///
/// Everything not listed here is compared exactly: glyph ids, clusters, flags,
/// BiDi levels, directions, scripts, break causes, overflow flags, caret bytes
/// and every count.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tolerances {
    pub advance_px: f32,
    pub offset_px: f32,
    pub baseline_px: f32,
    pub line_height_px: f32,
    pub bounds_px: f32,
    pub caret_x_px: f32,
}

/// 0.05 px for shaped quantities, 0.5 px for derived ones.
///
/// f32 noise at a 24 px body size is around 2e-6 px, so 0.05 sits four orders
/// of magnitude above the noise floor and will not flake; it is also a fifth of
/// the 0.25 px subpixel quantum a glyph atlas quantizes to, so a real change in
/// advance rounding still fails. `bounds` and `caret_x` are derived from the
/// per-glyph numbers and two engines legitimately disagree on whether bounds
/// use advance width or inked width, so those get 0.5 px and stay a sanity
/// check rather than a contract.
pub const DEFAULT_TOLERANCES: Tolerances = Tolerances {
    advance_px: 0.05,
    offset_px: 0.05,
    baseline_px: 0.05,
    line_height_px: 0.05,
    bounds_px: 0.5,
    caret_x_px: 0.5,
};

impl Default for Tolerances {
    fn default() -> Self {
        DEFAULT_TOLERANCES
    }
}

const BOUNDS_FIELDS: [&str; 4] = ["bounds.x", "bounds.y", "bounds.width", "bounds.height"];

struct Diff {
    tol: Tolerances,
    deltas: Vec<LayoutDelta>,
    /// Expected font handle -> actual font handle, built as runs are walked,
    /// plus its inverse so the mapping is checked for being a bijection.
    /// Fonts are compared as an equivalence class within the layout, never as
    /// raw ids: a change in face registration order is not a text behaviour.
    fonts: HashMap<u32, u32>,
    fonts_inverse: HashMap<u32, u32>,
}

impl Diff {
    fn new(tol: Tolerances) -> Self {
        Self {
            tol,
            deltas: Vec::new(),
            fonts: HashMap::new(),
            fonts_inverse: HashMap::new(),
        }
    }

    fn exact<T: PartialEq + std::fmt::Debug>(
        &mut self,
        scope: DeltaScope,
        field: &'static str,
        expected: &T,
        actual: &T,
    ) {
        if expected != actual {
            self.deltas.push(LayoutDelta {
                scope,
                field,
                expected: DeltaValue::Text(format!("{expected:?}")),
                actual: DeltaValue::Text(format!("{actual:?}")),
                delta: None,
                tolerance: None,
            });
        }
    }

    fn count(
        &mut self,
        scope: DeltaScope,
        field: &'static str,
        expected: usize,
        actual: usize,
    ) -> bool {
        if expected != actual {
            self.deltas.push(LayoutDelta {
                scope,
                field,
                expected: DeltaValue::Int(expected as i64),
                actual: DeltaValue::Int(actual as i64),
                delta: Some(actual as f64 - expected as f64),
                tolerance: None,
            });
            return false;
        }
        true
    }

    fn int(&mut self, scope: DeltaScope, field: &'static str, expected: u64, actual: u64) {
        if expected != actual {
            self.deltas.push(LayoutDelta {
                scope,
                field,
                expected: DeltaValue::Int(expected as i64),
                actual: DeltaValue::Int(actual as i64),
                delta: Some(actual as f64 - expected as f64),
                tolerance: None,
            });
        }
    }

    fn near(
        &mut self,
        scope: DeltaScope,
        field: &'static str,
        expected: f32,
        actual: f32,
        tolerance: f32,
    ) {
        let delta = (actual - expected) as f64;
        let diff = (actual - expected).abs();
        // A NaN on either side is itself a finding, so it must not slip through
        // a comparison that is false for NaN.
        if diff.is_nan() || diff > tolerance {
            self.deltas.push(LayoutDelta {
                scope,
                field,
                expected: DeltaValue::Float(expected as f64),
                actual: DeltaValue::Float(actual as f64),
                delta: Some(delta),
                tolerance: Some(tolerance),
            });
        }
    }

    fn rect(
        &mut self,
        scope: DeltaScope,
        fields: [&'static str; 4],
        expected: TextRect,
        actual: TextRect,
    ) {
        let tol = self.tol.bounds_px;
        self.near(scope, fields[0], expected.x, actual.x, tol);
        self.near(scope, fields[1], expected.y, actual.y, tol);
        self.near(scope, fields[2], expected.width, actual.width, tol);
        self.near(scope, fields[3], expected.height, actual.height, tol);
    }

    /// Fonts match when the *partition* of runs into faces matches, not when the
    /// raw handles do.
    ///
    /// The map has to be a bijection in both directions: one expected face
    /// splitting across two actual faces and two expected faces merging into
    /// one are both behaviour changes, and only checking one direction would
    /// miss the merge.
    fn font(&mut self, scope: DeltaScope, expected: &ShapedRun, actual: &ShapedRun) {
        let want = expected.font.index();
        let got = actual.font.index();
        if let Some(&bound) = self.fonts.get(&want)
            && bound != got
        {
            self.deltas.push(LayoutDelta {
                scope,
                field: "font_class",
                expected: DeltaValue::Int(bound as i64),
                actual: DeltaValue::Int(got as i64),
                delta: None,
                tolerance: None,
            });
            return;
        }
        if let Some(&bound) = self.fonts_inverse.get(&got)
            && bound != want
        {
            self.deltas.push(LayoutDelta {
                scope,
                field: "font_class",
                expected: DeltaValue::Int(want as i64),
                actual: DeltaValue::Int(bound as i64),
                delta: None,
                tolerance: None,
            });
            return;
        }
        self.fonts.insert(want, got);
        self.fonts_inverse.insert(got, want);
    }

    fn glyph(&mut self, scope: DeltaScope, expected: &ShapedGlyph, actual: &ShapedGlyph) {
        self.int(
            scope,
            "glyph_id",
            expected.glyph_id as u64,
            actual.glyph_id as u64,
        );
        self.int(
            scope,
            "cluster",
            expected.cluster as u64,
            actual.cluster as u64,
        );
        self.int(
            scope,
            "cluster_end",
            expected.cluster_end as u64,
            actual.cluster_end as u64,
        );
        self.exact(scope, "flags", &expected.flags, &actual.flags);
        let advance = self.tol.advance_px;
        let offset = self.tol.offset_px;
        self.near(
            scope,
            "advance_px",
            expected.advance_px,
            actual.advance_px,
            advance,
        );
        self.near(
            scope,
            "advance_y_px",
            expected.advance_y_px,
            actual.advance_y_px,
            advance,
        );
        self.near(
            scope,
            "offset_x_px",
            expected.offset_x_px,
            actual.offset_x_px,
            offset,
        );
        self.near(
            scope,
            "offset_y_px",
            expected.offset_y_px,
            actual.offset_y_px,
            offset,
        );
    }

    fn run(&mut self, line: u32, index: u32, expected: &ShapedRun, actual: &ShapedRun) {
        let scope = DeltaScope::Run { line, run: index };
        self.exact(scope, "source", &expected.source, &actual.source);
        self.exact(scope, "direction", &expected.direction, &actual.direction);
        self.int(
            scope,
            "bidi_level",
            expected.bidi_level as u64,
            actual.bidi_level as u64,
        );
        self.exact(scope, "script", &expected.script, &actual.script);
        self.font(scope, expected, actual);
        let advance = self.tol.advance_px;
        let offset = self.tol.offset_px;
        let baseline = self.tol.baseline_px;
        self.near(
            scope,
            "font_size_px",
            expected.font_size_px,
            actual.font_size_px,
            advance,
        );
        self.near(
            scope,
            "advance_px",
            expected.advance_px,
            actual.advance_px,
            advance,
        );
        self.near(
            scope,
            "origin_x_px",
            expected.origin_x_px,
            actual.origin_x_px,
            offset,
        );
        self.near(
            scope,
            "metrics.ascent_px",
            expected.metrics.ascent_px,
            actual.metrics.ascent_px,
            baseline,
        );
        self.near(
            scope,
            "metrics.descent_px",
            expected.metrics.descent_px,
            actual.metrics.descent_px,
            baseline,
        );
        self.near(
            scope,
            "metrics.line_gap_px",
            expected.metrics.line_gap_px,
            actual.metrics.line_gap_px,
            baseline,
        );

        // Counts before descending: one extra glyph would otherwise produce a
        // page of misaligned per-glyph deltas and an unreadable report.
        if !self.count(
            scope,
            "glyph_count",
            expected.glyphs.len(),
            actual.glyphs.len(),
        ) {
            return;
        }
        for (glyph_index, (want, got)) in expected.glyphs.iter().zip(&actual.glyphs).enumerate() {
            self.glyph(
                DeltaScope::Glyph {
                    line,
                    run: index,
                    glyph: glyph_index as u32,
                },
                want,
                got,
            );
        }
    }

    fn line(
        &mut self,
        index: u32,
        expected: &TextLayout,
        actual: &TextLayout,
        want: &LineBox,
        got: &LineBox,
    ) {
        let scope = DeltaScope::Line(index);
        self.int(scope, "index", want.index as u64, got.index as u64);
        self.exact(scope, "source", &want.source, &got.source);
        self.exact(scope, "break_cause", &want.break_cause, &got.break_cause);
        self.exact(
            scope,
            "base_direction",
            &want.base_direction,
            &got.base_direction,
        );
        let baseline = self.tol.baseline_px;
        let height = self.tol.line_height_px;
        let advance = self.tol.advance_px;
        self.near(
            scope,
            "metrics.baseline_y_px",
            want.metrics.baseline_y_px,
            got.metrics.baseline_y_px,
            baseline,
        );
        self.near(
            scope,
            "metrics.top_y_px",
            want.metrics.top_y_px,
            got.metrics.top_y_px,
            baseline,
        );
        self.near(
            scope,
            "metrics.height_px",
            want.metrics.height_px,
            got.metrics.height_px,
            height,
        );
        self.near(
            scope,
            "metrics.ascent_px",
            want.metrics.ascent_px,
            got.metrics.ascent_px,
            baseline,
        );
        self.near(
            scope,
            "metrics.descent_px",
            want.metrics.descent_px,
            got.metrics.descent_px,
            baseline,
        );
        self.near(
            scope,
            "metrics.width_px",
            want.metrics.width_px,
            got.metrics.width_px,
            advance,
        );
        self.rect(scope, BOUNDS_FIELDS, want.bounds, got.bounds);

        let want_runs = expected.line_runs(want);
        let got_runs = actual.line_runs(got);
        if !self.count(scope, "run_count", want_runs.len(), got_runs.len()) {
            return;
        }
        for (run_index, (want_run, got_run)) in want_runs.iter().zip(got_runs).enumerate() {
            self.run(index, run_index as u32, want_run, got_run);
        }
    }
}

/// Compares two layouts structurally.
///
/// Engine bookkeeping — the layout id, the source revision and the font
/// generation — is deliberately **not** compared: those describe where a layout
/// came from, not what it says, and two engines have no reason to agree on them.
pub fn compare(expected: &TextLayout, actual: &TextLayout, tol: &Tolerances) -> Vec<LayoutDelta> {
    let mut diff = Diff::new(*tol);
    let scope = DeltaScope::Layout;
    diff.exact(scope, "kind", &expected.kind, &actual.kind);
    diff.exact(scope, "overflow", &expected.overflow, &actual.overflow);
    diff.rect(scope, BOUNDS_FIELDS, expected.bounds, actual.bounds);
    diff.count(scope, "run_count", expected.runs.len(), actual.runs.len());
    if diff.count(
        scope,
        "line_count",
        expected.lines.len(),
        actual.lines.len(),
    ) {
        for (index, (want, got)) in expected.lines.iter().zip(&actual.lines).enumerate() {
            diff.line(index as u32, expected, actual, want, got);
        }
    }

    // The font partition must have the same cardinality on both sides.
    let want_fonts = distinct_fonts(expected);
    let got_fonts = distinct_fonts(actual);
    diff.count(scope, "distinct_font_count", want_fonts, got_fonts);
    diff.deltas
}

fn distinct_fonts(layout: &TextLayout) -> usize {
    let mut seen: Vec<u32> = layout.runs.iter().map(|run| run.font.index()).collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

/// Compares a layout plus its derived caret answers against a golden.
pub fn compare_golden(
    expected: &super::corpus::Golden,
    actual: &TextLayout,
    actual_hit_tests: &[HitTestResult],
    actual_carets: &[Option<CaretGeometry>],
    tol: &Tolerances,
) -> Vec<LayoutDelta> {
    let mut deltas = compare(&expected.layout, actual, tol);
    let mut diff = Diff::new(*tol);

    if diff.count(
        DeltaScope::Layout,
        "hit_test_count",
        expected.hit_tests.len(),
        actual_hit_tests.len(),
    ) {
        for (index, (want, got)) in expected.hit_tests.iter().zip(actual_hit_tests).enumerate() {
            let scope = DeltaScope::HitTest(index as u32);
            diff.int(
                scope,
                "caret.byte",
                want.caret.byte as u64,
                got.caret.byte as u64,
            );
            diff.int(
                scope,
                "caret.line",
                want.caret.line as u64,
                got.caret.line as u64,
            );
            diff.exact(
                scope,
                "caret.affinity",
                &want.caret.affinity,
                &got.caret.affinity,
            );
            diff.exact(scope, "inside", &want.inside, &got.inside);
        }
    }

    if diff.count(
        DeltaScope::Layout,
        "caret_count",
        expected.carets.len(),
        actual_carets.len(),
    ) {
        for (index, (want, got)) in expected.carets.iter().zip(actual_carets).enumerate() {
            let scope = DeltaScope::Caret(index as u32);
            match (want, got) {
                (Some(want), Some(got)) => {
                    let caret_x = diff.tol.caret_x_px;
                    let baseline = diff.tol.baseline_px;
                    let height = diff.tol.line_height_px;
                    diff.near(scope, "x_px", want.x_px, got.x_px, caret_x);
                    diff.near(scope, "top_y_px", want.top_y_px, got.top_y_px, baseline);
                    diff.near(scope, "height_px", want.height_px, got.height_px, height);
                    diff.exact(scope, "direction", &want.direction, &got.direction);
                }
                (None, None) => {}
                (want, got) => {
                    diff.deltas.push(LayoutDelta {
                        scope,
                        field: "resolved",
                        expected: value_or_missing(want.is_some()),
                        actual: value_or_missing(got.is_some()),
                        delta: None,
                        tolerance: None,
                    });
                }
            }
        }
    }

    deltas.extend(diff.deltas);
    deltas
}

fn value_or_missing(resolved: bool) -> DeltaValue {
    if resolved {
        DeltaValue::Text("resolved".to_string())
    } else {
        DeltaValue::Missing
    }
}

/// One-line summary plus one line per delta, matching `tools/css-parity`'s
/// report shape.
pub fn format_report(report: &ParityReport) -> String {
    if report.ok() {
        return format!("{} OK", report.case_id);
    }
    let mut out = format!("{} FAIL ({} deltas)", report.case_id, report.deltas.len());
    for delta in &report.deltas {
        let _ = write!(
            out,
            "\n  {} {}: expected {} got {}",
            delta.scope.label(),
            delta.field,
            delta.expected,
            delta.actual
        );
        if let (Some(value), Some(tolerance)) = (delta.delta, delta.tolerance) {
            let _ = write!(out, " (Δ={value:.2}, tol={tolerance:.2})");
        }
    }
    out
}
