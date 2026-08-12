//! Offscreen iced_wgpu PNG capture for L1 page evidence (no WebView / Blitz).
//!
//! Same boot path as windowed/`headless-js`, then paints the semantic tree through
//! Nana iced-view and GPU→CPU readback. Use for same-session QJS↔V8 pairs.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use iced::widget::{column, container, text};
use iced::{Color, Element, Length, Pixels, Size, Theme, font};
use iced_wgpu::graphics::{Antialiasing, Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::core::time::Instant;
use iced_winit::core::{Event, mouse, renderer, shell, window};
use iced_winit::futures::futures::executor;
use iced_winit::runtime::UserInterface;
use iced_winit::runtime::user_interface;
use nana_js_engine::HostValue;
use nana_ui::{ThemeMode, ThemeModeExt, UI_BASE_TEXT_SIZE};
use nana_ui_vue::{
    BridgeEvent, WidgetKind, theme_tokens_from_snapshot, view_semantic_tree_static_with_editors,
};

use crate::loader::{self, BootOptions, BootedRuntime, engine_label};

const EVIDENCE_SIZE: Size<u32> = Size::new(960, 640);
const TITLE_BAR_HEIGHT: f32 = 36.0;
const SETTLE_PUMPS: usize = 24;

pub fn run(
    mut opts: BootOptions,
    png_path: PathBuf,
    interact: Option<String>,
) -> Result<(), String> {
    opts.width = EVIDENCE_SIZE.width;
    opts.height = (EVIDENCE_SIZE.height as f32 - TITLE_BAR_HEIGHT).max(1.0) as u32;
    opts.scale = 1.0;
    let content_size = (opts.width as f32, opts.height as f32);

    let BootedRuntime {
        mut host,
        mut engine,
        page,
        theme: theme_s,
        title,
        bundle,
    } = loader::boot(opts)?;

    for _ in 0..SETTLE_PUMPS {
        let _ = host.pump_frame(&mut *engine).map_err(|e| e.to_string())?;
    }

    let mut overlay_report: Option<OverlayInteractReport> = None;
    if let Some(script) = interact.as_deref() {
        if script.eq_ignore_ascii_case("overlays") {
            overlay_report = Some(run_overlays_interact(&mut host, &mut *engine)?);
        } else {
            return Err(format!("unknown --interact={script} (supported: overlays)"));
        }
    }

    host.resolve_layout();
    host.prepare_editors();
    host.prepare_menus();

    let snap = host.semantic_snapshot();

    // Reachability + density checks run *before* PNG write so baseline and
    // candidate captures both hard-fail on degenerate trees (avoids SSIM=1.0
    // false pass when both sides paint the same empty shell).
    let cards = snap
        .widgets
        .iter()
        .filter(|w| {
            matches!(
                w.kind,
                nana_ui_vue::WidgetKind::Card | nana_ui_vue::WidgetKind::SettingsCard
            )
        })
        .count();
    let settings_rows = snap
        .widgets
        .iter()
        .filter(|w| w.kind == nana_ui_vue::WidgetKind::SettingsRow)
        .count();
    let boxes = snap
        .widgets
        .iter()
        .filter(|w| w.kind == nana_ui_vue::WidgetKind::Box)
        .count();
    let rows = snap
        .widgets
        .iter()
        .filter(|w| w.kind == nana_ui_vue::WidgetKind::Row)
        .count();
    let labels: Vec<_> = snap
        .widgets
        .iter()
        .filter_map(|w| {
            let l = w.props.display_label();
            if l.contains("最近")
                || l.contains("语言")
                || l.contains("仓库状态")
                || l.contains("主题")
                || l.contains("窗口材质")
                || l.contains("外观")
                || l.contains("工作区")
            {
                Some(l.chars().take(24).collect::<String>())
            } else {
                None
            }
        })
        .take(12)
        .collect();
    let mut reachable = std::collections::HashSet::new();
    fn walk(
        snap: &nana_ui_vue::SemanticSnapshot,
        id: u64,
        out: &mut std::collections::HashSet<u64>,
    ) {
        if !out.insert(id) {
            return;
        }
        if let Some(w) = snap.get(id) {
            for &c in &w.children {
                walk(snap, c, out);
            }
        }
    }
    for &r in &snap.roots {
        walk(&snap, r, &mut reachable);
    }
    let cards_reachable = snap
        .widgets
        .iter()
        .filter(|w| {
            matches!(
                w.kind,
                nana_ui_vue::WidgetKind::Card | nana_ui_vue::WidgetKind::SettingsCard
            ) && reachable.contains(&w.id)
        })
        .count();
    let settings_rows_reachable = snap
        .widgets
        .iter()
        .filter(|w| w.kind == nana_ui_vue::WidgetKind::SettingsRow && reachable.contains(&w.id))
        .count();
    let home_reachable = snap
        .widgets
        .iter()
        .any(|w| reachable.contains(&w.id) && w.props.class_names.iter().any(|c| c == "home-page"));
    let overview_reachable = snap.widgets.iter().any(|w| {
        reachable.contains(&w.id) && w.props.class_names.iter().any(|c| c == "overview-grid")
    });
    let repo_reachable = snap.widgets.iter().any(|w| {
        reachable.contains(&w.id)
            && (w.props.class_names.iter().any(|c| c == "nana-repo")
                || w.props.agent_id == "repo.page"
                || w.props.attrs.get("data-page").map(|s| s.as_str()) == Some("repo"))
    });
    let repo_readme_reachable = snap.widgets.iter().any(|w| {
        reachable.contains(&w.id)
            && (w.props.agent_id == "repo.panel.readme"
                || w.props.agent_id == "repo.readme"
                || w.props.class_names.iter().any(|c| c == "nana-repo__readme"))
    });
    let segmented_opts: Vec<String> = snap
        .widgets
        .iter()
        .filter(|w| w.kind == nana_ui_vue::WidgetKind::Segmented)
        .take(2)
        .flat_map(|w| {
            w.props
                .options
                .iter()
                .map(|o| format!("{}:{}", o.value, o.label))
        })
        .collect();

    if page.eq_ignore_ascii_case("home") {
        if !home_reachable {
            return Err(format!(
                "evidence hard-fail: page=home but home-page is not reachable \
                 (widgets={} roots={} cards_reachable={cards_reachable}); \
                 refusing PNG write to avoid false SSIM pass",
                snap.widgets.len(),
                snap.roots.len(),
            ));
        }
        if !overview_reachable {
            return Err(format!(
                "evidence hard-fail: page=home but overview-grid is not reachable \
                 (widgets={} roots={} home_ok={home_reachable} cards_reachable={cards_reachable}); \
                 refusing PNG write to avoid false SSIM pass",
                snap.widgets.len(),
                snap.roots.len(),
            ));
        }
        if cards_reachable == 0 {
            return Err(format!(
                "evidence hard-fail: page=home has zero reachable cards \
                 (widgets={} roots={}); refusing PNG write to avoid false SSIM pass",
                snap.widgets.len(),
                snap.roots.len(),
            ));
        }
    } else if page.eq_ignore_ascii_case("settings") {
        if settings_rows_reachable == 0 {
            return Err(format!(
                "evidence hard-fail: page=settings has zero reachable SettingsRow \
                 (widgets={} roots={} cards_reachable={cards_reachable}); \
                 refusing PNG write to avoid false SSIM pass",
                snap.widgets.len(),
                snap.roots.len(),
            ));
        }
        // Parity with home's multi-gate: require Appearance key rows, not just
        // any SettingsRow (Account/Workspace stubs must not SSIM-pass alone).
        let appearance_theme_reachable = snap.widgets.iter().any(|w| {
            w.kind == nana_ui_vue::WidgetKind::SettingsRow
                && reachable.contains(&w.id)
                && w.props.display_label().contains("主题")
        });
        let appearance_material_reachable = snap.widgets.iter().any(|w| {
            w.kind == nana_ui_vue::WidgetKind::SettingsRow
                && reachable.contains(&w.id)
                && w.props.display_label().contains("窗口材质")
        });
        if !appearance_theme_reachable || !appearance_material_reachable {
            return Err(format!(
                "evidence hard-fail: page=settings missing reachable Appearance rows \
                 (theme_ok={appearance_theme_reachable} material_ok={appearance_material_reachable} \
                 settings_rows_reachable={settings_rows_reachable} widgets={} roots={}); \
                 refusing PNG write to avoid false SSIM pass",
                snap.widgets.len(),
                snap.roots.len(),
            ));
        }
    } else if page.eq_ignore_ascii_case("repo") {
        // NanaRepoPage default tab=readme: require page shell + README panel + card.
        if !repo_reachable {
            return Err(format!(
                "evidence hard-fail: page=repo but nana-repo / repo.page is not reachable \
                 (widgets={} roots={} cards_reachable={cards_reachable}); \
                 refusing PNG write to avoid false SSIM pass",
                snap.widgets.len(),
                snap.roots.len(),
            ));
        }
        if !repo_readme_reachable {
            return Err(format!(
                "evidence hard-fail: page=repo missing reachable README panel \
                 (repo_ok={repo_reachable} cards_reachable={cards_reachable} widgets={} roots={}); \
                 refusing PNG write to avoid false SSIM pass",
                snap.widgets.len(),
                snap.roots.len(),
            ));
        }
        if cards_reachable == 0 {
            return Err(format!(
                "evidence hard-fail: page=repo has zero reachable cards \
                 (repo_ok={repo_reachable} readme_ok={repo_readme_reachable} widgets={} roots={}); \
                 refusing PNG write to avoid false SSIM pass",
                snap.widgets.len(),
                snap.roots.len(),
            ));
        }
    }

    let theme = if theme_s.eq_ignore_ascii_case("dark") {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    let tokens = theme_tokens_from_snapshot(&snap, false);
    let tree = container(view_semantic_tree_static_with_editors(
        &snap,
        tokens,
        Some(content_size),
        Some(host.editors()),
        Some(host.menus()),
        |e| e,
    ))
    .width(Length::Fixed(content_size.0))
    .height(Length::Fixed(content_size.1));
    let view: Element<'_, BridgeEvent> = column![
        container(text(title.clone()).size(12))
            .width(Length::Fixed(EVIDENCE_SIZE.width as f32))
            .height(Length::Fixed(TITLE_BAR_HEIGHT))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        tree,
    ]
    .width(Length::Fixed(EVIDENCE_SIZE.width as f32))
    .height(Length::Fixed(EVIDENCE_SIZE.height as f32))
    .into();

    let background = theme.colors().background;
    let pixels = screenshot(view, &theme.iced_theme(), background, EVIDENCE_SIZE)?;

    if page.eq_ignore_ascii_case("home") {
        let iced_boxes = nana_ui_vue::shared_layout_box_store().snapshot();
        let mut by_id = std::collections::HashMap::new();
        for (h, b) in &iced_boxes {
            by_id.insert(h.0, *b);
        }
        let keys = [
            "home-page",
            "page-header",
            "overview-grid",
            "repo-overview-grid",
            "contribution-card",
            "contribution-stack",
            "contribution-chart",
            "contribution-window",
            "calendar-heatmap",
            "calendar-heatmap-wrap",
            "card-heading",
            "language-card",
            "language-chart",
            "language-pie",
            "language-list",
            "language-actions",
            "language-total",
            "contribution-total",
            "github-timeline-card",
            "repo-status-card",
        ];
        let mut lines = Vec::new();
        for w in &snap.widgets {
            let hit = w
                .props
                .class_names
                .iter()
                .any(|c| keys.iter().any(|k| c == k))
                || w.props.display_label().contains("最近工作")
                || w.props.display_label().contains("编程语言占比")
                || w.props.display_label().contains("356")
                || w.props.display_label().contains("686")
                || w.parent.is_some_and(|p| {
                    snap.get(p)
                        .is_some_and(|pw| pw.props.class_names.iter().any(|c| c == "card-heading"))
                });
            if !hit {
                continue;
            }
            let box_ = by_id.get(&w.id).copied();
            let l = &w.props.layout;
            lines.push(format!(
                "iced#{}:{:?}:cls={:?}:lbl={:?}:w={:?}:h={:?}:minh={:?}:grow={:?}:align={:?}:justify={:?}:gap={:?}:dir={:?}:box={:?}",
                w.id,
                w.kind,
                w.props
                    .class_names
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>(),
                w.props.display_label().chars().take(24).collect::<String>(),
                l.width,
                l.height,
                l.min_height,
                l.flex_grow,
                l.align_items,
                l.justify_content,
                l.gap.or(l.row_gap),
                l.direction,
                box_
            ));
        }
        eprintln!("home iced boxes:\n{}", lines.join("\n"));
    }

    // After iced layout+draw, require Repo main content to actually paint.
    // Tree reachability alone can SSIM-pass two empty mains (both white shells).
    // Prefer iced LayoutProbe boxes; fall back to main-region pixel occupancy.
    if page.eq_ignore_ascii_case("repo") {
        let iced_boxes = nana_ui_vue::shared_layout_box_store().snapshot();
        let mut by_id = std::collections::HashMap::new();
        for (h, b) in &iced_boxes {
            by_id.insert(h.0, *b);
        }
        let repo_box = snap.widgets.iter().find_map(|w| {
            if !(w.props.class_names.iter().any(|c| c == "nana-repo")
                || w.props.agent_id == "repo.page")
            {
                return None;
            }
            by_id.get(&w.id).copied()
        });
        let readme_box = snap.widgets.iter().find_map(|w| {
            if !(w.props.agent_id == "repo.panel.readme"
                || w.props.agent_id == "repo.readme"
                || w.props.class_names.iter().any(|c| c == "nana-repo__readme"))
            {
                return None;
            }
            by_id.get(&w.id).copied()
        });
        let repo_ok = repo_box.is_some_and(|b| b.width >= 120.0 && b.height >= 40.0);
        let readme_ok = readme_box.is_some_and(|b| b.width >= 80.0 && b.height >= 24.0);
        let main_nonwhite = count_nonwhite_in_main(&pixels, EVIDENCE_SIZE);
        let pixels_ok = main_nonwhite >= 800;
        if !(repo_ok && readme_ok) && !pixels_ok {
            return Err(format!(
                "evidence hard-fail: page=repo iced paint too empty \
                 (repo={repo_box:?} readme={readme_box:?} main_nonwhite={main_nonwhite} \
                 probes={}); refusing PNG write to avoid empty-shell SSIM false pass",
                iced_boxes.len()
            ));
        }
    }

    write_png(&png_path, EVIDENCE_SIZE, &pixels)?;

    eprintln!(
        "nana-tauri-demo evidence: engine={} page={} widgets={} roots={} cards={} cards_reachable={} \
         settings_rows={settings_rows}/{settings_rows_reachable} home_ok={} overview_ok={} \
         repo_ok={} repo_readme_ok={} boxes={} rows={} \
         labels={:?} segmented_opts={segmented_opts:?} -> {}",
        engine_label(),
        page,
        snap.widgets.len(),
        snap.roots.len(),
        cards,
        cards_reachable,
        home_reachable,
        overview_reachable,
        repo_reachable,
        repo_readme_reachable,
        boxes,
        rows,
        labels,
        png_path.display()
    );
    if let Some(report) = overlay_report.as_ref() {
        eprintln!(
            "nana-tauri-demo overlay-interact: dialog_open={} drawer_open={} context_menu_open={} \
             select_dropdown={} overlay_fixed_stripped={} path={} js={:?}",
            report.dialog_open,
            report.drawer_open,
            report.context_menu_open,
            report.select_dropdown,
            report.overlay_fixed_stripped,
            report.path,
            report.js_state
        );
        write_overlay_log(&png_path, report)?;
    }
    let _ = (engine, bundle);
    Ok(())
}

#[derive(Debug, Clone)]
struct OverlayInteractReport {
    path: &'static str,
    dialog_open: bool,
    drawer_open: bool,
    context_menu_open: bool,
    select_dropdown: bool,
    overlay_fixed_stripped: bool,
    js_state: String,
}

fn run_overlays_interact(
    host: &mut nana_ui_vue::VueHost,
    engine: &mut dyn nana_js_engine::JsEngine,
) -> Result<OverlayInteractReport, String> {
    let open_fn = engine
        .resolve_function("__nanaLiliaOpenOverlays")
        .map_err(|e| {
            format!(
                "overlay interact: missing __nanaLiliaOpenOverlays ({e}); \
                 rebuild LiliaGithub Nana IIFE with NanaOverlayEvidence host"
            )
        })?;
    let js = engine
        .invoke(open_fn, &[])
        .map_err(|e| format!("overlay interact invoke: {e}"))?;
    engine
        .run_microtasks()
        .map_err(|e| format!("overlay interact microtasks: {e}"))?;
    for _ in 0..SETTLE_PUMPS {
        let _ = host.pump_frame(engine).map_err(|e| e.to_string())?;
    }
    host.resolve_layout();
    host.prepare_editors();
    host.prepare_menus();

    let snap = host.semantic_snapshot();
    let mut dialog_open = false;
    let mut drawer_open = false;
    let mut context_menu_open = false;
    let mut select_dropdown = false;
    let mut overlay_with_fixed = Vec::new();

    for w in &snap.widgets {
        let open = w.props.active || w.props.toggled;
        let is_fixed = w.props.layout.is_fixed();
        match w.kind {
            WidgetKind::Dialog if open => {
                dialog_open = true;
                if is_fixed {
                    overlay_with_fixed.push(("Dialog", w.id));
                }
            }
            WidgetKind::Drawer if open => {
                drawer_open = true;
                if is_fixed {
                    overlay_with_fixed.push(("Drawer", w.id));
                }
            }
            WidgetKind::ContextMenu if open => {
                context_menu_open = true;
                if is_fixed {
                    overlay_with_fixed.push(("ContextMenu", w.id));
                }
            }
            WidgetKind::Select => {
                if w.props.class_names.iter().any(|c| {
                    c == "nana-dropdown" || c == "dd" || c == "nana-select" || c == "dropdown"
                }) || w.props.agent_id.contains("dropdown")
                    || w.props.agent_id.contains("nana.overlay.evidence.dropdown")
                {
                    select_dropdown = true;
                    if is_fixed {
                        overlay_with_fixed.push(("Select/Dropdown", w.id));
                    }
                }
            }
            WidgetKind::Popover if open && is_fixed => {
                overlay_with_fixed.push(("Popover", w.id));
            }
            _ => {}
        }
    }

    if !dialog_open || !drawer_open || !context_menu_open || !select_dropdown {
        return Err(format!(
            "overlay interact hard-fail: expected open Dialog/Drawer/ContextMenu + Dropdown→Select \
             (dialog={dialog_open} drawer={drawer_open} context_menu={context_menu_open} \
             select_dropdown={select_dropdown} widgets={})",
            snap.widgets.len()
        ));
    }
    if !overlay_with_fixed.is_empty() {
        return Err(format!(
            "overlay interact hard-fail: product overlays must strip companion CSS fixed/sticky; \
             found fixed on {overlay_with_fixed:?}"
        ));
    }

    let js_state = match js.as_object() {
        Some(obj) => format!(
            "ok={} dialog={:?} drawer={:?} contextMenu={:?} dropdown={:?} path={:?} fixedEngine={:?}",
            obj.get("ok")
                .and_then(|v| match v {
                    HostValue::Bool(b) => Some(*b),
                    _ => None,
                })
                .unwrap_or(false),
            obj.get("dialog"),
            obj.get("drawer"),
            obj.get("contextMenu"),
            obj.get("dropdown"),
            obj.get("path"),
            obj.get("fixedEngine"),
        ),
        None => format!("{js:?}"),
    };

    Ok(OverlayInteractReport {
        path: "nana-overlay",
        dialog_open,
        drawer_open,
        context_menu_open,
        select_dropdown,
        overlay_fixed_stripped: true,
        js_state,
    })
}

fn write_overlay_log(png_path: &Path, report: &OverlayInteractReport) -> Result<(), String> {
    let log_path = png_path.with_extension("overlay.json");
    let body = format!(
        "{{\n  \"path\": \"{}\",\n  \"dialog_open\": {},\n  \"drawer_open\": {},\n  \
         \"context_menu_open\": {},\n  \"select_dropdown\": {},\n  \
         \"overlay_fixed_stripped\": {},\n  \"fixed_engine\": false,\n  \
         \"js_state\": {}\n}}\n",
        report.path,
        report.dialog_open,
        report.drawer_open,
        report.context_menu_open,
        report.select_dropdown,
        report.overlay_fixed_stripped,
        serde_json_escape(&report.js_state),
    );
    fs::write(&log_path, body).map_err(|e| format!("write {}: {e}", log_path.display()))?;
    eprintln!(
        "nana-tauri-demo overlay-interact log -> {}",
        log_path.display()
    );
    Ok(())
}

fn serde_json_escape(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn screenshot<Message>(
    view: Element<'_, Message, Theme, Renderer>,
    theme: &Theme,
    background: Color,
    size: Size<u32>,
) -> Result<Vec<u8>, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|e| format!("wgpu adapter: {e}"))?;
    let (device, queue) = executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nana-tauri-demo evidence"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .map_err(|e| format!("wgpu device: {e}"))?;
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let engine = Engine::new(
        &adapter,
        device,
        queue,
        format,
        Some(Antialiasing::MSAAx4),
        Shell::headless(),
    );
    let mut renderer = Renderer::new(
        engine,
        renderer::Settings {
            default_font: nana_ui::ui_font(font::Weight::Normal),
            default_text_size: Pixels::from(UI_BASE_TEXT_SIZE),
            metrics_hinting: true,
        },
    );
    {
        let mut font_system = iced_wgpu::graphics::text::font_system()
            .write()
            .expect("font system");
        for source in nana_ui::ui_font_sources() {
            font_system.load_font(std::borrow::Cow::Borrowed(source));
        }
    }

    let viewport = Viewport::with_physical_size(size, renderer::Scale::default());
    let mut interface = UserInterface::build(
        view,
        viewport.logical_size(),
        user_interface::Cache::new(),
        &mut renderer,
    );
    let window = window::Headless;
    let waker = shell::Waker::noop();
    let cursor = mouse::Cursor::Unavailable;
    let _ = interface.update(
        &window,
        &waker,
        &[Event::Window(
            window::Event::RedrawRequested(Instant::now()),
        )],
        cursor,
        &mut renderer,
        &mut Vec::new(),
    );
    interface.draw(
        &mut renderer,
        theme,
        &renderer::Style {
            text_color: theme.palette().background.base.text,
        },
        cursor,
    );
    let cache = interface.into_cache();
    let pixels = renderer.screenshot(&viewport, background);
    drop(cache);
    Ok(pixels)
}

fn write_png(path: &Path, size: Size<u32>, pixels: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), size.width, size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(pixels).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Count non-near-white pixels in the primary pane (right of sidebar, below title).
fn count_nonwhite_in_main(pixels: &[u8], size: Size<u32>) -> usize {
    let w = size.width as usize;
    let h = size.height as usize;
    let x0 = 240usize; // past SidebarFrame (~220) + seam
    let y0 = TITLE_BAR_HEIGHT as usize;
    let mut n = 0usize;
    for y in y0..h {
        for x in x0..w {
            let i = (y * w + x) * 4;
            if i + 2 >= pixels.len() {
                continue;
            }
            let (r, g, b) = (pixels[i], pixels[i + 1], pixels[i + 2]);
            if r <= 250 || g <= 250 || b <= 250 {
                n += 1;
            }
        }
    }
    n
}
