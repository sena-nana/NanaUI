//! CLI argument parsing for `nana-tauri-demo`.

use std::path::PathBuf;

use crate::project::ResolveRequest;

#[derive(Debug, Clone)]
pub struct CliArgs {
    pub resolve: ResolveRequest,
    pub window: bool,
    pub headless: bool,
    pub complete_setup: bool,
    pub repo_id: String,
    pub grant_workspace_switch: bool,
    pub png: Option<PathBuf>,
    /// Optional interact script (e.g. `overlays` for Phase E / X3 evidence).
    pub interact: Option<String>,
    pub help: bool,
}

pub fn parse(args: &[String]) -> Result<CliArgs, String> {
    let mut project: Option<PathBuf> = None;
    let mut bundle: Option<PathBuf> = None;
    let mut entry: Option<String> = None;
    let mut page: Option<String> = None;
    let mut theme: Option<String> = None;
    let mut title: Option<String> = None;
    let mut window = true;
    let mut headless = false;
    let mut complete_setup = false;
    let mut repo_id = String::from("repo-1");
    let mut grant_workspace_switch = true;
    let mut png: Option<PathBuf> = None;
    let mut interact: Option<String> = None;
    let mut help = false;
    let mut positional_page: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => help = true,
            "--window" => window = true,
            "--no-window" | "--headless" => {
                window = false;
                headless = true;
            }
            "--complete-setup" => complete_setup = true,
            "--no-grant-workspace-switch" => grant_workspace_switch = false,
            "--grant-workspace-switch" => grant_workspace_switch = true,
            "--project" => {
                i += 1;
                project = Some(require_value("--project", args.get(i))?);
            }
            "--bundle" => {
                i += 1;
                bundle = Some(require_value("--bundle", args.get(i))?);
            }
            "--entry" => {
                i += 1;
                entry = Some(require_value_string("--entry", args.get(i))?);
            }
            "--page" => {
                i += 1;
                page = Some(require_value_string("--page", args.get(i))?);
            }
            "--theme" => {
                i += 1;
                theme = Some(require_value_string("--theme", args.get(i))?);
            }
            "--title" => {
                i += 1;
                title = Some(require_value_string("--title", args.get(i))?);
            }
            "--repo-id" => {
                i += 1;
                repo_id = require_value_string("--repo-id", args.get(i))?;
            }
            "--png" => {
                i += 1;
                png = Some(require_value("--png", args.get(i))?);
                window = false;
                headless = false;
            }
            "--interact" => {
                i += 1;
                interact = Some(require_value_string("--interact", args.get(i))?);
            }
            other if other.starts_with("--project=") => {
                project = Some(PathBuf::from(other.trim_start_matches("--project=")));
            }
            other if other.starts_with("--bundle=") => {
                bundle = Some(PathBuf::from(other.trim_start_matches("--bundle=")));
            }
            other if other.starts_with("--entry=") => {
                entry = Some(other.trim_start_matches("--entry=").to_string());
            }
            other if other.starts_with("--page=") => {
                page = Some(other.trim_start_matches("--page=").to_string());
            }
            other if other.starts_with("--theme=") => {
                theme = Some(other.trim_start_matches("--theme=").to_string());
            }
            other if other.starts_with("--title=") => {
                title = Some(other.trim_start_matches("--title=").to_string());
            }
            other if other.starts_with("--repo-id=") => {
                repo_id = other.trim_start_matches("--repo-id=").to_string();
            }
            other if other.starts_with("--png=") => {
                png = Some(PathBuf::from(other.trim_start_matches("--png=")));
                window = false;
                headless = false;
            }
            other if other.starts_with("--interact=") => {
                interact = Some(other.trim_start_matches("--interact=").to_string());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown flag: {other}\n{}", usage()));
            }
            other => {
                if positional_page.is_some() {
                    return Err(format!("unexpected argument: {other}\n{}", usage()));
                }
                positional_page = Some(other.to_string());
            }
        }
        i += 1;
    }

    if help {
        return Ok(CliArgs {
            resolve: ResolveRequest {
                project: PathBuf::new(),
                bundle: None,
                entry: None,
                page: None,
                theme: None,
                title: None,
            },
            window: false,
            headless: false,
            complete_setup: false,
            repo_id,
            grant_workspace_switch: false,
            png: None,
            interact: None,
            help: true,
        });
    }

    let Some(project) = project else {
        return Err(format!(
            "missing required `--project <tauri-project-root>`\n{}",
            usage()
        ));
    };

    let page = page.or(positional_page);

    Ok(CliArgs {
        resolve: ResolveRequest {
            project,
            bundle,
            entry,
            page,
            theme,
            title,
        },
        window,
        headless,
        complete_setup,
        repo_id,
        grant_workspace_switch,
        png,
        interact,
        help: false,
    })
}

fn require_value(flag: &str, value: Option<&String>) -> Result<PathBuf, String> {
    value
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} requires a value\n{}", usage()))
}

fn require_value_string(flag: &str, value: Option<&String>) -> Result<String, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value\n{}", usage()))
}

pub fn usage() -> String {
    r#"nana-tauri-demo — load a Tauri frontend bundle into NanaUI (no WebView)

Usage:
  cargo run -p nana-tauri-demo -- --project <tauri-root> [options] [page]
  nana-tauri-demo --project <tauri-root> [--bundle <iife.js>] [--entry <fn>] [--page home] [--window]

Required:
  --project <path>     Tauri project root (expects src-tauri/ + frontend)

Bundle resolution (first hit wins; never looks inside NanaUI):
  1. --bundle <path>          absolute, or relative to --project root
  2. nana-demo.toml `bundle`  (relative to --project)
  3. probe common dist/*.iife.js paths under --project only

Optional:
  --entry <fn>         JS global to invoke after boot (or set [pages] / entry in nana-demo.toml)
  --page <name>        page id (also accepted as trailing positional)
  --theme light|dark
  --title <window>
  --window             open Nana hosted window (default when `windowed` feature is on)
  --headless           JS/layout smoke only (requires `--features headless-js`)
  --png <path>         offscreen iced_wgpu evidence PNG (requires `--features evidence-png`)
  --interact overlays  Phase E / X3: open Dialog/Drawer/ContextMenu (+ Dropdown→Select)
                       via `__nanaLiliaOpenOverlays` (Nana Overlay; not CSS fixed)
  --complete-setup     pass {completeSetup:true} to entry when page=home
  --repo-id <id>       pass {repoId} when page=repo
  --no-grant-workspace-switch

Example (external Tauri app — build its frontend first, then point --project at it):
  cargo run -p nana-tauri-demo --features windowed -- \
    --project ~/work/LiliaGithub \
    --bundle dist/lilia-github.iife.js \
    --entry __nanaLiliaRunHome \
    --page home \
    --complete-setup \
    --window
"#
    .to_string()
}
