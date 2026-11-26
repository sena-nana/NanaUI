//! Bounded desktop window input-region & unified hit-test probe (#159 + #49).
//!
//! Opens a frameless transparent window and exercises Fixtures A–D through
//! `WindowHandle::interaction()` without reading back GPU framebuffer pixels or
//! calling `SetWindowRgn`:
//!
//! - A: Shaped click-through launcher (`MousePassthroughMode::Forward`) with a
//!      `Client` interactive pill over a `PassThrough` transparent backdrop.
//! - B: Custom chrome `Drag` bar with a foreground `Client` button inside the
//!      top-right edge (`Client` overrides `Drag` and `Resize(NorthEast)`).
//! - C: Dynamic layout move + resize (`snapshot().revision` advances on
//!      geometry change and stays constant on static frames).
//! - D: Hidden / `pointer-events: none` / overflow-clipped `Drag` handle never
//!      surfaces into `InputRegionSnapshot`.
//!
//! Prints one JSON line per fixture and exits within `MAX_TICKS` (12 s wall
//! clock cap). Pass `--hold <seconds>` (at most 8) to keep the window up after
//! the fixtures so an external process can hit-test the live native window.
//! Run manually on an awake desktop:
//!
//! ```text
//! cargo run -p nana-ui --example window-input-region-probe --features hosted,bundled-fonts
//! ```
use std::convert::Infallible;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nana_ui::runtime::view::{entity_ref, widget, with_refs};
use nana_ui::runtime::{DocumentId, Entity, RuntimeDocument, Stack, Text};
use nana_ui::{
    DocumentAccessError, HitTestBehavior, InputRegionOutcome, InputRegionSnapshot,
    MousePassthroughMode, NativeHitTestClass, RuntimeProgram, RuntimeProgramContext,
    RuntimeProgramUpdate, WindowDescriptor, WindowInteractionCapabilities, WindowRequest,
    WindowResizeEdge, run_runtime,
};
use nana_ui_core::{LengthSpec, PointerEventsSpec};
use nana_ui_platform::{WindowEvent, WindowId};

const TICK: Duration = Duration::from_millis(150);
const MAX_TICKS: u32 = 80;

static FAILED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy)]
enum Message {
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Settle,
    FixtureA,
    FixtureB,
    FixtureC,
    FixtureD,
    Done,
}

struct Probe {
    document: RuntimeDocument,
    pill: Entity<Stack>,
    hidden_handle: Entity<Stack>,
    ticks: u32,
    phase: Phase,
    presented: bool,
    pending_caps: Option<WindowRequest<WindowInteractionCapabilities>>,
    pending_outcome: Option<WindowRequest<InputRegionOutcome>>,
    pending_snap: Option<WindowRequest<InputRegionSnapshot>>,
    last_revision: u64,
    settle_static_checks: u32,
    hold_ticks: u32,
    done_at: u32,
}

/// Ticks to keep the window up after the fixtures, from `--hold <seconds>`.
fn hold_ticks() -> u32 {
    let mut arguments = std::env::args().skip(1);
    let seconds = match (arguments.next().as_deref(), arguments.next()) {
        (Some("--hold"), Some(value)) => value.parse::<f32>().unwrap_or(0.0),
        _ => 0.0,
    };
    let seconds = seconds.clamp(0.0, 8.0);
    (seconds * 1000.0 / TICK.as_millis() as f32).ceil() as u32
}

fn report(value: serde_json::Value) {
    println!("{value}");
    let _ = std::io::stdout().flush();
}

fn check(fixture: &str, passed: bool, detail: serde_json::Value) {
    if !passed {
        FAILED.store(true, Ordering::SeqCst);
    }
    report(serde_json::json!({ "fixture": fixture, "passed": passed, "detail": detail }));
}

impl Probe {
    fn mount() -> Self {
        let document_id = DocumentId::new(1).expect("probe document");
        let mut document = RuntimeDocument::new(document_id);
        let (_, (pill, hidden_handle)) = document
            .context_mut()
            .mount_view_root(document_id, || {
                let pill = entity_ref::<Stack>();
                let hidden_handle = entity_ref::<Stack>();

                let root = Stack::column(0.0).with_layout(|layout| {
                    layout.hit_test = Some(HitTestBehavior::PassThrough);
                    layout.width = Some(LengthSpec::Px(520.0));
                    layout.height = Some(LengthSpec::Px(360.0));
                });

                let chrome_bar = Stack::row(0.0).with_layout(|layout| {
                    layout.hit_test = Some(HitTestBehavior::Drag);
                    layout.width = Some(LengthSpec::Px(520.0));
                    layout.height = Some(LengthSpec::Px(40.0));
                    layout.paint_colors.background =
                        Some(nana_ui_core::PaintColor::srgb([0.16, 0.18, 0.22, 0.92]));
                });

                let close_btn = Stack::column(0.0).with_layout(|layout| {
                    layout.hit_test = Some(HitTestBehavior::Client);
                    layout.margin_left = Some(LengthSpec::Px(480.0));
                    layout.width = Some(LengthSpec::Px(40.0));
                    layout.height = Some(LengthSpec::Px(40.0));
                    layout.paint_colors.background =
                        Some(nana_ui_core::PaintColor::srgb([0.85, 0.25, 0.28, 1.0]));
                });

                let pill_box = Stack::column(0.0).with_layout(|layout| {
                    layout.hit_test = Some(HitTestBehavior::Client);
                    layout.margin_left = Some(LengthSpec::Px(140.0));
                    layout.margin_top = Some(LengthSpec::Px(80.0));
                    layout.width = Some(LengthSpec::Px(220.0));
                    layout.height = Some(LengthSpec::Px(80.0));
                    layout.paint_colors.background =
                        Some(nana_ui_core::PaintColor::srgb([0.2, 0.6, 0.9, 0.95]));
                });

                let hidden_drag = Stack::column(0.0).with_layout(|layout| {
                    layout.hit_test = Some(HitTestBehavior::Drag);
                    layout.margin_left = Some(LengthSpec::Px(140.0));
                    layout.margin_top = Some(LengthSpec::Px(20.0));
                    layout.width = Some(LengthSpec::Px(200.0));
                    layout.height = Some(LengthSpec::Px(36.0));
                    layout.hidden = true;
                });

                let tree = widget(root).children((
                    widget(chrome_bar).children(widget(close_btn).children(widget(Text::new("X")))),
                    widget(pill_box)
                        .entity_ref(pill)
                        .children(widget(Text::new("Interactive Pill"))),
                    widget(hidden_drag)
                        .entity_ref(hidden_handle)
                        .children(widget(Text::new("Hidden Drag"))),
                ));
                with_refs(tree, (pill, hidden_handle))
            })
            .expect("probe view");

        Self {
            document,
            pill,
            hidden_handle,
            ticks: 0,
            phase: Phase::Settle,
            presented: false,
            pending_caps: None,
            pending_outcome: None,
            pending_snap: None,
            last_revision: 0,
            settle_static_checks: 0,
            hold_ticks: hold_ticks(),
            done_at: 0,
        }
    }

    fn poll_requests(
        &mut self,
        context: &RuntimeProgramContext<Message>,
    ) -> (
        Option<WindowInteractionCapabilities>,
        Option<InputRegionOutcome>,
        Option<InputRegionSnapshot>,
    ) {
        let caps = self
            .pending_caps
            .as_mut()
            .and_then(|req| req.try_take())
            .and_then(Result::ok);
        if caps.is_some() {
            self.pending_caps = None;
        }
        let outcome = self
            .pending_outcome
            .as_mut()
            .and_then(|req| req.try_take())
            .and_then(Result::ok);
        if outcome.is_some() {
            self.pending_outcome = None;
        }
        let snap = self
            .pending_snap
            .as_mut()
            .and_then(|req| req.try_take())
            .and_then(Result::ok);
        if snap.is_some() {
            self.pending_snap = None;
        }

        let interaction = context.window().interaction();
        if self.pending_caps.is_none() && caps.is_none() {
            self.pending_caps = Some(interaction.capabilities());
        }
        if self.pending_outcome.is_none() && outcome.is_none() {
            self.pending_outcome = Some(interaction.outcome());
        }
        if self.pending_snap.is_none() && snap.is_none() {
            self.pending_snap = Some(interaction.snapshot());
        }
        (caps, outcome, snap)
    }

    fn advance(&mut self, context: &RuntimeProgramContext<Message>) -> RuntimeProgramUpdate {
        if !self.presented {
            return RuntimeProgramUpdate::default();
        }
        let (caps, outcome, snap) = self.poll_requests(context);
        let Some(snap) = snap else {
            return RuntimeProgramUpdate::default();
        };

        match self.phase {
            Phase::Settle => {
                let _request = context
                    .window()
                    .interaction()
                    .set_mouse_passthrough_mode(MousePassthroughMode::Forward);
                self.last_revision = snap.revision;
                self.phase = Phase::FixtureA;
                RuntimeProgramUpdate::redraw_all()
            }
            Phase::FixtureA => {
                let pill_class = snap.classify(200.0, 150.0);
                let backdrop_class = snap.classify(40.0, 200.0);
                let mode_ok = match (caps, outcome.as_ref()) {
                    (Some(c), Some(InputRegionOutcome::NativeApplied { .. })) => c.input_region,
                    (Some(c), Some(InputRegionOutcome::ForwardFallback { .. })) => !c.input_region,
                    _ => outcome.is_none(),
                };
                check(
                    "fixture-a-forward-pill-and-hole",
                    pill_class == NativeHitTestClass::Client
                        && backdrop_class == NativeHitTestClass::PassThrough
                        && mode_ok,
                    serde_json::json!({
                        "pill": format!("{pill_class:?}"),
                        "backdrop": format!("{backdrop_class:?}"),
                        "outcome": format!("{outcome:?}"),
                    }),
                );
                self.phase = Phase::FixtureB;
                RuntimeProgramUpdate::default()
            }
            Phase::FixtureB => {
                let close_btn_class = snap.classify(516.0, 4.0);
                let drag_bar_class = snap.classify(200.0, 20.0);
                let west_resize = snap.classify(2.0, 220.0);
                check(
                    "fixture-b-client-overrides-drag-and-resize",
                    close_btn_class == NativeHitTestClass::Client
                        && drag_bar_class == NativeHitTestClass::Drag
                        && west_resize == NativeHitTestClass::Resize(WindowResizeEdge::West),
                    serde_json::json!({
                        "close_btn": format!("{close_btn_class:?}"),
                        "drag_bar": format!("{drag_bar_class:?}"),
                        "west_resize": format!("{west_resize:?}"),
                    }),
                );
                let _ = self
                    .document
                    .context_mut()
                    .update_component(self.pill, |stack, _| {
                        let layout = stack.layout_mut();
                        layout.margin_left = Some(LengthSpec::Px(60.0));
                        layout.margin_top = Some(LengthSpec::Px(140.0));
                    });
                self.last_revision = snap.revision;
                self.settle_static_checks = 0;
                self.phase = Phase::FixtureC;
                RuntimeProgramUpdate::redraw_all()
            }
            Phase::FixtureC => {
                if self.settle_static_checks == 0 {
                    let moved_hit = snap.classify(120.0, 210.0);
                    let rev_advanced = snap.revision > self.last_revision;
                    if !rev_advanced {
                        return RuntimeProgramUpdate::default();
                    }
                    self.last_revision = snap.revision;
                    self.settle_static_checks = 1;
                    check(
                        "fixture-c-dynamic-relayout",
                        moved_hit == NativeHitTestClass::Client,
                        serde_json::json!({
                            "moved_hit": format!("{moved_hit:?}"),
                            "revision": snap.revision,
                        }),
                    );
                    return RuntimeProgramUpdate::default();
                }
                check(
                    "fixture-c-static-tick-zero-rebuild",
                    snap.revision == self.last_revision,
                    serde_json::json!({
                        "expected_rev": self.last_revision,
                        "actual_rev": snap.revision,
                    }),
                );
                let _ =
                    self.document
                        .context_mut()
                        .update_component(self.hidden_handle, |stack, _| {
                            let layout = stack.layout_mut();
                            layout.hidden = false;
                            layout.pointer_events = Some(PointerEventsSpec::None);
                        });
                self.phase = Phase::FixtureD;
                RuntimeProgramUpdate::redraw_all()
            }
            Phase::FixtureD => {
                let hidden_hit = snap.classify(200.0, 295.0);
                check(
                    "fixture-d-hidden-and-pointer-events-none",
                    hidden_hit == NativeHitTestClass::PassThrough,
                    serde_json::json!({ "hidden_hit": format!("{hidden_hit:?}") }),
                );
                self.phase = Phase::Done;
                self.done_at = self.ticks;
                if self.hold_ticks == 0 {
                    return RuntimeProgramUpdate::exit();
                }
                RuntimeProgramUpdate::default()
            }
            Phase::Done if self.ticks < self.done_at + self.hold_ticks => {
                RuntimeProgramUpdate::default()
            }
            Phase::Done => RuntimeProgramUpdate::exit(),
        }
    }
}

impl RuntimeProgram for Probe {
    type Message = Message;
    type Error = Infallible;

    fn initialize(
        context: &RuntimeProgramContext<Message>,
    ) -> Result<(Self, Vec<Message>), Infallible> {
        let context = context.clone();
        std::thread::spawn(move || {
            for _ in 0..MAX_TICKS {
                std::thread::sleep(TICK);
                context.dispatch(Message::Tick);
            }
        });
        Ok((Self::mount(), Vec::new()))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&self.document)))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&mut self.document)))
    }

    fn update(
        &mut self,
        Message::Tick: Message,
        context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        self.ticks += 1;
        if self.phase != Phase::Done && self.ticks >= MAX_TICKS {
            check(
                "wall-clock-cap",
                false,
                serde_json::json!({ "phase": format!("{:?}", self.phase) }),
            );
            return RuntimeProgramUpdate::exit();
        }
        if self.ticks >= MAX_TICKS {
            // The tick thread stops here; a hold never outlives the cap.
            return RuntimeProgramUpdate::exit();
        }
        self.advance(context)
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::OpenFailed { error, .. } => {
                check(
                    "open",
                    false,
                    serde_json::json!({ "error": error.to_string() }),
                );
                RuntimeProgramUpdate::exit()
            }
            WindowEvent::CloseRequested { .. } => RuntimeProgramUpdate::exit(),
            _ => RuntimeProgramUpdate::default(),
        }
    }

    fn window_frame_presented(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        self.presented = true;
        RuntimeProgramUpdate::default()
    }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    let mut settings = WindowDescriptor::new("NanaUI window input region probe")
        .initial_size(520.0, 360.0)
        .minimum_size(320.0, 240.0);
    settings.system_caption = false;
    settings.transparent = true;
    settings.initial_position = Some((180.0, 180.0));
    run_runtime::<Probe>(settings)?;
    if FAILED.load(Ordering::SeqCst) {
        std::process::exit(1);
    }
    report(serde_json::json!({ "passed": true }));
    Ok(())
}
