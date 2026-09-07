//! Headless session for a Rust L3 Runtime document. No Vue renderer, no V8.
//!
//! A CLI cannot load an arbitrary Rust application from a path, so this drives
//! named built-in fixtures. To drive your own document, call
//! `nana_ui_devtools::agent::cli::runtime_main(build_my_document(), args)` from
//! your own crate's dev binary and get the same flags.

use std::env;
use std::process::ExitCode;

use nana_ui_devtools::agent::cli::{self, COMMON_USAGE};
use nana_ui_devtools::agent::fixtures;

fn main() -> ExitCode {
    let args = match cli::parse(env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("nana-runtime-agent: {error}");
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
    if flag(&args.rest, "--list-fixtures").is_some()
        || args.rest.iter().any(|a| a == "--list-fixtures")
    {
        for fixture in fixtures::list() {
            println!("{:<16} {}", fixture.name, fixture.summary);
        }
        return ExitCode::SUCCESS;
    }

    let name = flag(&args.rest, "--fixture").unwrap_or_else(|| "counter".to_owned());
    let Some(document) = fixtures::build(&name) else {
        eprintln!("nana-runtime-agent: unknown fixture {name}; try --list-fixtures");
        return ExitCode::from(2);
    };
    cli::runtime_main(document, &args)
}

fn usage() {
    println!(
        "nana-runtime-agent — headless NanaUI Runtime session\n\n\
         Usage: nana-runtime-agent [--fixture <name>] [options]\n\n\
         Runtime options:\n\
         \x20 --fixture <name>      built-in document to drive (default counter)\n\
         \x20 --list-fixtures       print the available fixtures\n\n\
         Common options:\n{COMMON_USAGE}"
    );
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
