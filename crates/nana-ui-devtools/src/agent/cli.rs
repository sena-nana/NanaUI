//! Flag parsing and the driver both headless binaries share.
//!
//! The two binaries differ only in how they build a session; everything after
//! that — flags, one-shot screenshot, a11y dump, stdio loop — is identical, and
//! an Agent's muscle memory should transfer between them.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use super::protocol::ThemeName;
use super::session::{AgentSession, run_stdio};
use crate::offscreen;

pub const COMMON_USAGE: &str = "\
  --width <px>          logical width (default 480)
  --height <px>         logical height (default 320)
  --scale <factor>      device pixel ratio for screenshots (default 1)
  --theme light|dark    theme before the first frame
  --out-dir <dir>       where relative screenshots land (default target/agent-session)
  --screenshot [<name>] write one PNG and print its path
  --a11y                print the accessibility dump
  --stdio               read JSON-lines commands on stdin, reply on stdout
  --gpu-probe           print the snapshot adapter and exit
  --help                print this message";

#[derive(Debug, Clone)]
pub struct Args {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub theme: Option<ThemeName>,
    pub out_dir: PathBuf,
    pub screenshot: Option<String>,
    pub a11y: bool,
    pub stdio: bool,
    pub gpu_probe: bool,
    pub help: bool,
    /// Everything the caller's own binary interprets (`--js`, `--fixture`, …).
    pub rest: Vec<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            width: 480,
            height: 320,
            scale: 1.0,
            theme: None,
            out_dir: PathBuf::from("target/agent-session"),
            screenshot: None,
            a11y: false,
            stdio: false,
            gpu_probe: false,
            help: false,
            rest: Vec::new(),
        }
    }
}

/// Accepts both `--flag value` and `--flag=value`.
pub fn parse(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let argv: Vec<String> = argv.into_iter().collect();
    let mut args = Args::default();
    let mut index = 0;
    while index < argv.len() {
        let (name, inline) = match argv[index].split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (argv[index].as_str(), None),
        };
        let value = |args_index: &mut usize| -> Option<String> {
            if let Some(inline) = inline.clone() {
                return Some(inline);
            }
            let next = argv.get(*args_index + 1)?;
            if next.starts_with("--") {
                return None;
            }
            *args_index += 1;
            Some(next.clone())
        };
        match name {
            "--width" => {
                args.width = parse_field(value(&mut index), "--width")?;
            }
            "--height" => {
                args.height = parse_field(value(&mut index), "--height")?;
            }
            "--scale" => {
                args.scale = parse_field(value(&mut index), "--scale")?;
            }
            "--theme" => match value(&mut index).as_deref() {
                Some("light") => args.theme = Some(ThemeName::Light),
                Some("dark") => args.theme = Some(ThemeName::Dark),
                other => return Err(format!("--theme expects light|dark, got {other:?}")),
            },
            "--out-dir" => {
                args.out_dir = PathBuf::from(
                    value(&mut index).ok_or_else(|| "--out-dir expects a path".to_owned())?,
                );
            }
            // The name is optional: without one the session picks a default,
            // so a caller that only wants "a PNG, anywhere" need not invent a path.
            "--screenshot" => {
                args.screenshot =
                    Some(value(&mut index).unwrap_or_else(|| "screenshot.png".into()));
            }
            "--a11y" => args.a11y = true,
            "--stdio" => args.stdio = true,
            "--gpu-probe" => args.gpu_probe = true,
            "--help" | "-h" => args.help = true,
            _ => {
                args.rest.push(argv[index].clone());
                if inline.is_none()
                    && let Some(next) = argv.get(index + 1)
                    && !next.starts_with("--")
                {
                    args.rest.push(next.clone());
                    index += 1;
                }
            }
        }
        index += 1;
    }
    if !args.scale.is_finite() || args.scale <= 0.0 {
        return Err("--scale must be finite and positive".into());
    }
    Ok(args)
}

fn parse_field<T: std::str::FromStr>(value: Option<String>, name: &str) -> Result<T, String> {
    value
        .ok_or_else(|| format!("{name} expects a value"))?
        .parse()
        .map_err(|_| format!("{name} has an unparseable value"))
}

/// Print the snapshot adapter. Separate from a session so it answers even when
/// nothing can be built.
pub fn print_gpu_probe() {
    let probe = offscreen::gpu_probe();
    if probe.available {
        println!(
            "gpu: available adapter={} backend={}",
            probe.adapter.as_deref().unwrap_or("unknown"),
            probe.backend.as_deref().unwrap_or("unknown")
        );
    } else {
        println!(
            "gpu: unavailable reason={}",
            probe.reason.as_deref().unwrap_or("no snapshot adapter")
        );
    }
}

/// Build a Runtime session from `document` and run the requested mode.
///
/// The entry point a consuming product uses so its own dev binary is three
/// lines and speaks exactly the protocol the framework's binaries speak.
pub fn runtime_main(document: nana_ui::runtime::RuntimeDocument, args: &Args) -> ExitCode {
    let mut session = match crate::agent::RuntimeAgentSession::new_scaled(
        document,
        args.width,
        args.height,
        args.scale,
    ) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("session failed: {error}");
            return ExitCode::from(1);
        }
    };
    run(&mut session, args)
}

/// Apply the flags a session honours, then run the requested mode.
pub fn run(session: &mut dyn AgentSession, args: &Args) -> ExitCode {
    if let Some(theme) = args.theme
        && let Err(error) = session.set_theme(theme)
    {
        eprintln!("theme failed: {error}");
        return ExitCode::from(1);
    }

    if let Some(name) = &args.screenshot {
        let path = args.out_dir.join(name);
        match session.screenshot_png(&path) {
            Ok(stats) => println!(
                "{} unique_colors={} nonclear_ratio={:.4}",
                path.display(),
                stats.unique_colors,
                stats.nonclear_ratio
            ),
            Err(error) => {
                eprintln!("screenshot failed: {error}");
                return ExitCode::from(1);
            }
        }
    }

    if args.a11y {
        match serde_json::to_string_pretty(&session.accessibility_nodes()) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("a11y dump failed: {error}");
                return ExitCode::from(1);
            }
        }
    }

    if args.stdio {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        if let Err(error) = run_stdio(session, &mut stdin.lock(), &mut stdout) {
            eprintln!("stdio session failed: {error}");
            return ExitCode::from(1);
        }
        return ExitCode::SUCCESS;
    }

    // No mode requested: the dump is the useful default, and saying so beats
    // exiting silently.
    if args.screenshot.is_none() && !args.a11y {
        match serde_json::to_string_pretty(&session.accessibility_nodes()) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("a11y dump failed: {error}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(argv: &[&str]) -> Args {
        parse(argv.iter().map(|value| (*value).to_owned())).expect("parse")
    }

    #[test]
    fn both_flag_spellings_reach_the_same_field() {
        assert_eq!(args(&["--width", "800"]).width, 800);
        assert_eq!(args(&["--width=800"]).width, 800);
    }

    #[test]
    fn screenshot_name_is_optional_so_a_caller_need_not_invent_a_path() {
        assert_eq!(
            args(&["--screenshot", "--stdio"]).screenshot.as_deref(),
            Some("screenshot.png")
        );
        assert!(args(&["--screenshot", "--stdio"]).stdio);
        assert_eq!(
            args(&["--screenshot", "out.png"]).screenshot.as_deref(),
            Some("out.png")
        );
    }

    #[test]
    fn unknown_flags_and_their_values_reach_the_owning_binary() {
        let parsed = args(&["--js", "app.js", "--width", "640", "--fixture=counter"]);
        assert_eq!(parsed.width, 640);
        assert_eq!(parsed.rest, vec!["--js", "app.js", "--fixture=counter"]);
    }

    #[test]
    fn a_nonsense_scale_is_refused_rather_than_producing_an_empty_png() {
        assert!(parse(["--scale=0".to_owned()]).is_err());
        assert!(parse(["--theme=teal".to_owned()]).is_err());
    }
}
