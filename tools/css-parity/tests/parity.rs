//! CSS layout parity：优先用例与回归合同门禁。
//!
//! 所有 `status = Pass` 的 fixture 由 `all_pass_fixtures_succeed` 统一跑通；
//! 下面仅保留带额外结构断言的定点回归测试。

use nana_css_parity::cases::{PRIORITY_CASE_IDS, catalog};
use nana_css_parity::{
    CaseStatus, compare_to_expected, format_report, load_all_fixtures, load_fixture,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn fixture_path(id: &str) -> PathBuf {
    nana_css_parity::fixtures_dir().join(format!("{id}.json"))
}

fn assert_pass_case(id: &str) {
    let case = load_fixture(&fixture_path(id)).unwrap_or_else(|e| panic!("{id}: {e}"));
    assert_eq!(case.status, CaseStatus::Pass, "{id} fixture status");
    assert_eq!(case.id, id);
    let report = compare_to_expected(&case);
    assert!(report.ok, "{}", format_report(&report));
}

#[test]
fn t_s20_fit_content_not_fill_under_stretch() {
    let case = load_fixture(&fixture_path("T-S20")).unwrap();
    let box_ = case
        .expected
        .iter()
        .find(|box_| box_.id == "box")
        .expect("T-S20 box");
    assert!(
        (box_.w - 120.0).abs() < 0.5,
        "T-S20 must expect 120 so Fill/stretch 400 fails, got w={}",
        box_.w
    );
    assert_pass_case("T-S20");
}

#[test]
fn t_g34_auto_fill_nth_named_line_expanded() {
    let case = load_fixture(&fixture_path("T-G34")).unwrap();
    let cell = case
        .expected
        .iter()
        .find(|box_| box_.id == "cell")
        .expect("T-G34 cell");
    assert!(
        (cell.x - 80.0).abs() < 0.5 && (cell.w - 80.0).abs() < 0.5,
        "T-G34 must expect the second 80px track so a single pattern copy (x=0) fails, got x={} w={}",
        cell.x,
        cell.w
    );
    assert_pass_case("T-G34");
}

#[test]
fn t_i03_white_space_pre() {
    let case = load_fixture(&fixture_path("T-I03")).unwrap();
    let node = nana_css_parity::fixture_to_layout_node(&case.tree);
    assert_eq!(
        node.style.font_size,
        Some(16.0),
        "T-I03 fixture must keep font-size:16px, got {:?}",
        node.style.font_size
    );
    assert_eq!(
        node.text.as_deref(),
        Some("ab\ncd"),
        "T-I03 fixture text must preserve the newline"
    );
    assert_pass_case("T-I03");
}

#[test]
fn catalog_covers_priority_ids() {
    let ids: BTreeSet<_> = catalog().iter().map(|(id, _, _)| *id).collect();
    for id in PRIORITY_CASE_IDS {
        assert!(ids.contains(id), "catalog missing {id}");
        let path = fixture_path(id);
        assert!(path.is_file(), "fixture file missing for {id}");
        let case = load_fixture(&path).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(case.id, *id);
        assert_eq!(
            case.status,
            CaseStatus::Pass,
            "{id} priority fixture must remain Status::Pass so all_pass_fixtures_succeed runs it"
        );
    }
}

#[test]
fn all_pass_fixtures_succeed() {
    let fixtures = load_all_fixtures().expect("fixtures");
    let mut failures = Vec::new();
    for (_path, case) in fixtures {
        if case.status != CaseStatus::Pass {
            continue;
        }
        let report = compare_to_expected(&case);
        if !report.ok {
            failures.push(format_report(&report));
        }
    }
    assert!(
        failures.is_empty(),
        "pass fixtures failed:\n{}",
        failures.join("\n")
    );
}
