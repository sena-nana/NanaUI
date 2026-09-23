//! `Nana.startup` (Issue #225): the host's startup, as JavaScript sees it.
//!
//! The state machine stays in the host. JS reads the one record (so a bundle
//! that attaches late asks for the state rather than waiting for an event it
//! already missed), asks to keep the Early Splash while its bundle is
//! evaluated, and asks to take over. Nothing here is a second lifecycle.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nana_js_engine::{HostApiRegistry, HostValue, JsException};
use nana_ui::{StartupHandle, StartupStatus, StartupTicket};

/// Framework host API names. Registered next to the application's own
/// `HostApiRegistry`; a clash fails startup like any other.
pub(crate) const STATUS: &str = "startupStatus";
pub(crate) const DEFER: &str = "startupDeferTakeover";
pub(crate) const TAKE_OVER: &str = "startupTakeOver";
pub(crate) const CANCEL: &str = "startupCancelTakeover";

/// What `VueRuntimeProgram` keeps of the startup: the host's record and the
/// bundle's wish to keep the splash.
#[derive(Clone)]
pub(crate) struct StartupBridge {
    handle: StartupHandle,
    deferred: Arc<AtomicBool>,
}

impl StartupBridge {
    pub(crate) fn new(handle: StartupHandle) -> Self {
        Self {
            handle,
            deferred: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Whether the bundle asked to keep the splash while it was evaluated.
    pub(crate) fn deferred(&self) -> bool {
        self.deferred.load(Ordering::Acquire)
    }

    pub(crate) fn register(&self, api: &mut HostApiRegistry) {
        let handle = self.handle.clone();
        api.register(STATUS, move |_| Ok(status_value(&handle.status())));
        let (handle, deferred) = (self.handle.clone(), Arc::clone(&self.deferred));
        api.register(DEFER, move |_| {
            // Read once, when `initialize` returns: a later call has nothing
            // left to change, and says so.
            let open = handle.status().phase == nana_ui::StartupPhase::Starting;
            if open {
                deferred.store(true, Ordering::Release);
            }
            Ok(HostValue::Bool(open))
        });
        let handle = self.handle.clone();
        api.register(TAKE_OVER, move |args| {
            let ticket = ticket_argument(&handle, args)?;
            handle
                .take_over(ticket)
                .map(|()| HostValue::Null)
                .map_err(startup_exception)
        });
        let handle = self.handle.clone();
        api.register(CANCEL, move |args| {
            let ticket = ticket_argument(&handle, args)?;
            handle
                .cancel_takeover(ticket)
                .map(|()| HostValue::Null)
                .map_err(startup_exception)
        });
    }
}

/// The ticket JS passed, or the current one when it passed none.
fn ticket_argument(
    handle: &StartupHandle,
    args: &[HostValue],
) -> Result<StartupTicket, JsException> {
    let current = handle.status().ticket;
    match args.first() {
        None | Some(HostValue::Null | HostValue::Undefined) => {
            current.ok_or_else(|| startup_exception(nana_ui::StartupError::StaleTicket))
        }
        Some(HostValue::Number(value)) => current
            .filter(|ticket| ticket_number(*ticket) == *value)
            .ok_or_else(|| startup_exception(nana_ui::StartupError::StaleTicket)),
        Some(_) => Err(JsException::new("startup ticket must be a number")),
    }
}

fn ticket_number(ticket: StartupTicket) -> f64 {
    ticket.generation() as f64
}

fn startup_exception(error: nana_ui::StartupError) -> JsException {
    let mut exception = JsException::new(error.to_string());
    exception.name = "StartupError".into();
    exception.code = Some(
        match error {
            nana_ui::StartupError::StaleTicket => "stale-ticket",
            nana_ui::StartupError::AlreadyHandedOff => "handed-off",
            nana_ui::StartupError::HostStopped => "host-stopped",
        }
        .into(),
    );
    exception
}

/// The record as a plain object: `phase`, `splash`, `ticket`, `timeline`
/// (milliseconds from the host's entry, or `null`).
pub(crate) fn status_value(status: &StartupStatus) -> HostValue {
    let millis = |value: Option<Duration>| {
        value.map_or(HostValue::Null, |value| {
            HostValue::Number(value.as_secs_f64() * 1e3)
        })
    };
    let timeline = &status.timeline;
    let timeline = BTreeMap::from([
        ("splashCommitted".into(), millis(timeline.splash_committed)),
        ("uiReady".into(), millis(timeline.ui_ready)),
        (
            "takeoverRequested".into(),
            millis(timeline.takeover_requested),
        ),
        (
            "firstFrameSubmitted".into(),
            millis(timeline.first_frame_submitted),
        ),
        (
            "handoffCompleted".into(),
            millis(timeline.handoff_completed),
        ),
        ("splashReleased".into(), millis(timeline.splash_released)),
    ]);
    let splash = match &status.splash {
        nana_ui::SplashOutcome::Shown {
            animation: nana_ui::SplashAnimationOutcome::Applied(_),
        } => "animated",
        nana_ui::SplashOutcome::Shown { .. } => "static",
        nana_ui::SplashOutcome::Skipped(_) => "skipped",
        nana_ui::SplashOutcome::Failed(_) => "failed",
    };
    HostValue::Object(BTreeMap::from([
        (
            "phase".into(),
            HostValue::String(status.phase.label().into()),
        ),
        ("splash".into(), HostValue::String(splash.into())),
        (
            "ticket".into(),
            status.ticket.map_or(HostValue::Null, |ticket| {
                HostValue::Number(ticket_number(ticket))
            }),
        ),
        ("timeline".into(), HostValue::Object(timeline)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(
        api: &HostApiRegistry,
        name: &str,
        args: &[HostValue],
    ) -> Result<HostValue, JsException> {
        api.call(name, args)
    }

    #[test]
    fn a_bundle_reads_the_record_and_may_defer_only_while_it_is_evaluated() {
        let handle = StartupHandle::detached();
        let bridge = StartupBridge::new(handle.clone());
        let mut api = HostApiRegistry::new();
        bridge.register(&mut api);
        let HostValue::Object(status) = call(&api, STATUS, &[]).unwrap() else {
            panic!("status is an object");
        };
        assert_eq!(status["phase"], HostValue::String("starting".into()));
        assert_eq!(status["ticket"], HostValue::Null);
        assert_eq!(call(&api, DEFER, &[]).unwrap(), HostValue::Bool(true));
        assert!(bridge.deferred());
        // No ticket before UiReady: a takeover is refused, not queued.
        let error = call(&api, TAKE_OVER, &[]).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("stale-ticket"));
    }
}
