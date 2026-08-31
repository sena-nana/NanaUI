use nana_css_parity::{
    CaseStatus, compare_maps, expected_to_map, format_report, load_all_fixtures, measure_nana,
};
use nana_css_parity_webview::measure_webview;

#[test]
#[ignore = "需要本机显示环境；CI 无显示时跳过。运行: (cd tools/css-parity-webview && cargo test --locked -- --ignored)"]
fn webview_vs_nana_pass_cases() {
    let fixtures = load_all_fixtures().expect("fixtures");
    for (_path, case) in fixtures {
        if case.status != CaseStatus::Pass {
            continue;
        }
        match measure_webview(&case) {
            Err(e) if e.starts_with("skip:") => {
                eprintln!("skip {}: {e}", case.id);
                continue;
            }
            Err(e) => panic!("{}: {e}", case.id),
            Ok(boxes) => {
                let nana = measure_nana(&case);
                let expected = expected_to_map(&boxes);
                let report = compare_maps(&case.id, &expected, &nana, case.tolerance_px);
                assert!(report.ok, "{}", format_report(&report));
            }
        }
    }
}
