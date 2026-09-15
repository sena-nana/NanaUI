//! Real Surface acceptance for closing the primary Vue document independently.
//!
//! With `--isolated` the auxiliary window runs in its own JavaScript realm on the
//! same GPU, and must not see the main realm's `localStorage`.
use nana_js_engine::{HostValue, JsEngine};
use nana_ui::{RuntimeProgram, RuntimeProgramContext, WindowHandle, WindowRequest};
use nana_ui_platform::WindowId;
use nana_ui_vue::{VueMessage, VueRuntimeProgram, VueWindowId};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

static PASSED: AtomicBool = AtomicBool::new(false);

pub struct Probe {
    primary: WindowHandle,
    auxiliary: Option<WindowHandle>,
    focusing: Option<WindowRequest<()>>,
    generation: u64,
    presented: BTreeSet<WindowId>,
    primary_closed: bool,
    presented_after_close: bool,
    closing: Option<WindowRequest<()>>,
    isolated: bool,
}
impl Probe {
    pub fn new(context: &RuntimeProgramContext<VueMessage>, isolated: bool) -> Self {
        Self {
            isolated,
            primary: context.window(),
            auxiliary: None,
            focusing: None,
            generation: context.gpu().generation(),
            presented: BTreeSet::new(),
            primary_closed: false,
            presented_after_close: false,
            closing: None,
        }
    }
    pub fn ready(&mut self, id: WindowId, context: &RuntimeProgramContext<VueMessage>) {
        if id != WindowId::PRIMARY {
            self.auxiliary = Some(context.window());
        }
    }
    pub fn presented(&mut self, id: WindowId, context: &RuntimeProgramContext<VueMessage>) {
        assert_eq!(
            context.gpu().generation(),
            self.generation,
            "Vue windows changed shared GPU"
        );
        if !self.presented.contains(&id) || self.primary_closed {
            eprintln!(
                "nana lifecycle present id={} generation={} primary_closed={}",
                id.0,
                context.gpu().generation(),
                self.primary_closed
            );
        }
        self.presented.insert(id);
        if id == WindowId::PRIMARY && self.presented.len() == 1 && self.focusing.is_none() {
            // Startup shows the primary last; expose the auxiliary Surface before
            // waiting for its first frame on platforms that suppress occluded draws.
            if let Some(auxiliary) = &self.auxiliary {
                self.focusing = Some(auxiliary.focus());
            }
        }
        if self.primary_closed && id != WindowId::PRIMARY && !self.presented_after_close {
            assert!(matches!(
                self.closing.as_mut().unwrap().try_take(),
                Some(Ok(()))
            ));
            self.presented_after_close = true;
            self.closing = Some(context.window().close());
        } else if self.closing.is_none()
            && self.presented.contains(&WindowId::PRIMARY)
            && self.presented.len() >= 2
        {
            self.closing = Some(self.primary.close());
        }
    }
    pub fn closed(
        &mut self,
        id: WindowId,
        program: &mut VueRuntimeProgram<nana_js_v8::V8Engine>,
    ) -> bool {
        assert!(
            program.with_document(id, |_| ()).unwrap().is_none(),
            "closed Vue document retained by program"
        );
        for getter in ["Nana.windows.get", "__nanaGetWindowContext"] {
            let engine = program.runtime_mut().engine_mut();
            let function = engine.resolve_function(getter).unwrap();
            assert!(
                matches!(
                    engine
                        .invoke(function, &[HostValue::Number(id.0 as f64)])
                        .unwrap(),
                    HostValue::Null
                ),
                "closed JS window {} retained by {getter}",
                id.0
            );
        }
        if id == WindowId::PRIMARY {
            if self.isolated {
                let auxiliary = self
                    .auxiliary
                    .as_ref()
                    .expect("auxiliary window ready")
                    .id();
                let storage = |key: &str| {
                    program
                        .runtime()
                        .vue()
                        .host(VueWindowId(auxiliary.0))
                        .expect("isolated auxiliary document")
                        .lock()
                        .unwrap()
                        .host_api_registry()
                        .call(
                            "storageGet",
                            &[HostValue::string("local"), HostValue::string(key)],
                        )
                        .unwrap()
                };
                assert_eq!(
                    storage("nana.acceptance.realm").as_str(),
                    Some("isolated"),
                    "isolated realm did not run the application"
                );
                assert_eq!(
                    storage("nana.acceptance.sawMain").as_str(),
                    Some("null"),
                    "isolated realm saw the main realm's localStorage"
                );
                assert_eq!(
                    storage("nana.acceptance.timer").as_str(),
                    Some("fired"),
                    "isolated realm timer did not fire"
                );
            }
            assert!(
                program
                    .runtime_mut()
                    .engine_mut()
                    .resolve_function("__nanaHostedAcceptanceControl.state")
                    .is_err(),
                "primary Vue onBeforeUnmount did not finish"
            );
            if let Some(request) = self.focusing.as_mut() {
                assert!(matches!(request.try_take(), Some(Ok(()))));
            }
            self.primary_closed = true;
            true
        } else {
            assert!(
                self.presented_after_close,
                "auxiliary window did not present after primary close"
            );
            assert!(program.runtime().vue().window_ids().is_empty());
            PASSED.store(true, Ordering::Release);
            false
        }
    }
}
pub fn verify() {
    assert!(
        PASSED.load(Ordering::Acquire),
        "Vue native lifecycle probe did not finish"
    );
    println!(
        "Vue native lifecycle passed: primary released, auxiliary presented, final JS close delivered{}",
        if std::env::args().any(|arg| arg == "--isolated") {
            ", isolated realm kept its own localStorage"
        } else {
            ""
        }
    );
}
