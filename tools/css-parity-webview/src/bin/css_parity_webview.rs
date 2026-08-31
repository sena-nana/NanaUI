//! CLI：`(cd tools/css-parity-webview && cargo run --locked -- compare --webview)`

use std::process::ExitCode;

use nana_css_parity::{
    CaseStatus, compare_maps, compare_to_expected, expected_to_map, format_report,
    load_all_fixtures, measure_nana,
};
use nana_css_parity_webview::measure_webview;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("compare");
    let want_webview = args.iter().any(|a| a == "--webview") || cmd == "compare";

    if !want_webview {
        eprintln!("css-parity-webview only runs WebView compare. Pass --webview or omit args.");
        return ExitCode::FAILURE;
    }

    let fixtures = match load_all_fixtures() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let mut failed = 0usize;
    let mut passed = 0usize;
    let mut ignored = 0usize;
    for (_path, case) in fixtures {
        if case.status == CaseStatus::Ignore {
            ignored += 1;
            println!(
                "{} IGNORE ({})",
                case.id,
                case.gap.as_deref().unwrap_or("gap")
            );
            continue;
        }
        let report = compare_to_expected(&case);
        println!("{}", format_report(&report));
        if report.ok {
            passed += 1;
        } else {
            failed += 1;
        }

        match measure_webview(&case) {
            Ok(boxes) => {
                let nana = measure_nana(&case);
                let expected = expected_to_map(&boxes);
                let wr = compare_maps(
                    &format!("{}+webview", case.id),
                    &expected,
                    &nana,
                    case.tolerance_px,
                );
                println!("{}", format_report(&wr));
                if !wr.ok {
                    failed += 1;
                }
            }
            Err(e) if e.starts_with("skip:") => {
                println!("{} webview {}", case.id, e);
            }
            Err(e) => {
                eprintln!("{} webview error: {e}", case.id);
                failed += 1;
            }
        }
    }
    println!("summary: pass={passed} fail={failed} ignore={ignored}");
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
