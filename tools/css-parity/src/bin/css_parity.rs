//! CLI：`cargo run -p nana-css-parity -- compare`
//!
//! WebView 对照不在本 crate。用 `tools/css-parity-webview`：
//! `(cd tools/css-parity-webview && cargo run --locked -- compare --webview)`

use std::process::ExitCode;

use nana_css_parity::{
    CaseStatus, compare_to_expected, fixture_to_html, format_report, load_all_fixtures,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("compare");
    let want_webview = args.iter().any(|a| a == "--webview");

    match cmd {
        "list" => {
            for (path, case) in load_all_fixtures().expect("fixtures") {
                println!(
                    "{:<8} {:<8} {:<6} {}",
                    case.id,
                    match case.status {
                        CaseStatus::Pass => "pass",
                        CaseStatus::Ignore => "ignore",
                    },
                    case.gap.as_deref().unwrap_or("-"),
                    path.file_name().unwrap().to_string_lossy()
                );
            }
            ExitCode::SUCCESS
        }
        "html" => {
            let id = args.get(1).map(String::as_str).unwrap_or("");
            let fixtures = load_all_fixtures().expect("fixtures");
            let Some((_, case)) = fixtures.iter().find(|(_, c)| c.id == id) else {
                eprintln!("unknown case id: {id}");
                return ExitCode::FAILURE;
            };
            print!("{}", fixture_to_html(case));
            ExitCode::SUCCESS
        }
        _ => {
            if want_webview {
                eprintln!(
                    "--webview is provided by tools/css-parity-webview, not nana-css-parity.\n\
                     Run: (cd tools/css-parity-webview && cargo run --locked -- compare --webview)"
                );
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
            }
            println!("summary: pass={passed} fail={failed} ignore={ignored}");
            if failed > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}
