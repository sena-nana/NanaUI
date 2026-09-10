//! A Rust L3 development binary, end to end.
//!
//! Compiled but not run here: the windowed path needs a display. Its job is to
//! keep the public dev surface honest — if the API drifts, this stops building.
//!
//! Rust code cannot be swapped into a live process (see the crate docs for the
//! `TypeId` reason), so this rebuilds, hands the window's geometry and an opaque
//! state blob to the next process, and re-execs. The window blinks once;
//! everything else in the cycle is compilation.
//!
//! Two entry points, one document builder:
//!
//! - default: watch, rebuild, restart, restore.
//! - `--features headless` plus `--stdio` / `--screenshot` / `--a11y`: the same
//!   tree in a headless Agent session, so "did that change land?" is answered by
//!   a PNG and an a11y dump instead of by opening a window and looking. See
//!   `docs/hot-reload.md`.
//!
//! The release binary is a separate `main.rs` that calls `run_runtime` directly
//! and never mentions this crate — which is what keeps `nana-ui-dev`'s
//! `debug_assertions` guard from ever firing on a packaged build.

use std::path::Path;
use std::process::ExitCode;

use nana_ui::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeWindowSettings, ThemeMode,
};
use nana_ui_dev::{DevConfig, DevHandoff, DevSignal, DevWatcher, RebuildCommand};
use nana_ui_platform::WindowId;
use nana_ui_runtime::{Button, DocumentId, Stack, Text};
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

/// Whatever this application wants to survive a restart. Opaque to the
/// framework, which only guarantees it arrives — so it can be a real type
/// rather than a hand-rolled string, and it is worth making it the *view*
/// state. Coming back on the page you were editing is most of what separates a
/// rebuild that feels like a reload from one that feels like a relaunch.
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct Session {
    page: String,
    scroll: f32,
    selected: Option<u32>,
}

/// One builder, both entry points.
///
/// Keeping this a free function over `&Session` is the whole trick behind
/// headless verification: the same tree the window shows is the tree the Agent
/// screenshots, so a PNG is evidence about the real application rather than
/// about a fixture that resembles it. The `child` keys double as the Agent's
/// `agent_path` selectors (`page`, `open`).
fn build_document(session: &Session) -> RuntimeDocument {
    let id = DocumentId::new(1).expect("document id");
    let mut document = RuntimeDocument::new(id);
    let page = if session.page.is_empty() {
        "home"
    } else {
        session.page.as_str()
    };
    document
        .context_mut()
        .build(id, |ui| {
            ui.with("root", Stack::column(8.0), |ui| {
                ui.child("page", Text::new(page));
                ui.child("open", Button::new("Open"));
            })
        })
        .expect("root");
    document
}

struct DemoProgram {
    document: RuntimeDocument,
    /// Dropping the watcher stops the watch, so it lives as long as the program.
    _watcher: Option<DevWatcher>,
    session: Session,
}

impl RuntimeProgram for DemoProgram {
    type Message = Message;
    type Error = String;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        // Whatever the previous process was showing, if this is a restart.
        // `run_with_restart` already consumed the handoff to place the window
        // and parked it for exactly this call, so the blob is still here.
        // A `None` is either a first launch or a state type that changed since
        // the last build; both restore fresh rather than failing.
        let session = nana_ui_dev::restored_handoff()
            .and_then(|handoff| handoff.state_as::<Session>())
            .unwrap_or_default();

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
                document: build_document(&session),
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
                nana_ui_dev::request_relaunch(
                    DevHandoff {
                        position: geometry.logical_position.map(|(x, y)| (x as i64, y as i64)),
                        size: Some((
                            geometry.logical_size.0 as u64,
                            geometry.logical_size.1 as u64,
                        )),
                        maximized: geometry.maximized,
                        state: None,
                    }
                    .with_state(&self.session),
                );
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

/// Headless verification: same document, no window, no display.
///
/// Returns `None` when the caller asked for nothing headless, so the windowed
/// loop still runs on a bare invocation.
#[cfg(feature = "headless")]
fn run_headless() -> Option<ExitCode> {
    use nana_ui_devtools::agent::cli;

    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("l3-dev-entry: {error}");
            return Some(ExitCode::from(2));
        }
    };
    if args.help {
        println!(
            "l3-dev-entry — windowed dev loop, or a headless Agent session\n\n\
             Without a headless flag this watches, rebuilds and restarts.\n\n\
             Headless options:\n{}",
            cli::COMMON_USAGE
        );
        return Some(ExitCode::SUCCESS);
    }
    if args.gpu_probe {
        cli::print_gpu_probe();
        return Some(ExitCode::SUCCESS);
    }
    if !(args.stdio || args.a11y || args.screenshot.is_some()) {
        return None;
    }
    // A headless run has no previous process to restore from, so it always
    // renders the first-launch tree. That is the point: reproducible evidence.
    Some(cli::runtime_main(
        build_document(&Session::default()),
        &args,
    ))
}

#[cfg(not(feature = "headless"))]
fn run_headless() -> Option<ExitCode> {
    None
}

fn main() -> ExitCode {
    if let Some(code) = run_headless() {
        return code;
    }
    match nana_ui_dev::run_with_restart::<DemoProgram>(
        RuntimeWindowSettings::new("L3 dev entry"),
        Path::new(HANDOFF),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("l3-dev-entry: {error}");
            ExitCode::from(1)
        }
    }
}
