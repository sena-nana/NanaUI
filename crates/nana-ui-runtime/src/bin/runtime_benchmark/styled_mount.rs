//! Mounting styled rows across stylesheet sizes (#348). Each row is a row,
//! a text and a button, each with its own class. A sheet of `rules` rules
//! either gives every element one matching rule among many it does not
//! match (`spread`), or spreads all of them over the three classes in use
//! (`all_match`). Rounds interleave the sheets; the first mount of a sheet
//! also resolves its classes and is reported apart.
use super::*;

use nana_ui_runtime::AppContext;
use nana_ui_runtime::view::{
    Class, IntoView, Sheet, SheetRule, StylePatch, button, column, row, text,
};

const ROWS: usize = 4_000;
const APPENDED: usize = 500;
const ROUNDS: usize = 8;

#[derive(Serialize)]
struct StyledMountCase {
    rules: usize,
    shape: &'static str,
    /// Rules each element matches.
    matched_per_element: usize,
    rows: usize,
    nodes: usize,
    first_mount_ms: f64,
    cold_mount_ms: Distribution,
    cold_us_per_row: f64,
    appended_rows: usize,
    append_ms: Distribution,
    append_us_per_row: f64,
}

struct Styled {
    rules: usize,
    shape: &'static str,
    matched: usize,
    classes: [Class; 3],
}

fn sheet(rules: usize, all_match: bool) -> Styled {
    let patches: &'static [StylePatch] = (0..rules)
        .map(|rule| {
            let json = format!(r#"{{"padding":{{"Px":{}.0}}}}"#, rule % 8);
            StylePatch::new(Box::leak(json.into_boxed_str()))
        })
        .collect::<Vec<_>>()
        .leak();
    let classes: &'static [[u16; 1]] = (0..rules)
        .map(|rule| {
            [if all_match {
                (rule % 3) as u16
            } else {
                rule as u16
            }]
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
        shape: if all_match { "all_match" } else { "spread" },
        matched: if all_match { rules.div_ceil(3) } else { 1 },
        classes: [0, 1, 2].map(|index| Class::new(sheet, index)),
    }
}

fn rows(styled: &Styled, from: usize, count: usize) -> Vec<impl IntoView + use<>> {
    let [row_class, text_class, button_class] = styled.classes;
    (from..from + count)
        .map(|index| {
            row().class(row_class).children((
                text(format!("row {index}")).class(text_class),
                button(format!("button {index}")).class(button_class),
            ))
        })
        .collect()
}

pub(super) fn run(document: DocumentId) {
    let styles: Vec<Styled> = [10, 100, 1_000]
        .into_iter()
        .flat_map(|rules| [sheet(rules, false), sheet(rules, true)])
        .collect();
    let mut first = vec![Duration::ZERO; styles.len()];
    let mut cold = vec![Vec::new(); styles.len()];
    let mut append = vec![Vec::new(); styles.len()];
    for round in 0..=ROUNDS {
        // Alternate the order so neither end of the list always runs warm.
        let order: Vec<usize> = if round % 2 == 0 {
            (0..styles.len()).collect()
        } else {
            (0..styles.len()).rev().collect()
        };
        for at in order {
            let styled = &styles[at];
            let mut cx = AppContext::new();
            let started = Instant::now();
            let list = cx
                .mount_view_root(document, || column().children(rows(styled, 0, ROWS)))
                .unwrap()
                .roots()[0];
            let cold_elapsed = started.elapsed();
            let started = Instant::now();
            cx.mount_view(list, || rows(styled, ROWS, APPENDED))
                .unwrap();
            let append_elapsed = started.elapsed();
            assert_eq!(
                cx.world().node(list).unwrap().children.len(),
                ROWS + APPENDED
            );
            if round == 0 {
                first[at] = cold_elapsed;
            } else {
                cold[at].push(cold_elapsed);
                append[at].push(append_elapsed);
            }
        }
    }
    let min_ms = |samples: &[Duration]| {
        samples
            .iter()
            .min()
            .map_or(0.0, |min| min.as_secs_f64() * 1_000.0)
    };
    let cases: Vec<StyledMountCase> = styles
        .iter()
        .enumerate()
        .map(|(at, styled)| StyledMountCase {
            rules: styled.rules,
            shape: styled.shape,
            matched_per_element: styled.matched,
            rows: ROWS,
            nodes: ROWS * 3 + 1,
            first_mount_ms: (first[at].as_secs_f64() * 1_000_000.0).round() / 1_000.0,
            cold_mount_ms: summarize(&cold[at]),
            cold_us_per_row: (min_ms(&cold[at]) * 1_000.0 / ROWS as f64 * 100.0).round() / 100.0,
            appended_rows: APPENDED,
            append_ms: summarize(&append[at]),
            append_us_per_row: (min_ms(&append[at]) * 1_000.0 / APPENDED as f64 * 100.0).round()
                / 100.0,
        })
        .collect();
    let report = serde_json::json!({
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "rounds": ROUNDS,
        "cases": cases,
        "notes": [
            "Generated by: cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark -- --profile-styled-mount",
            "first_mount_ms is a sheet's first mount, which also resolves its class sets; per-row figures use the minimum of the later rounds.",
        ],
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
