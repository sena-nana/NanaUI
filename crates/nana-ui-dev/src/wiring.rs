//! Wiring the watcher to a running program.
//!
//! Two helpers, one per tier. Both run the same shape: the watcher thread does
//! the slow work (reading a file, running a build) and hands the UI thread a
//! message through `RuntimeProgramContext::dispatch`, which is a channel send
//! plus an event-loop wake and is safe from any thread.

use std::path::{Path, PathBuf};

use nana_ui::{RuntimeProgramContext, RuntimeWindowSettings};

#[cfg(feature = "vue")]
use crate::ReloadRequest;
use crate::restart::{DevHandoff, HANDOFF_ENV, RebuildCommand, RebuildOutcome};
use crate::{DevConfig, DevSignal, DevWatcher};

thread_local! {
    /// Set by the running program when it is ready to hand off, read by
    /// [`run_with_restart`] once the event loop has stopped.
    ///
    /// Thread-local rather than a `static Mutex`: both ends are the UI thread,
    /// and `nana-ui-vue` already crosses `run_runtime` this way for its
    /// bootstrap payload. A mutex here would imply a contention that cannot
    /// happen.
    static PENDING_RELAUNCH: std::cell::RefCell<Option<DevHandoff>> =
        const { std::cell::RefCell::new(None) };

    /// The handoff [`run_with_restart`] already consumed from disk, parked for
    /// the program's own `initialize` to read.
    ///
    /// Without this the two readers race for one file and the program always
    /// loses: `run_with_restart` reads the handoff to restore geometry *before*
    /// `run_runtime` calls `initialize`, and reading deletes the file. The
    /// application's state blob -- the whole reason the blob exists -- would
    /// never survive a single restart.
    static RESTORED_HANDOFF: std::cell::RefCell<Option<DevHandoff>> =
        const { std::cell::RefCell::new(None) };
}

/// Read and delete the handoff file named by [`HANDOFF_ENV`].
fn take_handoff_file() -> Option<DevHandoff> {
    std::env::var_os(HANDOFF_ENV)
        .map(PathBuf::from)
        .and_then(|path| DevHandoff::take(&path))
}

/// Keep a consumed handoff for the program's `initialize`, and hand it back to
/// the caller that consumed it.
fn park_for_initialize(handoff: Option<DevHandoff>) -> Option<DevHandoff> {
    RESTORED_HANDOFF.with_borrow_mut(|slot| slot.clone_from(&handoff));
    handoff
}

/// Ask [`run_with_restart`] to re-exec once the event loop stops.
///
/// Call this from the program's `update` when it handles [`DevSignal::Rebuilt`],
/// then return `RuntimeProgramUpdate { exit: true, .. }`. Capture the geometry
/// from the update's `context` and put the application's own state in
/// [`DevHandoff::state`]; nothing here interprets it.
pub fn request_relaunch(handoff: DevHandoff) {
    PENDING_RELAUNCH.with_borrow_mut(|slot| *slot = Some(handoff));
}

/// What the previous process was showing, if this one was restarted.
///
/// Returns `None` on a normal first launch. Consumes the handoff, so a later
/// manual restart starts fresh rather than restoring a stale frame -- and so
/// two calls in one process do not both claim to be the restore.
///
/// Safe to call from `RuntimeProgram::initialize` under [`run_with_restart`]:
/// that entry point parks what it read rather than leaving the program to
/// re-read a file it has already deleted.
pub fn restored_handoff() -> Option<DevHandoff> {
    if let Some(parked) = RESTORED_HANDOFF.with_borrow_mut(Option::take) {
        return Some(parked);
    }
    take_handoff_file()
}

/// Run an L3 application with restart-on-rebuild.
///
/// Restores the previous process's window geometry into `settings`, runs the
/// event loop, and re-execs afterwards if the program asked for it through
/// [`request_relaunch`]. The application still spawns its own watcher from
/// `initialize` with [`watch_and_rebuild`] -- only it has the context.
///
/// `handoff` is where the geometry file lives; put it under `target/`.
pub fn run_with_restart<Program: nana_ui::RuntimeProgram>(
    mut settings: RuntimeWindowSettings,
    handoff: &Path,
) -> Result<(), nana_ui::HostedRunError> {
    // Geometry is this function's business; the state blob is the program's, and
    // it asks for it from `initialize`, which runs inside `run_runtime` below.
    if let Some(restored) = park_for_initialize(take_handoff_file()) {
        restored.apply(&mut settings);
    }
    let result = nana_ui::run_runtime::<Program>(settings);
    let pending = PENDING_RELAUNCH.with_borrow_mut(Option::take);
    if let Some(pending) = pending
        && pending.write(handoff).is_ok()
    {
        // On Unix this never returns; on Windows the replacement is already
        // spawned and this process is the one that should leave.
        let _ = crate::relaunch(handoff);
    }
    result
}

/// Watch the Rust tier: on a change, rebuild, then tell the program.
///
/// The build runs on the watcher thread. A `cargo build` of a real workspace
/// takes seconds, and running it on the UI thread would freeze the window that
/// the whole restart dance exists to preserve.
pub fn watch_and_rebuild<Message>(
    config: &DevConfig,
    rebuild: RebuildCommand,
    context: &RuntimeProgramContext<Message>,
) -> Result<DevWatcher, notify::Error>
where
    Message: Send + 'static + From<DevSignal>,
{
    let context = context.clone();
    DevWatcher::spawn(config, move |_batch| {
        // Every change is the same question for this tier: does it still build?
        let signal = match rebuild.run() {
            RebuildOutcome::Rebuilt => DevSignal::Rebuilt,
            RebuildOutcome::Failed(diagnostics) => DevSignal::BuildFailed(diagnostics),
        };
        context.dispatch(Message::from(signal));
    })
}

/// Watch the Vue tier: on a change, read the file and tell the program.
///
/// Reading happens here rather than in the program so a large bundle never
/// touches the UI thread, and so a half-written file is rejected before it can
/// reach the engine.
#[cfg(feature = "vue")]
pub fn watch_and_reload<Message>(
    config: &DevConfig,
    context: &RuntimeProgramContext<Message>,
) -> Result<DevWatcher, notify::Error>
where
    Message: Send + 'static + From<nana_ui_vue::dev::DevReload>,
{
    use nana_ui_vue::dev::DevReload;

    let context = context.clone();
    let jail = config.jail_root().to_path_buf();
    let artifact = config.artifact().map(Path::to_path_buf);
    DevWatcher::spawn(config, move |batch| {
        for request in batch {
            // Both arms read through the same jailed, size-capped,
            // empty-file-checked path, so a stylesheet reload names the key the
            // same way the artifact does -- and a save caught between truncate
            // and write never reaches the engine.
            let reload = match request {
                ReloadRequest::Css { path } => crate::read_within_jail(&path, &jail)
                    .ok()
                    .map(|(key, css)| DevReload::Stylesheet { key, css }),
                ReloadRequest::Full => artifact.as_ref().and_then(|path| {
                    crate::read_within_jail(path, &jail)
                        .ok()
                        .map(|(name, source)| DevReload::Artifact { name, source })
                }),
            };
            if let Some(reload) = reload {
                context.dispatch(Message::from(reload));
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape an application would actually carry: which page it was on.
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Session {
        tab: u8,
        scroll: f32,
    }

    #[test]
    fn the_program_still_gets_the_state_blob_after_the_entry_point_read_geometry() {
        // `run_with_restart` reads the handoff to place the window, and reading
        // deletes the file. Before this parked, `initialize` then found nothing
        // and every restart came back on the default page.
        let session = Session {
            tab: 2,
            scroll: 480.0,
        };
        let handoff = DevHandoff {
            position: Some((-40, 120)),
            size: Some((1280, 800)),
            maximized: false,
            state: None,
        }
        .with_state(&session);

        let for_geometry = park_for_initialize(Some(handoff.clone()));
        assert_eq!(
            for_geometry.and_then(|handoff| handoff.size),
            Some((1280, 800)),
            "the entry point still needs the geometry it consumed"
        );

        let in_initialize = restored_handoff().expect("the program's turn");
        assert_eq!(in_initialize.state_as::<Session>(), Some(session));
    }

    #[test]
    fn a_parked_handoff_restores_once() {
        // Twice would mean a later `remove_view` + rebuild restored a frame the
        // application had already moved on from.
        park_for_initialize(Some(DevHandoff {
            size: Some((640, 480)),
            ..DevHandoff::default()
        }));

        assert!(restored_handoff().is_some());
        assert_eq!(restored_handoff(), None);
    }

    #[test]
    fn parking_nothing_is_the_normal_first_launch() {
        park_for_initialize(None);
        assert_eq!(restored_handoff(), None);
    }
}
