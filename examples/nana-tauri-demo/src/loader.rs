//! Boot an arbitrary frontend IIFE into [`VueHost`] with Tauri compat.

use std::fs;
use std::path::Path;

use nana_js_engine::{HostValue, JsEngine, RuntimeArtifact};
use nana_ui_vue::{Capability, PermissionPolicy, VueHost};

use crate::project::ResolvedProject;
use crate::tauri_compat::{self, TAURI_COMPAT_JS};

#[derive(Debug, Clone)]
pub struct BootOptions {
    pub project: ResolvedProject,
    pub complete_setup: bool,
    pub repo_id: String,
    pub grant_workspace_switch: bool,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

impl BootOptions {
    pub fn from_resolved(project: ResolvedProject) -> Self {
        Self {
            project,
            complete_setup: false,
            repo_id: "repo-1".into(),
            grant_workspace_switch: true,
            width: 960,
            height: 640,
            scale: 1.0,
        }
    }
}

pub struct BootedRuntime {
    pub host: VueHost,
    #[allow(dead_code)] // consumed by windowed host; kept for headless callers
    pub engine: Box<dyn JsEngine>,
    #[allow(dead_code)] // status strip removed; still used by headless / evidence paths
    pub page: String,
    pub theme: String,
    pub title: String,
    #[allow(dead_code)] // status strip removed; still used by headless / evidence paths
    pub bundle: String,
}

pub fn create_engine() -> Result<Box<dyn JsEngine>, String> {
    #[cfg(all(feature = "engine-quickjs", not(feature = "engine-v8")))]
    {
        return Ok(Box::new(nana_js_quickjs::QuickJsEngine::new()));
    }
    #[cfg(feature = "engine-v8")]
    {
        return Ok(Box::new(nana_js_v8::V8Engine::new()));
    }
    #[cfg(not(any(feature = "engine-quickjs", feature = "engine-v8")))]
    {
        Err("enable engine-quickjs or engine-v8".into())
    }
}

pub fn engine_label() -> &'static str {
    #[cfg(all(feature = "engine-quickjs", not(feature = "engine-v8")))]
    {
        "quickjs"
    }
    #[cfg(feature = "engine-v8")]
    {
        "v8"
    }
    #[cfg(not(any(feature = "engine-quickjs", feature = "engine-v8")))]
    {
        "none"
    }
}

pub fn load_bundle_artifact(bundle: &Path) -> Result<RuntimeArtifact, String> {
    let source =
        fs::read_to_string(bundle).map_err(|e| format!("read bundle {}: {e}", bundle.display()))?;
    let name = bundle
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("app.iife.js")
        .to_string();
    let composed = format!("{TAURI_COMPAT_JS}\n{source}");
    Ok(RuntimeArtifact::from_source(name, composed))
}

pub fn boot(opts: BootOptions) -> Result<BootedRuntime, String> {
    let mut host = VueHost::with_viewport(opts.width, opts.height, opts.scale);
    {
        let mut policy = PermissionPolicy::with_workspace_read();
        if opts.grant_workspace_switch {
            policy.grant(Capability::WORKSPACE_SWITCH);
        }
        host.set_permission_policy(policy);
    }
    host.document()
        .lock()
        .map_err(|_| "doc poisoned".to_string())?
        .set_document_theme(&opts.project.theme);

    let mut engine = create_engine()?;
    {
        let mut api = host.host_api_registry();
        tauri_compat::register_tauri_compat(&mut api);
        engine.register_host_api(&api).map_err(|e| e.to_string())?;
    }

    let artifact = load_bundle_artifact(&opts.project.bundle)?;
    {
        let source = artifact.source_utf8().map_err(|e| e.to_string())?;
        let composed = if source.contains("__nanaWebApi") {
            artifact
        } else {
            nana_ui_web_api::compose_runtime_artifact(artifact.name.clone(), source)
        };
        engine.initialize(composed).map_err(|e| e.to_string())?;
    }

    engine.run_microtasks().map_err(|e| e.to_string())?;
    host.bind_event_bridge(&mut *engine)
        .map_err(|e| e.to_string())?;
    // Vite library builds emit companion `.css` beside the IIFE (scoped SFC rules
    // like `.home-page { display:grid; height:100% }`). Inject so cascade can
    // match arbitrary classes — not via class-name layout invention.
    if let Some(css) = companion_bundle_css(&opts.project.bundle) {
        host.inject_stylesheet(&css);
    }
    let _ = host.pump_frame(&mut *engine).map_err(|e| e.to_string())?;

    invoke_entry(&mut host, &mut *engine, &opts)?;

    // Entry runners return immediately with `{pending:true}`; poll until the
    // async mount settles (or timeout). Without this, --complete-setup never
    // finishes and the tree stays on the setup stub.
    await_js_ready(&mut host, &mut *engine, 120)?;

    let theme_mode = if opts.project.theme.eq_ignore_ascii_case("dark") {
        nana_ui_vue::ThemeMode::Dark
    } else {
        nana_ui_vue::ThemeMode::Light
    };
    // Prefer a product theme hook when present; otherwise Nana inject.
    if let Ok(force) = engine.resolve_function("__nanaForceTheme") {
        let _ = engine.invoke(force, &[HostValue::string(&opts.project.theme)]);
        let _ = engine.run_microtasks();
    } else if let Ok(force) = engine.resolve_function("__nanaLiliaForceTheme") {
        // Optional legacy alias used by some fixtures — not a product default.
        let _ = engine.invoke(force, &[HostValue::string(&opts.project.theme)]);
        let _ = engine.run_microtasks();
    } else {
        let _ = host.inject_theme(&mut *engine, theme_mode);
    }

    for _ in 0..24 {
        engine.run_microtasks().map_err(|e| e.to_string())?;
        let _ = host.pump_frame(&mut *engine).map_err(|e| e.to_string())?;
    }

    // Prefer the remount-with-startReady helper only when the first settle
    // left the workspace unready (avoids duplicate trees).
    if opts.complete_setup && opts.project.page == "home" && !js_last_ready(&mut *engine) {
        if let Ok(complete_fn) = engine.resolve_function("__nanaCompleteSetup") {
            let _ = engine.invoke(complete_fn, &[]).map_err(|e| e.to_string())?;
            await_js_ready(&mut host, &mut *engine, 120)?;
        } else if let Ok(complete_fn) = engine.resolve_function("__nanaLiliaCompleteSetup") {
            let _ = engine.invoke(complete_fn, &[]).map_err(|e| e.to_string())?;
            await_js_ready(&mut host, &mut *engine, 120)?;
        } else {
            pump_extra(&mut host, &mut *engine, 16)?;
        }
    }

    if let Ok(last_fn) = engine.resolve_function("__nanaLiliaGetLast") {
        if let Ok(last) = engine.invoke(last_fn, &[]) {
            if let Some(err) = last.as_object().and_then(|o| {
                o.get("error")
                    .and_then(HostValue::as_str)
                    .map(str::to_string)
            }) {
                if !err.is_empty() && err != "no last result" {
                    eprintln!("[nana-tauri-demo] JS last error: {err}");
                }
            }
            if let Some(obj) = last.as_object() {
                let ready = obj
                    .get("ready")
                    .and_then(|v| match v {
                        HostValue::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(false);
                let route = obj.get("route").and_then(HostValue::as_str).unwrap_or("?");
                eprintln!("[nana-tauri-demo] JS settle ready={ready} route={route}");
            }
        }
    }

    host.resolve_layout();

    Ok(BootedRuntime {
        host,
        engine,
        page: opts.project.page.clone(),
        theme: opts.project.theme.clone(),
        title: opts.project.title.clone(),
        bundle: opts.project.bundle.display().to_string(),
    })
}

fn companion_bundle_css(bundle: &Path) -> Option<String> {
    // `foo.iife.js` → prefer `foo.css` (vite asset), then `foo.iife.css`.
    let candidates = {
        let mut v = Vec::new();
        let file = bundle.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(stem) = file.strip_suffix(".iife.js") {
            v.push(bundle.with_file_name(format!("{stem}.css")));
        }
        if let Some(stem) = file.strip_suffix(".js") {
            v.push(bundle.with_file_name(format!("{stem}.css")));
        }
        v.push(bundle.with_extension("css"));
        v
    };
    for css_path in candidates {
        if let Ok(css) = fs::read_to_string(&css_path) {
            if css.trim().is_empty() {
                continue;
            }
            eprintln!(
                "[nana-tauri-demo] injected companion stylesheet {} ({} bytes)",
                css_path.display(),
                css.len()
            );
            return Some(css);
        }
    }
    None
}

fn pump_extra(host: &mut VueHost, engine: &mut dyn JsEngine, n: usize) -> Result<(), String> {
    for _ in 0..n {
        engine.run_microtasks().map_err(|e| e.to_string())?;
        let _ = host.pump_frame(engine).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn await_js_ready(
    host: &mut VueHost,
    engine: &mut dyn JsEngine,
    max_pumps: usize,
) -> Result<(), String> {
    for i in 0..max_pumps {
        engine.run_microtasks().map_err(|e| e.to_string())?;
        let _ = host.pump_frame(engine).map_err(|e| e.to_string())?;
        if js_ready_flag(engine) {
            if i > 0 {
                eprintln!("[nana-tauri-demo] JS ready after {i} pumps");
            }
            // Drain a few more frames for Vue flush / router.
            pump_extra(host, engine, 8)?;
            return Ok(());
        }
    }
    eprintln!("[nana-tauri-demo] warning: JS ready flag not set after {max_pumps} pumps");
    Ok(())
}

fn js_ready_flag(engine: &mut dyn JsEngine) -> bool {
    for name in ["__nanaLiliaIsReady", "__nanaIsReady", "__nanaAppIsReady"] {
        if let Ok(f) = engine.resolve_function(name) {
            if let Ok(v) = engine.invoke(f, &[]) {
                match v {
                    HostValue::Bool(b) => return b,
                    HostValue::Number(n) => return n != 0.0,
                    HostValue::String(s) => {
                        return s == "true" || s == "1";
                    }
                    _ => {}
                }
            }
        }
    }
    false
}

fn js_last_ready(engine: &mut dyn JsEngine) -> bool {
    let Ok(last_fn) = engine.resolve_function("__nanaLiliaGetLast") else {
        return false;
    };
    let Ok(last) = engine.invoke(last_fn, &[]) else {
        return false;
    };
    last.as_object()
        .and_then(|o| o.get("ready"))
        .and_then(HostValue::as_bool)
        .unwrap_or(false)
}

fn invoke_entry(
    host: &mut VueHost,
    engine: &mut dyn JsEngine,
    opts: &BootOptions,
) -> Result<(), String> {
    let Some(entry) = opts.project.entry.as_deref() else {
        // Bundle may self-mount; nothing to invoke.
        return Ok(());
    };

    let run_fn = engine.resolve_function(entry).map_err(|e| {
        format!(
            "entry `{entry}` not found in bundle {}: {e}\n\
             Pass --entry <globalFn> or map the page in nana-demo.toml [pages]",
            opts.project.bundle.display()
        )
    })?;

    let invoke_args: Vec<HostValue> = match opts.project.page.as_str() {
        "repo" => vec![HostValue::Object(
            [("repoId".into(), HostValue::string(&opts.repo_id))]
                .into_iter()
                .collect(),
        )],
        "home" if opts.complete_setup => vec![HostValue::Object(
            [("completeSetup".into(), HostValue::Bool(true))]
                .into_iter()
                .collect(),
        )],
        _ => vec![],
    };

    let _ = engine
        .invoke(run_fn, &invoke_args)
        .map_err(|e| e.to_string())?;
    let _ = host.pump_frame(engine).map_err(|e| e.to_string())?;
    Ok(())
}

/// Headless smoke: boot bundle + report layout box counts (no paint).
#[cfg(feature = "headless-js")]
pub fn headless_report(opts: BootOptions) -> Result<String, String> {
    let t0 = std::time::Instant::now();
    let project = opts.project.project_root.display().to_string();
    let page = opts.project.page.clone();
    let theme = opts.project.theme.clone();
    let bundle = opts.project.bundle.display().to_string();
    let entry = opts
        .project
        .entry
        .clone()
        .unwrap_or_else(|| "(self-mount)".into());
    let mut runtime = boot(opts)?;
    // Reparent orphans + Style-Model measure so layoutBox dump matches paint intent.
    runtime.host.resolve_layout();
    let snap = runtime
        .host
        .document()
        .lock()
        .map_err(|_| "doc poisoned".to_string())?
        .snapshot_boxes();
    let semantic = runtime.host.semantic_snapshot();
    let layout_box_dump: Vec<String> = {
        let doc_arc = runtime.host.document();
        let doc = doc_arc.lock().map_err(|_| "doc poisoned".to_string())?;
        semantic
            .widgets
            .iter()
            .filter(|w| {
                matches!(
                    w.kind,
                    nana_ui_vue::WidgetKind::SidebarFrame
                        | nana_ui_vue::WidgetKind::SidebarRow
                        | nana_ui_vue::WidgetKind::Text
                        | nana_ui_vue::WidgetKind::Row
                ) || w.props.class_names.iter().any(|c| {
                    c == "nana-sidebar-frame"
                        || c == "nana-sidebar-frame__body"
                        || c == "nana-sidebar-frame__top"
                        || c == "sidebar-sections"
                        || c == "nana-sidebar-row"
                        || c == "sb-tree__row"
                        || c == "sb-section__header"
                })
            })
            .filter(|w| {
                matches!(
                    w.kind,
                    nana_ui_vue::WidgetKind::SidebarFrame
                        | nana_ui_vue::WidgetKind::SidebarRow
                        | nana_ui_vue::WidgetKind::Row
                ) || w.props.class_names.iter().any(|c| {
                    c == "nana-sidebar-frame__body"
                        || c == "nana-sidebar-frame__top"
                        || c == "sidebar-sections"
                        || c == "sb-tree__row"
                        || c == "sb-section__header"
                }) || {
                    let label = w.props.display_label();
                    label.contains("置顶")
                        || label.contains("NanaUI")
                        || label.contains("LiliaGithub")
                        || label.contains("项目总览")
                        || label.contains("未分组")
                }
            })
            .take(60)
            .map(|w| {
                let box_ = doc
                    .layout_box(nana_ui_vue::NodeHandle(w.id))
                    .map(|b| format!("x={:.1},y={:.1},w={:.1},h={:.1}", b.x, b.y, b.width, b.height))
                    .unwrap_or_else(|| "none".into());
                let l = &w.props.layout;
                format!(
                    "#{}:{:?}:parent={:?}:lbl={:?}:cls={:?}:style(w={:?},h={:?},grow={:?},ovY={:?}):cb=({:?},{:?}):box={box_}",
                    w.id,
                    w.kind,
                    w.parent,
                    w.props.display_label().chars().take(20).collect::<String>(),
                    w.props.class_names.iter().take(3).cloned().collect::<Vec<_>>(),
                    l.width,
                    l.height,
                    l.flex_grow,
                    l.overflow_y,
                    w.props.containing_block_width,
                    w.props.containing_block_height,
                )
            })
            .collect()
    };
    let root_info: Vec<String> = semantic
        .roots
        .iter()
        .filter_map(|id| {
            let w = semantic.get(*id)?;
            Some(format!(
                "root#{id}:{:?} kids={} label={:?}",
                w.kind,
                w.children.len(),
                w.props.display_label().chars().take(24).collect::<String>()
            ))
        })
        .collect();
    let body_info = semantic
        .widgets
        .iter()
        .find(|w| w.props.class_names.iter().any(|c| c == "nana-mount-root"))
        .map(|w| {
            let child_kinds: Vec<_> = w
                .children
                .iter()
                .filter_map(|cid| {
                    let c = semantic.get(*cid)?;
                    let label: String = c.props.display_label().chars().take(16).collect();
                    let dir = format!("{:?}", c.props.layout.direction);
                    let class = c.props.class_names.first().cloned().unwrap_or_default();
                    Some(format!("{:?}(dir={dir},cls={class},lbl={label:?})", c.kind))
                })
                .take(8)
                .collect();
            format!(
                "body#{} kids={} -> [{}]",
                w.id,
                w.children.len(),
                child_kinds.join(" | ")
            )
        })
        .unwrap_or_else(|| "body:?".into());
    let dir_row = semantic
        .widgets
        .iter()
        .filter(|w| w.props.layout.direction == Some(nana_ui_vue::FlexDirection::Row))
        .count();
    let flex_samples: Vec<String> = semantic
        .widgets
        .iter()
        .filter(|w| {
            w.props.class_names.iter().any(|c| {
                c.contains("workspace") || c.contains("flex-row") || c.contains("nana-root")
            }) || w.props.layout.direction == Some(nana_ui_vue::FlexDirection::Row)
        })
        .take(12)
        .map(|w| {
            format!(
                "#{}:{:?}:dir={:?}:class={:?}",
                w.id, w.kind, w.props.layout.direction, w.props.class_names
            )
        })
        .collect();
    let row_kids: Vec<String> = semantic
        .widgets
        .iter()
        .find(|w| w.kind == nana_ui_vue::WidgetKind::Row)
        .map(|w| {
            w.children
                .iter()
                .filter_map(|cid| {
                    let c = semantic.get(*cid)?;
                    Some(format!(
                        "#{}:{:?}:w={:?}:h={:?}:cls={:?}",
                        c.id,
                        c.kind,
                        c.props.layout.width,
                        c.props.layout.height,
                        c.props
                            .class_names
                            .iter()
                            .take(2)
                            .cloned()
                            .collect::<Vec<_>>()
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let sidebar_samples: Vec<String> =
        semantic
            .widgets
            .iter()
            .filter(|w| {
                w.props.class_names.iter().any(|c| {
                    c.contains("secondary") || c.contains("sidebar") || c == "secondary-panel"
                }) || matches!(
                    w.kind,
                    nana_ui_vue::WidgetKind::SidebarFrame | nana_ui_vue::WidgetKind::SidebarRow
                )
            })
            .take(10)
            .map(|w| {
                format!(
                    "#{}:{:?}:parent={:?}:cls={:?}",
                    w.id, w.kind, w.parent, w.props.class_names
                )
            })
            .collect();
    let row_count = semantic
        .widgets
        .iter()
        .filter(|w| w.kind == nana_ui_vue::WidgetKind::Row)
        .count();
    let hidden_count = semantic
        .widgets
        .iter()
        .filter(|w| w.props.layout.hidden)
        .count();
    let kind_top = {
        use std::collections::BTreeMap;
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for w in &semantic.widgets {
            let name = match w.kind {
                nana_ui_vue::WidgetKind::Column => "Column",
                nana_ui_vue::WidgetKind::Row => "Row",
                nana_ui_vue::WidgetKind::Box => "Box",
                nana_ui_vue::WidgetKind::Card => "Card",
                nana_ui_vue::WidgetKind::Text => "Text",
                nana_ui_vue::WidgetKind::Button => "Button",
                nana_ui_vue::WidgetKind::Icon => "Icon",
                nana_ui_vue::WidgetKind::SidebarRow => "SidebarRow",
                nana_ui_vue::WidgetKind::ListItem => "ListItem",
                nana_ui_vue::WidgetKind::Spinner => "Spinner",
                _ => "Other",
            };
            *counts.entry(name).or_default() += 1;
        }
        counts
            .into_iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
    };
    let text_preview: Vec<_> = snap
        .texts
        .iter()
        .take(12)
        .map(|(_, t)| {
            let trimmed: String = t.chars().take(40).collect();
            trimmed
        })
        .collect();

    let shell_row_dump: Vec<String> = semantic
        .widgets
        .iter()
        .filter(|w| w.id == 6 || w.parent == Some(6) || w.props.element_tag == "aside" || w.props.attrs.get("data-region-role").map(|s| s.as_str()) == Some("resources") || w.props.class_names.iter().any(|c| c.contains("resources") || c == "secondary-panel"))
        .take(20)
        .map(|w| {
            format!(
                "#{}:parent={:?}:{:?}:tag={}:agent={:?}:region={:?}:drr={:?}:cls={:?}:w={:?}:h={:?}:kids={}",
                w.id, w.parent, w.kind, w.props.element_tag, w.props.agent_id, w.props.region,
                w.props.attrs.get("data-region-role"),
                w.props.class_names.iter().take(4).cloned().collect::<Vec<_>>(),
                w.props.layout.width, w.props.layout.height, w.children.len()
            )
        })
        .collect();

    let layout_style_dump: Vec<String> = {
        let keys = [
            "flex-row",
            "lilia-app-shell",
            "lilia-app-shell__content",
            "lilia-workspace",
            "secondary-panel",
            "lilia-workspace-region--resources",
            "lilia-workspace-region--primary",
            "lilia-workspace-region__content",
            "home-page",
            "overview-grid",
            "repo-overview-grid",
            "nana-repo",
            "nana-repo__grid",
            "nana-repo__files",
            "nana-sidebar-frame",
            "sidebar-sections",
        ];
        semantic
            .widgets
            .iter()
            .filter(|w| {
                w.kind == nana_ui_vue::WidgetKind::SidebarFrame
                    || w.props.class_names.iter().any(|c| keys.iter().any(|k| c == *k || c.contains(k)))
                    || w.props.element_tag.contains("sidebar")
                    || !w.props.region.is_empty()
                    || w.props.attrs.contains_key("data-region-role")
            })
            .take(24)
            .map(|w| {
                let l = &w.props.layout;
                format!(
                    "#{}:{:?}:tag={}:agent={:?}:region={:?}:role={:?}:cls={:?}:attrs_role={:?}:attrs_chrome={:?}:w={:?}:h={:?}:minw={:?}:minh={:?}:maxw={:?}:grow={:?}:basis={:?}:dir={:?}:gap={:?}:pad={:?}/{:?}/{:?}/{:?}:gcols={:?}:grows={:?}:ovY={:?}:justify={:?}:align={:?}",
                    w.id,
                    w.kind,
                    w.props.element_tag,
                    w.props.agent_id,
                    w.props.region,
                    w.props.role,
                    w.props.class_names.iter().take(4).cloned().collect::<Vec<_>>(),
                    w.props.attrs.get("data-region-role"),
                    w.props.attrs.get("data-nana-host-chrome"),
                    l.width, l.height, l.min_width, l.min_height, l.max_width, l.flex_grow, l.flex_basis, l.direction,
                    l.gap, l.padding_top, l.padding_right, l.padding_bottom, l.padding_left,
                    l.grid_columns, l.grid_rows, l.overflow_y, l.justify_content, l.align_items,
                )
            })
            .collect()
    };

    let card_dump: Vec<String> = semantic
        .widgets
        .iter()
        .filter(|w| {
            w.kind == nana_ui_vue::WidgetKind::Card
                || w.props.class_names.iter().any(|c| {
                    c == "card"
                        || c.contains("overview")
                        || c.contains("heatmap")
                        || c.contains("language")
                })
        })
        .take(24)
        .map(|w| {
            let kids = w.children.len();
            let label: String = w.props.display_label().chars().take(20).collect();
            format!(
                "#{}:{:?}:kids={kids}:hid={}:w={:?}:h={:?}:cls={:?}:lbl={label:?}",
                w.id,
                w.kind,
                w.props.layout.hidden,
                w.props.layout.width,
                w.props.layout.height,
                w.props
                    .class_names
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
            )
        })
        .collect();
    let interesting_texts: Vec<String> = snap
        .texts
        .iter()
        .filter_map(|(_, t)| {
            let keys = [
                "最近",
                "语言",
                "现在处理",
                "仓库状态",
                "同步",
                "提交",
                "正在打开",
                "项目总览",
            ];
            if keys.iter().any(|k| t.contains(k)) {
                Some(t.chars().take(48).collect())
            } else {
                None
            }
        })
        .take(20)
        .collect();
    let typography_dump: Vec<String> = semantic
        .widgets
        .iter()
        .filter(|w| {
            w.kind == nana_ui_vue::WidgetKind::Text
                && (w.props.layout.font_size.is_some()
                    || w.props.layout.font_weight.is_some()
                    || w.props.layout.color.is_some()
                    || w.props.layout.letter_spacing.is_some())
        })
        .take(24)
        .map(|w| {
            let l = &w.props.layout;
            format!(
                "#{}:lbl={:?}:fs={:?}:fw={:?}:ff={:?}:lh={:?}:ls={:?}:color={:?}",
                w.id,
                w.props.display_label().chars().take(16).collect::<String>(),
                l.font_size,
                l.font_weight,
                l.font_family.as_deref(),
                l.line_height,
                l.letter_spacing,
                l.color.map(|c| format!(
                    "rgba({:.0},{:.0},{:.0},{:.2})",
                    c[0] * 255.0,
                    c[1] * 255.0,
                    c[2] * 255.0,
                    c[3]
                )),
            )
        })
        .collect();
    let setup_dump: Vec<String> = semantic
        .widgets
        .iter()
        .filter(|w| {
            w.props.class_names.iter().any(|c| {
                c.contains("setup") || c == "home-page" || c.contains("workspace-region__content")
            }) || w.props.agent_id.contains("setup")
                || w.props.display_label().contains("正在打开")
        })
        .take(16)
        .map(|w| {
            format!(
                "#{}:{:?}:kids={}:hid={}:h={:?}:dir={:?}:cls={:?}:agent={:?}:lbl={:?}",
                w.id,
                w.kind,
                w.children.len(),
                w.props.layout.hidden,
                w.props.layout.height,
                w.props.layout.direction,
                w.props
                    .class_names
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>(),
                w.props.agent_id,
                w.props.display_label().chars().take(20).collect::<String>()
            )
        })
        .collect();
    let home_kids: Vec<String> = semantic
        .widgets
        .iter()
        .find(|w| w.props.class_names.iter().any(|c| c == "home-page"))
        .map(|home| {
            home.children
                .iter()
                .filter_map(|cid| {
                    let c = semantic.get(*cid)?;
                    Some(format!(
                        "#{}:{:?}:kids={}:h={:?}:w={:?}:cls={:?}:lbl={:?}",
                        c.id,
                        c.kind,
                        c.children.len(),
                        c.props.layout.height,
                        c.props.layout.width,
                        c.props
                            .class_names
                            .iter()
                            .take(2)
                            .cloned()
                            .collect::<Vec<_>>(),
                        c.props.display_label().chars().take(18).collect::<String>()
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let root_kids: Vec<String> = semantic
        .widgets
        .iter()
        .find(|w| w.props.class_names.iter().any(|c| c == "nana-root-paint"))
        .map(|root| {
            root.children
                .iter()
                .filter_map(|cid| {
                    let c = semantic.get(*cid)?;
                    Some(format!(
                        "#{}:{:?}:kids={}:h={:?}:w={:?}:grow={:?}:cls={:?}",
                        c.id,
                        c.kind,
                        c.children.len(),
                        c.props.layout.height,
                        c.props.layout.width,
                        c.props.layout.flex_grow,
                        c.props
                            .class_names
                            .iter()
                            .take(3)
                            .cloned()
                            .collect::<Vec<_>>()
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let shell_kids: Vec<String> = semantic
        .widgets
        .iter()
        .find(|w| w.props.class_names.iter().any(|c| c == "lilia-app-shell"))
        .map(|shell| {
            shell
                .children
                .iter()
                .filter_map(|cid| {
                    let c = semantic.get(*cid)?;
                    Some(format!(
                        "#{}:{:?}:kids={}:h={:?}:w={:?}:grow={:?}:grows={:?}:ovY={:?}:justify={:?}:cls={:?}",
                        c.id,
                        c.kind,
                        c.children.len(),
                        c.props.layout.height,
                        c.props.layout.width,
                        c.props.layout.flex_grow,
                        c.props.layout.grid_rows,
                        c.props.layout.overflow_y,
                        c.props.layout.justify_content,
                        c.props
                            .class_names
                            .iter()
                            .take(3)
                            .cloned()
                            .collect::<Vec<_>>()
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let titlebar_kids: Vec<String> = semantic
        .widgets
        .iter()
        .find(|w| w.props.class_names.iter().any(|c| c == "titlebar"))
        .map(|tb| {
            tb.children
                .iter()
                .filter_map(|cid| {
                    let c = semantic.get(*cid)?;
                    Some(format!(
                        "#{}:{:?}:kids={}:h={:?}:w={:?}:grow={:?}:cls={:?}:lbl={:?}",
                        c.id,
                        c.kind,
                        c.children.len(),
                        c.props.layout.height,
                        c.props.layout.width,
                        c.props.layout.flex_grow,
                        c.props
                            .class_names
                            .iter()
                            .take(3)
                            .cloned()
                            .collect::<Vec<_>>(),
                        c.props.display_label().chars().take(16).collect::<String>()
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let gpu_dump: Vec<String> = semantic
        .widgets
        .iter()
        .filter(|w| {
            w.props
                .class_names
                .iter()
                .any(|c| c.contains("gpu") || c == "nana-gpu-preview" || c == "lilia-gpu-slot")
                || w.props.agent_id.contains("gpu")
        })
        .take(8)
        .map(|w| {
            format!(
                "#{}:{:?}:kids={}:h={:?}:w={:?}:bg={:?}:grow={:?}:cls={:?}",
                w.id,
                w.kind,
                w.children.len(),
                w.props.layout.height,
                w.props.layout.width,
                w.props.layout.background,
                w.props.layout.flex_grow,
                w.props
                    .class_names
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
            )
        })
        .collect();
    let grid_kids: Vec<String> = semantic
        .widgets
        .iter()
        .filter(|w| {
            w.props
                .class_names
                .iter()
                .any(|c| c == "overview-grid" || c == "contribution-stack")
        })
        .flat_map(|g| {
            let mut lines = vec![format!(
                "GRID#{}:{:?}:dir={:?}:wrap={:?}:cls={:?}",
                g.id,
                g.kind,
                g.props.layout.direction,
                g.props.layout.flex_wrap,
                g.props
                    .class_names
                    .iter()
                    .take(2)
                    .cloned()
                    .collect::<Vec<_>>()
            )];
            for cid in &g.children {
                if let Some(c) = semantic.get(*cid) {
                    lines.push(format!(
                        "  #{cid}:{:?}:grow={:?}:basis={:?}:w={:?}:h={:?}:cls={:?}",
                        c.kind,
                        c.props.layout.flex_grow,
                        c.props.layout.flex_basis,
                        c.props.layout.width,
                        c.props.layout.height,
                        c.props
                            .class_names
                            .iter()
                            .take(2)
                            .cloned()
                            .collect::<Vec<_>>()
                    ));
                }
            }
            lines
        })
        .collect();
    let card_kids: Vec<String> = semantic
        .widgets
        .iter()
        .filter(|w| w.kind == nana_ui_vue::WidgetKind::Card)
        .take(4)
        .flat_map(|card| {
            let head = format!(
                "CARD#{}:kids={}:cls={:?}",
                card.id,
                card.children.len(),
                card.props
                    .class_names
                    .iter()
                    .take(2)
                    .cloned()
                    .collect::<Vec<_>>()
            );
            std::iter::once(head).chain(card.children.iter().take(6).flat_map(|cid| {
                let Some(c) = semantic.get(*cid) else {
                    return Vec::new();
                };
                let mut lines = vec![format!(
                    "  ch#{}:{:?}:kids={}:lbl={:?}:cls={:?}",
                    c.id,
                    c.kind,
                    c.children.len(),
                    c.props.display_label().chars().take(24).collect::<String>(),
                    c.props
                        .class_names
                        .iter()
                        .take(2)
                        .cloned()
                        .collect::<Vec<_>>()
                )];
                for gid in c.children.iter().take(4) {
                    if let Some(g) = semantic.get(*gid) {
                        lines.push(format!(
                            "    g#{}:{:?}:kids={}:lbl={:?}:cls={:?}",
                            g.id,
                            g.kind,
                            g.children.len(),
                            g.props.display_label().chars().take(28).collect::<String>(),
                            g.props
                                .class_names
                                .iter()
                                .take(2)
                                .cloned()
                                .collect::<Vec<_>>()
                        ));
                        for ggid in g.children.iter().take(3) {
                            if let Some(gg) = semantic.get(*ggid) {
                                lines.push(format!(
                                    "      gg#{}:{:?}:kids={}:lbl={:?}",
                                    gg.id,
                                    gg.kind,
                                    gg.children.len(),
                                    gg.props
                                        .display_label()
                                        .chars()
                                        .take(28)
                                        .collect::<String>()
                                ));
                                for tid in gg.children.iter().take(2) {
                                    if let Some(t) = semantic.get(*tid) {
                                        lines.push(format!(
                                            "        t#{}:{:?}:kids={}:lbl={:?}",
                                            t.id,
                                            t.kind,
                                            t.children.len(),
                                            t.props
                                                .display_label()
                                                .chars()
                                                .take(28)
                                                .collect::<String>()
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                lines
            }))
        })
        .collect();
    let views = semantic.region_views();
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
    Ok(format!(
        "nana-tauri-demo page={page} theme={theme} engine={engine} project={project} \
         bundle={bundle} entry={entry} nodes={nodes} texts={texts} widgets={widgets} events={events} \
         roots={roots} rows={rows} dir_row={dir_row} hidden={hidden} body={body_info} \
         flex_samples={flex_samples:?} row_kids={row_kids:?} sidebar={sidebar_samples:?} \
         root_info={root_info:?} elapsed_ms={elapsed_ms:.1} text_preview={text_preview:?} \
         kind_top={kind_top:?} shell_row_dump={shell_row_dump:?} layout_style_dump={layout_style_dump:?} layout_box_dump={layout_box_dump:?} card_dump={card_dump:?} interesting_texts={interesting_texts:?} typography_dump={typography_dump:?} \
         setup_dump={setup_dump:?} root_kids={root_kids:?} shell_kids={shell_kids:?} titlebar_kids={titlebar_kids:?} home_kids={home_kids:?} gpu_dump={gpu_dump:?} grid_kids={grid_kids:?} card_kids={card_kids:?} \
         REGION_VIEWS nav_widgets={nav_n} insp_widgets={insp_n} primary_widgets={prim_n} overlap={overlap:?}",
        engine = engine_label(),
        nodes = snap.boxes.len(),
        texts = snap.texts.len(),
        widgets = semantic.widgets.len(),
        events = snap.event_targets.len(),
        roots = semantic.roots.len(),
        rows = row_count,
        hidden = hidden_count,
        nav_n = views.navigation.widgets.len(),
        insp_n = views.inspector.widgets.len(),
        prim_n = views.primary.widgets.len(),
        overlap = views.overlapping_ids(),
    ))
}
