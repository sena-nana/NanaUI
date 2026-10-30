//! Mounting styled rows across stylesheet sizes (#348). Each row is a row,
//! a text and a button, each with its own class. A sheet of `rules` rules
//! either gives every element one matching rule among many it does not
//! match (`spread`), spreads all of them over the three classes in use
//! (`all_match`), or does that with every other rule also needing a
//! conditional class each element takes through `class_when` on one signal
//! (`conditional`). For that shape, switching the signal on recomposes every
//! element, and is timed too. Rounds interleave the sheets; the first mount
//! of a sheet also resolves its classes and is reported apart.
use super::*;

use nana_ui_runtime::AppContext;
use nana_ui_runtime::view::{
    Class, IntoView, Sheet, SheetRule, Signal, StylePatch, button, column, row, signal, text,
};

const ROWS: usize = 4_000;
const APPENDED: usize = 500;
const ROUNDS: usize = 8;
/// The class `conditional` sheets switch on.
const DIM: u16 = 3;

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Spread,
    AllMatch,
    Conditional,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Self::Spread => "spread",
            Self::AllMatch => "all_match",
            Self::Conditional => "conditional",
        }
    }
}

#[derive(Serialize)]
struct StyledMountCase {
    rules: usize,
    shape: &'static str,
    /// Rules each element matches, with its conditional class on.
    matched_per_element: usize,
    rows: usize,
    nodes: usize,
    first_mount_ms: f64,
    cold_mount_ms: Distribution,
    cold_us_per_row: f64,
    appended_rows: usize,
    append_ms: Distribution,
    append_us_per_row: f64,
    /// Switching the conditional class on for every element at once.
    #[serde(skip_serializing_if = "Option::is_none")]
    switch_on_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    switch_on_us_per_row: Option<f64>,
}

struct Styled {
    rules: usize,
    shape: Shape,
    matched: usize,
    classes: [Class; 3],
    dim: Class,
}

fn sheet(rules: usize, shape: Shape) -> Styled {
    let patches: &'static [StylePatch] = (0..rules)
        .map(|rule| {
            let json = format!(r#"{{"padding":{{"Px":{}.0}}}}"#, rule % 8);
            StylePatch::new(Box::leak(json.into_boxed_str()))
        })
        .collect::<Vec<_>>()
        .leak();
    let classes: &'static [&'static [u16]] = (0..rules)
        .map(|rule| -> &'static [u16] {
            let class = (rule % 3) as u16;
            match shape {
                Shape::Spread => vec![rule as u16].leak(),
                Shape::AllMatch => vec![class].leak(),
                Shape::Conditional if rule % 2 == 1 => vec![class, DIM].leak(),
                Shape::Conditional => vec![class].leak(),
            }
        })
        .collect::<Vec<_>>()
        .leak();
    let sheet_rules: &'static [SheetRule] = patches
        .iter()
        .zip(classes)
        .map(|(patch, classes)| SheetRule {
            classes,
            patch,
            query: None,
        })
        .collect::<Vec<_>>()
        .leak();
    let sheet: &'static Sheet = Box::leak(Box::new(Sheet::new(sheet_rules, &[])));
    Styled {
        rules,
        shape,
        matched: match shape {
            Shape::Spread => 1,
            Shape::AllMatch | Shape::Conditional => rules.div_ceil(3),
        },
        classes: [0, 1, 2].map(|index| Class::new(sheet, index)),
        dim: Class::new(sheet, DIM),
    }
}

fn rows(
    styled: &Styled,
    dim: Option<Signal<bool>>,
    from: usize,
    count: usize,
) -> Vec<impl IntoView + use<>> {
    let [row_class, text_class, button_class] = styled.classes;
    let dim_class = styled.dim;
    (from..from + count)
        .map(|index| {
            let mut container = row().class(row_class);
            let mut label = text(format!("row {index}")).class(text_class);
            let mut action = button(format!("button {index}")).class(button_class);
            if let Some(dim) = dim {
                container = container.class_when(dim_class, dim);
                label = label.class_when(dim_class, dim);
                action = action.class_when(dim_class, dim);
            }
            container.children((label, action))
        })
        .collect()
}

pub(super) fn run(document: DocumentId) {
    let styles: Vec<Styled> = [10, 100, 1_000]
        .into_iter()
        .flat_map(|rules| {
            [Shape::Spread, Shape::AllMatch, Shape::Conditional].map(|shape| sheet(rules, shape))
        })
        .collect();
    let mut first = vec![Duration::ZERO; styles.len()];
    let mut cold = vec![Vec::new(); styles.len()];
    let mut append = vec![Vec::new(); styles.len()];
    let mut switch_on = vec![Vec::new(); styles.len()];
    for round in 0..=ROUNDS {
        // Alternate the order so neither end of the list always runs warm.
        let order: Vec<usize> = if round % 2 == 0 {
            (0..styles.len()).collect()
        } else {
            (0..styles.len()).rev().collect()
        };
        for at in order {
            let styled = &styles[at];
            let conditional = styled.shape == Shape::Conditional;
            let mut cx = AppContext::new();
            let mut dim = None;
            let started = Instant::now();
            let list = cx
                .mount_view_root(document, || {
                    dim = conditional.then(|| signal(false));
                    column().children(rows(styled, dim, 0, ROWS))
                })
                .unwrap()
                .roots()[0];
            let cold_elapsed = started.elapsed();
            let started = Instant::now();
            cx.mount_view(list, || rows(styled, dim, ROWS, APPENDED))
                .unwrap();
            let append_elapsed = started.elapsed();
            assert_eq!(
                cx.world().node(list).unwrap().children.len(),
                ROWS + APPENDED
            );
            let switch_elapsed = dim.map(|dim| {
                let started = Instant::now();
                dim.set(true);
                cx.flush_reactive().unwrap();
                started.elapsed()
            });
            if round == 0 {
                first[at] = cold_elapsed;
            } else {
                cold[at].push(cold_elapsed);
                append[at].push(append_elapsed);
                switch_on[at].extend(switch_elapsed);
            }
        }
    }
    let us_per_row = |samples: &[Duration], rows: usize| {
        let min_ms = samples
            .iter()
            .min()
            .map_or(0.0, |min| min.as_secs_f64() * 1_000.0);
        (min_ms * 1_000.0 / rows as f64 * 100.0).round() / 100.0
    };
    let cases: Vec<StyledMountCase> = styles
        .iter()
        .enumerate()
        .map(|(at, styled)| {
            let switched = !switch_on[at].is_empty();
            StyledMountCase {
                rules: styled.rules,
                shape: styled.shape.name(),
                matched_per_element: styled.matched,
                rows: ROWS,
                nodes: ROWS * 3 + 1,
                first_mount_ms: (first[at].as_secs_f64() * 1_000_000.0).round() / 1_000.0,
                cold_mount_ms: summarize(&cold[at]),
                cold_us_per_row: us_per_row(&cold[at], ROWS),
                appended_rows: APPENDED,
                append_ms: summarize(&append[at]),
                append_us_per_row: us_per_row(&append[at], APPENDED),
                switch_on_ms: switched.then(|| summarize(&switch_on[at])),
                switch_on_us_per_row: switched.then(|| us_per_row(&switch_on[at], ROWS + APPENDED)),
            }
        })
        .collect();
    let report = serde_json::json!({
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "rounds": ROUNDS,
        "cases": cases,
        "notes": [
            "Generated by: cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark -- --profile-styled-mount",
            "first_mount_ms is a sheet's first mount, which also resolves its class sets; per-row figures use the minimum of the later rounds.",
            "switch_on is one signal switching the conditional class on for all 4,500 rows (13,500 elements), through flush_reactive.",
        ],
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
