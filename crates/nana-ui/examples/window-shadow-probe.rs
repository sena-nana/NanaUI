//! Desktop window shadow probe (#215). Opens a transparent window whose
//! visible body is an inset rounded card, then drives it through the issue's
//! fixtures and checks the shadow's work counters and the window's geometry:
//!
//! - A: `WindowShadow::Auto` on the transparent card window resolves to what
//!   the platform can show (a companion on macOS and Windows).
//! - static: changing the card's text repaints the window but rebuilds,
//!   rasterizes or recreates nothing for its shadow.
//! - C: moving the window only moves the companion (Windows counts the moves;
//!   AppKit moves the child window itself).
//! - D: resizing updates the companion's geometry without recreating it or
//!   rasterizing again.
//! - B / F: `WindowShadow::None` removes the companion, and turning the
//!   shadow off and on leaves the window's position and size exactly as they
//!   were: the shadow never adds a margin to the primary window.
//!
//! Prints one JSON line per fixture and exits non-zero when a gate fails or
//! the run passes its wall-clock cap. Run it with the display awake:
//!
//! ```text
//! cargo run -p nana-ui --example window-shadow-probe --features hosted,bundled-fonts
//! ```
use std::convert::Infallible;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nana_ui::runtime::view::{entity_ref, widget, with_refs};
use nana_ui::runtime::{DocumentId, Entity, RuntimeDocument, Stack, Text};
use nana_ui::{
    DocumentAccessError, RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate,
    WindowDescriptor, WindowShadow, WindowShadowBackend, WindowShadowOutcome, WindowShadowWork,
    run_runtime,
};
use nana_ui_core::LengthSpec;
use nana_ui_platform::host::WindowCommand;
use nana_ui_platform::{WindowEvent, WindowGeometry, WindowId};

const TICK: Duration = Duration::from_millis(150);
/// Wall-clock cap: the whole run, settle included, ends within this many
/// ticks (15 s).
const MAX_TICKS: u32 = 100;
const ORIGIN: (f32, f32) = (160.0, 160.0);
const SIZE: (f64, f64) = (520.0, 360.0);
const INSET: f32 = 24.0;
const MOVES: u32 = 10;
const RESIZES: u32 = 6;
/// Ticks a request is given to land before it is checked.
const SETTLE: u32 = 4;

static FAILED: AtomicBool = AtomicBool::new(false);

/// Whether the platform draws the companion this probe gates. Elsewhere the
/// outcome must say why there is none, and only geometry is checked.
const COMPANION: bool = cfg!(any(target_os = "macos", target_os = "windows"));

#[derive(Debug, Clone, Copy)]
enum Message {
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Settle,
    Static,
    Move,
    Resize,
    Off,
    On,
    Done,
}

struct Probe {
    document: RuntimeDocument,
    label: Entity<Text>,
    presented: bool,
    ticks: u32,
    phase: Phase,
    step: u32,
    /// Counters at the start of the phase being checked.
    mark: WindowShadowWork,
    /// Geometry with the shadow on, before it is turned off.
    geometry_on: Option<WindowGeometry>,
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

fn work_json(work: WindowShadowWork) -> serde_json::Value {
    serde_json::json!({
        "shape_rebuilds": work.shape_rebuilds,
        "effect_updates": work.effect_updates,
        "rasterizations": work.rasterizations,
        "resource_recreates": work.resource_recreates,
        "companion_moves": work.companion_moves,
        "companion_resizes": work.companion_resizes,
        "companion_count": work.companion_count,
    })
}

fn outcome_json(outcome: WindowShadowOutcome) -> serde_json::Value {
    serde_json::json!({
        "backend": format!("{:?}", outcome.backend),
        "fallback": outcome.fallback.map(|reason| format!("{reason:?}")),
    })
}

fn geometry_json(geometry: WindowGeometry) -> serde_json::Value {
    serde_json::json!({
        "physical_position": geometry.physical_position,
        "physical_size": geometry.physical_size,
        "logical_size": geometry.logical_size,
        "scale_factor": geometry.scale_factor,
    })
}

/// The window's frame as the platform reports it, without the scale factor,
/// which a toggle cannot change.
fn frame(geometry: WindowGeometry) -> (Option<(i32, i32)>, (u32, u32)) {
    (geometry.physical_position, geometry.physical_size)
}

impl Probe {
    fn mount() -> Self {
        let document_id = DocumentId::new(1).expect("probe document");
        let mut document = RuntimeDocument::new(document_id);
        let (_, label) = document
            .context_mut()
            .mount_view_root(document_id, || {
                let label = entity_ref::<Text>();
                // The transparent window paints only this card: the shadow
                // follows it, not the client area or any input region.
                let root = Stack::column(0.0).with_layout(|layout| {
                    layout.padding = Some(LengthSpec::Px(INSET));
                    layout.width = Some(LengthSpec::Percent(100.0));
                    layout.height = Some(LengthSpec::Percent(100.0));
                });
                let card = Stack::column(12.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Percent(100.0));
                    layout.height = Some(LengthSpec::Percent(100.0));
                    layout.padding = Some(LengthSpec::Px(20.0));
                    layout.background = Some([0.96, 0.96, 0.97, 1.0]);
                    layout.border_radius = Some(16.0);
                });
                let tree = widget(root).children(widget(card).children((
                    widget(Text::new("Window shadow probe")),
                    widget(Text::new("tick 0")).entity_ref(label),
                )));
                with_refs(tree, label)
            })
            .expect("probe view");
        Self {
            document,
            label,
            presented: false,
            ticks: 0,
            phase: Phase::Settle,
            step: 0,
            mark: WindowShadowWork::default(),
            geometry_on: None,
        }
    }

    fn enter(&mut self, phase: Phase, work: WindowShadowWork) {
        self.phase = phase;
        self.step = 0;
        self.mark = work;
    }

    /// Repaint the card with new text: content that does not move the body.
    fn touch_content(&mut self) {
        let tick = self.ticks;
        let _ = self
            .document
            .context_mut()
            .update_component(self.label, |text, _| {
                text.value = format!("tick {tick}");
            });
    }

    fn advance(&mut self, context: &RuntimeProgramContext<Message>) -> RuntimeProgramUpdate {
        let work = context.shadow_work();
        let outcome = context.presentation().shadow();
        let geometry = context.geometry();
        let window = context.window();
        self.step += 1;
        match self.phase {
            Phase::Settle => {
                let pending =
                    outcome.fallback == Some(nana_ui_platform::WindowShadowFallback::Pending);
                if self.presented && !pending && self.step >= SETTLE {
                    let expected = if COMPANION {
                        outcome.backend == WindowShadowBackend::Companion
                            && work.companion_count == 1
                    } else {
                        outcome.backend == WindowShadowBackend::None && outcome.fallback.is_some()
                    };
                    check(
                        "A-auto-transparent-card",
                        expected,
                        serde_json::json!({
                            "outcome": outcome_json(outcome),
                            "work": work_json(work),
                            "geometry": geometry_json(geometry),
                        }),
                    );
                    self.enter(Phase::Static, work);
                }
            }
            Phase::Static => {
                self.touch_content();
                if self.step > SETTLE + 2 {
                    let still = work.shape_rebuilds == self.mark.shape_rebuilds
                        && work.effect_updates == self.mark.effect_updates
                        && work.rasterizations == self.mark.rasterizations
                        && work.resource_recreates == self.mark.resource_recreates
                        && work.companion_resizes == self.mark.companion_resizes;
                    check(
                        "static-content-repaint",
                        still,
                        serde_json::json!({ "before": work_json(self.mark), "after": work_json(work) }),
                    );
                    self.enter(Phase::Move, work);
                }
                return RuntimeProgramUpdate::redraw_all();
            }
            Phase::Move => {
                if self.step <= MOVES {
                    let offset = self.step as f32 * 12.0;
                    let _request =
                        window.set_position((ORIGIN.0 + offset, ORIGIN.1 + offset * 0.5));
                } else if self.step == MOVES + 1 {
                    let _request = window.set_position(ORIGIN);
                } else if self.step > MOVES + 1 + SETTLE {
                    let only_moved = work.shape_rebuilds == self.mark.shape_rebuilds
                        && work.effect_updates == self.mark.effect_updates
                        && work.rasterizations == self.mark.rasterizations
                        && work.resource_recreates == self.mark.resource_recreates
                        && work.companion_resizes == self.mark.companion_resizes;
                    // A Windows companion is moved by the owner's window
                    // procedure and counts it; AppKit moves a child window
                    // with its parent, which costs this process nothing.
                    let moved = !cfg!(target_os = "windows")
                        || work.companion_moves > self.mark.companion_moves;
                    check(
                        "C-move-only",
                        only_moved && moved,
                        serde_json::json!({ "before": work_json(self.mark), "after": work_json(work) }),
                    );
                    self.enter(Phase::Resize, work);
                }
            }
            Phase::Resize => {
                if self.step <= RESIZES {
                    let grow = if self.step % 2 == 1 { 48.0 } else { 0.0 };
                    let _request = window.set_size((SIZE.0 + grow, SIZE.1 + grow));
                } else if self.step == RESIZES + 1 {
                    let _request = window.set_size(SIZE);
                } else if self.step > RESIZES + 1 + SETTLE {
                    // Geometry may update; nothing is recreated or rasterized.
                    let bounded = work.rasterizations == self.mark.rasterizations
                        && work.resource_recreates == self.mark.resource_recreates
                        && work.companion_count == self.mark.companion_count;
                    let followed =
                        !COMPANION || work.companion_resizes > self.mark.companion_resizes;
                    check(
                        "D-resize",
                        bounded && followed,
                        serde_json::json!({ "before": work_json(self.mark), "after": work_json(work) }),
                    );
                    self.geometry_on = Some(geometry);
                    self.enter(Phase::Off, work);
                    return RuntimeProgramUpdate {
                        window_commands: vec![WindowCommand::SetShadow {
                            id: WindowId::PRIMARY,
                            shadow: WindowShadow::None,
                        }],
                        ..RuntimeProgramUpdate::default()
                    };
                }
            }
            Phase::Off => {
                if self.step > SETTLE {
                    let on = self.geometry_on.expect("geometry before the toggle");
                    let disabled =
                        outcome == WindowShadowOutcome::disabled() && work.companion_count == 0;
                    check(
                        "B-shadow-off",
                        disabled,
                        serde_json::json!({ "outcome": outcome_json(outcome), "work": work_json(work) }),
                    );
                    check(
                        "F-geometry-off",
                        frame(on) == frame(geometry),
                        serde_json::json!({ "on": geometry_json(on), "off": geometry_json(geometry) }),
                    );
                    self.enter(Phase::On, work);
                    return RuntimeProgramUpdate {
                        window_commands: vec![WindowCommand::SetShadow {
                            id: WindowId::PRIMARY,
                            shadow: WindowShadow::Auto,
                        }],
                        ..RuntimeProgramUpdate::default()
                    };
                }
            }
            Phase::On => {
                if self.step > SETTLE {
                    let on = self.geometry_on.expect("geometry before the toggle");
                    let restored = !COMPANION
                        || (outcome.backend == WindowShadowBackend::Companion
                            && work.companion_count == 1);
                    check(
                        "A-shadow-back-on",
                        restored,
                        serde_json::json!({ "outcome": outcome_json(outcome), "work": work_json(work) }),
                    );
                    check(
                        "F-geometry-on",
                        frame(on) == frame(geometry),
                        serde_json::json!({ "first": geometry_json(on), "again": geometry_json(geometry) }),
                    );
                    self.phase = Phase::Done;
                    return RuntimeProgramUpdate::exit();
                }
            }
            Phase::Done => return RuntimeProgramUpdate::exit(),
        }
        RuntimeProgramUpdate::default()
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
    let mut settings = WindowDescriptor::new("NanaUI window shadow probe")
        .initial_size(SIZE.0, SIZE.1)
        .minimum_size(320.0, 240.0);
    settings.transparent = true;
    settings.initial_position = Some((f64::from(ORIGIN.0), f64::from(ORIGIN.1)));
    settings.shadow = WindowShadow::Auto;
    run_runtime::<Probe>(settings)?;
    if FAILED.load(Ordering::SeqCst) {
        std::process::exit(1);
    }
    report(serde_json::json!({ "passed": true }));
    Ok(())
}
