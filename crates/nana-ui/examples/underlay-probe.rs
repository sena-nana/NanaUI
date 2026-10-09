//! Underlay acceptance probe: a transparent main window with a small control
//! list, and an underlay window glued beneath it that draws the picture.
//!
//! Move, resize, minimize, fullscreen and raise the main window: the picture
//! must stay exactly under it. Screen-capture software lists the underlay as
//! "NanaUI Underlay Probe — picture" and captures the picture without the
//! controls, also while another window covers both. 「切换贴底窗口」 closes and
//! reopens the underlay.
//!
//! With `UNDERLAY_PROBE_STEPS=1` the main window moves, resizes, minimizes
//! and restores itself every three seconds, logging each step, so the
//! underlay's frame can be read back from the window list in between.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Activate, Button, DocumentId, FrameworkError, List, RuntimeDocument, Text};
use nana_ui::{
    RoutedInput, RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw,
    ThemeAppearance, WindowDescriptor, run_runtime,
};
use nana_ui_platform::host::WindowCommand;
use nana_ui_platform::{WindowEvent, WindowId, WindowRole};

const UNDERLAY: WindowId = WindowId(2);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Message {
    OpenUnderlay,
}

struct Probe {
    documents: BTreeMap<WindowId, RuntimeDocument>,
    toggle: Arc<AtomicBool>,
}

fn controls(toggle: &Arc<AtomicBool>) -> Result<RuntimeDocument, FrameworkError> {
    let id = DocumentId::new(1).expect("controls document");
    let mut document = RuntimeDocument::new(id);
    let pending = Arc::clone(toggle);
    document.context_mut().mount_view_root(id, || {
        widget(List::new().label("Controls")).children((
            widget(Text::new("UI 层")),
            widget(Button::new("切换贴底窗口")).on(move |_: &Activate| {
                pending.store(true, Ordering::SeqCst);
            }),
        ))
    })?;
    Ok(document)
}

fn picture() -> Result<RuntimeDocument, FrameworkError> {
    let id = DocumentId::new(2).expect("picture document");
    let mut document = RuntimeDocument::new(id);
    document.context_mut().mount_view_root(id, || {
        widget(List::new().label("Picture")).children((
            widget(Text::new("")),
            widget(Text::new("")),
            widget(Text::new("")),
            widget(Text::new("渲染层：只有画面，没有界面")),
        ))
    })?;
    Ok(document)
}

fn underlay_settings() -> WindowDescriptor {
    WindowDescriptor {
        title: "NanaUI Underlay Probe — picture".into(),
        role: WindowRole::Underlay,
        parent: Some(WindowId::PRIMARY),
        ..WindowDescriptor::default()
    }
}

impl Probe {
    fn open_underlay(&mut self) -> RuntimeProgramUpdate {
        self.documents
            .insert(UNDERLAY, picture().expect("picture document"));
        RuntimeProgramUpdate {
            redraw: RuntimeRedraw::All,
            window_commands: vec![WindowCommand::Open {
                id: UNDERLAY,
                settings: underlay_settings(),
            }],
            exit: false,
        }
    }
}

impl RuntimeProgram for Probe {
    type Message = Message;
    type Error = Infallible;

    fn initialize(
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let toggle = Arc::new(AtomicBool::new(false));
        let mut documents = BTreeMap::new();
        documents.insert(
            WindowId::PRIMARY,
            controls(&toggle).expect("controls document"),
        );
        Ok((Self { documents, toggle }, vec![Message::OpenUnderlay]))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok(self.documents.get(&id).map(f))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok(self.documents.get_mut(&id).map(f))
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match message {
            Message::OpenUnderlay => self.open_underlay(),
        }
    }

    fn theme(&self) -> Arc<nana_ui::CompiledTheme> {
        nana_ui::builtin_theme_arc(ThemeAppearance::Dark)
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::Ready { id, geometry } => {
                eprintln!(
                    "underlay-probe ready id={} size={:?} alpha={:?}",
                    id.0,
                    geometry.logical_size,
                    context.surface_alpha_mode()
                );
                if id == WindowId::PRIMARY && std::env::var_os("UNDERLAY_PROBE_STEPS").is_some() {
                    let window = context.window();
                    std::thread::spawn(move || {
                        let step = |name: &str| {
                            std::thread::sleep(std::time::Duration::from_secs(3));
                            eprintln!("underlay-probe step {name}");
                        };
                        step("move");
                        let _ = window.set_position((200.0, 200.0)).wait();
                        step("resize");
                        let _ = window.set_size((900.0, 600.0)).wait();
                        step("minimize");
                        let _ = window.set_minimized(true).wait();
                        step("restore");
                        let _ = window.set_minimized(false).wait();
                        step("done");
                    });
                }
                RuntimeProgramUpdate::default()
            }
            WindowEvent::CloseRequested { .. } => RuntimeProgramUpdate::exit(),
            WindowEvent::Closed { id } => {
                self.documents.remove(&id);
                RuntimeProgramUpdate::default()
            }
            _ => RuntimeProgramUpdate::default(),
        }
    }

    fn input_event(
        &mut self,
        _id: WindowId,
        input: RoutedInput<'_>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let _event = input.event;
        if !self.toggle.swap(false, Ordering::SeqCst) {
            return Ok(RuntimeProgramUpdate::default());
        }
        Ok(if self.documents.contains_key(&UNDERLAY) {
            RuntimeProgramUpdate {
                redraw: RuntimeRedraw::All,
                window_commands: vec![WindowCommand::Close(UNDERLAY)],
                exit: false,
            }
        } else {
            self.open_underlay()
        })
    }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    let mut settings = WindowDescriptor::new("NanaUI Underlay Probe")
        .initial_size(720.0, 480.0)
        .minimum_size(480.0, 320.0);
    settings.transparent = true;
    run_runtime::<Probe>(settings)
}
