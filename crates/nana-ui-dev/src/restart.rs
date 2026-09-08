//! Restart-with-restore for the Rust L3 tier.
//!
//! # Why a restart and not a hot swap
//!
//! Rust code cannot be swapped into a live process here, and the reason is not
//! effort. `TypeId` is the tree reconciliation key -- `UiBuilder::child`
//! compares `TypeId::of::<C>()` and `AppContext::remove_view` downcasts on it --
//! and `TypeId` is not stable across compilations. A freshly compiled dynamic
//! library produces different ids for the same types, so every keyed child would
//! mismatch and every `remove_view` would fail its downcast. On top of that
//! `RuntimeProgram` is not object-safe (`with_document` is generic, so it cannot
//! sit in a vtable), event handlers are code pointers the host outlives, and
//! macOS will not unload an image containing Objective-C metadata or TLS -- both
//! of which this process has.
//!
//! So the honest L3 answer is: rebuild, hand the window's geometry and an opaque
//! application blob to the next process, and re-exec. The window blinks once.
//! Everything else in the 4-10 second cycle is compilation, which a dynamic
//! library would have paid too.
//!
//! Link time dominates that cycle, so configuring a faster linker for the dev
//! profile buys more than anything in this module.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::DevError;

/// Environment variable naming the handoff file, so the restarted process finds
/// it without the application having to thread a CLI flag through its own
/// argument parsing.
pub const HANDOFF_ENV: &str = "NANA_DEV_HANDOFF";

/// Window geometry and opaque application state carried across a restart.
///
/// The application blob is a string this crate never interprets: business state
/// belongs to the consuming application, and the framework's job is only to make
/// sure it arrives. JSON, matching how the workspace already persists layout
/// (`DockWorkspace::layout_json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevHandoff {
    pub position: Option<(i64, i64)>,
    pub size: Option<(u64, u64)>,
    pub maximized: bool,
    pub state: Option<String>,
}

impl DevHandoff {
    /// Write the handoff for the next process to pick up.
    pub fn write(&self, path: &Path) -> Result<(), DevError> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json =
            serde_json::to_string(self).map_err(|_| DevError::Unreadable(path.to_path_buf()))?;
        std::fs::write(path, json).map_err(|_| DevError::Unreadable(path.to_path_buf()))
    }

    /// Read a handoff and delete it, so a crash-and-manual-restart does not
    /// silently restore geometry from an unrelated session weeks later.
    ///
    /// Returns `None` when no handoff was left, which is the normal first run.
    pub fn take(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let _ = std::fs::remove_file(path);
        serde_json::from_str(&text).ok()
    }

    /// Carry the application's own state across the restart, serialized.
    ///
    /// The blob stays a string this crate never interprets; this is only the
    /// JSON round trip every application would otherwise hand-write. State that
    /// fails to serialize is dropped rather than failing the restart: coming
    /// back on the wrong page is a survivable dev cycle, losing the rebuild is
    /// not.
    pub fn with_state<T: Serialize>(mut self, state: &T) -> Self {
        self.state = serde_json::to_string(state).ok();
        self
    }

    /// The application state carried by [`Self::with_state`].
    ///
    /// `None` covers all of: nothing was carried, the type changed since the
    /// build that wrote it, and the blob came from somewhere else. A dev loop
    /// has to survive edits to the very type it is restoring -- that is the
    /// case this exists for, so a mismatch restores fresh instead of failing.
    pub fn state_as<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_str(self.state.as_deref()?).ok()
    }

    /// Apply the carried geometry to window settings.
    ///
    /// Only geometry: the state blob is the application's to interpret.
    pub fn apply(&self, settings: &mut nana_ui::RuntimeWindowSettings) {
        if let Some((x, y)) = self.position {
            settings.initial_position = Some((x as f64, y as f64));
        }
        if let Some((width, height)) = self.size {
            settings.initial_size = (width as f64, height as f64);
        }
        settings.maximized = self.maximized;
        // A restored frame may name a display that is no longer attached.
        settings.constrain_to_work_area = true;
    }
}

/// Outcome of one rebuild attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildOutcome {
    /// The binary was rebuilt; the process should hand off and re-exec.
    Rebuilt,
    /// Compilation failed. Carries the rendered diagnostics, which is the only
    /// thing that makes a failed save actionable.
    Failed(String),
}

/// Runs the project's build command when a watched source changes.
///
/// Lives on the watcher thread, never the UI thread: a `cargo build` of this
/// workspace takes seconds, and blocking the event loop for it would freeze the
/// window the restart is trying to preserve.
#[derive(Debug, Clone)]
pub struct RebuildCommand {
    program: String,
    args: Vec<String>,
}

impl RebuildCommand {
    /// `cargo build -p <package>`, rendered so diagnostics arrive as the text a
    /// developer would have seen in a terminal.
    pub fn cargo_package(package: &str) -> Self {
        Self {
            program: "cargo".into(),
            args: vec![
                "build".into(),
                "-p".into(),
                package.into(),
                "--message-format=short".into(),
            ],
        }
    }

    pub fn run(&self) -> RebuildOutcome {
        match Command::new(&self.program).args(&self.args).output() {
            Ok(output) if output.status.success() => RebuildOutcome::Rebuilt,
            Ok(output) => {
                let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
                if text.trim().is_empty() {
                    text = String::from_utf8_lossy(&output.stdout).into_owned();
                }
                RebuildOutcome::Failed(text)
            }
            Err(error) => {
                RebuildOutcome::Failed(format!("could not run `{}`: {error}", self.program))
            }
        }
    }
}

/// Replace this process with a fresh copy of the (just rebuilt) binary.
///
/// Argument list, working directory and environment are inherited, plus
/// [`HANDOFF_ENV`] pointing at `handoff`.
///
/// On Unix this `exec`s, so it never returns on success -- the window server
/// sees one process the whole time. On Windows there is no `exec`, so the
/// replacement is spawned and the caller is told to exit; returning `Ok(())`
/// there means "you are the old process, leave now".
pub fn relaunch(handoff: &Path) -> Result<(), DevError> {
    let exe = std::env::current_exe().map_err(|_| DevError::Unreadable(PathBuf::from("<exe>")))?;
    let mut command = Command::new(&exe);
    command.args(std::env::args_os().skip(1));
    command.env(HANDOFF_ENV, handoff);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Only returns on failure.
        let error = command.exec();
        Err(DevError::Unreadable(PathBuf::from(format!(
            "{}: {error}",
            exe.display()
        ))))
    }
    #[cfg(not(unix))]
    {
        command
            .spawn()
            .map(|_| ())
            .map_err(|_| DevError::Unreadable(exe))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch;

    #[test]
    fn geometry_and_state_survive_a_round_trip() {
        // The blob is the application's, so it can contain anything at all --
        // quotes, newlines, and text that looks like this format's own fields.
        let dir = scratch("handoff-round-trip");
        let path = dir.join("handoff");
        let handoff = DevHandoff {
            position: Some((-40, 120)),
            size: Some((1280, 800)),
            maximized: true,
            state: Some("{\"tab\":2}\nsize 1 2\n".into()),
        };
        handoff.write(&path).expect("write");
        assert_eq!(DevHandoff::take(&path), Some(handoff));
    }

    /// What an application actually wants back: where it was, not a string.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Session {
        page: String,
        selected: Option<u32>,
    }

    #[test]
    fn typed_state_survives_the_round_trip() {
        let session = Session {
            page: "settings/appearance".into(),
            selected: Some(7),
        };
        let handoff = DevHandoff::default().with_state(&session);
        assert_eq!(handoff.state_as::<Session>(), Some(session));
    }

    #[test]
    fn state_written_by_an_older_build_restores_fresh_rather_than_failing() {
        // Editing the state type is the single most likely thing to happen
        // between two runs of a dev loop, so a mismatch must not be an error.
        let handoff = DevHandoff::default().with_state(&"a string, not a Session");
        assert_eq!(handoff.state_as::<Session>(), None);
        assert_eq!(DevHandoff::default().state_as::<Session>(), None);
    }

    #[test]
    fn taking_a_handoff_consumes_it() {
        let dir = scratch("handoff-take");
        let path = dir.join("handoff");
        let handoff = DevHandoff {
            size: Some((640, 480)),
            ..DevHandoff::default()
        };
        handoff.write(&path).expect("write");

        assert_eq!(DevHandoff::take(&path), Some(handoff));
        assert_eq!(
            DevHandoff::take(&path),
            None,
            "a consumed handoff must not restore a second time"
        );
    }

    #[test]
    fn a_missing_handoff_is_the_normal_first_run() {
        let dir = scratch("handoff-missing");
        assert_eq!(DevHandoff::take(&dir.join("absent")), None);
    }

    #[test]
    fn applying_a_handoff_sets_geometry_and_constrains_to_the_work_area() {
        let mut settings = nana_ui::RuntimeWindowSettings::new("App");
        DevHandoff {
            position: Some((100, 50)),
            size: Some((1024, 768)),
            maximized: true,
            state: None,
        }
        .apply(&mut settings);

        assert_eq!(settings.initial_position, Some((100.0, 50.0)));
        assert_eq!(settings.initial_size, (1024.0, 768.0));
        assert!(settings.maximized);
        // The display that frame lived on may be gone by the time it restores.
        assert!(settings.constrain_to_work_area);
    }

    #[test]
    fn a_failing_build_reports_its_diagnostics() {
        // A build that fails silently is the worst outcome a dev loop can have:
        // the developer saves, nothing happens, and nothing says why.
        match RebuildCommand::cargo_package("nana-does-not-exist").run() {
            RebuildOutcome::Failed(text) => assert!(
                !text.trim().is_empty(),
                "a failed build must carry something the developer can act on"
            ),
            RebuildOutcome::Rebuilt => panic!("that package does not exist"),
        }
    }
}
