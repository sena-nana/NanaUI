#![forbid(unsafe_code)]

use nana_ui::runtime::{
    Button, DocumentId, Entity, FrameworkError, LayoutBox, MutationQueue, RuntimeDocument,
};
use nana_ui::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeWindowSettings, ThemeMode,
    run_runtime,
};
use nana_ui_platform::{WindowEvent, WindowId};

struct Fixture {
    document: RuntimeDocument,
    button: Entity<Button>,
}

impl Fixture {
    fn layout(&mut self, width: f32) -> Result<(), FrameworkError> {
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            self.button.stable_id(),
            LayoutBox {
                x: 24.0,
                y: 24.0,
                width: (width - 48.0).clamp(120.0, 320.0),
                height: 36.0,
            },
        );
        self.document.context_mut().commit_mutations(mutations)?;
        Ok(())
    }
}

impl RuntimeProgram for Fixture {
    type Message = ();
    type Error = FrameworkError;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let document_id = DocumentId::new(1).expect("fixture document id");
        let mut document = RuntimeDocument::new(document_id);
        let button = document
            .context_mut()
            .create_component(document_id, Button::new("Build"))?;
        let mut fixture = Self { document, button };
        fixture.layout(context.geometry().logical_size.0)?;
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

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        if let WindowEvent::Resized { id, geometry } = event
            && self.layout(geometry.logical_size.0).is_ok()
        {
            return RuntimeProgramUpdate::redraw(id);
        }
        RuntimeProgramUpdate::default()
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
