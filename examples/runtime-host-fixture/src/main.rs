#![forbid(unsafe_code)]

use nana_ui::runtime::{Button, DocumentId, FrameworkError, RuntimeDocument};
use nana_ui::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeWindowSettings, ThemeMode,
    run_runtime,
};
use nana_ui_platform::WindowId;

struct Fixture {
    document: RuntimeDocument,
}

impl RuntimeProgram for Fixture {
    type Message = ();
    type Error = FrameworkError;

    fn initialize(
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let document_id = DocumentId::new(1).expect("fixture document id");
        let mut document = RuntimeDocument::new(document_id);
        document
            .context_mut()
            .create_component(document_id, Button::new("Build"))?;
        let fixture = Self { document };
        Ok((fixture, Vec::new()))
    }

    fn document(&self, id: WindowId) -> Option<&RuntimeDocument> {
        (id == WindowId::PRIMARY).then_some(&self.document)
    }

    fn document_mut(&mut self, id: WindowId) -> Option<&mut RuntimeDocument> {
        (id == WindowId::PRIMARY).then_some(&mut self.document)
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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_runtime::<Fixture>(
        RuntimeWindowSettings::new("NanaUI fixture")
            .initial_size(480.0, 180.0)
            .minimum_size(320.0, 140.0),
    )?;
    Ok(())
}
