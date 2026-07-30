#[path = "ui_benchmark/report.rs"]
mod report;
#[path = "ui_benchmark/runner.rs"]
mod runner;

fn main() {
    let report = runner::run();
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("benchmark report must serialize")
    );
}
