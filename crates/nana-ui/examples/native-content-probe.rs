//! Real Windows DirectComposition acceptance probe; exits after exercising two windows.
//! `--surface-lifecycle` additionally verifies surface and device recreation on one tree.
#[cfg(target_os = "windows")]
mod windows_probe {
    use nana_ui::runtime::{DocumentId, NativeContent, RuntimeDocument, Stack, Text};
    use nana_ui::{
        DocumentAccessError, HostedSurfaceMode, NativeContentRegion, RuntimeProgram,
        RuntimeProgramContext, RuntimeProgramUpdate, RuntimeWindowSettings, WindowsComposition,
        WindowsCompositionRect, WindowsNativeVisual,
    };
    use nana_ui_platform::{WindowCommand, WindowEvent, WindowId};
    use std::{collections::BTreeMap, convert::Infallible, sync::Arc, time::Duration};
    const AUX: WindowId = WindowId(1);
    #[derive(Clone)]
    pub enum Message {
        Resize,
        Visibility(bool),
        Finish,
    }
    pub struct Probe {
        documents: BTreeMap<WindowId, RuntimeDocument>,
        visuals: BTreeMap<WindowId, WindowsNativeVisual>,
        visible: bool,
        seen_primary: bool,
        seen_auxiliary: bool,
    }
    fn document(id: WindowId) -> RuntimeDocument {
        let document_id = DocumentId::new(id.0 + 1).unwrap();
        let mut document = RuntimeDocument::new(document_id);
        document
            .context_mut()
            .build(document_id, |ui| {
                ui.with("root", Stack::fill_column(12.0).padding(16.0), |ui| {
                    ui.child(
                        "heading",
                        Text::new("DirectComposition native-content probe"),
                    );
                    ui.child("native", NativeContent::new("probe-native"));
                    ui.child(
                        "footer",
                        Text::new("NanaUI controls remain on the UI surface"),
                    );
                });
            })
            .unwrap();
        document
    }
    impl RuntimeProgram for Probe {
        type Message = Message;
        type Error = Infallible;
        fn theme_mode(&self) -> nana_ui::ThemeMode {
            nana_ui::ThemeMode::Dark
        }
        fn surface_mode() -> HostedSurfaceMode {
            HostedSurfaceMode::WindowsComposition
        }
        fn initialize(
            context: &RuntimeProgramContext<Message>,
        ) -> Result<(Self, Vec<Message>), Infallible> {
            assert_eq!(
                context.surface_alpha_mode(),
                wgpu::CompositeAlphaMode::PreMultiplied
            );
            assert_eq!(context.gpu().adapter_info().backend, wgpu::Backend::Dx12);
            let sender = context.clone();
            std::thread::spawn(move || {
                for message in [
                    Message::Resize,
                    Message::Visibility(false),
                    Message::Visibility(true),
                    Message::Finish,
                ] {
                    std::thread::sleep(Duration::from_secs(2));
                    sender.dispatch(message);
                }
            });
            Ok((
                Self {
                    documents: BTreeMap::from([
                        (WindowId::PRIMARY, document(WindowId::PRIMARY)),
                        (AUX, document(AUX)),
                    ]),
                    visuals: BTreeMap::new(),
                    visible: true,
                    seen_primary: false,
                    seen_auxiliary: false,
                },
                vec![],
            ))
        }
        fn with_document<R>(
            &self,
            id: WindowId,
            f: impl FnOnce(&RuntimeDocument) -> R,
        ) -> Result<Option<R>, DocumentAccessError> {
            Ok(self.documents.get(&id).map(f))
        }
        fn with_document_mut<R>(
            &mut self,
            id: WindowId,
            f: impl FnOnce(&mut RuntimeDocument) -> R,
        ) -> Result<Option<R>, DocumentAccessError> {
            Ok(self.documents.get_mut(&id).map(f))
        }
        fn update(
            &mut self,
            message: Message,
            _: &RuntimeProgramContext<Message>,
        ) -> RuntimeProgramUpdate {
            match message {
                Message::Resize => RuntimeProgramUpdate {
                    window_commands: vec![WindowCommand::SetBounds {
                        id: AUX,
                        position: (480.0, 180.0),
                        size: (500.0, 380.0),
                    }],
                    ..RuntimeProgramUpdate::redraw_all()
                },
                Message::Visibility(visible) => {
                    self.visible = visible;
                    println!("NATIVE_VISIBLE {visible}");
                    RuntimeProgramUpdate::redraw_all()
                }
                Message::Finish => {
                    assert!(
                        self.seen_primary && self.seen_auxiliary,
                        "both windows must submit native regions"
                    );
                    for visual in self.visuals.values() {
                        visual.remove().unwrap();
                    }
                    self.visuals.clear();
                    println!("RUNTIME_NATIVE_COMPOSITION_PASS");
                    RuntimeProgramUpdate::exit()
                }
            }
        }
        fn native_content_frame(
            &mut self,
            id: WindowId,
            composition: &WindowsComposition,
            regions: &[NativeContentRegion],
            context: &RuntimeProgramContext<Message>,
        ) -> Result<(), String> {
            if regions.is_empty() {
                return Ok(());
            }
            if id == WindowId::PRIMARY {
                self.seen_primary = true;
            } else {
                self.seen_auxiliary = true;
            }
            let visual = match self.visuals.entry(id) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    println!("NATIVE_REGION_READY {} {:?}", id.0, regions[0].bounds);
                    entry.insert(
                        composition
                            .create_native_visual()
                            .map_err(|e| e.to_string())?,
                    )
                }
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            };
            let region = &regions[0];
            let scale = context.geometry().scale_factor;
            let rect = |r: nana_ui_scene::SceneRect| WindowsCompositionRect {
                x: r.x * scale,
                y: r.y * scale,
                width: r.width * scale,
                height: r.height * scale,
            };
            visual
                .set_geometry(rect(region.bounds), Some(rect(region.clip)))
                .map_err(|e| e.to_string())?;
            visual
                .set_visible(self.visible)
                .map_err(|e| e.to_string())?;
            composition.commit().map_err(|e| e.to_string())
        }
        fn window_event(
            &mut self,
            event: WindowEvent,
            _: &RuntimeProgramContext<Message>,
        ) -> RuntimeProgramUpdate {
            match event {
                WindowEvent::Ready {
                    id: WindowId::PRIMARY,
                    ..
                } => RuntimeProgramUpdate {
                    window_commands: vec![WindowCommand::Open {
                        id: AUX,
                        settings: RuntimeWindowSettings::new("Native auxiliary")
                            .initial_size(420.0, 300.0),
                    }],
                    ..RuntimeProgramUpdate::redraw_all()
                },
                WindowEvent::OpenFailed { error, .. } => panic!("{error}"),
                WindowEvent::Closed { id } => {
                    self.visuals.remove(&id);
                    RuntimeProgramUpdate::default()
                }
                WindowEvent::CloseRequested { .. } => RuntimeProgramUpdate::exit(),
                _ => RuntimeProgramUpdate::default(),
            }
        }
    }

    pub fn surface_lifecycle() {
        use nana_ui::{HostedGpuContext, HostedSurfaceFrame};
        use winit::{
            application::ApplicationHandler,
            event::WindowEvent,
            event_loop::{ActiveEventLoop, EventLoop},
            window::{WindowAttributes, WindowId},
        };
        #[derive(Default)]
        struct Lifecycle {
            done: bool,
        }
        impl ApplicationHandler for Lifecycle {
            fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
                if self.done {
                    return;
                }
                self.done = true;
                let window: Arc<dyn winit::window::Window> = Arc::from(
                    event_loop
                        .create_window(
                            WindowAttributes::default().with_title("DComp lifecycle probe"),
                        )
                        .unwrap(),
                );
                let auxiliary: Arc<dyn winit::window::Window> = Arc::from(
                    event_loop
                        .create_window(
                            WindowAttributes::default().with_title("DComp lifecycle auxiliary"),
                        )
                        .unwrap(),
                );
                let mut graphics = pollster::block_on(HostedGpuContext::new_with_surface_mode(
                    window,
                    wgpu::Features::empty(),
                    false,
                    HostedSurfaceMode::WindowsComposition,
                ))
                .unwrap();
                let mut aux = graphics
                    .create_surface_with_mode(
                        auxiliary,
                        false,
                        HostedSurfaceMode::WindowsComposition,
                    )
                    .unwrap();
                let original = graphics.windows_composition().unwrap().clone();
                let visual = original.create_native_visual().unwrap();
                let identity = visual.as_raw();
                visual
                    .set_geometry(
                        WindowsCompositionRect {
                            x: 20.0,
                            y: 20.0,
                            width: 200.0,
                            height: 100.0,
                        },
                        None,
                    )
                    .unwrap();
                visual.set_visible(true).unwrap();
                original.commit().unwrap();
                graphics.recover_surface().unwrap();
                let replacement =
                    pollster::block_on(graphics.recreate(wgpu::Features::empty())).unwrap();
                aux = replacement.recreate_surface(&aux).unwrap();
                graphics = replacement;
                assert_eq!(visual.as_raw(), identity);
                assert_eq!(
                    graphics.windows_composition().unwrap().window_handle(),
                    original.window_handle()
                );
                assert_eq!(
                    graphics.alpha_mode(),
                    wgpu::CompositeAlphaMode::PreMultiplied
                );
                let mut presented = 0;
                for auxiliary in [false, true] {
                    let frame = if auxiliary {
                        graphics.acquire_surface_frame(&mut aux)
                    } else {
                        graphics.acquire_frame()
                    }
                    .unwrap();
                    let HostedSurfaceFrame::Ready(frame) = frame else {
                        panic!("surface must produce a drawable frame");
                    };
                    let view = frame
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    let mut encoder = graphics
                        .resources()
                        .device()
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                    {
                        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color {
                                        r: 0.1,
                                        g: 0.2,
                                        b: 0.3,
                                        a: 1.0,
                                    }),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            ..Default::default()
                        });
                    }
                    graphics.resources().queue().submit([encoder.finish()]);
                    graphics.present(frame);
                    presented += 1;
                }
                visual.remove().unwrap();
                visual.remove().unwrap();
                assert!(visual.set_visible(true).is_err());
                original.commit().unwrap();
                println!(
                    "SURFACE_LIFECYCLE_PASS presented={presented} retained_native_visual=true"
                );
                event_loop.exit();
            }
            fn window_event(&mut self, _: &dyn ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }
        EventLoop::new()
            .unwrap()
            .run_app(Lifecycle::default())
            .unwrap();
    }
}
#[cfg(target_os = "windows")]
fn main() {
    if std::env::args().any(|arg| arg == "--surface-lifecycle") {
        windows_probe::surface_lifecycle();
    } else {
        nana_ui::run_runtime::<windows_probe::Probe>(
            nana_ui::RuntimeWindowSettings::new("NanaUI native composition probe")
                .initial_size(800.0, 600.0),
        )
        .unwrap();
    }
}
#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("DirectComposition is available on Windows only");
}
