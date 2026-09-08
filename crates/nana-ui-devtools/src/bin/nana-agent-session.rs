//! Headless session for a Vue/JS artifact. Product windows never use this.
//!
//! Default fixture is the built-in counter; `--js <file>` drives a product
//! artifact. See `nana-runtime-agent` for the Vue-free, V8-free Rust L3 tier.

use std::env;
use std::process::ExitCode;

use nana_js_engine::RuntimeArtifact;
use nana_js_v8::V8Engine;
use nana_ui_devtools::agent::cli::{self, COMMON_USAGE};
use nana_ui_devtools::agent::{VueAgentSession, semantic_counter_artifact};

fn main() -> ExitCode {
    let args = match cli::parse(env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("nana-agent-session: {error}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        usage();
        return ExitCode::SUCCESS;
    }
    if args.gpu_probe {
        cli::print_gpu_probe();
        return ExitCode::SUCCESS;
    }

    let artifact = match load_artifact(&args.rest) {
        Ok(artifact) => artifact,
        Err(error) => {
            eprintln!("nana-agent-session failed: {error}");
            return ExitCode::from(1);
        }
    };
    let mut session = match VueAgentSession::new_scaled(
        V8Engine::new,
        artifact,
        args.width,
        args.height,
        args.scale,
    ) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("nana-agent-session failed: {error}");
            return ExitCode::from(1);
        }
    };
    cli::run(&mut session, &args)
}

fn usage() {
    println!(
        "nana-agent-session — headless NanaUI Vue/JS session\n\n\
         Usage: nana-agent-session [--js <app.js>] [options]\n\n\
         Vue options:\n\
         \x20 --js <file>           artifact to load (default: built-in counter)\n\n\
         Reload (stdio):\n\
         \x20 {{\"cmd\":\"reload\",\"js\":\"app.js\"}}       re-evaluate and rebuild the tree\n\
         \x20 {{\"cmd\":\"reload\",\"css\":\"app.css\"}}    swap one stylesheet, keep the tree\n\n\
         Common options:\n{COMMON_USAGE}"
    );
}

fn load_artifact(rest: &[String]) -> Result<RuntimeArtifact, String> {
    let Some(path) = flag(rest, "--js") else {
        return Ok(semantic_counter_artifact());
    };
    let source = std::fs::read_to_string(&path).map_err(|error| format!("read {path}: {error}"))?;
    Ok(RuntimeArtifact::from_source(&path, source))
}

fn flag(rest: &[String], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    let mut index = 0;
    while index < rest.len() {
        if let Some(value) = rest[index].strip_prefix(&prefix) {
            return Some(value.to_owned());
        }
        if rest[index] == name {
            return rest.get(index + 1).cloned();
        }
        index += 1;
    }
    None
}
