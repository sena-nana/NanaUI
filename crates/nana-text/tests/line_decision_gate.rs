//! Issue #214 Gate D: the CJK line decision's work, as counters.
//!
//! A paragraph of 1k and of 10k break opportunities (every ideograph is
//! one): the greedy decision looks at each opportunity a bounded number of
//! times, so 10k costs about ten times 1k; a pretty paragraph's beam stays
//! within its width and its budget; and a box resized two hundred times lays
//! the shaped runs out again two hundred times without shaping once.

mod support;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    NativeTextEngine, TextConstraints, TextEngine, TextKind, TextSource, TextStyle,
    TextWorkCounters,
};
use nana_ui_core::{TextSpacingTrimSpec, TextWrapBreak, TextWrapStyleSpec};
use std::sync::Arc;

use support::corpus::{fixture_bytes, fixture_family};

fn engine() -> NativeTextEngine {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    fonts
        .register_bytes(fixture_bytes("noto-sans-sc"), &FaceDescriptor::default())
        .expect("fixture registers");
    NativeTextEngine::new(fonts)
}

fn style() -> TextStyle {
    TextStyle {
        font_family: Some(Arc::from(format!("\"{}\"", fixture_family("noto-sans-sc")))),
        font_size_px: 16.0,
        ..TextStyle::default()
    }
}

/// `chars` ideographs, a comma every seventh.
fn paragraph(chars: usize) -> String {
    (0..chars)
        .map(|at| {
            if at % 7 == 6 {
                '，'
            } else {
                ['中', '文', '排', '版'][at % 4]
            }
        })
        .collect()
}

fn constraints(width: f32, wrap_style: TextWrapStyleSpec) -> TextConstraints {
    TextConstraints {
        max_width_px: Some(width),
        wrap: Some(TextWrapBreak::Word),
        spacing_trim: TextSpacingTrimSpec::Auto,
        wrap_style,
        ..TextConstraints::default()
    }
}

fn counted(
    engine: &mut NativeTextEngine,
    text: &str,
    constraints: &TextConstraints,
) -> (usize, TextWorkCounters) {
    let source = TextSource::new(text);
    let mut counters = TextWorkCounters::default();
    let layout = engine.layout(
        TextKind::Paragraph,
        &source,
        &style(),
        constraints,
        &mut counters,
    );
    (layout.lines.len(), counters)
}

#[test]
fn greedy_decisions_cost_each_opportunity_a_bounded_number_of_times() {
    let mut work = Vec::new();
    for chars in [1_000usize, 10_000] {
        let text = paragraph(chars);
        let (lines, counters) = counted(
            &mut engine(),
            &text,
            &constraints(330.0, TextWrapStyleSpec::Auto),
        );
        assert!(lines > 1);
        assert!(
            counters.line_opportunities_considered <= 3 * chars,
            "{chars}: {}",
            counters.line_opportunities_considered
        );
        work.push(counters.line_opportunities_considered);
    }
    let ratio = work[1] as f64 / work[0] as f64;
    assert!(ratio <= 10.5, "{work:?}");
}

#[test]
fn a_pretty_paragraph_stays_within_its_beam_and_budget() {
    for chars in [1_000usize, 10_000] {
        let text = paragraph(chars);
        let (lines, counters) = counted(
            &mut engine(),
            &text,
            &constraints(330.0, TextWrapStyleSpec::Pretty),
        );
        assert!(counters.line_beam_states <= 4, "{chars}");
        assert_eq!(counters.line_budget_fallbacks, 0, "{chars}");
        // At most four states, each extended by at most four ends, a line.
        assert!(
            counters.line_break_comparisons <= 16 * (lines + 1),
            "{chars}: {counters:?}"
        );
        let (again, _) = counted(
            &mut engine(),
            &text,
            &constraints(330.0, TextWrapStyleSpec::Pretty),
        );
        assert_eq!(again, lines);
    }
}

#[test]
fn a_resize_lays_out_again_without_shaping() {
    let mut engine = engine();
    let text = paragraph(600);
    counted(
        &mut engine,
        &text,
        &constraints(300.0, TextWrapStyleSpec::Auto),
    );
    let mut relayouts = 0;
    let mut shaped = 0;
    for step in 1..=200 {
        let (_, counters) = counted(
            &mut engine,
            &text,
            &constraints(300.0 + step as f32 * 0.5, TextWrapStyleSpec::Auto),
        );
        relayouts += counters.constraint_only_relayouts;
        shaped += counters.text_nodes_shaped;
    }
    assert_eq!(relayouts, 200);
    assert_eq!(shaped, 0);
}
