//! Release acceptance window for the Vue-first Scene/`run_runtime` host.
//!
//! Pure Vue+JS mode:
//! `cargo run -p vue-hosted-acceptance --locked`
//!
//! Vue + registered native-component probe (JS/Runtime authority; no Iced chrome):
//! `cargo run -p vue-hosted-acceptance --locked -- --hybrid`
//!
//! Frameless client chrome probe (`NanaAppShell` / `nana-app-title-bar`):
//! `cargo run -p vue-hosted-acceptance --locked -- --chrome-probe`

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use nana_js_engine::probe::{VUE_SFC_COMPAT_CSS, vue_sfc_compat_artifact};
use nana_js_engine::{HostApiRegistry, HostValue, JsEngine};
use nana_ui::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeWindowSettings, ThemeMode,
    run_runtime,
};
use nana_ui_platform::{InputEvent, WindowEvent, WindowId};
use nana_ui_runtime::FrameworkError;
use nana_ui_scene::RuntimeDocument;
use nana_ui_vue::{
    BridgeEvent, NativeComponentCommand, NativeComponentDescriptor, NativeComponentFactory,
    NativePropSchema, NativePropType, VueHostedRuntime, VueRuntimeProgram, WidgetId,
};

#[derive(Clone)]
struct AcceptanceProbe {
    calls: Arc<Mutex<BTreeMap<WidgetId, u64>>>,
}

impl NativeComponentFactory for AcceptanceProbe {
    fn command(
        &self,
        command: NativeComponentCommand,
    ) -> Result<HostValue, nana_js_engine::JsException> {
        let mut calls = self
            .calls
            .lock()
            .map_err(|_| nana_js_engine::JsException::new("acceptance probe state poisoned"))?;
        let call_count = calls.entry(command.id).or_default();
        *call_count += 1;
        let score = command
            .args
            .as_object()
            .and_then(|args| args.get("score"))
            .cloned()
            .unwrap_or(HostValue::Null);
        Ok(HostValue::Object(BTreeMap::from([
            ("score".into(), score),
            ("calls".into(), HostValue::Number(*call_count as f64)),
        ])))
    }

    fn unmount(&self, id: WidgetId) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.remove(&id);
        }
    }
}

struct AcceptanceProgram {
    inner: VueRuntimeProgram<nana_js_v8::V8Engine>,
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    let chrome_probe = std::env::args().any(|argument| argument == "--chrome-probe");
    let hybrid = std::env::args().any(|argument| argument == "--hybrid");
    run_runtime::<AcceptanceProgram>(primary_window_settings(chrome_probe, hybrid))
}

fn primary_window_settings(chrome_probe: bool, hybrid: bool) -> RuntimeWindowSettings {
    RuntimeWindowSettings::new(if chrome_probe {
        "NanaUI chrome probe"
    } else if hybrid {
        "NanaUI Vue + native probe acceptance"
    } else {
        "NanaUI pure Vue acceptance"
    })
    .initial_size(1120.0, 760.0)
    .minimum_size(760.0, 520.0)
    .system_caption(!chrome_probe)
}

fn build_runtime(
    gpu: nana_ui::HostedGpuResources,
    hybrid: bool,
    auto_windows: bool,
    input_probe: bool,
    chrome_probe: bool,
    width: u32,
    height: u32,
    scale_factor: f32,
) -> Result<VueHostedRuntime<nana_js_v8::V8Engine>, nana_js_engine::JsEngineError> {
    let mut application_api = HostApiRegistry::new();
    application_api.register("acceptanceMode", move |_| {
        Ok(HostValue::String(
            if chrome_probe {
                "chrome-probe"
            } else {
                match (hybrid, auto_windows) {
                    (true, true) => "hybrid-windows",
                    (true, false) => "hybrid",
                    (false, true) => "pure-windows",
                    (false, false) => "pure",
                }
            }
            .into(),
        ))
    });
    application_api.register("acceptanceInputProbe", move |_| {
        Ok(HostValue::Bool(input_probe))
    });
    let mut runtime = VueHostedRuntime::new(
        nana_js_v8::V8Engine::new(),
        vue_sfc_compat_artifact(),
        application_api,
        width,
        height,
        scale_factor,
    )?;
    if hybrid {
        runtime
            .components()
            .register(
                NativeComponentDescriptor::new(
                    "acceptance-probe",
                    AcceptanceProbe {
                        calls: Arc::new(Mutex::new(BTreeMap::new())),
                    },
                )
                .props(
                    NativePropSchema::default()
                        .property("label", NativePropType::String)
                        .property("score", NativePropType::Number),
                )
                .events(["activated"])
                .commands(["ping"]),
            )
            .map_err(|error| nana_js_engine::JsEngineError::new(error.to_string()))?;
    }
    runtime.bind_host_gpu(gpu)?;
    runtime.inject_stylesheet(VUE_SFC_COMPAT_CSS)?;
    let mount = runtime
        .engine_mut()
        .resolve_function("__nanaHostedAcceptance.mount")?;
    runtime.engine_mut().invoke(mount, &[])?;
    runtime.engine_mut().run_microtasks()?;
    Ok(runtime)
}

impl RuntimeProgram for AcceptanceProgram {
    type Message = BridgeEvent;
    type Error = nana_js_engine::JsEngineError;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let chrome_probe = std::env::args().any(|argument| argument == "--chrome-probe");
        let hybrid = !chrome_probe && std::env::args().any(|argument| argument == "--hybrid");
        let auto_windows =
            !chrome_probe && std::env::args().any(|argument| argument == "--windows");
        let input_probe =
            !chrome_probe && std::env::args().any(|argument| argument == "--input-probe");
        let geometry = context.geometry();
        let runtime = build_runtime(
            context.gpu().clone(),
            hybrid,
            auto_windows,
            input_probe,
            chrome_probe,
            geometry.physical_size.0.max(1),
            geometry.physical_size.1.max(1),
            geometry.scale_factor.max(0.01),
        )?;
        Ok((
            Self {
                inner: VueRuntimeProgram::from_runtime(runtime),
            },
            Vec::new(),
        ))
    }

    fn document(&self, id: WindowId) -> Option<&RuntimeDocument> {
        self.inner.document(id)
    }

    fn document_mut(&mut self, id: WindowId) -> Option<&mut RuntimeDocument> {
        self.inner.document_mut(id)
    }

    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.inner.update(message, context)
    }

    fn theme_mode(&self) -> ThemeMode {
        self.inner.theme_mode()
    }

    fn host_textures(&self, id: WindowId) -> Option<nana_ui::HostTextureRegistry> {
        self.inner.host_textures(id)
    }

    fn prepare_window_frame(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        self.inner.prepare_window_frame(id, context);
    }

    fn take_accessibility_update(
        &mut self,
        id: WindowId,
    ) -> Option<nana_ui_runtime::AccessibilityUpdate> {
        self.inner.take_accessibility_update(id)
    }

    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        self.inner.rebuild_gpu(context);
    }

    fn input_event(
        &mut self,
        id: WindowId,
        event: &InputEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.inner.input_event(id, event, context)
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.inner.window_event(event, context)
    }

    fn next_wakeup(&self) -> Option<std::time::Instant> {
        self.inner.next_wakeup()
    }

    fn wake(
        &mut self,
        now: std::time::Instant,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.inner.wake(now, context)
    }

    fn accessibility_action(
        &mut self,
        id: WindowId,
        request: nana_ui::AccessibilityActionRequest,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.inner.accessibility_action(id, request, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui::HostedGpuResources;
    use nana_ui_platform::WindowGeometry;
    use nana_ui_vue::{VueWindowCommand, VueWindowId, WidgetKind};

    fn gpu() -> HostedGpuResources {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("headless WGPU adapter required for hosted acceptance");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Nana hosted acceptance test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("headless WGPU device");
        HostedGpuResources::from_existing(adapter, Arc::new(device), Arc::new(queue))
    }

    #[test]
    fn real_vue_sfc_mounts_pure_and_registered_native_modes() {
        let gpu = gpu();
        for hybrid in [false, true] {
            let mut runtime =
                build_runtime(gpu.clone(), hybrid, false, false, false, 1120, 760, 1.0).unwrap();
            for _ in 0..24 {
                runtime.pump().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let host = runtime.vue().host(VueWindowId::PRIMARY).unwrap();
            let snapshot = host.lock().unwrap().semantic_snapshot();
            let accessibility = runtime.accessibility_snapshot(WindowId::PRIMARY);
            assert!(
                accessibility
                    .iter()
                    .any(|node| { node.role == nana_ui::AccessibilityRole::TextInput })
            );
            assert!(
                snapshot
                    .widgets
                    .iter()
                    .any(|widget| widget.kind == WidgetKind::Input)
            );
            assert!(
                snapshot
                    .widgets
                    .iter()
                    .any(|widget| { widget.props.element_tag.eq_ignore_ascii_case("canvas") })
            );
            assert!(
                snapshot
                    .widgets
                    .iter()
                    .any(|widget| { widget.props.element_tag.eq_ignore_ascii_case("img") })
            );
            assert!(
                snapshot
                    .widgets
                    .iter()
                    .any(|widget| { widget.props.label.contains("Vue + Canvas + WebGPU") }),
                "WebGPU completion status missing: {snapshot:?}"
            );
            assert_eq!(
                snapshot.widgets.iter().any(|widget| {
                    widget.props.native_component.as_deref() == Some("acceptance-probe")
                }),
                hybrid,
            );
            if hybrid {
                let native = snapshot
                    .widgets
                    .iter()
                    .find(|widget| {
                        widget.props.native_component.as_deref() == Some("acceptance-probe")
                    })
                    .expect("registered native component");
                let first_id = native.id;
                assert_eq!(
                    native
                        .props
                        .native_props
                        .get("score")
                        .and_then(HostValue::as_f64),
                    Some(7.0),
                    "numeric Vue props must remain structured"
                );
                assert!(
                    snapshot
                        .widgets
                        .iter()
                        .any(|widget| { widget.props.label.contains("Vue slot content: 7") })
                );
                assert!(
                    snapshot
                        .widgets
                        .iter()
                        .any(|widget| { widget.props.label.contains("command:7:1") })
                );

                invoke_control(&mut runtime, "pingNative", &[]);
                assert!(
                    snapshot_after_pump(&mut runtime)
                        .widgets
                        .iter()
                        .any(|widget| { widget.props.label.contains("command:7:2") })
                );

                runtime
                    .dispatch_bridge_event(
                        WindowId::PRIMARY,
                        BridgeEvent::Native {
                            id: first_id,
                            name: "activated".into(),
                            payload: HostValue::Object(BTreeMap::from([(
                                "score".into(),
                                HostValue::Number(7.0),
                            )])),
                        },
                    )
                    .unwrap();
                assert!(
                    snapshot_after_pump(&mut runtime)
                        .widgets
                        .iter()
                        .any(|widget| { widget.props.label.contains("event:7") })
                );

                invoke_control(&mut runtime, "setNativeVisible", &[HostValue::Bool(false)]);
                assert!(
                    !snapshot_after_pump(&mut runtime)
                        .widgets
                        .iter()
                        .any(|widget| {
                            widget.props.native_component.as_deref() == Some("acceptance-probe")
                        })
                );
                invoke_control(&mut runtime, "setNativeVisible", &[HostValue::Bool(true)]);
                invoke_control(&mut runtime, "pingNative", &[]);
                let remounted = snapshot_after_pump(&mut runtime);
                let remounted_native = remounted
                    .widgets
                    .iter()
                    .find(|widget| {
                        widget.props.native_component.as_deref() == Some("acceptance-probe")
                    })
                    .expect("remounted native component");
                assert_ne!(remounted_native.id, first_id);
                assert!(
                    remounted
                        .widgets
                        .iter()
                        .any(|widget| { widget.props.label.contains("command:7:1") })
                );
            }
        }
    }

    #[test]
    fn hosted_acceptance_canvas_draws_and_webgpu_stays_below_header() {
        const VIEW_W: f32 = 1120.0;
        const VIEW_H: f32 = 760.0;
        let mut runtime = build_runtime(gpu(), false, false, false, false, 1120, 760, 1.0).unwrap();
        for _ in 0..24 {
            runtime.pump().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let host = runtime.vue().host(VueWindowId::PRIMARY).unwrap();
        let snapshot = host.lock().unwrap().semantic_snapshot();
        let canvases: Vec<_> = snapshot
            .widgets
            .iter()
            .filter(|widget| widget.props.element_tag.eq_ignore_ascii_case("canvas"))
            .collect();
        assert_eq!(canvases.len(), 2);
        assert!(canvases.iter().any(|widget| {
            widget
                .props
                .attrs
                .get("data-nana-canvas")
                .is_some_and(|id| !id.is_empty())
        }));
        let gpu_canvas = canvases
            .iter()
            .find(|widget| {
                widget
                    .props
                    .attrs
                    .get("data-nana-gpu")
                    .is_some_and(|slot| slot.starts_with("webgpu-canvas:"))
            })
            .expect("WebGPU canvas");
        let header = snapshot
            .widgets
            .iter()
            .find(|widget| widget.props.element_tag.eq_ignore_ascii_case("header"))
            .expect("page header");
        let gpu_title = snapshot
            .widgets
            .iter()
            .find(|widget| {
                widget.parent == gpu_canvas.parent
                    && widget.props.element_tag.eq_ignore_ascii_case("h2")
            })
            .expect("WebGPU card title");

        let document = host.lock().unwrap().document();
        let mut document = document.lock().unwrap();
        document
            .runtime_document_mut()
            .flush(
                nana_ui_runtime::LayoutViewport::new(VIEW_W, VIEW_H),
                &mut nana_ui::NanaTextShaper::default(),
            )
            .expect("acceptance document must flush");

        assert!(document.scene().primitives().any(|primitive| {
            matches!(
                &primitive.kind,
                nana_ui_scene::ScenePrimitiveKind::Custom(custom)
                    if custom.renderer.as_ref() == "nana.host-texture"
                        && custom.resource.as_ref().starts_with("canvas:")
            )
        }));

        let gpu_box = document
            .layout_box(nana_ui_vue::NodeHandle(gpu_canvas.id))
            .expect("WebGPU canvas box");
        let header_box = document
            .layout_box(nana_ui_vue::NodeHandle(header.id))
            .expect("header box");
        let title_box = document
            .layout_box(nana_ui_vue::NodeHandle(gpu_title.id))
            .expect("WebGPU title box");
        assert!(gpu_box.height >= 159.5, "got {gpu_box:?}");
        assert!(
            gpu_box.y + 0.5 >= title_box.y + title_box.height
                && gpu_box.y + 0.5 >= header_box.y + header_box.height,
            "WebGPU slot must sit below header and card title, gpu={gpu_box:?} title={title_box:?} header={header_box:?}"
        );
        let gpu_primitive = document
            .scene()
            .primitives()
            .find(|primitive| {
                primitive.node.get() == gpu_canvas.id
                    && matches!(
                        &primitive.kind,
                        nana_ui_scene::ScenePrimitiveKind::Custom(custom)
                            if custom.renderer.as_ref() == "nana.host-texture"
                    )
            })
            .expect("WebGPU HostTexture primitive");
        assert!(
            gpu_primitive.bounds.y + 0.5 >= title_box.y + title_box.height
                && gpu_primitive.bounds.y + 0.5 >= header_box.y + header_box.height,
            "extracted WebGPU primitive must not cover headers, bounds={:?} title={title_box:?} header={header_box:?}",
            gpu_primitive.bounds
        );
    }

    #[test]
    fn real_vue_sfc_mounts_an_auxiliary_window_after_native_ready() {
        let mut application_api = HostApiRegistry::new();
        application_api.register("acceptanceMode", |_| Ok(HostValue::String("pure".into())));
        let mut runtime = VueHostedRuntime::new(
            nana_js_v8::V8Engine::new(),
            vue_sfc_compat_artifact(),
            application_api,
            1120,
            760,
            1.0,
        )
        .unwrap();
        runtime.bind_host_gpu(gpu()).unwrap();
        runtime.inject_stylesheet(VUE_SFC_COMPAT_CSS).unwrap();
        let mount = runtime
            .engine_mut()
            .resolve_function("__nanaHostedAcceptance.mount")
            .unwrap();
        runtime.engine_mut().invoke(mount, &[]).unwrap();
        runtime.engine_mut().run_microtasks().unwrap();
        invoke_control(&mut runtime, "openAuxiliaryWindow", &[]);
        let commands = runtime.drain_window_commands();
        assert!(commands.iter().any(|command| matches!(
            command,
            VueWindowCommand::Open {
                id: VueWindowId(1),
                ..
            }
        )));

        runtime
            .handle_window_event(WindowEvent::Ready {
                id: WindowId(1),
                geometry: WindowGeometry {
                    physical_position: Some((0, 0)),
                    physical_size: (480, 300),
                    logical_position: Some((0.0, 0.0)),
                    logical_size: (480.0, 300.0),
                    scale_factor: 1.0,
                    maximized: false,
                },
            })
            .unwrap();
        for _ in 0..8 {
            runtime.pump().unwrap();
        }

        let snapshot = runtime
            .vue()
            .semantic_snapshot(VueWindowId(1))
            .expect("auxiliary Vue document");
        assert!(
            snapshot
                .widgets
                .iter()
                .any(|widget| { widget.props.label.contains("Vue auxiliary window") }),
            "auxiliary Vue mount missing: {snapshot:?}"
        );
        for _ in 0..32 {
            runtime.pump().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let primary = runtime
            .vue()
            .semantic_snapshot(VueWindowId::PRIMARY)
            .expect("primary Vue document");
        assert!(
            primary
                .widgets
                .iter()
                .any(|widget| { widget.props.label.contains("Vue + Canvas + WebGPU") }),
            "primary WebGPU completion stalled after auxiliary mount: {primary:?}"
        );
        let image = primary
            .widgets
            .iter()
            .find(|widget| widget.props.attrs.contains_key("data-nana-image"))
            .unwrap_or_else(|| panic!("Vue img node missing: {primary:#?}"));
        assert!(
            image.props.attrs.contains_key("data-nana-image"),
            "Vue img did not finish decode: {image:?}"
        );
        assert_eq!(
            image.props.layout.width,
            Some(nana_ui_vue::LengthSpec::Px(96.0)),
            "Vue img stylesheet width was not applied: {image:?}"
        );
        assert_eq!(
            image.props.layout.height,
            Some(nana_ui_vue::LengthSpec::Px(48.0)),
            "Vue img stylesheet height was not applied: {image:?}"
        );
    }

    #[test]
    fn chrome_probe_mounts_app_shell_on_frameless_primary() {
        assert!(!primary_window_settings(true, false).system_caption);
        assert!(primary_window_settings(false, false).system_caption);
        assert!(primary_window_settings(false, true).system_caption);

        let mut runtime = build_runtime(gpu(), false, false, false, true, 1120, 760, 1.0).unwrap();
        for _ in 0..24 {
            runtime.pump().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let snapshot = runtime
            .vue()
            .host(VueWindowId::PRIMARY)
            .unwrap()
            .lock()
            .unwrap()
            .semantic_snapshot();
        assert!(
            snapshot.widgets.iter().any(|widget| {
                widget.kind == WidgetKind::AppShell
                    || widget
                        .props
                        .element_tag
                        .eq_ignore_ascii_case("nana-app-shell")
            }),
            "nana-app-shell missing: {snapshot:?}"
        );
        assert!(
            snapshot.widgets.iter().any(|widget| {
                widget
                    .props
                    .element_tag
                    .eq_ignore_ascii_case("nana-app-title-bar")
                    || widget
                        .props
                        .class_names
                        .iter()
                        .any(|class| class.contains("nana-app-title-bar"))
            }),
            "nana-app-title-bar missing: {snapshot:?}"
        );
        assert!(
            snapshot.widgets.iter().any(|widget| {
                widget.kind == WidgetKind::Input && widget.props.agent_id == "chrome-probe-input"
            }),
            "chrome-probe input missing: {snapshot:?}"
        );
        assert!(
            !runtime
                .drain_window_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    VueWindowCommand::Open {
                        id: VueWindowId(1),
                        ..
                    }
                )),
            "chrome-probe must not open the auxiliary window"
        );
    }

    fn invoke_control(
        runtime: &mut VueHostedRuntime<nana_js_v8::V8Engine>,
        name: &str,
        args: &[HostValue],
    ) {
        let function = runtime
            .engine_mut()
            .resolve_function(&format!("__nanaHostedAcceptanceControl.{name}"))
            .unwrap();
        runtime.engine_mut().invoke(function, args).unwrap();
        for _ in 0..8 {
            runtime.pump().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn snapshot_after_pump(
        runtime: &mut VueHostedRuntime<nana_js_v8::V8Engine>,
    ) -> nana_ui_vue::SemanticSnapshot {
        for _ in 0..4 {
            runtime.pump().unwrap();
        }
        runtime
            .vue()
            .host(VueWindowId::PRIMARY)
            .unwrap()
            .lock()
            .unwrap()
            .semantic_snapshot()
    }
}
