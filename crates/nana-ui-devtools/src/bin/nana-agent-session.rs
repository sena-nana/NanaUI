//! Headless Agent stdio session. Product windows never use this binary.
//!
//! Default fixture is the built-in counter. Pass `--js <file>` to drive a
//! product Vue/JS artifact. JSON lines on stdin, JSON replies on stdout:
//! `{"cmd":"screenshot","path":"..."}`, `{"cmd":"a11y"}`,
//! `{"cmd":"click","agent_id":"increment"}`, `{"cmd":"type","text":"hi"}`.

use std::env;
use std::io;
use std::process::ExitCode;

use nana_js_engine::RuntimeArtifact;
use nana_js_v8::V8Engine;
use nana_ui_devtools::agent::{VueAgentSession, semantic_counter_artifact};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let width = parse_flag_u32(&args, "--width").unwrap_or(480);
    let height = parse_flag_u32(&args, "--height").unwrap_or(320);
    let screenshot = flag_value(&args, "--screenshot");
    let stdio = args.iter().any(|arg| arg == "--stdio");
    let artifact = match load_artifact(&args) {
        Ok(artifact) => artifact,
        Err(error) => {
            eprintln!("nana-agent-session failed: {error}");
            return ExitCode::from(1);
        }
    };

    let mut session = match VueAgentSession::new(V8Engine::new(), artifact, width, height) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("nana-agent-session failed: {error}");
            return ExitCode::from(1);
        }
    };

    if let Some(path) = screenshot {
        if let Err(error) = session.screenshot_png(path) {
            eprintln!("screenshot failed: {error}");
            return ExitCode::from(1);
        }
        println!("{path}");
    }

    if stdio {
        let stdin = io::stdin();
        if let Err(error) = session.run_stdio(stdin.lock(), io::stdout()) {
            eprintln!("stdio session failed: {error}");
            return ExitCode::from(1);
        }
        return ExitCode::SUCCESS;
    }

    if screenshot.is_none() {
        let dump = session.accessibility_dump();
        match serde_json::to_string_pretty(&dump) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("a11y dump failed: {error}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

fn load_artifact(args: &[String]) -> Result<RuntimeArtifact, String> {
    let Some(path) = flag_value(args, "--js") else {
        return Ok(semantic_counter_artifact());
    };
    let source = std::fs::read_to_string(path).map_err(|error| format!("read {path}: {error}"))?;
    Ok(RuntimeArtifact::from_source(path, source))
}

fn parse_flag_u32(args: &[String], name: &str) -> Option<u32> {
    flag_value(args, name)?.parse().ok()
}

fn flag_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=");
    let mut index = 0;
    while index < args.len() {
        if let Some(value) = args[index].strip_prefix(&prefix) {
            return Some(value);
        }
        if args[index] == name {
            return args.get(index + 1).map(String::as_str);
        }
        index += 1;
    }
    None
}
