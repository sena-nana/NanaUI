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

use std::fmt;

use nana_diagnostics::{
    Diagnostics, DiagnosticsConfig, DiagnosticsGuard, DiagnosticsPaths, SessionMetadata,
};
use nana_ui_platform::{ApplicationIdentity, ApplicationPaths, PathsError, RuntimeLayout};

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

    /// The paths published by the running application, if it was started
    /// through a builder.
    pub fn paths() -> Option<&'static ApplicationPaths> {
        ApplicationPaths::current()
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

#[derive(Debug)]
pub enum ApplicationStartError {
    Paths(PathsError),
}

impl fmt::Display for ApplicationStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paths(e) => write!(f, "cannot resolve application paths: {e}"),
        }
    }
}

impl std::error::Error for ApplicationStartError {}

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
    /// resolved, the session has no paths and diagnostics stay in memory
    /// (snapshots cannot be written). Use [`Self::try_start`] to treat that
    /// as an error instead.
    pub fn start(self) -> ApplicationSession {
        match self.try_start() {
            Ok(session) => session,
            Err((builder, error)) => {
                eprintln!("NanaUI: {error}; running without application paths");
                builder.start_without_paths()
            }
        }
    }

    /// Like [`Self::start`], but hands the builder back when the paths
    /// cannot be resolved.
    #[allow(clippy::result_large_err)]
    pub fn try_start(self) -> Result<ApplicationSession, (Self, ApplicationStartError)> {
        let resolved = match &self.paths {
            Some(paths) => Ok(paths.clone()),
            None => ApplicationPaths::resolve(&self.identity),
        };
        match resolved {
            Ok(paths) => {
                // First publisher wins; a second builder in one process
                // shares it.
                let paths = paths.install_current();
                let diagnostics_paths = DiagnosticsPaths::new(paths.logs(), paths.crash());
                let diagnostics = self.install_diagnostics(Some(paths), diagnostics_paths);
                Ok(ApplicationSession {
                    paths: Some(paths),
                    diagnostics,
                })
            }
            Err(error) => Err((self, ApplicationStartError::Paths(error))),
        }
    }

    fn start_without_paths(self) -> ApplicationSession {
        let diagnostics = self.install_diagnostics(None, DiagnosticsPaths::in_memory());
        ApplicationSession {
            paths: None,
            diagnostics,
        }
    }

    fn install_diagnostics(
        self,
        paths: Option<&ApplicationPaths>,
        diagnostic_paths: DiagnosticsPaths,
    ) -> Option<DiagnosticsGuard> {
        if !self.diagnostics.enabled {
            return None;
        }

        let identity = &self.identity;
        let layout = match paths.map(ApplicationPaths::layout) {
            Some(RuntimeLayout::Installed) => "installed",
            Some(RuntimeLayout::Portable) => "portable",
            Some(RuntimeLayout::Development) => "development",
            None => "unresolved",
        };
        let mut meta = SessionMetadata::new(&identity.id, &identity.name, &identity.version)
            .framework_version(env!("CARGO_PKG_VERSION"))
            .extra("layout", layout);
        if let Some(build_id) = &identity.build_id {
            meta = meta.build_id(build_id);
        }
        if let Some(vendor) = &identity.vendor {
            meta = meta.extra("vendor", vendor);
        }
        // A second builder in one process keeps the first runtime.
        nana_diagnostics::install(self.diagnostics, meta, diagnostic_paths).ok()
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

#[cfg(feature = "hosted")]
fn record_run_result(result: &Result<(), crate::HostedRunError>) {
    if let Err(error) = result {
        nana_diagnostics::fault!(
            nana_diagnostics::framework::host::RUN_FAILED;
            "{error}"
        );
    }
}
