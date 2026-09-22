//! `.nlog` viewer: `nana-nlog <text|json> <file.nlog> [--redact-home]`.

use std::process::ExitCode;

use nana_diagnostics::{ExportOptions, nlog, to_json_lines, to_text};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let redact_home = args.iter().any(|a| a == "--redact-home");
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let (Some(format), Some(path)) = (positional.first(), positional.get(1)) else {
        eprintln!("usage: nana-nlog <text|json> <file.nlog> [--redact-home]");
        return ExitCode::from(2);
    };
    let file = match nlog::read_file(path.as_str()) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("nana-nlog: {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let options = ExportOptions { redact_home };
    match format.as_str() {
        "text" => print!("{}", to_text(&file, &options)),
        "json" => print!("{}", to_json_lines(&file, &options)),
        other => {
            eprintln!("nana-nlog: unknown format `{other}` (text | json)");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}
