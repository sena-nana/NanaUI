//! Resolve a Tauri project root, optional `nana-demo.toml`, and the JS bundle.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Optional project-side config (`nana-demo.toml` at the Tauri project root).
#[derive(Debug, Clone, Default)]
pub struct DemoConfig {
    pub title: Option<String>,
    pub bundle: Option<PathBuf>,
    pub entry: Option<String>,
    pub default_page: Option<String>,
    pub theme: Option<String>,
    /// page name → JS global entry function
    pub pages: BTreeMap<String, String>,
}

/// Resolved load plan for one demo run.
#[derive(Debug, Clone)]
pub struct ResolvedProject {
    pub project_root: PathBuf,
    pub bundle: PathBuf,
    pub entry: Option<String>,
    pub title: String,
    pub page: String,
    pub theme: String,
    /// Retained for diagnostics / future config-driven hooks.
    #[allow(dead_code)]
    pub config: DemoConfig,
}

#[derive(Debug, Clone)]
pub struct ResolveRequest {
    pub project: PathBuf,
    pub bundle: Option<PathBuf>,
    pub entry: Option<String>,
    pub page: Option<String>,
    pub theme: Option<String>,
    pub title: Option<String>,
}

pub fn resolve(req: ResolveRequest) -> Result<ResolvedProject, String> {
    let project_root = canonicalize_dir(&req.project)?;
    validate_tauri_project(&project_root)?;

    let config = load_config(&project_root.join("nana-demo.toml"))?;

    let page = req
        .page
        .or_else(|| config.default_page.clone())
        .unwrap_or_else(|| "home".into());

    let entry = req
        .entry
        .or_else(|| config.pages.get(&page).cloned())
        .or_else(|| config.entry.clone());

    let theme = req
        .theme
        .or_else(|| config.theme.clone())
        .unwrap_or_else(|| "light".into());

    let title = req
        .title
        .or_else(|| config.title.clone())
        .unwrap_or_else(|| {
            project_root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("nana-tauri-demo")
                .to_string()
        });

    let bundle = match req.bundle {
        Some(path) => resolve_bundle_path(&project_root, &path)?,
        None => match config.bundle.as_ref() {
            Some(path) => resolve_bundle_path(&project_root, path)?,
            None => probe_bundle(&project_root)?,
        },
    };

    Ok(ResolvedProject {
        project_root,
        bundle,
        entry,
        title,
        page,
        theme,
        config,
    })
}

fn canonicalize_dir(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if !path.is_dir() {
        return Err(format!(
            "--project must be an existing directory: {}",
            path.display()
        ));
    }
    fs::canonicalize(&path).map_err(|e| format!("canonicalize {}: {e}", path.display()))
}

fn validate_tauri_project(root: &Path) -> Result<(), String> {
    let src_tauri = root.join("src-tauri");
    let has_frontend = ["package.json", "src", "dist", "index.html"]
        .iter()
        .any(|name| root.join(name).exists());
    if !src_tauri.is_dir() && !has_frontend {
        return Err(format!(
            "--project does not look like a Tauri app root (missing `src-tauri/` and frontend markers):\n  {}",
            root.display()
        ));
    }
    if !src_tauri.is_dir() {
        eprintln!(
            "nana-tauri-demo: warning: no `src-tauri/` under {}; continuing with frontend-only layout",
            root.display()
        );
    }
    Ok(())
}

fn resolve_bundle_path(project_root: &Path, path: &Path) -> Result<PathBuf, String> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        // Relative `--bundle` / `nana-demo.toml` paths are always under `--project`.
        project_root.join(path)
    };
    if !candidate.is_file() {
        return Err(format!(
            "bundle not found: {} \
             (relative --bundle paths resolve against --project, not NanaUI / CWD; \
             pass an absolute path, a project-relative iife.js, or set `bundle` in nana-demo.toml)",
            candidate.display()
        ));
    }
    fs::canonicalize(&candidate).map_err(|e| format!("canonicalize {}: {e}", candidate.display()))
}

/// Probe common relative locations under the Tauri project.
fn probe_bundle(project_root: &Path) -> Result<PathBuf, String> {
    let candidates = [
        "nana-demo/dist/app.iife.js",
        "nana/dist/app.iife.js",
        "dist/nana.iife.js",
        "dist/app.iife.js",
        "dist/main.iife.js",
        "src-tauri/nana-demo/app.iife.js",
    ];
    let mut tried = Vec::new();
    for rel in candidates {
        let path = project_root.join(rel);
        tried.push(path.display().to_string());
        if path.is_file() {
            return fs::canonicalize(&path)
                .map_err(|e| format!("canonicalize {}: {e}", path.display()));
        }
    }

    // Also accept a single *.iife.js under dist/ if unambiguous.
    let dist = project_root.join("dist");
    if dist.is_dir() {
        let mut iifes = Vec::new();
        if let Ok(entries) = fs::read_dir(&dist) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|n| n.ends_with(".iife.js"))
                {
                    iifes.push(path);
                }
            }
        }
        if iifes.len() == 1 {
            let path = &iifes[0];
            return fs::canonicalize(path)
                .map_err(|e| format!("canonicalize {}: {e}", path.display()));
        }
        for path in &iifes {
            tried.push(path.display().to_string());
        }
    }

    Err(format!(
        "could not find a JS bundle under {}\n\
         Pass `--bundle <path-to.iife.js>` or add `nana-demo.toml` with `bundle = \"...\"`.\n\
         Probed:\n  - {}",
        project_root.display(),
        tried.join("\n  - ")
    ))
}

fn load_config(path: &Path) -> Result<DemoConfig, String> {
    if !path.is_file() {
        return Ok(DemoConfig::default());
    }
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    parse_demo_toml(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Minimal TOML subset for `nana-demo.toml` (no external toml crate).
fn parse_demo_toml(text: &str) -> Result<DemoConfig, String> {
    let mut cfg = DemoConfig::default();
    let mut section = String::new();

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {}: expected key = value", lineno + 1));
        };
        let key = key.trim();
        let value = parse_toml_string(value.trim())?;
        match (section.as_str(), key) {
            ("", "title") => cfg.title = Some(value),
            ("", "bundle") => cfg.bundle = Some(PathBuf::from(value)),
            ("", "entry") => cfg.entry = Some(value),
            ("", "default_page") => cfg.default_page = Some(value),
            ("", "theme") => cfg.theme = Some(value),
            ("pages", page) => {
                cfg.pages.insert(page.to_string(), value);
            }
            (sec, other) => {
                return Err(format!(
                    "line {}: unknown key `{other}` in section [{sec}]",
                    lineno + 1
                ));
            }
        }
    }
    Ok(cfg)
}

fn parse_toml_string(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return Ok(value[1..value.len() - 1].replace("\\\"", "\""));
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return Ok(value[1..value.len() - 1].to_string());
    }
    // Bare identifier / path.
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | '@'))
    {
        return Ok(value.to_string());
    }
    Err(format!("unsupported value `{value}` (use quoted strings)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nana_demo_toml_subset() {
        let cfg = parse_demo_toml(
            r#"
title = "Demo"
bundle = "dist/app.iife.js"
entry = "__nanaBoot"
default_page = "home"

[pages]
home = "__nanaRunHome"
settings = "__nanaRunSettings"
"#,
        )
        .unwrap();
        assert_eq!(cfg.title.as_deref(), Some("Demo"));
        assert_eq!(cfg.bundle.as_deref(), Some(Path::new("dist/app.iife.js")));
        assert_eq!(
            cfg.pages.get("home").map(String::as_str),
            Some("__nanaRunHome")
        );
    }

    #[test]
    fn relative_bundle_resolves_against_project_not_cwd() {
        let tmp =
            std::env::temp_dir().join(format!("nana-tauri-demo-bundle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("dist")).unwrap();
        fs::create_dir_all(tmp.join("src-tauri")).unwrap();
        fs::write(tmp.join("package.json"), "{}").unwrap();
        fs::write(tmp.join("dist/app.iife.js"), "// ok\n").unwrap();

        let resolved = resolve(ResolveRequest {
            project: tmp.clone(),
            bundle: Some(PathBuf::from("dist/app.iife.js")),
            entry: None,
            page: None,
            theme: None,
            title: None,
        })
        .unwrap();
        assert!(resolved.bundle.ends_with("dist/app.iife.js"));

        let err = resolve(ResolveRequest {
            project: tmp.clone(),
            bundle: Some(PathBuf::from(
                "crates/nana-js-engine/fixtures/missing.iife.js",
            )),
            entry: None,
            page: None,
            theme: None,
            title: None,
        })
        .unwrap_err();
        assert!(
            err.contains("bundle not found"),
            "expected project-relative miss, got: {err}"
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}
