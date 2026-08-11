//! Compile UTF-8 JS (optionally composed with web-api shim) to QuickJS bytecode.
//!
//! ```text
//! cargo run -p nana-js-quickjs --bin nana-qjs-compile -- \
//!   --in path/to/app.iife.js --out path/to/app.qbc \
//!   --compose-shim
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use nana_js_quickjs::QuickJsEngine;

fn main() {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut compose_shim = false;
    let mut name = String::from("app.qbc.js");
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
            "--compose-shim" => compose_shim = true,
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
    let output = output.unwrap_or_else(|| input.with_extension("qbc"));

    let mut source = fs::read_to_string(&input).unwrap_or_else(|err| {
        eprintln!("read {}: {err}", input.display());
        process::exit(1);
    });
    if compose_shim {
        source = format!("{}\n{source}", nana_ui_web_api::WEB_API_SHIM_JS);
    }

    match QuickJsEngine::compile_bytecode(&name, &source) {
        Ok(artifact) => {
            if let Err(err) = fs::write(&output, &artifact.bytes) {
                eprintln!("write {}: {err}", output.display());
                process::exit(1);
            }
            println!(
                "ok kind=QuickJsBytecode name={} bytes={} out={}",
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
        "nana-qjs-compile — QuickJS bytecode Release packager\n\n\
         Usage:\n  nana-qjs-compile --in <file.js> [--out <file.qbc>] [--name <module>] [--compose-shim]\n"
    );
}
