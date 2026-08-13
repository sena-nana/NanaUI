//! Vue Counter MVP — JS state bridge + optional NanaUI Iced window.
//!
//! Blitz / paint-stub / paint-vello / CustomContent paths were removed. UI is
//! NanaUI when `--features windowed -- --window` is used; otherwise headless
//! JS probe only.
//!
//! Semantic message bridge (`createWidget` → `MessageBridge` → Iced `Button`):
//! `cargo run -p vue-counter -- counter --semantic --clicks=2`
//!
//! `cargo run -p vue-counter --features windowed -- --window`

#![allow(unexpected_cfgs)]

use std::env;

use nana_js_engine::probe::vue_phase3_artifact;
use nana_js_engine::{HostValue, JsEngine, RuntimeArtifact};
use nana_ui_vue::{BridgeEvent, NodeHandle, VueHost, WidgetKind};

nana_ui_vue::refuse_dual_js_engines!();

/// Minimal JS counter that owns state and drives the Rust semantic bridge.
const SEMANTIC_COUNTER_JS: &str = r#"
(function () {
  let count = 0;
  const host = globalThis.__nanaHost;
  const root = host.call("mountRoot", []);
  const col = host.call("createWidget", ["column", {}]);
  const title = host.call("createWidget", ["text", { label: "Vue Counter · semantic bridge" }]);
  const text = host.call("createWidget", ["text", { label: "count = 0" }]);
  const btn = host.call("createWidget", ["button", { label: "Increment", kind: "primary" }]);
  const reset = host.call("createWidget", ["button", { label: "Reset", kind: "subtle" }]);
  host.call("insert", [col, root, null]);
  host.call("insert", [title, col, null]);
  host.call("insert", [text, col, null]);
  host.call("insert", [btn, col, null]);
  host.call("insert", [reset, col, null]);
  host.call("patchProp", [btn, "onPress", true]);
  host.call("patchProp", [reset, "onPress", true]);

  const listeners = new Map();
  function key(nid, event) { return Number(nid) + ":" + String(event).toLowerCase(); }
  function sync() {
    host.call("patchProp", [text, "label", "count = " + count]);
  }
  listeners.set(key(btn, "press"), function () { count += 1; sync(); });
  listeners.set(key(reset, "press"), function () { count = 0; sync(); });

  globalThis.__nanaFireEvent = function (nid, event, detail) {
    const fn = listeners.get(key(nid, event));
    if (typeof fn === "function") fn(detail || {});
    return true;
  };
  globalThis.__nanaApplyTheme = function (mode) {
    const theme = String(mode || "light").toLowerCase() === "dark" ? "dark" : "light";
    host.call("setDocumentTheme", [theme]);
    return theme;
  };
  globalThis.__nanaSemanticCounter = {
    get count() { return count; },
    get buttonId() { return btn; },
    get resetId() { return reset; },
    get textId() { return text; },
  };
  return { ok: true, app: "semantic-counter", buttonId: btn, resetId: reset, textId: text };
})();
"#;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let app = args
        .iter()
        .find(|a| *a == "todo" || *a == "counter")
        .map(|s| s.as_str())
        .unwrap_or("counter");
    let clicks: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("--clicks=")?.parse().ok())
        .unwrap_or(0);
    let use_bytecode = args.iter().any(|a| *a == "--bytecode");
    let semantic = args.iter().any(|a| *a == "--semantic");

    #[cfg(feature = "windowed")]
    if args.iter().any(|a| a == "--window") {
        if let Err(err) = windowed::run(app) {
            eprintln!("vue-counter windowed failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    let result = if semantic {
        run_semantic(clicks)
    } else {
        run_headless(app, clicks, use_bytecode)
    };

    match result {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("vue-counter failed: {err}");
            std::process::exit(1);
        }
    }
}

fn run_semantic(clicks: usize) -> Result<String, String> {
    let mut host = VueHost::with_viewport(480, 320, 1.0);
    let mut engine = create_engine()?;
    host.attach_engine(&mut *engine)
        .map_err(|e| e.to_string())?;
    engine
        .initialize(RuntimeArtifact::from_source(
            "semantic-counter.js",
            SEMANTIC_COUNTER_JS,
        ))
        .map_err(|e| e.to_string())?;
    host.bind_event_bridge(&mut *engine)
        .map_err(|e| e.to_string())?;
    host.inject_theme(&mut *engine, nana_ui_vue::ThemeMode::Light)
        .map_err(|e| e.to_string())?;

    let snap0 = host.semantic_snapshot();
    let button = snap0
        .widgets
        .iter()
        .find(|w| w.kind == WidgetKind::Button && w.props.label == "Increment")
        .ok_or_else(|| "missing Increment button in semantic snapshot".to_string())?;
    let button_id = button.id;

    for _ in 0..clicks {
        host.dispatch_bridge_event(&mut *engine, BridgeEvent::Press { id: button_id })
            .map_err(|e| e.to_string())?;
    }

    let snap = host.semantic_snapshot();
    let count_label = snap
        .widgets
        .iter()
        .find(|w| w.kind == WidgetKind::Text && w.props.label.starts_with("count ="))
        .map(|w| w.props.label.clone())
        .unwrap_or_default();
    let expected = format!("count = {clicks}");
    if count_label != expected {
        return Err(format!("expected {expected}, got {count_label}"));
    }

    let theme = match snap.theme {
        nana_ui_vue::ThemeMode::Light => "light",
        nana_ui_vue::ThemeMode::Dark => "dark",
    };

    Ok(format!(
        "engine={}\napp=semantic-counter\nok=true\nbridge=message\nrevision={}\nwidgets={}\n{}\nclicks={clicks}\ntheme={theme}",
        engine_label(),
        snap.revision,
        snap.widgets.len(),
        count_label,
    ))
}

fn run_headless(app: &str, clicks: usize, use_bytecode: bool) -> Result<String, String> {
    let mut host = VueHost::with_viewport(800, 600, 1.0);
    let mut engine = create_engine()?;

    if use_bytecode {
        #[cfg(all(feature = "engine-quickjs", not(feature = "engine-v8")))]
        {
            use nana_js_engine::probe::VUE_PHASE3_JS;
            use nana_ui_web_api::WEB_API_SHIM_JS;
            let composed = format!("{WEB_API_SHIM_JS}\n{VUE_PHASE3_JS}");
            let artifact =
                nana_js_quickjs::QuickJsEngine::compile_bytecode("vue-phase3.qbc.js", &composed)
                    .map_err(|e| e.to_string())?;
            host.initialize_with_web_api(&mut *engine, artifact)
                .map_err(|e| e.to_string())?;
            engine.run_microtasks().map_err(|e| e.to_string())?;
        }
        #[cfg(feature = "engine-v8")]
        {
            return Err("--bytecode is QuickJS-only (QuickJsBytecode)".into());
        }
        #[cfg(not(any(feature = "engine-quickjs", feature = "engine-v8")))]
        {
            return Err("no engine feature".into());
        }
    } else {
        host.attach_engine(&mut *engine)
            .map_err(|e| e.to_string())?;
        engine
            .initialize(vue_phase3_artifact())
            .map_err(|e| e.to_string())?;
    }
    host.bind_event_bridge(&mut *engine)
        .map_err(|e| e.to_string())?;

    let entry = match app {
        "todo" => "__nanaVue.runTodo",
        _ => "__nanaVue.runCounter",
    };
    let run = engine.resolve_function(entry).map_err(|e| e.to_string())?;
    let result = engine.invoke(run, &[]).map_err(|e| e.to_string())?;
    engine.run_microtasks().map_err(|e| e.to_string())?;
    host.resolve_layout();

    for i in 0..clicks {
        let (x, y) = {
            let doc = host.document();
            let guard = doc.lock().map_err(|_| "doc poisoned".to_string())?;
            let target = guard
                .snapshot_boxes()
                .event_targets
                .iter()
                .find(|(_, ev)| ev == "click")
                .map(|(id, _)| NodeHandle(*id))
                .and_then(|h| guard.layout_box(h));
            match target {
                Some(b) => (b.x + b.width * 0.5, b.y + b.height * 0.5),
                None => (60.0 + i as f32, 70.0),
            }
        };
        let hit = host
            .pointer_click(&mut *engine, x, y)
            .map_err(|e| e.to_string())?;
        if !hit {
            return Err(format!("click #{i} at ({x:.1},{y:.1}) missed event target"));
        }
        host.resolve_layout();
    }

    let _ = settle_layout_stable(&mut host, &mut *engine, 24)?;

    let snap = {
        let doc = host.document();
        let guard = doc.lock().map_err(|_| "doc poisoned".to_string())?;
        guard.snapshot_boxes()
    };

    let engine_name = engine_label();
    let boxes = snap.boxes.len();
    let texts: Vec<_> = snap.texts.iter().map(|(_, t)| t.clone()).collect();
    let ok = result
        .as_object()
        .and_then(|o| o.get("ok"))
        .and_then(HostValue::as_bool)
        .unwrap_or(false);
    let artifact_kind = if use_bytecode {
        "QuickJsBytecode"
    } else {
        "SourceUtf8"
    };

    Ok(format!(
        "engine={engine_name}\napp={app}\nok={ok}\nartifact={artifact_kind}\nboxes={boxes}\ntexts={texts:?}\nclicks={clicks}\nbridge=message"
    ))
}

fn settle_layout_stable(
    host: &mut VueHost,
    engine: &mut dyn JsEngine,
    max_pumps: usize,
) -> Result<(usize, usize), String> {
    let mut last = usize::MAX;
    let mut stable = 0usize;
    let mut pumps = 0usize;
    for _ in 0..max_pumps {
        engine.run_microtasks().map_err(|e| e.to_string())?;
        let _ = host.pump_frame(engine).map_err(|e| e.to_string())?;
        pumps += 1;
        let boxes = {
            let doc = host.document();
            let guard = doc.lock().map_err(|_| "doc poisoned".to_string())?;
            guard.snapshot_boxes().boxes.len()
        };
        if boxes == last {
            stable += 1;
            if stable >= 3 {
                return Ok((boxes, pumps));
            }
        } else {
            last = boxes;
            stable = 1;
        }
    }
    Ok((last, pumps))
}

fn create_engine() -> Result<Box<dyn JsEngine>, String> {
    #[cfg(all(feature = "engine-quickjs", not(feature = "engine-v8")))]
    {
        Ok(Box::new(nana_js_quickjs::QuickJsEngine::new()))
    }
    #[cfg(all(feature = "engine-v8", not(feature = "engine-quickjs")))]
    {
        Ok(Box::new(nana_js_v8::V8Engine::new()))
    }
    #[cfg(all(feature = "engine-quickjs", feature = "engine-v8"))]
    {
        compile_error!("enable only one of engine-quickjs / engine-v8");
    }
    #[cfg(not(any(feature = "engine-quickjs", feature = "engine-v8")))]
    {
        Err("enable engine-quickjs or engine-v8".into())
    }
}

fn engine_label() -> &'static str {
    #[cfg(all(feature = "engine-quickjs", not(feature = "engine-v8")))]
    {
        "quickjs"
    }
    #[cfg(all(feature = "engine-v8", not(feature = "engine-quickjs")))]
    {
        "v8"
    }
    #[cfg(not(any(feature = "engine-quickjs", feature = "engine-v8")))]
    {
        "none"
    }
}

#[cfg(feature = "windowed")]
mod windowed {
    use std::time::Instant;

    use iced::widget::column;
    use iced::{Element, Length};
    use nana_js_engine::{HostApiRegistry, RuntimeArtifact};
    use nana_ui::{
        AppearanceSettings, Button, HostedInputDisposition, HostedInputEvent, HostedProgram,
        HostedProgramContext, HostedProgramUpdate, HostedRunError, HostedRuntimeEvent,
        HostedWindowEvent, HostedWindowId, HostedWindowSettings, ThemeMode, ThemeModeExt,
        ThemeTokens, WindowMaterialMode, run_hosted,
    };
    use nana_ui_vue::{BridgeEvent, VueHostedRuntime};

    use super::{SEMANTIC_COUNTER_JS, engine_label};

    fn hosted_appearance() -> AppearanceSettings {
        let mut appearance = AppearanceSettings::default();
        // Match HostedProgram default: translucent material + documented opacity.
        let _ = appearance.set_window_material(WindowMaterialMode::Translucent);
        appearance
    }

    #[derive(Debug, Clone)]
    enum Message {
        Widget(BridgeEvent),
        ToggleTheme,
    }

    #[cfg(all(feature = "engine-quickjs", not(feature = "engine-v8")))]
    type CounterEngine = nana_js_quickjs::QuickJsEngine;
    #[cfg(all(feature = "engine-v8", not(feature = "engine-quickjs")))]
    type CounterEngine = nana_js_v8::V8Engine;

    struct CounterProgram {
        runtime: VueHostedRuntime<CounterEngine>,
        theme: ThemeMode,
        appearance: AppearanceSettings,
    }

    impl CounterProgram {
        fn theme_tokens(&self, native_material: bool) -> ThemeTokens {
            ThemeTokens::new(self.theme.colors(), self.appearance.metrics())
                .with_workspace_corners(self.appearance.workspace_corners_enabled())
                .with_backdrop(
                    native_material,
                    self.appearance.backdrop_target(),
                    self.appearance.backdrop_opacity(),
                    self.appearance.titlebar_follows_sidebar(),
                )
        }
    }

    pub fn run(_app: &str) -> Result<(), HostedRunError> {
        let title = format!("Vue Counter NanaUI bridge ({})", engine_label());
        run_hosted::<CounterProgram>(
            HostedWindowSettings::new(title)
                .initial_size(480.0, 360.0)
                .minimum_size(360.0, 240.0),
        )
    }

    impl HostedProgram for CounterProgram {
        type Message = Message;
        type Error = String;

        fn initialize(
            context: &HostedProgramContext<Self::Message>,
        ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
            #[cfg(all(feature = "engine-quickjs", not(feature = "engine-v8")))]
            let engine = nana_js_quickjs::QuickJsEngine::new();
            #[cfg(all(feature = "engine-v8", not(feature = "engine-quickjs")))]
            let engine = nana_js_v8::V8Engine::new();
            let geometry = context.geometry();
            let mut runtime = VueHostedRuntime::new(
                engine,
                RuntimeArtifact::from_source("semantic-counter.js", SEMANTIC_COUNTER_JS),
                HostApiRegistry::new(),
                geometry.physical_size.width.max(1),
                geometry.physical_size.height.max(1),
                geometry.scale_factor.max(0.01),
            )
            .map_err(|e| e.to_string())?;
            runtime
                .bind_host_gpu(context.gpu().clone())
                .map_err(|e| e.to_string())?;
            let theme = ThemeMode::Light;
            runtime.inject_theme(theme).map_err(|e| e.to_string())?;

            Ok((
                Self {
                    runtime,
                    theme,
                    appearance: hosted_appearance(),
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
                    if let Err(err) = self
                        .runtime
                        .dispatch_bridge_event(HostedWindowId::PRIMARY, event)
                    {
                        eprintln!("bridge event failed: {err}");
                        return HostedProgramUpdate::default();
                    }
                    self.runtime.hosted_wake()
                }
                Message::ToggleTheme => {
                    self.theme = self.theme.toggle();
                    if let Err(err) = self.runtime.inject_theme(self.theme) {
                        eprintln!("theme inject failed: {err}");
                    }
                    self.runtime.hosted_wake()
                }
            }
        }

        fn view(&self, native_material: bool) -> Element<'static, Self::Message> {
            let tokens = self.theme_tokens(native_material);
            column![
                self.runtime
                    .view_window(HostedWindowId::PRIMARY, native_material)
                    .unwrap_or_else(|_| iced::widget::Space::new().into())
                    .map(Message::Widget),
                Button::label("Toggle theme")
                    .kind(nana_ui::ButtonKind::Text)
                    .on_press(Message::ToggleTheme)
                    .view(tokens),
            ]
            .spacing(8)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
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

        fn window_event(
            &mut self,
            event: HostedWindowEvent,
            _context: &HostedProgramContext<Self::Message>,
        ) -> HostedProgramUpdate {
            self.runtime.hosted_window_event(event)
        }

        fn input_event(
            &mut self,
            id: HostedWindowId,
            event: HostedInputEvent,
            _context: &HostedProgramContext<Self::Message>,
        ) -> (HostedInputDisposition, HostedProgramUpdate) {
            self.runtime.hosted_input(id, event)
        }

        fn runtime_event(
            &mut self,
            event: HostedRuntimeEvent,
            _context: &HostedProgramContext<Self::Message>,
        ) -> HostedProgramUpdate {
            self.runtime.hosted_runtime_event(event)
        }

        fn next_wakeup(&self) -> Option<Instant> {
            self.runtime.next_wakeup()
        }

        fn wake(
            &mut self,
            _now: Instant,
            _context: &HostedProgramContext<Self::Message>,
        ) -> HostedProgramUpdate {
            self.runtime.hosted_wake()
        }

        fn rebuild_gpu(&mut self, context: &HostedProgramContext<Self::Message>) {
            let _ = self.runtime.hosted_rebuild_gpu(context.gpu().clone());
        }
    }
}
