//! **L1** generic NanaUI demo host for Tauri frontend bundles.
//!
//! Loads external Vue/IIFE + Tauri-compat invoke; maps CSS/class/inline through
//! `nana-ui-vue` (`css_map` / `widget_map`) into the Nana Style Model, then draws
//! with Nana iced-view. May mix **L2** `createWidget` / `nana-*` in the same tree.
//! NanaUI owns the shell (title bar / hosted window) — **L3** chrome can wrap
//! L1/L2 semantic snapshots.
//!
//! No WebView, no Blitz, no CustomContent, no CSSOM in `nana-ui`.
//!
//! ```text
//! cargo run -p nana-tauri-demo -- --project /path/to/SomeTauriApp --window
//! cargo run -p nana-tauri-demo -- \
//!   --project ~/work/LiliaGithub \
//!   --bundle dist/lilia-github.iife.js \
//!   --entry __nanaLiliaRunHome \
//!   --page home --complete-setup --window
//! ```

#![allow(unexpected_cfgs)]

use std::env;

nana_ui_vue::refuse_dual_js_engines!();

mod cli;
mod loader;
mod project;
mod tauri_compat;

#[cfg(feature = "windowed")]
mod windowed;

#[cfg(feature = "evidence-png")]
mod evidence;

fn main() {
    let raw: Vec<String> = env::args().skip(1).collect();
    let args = match cli::parse(&raw) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("nana-tauri-demo: {err}");
            std::process::exit(2);
        }
    };

    if args.help {
        println!("{}", cli::usage());
        return;
    }

    let resolved = match project::resolve(args.resolve.clone()) {
        Ok(resolved) => resolved,
        Err(err) => {
            eprintln!("nana-tauri-demo: {err}");
            std::process::exit(2);
        }
    };

    eprintln!(
        "nana-tauri-demo: project={} bundle={} entry={} page={} theme={}",
        resolved.project_root.display(),
        resolved.bundle.display(),
        resolved.entry.as_deref().unwrap_or("(self-mount)"),
        resolved.page,
        resolved.theme,
    );

    let mut boot = loader::BootOptions::from_resolved(resolved);
    boot.complete_setup = args.complete_setup;
    boot.repo_id = args.repo_id;
    boot.grant_workspace_switch = args.grant_workspace_switch;

    if let Some(png_path) = args.png.clone() {
        #[cfg(feature = "evidence-png")]
        {
            if let Err(err) = evidence::run(boot, png_path, args.interact.clone()) {
                eprintln!("nana-tauri-demo evidence failed: {err}");
                std::process::exit(1);
            }
            return;
        }
        #[cfg(not(feature = "evidence-png"))]
        {
            let _ = (boot, png_path);
            eprintln!(
                "nana-tauri-demo: `--png` requires `--features evidence-png` \
                 (iced_wgpu offscreen capture)."
            );
            std::process::exit(2);
        }
    }

    #[cfg(feature = "windowed")]
    if args.window && !args.headless {
        if let Err(err) = windowed::run(boot) {
            eprintln!("nana-tauri-demo windowed failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    #[cfg(feature = "headless-js")]
    {
        match loader::headless_report(boot) {
            Ok(report) => println!("{report}"),
            Err(err) => {
                eprintln!("nana-tauri-demo headless failed: {err}");
                std::process::exit(1);
            }
        }
        return;
    }

    #[cfg(not(any(feature = "windowed", feature = "headless-js")))]
    {
        let _ = boot;
        eprintln!(
            "nana-tauri-demo: enable `windowed` and/or `headless-js`. \
             No WebView / Blitz paint paths."
        );
        std::process::exit(2);
    }

    #[cfg(all(feature = "windowed", not(feature = "headless-js")))]
    {
        eprintln!(
            "nana-tauri-demo: windowed path skipped (--headless) but `headless-js` \
             feature is off. Re-run with `--features headless-js` or omit --headless."
        );
        std::process::exit(2);
    }
}
