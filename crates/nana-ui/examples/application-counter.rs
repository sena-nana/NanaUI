use std::collections::HashMap;

use nana_ui::runtime::{Activate, Button, Entity, FrameworkError, List, Text};
use nana_ui::{
    ApplicationState, ApplicationWindow, RuntimeApplication, RuntimeProgramContext,
    RuntimeProgramUpdate, RuntimeWindowSettings, run_runtime,
};
use nana_ui_platform::WindowId;

#[derive(Default)]
struct Counter {
    values: HashMap<WindowId, (u64, Entity<Text>)>,
}

impl ApplicationState for Counter {
    type Message = WindowId;
    type Error = FrameworkError;

    fn initialize(_: &RuntimeProgramContext<Self::Message>) -> Result<Self, Self::Error> {
        Ok(Self::default())
    }

    fn window_closed(&mut self, id: WindowId) {
        self.values.remove(&id);
    }

    fn build(
        &mut self,
        window: &mut ApplicationWindow,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), Self::Error> {
        let id = context.window_id();
        let document = window.document.document();
        let label = window.document.context_mut().build(document, |ui| {
            ui.with("counter", List::new(), |ui| {
                let label = ui.child("value", Text::new("0"));
                let increment = ui.child("increment", Button::new("增加"));
                // `dispatch_program_all`, not `dispatch_program`: the message
                // type is `WindowId`, so coalescing by type would drop every
                // click but the last whenever two land in the same frame.
                ui.on(increment, move |_, _: &Activate, cx| {
                    cx.dispatch_program_all(id)
                });
                label
            })
        })?;
        self.values.insert(id, (0, label));
        Ok(())
    }

    fn update(
        &mut self,
        id: WindowId,
        windows: &mut HashMap<WindowId, ApplicationWindow>,
        _: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        if let Some(window) = windows.get_mut(&id)
            && let Some((count, label)) = self.values.get_mut(&id)
        {
            *count += 1;
            window
                .document
                .context_mut()
                .update_component(*label, |text, _| {
                    text.value = count.to_string();
                })
                .expect("counter label belongs to this window");
            return RuntimeProgramUpdate::redraw(id);
        }
        RuntimeProgramUpdate::default()
    }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    run_runtime::<RuntimeApplication<Counter>>(
        RuntimeWindowSettings::new("NanaUI Counter").initial_size(480.0, 320.0),
    )
}
