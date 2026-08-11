//! Nana hosted window: Vue tree → MessageBridge → NanaUI Iced widgets.
//!
//! L3 [`DesktopShell`] hosts mutually exclusive semantic region projections:
//! [`SemanticSnapshot::region_views`] — each widget id is painted in at most one
//! Region. Untagged forests stay entirely in Primary (single paint entry). Hosted
//! `Element<'static>` is satisfied by an **owned** workspace snapshot
//! (`controller.clone()`), not by borrowing the live controller.
//!
//! Chrome: this host draws the only titlebar ([`AppTitleBar`]). Lilia's Nana
//! path (`data-nana-host-chrome`) must not mount a second TitleBar.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use iced::Element;
use nana_js_engine::JsEngine;
use nana_ui::{
    AppTitleBar, AppearanceSettings, BackdropTarget, Button, ButtonKind, DesktopShell,
    HostedProgram, HostedProgramContext, HostedProgramUpdate, HostedRunError, HostedWindowEvent,
    HostedWindowSettings, ThemeMode, ThemeModeExt, ThemeTokens, WindowChrome, WindowChromeEvent,
    WindowChromeState, WindowMaterialMode, WorkspaceAction, WorkspaceController, run_hosted,
};
use nana_ui_vue::{
    BridgeEvent, SemanticSnapshot, VueHost, WindowLifecycleEvent,
    view_semantic_tree_static_with_editors,
};

use crate::loader::{self, BootOptions, BootedRuntime, engine_label};

const TITLE_BAR_HEIGHT: f32 = 36.0;

#[derive(Debug, Clone)]
enum Message {
    Widget(BridgeEvent),
    ToggleTheme,
    Chrome(WindowChromeEvent),
    Workspace(WorkspaceAction),
}

struct LoaderProgram {
    host: RefCell<VueHost>,
    engine: RefCell<Box<dyn JsEngine>>,
    snapshot: SemanticSnapshot,
    theme: ThemeMode,
    appearance: AppearanceSettings,
    title: String,
    chrome: WindowChromeState,
    last_pump: Instant,
    /// Logical content viewport for `%` / height-chain resolution (P0-3/P0-4).
    content_size: (f32, f32),
    /// L3 workspace shell; Primary Region hosts the L1/L2 semantic tree.
    workspace: WorkspaceController,
}

fn hosted_appearance() -> AppearanceSettings {
    let mut appearance = AppearanceSettings::default();
    let _ = appearance.set_window_material(WindowMaterialMode::Translucent);
    appearance
}

thread_local! {
    static BOOT_SEED: RefCell<Option<BootOptions>> = const { RefCell::new(None) };
}

pub fn run(opts: BootOptions) -> Result<(), HostedRunError> {
    let title = format!(
        "{} · {} · {} (nana-tauri-demo)",
        opts.project.title,
        opts.project.page,
        engine_label()
    );
    BOOT_SEED.with(|slot| {
        *slot.borrow_mut() = Some(opts);
    });
    let result = run_hosted::<LoaderProgram>(
        HostedWindowSettings::new(title)
            .initial_size(960.0, 640.0)
            .minimum_size(640.0, 480.0),
    );
    BOOT_SEED.with(|slot| *slot.borrow_mut() = None);
    result
}

fn take_boot() -> BootOptions {
    BOOT_SEED.with(|slot| {
        slot.borrow_mut()
            .take()
            .expect("nana-tauri-demo boot options missing")
    })
}

impl HostedProgram for LoaderProgram {
    type Message = Message;
    type Error = String;

    fn initialize(
        context: &HostedProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let mut opts = take_boot();
        let logical = context.logical_size();
        opts.width = logical.width.max(1.0) as u32;
        opts.height = (logical.height - TITLE_BAR_HEIGHT).max(1.0) as u32;
        opts.scale = context.scale_factor();
        let content_size = (opts.width as f32, opts.height as f32);

        let BootedRuntime {
            host,
            engine,
            theme: theme_s,
            title,
            ..
        } = loader::boot(opts)?;

        let theme = if theme_s.eq_ignore_ascii_case("dark") {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        // Seed translucent material so the hosted path exercises native_material
        // until L1 writes an explicit translucent/solid choice we can honor.
        let mut host = host;
        {
            let bridge = host.bridge();
            let mut bridge = bridge.lock().expect("vue bridge");
            bridge.set_appearance(hosted_appearance());
        }
        host.prepare_editors();
        host.prepare_menus();
        let snapshot = host.semantic_snapshot();
        let mut appearance = snapshot.appearance;
        if appearance.window_material() == WindowMaterialMode::Solid {
            let _ = appearance.set_window_material(WindowMaterialMode::Translucent);
        }

        Ok((
            Self {
                host: RefCell::new(host),
                engine: RefCell::new(engine),
                snapshot,
                theme,
                appearance,
                title,
                chrome: WindowChromeState::new(WindowChrome::custom()),
                last_pump: Instant::now(),
                content_size,
                workspace: WorkspaceController::new(),
            },
            Vec::new(),
        ))
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        match message {
            Message::Widget(event) => {
                {
                    let mut host = self.host.borrow_mut();
                    let mut engine = self.engine.borrow_mut();
                    if let Err(err) = host.dispatch_bridge_event(&mut **engine, event) {
                        eprintln!("bridge event failed: {err}");
                        return HostedProgramUpdate::default();
                    }
                    host.prepare_editors();
                    host.prepare_menus();
                    self.snapshot = host.semantic_snapshot();
                }
                self.pull_appearance_from_snapshot();
                HostedProgramUpdate::redraw()
            }
            Message::ToggleTheme => {
                self.theme = self.theme.toggle();
                let label = match self.theme {
                    ThemeMode::Light => "light",
                    ThemeMode::Dark => "dark",
                };
                {
                    let mut host = self.host.borrow_mut();
                    let mut engine = self.engine.borrow_mut();
                    if let Ok(force) = engine.resolve_function("__nanaForceTheme") {
                        let _ = engine.invoke(force, &[nana_js_engine::HostValue::string(label)]);
                        let _ = engine.run_microtasks();
                    } else if let Ok(force) = engine.resolve_function("__nanaLiliaForceTheme") {
                        let _ = engine.invoke(force, &[nana_js_engine::HostValue::string(label)]);
                        let _ = engine.run_microtasks();
                    }
                    if let Err(err) = host.inject_theme(&mut **engine, self.theme) {
                        eprintln!("theme inject failed: {err}");
                    }
                    for _ in 0..8 {
                        let _ = host.pump_frame(&mut **engine);
                    }
                    host.prepare_editors();
                    host.prepare_menus();
                    self.snapshot = host.semantic_snapshot();
                }
                self.pull_appearance_from_snapshot();
                HostedProgramUpdate::redraw()
            }
            Message::Chrome(event) => {
                if let Some(action) = self.chrome.update(event) {
                    return HostedProgramUpdate::with_window_action(action);
                }
                HostedProgramUpdate::default()
            }
            Message::Workspace(action) => {
                self.workspace.update(action);
                HostedProgramUpdate::redraw()
            }
        }
    }

    fn view(&self, native_material: bool) -> Element<'static, Self::Message> {
        // `native_material` is the host MaterialOutcome; Appearance shapes region alphas.
        let tokens = ThemeTokens::new(self.theme.colors(), self.appearance.metrics())
            .with_workspace_corners(self.appearance.workspace_corners_enabled())
            .with_backdrop(
                native_material,
                self.appearance.backdrop_target(),
                self.appearance.backdrop_opacity(),
                self.appearance.titlebar_follows_sidebar(),
            );
        let title = self.title.clone();

        // Owned title String → Cow satisfies Hosted Element<'static> (no Box::leak).
        let title_bar = AppTitleBar::new(title, tokens)
            .window_chrome(&self.chrome, Message::Chrome)
            .trailing(
                Button::label("主题")
                    .kind(ButtonKind::Text)
                    .on_press(Message::ToggleTheme)
                    .view(tokens),
            )
            .view();

        let host = self.host.borrow();
        // Exclusive projections: never paint the full forest in Primary *and* a
        // Navigation/Inspector slice of the same ids (that was the double-view bug).
        let views = self.snapshot.region_views_limited(12, 8);
        debug_assert!(
            views.overlapping_ids().is_empty(),
            "semantic region views overlap: {:?}",
            views.overlapping_ids()
        );

        let primary = view_semantic_tree_static_with_editors(
            &views.primary,
            tokens,
            Some(self.content_size),
            Some(host.editors()),
            Some(host.menus()),
            Message::Widget,
        );

        let mut shell = DesktopShell::new(
            title_bar,
            self.workspace.clone(),
            primary,
            Message::Workspace,
            tokens,
        );

        // Mount Navigation / Inspector only when the forest explicitly tags that
        // region. Untagged content stays in Primary — no demo bridge-facts panel.
        if !views.navigation.widgets.is_empty() {
            let navigation = view_semantic_tree_static_with_editors(
                &views.navigation,
                tokens,
                Some((220.0, self.content_size.1)),
                Some(host.editors()),
                Some(host.menus()),
                Message::Widget,
            );
            shell = shell.navigation(navigation);
        }

        if !views.inspector.widgets.is_empty() {
            let inspector = view_semantic_tree_static_with_editors(
                &views.inspector,
                tokens,
                Some((280.0, self.content_size.1)),
                Some(host.editors()),
                Some(host.menus()),
                Message::Widget,
            );
            shell = shell.inspector(inspector);
        }

        shell.view()
    }

    fn theme_mode(&self) -> ThemeMode {
        self.theme
    }

    fn window_material_mode(&self) -> WindowMaterialMode {
        self.appearance.window_material()
    }

    fn backdrop_opacity(&self) -> f32 {
        self.appearance.backdrop_opacity()
    }

    fn backdrop_target(&self) -> BackdropTarget {
        self.appearance.backdrop_target()
    }

    fn titlebar_follows_sidebar(&self) -> bool {
        self.appearance.titlebar_follows_sidebar()
    }

    fn window_event(
        &mut self,
        event: HostedWindowEvent,
        context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        match event {
            HostedWindowEvent::CloseRequested { .. } => HostedProgramUpdate::exit(),
            HostedWindowEvent::Ready { .. } | HostedWindowEvent::Resized { .. } => {
                let logical = context.logical_size();
                let w = logical.width.max(1.0) as u32;
                let h = (logical.height - TITLE_BAR_HEIGHT).max(1.0) as u32;
                let scale = context.scale_factor();
                self.content_size = (w as f32, h as f32);
                self.workspace.update(WorkspaceAction::WindowResized {
                    width: logical.width.max(1.0),
                    height: logical.height.max(1.0),
                });
                self.workspace
                    .update(WorkspaceAction::WindowScaleFactorChanged(scale));
                {
                    let mut host = self.host.borrow_mut();
                    let mut engine = self.engine.borrow_mut();
                    host.set_viewport(w, h, scale);
                    let _ = host.pump_frame(&mut **engine);
                    // Content viewport → JS window.innerWidth/Height + `resize` EventTarget.
                    let _ = host.pump_lifecycle(
                        &mut **engine,
                        WindowLifecycleEvent::Resize {
                            width: self.content_size.0 as f64,
                            height: self.content_size.1 as f64,
                        },
                    );
                    host.prepare_editors();
                    host.prepare_menus();
                    self.snapshot = host.semantic_snapshot();
                }
                self.pull_appearance_from_snapshot();
                HostedProgramUpdate::redraw()
            }
            HostedWindowEvent::FocusChanged { focused, .. } => {
                {
                    let mut host = self.host.borrow_mut();
                    let mut engine = self.engine.borrow_mut();
                    let lifecycle = if focused {
                        WindowLifecycleEvent::Focus
                    } else {
                        WindowLifecycleEvent::Blur
                    };
                    let _ = host.pump_lifecycle(&mut **engine, lifecycle);
                    let _ = host.pump_frame(&mut **engine);
                    host.prepare_editors();
                    host.prepare_menus();
                    self.snapshot = host.semantic_snapshot();
                }
                self.pull_appearance_from_snapshot();
                HostedProgramUpdate::redraw()
            }
            HostedWindowEvent::VisibilityChanged { hidden, .. } => {
                {
                    let mut host = self.host.borrow_mut();
                    let mut engine = self.engine.borrow_mut();
                    let _ = host.pump_lifecycle(
                        &mut **engine,
                        WindowLifecycleEvent::VisibilityChange { hidden },
                    );
                    let _ = host.pump_frame(&mut **engine);
                    host.prepare_editors();
                    host.prepare_menus();
                    self.snapshot = host.semantic_snapshot();
                }
                self.pull_appearance_from_snapshot();
                HostedProgramUpdate::redraw()
            }
            _ => HostedProgramUpdate::default(),
        }
    }

    fn next_wakeup(&self) -> Option<Instant> {
        Some(self.last_pump + Duration::from_millis(33))
    }

    fn wake(
        &mut self,
        _now: Instant,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        {
            let mut host = self.host.borrow_mut();
            let mut engine = self.engine.borrow_mut();
            let fired = host.pump_frame(&mut **engine).unwrap_or(0);
            if fired == 0 {
                self.last_pump = Instant::now();
                return HostedProgramUpdate::default();
            }
            host.prepare_editors();
            host.prepare_menus();
            self.snapshot = host.semantic_snapshot();
        }
        self.pull_appearance_from_snapshot();
        self.last_pump = Instant::now();
        HostedProgramUpdate::redraw()
    }
}

impl LoaderProgram {
    fn pull_appearance_from_snapshot(&mut self) {
        let mut doc = self.snapshot.appearance;
        // Keep the hosted translucent seed only while L1 has not written `backdrop`.
        let dataset_has_backdrop = self
            .host
            .try_borrow()
            .ok()
            .and_then(|host| {
                host.web_api()
                    .lock()
                    .ok()
                    .map(|web| web.document_dataset().contains_key("backdrop"))
            })
            .unwrap_or(false);
        if !dataset_has_backdrop && doc.window_material() == WindowMaterialMode::Solid {
            let _ = doc.set_window_material(self.appearance.window_material());
        }
        self.appearance = doc;
        // JS Appearance theme toggle writes `dataset.theme`; bridge syncs it into
        // the snapshot — pull so Iced tokens follow without a host ToggleTheme.
        self.theme = self.snapshot.theme;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::text;
    use nana_ui_vue::{
        SemanticWidget, WidgetId, WidgetKind, WidgetProps, view_semantic_tree_static,
    };

    #[test]
    fn desktop_shell_paints_exclusive_region_views_once() {
        let tokens = ThemeMode::Light.tokens();
        let workspace = WorkspaceController::new();
        let snap = SemanticSnapshot {
            revision: 0,
            theme: ThemeMode::Light,
            appearance: AppearanceSettings::default(),
            roots: vec![1],
            widgets: vec![
                SemanticWidget {
                    id: 1,
                    kind: WidgetKind::Row,
                    props: WidgetProps::default(),
                    children: vec![2, 3],
                    parent: None,
                },
                SemanticWidget {
                    id: 2,
                    kind: WidgetKind::Column,
                    props: WidgetProps {
                        agent_id: "app.workspace.navigation".into(),
                        ..WidgetProps::default()
                    },
                    children: vec![],
                    parent: Some(1),
                },
                SemanticWidget {
                    id: 3,
                    kind: WidgetKind::Column,
                    props: WidgetProps {
                        label: "main".into(),
                        ..WidgetProps::default()
                    },
                    children: vec![],
                    parent: Some(1),
                },
            ],
        };
        let views = snap.region_views();
        assert!(views.overlapping_ids().is_empty());
        assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
        assert!(views.primary.widgets.iter().any(|w| w.id == 3));
        assert!(!views.primary.widgets.iter().any(|w| w.id == 2));

        let primary = view_semantic_tree_static(&views.primary, tokens, Message::Widget);
        let navigation = view_semantic_tree_static(&views.navigation, tokens, Message::Widget);
        let title_bar = AppTitleBar::new(String::from("test"), tokens).view();
        // Tagged inspector content mounts; untagged forests omit `.inspector(...)`.
        let inspector: Element<'_, Message> = text("inspector").into();
        let _shell: Element<'static, Message> =
            DesktopShell::new(title_bar, workspace, primary, Message::Workspace, tokens)
                .navigation(navigation)
                .inspector(inspector)
                .view();
    }

    #[test]
    fn untagged_shell_omits_inspector_slot() {
        let tokens = ThemeMode::Light.tokens();
        let workspace = WorkspaceController::new();
        let snap = SemanticSnapshot {
            revision: 0,
            theme: ThemeMode::Light,
            appearance: AppearanceSettings::default(),
            roots: vec![1],
            widgets: vec![SemanticWidget {
                id: 1 as WidgetId,
                kind: WidgetKind::Column,
                props: WidgetProps {
                    label: "main".into(),
                    ..WidgetProps::default()
                },
                children: vec![],
                parent: None,
            }],
        };
        let views = snap.region_views();
        assert!(views.inspector.widgets.is_empty());
        let primary = view_semantic_tree_static(&views.primary, tokens, Message::Widget);
        let title_bar = AppTitleBar::new(String::from("test"), tokens).view();
        let _shell: Element<'static, Message> =
            DesktopShell::new(title_bar, workspace, primary, Message::Workspace, tokens).view();
    }

    #[test]
    fn untagged_forest_stays_in_primary_only() {
        let snap = SemanticSnapshot {
            revision: 0,
            theme: ThemeMode::Light,
            appearance: AppearanceSettings::default(),
            roots: vec![1],
            widgets: vec![SemanticWidget {
                id: 1 as WidgetId,
                kind: WidgetKind::SidebarFrame,
                props: WidgetProps::default(),
                children: vec![],
                parent: None,
            }],
        };
        let views = snap.region_views();
        assert!(views.navigation.widgets.is_empty());
        assert!(views.inspector.widgets.is_empty());
        assert_eq!(views.primary.widgets.len(), 1);
        assert!(views.overlapping_ids().is_empty());
    }
}
