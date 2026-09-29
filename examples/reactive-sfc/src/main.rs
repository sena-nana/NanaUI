//! `cargo run -p reactive-sfc`

use std::collections::HashMap;

use nana_ui::runtime::{BuiltinComponents, FrameworkError, MountedView};
use nana_ui::{
    ApplicationIdentity, ApplicationState, ApplicationWindow, DiagnosticsConfig, NanaApplication,
    RuntimeApplication, RuntimeProgramContext, RuntimeProgramUpdate, WindowDescriptor,
};
use nana_ui_platform::WindowId;
use reactive_sfc::views;

#[derive(Default)]
struct Sfc {
    views: HashMap<WindowId, MountedView>,
}

impl ApplicationState for Sfc {
    type Message = ();
    type Error = FrameworkError;
    // Views are compiled Rust: nothing builds a component from a tag, so the
    // built-in components this app never creates are not linked.
    const BUILTINS: BuiltinComponents = BuiltinComponents::Typed;

    fn initialize(_: &RuntimeProgramContext<Self::Message>) -> Result<Self, Self::Error> {
        Ok(Self::default())
    }

    fn window_closed(&mut self, id: WindowId) {
        self.views.remove(&id);
    }

    fn build(
        &mut self,
        window: &mut ApplicationWindow,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), Self::Error> {
        let document = window.document.document();
        let view = window
            .document
            .context_mut()
            .mount_view_root(document, views::app)?;
        self.views.insert(context.window_id(), view);
        Ok(())
    }

    fn update(
        &mut self,
        (): (),
        _: &mut HashMap<WindowId, ApplicationWindow>,
        _: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    let identity = ApplicationIdentity::new(
        "dev.nanaui.reactive-sfc",
        "NanaUI Reactive SFC",
        env!("CARGO_PKG_VERSION"),
    );
    NanaApplication::builder(identity)
        .diagnostics(DiagnosticsConfig::default())
        .run::<RuntimeApplication<Sfc>>(
            WindowDescriptor::new("NanaUI .vue views").initial_size(480.0, 420.0),
        )
}
