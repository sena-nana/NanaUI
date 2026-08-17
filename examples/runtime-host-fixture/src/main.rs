#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nana_ui::runtime::{
    Activate, Button, DocumentId, FrameworkError, List, RuntimeDocument, Text, TextInput,
};
use nana_ui::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw,
    RuntimeWindowSettings, ThemeMode, run_runtime,
};
use nana_ui_platform::{WindowCommand, WindowEvent, WindowId, WindowRole, WindowSettings};

const TOOL: WindowId = WindowId(1);

struct Fixture {
    documents: HashMap<WindowId, RuntimeDocument>,
    pending_open: Arc<AtomicBool>,
}

impl Fixture {
    fn open_tool(&mut self) -> Result<RuntimeProgramUpdate, FrameworkError> {
        if !self.documents.contains_key(&TOOL) {
            self.documents.insert(TOOL, tool_document()?);
        }
        Ok(RuntimeProgramUpdate {
            redraw: RuntimeRedraw::All,
            window_commands: vec![WindowCommand::Open {
                id: TOOL,
                settings: WindowSettings {
                    title: "NanaUI fixture tool".into(),
                    initial_size: (360.0, 180.0),
                    minimum_size: (240.0, 120.0),
                    initial_position: Some((80.0, 80.0)),
                    maximized: false,
                    transparent: false,
                    always_on_top: false,
                    resizable: true,
                    role: WindowRole::Tool,
                    modal: false,
                    parent: Some(WindowId::PRIMARY),
                },
            }],
            exit: false,
        })
    }
}

impl RuntimeProgram for Fixture {
    type Message = ();
    type Error = FrameworkError;

    fn initialize(
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let pending_open = Arc::new(AtomicBool::new(false));
        let document_id = DocumentId::new(1).expect("fixture document id");
        let mut document = RuntimeDocument::new(document_id);
        let list = document
            .context_mut()
            .create_component(document_id, List::new().label("Fixture"))?;
        let input = document
            .context_mut()
            .create_component(document_id, TextInput::new("NanaUI").label("Name"))?;
        let button = document
            .context_mut()
            .create_component(document_id, Button::new("Open tool window"))?;
        document.context_mut().append_child(list, input)?;
        document.context_mut().append_child(list, button)?;
        document
            .context_mut()
            .focus_node(document_id, input.stable_id())?;
        let pending = Arc::clone(&pending_open);
        document
            .context_mut()
            .on(button, move |_button, _event: &Activate, _cx| {
                pending.store(true, Ordering::SeqCst);
            })?;
        Ok((
            Self {
                documents: HashMap::from([(WindowId::PRIMARY, document)]),
                pending_open,
            },
            Vec::new(),
        ))
    }

    fn document(&self, id: WindowId) -> Option<&RuntimeDocument> {
        self.documents.get(&id)
    }

    fn document_mut(&mut self, id: WindowId) -> Option<&mut RuntimeDocument> {
        self.documents.get_mut(&id)
    }

    fn update(
        &mut self,
        _message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
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
            WindowEvent::Ready { id, .. } if id == WindowId::PRIMARY => self
                .open_tool()
                .unwrap_or_else(|error| panic!("fixture failed to open tool window: {error}")),
            WindowEvent::Closed { id } => {
                self.documents.remove(&id);
                RuntimeProgramUpdate::default()
            }
            _ => RuntimeProgramUpdate::default(),
        }
    }

    fn window_frame_presented(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        if self.pending_open.swap(false, Ordering::SeqCst) {
            self.open_tool()
                .unwrap_or_else(|error| panic!("fixture failed to open tool window: {error}"))
        } else {
            RuntimeProgramUpdate::default()
        }
    }
}

fn tool_document() -> Result<RuntimeDocument, FrameworkError> {
    let document_id = DocumentId::new(2).expect("tool document id");
    let mut document = RuntimeDocument::new(document_id);
    let list = document
        .context_mut()
        .create_component(document_id, List::new().label("Tool"))?;
    let label = document
        .context_mut()
        .create_component(document_id, Text::new("Auxiliary window"))?;
    let button = document
        .context_mut()
        .create_component(document_id, Button::new("Tool"))?;
    document.context_mut().append_child(list, label)?;
    document.context_mut().append_child(list, button)?;
    Ok(document)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_runtime::<Fixture>(
        RuntimeWindowSettings::new("NanaUI fixture")
            .initial_size(480.0, 220.0)
            .minimum_size(320.0, 160.0),
    )?;
    Ok(())
}
