//! Compile UTF-8 JS (host-free) to a V8 StartupData snapshot blob.
//!
//! ```text
//! cargo run -p nana-js-v8 --features engine --bin nana-v8-snapshot -- \
//!   --in path/to/probe.js --out path/to/probe.v8snap
//! ```
//!
//! Snapshot source must not call `__nanaHost` at top-level. Host bridge is
//! installed after load by `V8Engine::initialize(V8Snapshot)`.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use nana_js_v8::V8Engine;

fn main() {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut name = String::from("app.v8snap.js");
    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--in" => {
                i += 1;
                input = args.get(i).map(PathBuf::from);
            }
            "--out" => {
                i += 1;
                output = args.get(i).map(PathBuf::from);
            }
            "--name" => {
                i += 1;
                if let Some(n) = args.get(i) {
                    name = n.clone();
                }
            }
            "-h" | "--help" => {
                print_help();
                return;
            }
            other => {
                eprintln!("unknown arg: {other}");
                print_help();
                process::exit(2);
            }
        }
        i += 1;
    }

    let input = input.unwrap_or_else(|| {
        eprintln!("--in is required");
        print_help();
        process::exit(2);
    });
    let output = output.unwrap_or_else(|| input.with_extension("v8snap"));

    let source = fs::read_to_string(&input).unwrap_or_else(|err| {
        eprintln!("read {}: {err}", input.display());
        process::exit(1);
    });

    match V8Engine::compile_snapshot(&name, &source) {
        Ok(artifact) => {
            if let Err(err) = fs::write(&output, &artifact.bytes) {
                eprintln!("write {}: {err}", output.display());
                process::exit(1);
            }
            println!(
                "ok kind=V8Snapshot name={} bytes={} out={}",
                artifact.name,
                artifact.bytes.len(),
                output.display()
            );
        }
        Err(err) => {
            eprintln!("compile failed: {err}");
            process::exit(1);
        }
    }
}

fn print_help() {
    eprintln!(
        "nana-v8-snapshot — V8 StartupData Release packager\n\n\
         Usage:\n  nana-v8-snapshot --in <file.js> [--out <file.v8snap>] [--name <module>]\n\n\
         Source must be host-free (no __nanaHost at top-level).\n"
    );
}
