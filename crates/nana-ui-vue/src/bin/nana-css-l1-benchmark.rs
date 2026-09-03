//! L1 CSS parse / cascade / layout / paint-parse timings.
//! Complements `nana-vue-runtime-benchmark` (no stylesheet). Not a #8 gate.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use nana_ui_vue::{
    LayoutNode, LayoutStyle, LayoutStyleCss, MatchContext, MatchNode, StyleRule,
    apply_stylesheet_to_layout, measure_layout, parse_stylesheet,
};
use serde::Serialize;

const FRAME_BUDGET_MS: f64 = 16.67;
const WARMUP: usize = 20;
const ITERATIONS: usize = 80;
const TREE_CARDS: usize = 128;

const CARD_CSS: &str = concat!(
    "display:flex;flex-direction:column;",
    "width:calc(min(100%, max(40px, 20%)));",
    "height:calc(clamp(24px, 10%, 80px));",
    "min-height:calc(18 * min(10px, 20px));",
    "aspect-ratio:16/9;"
);
const NOTE_CSS: &str =
    "display:block;width:200px;font-size:16px;line-height:20px;white-space:pre-wrap;";
const PAINT_CSS: &str = concat!(
    "background-image:url(a.png);",
    "background-position:left 10px top 20px;",
    "filter:invert(50%) opacity(0.8);",
    "mask-image:url(mask.png);",
    "clip-path:circle(40%);",
    "background-repeat:space;"
);
fn l1_stylesheet() -> &'static str {
    static SHEET: OnceLock<String> = OnceLock::new();
    SHEET.get_or_init(|| {
        format!(
            ".card{{{CARD_CSS}}}.note{{{NOTE_CSS}}}.hero{{{PAINT_CSS}}}\
.hero-3{{background-image:url(b.png);background-position:right calc(8px) center;}}\
input:checked{{width:18px;}}span:empty{{min-height:8px;}}\
p:nth-of-type(odd){{padding:4px;}}p:nth-of-type(even){{padding:2px;}}\
.shell:focus-within{{border-width:1px;}}"
        )
    })
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    profile: &'static str,
    frame_budget_ms: f64,
    warmup_iterations: usize,
    iterations: usize,
    cases: Vec<Case>,
}

#[derive(Serialize)]
struct Case {
    id: &'static str,
    kind: &'static str,
    warmup_iterations: usize,
    iterations: usize,
    ms: Distribution,
}

#[derive(Serialize)]
struct Distribution {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    frame_budget_ms: f64,
    frame_budget_misses: usize,
}

fn main() {
    let cases = vec![
        bench("parse-stylesheet", "parse", parse_once),
        bench(
            "cascade-checked-empty-of-type-focus-within",
            "cascade",
            cascade_once,
        ),
        bench("layout-calc-aspect-ratio-prewrap", "layout", layout_once),
        bench(
            "paint-parse-position-filter-mask-clip",
            "paint-parse",
            paint_parse_once,
        ),
    ];
    write_report(&Report {
        schema_version: 1,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        frame_budget_ms: FRAME_BUDGET_MS,
        warmup_iterations: WARMUP,
        iterations: ITERATIONS,
        cases,
    });
}

fn bench(id: &'static str, kind: &'static str, mut body: impl FnMut()) -> Case {
    let mut samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP + ITERATIONS) {
        let started = Instant::now();
        body();
        let elapsed = started.elapsed();
        if iteration >= WARMUP {
            samples.push(elapsed);
        }
    }
    Case {
        id,
        kind,
        warmup_iterations: WARMUP,
        iterations: ITERATIONS,
        ms: summarize(&samples),
    }
}

fn large_stylesheet() -> &'static str {
    static SHEET: OnceLock<String> = OnceLock::new();
    SHEET.get_or_init(|| l1_stylesheet().repeat(32))
}

fn shared_rules() -> &'static [StyleRule] {
    static RULES: OnceLock<Vec<StyleRule>> = OnceLock::new();
    RULES.get_or_init(|| parse_stylesheet(l1_stylesheet(), 0))
}

fn parse_once() {
    let rules = parse_stylesheet(black_box(large_stylesheet()), 0);
    assert!(!rules.is_empty());
    black_box(rules);
}

fn node<'a>(
    tag: &'a str,
    classes: &'a [String],
    attrs: &'a BTreeMap<String, String>,
    is_empty: bool,
    checked: bool,
) -> MatchNode<'a> {
    MatchNode {
        tag,
        id: "",
        classes,
        attrs,
        is_empty,
        checked,
    }
}

fn ctx<'a>(
    tag: &'a str,
    classes: &'a [String],
    attrs: &'a BTreeMap<String, String>,
    ancestors: &'a [MatchNode<'a>],
    preceding_siblings: &'a [MatchNode<'a>],
    sibling_index: usize,
    sibling_count: usize,
    of_type_index: usize,
    of_type_count: usize,
    focus_within: bool,
    is_empty: bool,
    checked: bool,
) -> MatchContext<'a> {
    MatchContext {
        tag,
        id: "",
        classes,
        attrs,
        ancestors,
        preceding_siblings,
        sibling_index,
        sibling_count,
        of_type_index,
        of_type_count,
        has_bits: 0,
        has_args: &[],
        focus_within,
        is_empty,
        checked,
        media: Default::default(),
        children: &[],
        following_siblings: &[],
        all_siblings: &[],
        ancestor_subtrees: &[],
        owned_children: &[],
        owned_following: &[],
        owned_ancestor_trees: &[],
        relative: None,
        relative_id: 0,
    }
}

fn cascade_once() {
    let rules = shared_rules();
    let empty_attrs = BTreeMap::new();
    let none: [String; 0] = [];
    let shell = ["shell".to_string()];
    let card = ["card".to_string()];
    let mut applied = 0usize;
    for i in 0..TREE_CARDS {
        let parent = node("div", &shell, &empty_attrs, false, false);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut layout,
            rules,
            &ctx(
                "div",
                &shell,
                &empty_attrs,
                &[],
                &[],
                0,
                1,
                0,
                1,
                i % 7 == 0,
                false,
                false,
            ),
            Some(400.0),
            Some(800.0),
        );
        applied += 1;

        let mut input = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut input,
            rules,
            &ctx(
                "input",
                &none,
                &empty_attrs,
                std::slice::from_ref(&parent),
                &[],
                0,
                4,
                0,
                1,
                false,
                true,
                i % 2 == 0,
            ),
            None,
            None,
        );
        applied += 1;

        let mut empty_span = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut empty_span,
            rules,
            &ctx(
                "span",
                &none,
                &empty_attrs,
                std::slice::from_ref(&parent),
                &[],
                1,
                4,
                0,
                2,
                false,
                true,
                false,
            ),
            None,
            None,
        );
        applied += 1;

        let prev_empty = node("span", &none, &empty_attrs, true, false);
        let mut p = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut p,
            rules,
            &ctx(
                "p",
                &card,
                &empty_attrs,
                std::slice::from_ref(&parent),
                std::slice::from_ref(&prev_empty),
                2,
                4,
                i % 2,
                2,
                false,
                false,
                false,
            ),
            Some(400.0),
            Some(800.0),
        );
        applied += 1;
        black_box((layout, input, empty_span, p));
    }
    assert_eq!(applied, TREE_CARDS * 4);
}

fn layout_once() {
    let mut children = Vec::with_capacity(TREE_CARDS);
    for i in 0..TREE_CARDS {
        if i % 2 == 0 {
            let mut style = LayoutStyle::default();
            style.apply_css_text(CARD_CSS, Some(400.0), Some(800.0));
            children.push(LayoutNode::leaf(format!("box-{i}"), style));
        } else {
            let mut style = LayoutStyle::default();
            style.apply_css_text(NOTE_CSS, None, None);
            let mut node = LayoutNode::leaf(format!("note-{i}"), style);
            node.text = Some("ab\ncd efghijklmnopqrstuvwxyz".into());
            children.push(node);
        }
    }
    let mut root_style = LayoutStyle::default();
    root_style.apply_css_text(
        "display:flex;flex-direction:column;width:400px;",
        None,
        None,
    );
    let root = LayoutNode::with_children("root", root_style, children);
    let boxes = measure_layout(&root, 400.0, 800.0);
    assert!(boxes.len() > TREE_CARDS);
    black_box(boxes);
}

fn paint_parse_once() {
    for _ in 0..256 {
        let mut style = LayoutStyle::default();
        style.apply_css_text(black_box(PAINT_CSS), None, None);
        black_box(style);
    }
}

fn elapsed_ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1_000_000.0).round() / 1_000.0
}

fn summarize(samples: &[Duration]) -> Distribution {
    let mut values = samples.iter().copied().map(elapsed_ms).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    Distribution {
        p50: percentile(&values, 0.50),
        p95: percentile(&values, 0.95),
        p99: percentile(&values, 0.99),
        max: values.last().copied().unwrap_or(0.0),
        frame_budget_ms: FRAME_BUDGET_MS,
        frame_budget_misses: values
            .iter()
            .filter(|value| **value > FRAME_BUDGET_MS)
            .count(),
    }
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    values[((values.len() - 1) as f64 * percentile).round() as usize]
}

fn write_report(report: &Report) {
    let json = serde_json::to_string_pretty(report).expect("benchmark report must serialize");
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next() {
        None => println!("{json}"),
        Some(flag) if flag == "--output" => {
            let path = std::path::PathBuf::from(
                arguments
                    .next()
                    .expect("--output requires a destination path"),
            );
            assert!(arguments.next().is_none(), "unexpected benchmark arguments");
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).expect("benchmark directory must be writable");
            }
            std::fs::write(&path, format!("{json}\n"))
                .expect("benchmark destination must be writable");
            println!("{}", path.display());
        }
        Some(argument) => panic!(
            "unsupported argument `{}`; expected --output <path>",
            argument.to_string_lossy()
        ),
    }
}
