//! A Rust L3 development binary, end to end.
//!
//! Compiled but not run here: it needs a display. Its job is to keep the public
//! dev surface honest — if the API drifts, this stops building.
//!
//! Rust code cannot be swapped into a live process (see the crate docs for the
//! `TypeId` reason), so this rebuilds, hands the window's geometry and an opaque
//! state blob to the next process, and re-execs. The window blinks once;
//! everything else in the cycle is compilation.
//!
//! The release binary is a separate `main.rs` that calls `run_runtime` directly
//! and never mentions this crate — which is what keeps `nana-ui-dev`'s
//! `debug_assertions` guard from ever firing on a packaged build.

use std::path::Path;

use nana_ui::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeWindowSettings, ThemeMode,
};
use nana_ui_dev::{DevConfig, DevHandoff, DevSignal, DevWatcher, RebuildCommand};
use nana_ui_platform::WindowId;
use nana_ui_runtime::DocumentId;
use nana_ui_scene::{DocumentAccessError, RuntimeDocument};

/// Where the geometry handoff lives between the two processes.
const HANDOFF: &str = "target/nana-dev-handoff";

/// The application's message type gains one variant for the dev loop. The
/// `From` impl is what `watch_and_rebuild` needs to dispatch into it.
#[derive(Debug)]
enum Message {
    Dev(DevSignal),
}

impl From<DevSignal> for Message {
    fn from(signal: DevSignal) -> Self {
        Self::Dev(signal)
    }
}

struct DemoProgram {
    document: RuntimeDocument,
    /// Dropping the watcher stops the watch, so it lives as long as the program.
    _watcher: Option<DevWatcher>,
    /// Whatever this application wants to survive a restart. Opaque to the
    /// framework, which only guarantees it arrives.
    session: String,
}

impl RuntimeProgram for DemoProgram {
    type Message = Message;
    type Error = String;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        // Whatever the previous process was showing, if this is a restart.
        let session = nana_ui_dev::restored_handoff()
            .and_then(|handoff| handoff.state)
            .unwrap_or_else(|| "fresh".to_owned());

        // One call starts the watch. The build runs on the watcher thread; the
        // UI thread only ever sees the resulting message.
        let watcher = nana_ui_dev::watch_and_rebuild(
            &DevConfig::new_rust("src"),
            RebuildCommand::cargo_package("my-app"),
            context,
        )
        .ok();

        Ok((
            Self {
                document: RuntimeDocument::new(DocumentId::new(1).expect("document id")),
                _watcher: watcher,
                session,
            },
            Vec::new(),
        ))
    }

    fn with_document<R>(
        &self,
        _id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok(Some(f(&self.document)))
    }

    fn with_document_mut<R>(
        &mut self,
        _id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok(Some(f(&mut self.document)))
    }

    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match message {
            Message::Dev(DevSignal::Rebuilt) => {
                let geometry = context.geometry();
                nana_ui_dev::request_relaunch(DevHandoff {
                    position: geometry.logical_position.map(|(x, y)| (x as i64, y as i64)),
                    size: Some((
                        geometry.logical_size.0 as u64,
                        geometry.logical_size.1 as u64,
                    )),
                    maximized: geometry.maximized,
                    state: Some(self.session.clone()),
                });
                // `run_with_restart` re-execs once the loop has stopped.
                RuntimeProgramUpdate {
                    exit: true,
                    ..RuntimeProgramUpdate::default()
                }
            }
            Message::Dev(DevSignal::BuildFailed(diagnostics)) => {
                // A save that silently did nothing is the worst outcome a dev
                // loop can produce, so this is never swallowed.
                eprintln!("{diagnostics}");
                RuntimeProgramUpdate::default()
            }
        }
    }

    fn theme_mode(&self) -> ThemeMode {
        ThemeMode::Dark
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    nana_ui_dev::run_with_restart::<DemoProgram>(
        RuntimeWindowSettings::new("L3 dev entry"),
        Path::new(HANDOFF),
    )?;
    Ok(())
}
