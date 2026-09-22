//! Process-level application setup: identity, [`ApplicationPaths`], and
//! diagnostics (Issues #226 / #227).
//!
//! ```ignore
//! NanaApplication::builder(ApplicationIdentity::new("dev.nana.live", "NanaLive", "1.0"))
//!     .diagnostics(DiagnosticsConfig::default())
//!     .run::<RuntimeApplication<App>>(WindowDescriptor::new("NanaLive"))
//! ```
//!
//! Diagnostics are opt-in: without `.diagnostics(..)` nothing is recorded and
//! every framework call site costs one relaxed atomic load.

use std::sync::OnceLock;

use nana_diagnostics::{
    Diagnostics, DiagnosticsConfig, DiagnosticsGuard, DiagnosticsPaths, SessionMetadata,
};
use nana_ui_platform::{ApplicationIdentity, ApplicationPaths};

/// The paths the first started builder resolved; first one wins.
static PATHS: OnceLock<ApplicationPaths> = OnceLock::new();

/// Entry point for process-level setup.
pub struct NanaApplication;

impl NanaApplication {
    pub fn builder(identity: ApplicationIdentity) -> NanaApplicationBuilder {
        NanaApplicationBuilder {
            identity,
            paths: None,
            diagnostics: DiagnosticsConfig::disabled(),
        }
    }

    /// The paths published by the first builder started in this process.
    pub fn paths() -> Option<&'static ApplicationPaths> {
        PATHS.get()
    }

    /// The process's diagnostics runtime, if one is installed.
    pub fn diagnostics() -> Option<Diagnostics> {
        nana_diagnostics::global()
    }
}

#[must_use]
pub struct NanaApplicationBuilder {
    identity: ApplicationIdentity,
    paths: Option<ApplicationPaths>,
    diagnostics: DiagnosticsConfig,
}

/// Keeps process-level services alive. Dropping it shuts diagnostics down
/// with a final durable flush, so bind it for the life of the app.
#[must_use = "dropping the session shuts diagnostics down immediately"]
pub struct ApplicationSession {
    paths: Option<&'static ApplicationPaths>,
    diagnostics: Option<DiagnosticsGuard>,
}

impl ApplicationSession {
    /// `None` when the platform directories could not be resolved (e.g. no
    /// `HOME` in a container); the application still runs.
    pub fn paths(&self) -> Option<&'static ApplicationPaths> {
        self.paths
    }

    /// `None` when diagnostics are disabled or another instance was already
    /// installed in this process.
    pub fn diagnostics(&self) -> Option<&Diagnostics> {
        self.diagnostics.as_ref().map(DiagnosticsGuard::diagnostics)
    }
}

impl NanaApplicationBuilder {
    /// Use these paths instead of resolving them for the running process.
    /// Paths are published once per process: if an earlier builder already
    /// published some, those stay in effect.
    pub fn paths(mut self, paths: ApplicationPaths) -> Self {
        self.paths = Some(paths);
        self
    }

    pub fn diagnostics(mut self, config: DiagnosticsConfig) -> Self {
        self.diagnostics = config;
        self
    }

    /// Resolve and publish the paths and start diagnostics. For hosts that
    /// run their own loop (Vue, embedded); [`Self::run`] calls this.
    ///
    /// Never fails the application: when the platform directories cannot be
    /// resolved it logs to stderr, the session has no paths, and diagnostics
    /// stay in memory.
    pub fn start(self) -> ApplicationSession {
        let resolved = match self.paths {
            Some(paths) => Ok(paths),
            None => ApplicationPaths::resolve(&self.identity),
        };
        let paths = match resolved {
            Ok(paths) => Some(PATHS.get_or_init(|| paths)),
            Err(error) => {
                eprintln!(
                    "NanaUI: cannot resolve application paths: {error}; running without them"
                );
                None
            }
        };
        let diagnostics = if self.diagnostics.enabled {
            install_diagnostics(&self.identity, self.diagnostics, paths)
        } else {
            None
        };
        ApplicationSession { paths, diagnostics }
    }

    /// Start process services, run the Scene host, then shut down cleanly.
    #[cfg(feature = "hosted")]
    pub fn run<Program: crate::RuntimeProgram>(
        self,
        settings: crate::WindowDescriptor,
    ) -> Result<(), crate::HostedRunError> {
        let session = self.start();
        let result = crate::run_runtime::<Program>(settings);
        record_run_result(&result);
        drop(session);
        result
    }

    /// [`Self::run`] with a host-injected persistent store.
    #[cfg(feature = "hosted")]
    pub fn run_with_store<Program: crate::RuntimeProgram>(
        self,
        settings: crate::WindowDescriptor,
        store: crate::SharedStore,
    ) -> Result<(), crate::HostedRunError> {
        let session = self.start();
        let result = crate::run_runtime_with_store::<Program>(settings, store);
        record_run_result(&result);
        drop(session);
        result
    }
}

fn install_diagnostics(
    identity: &ApplicationIdentity,
    config: DiagnosticsConfig,
    paths: Option<&ApplicationPaths>,
) -> Option<DiagnosticsGuard> {
    let layout = paths.map_or("unresolved", |p| p.layout().as_str());
    let mut meta = SessionMetadata::new(&identity.id, &identity.name, &identity.version)
        .framework_version(env!("CARGO_PKG_VERSION"))
        .extra("layout", layout);
    if let Some(build_id) = &identity.build_id {
        meta = meta.build_id(build_id);
    }
    if let Some(vendor) = &identity.vendor {
        meta = meta.extra("vendor", vendor);
    }
    let files = paths.map_or_else(DiagnosticsPaths::in_memory, |p| {
        DiagnosticsPaths::new(p.logs(), p.crash())
    });
    // A second builder in one process keeps the first runtime.
    let guard = nana_diagnostics::install(config, meta, files).ok();
    // `start` runs on the thread that will drive the event loop: register
    // it now so its first frame does not pay for ring allocation.
    nana_diagnostics::register_thread();
    guard
}

#[cfg(feature = "hosted")]
fn record_run_result(result: &Result<(), crate::HostedRunError>) {
    if let Err(error) = result {
        nana_diagnostics::fault!(
            nana_diagnostics::framework::host::RUN_FAILED;
            "{error}"
        );
    }
}
