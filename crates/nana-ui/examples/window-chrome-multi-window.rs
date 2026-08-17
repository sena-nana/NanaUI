use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nana_ui::runtime::{Activate, Button, DocumentId, FrameworkError, List, RuntimeDocument, Text};
use nana_ui::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw,
    RuntimeWindowSettings, ThemeMode, run_runtime,
};
use nana_ui_platform::{WindowCommand, WindowEvent, WindowId, WindowRole, WindowSettings};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Message {
    OpenWindow,
}

struct Smoke {
    windows: BTreeMap<WindowId, SmokeWindow>,
    next_window: u64,
    open: Arc<AtomicBool>,
}

struct SmokeWindow {
    document: RuntimeDocument,
}

impl Smoke {
    fn open_settings(number: usize) -> WindowSettings {
        let offset = 64.0 * number.saturating_sub(1) as f64;
        WindowSettings {
            title: format!("NanaUI Window {number}"),
            initial_size: (640.0, 420.0),
            minimum_size: (480.0, 320.0),
            initial_position: Some((120.0 + offset, 120.0 + offset)),
            maximized: false,
            transparent: false,
            always_on_top: false,
            resizable: true,
            role: if number == 1 {
                WindowRole::Main
            } else {
                WindowRole::Tool
            },
            modal: false,
            parent: (number > 1).then_some(WindowId::PRIMARY),
        }
    }

    fn mount_window(
        id: WindowId,
        number: usize,
        open: &Arc<AtomicBool>,
    ) -> Result<SmokeWindow, FrameworkError> {
        let document_id = DocumentId::new(id.0 + 1).expect("window document");
        let mut document = RuntimeDocument::new(document_id);
        let root = document
            .context_mut()
            .create_component(document_id, List::new().label("Window chrome"))?;
        let title = document
            .context_mut()
            .create_component(document_id, Text::new("NANA NanaUI Window"))?;
        let body = document
            .context_mut()
            .create_component(document_id, Text::new(format!("窗口 {number}")))?;
        let button = document
            .context_mut()
            .create_component(document_id, Button::new("新建窗口"))?;
        document.context_mut().append_child(root, title)?;
        document.context_mut().append_child(root, body)?;
        document.context_mut().append_child(root, button)?;
        let pending = Arc::clone(open);
        document
            .context_mut()
            .on(button, move |_button, _event: &Activate, _cx| {
                pending.store(true, Ordering::SeqCst);
            })?;
        Ok(SmokeWindow { document })
    }

    fn open_next(&mut self) -> RuntimeProgramUpdate {
        let number = self.next_window as usize;
        self.next_window = self.next_window.saturating_add(1);
        let id = if number == 1 {
            WindowId::PRIMARY
        } else {
            WindowId(number as u64)
        };
        let window = Self::mount_window(id, number, &self.open).expect("window document");
        self.windows.insert(id, window);
        if id == WindowId::PRIMARY {
            RuntimeProgramUpdate::redraw(id)
        } else {
            RuntimeProgramUpdate {
                redraw: RuntimeRedraw::All,
                window_commands: vec![WindowCommand::Open {
                    id,
                    settings: Self::open_settings(number),
                }],
                exit: false,
            }
        }
    }
}

impl RuntimeProgram for Smoke {
    type Message = Message;
    type Error = Infallible;

    fn initialize(
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let mut smoke = Self {
            windows: BTreeMap::new(),
            next_window: 1,
            open: Arc::new(AtomicBool::new(false)),
        };
        let _ = smoke.open_next();
        Ok((smoke, vec![Message::OpenWindow]))
    }

    fn document(&self, id: WindowId) -> Option<&RuntimeDocument> {
        self.windows.get(&id).map(|window| &window.document)
    }

    fn document_mut(&mut self, id: WindowId) -> Option<&mut RuntimeDocument> {
        self.windows.get_mut(&id).map(|window| &mut window.document)
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match message {
            Message::OpenWindow => self.open_next(),
        }
    }

    fn theme_mode(&self) -> ThemeMode {
        ThemeMode::Dark
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::CloseRequested { id } if id == WindowId::PRIMARY => {
                RuntimeProgramUpdate::exit()
            }
            WindowEvent::CloseRequested { id } => RuntimeProgramUpdate {
                redraw: RuntimeRedraw::All,
                window_commands: vec![WindowCommand::Close(id)],
                exit: false,
            },
            WindowEvent::Closed { id } => {
                self.windows.remove(&id);
                if self.windows.is_empty() {
                    RuntimeProgramUpdate::exit()
                } else {
                    RuntimeProgramUpdate::default()
                }
            }
            _ => RuntimeProgramUpdate::default(),
        }
    }

    fn input_event(
        &mut self,
        _id: WindowId,
        _event: &nana_ui_platform::InputEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        if self.open.swap(false, Ordering::SeqCst) {
            Ok(self.open_next())
        } else {
            Ok(RuntimeProgramUpdate::default())
        }
    }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    run_runtime::<Smoke>(
        RuntimeWindowSettings::new("NanaUI Window 1")
            .initial_size(640.0, 420.0)
            .minimum_size(480.0, 320.0),
    )
}
