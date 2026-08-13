//! Release acceptance window for the Vue-first hosted runtime.
//!
//! Pure Vue+JS mode:
//! `cargo run -p vue-hosted-acceptance --locked`
//!
//! Vue + registered Rust/Iced component mode:
//! `cargo run -p vue-hosted-acceptance --locked -- --hybrid`

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iced::widget::{Column, button, text};
use iced::{Element, Length};
use nana_js_engine::probe::{VUE_SFC_COMPAT_CSS, vue_sfc_compat_artifact};
use nana_js_engine::{HostApiRegistry, HostValue, JsEngine};
use nana_ui::{
    HostedInputDisposition, HostedInputEvent, HostedProgram, HostedProgramContext,
    HostedProgramUpdate, HostedRuntimeEvent, HostedWindowEvent, HostedWindowId,
    HostedWindowSettings, ThemeMode, WindowMaterialMode, run_hosted_with,
};
use nana_ui_vue::{
    BridgeEvent, NativeComponentCommand, NativeComponentContext, NativeComponentDescriptor,
    NativeComponentFactory, NativePropSchema, NativePropType, VueHostedRuntime, WidgetId,
};

#[derive(Clone)]
struct AcceptanceProbe {
    calls: Arc<Mutex<BTreeMap<WidgetId, u64>>>,
}

impl NativeComponentFactory for AcceptanceProbe {
    fn view(
        &self,
        context: NativeComponentContext,
        children: Vec<Element<'static, BridgeEvent>>,
    ) -> Result<Element<'static, BridgeEvent>, nana_js_engine::JsException> {
        let label = context
            .props
            .get("label")
            .and_then(HostValue::as_str)
            .unwrap_or("probe")
            .to_owned();
        let score = context
            .props
            .get("score")
            .and_then(HostValue::as_f64)
            .unwrap_or_default();
        let activated = context.event(
            "activated",
            HostValue::Object(BTreeMap::from([("score".into(), HostValue::Number(score))])),
        )?;
        let mut content = Column::with_children(children)
            .spacing(8)
            .width(Length::Fill);
        content = content.push(text(format!(
            "Iced owns this component: {label} ({score:.0})"
        )));
        content = content.push(button("Emit native event").on_press(activated));
        Ok(content.into())
    }

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
    runtime: VueHostedRuntime<nana_js_v8::V8Engine>,
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    nana_ui_vue::refuse_dual_js_engines!();
    let hybrid = std::env::args().any(|argument| argument == "--hybrid");
    let auto_windows = std::env::args().any(|argument| argument == "--windows");
    let input_probe = std::env::args().any(|argument| argument == "--input-probe");
    let alpha_probe = std::env::args().any(|argument| argument == "--alpha-probe");
    run_hosted_with::<AcceptanceProgram, _>(
        HostedWindowSettings::new(if hybrid {
            "NanaUI Vue + Iced acceptance"
        } else {
            "NanaUI pure Vue acceptance"
        })
        .initial_size(1120.0, 760.0)
        .minimum_size(760.0, 520.0)
        .transparent_background(alpha_probe),
        move |context| {
            if alpha_probe {
                eprintln!(
                    "NanaUI transparent surface alpha mode: {:?}",
                    context.surface_alpha_mode()
                );
            }
            AcceptanceProgram::initialize_with_mode(context, hybrid, auto_windows, input_probe)
        },
    )
}

impl AcceptanceProgram {
    fn initialize_with_mode(
        context: &HostedProgramContext<BridgeEvent>,
        hybrid: bool,
        auto_windows: bool,
        input_probe: bool,
    ) -> Result<(Self, Vec<BridgeEvent>), nana_js_engine::JsEngineError> {
        let geometry = context.geometry();
        let runtime = build_runtime(
            context.gpu().clone(),
            hybrid,
            auto_windows,
            input_probe,
            geometry.physical_size.width.max(1),
            geometry.physical_size.height.max(1),
            geometry.scale_factor.max(0.01),
        )?;
        Ok((Self { runtime }, Vec::new()))
    }
}

fn build_runtime(
    gpu: nana_ui::HostedGpuResources,
    hybrid: bool,
    auto_windows: bool,
    input_probe: bool,
    width: u32,
    height: u32,
    scale_factor: f32,
) -> Result<VueHostedRuntime<nana_js_v8::V8Engine>, nana_js_engine::JsEngineError> {
    let mut application_api = HostApiRegistry::new();
    application_api.register("acceptanceMode", move |_| {
        Ok(HostValue::String(
            match (hybrid, auto_windows) {
                (true, true) => "hybrid-windows",
                (true, false) => "hybrid",
                (false, true) => "pure-windows",
                (false, false) => "pure",
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

impl HostedProgram for AcceptanceProgram {
    type Message = BridgeEvent;
    type Error = nana_js_engine::JsEngineError;

    fn initialize(
        _context: &HostedProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        Err(nana_js_engine::JsEngineError::new(
            "use the mode-aware hosted initializer",
        ))
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        match self
            .runtime
            .dispatch_bridge_event(HostedWindowId::PRIMARY, message)
        {
            Ok(_) => self.runtime.hosted_wake(),
            Err(error) => {
                eprintln!("acceptance event failed: {error}");
                HostedProgramUpdate::default()
            }
        }
    }

    fn view(&self, native_material: bool) -> Element<'static, Self::Message> {
        self.view_window(HostedWindowId::PRIMARY, native_material)
    }

    fn view_window(
        &self,
        id: HostedWindowId,
        native_material: bool,
    ) -> Element<'static, Self::Message> {
        self.runtime
            .view_window(id, native_material)
            .unwrap_or_else(|error| {
                eprintln!("acceptance view for window {} failed: {error}", id.0);
                iced::widget::Space::new().into()
            })
    }

    fn theme_mode(&self) -> ThemeMode {
        ThemeMode::Light
    }

    fn window_material_mode(&self) -> WindowMaterialMode {
        WindowMaterialMode::Solid
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

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::executor;
    use iced::wgpu;
    use nana_ui::HostedGpuResources;
    use nana_ui::HostedWindowGeometry;
    use nana_ui_vue::{VueWindowId, WidgetKind};

    fn gpu() -> HostedGpuResources {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("headless WGPU adapter required for hosted acceptance");
        let (device, queue) = executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
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
    fn real_vue_sfc_mounts_pure_and_registered_iced_modes() {
        let gpu = gpu();
        for hybrid in [false, true] {
            let mut runtime =
                build_runtime(gpu.clone(), hybrid, false, false, 1120, 760, 1.0).unwrap();
            for _ in 0..24 {
                runtime.pump().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let host = runtime.vue().host(VueWindowId::PRIMARY).unwrap();
            let snapshot = host.lock().unwrap().semantic_snapshot();
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
                        HostedWindowId::PRIMARY,
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
            nana_ui::HostedWindowCommand::Open {
                id: HostedWindowId(1),
                ..
            }
        )));

        runtime.hosted_window_event(HostedWindowEvent::Ready {
            id: HostedWindowId(1),
            window_id: iced::window::Id::unique(),
            geometry: HostedWindowGeometry {
                physical_position: Some((0, 0)),
                physical_size: iced::Size::new(480, 300),
                logical_position: Some(iced::Point::ORIGIN),
                logical_size: iced::Size::new(480.0, 300.0),
                scale_factor: 1.0,
                maximized: false,
            },
        });
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
