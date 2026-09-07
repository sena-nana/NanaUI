//! Gallery visual-regression suite.
//!
//! Renders every fixture through the product Runtime / Scene painter and
//! compares it against the committed per-adapter baseline. Exit codes: `0` all
//! snapshots agree (or `--bless` recorded them), `1` at least one disagrees,
//! `2` the suite could not run.

use std::path::PathBuf;
use std::process::ExitCode;

#[path = "ui_snapshots/baseline.rs"]
mod baseline;
#[path = "ui_snapshots/render.rs"]
mod render;
#[path = "ui_snapshots/write.rs"]
mod write;

const USAGE: &str = "\
usage: ui-snapshots [--bless [PREFIX ...]] [--output DIR] [--baseline DIR] [--platform KEY]

  --bless [PREFIX ...]  Record baselines instead of verifying them. Without a
                        PREFIX this re-records every snapshot; with one it
                        records only keys starting with PREFIX and keeps
                        verifying the rest.
  --output DIR          Render target (env NANA_UI_SNAPSHOT_OUTPUT).
  --baseline DIR        Committed baseline root (env NANA_UI_SNAPSHOT_BASELINE).
  --platform KEY        Override the adapter key (env NANA_UI_SNAPSHOT_PLATFORM).
";

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            print!("{}", report.summary);
            if report.failures > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("ui-snapshots could not run: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<baseline::Report, Box<dyn std::error::Error>> {
    let options = parse(std::env::args().skip(1))?;
    render::generate(baseline::Recorder::new(options))
}

fn parse(
    arguments: impl Iterator<Item = String>,
) -> Result<baseline::Options, Box<dyn std::error::Error>> {
    let mut mode = baseline::Mode::Verify;
    let mut output = None;
    let mut baseline_root = None;
    let mut platform = None;

    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--bless" => {
                let mut prefixes = Vec::new();
                while arguments.peek().is_some_and(|next| !next.starts_with("--")) {
                    prefixes.push(arguments.next().unwrap_or_default());
                }
                mode = baseline::Mode::Bless { prefixes };
            }
            "--output" => output = Some(PathBuf::from(expect(arguments.next(), "--output")?)),
            "--baseline" => {
                baseline_root = Some(PathBuf::from(expect(arguments.next(), "--baseline")?))
            }
            "--platform" => platform = Some(expect(arguments.next(), "--platform")?),
            "--help" | "-h" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument `{other}`\n\n{USAGE}").into()),
        }
    }

    if matches!(mode, baseline::Mode::Verify)
        && std::env::var_os("NANA_UI_SNAPSHOT_BLESS").is_some_and(|value| value != "0")
    {
        mode = baseline::Mode::Bless {
            prefixes: Vec::new(),
        };
    }

    // The adapter decides which baseline applies, so the suite refuses to run
    // without one rather than reporting a pass it never made.
    let probe = nana_ui_devtools::offscreen::gpu_probe();
    let (adapter, backend) = match (probe.adapter, probe.backend) {
        (Some(adapter), Some(backend)) => (adapter, backend),
        _ => {
            return Err(format!(
                "no GPU adapter for snapshot rendering: {}",
                probe.reason.unwrap_or_else(|| "unknown".to_owned())
            )
            .into());
        }
    };

    Ok(baseline::Options {
        mode,
        output: output
            .or_else(|| std::env::var_os("NANA_UI_SNAPSHOT_OUTPUT").map(PathBuf::from))
            .unwrap_or(std::env::current_dir()?.join("target/ui-snapshots")),
        baseline_root: baseline_root
            .or_else(|| std::env::var_os("NANA_UI_SNAPSHOT_BASELINE").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("snapshots")),
        platform: platform
            .or_else(|| {
                std::env::var("NANA_UI_SNAPSHOT_PLATFORM")
                    .ok()
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| baseline::platform_key(&backend, &adapter)),
        adapter: format!("{adapter} ({backend})"),
    })
}

fn expect(value: Option<String>, flag: &str) -> Result<String, Box<dyn std::error::Error>> {
    value.ok_or_else(|| format!("{flag} needs a value\n\n{USAGE}").into())
}
