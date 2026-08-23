use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use nana_app_icon::{MacAppPackage, package_macos_app};

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(path) => {
            println!("{}", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<PathBuf, String> {
    let mut exe = None;
    let mut name = None;
    let mut identifier = None;
    let mut out = None;
    let mut icon = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let next = || {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{arg} requires a value"))
        };
        match arg.as_str() {
            "--exe" => exe = Some(PathBuf::from(next()?)),
            "--name" => name = Some(next()?),
            "--identifier" => identifier = Some(next()?),
            "--out" => out = Some(PathBuf::from(next()?)),
            "--icon" => icon = Some(PathBuf::from(next()?)),
            "--help" | "-h" => {
                return Err(
                    "nana-package-app --exe PATH --name NAME --identifier ID --out PATH [--icon ICNS]"
                        .into(),
                );
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 2;
    }
    package_macos_app(&MacAppPackage {
        exe: exe.ok_or("--exe is required")?,
        name: name.ok_or("--name is required")?,
        identifier: identifier.ok_or("--identifier is required")?,
        out: out.ok_or("--out is required")?,
        icon,
    })
}
