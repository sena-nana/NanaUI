//! `nana-sfc-lsp`: stdio LSP forwarding bridge between editors and
//! `rust-analyzer` for NanaUI `.vue` single-file components.

use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use nana_ui_sfc::lsp::{LspBridge, LspConfig, read_lsp_message, write_lsp_message};

fn parse_args() -> Result<(String, LspConfig), Box<dyn std::error::Error>> {
    let mut ra = std::env::var("RUST_ANALYZER").unwrap_or_else(|_| "rust-analyzer".to_owned());
    let mut config = LspConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "nana-sfc-lsp — LSP bridge between .vue views and rust-analyzer\n\n\
                     Options:\n  \
                     --rust-analyzer <path>  Path to rust-analyzer binary (default: rust-analyzer)\n  \
                     --views-dir <dir>       Directory holding .vue views\n  \
                     --virtual-rs <file>     Override generated nana_views.rs path\n  \
                     --runtime <path>        Override runtime path (e.g. ::nana_ui::runtime)"
                );
                std::process::exit(0);
            }
            "--rust-analyzer" => {
                ra = args.next().ok_or("--rust-analyzer requires a value")?;
            }
            "--views-dir" => {
                config.views_dir = Some(PathBuf::from(
                    args.next().ok_or("--views-dir requires a value")?,
                ));
            }
            "--virtual-rs" => {
                config.virtual_rs = Some(PathBuf::from(
                    args.next().ok_or("--virtual-rs requires a value")?,
                ));
            }
            "--runtime" => {
                config.runtime = Some(args.next().ok_or("--runtime requires a value")?);
            }
            other => {
                return Err(format!("unknown argument `{other}`; run with --help").into());
            }
        }
    }
    Ok((ra, config))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (ra_bin, config) = parse_args()?;
    let mut child = Command::new(&ra_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to launch `{ra_bin}`: {e}"))?;

    let ra_stdin = Arc::new(Mutex::new(BufWriter::new(
        child
            .stdin
            .take()
            .ok_or("rust-analyzer stdin unavailable")?,
    )));
    let ra_stdout = child
        .stdout
        .take()
        .ok_or("rust-analyzer stdout unavailable")?;
    let client_stdout = Arc::new(Mutex::new(BufWriter::new(io::stdout())));
    let bridge = Arc::new(Mutex::new(LspBridge::new(config)));

    let ra_reader_bridge = Arc::clone(&bridge);
    let ra_to_client_out = Arc::clone(&client_stdout);
    let ra_to_client_in = Arc::clone(&ra_stdin);
    let ra_thread = std::thread::spawn(move || {
        let mut reader = BufReader::new(ra_stdout);
        while let Ok(Some(msg)) = read_lsp_message(&mut reader) {
            let actions = ra_reader_bridge
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .handle_backend_message(msg);
            if !actions.to_backend.is_empty()
                && let Ok(mut out) = ra_to_client_in.lock()
            {
                for m in &actions.to_backend {
                    if write_lsp_message(&mut *out, m).is_err() {
                        return;
                    }
                }
            }
            if !actions.to_client.is_empty()
                && let Ok(mut out) = ra_to_client_out.lock()
            {
                for m in &actions.to_client {
                    if write_lsp_message(&mut *out, m).is_err() {
                        return;
                    }
                }
            }
        }
    });

    let mut client_reader = BufReader::new(io::stdin());
    while let Ok(Some(msg)) = read_lsp_message(&mut client_reader) {
        let actions = bridge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .handle_client_message(msg);
        if !actions.to_backend.is_empty()
            && let Ok(mut out) = ra_stdin.lock()
        {
            for m in &actions.to_backend {
                if write_lsp_message(&mut *out, m).is_err() {
                    break;
                }
            }
        }
        if !actions.to_client.is_empty()
            && let Ok(mut out) = client_stdout.lock()
        {
            for m in &actions.to_client {
                if write_lsp_message(&mut *out, m).is_err() {
                    break;
                }
            }
        }
    }

    drop(ra_stdin);
    let _ = ra_thread.join();
    let _ = child.wait();
    Ok(())
}
