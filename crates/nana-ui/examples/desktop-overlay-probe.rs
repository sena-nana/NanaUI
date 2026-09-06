//! Native two-window acceptance probe. Commands on stdin: lock, unlock, close, quit.
//! Run through the Scene host; inspect output plus native pointer/compositor behavior.
use nana_ui::runtime::{DocumentId, FrameworkError, RuntimeDocument, Stack, Text};
use nana_ui::{
    DocumentAccessError, MaterialEffect, RoutedInput, RuntimeProgram, RuntimeProgramContext,
    RuntimeProgramUpdate, RuntimeWindowSettings, ThemeMode, run_runtime,
};
use nana_ui_platform::{
    InputEvent, PointerPhase, WindowCommand, WindowEvent, WindowId, WindowRole,
};
use std::{
    convert::Infallible,
    io::{self, BufRead, Write},
};

const OVERLAY: WindowId = WindowId(1);
#[derive(Clone)]
enum Message {
    Lock(bool),
    Close,
    Fail,
    Quit,
}
struct Probe {
    primary: RuntimeDocument,
    overlay: RuntimeDocument,
}
fn report(value: serde_json::Value) {
    println!("{value}");
    io::stdout().flush().unwrap();
}
fn commands(window_commands: Vec<WindowCommand>) -> RuntimeProgramUpdate {
    RuntimeProgramUpdate {
        window_commands,
        ..RuntimeProgramUpdate::redraw_all()
    }
}
impl RuntimeProgram for Probe {
    type Message = Message;
    type Error = Infallible;
    fn initialize(
        context: &RuntimeProgramContext<Message>,
    ) -> Result<(Self, Vec<Message>), Infallible> {
        let primary_id = DocumentId::new(1).unwrap();
        let mut primary = RuntimeDocument::new(primary_id);
        primary
            .context_mut()
            .build(primary_id, |ui| {
                ui.child(
                    "native-composition-reference",
                    Stack::fill_column(0.0).with_layout(|layout| {
                        layout.background = Some([0.0, 1.0, 0.0, 1.0]);
                    }),
                );
            })
            .unwrap();
        let id = DocumentId::new(2).unwrap();
        let mut overlay = RuntimeDocument::new(id);
        overlay
            .context_mut()
            .build(id, |ui| {
                ui.child("contrast", Text::new("Native overlay pointer target"));
            })
            .unwrap();
        let context = context.clone();
        std::thread::spawn(move || {
            for line in io::stdin().lock().lines().map_while(Result::ok) {
                let message = match line.trim() {
                    "lock" => Message::Lock(true),
                    "unlock" => Message::Lock(false),
                    "close" => Message::Close,
                    "fail" => Message::Fail,
                    "quit" => Message::Quit,
                    _ => continue,
                };
                context.dispatch(message);
            }
        });
        Ok((Self { primary, overlay }, vec![]))
    }
    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok(match id {
            WindowId::PRIMARY => Some(f(&self.primary)),
            OVERLAY => Some(f(&self.overlay)),
            _ => None,
        })
    }
    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok(match id {
            WindowId::PRIMARY => Some(f(&mut self.primary)),
            OVERLAY => Some(f(&mut self.overlay)),
            _ => None,
        })
    }
    fn update(
        &mut self,
        message: Message,
        _: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        match message {
            Message::Lock(enabled) => commands(vec![WindowCommand::SetMousePassthrough {
                id: OVERLAY,
                enabled,
            }]),
            Message::Close => commands(vec![WindowCommand::Close(OVERLAY)]),
            Message::Fail => {
                let mut settings = RuntimeWindowSettings::new("invalid modal");
                settings.modal = true;
                commands(vec![WindowCommand::Open {
                    id: WindowId(2),
                    settings,
                }])
            }
            Message::Quit => RuntimeProgramUpdate::exit(),
        }
    }
    fn theme_mode(&self) -> ThemeMode {
        ThemeMode::Dark
    }
    fn window_material_mode_for(&self, id: WindowId) -> MaterialEffect {
        if id == OVERLAY {
            MaterialEffect::Transparent
        } else {
            MaterialEffect::Solid
        }
    }
    fn appearance_backdrop_opacity_for(&self, _: WindowId) -> f32 {
        0.0
    }
    fn window_event(
        &mut self,
        event: WindowEvent,
        context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::Ready {
                id: WindowId::PRIMARY,
                ..
            } => {
                let mut settings = RuntimeWindowSettings::new("NanaUI overlay probe layer")
                    .initial_size(420.0, 300.0)
                    .minimum_size(280.0, 240.0);
                settings.initial_position = Some((200.0, 200.0));
                settings.transparent = true;
                settings.always_on_top = true;
                settings.focus_on_show = false;
                settings.constrain_to_work_area = true;
                settings.role = WindowRole::Tool;
                assert_eq!(context.material().effect, MaterialEffect::Solid);
                report(serde_json::json!({"event":"primary_ready"}));
                commands(vec![WindowCommand::Open {
                    id: OVERLAY,
                    settings,
                }])
            }
            WindowEvent::Ready {
                id: OVERLAY,
                geometry,
            } => {
                assert_eq!(context.material().effect, MaterialEffect::Transparent);
                assert_ne!(
                    context.surface_alpha_mode(),
                    wgpu::CompositeAlphaMode::Opaque
                );
                report(
                    serde_json::json!({"event":"ready", "window":OVERLAY.0, "scale":geometry.scale_factor}),
                );
                commands(vec![WindowCommand::SetMousePassthrough {
                    id: WindowId(99),
                    enabled: true,
                }])
            }
            WindowEvent::MousePassthroughChanged {
                id,
                enabled,
                result,
            } => {
                if id == WindowId(99) {
                    assert!(result.is_err());
                } else {
                    assert!(result.is_ok(), "{result:?}");
                }
                report(
                    serde_json::json!({"event":"passthrough", "window":id.0, "enabled":enabled, "success":result.is_ok()}),
                );
                if id == OVERLAY && !enabled && result.is_ok() {
                    commands(vec![WindowCommand::Focus(OVERLAY)])
                } else {
                    RuntimeProgramUpdate::default()
                }
            }
            WindowEvent::Closed { id } => {
                report(serde_json::json!({"event":"closed", "window":id.0}));
                RuntimeProgramUpdate::default()
            }
            WindowEvent::OpenFailed {
                id: WindowId(2), ..
            } => {
                report(serde_json::json!({"event":"open_failed", "window":2}));
                RuntimeProgramUpdate::default()
            }
            WindowEvent::OpenFailed { error, .. } => panic!("{error}"),
            WindowEvent::CloseRequested {
                id: WindowId::PRIMARY,
            } => RuntimeProgramUpdate::exit(),
            WindowEvent::CloseRequested { id } => commands(vec![WindowCommand::Close(id)]),
            _ => RuntimeProgramUpdate::default(),
        }
    }
    fn input_event(
        &mut self,
        id: WindowId,
        input: RoutedInput<'_>,
        _: &RuntimeProgramContext<Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let event = input.event;
        if let InputEvent::Pointer {
            phase: PointerPhase::Down,
            x,
            y,
            ..
        } = event
        {
            report(serde_json::json!({"event":"pointer_down", "window":id.0, "x":x, "y":y}));
        }
        Ok(RuntimeProgramUpdate::default())
    }
}
fn main() -> Result<(), nana_ui::HostedRunError> {
    let mut settings = RuntimeWindowSettings::new("NanaUI overlay probe base")
        .initial_size(800.0, 600.0)
        .minimum_size(600.0, 400.0);
    settings.initial_position = Some((100.0, 100.0));
    run_runtime::<Probe>(settings)
}
