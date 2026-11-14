//! Time-based overlay show/hide policy. Hosts supply clocks and lock flags
//! through [`crate::AppContext::sync_overlay_visibility`], or let the runtime
//! drive a bar's policy from routed input and its own clock
//! ([`crate::MediaTransportBar::auto_hide`]). The policy does not own windows
//! or media, and is not a leaf control.
//!
//! Chrome that follows the pointer drives the same policy directly: the
//! pointer coming in is [`OverlayVisibility::set_pointer_inside`], leaving is
//! [`OverlayVisibility::conceal`] (after [`OverlayVisibilityConfig::leave_grace`],
//! which absorbs a leave immediately followed by an enter at an edge), a
//! keyboard reveal is [`OverlayVisibility::reveal`], interactions that must not
//! be hidden are [`OverlayLocks`], and with `active = false` nothing hides it
//! on a clock. Chrome that fades reports the end of each transition to
//! [`OverlayVisibility::transition_finished`] (from
//! [`crate::PresenceLifecycle::animation_events`] or its own animations), so
//! [`OverlayVisibility::phase`] knows when it is at rest.
//!
//! ```ignore
//! let mut chrome = OverlayVisibility::hidden(OverlayVisibilityConfig {
//!     leave_grace: Duration::from_millis(80),
//!     ..OverlayVisibilityConfig::default()
//! });
//! chrome.set_pointer_inside(now, true);           // reveal: start the fade in
//! chrome.conceal(now);                            // the pointer left
//! chrome.synchronize(now, false, locks);          // every update
//! chrome.tick(now);                               // at chrome.wakeup()
//! chrome.transition_finished(false);              // the fade out landed
//! ```

use std::time::{Duration, Instant};

use crate::{PresencePhase, PresenceSettled};

/// Idle auto-hide after this long while the overlay is not locked.
pub const OVERLAY_IDLE: Duration = Duration::from_secs(3);

/// Pointer / menu / drag locks that keep chrome visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayLocks {
    pub focused: bool,
    pub dragging: bool,
    pub menu_open: bool,
    /// An interaction the host tracks itself: a confirmation it opened from
    /// the chrome, text editing, a gesture over it.
    pub interaction: bool,
}

impl OverlayLocks {
    pub fn held(self) -> bool {
        self.focused || self.dragging || self.menu_open || self.interaction
    }
}

/// Timing knobs. Zero hover dwell reveals immediately; zero startup skips the
/// initial forced-visible window; zero leave grace hides as the pointer
/// leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayVisibilityConfig {
    pub idle: Duration,
    pub hover_dwell: Duration,
    pub startup: Duration,
    /// How long after [`OverlayVisibility::conceal`] (or a hold ending with
    /// the pointer away) the overlay hides; the pointer coming back within it
    /// keeps it up.
    pub leave_grace: Duration,
}

impl Default for OverlayVisibilityConfig {
    fn default() -> Self {
        Self {
            idle: OVERLAY_IDLE,
            hover_dwell: Duration::ZERO,
            startup: Duration::ZERO,
            leave_grace: Duration::ZERO,
        }
    }
}

/// A bar's idle visibility flipped. Emitted by the bar whichever way its
/// policy is driven, so chrome outside the bar (a title bar over the same
/// picture) can show and hide with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayVisibilityChanged {
    pub visible: bool,
}

/// Auto-hide machine for media chrome, stage HUDs and chrome that follows
/// the pointer. Not a leaf control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayVisibility {
    config: OverlayVisibilityConfig,
    /// The logical target is [`Self::visible`]; the phase adds whether the
    /// transition toward it has landed.
    phase: PresencePhase,
    active: bool,
    held: bool,
    /// The pointer is away: hidden by [`Self::conceal`] until the next
    /// activity.
    concealed: bool,
    deadline: Option<Instant>,
    /// When a conceal inside the leave grace hides.
    leave_at: Option<Instant>,
    startup_until: Option<Instant>,
    hot_since: Option<Instant>,
}

impl Default for OverlayVisibility {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl OverlayVisibility {
    pub fn new(now: Instant) -> Self {
        Self::with_config(now, OverlayVisibilityConfig::default())
    }

    pub fn with_config(now: Instant, config: OverlayVisibilityConfig) -> Self {
        let startup_until = (!config.startup.is_zero()).then(|| now + config.startup);
        Self {
            config,
            phase: PresencePhase::Shown,
            active: false,
            held: false,
            concealed: false,
            deadline: None,
            leave_at: None,
            startup_until,
            hot_since: None,
        }
    }

    /// Settled hidden with the pointer away, until the first activity or
    /// [`Self::reveal`]: chrome that appears only once the pointer comes.
    /// There is no startup window.
    pub fn hidden(config: OverlayVisibilityConfig) -> Self {
        Self {
            config,
            phase: PresencePhase::Hidden,
            active: false,
            held: false,
            concealed: true,
            deadline: None,
            leave_at: None,
            startup_until: None,
            hot_since: None,
        }
    }

    pub fn config(&self) -> OverlayVisibilityConfig {
        self.config
    }

    /// The logical target: shown, or on its way there.
    pub fn visible(&self) -> bool {
        matches!(self.phase, PresencePhase::Shown | PresencePhase::Revealing)
    }

    /// Where the overlay is, as far as [`Self::transition_finished`] has
    /// reported. An overlay whose host never reports stays `Revealing` or
    /// `Concealing` after a flip.
    pub fn phase(&self) -> PresencePhase {
        self.phase
    }

    pub(crate) fn concealed(&self) -> bool {
        self.concealed
    }

    /// Up, not concealed, and counting down to an idle hide: activity now
    /// would only move that deadline later.
    pub(crate) fn counting_down(&self) -> bool {
        self.visible() && !self.concealed && self.deadline.is_some()
    }

    pub fn wakeup(&self) -> Option<Instant> {
        [
            self.deadline,
            self.leave_at,
            self.startup_until,
            self.hot_since.map(|hot| hot + self.config.hover_dwell),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Point the target at `visible`, starting a transition when it changes.
    fn set_visible(&mut self, visible: bool) {
        if visible == self.visible() {
            return;
        }
        self.phase = if visible {
            PresencePhase::Revealing
        } else {
            PresencePhase::Concealing
        };
    }

    /// The pointer is away and nothing holds the overlay: hide once the
    /// leave grace has passed. A pending leave keeps its deadline.
    fn leave(&mut self, now: Instant) {
        if !self.visible() || self.leave_at.is_some() {
            return;
        }
        if self.config.leave_grace.is_zero() {
            self.deadline = None;
            self.set_visible(false);
        } else {
            self.leave_at = Some(now + self.config.leave_grace);
        }
    }

    /// `active` means the surface is in a mode that may hide chrome (playing
    /// media, idle stage). Loading / paused / empty pass false and stay up.
    ///
    /// A hold keeps the overlay up, and brings back one that is still fading
    /// out; one that has settled hidden stays hidden. When a hold ends with
    /// the pointer away, the overlay hides after the leave grace.
    pub fn synchronize(&mut self, now: Instant, active: bool, locks: OverlayLocks) -> bool {
        let old = self.visible();
        let held = locks.held();
        let released = self.held && !held;
        if self.startup_until.is_some_and(|until| now >= until) {
            self.startup_until = None;
        }
        if held {
            if self.phase != PresencePhase::Hidden {
                self.set_visible(true);
            }
            self.deadline = None;
            self.leave_at = None;
        } else if self.concealed && (released || !self.visible()) {
            self.deadline = None;
            self.leave(now);
        } else if !active || self.startup_until.is_some() {
            if !self.concealed {
                self.set_visible(true);
            }
            self.deadline = None;
        } else if !self.active || self.held || (self.visible() && self.deadline.is_none()) {
            self.deadline = Some(now + self.config.idle);
        }
        self.active = active;
        self.held = held;
        old != self.visible()
    }

    /// Immediate reveal (pointer / keyboard activity): the pointer is back,
    /// or a key counts as being back.
    pub fn activity(&mut self, now: Instant) -> bool {
        self.concealed = false;
        self.reveal(now)
    }

    /// Reveal without the pointer, e.g. keyboard navigation into chrome that
    /// follows the pointer: the pointer stays where it was, so a hold that
    /// ends with it away hides the overlay again.
    pub fn reveal(&mut self, now: Instant) -> bool {
        let changed = !self.visible();
        self.set_visible(true);
        self.leave_at = None;
        self.deadline = (self.active && !self.held).then_some(now + self.config.idle);
        changed
    }

    /// Hover dwell. `hot` true starts the dwell clock; false cancels it.
    pub fn set_pointer_inside(&mut self, now: Instant, hot: bool) -> bool {
        if hot {
            if self.config.hover_dwell.is_zero() {
                self.hot_since = None;
                return self.activity(now);
            }
            if self.hot_since.is_none() {
                self.hot_since = Some(now);
            }
            if now.saturating_duration_since(self.hot_since.unwrap()) >= self.config.hover_dwell {
                self.hot_since = None;
                return self.activity(now);
            }
            false
        } else {
            self.hot_since = None;
            false
        }
    }

    /// Hide, playing or not, until the next [`Self::activity`]: the pointer
    /// left the window. With a leave grace it hides once the grace has
    /// passed; a conceal while one is pending keeps that deadline. A held
    /// overlay (focus, drag, open menu) stays until the hold ends, then
    /// hides the same way.
    pub fn conceal(&mut self, now: Instant) -> bool {
        self.concealed = true;
        self.hot_since = None;
        if self.held {
            return false;
        }
        let shown = self.visible();
        self.leave(now);
        shown != self.visible()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        if self.startup_until.is_some_and(|until| now >= until) {
            self.startup_until = None;
        }
        if let Some(hot) = self.hot_since
            && now.saturating_duration_since(hot) >= self.config.hover_dwell
        {
            self.hot_since = None;
            return self.activity(now);
        }
        let left = self.leave_at.is_some_and(|due| due <= now);
        if left {
            self.leave_at = None;
        }
        let idle = self.deadline.is_some_and(|deadline| deadline <= now);
        if (left && !self.held) || idle {
            self.deadline = None;
            let changed = self.visible();
            self.set_visible(false);
            return changed;
        }
        false
    }

    /// The transition toward `shown` landed. Answers how the overlay came to
    /// rest; a finish that no longer matches the target (the pointer came
    /// back mid-fade) is stale and answers `None`.
    pub fn transition_finished(&mut self, shown: bool) -> Option<PresenceSettled> {
        match (self.phase, shown) {
            (PresencePhase::Revealing, true) => {
                self.phase = PresencePhase::Shown;
                Some(PresenceSettled::Shown)
            }
            (PresencePhase::Concealing, false) => {
                self.phase = PresencePhase::Hidden;
                Some(PresenceSettled::Hidden)
            }
            _ => None,
        }
    }
}

impl crate::AppContext {
    /// Whether `id` is `ancestor` or a descendant of it.
    pub fn is_descendant(&self, id: crate::StableNodeId, ancestor: crate::StableNodeId) -> bool {
        self.world().is_descendant_or_self(id, ancestor)
    }

    /// Focus, pointer capture, and open descendant menus for `root`.
    pub fn overlay_locks(
        &self,
        document: crate::DocumentId,
        root: crate::StableNodeId,
    ) -> OverlayLocks {
        self.overlay_locks_with(document, root, false)
    }

    /// [`Self::overlay_locks`], where with `focus_visible_only` only
    /// keyboard (focus-visible) focus holds.
    fn overlay_locks_with(
        &self,
        document: crate::DocumentId,
        root: crate::StableNodeId,
        focus_visible_only: bool,
    ) -> OverlayLocks {
        let focused = if focus_visible_only {
            self.world().focus_visible(document)
        } else {
            self.world().focused(document)
        };
        OverlayLocks {
            focused: focused.is_some_and(|focused| self.is_descendant(focused, root)),
            dragging: self
                .world()
                .pointer_captures(document)
                .into_iter()
                .any(|(_, owner)| self.is_descendant(owner, root)),
            menu_open: self.descendant_menu_open(root),
            interaction: false,
        }
    }

    /// Open popovers and action menus, and the detached options of a
    /// select, dropdown, search dropdown or color field, count as menus.
    fn descendant_menu_open(&self, root: crate::StableNodeId) -> bool {
        fn open<C: crate::View>(
            cx: &crate::AppContext,
            id: crate::StableNodeId,
            read: fn(&C) -> bool,
        ) -> bool {
            cx.view_entity::<C>(id)
                .and_then(|entity| cx.read(entity, read).ok())
                .unwrap_or(false)
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if open::<crate::ActionMenu>(self, id, |menu| menu.popover.open)
                || open::<crate::Popover>(self, id, |popover| popover.open)
                || open::<crate::Select>(self, id, |select| select.opened)
                || open::<crate::Dropdown>(self, id, |dropdown| dropdown.opened)
                || open::<crate::SearchDropdown>(self, id, |dropdown| dropdown.opened)
                || open::<crate::ColorField>(self, id, |field| field.opened || field.sampling)
            {
                return true;
            }
            stack.extend_from_slice(self.world().child_ids(id));
        }
        false
    }

    /// Collect locks from the world, drive the bar's overlay policy, and
    /// return the next wakeup instant. The bar hides while the policy says
    /// so ([`crate::MediaTransportBar::shown`]); its own `hidden` stays the
    /// application's.
    pub fn sync_overlay_visibility(
        &mut self,
        bar: crate::Entity<crate::MediaTransportBar>,
        now: Instant,
        active: bool,
    ) -> Result<Option<Instant>, crate::FrameworkError> {
        let (_, wakeup) = self.step_overlay(
            bar,
            OverlayStep {
                now,
                active,
                activity: None,
                conceal: false,
                focus_visible_only: false,
            },
        )?;
        Ok(wakeup)
    }

    /// One run of `bar`'s policy, however it is driven: activity, a menu that
    /// closed since the last run, the locks and `active`, the pointer leaving
    /// the window, then the clock. Answers whether the bar's visibility
    /// flipped, and its next wakeup.
    pub(crate) fn step_overlay(
        &mut self,
        bar: crate::Entity<crate::MediaTransportBar>,
        step: OverlayStep,
    ) -> Result<(bool, Option<Instant>), crate::FrameworkError> {
        let root = bar.stable_id();
        let document = self
            .world()
            .document_of(root)
            .ok_or(crate::FrameworkError::MissingView(root))?;
        let mut locks = self.overlay_locks_with(document, root, step.focus_visible_only);
        let menu_closed = self.read(bar, |bar| bar.menu_was_open)? && !locks.menu_open;
        // The menu hands focus back to its trigger, which would hold the bar
        // for good where any focus holds it.
        if menu_closed && locks.focused && !step.focus_visible_only {
            self.clear_focus(document)?;
            locks = self.overlay_locks_with(document, root, false);
        }
        self.update_component(bar, |bar, cx| {
            let before = bar.visibility.visible();
            if let Some(at) = step.activity {
                bar.visibility.activity(at);
            }
            if menu_closed {
                bar.visibility.activity(step.now);
            }
            bar.visibility.synchronize(step.now, step.active, locks);
            if step.conceal {
                bar.visibility.conceal(step.now);
            }
            bar.visibility.tick(step.now);
            bar.menu_was_open = locks.menu_open;
            bar.menu_toggled = false;
            report_visibility(before, bar, cx);
            (before != bar.visibility.visible(), bar.visibility.wakeup())
        })
    }

    /// Immediate reveal (pointer / keyboard activity over the chrome or stage).
    pub fn reveal_overlay(
        &mut self,
        bar: crate::Entity<crate::MediaTransportBar>,
        now: Instant,
    ) -> Result<bool, crate::FrameworkError> {
        self.update_component(bar, |bar, cx| {
            let (before, shown) = (bar.visibility.wakeup(), bar.visibility.visible());
            let changed = bar.visibility.activity(now);
            report_visibility(shown, bar, cx);
            changed || before != bar.visibility.wakeup()
        })
    }

    /// Hide the bar now until the next activity: the pointer left its window.
    /// See [`OverlayVisibility::conceal`]; a bar has no leave grace.
    pub fn conceal_overlay(
        &mut self,
        bar: crate::Entity<crate::MediaTransportBar>,
    ) -> Result<bool, crate::FrameworkError> {
        self.update_component(bar, |bar, cx| {
            let shown = bar.visibility.visible();
            let changed = bar.visibility.conceal(Instant::now());
            report_visibility(shown, bar, cx);
            changed
        })
    }

    pub fn overlay_wakeup(
        &self,
        bar: crate::Entity<crate::MediaTransportBar>,
    ) -> Result<Option<Instant>, crate::FrameworkError> {
        self.read(bar, |bar| bar.visibility.wakeup())
    }
}

/// What one run of a bar's policy acts on; see
/// [`crate::AppContext::step_overlay`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct OverlayStep {
    pub now: Instant,
    /// The bar may hide: playing media, an idle stage.
    pub active: bool,
    /// Pointer or key activity to apply first, at its own time.
    pub activity: Option<Instant>,
    /// The pointer left the window.
    pub conceal: bool,
    /// Only keyboard focus holds the bar, not focus a click left behind.
    pub focus_visible_only: bool,
}

/// Emit [`OverlayVisibilityChanged`] when the bar's idle visibility is no
/// longer `before`. The bar shows and hides at once, so its policy settles
/// as it flips.
pub(crate) fn report_visibility(
    before: bool,
    bar: &mut crate::MediaTransportBar,
    cx: &mut crate::ViewContext<'_, crate::MediaTransportBar>,
) {
    let visible = bar.visibility.visible();
    bar.visibility.transition_finished(visible);
    if visible != before {
        cx.emit(OverlayVisibilityChanged { visible });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AutoHideStage {
        cx: crate::AppContext,
        input: crate::HeadlessInput,
        document: crate::DocumentId,
        bar: crate::Entity<crate::MediaTransportBar>,
    }

    impl AutoHideStage {
        fn new(playing: bool) -> Self {
            use crate::{
                AppContext, DocumentId, HeadlessInput, LayoutViewport, MediaTransportBar, Stack,
            };
            use nana_ui_core::LengthSpec;

            let document = DocumentId::new(1).unwrap();
            let mut cx = AppContext::new();
            let window = cx
                .create_component(
                    document,
                    Stack::column(0.0).hittable().with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(960.0));
                        layout.height = Some(LengthSpec::Px(600.0));
                    }),
                )
                .unwrap();
            let stage = cx
                .create_detached_component(
                    document,
                    Stack::column(0.0).hittable().with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(640.0));
                        layout.height = Some(LengthSpec::Px(360.0));
                    }),
                )
                .unwrap();
            cx.append_child(window, stage).unwrap();
            let mut transport = MediaTransportBar::new().auto_hide(true);
            transport.playing = playing;
            transport.duration = 120.0;
            let bar = cx.create_detached_component(document, transport).unwrap();
            cx.append_child(stage, bar).unwrap();
            cx.assemble_media_transport_bar(bar).unwrap();
            cx.layout_document(document, LayoutViewport::new(960.0, 600.0))
                .unwrap();
            cx.compat_world_mut().rebuild_hit_test(document);
            let frame = cx.world().layout_box(stage.stable_id()).unwrap();
            assert!(frame.contains(100.0, 100.0));
            assert!(!frame.contains(800.0, 500.0));
            let input = HeadlessInput::bind(&mut cx, document);
            Self {
                cx,
                input,
                document,
                bar,
            }
        }

        fn shown(&self) -> bool {
            self.cx
                .read(self.bar, crate::MediaTransportBar::shown)
                .unwrap()
        }

        fn pointer(&mut self, phase: nana_ui_input::PointerPhase, x: f32, y: f32) {
            self.input.advance(Duration::from_millis(10));
            self.input.pointer(&mut self.cx, phase, x, y).unwrap();
        }

        fn key(&mut self, key: &'static str) {
            self.input.advance(Duration::from_millis(10));
            self.input
                .press(
                    &mut self.cx,
                    nana_ui_input::KeyInput::named(
                        key,
                        key,
                        nana_ui_input::KeyState::Pressed,
                        Default::default(),
                    ),
                    None,
                    None,
                )
                .unwrap();
        }
    }

    #[test]
    fn auto_hide_uses_the_stage_boundary_while_playing_and_paused() {
        use nana_ui_input::PointerPhase::Move;

        for playing in [false, true] {
            let mut stage = AutoHideStage::new(playing);
            stage.pointer(Move, 100.0, 100.0);
            assert!(stage.shown());
            stage.pointer(Move, 800.0, 500.0);
            assert!(
                !stage.shown(),
                "leaving the picture hides without leaving the window"
            );
            assert_eq!(stage.cx.next_animation_deadline(), None);
            stage
                .cx
                .update_component(stage.bar, |bar, _| bar.position = 15.0)
                .unwrap();
            assert!(
                !stage.shown(),
                "playback refresh must not reveal a concealed bar"
            );
            stage.pointer(Move, 100.0, 100.0);
            assert!(
                stage.shown(),
                "the picture stays hittable after its bar hides"
            );
            if playing {
                let deadline = stage.input.now() + OVERLAY_IDLE;
                stage.cx.advance_animations(deadline);
                assert!(!stage.shown(), "the existing idle timeout still applies");
                stage.input.set_now(deadline + Duration::from_millis(10));
                stage.pointer(Move, 100.0, 100.0);
                assert!(
                    stage.shown(),
                    "motion in the picture reveals an idle-hidden bar"
                );
            } else {
                assert_eq!(stage.cx.next_animation_deadline(), None);
            }
        }
    }

    #[test]
    fn auto_hide_uses_the_transformed_stage_position_for_routed_pointers() {
        use nana_ui_core::PaintTransform;
        use nana_ui_input::PointerPhase::Move;

        let mut stage = AutoHideStage::new(true);
        let picture_id = stage.cx.world().parent_id(stage.bar.stable_id()).unwrap();
        let picture = stage.cx.view_entity::<crate::Stack>(picture_id).unwrap();
        stage
            .cx
            .update_component(picture, |picture, _| {
                *picture = picture.clone().with_layout(|layout| {
                    layout.transform = Some(PaintTransform {
                        e: 200.0,
                        f: 100.0,
                        ..PaintTransform::default()
                    });
                });
            })
            .unwrap();
        stage
            .cx
            .layout_document(stage.document, crate::LayoutViewport::new(960.0, 600.0))
            .unwrap();
        stage.cx.compat_world_mut().rebuild_hit_test(stage.document);

        let raw_frame = stage.cx.world().layout_box(picture_id).unwrap();
        assert!(raw_frame.contains(100.0, 200.0));
        assert!(!raw_frame.contains(700.0, 200.0));
        stage.pointer(Move, 100.0, 200.0);
        assert!(
            !stage.shown(),
            "the old layout position is outside the translated picture"
        );
        stage.pointer(Move, 700.0, 200.0);
        assert!(
            stage.shown(),
            "the actual picture reveals its bar beyond the raw layout box"
        );
        stage.pointer(Move, 900.0, 200.0);
        assert!(
            !stage.shown(),
            "crossing the translated right edge conceals the bar"
        );
    }

    #[test]
    fn auto_hide_retains_an_outside_pointer_when_a_menu_closes() {
        use nana_ui_input::PointerPhase::Move;

        let mut stage = AutoHideStage::new(true);
        stage.pointer(Move, 100.0, 100.0);
        let settings = stage
            .cx
            .read(stage.bar, |bar| bar.settings())
            .unwrap()
            .unwrap();
        assert!(stage.cx.toggle_action_menu(settings).unwrap());
        stage.pointer(Move, 800.0, 500.0);
        assert!(
            stage.shown(),
            "the open menu protects the controls outside the picture"
        );
        assert_eq!(stage.cx.overlay_wakeup(stage.bar).unwrap(), None);
        assert!(stage.cx.dismiss_popovers_on_escape().unwrap());
        assert!(
            !stage.shown(),
            "menu close must not restart visibility outside the picture"
        );
        assert_eq!(stage.cx.overlay_wakeup(stage.bar).unwrap(), None);
        stage.pointer(Move, 100.0, 100.0);
        assert!(stage.shown());
    }

    #[test]
    fn auto_hide_uses_actual_drag_coordinates_and_hides_on_outside_release() {
        use nana_ui_input::PointerPhase::{Cancel, Down, Move, Up};

        for end in [Up, Cancel] {
            let mut stage = AutoHideStage::new(true);
            let seek = stage.cx.read(stage.bar, |bar| bar.seek()).unwrap().unwrap();
            let track = match stage.cx.world().component_geometry(seek.stable_id()) {
                Some(crate::ComponentGeometry::Range { track, .. }) => track,
                _ => panic!("seek range geometry expected"),
            };
            let (x, y) = (track.x + track.width / 2.0, track.y + track.height / 2.0);
            stage.pointer(Down, x, y);
            assert!(
                stage
                    .cx
                    .overlay_locks(stage.document, stage.bar.stable_id())
                    .dragging
            );
            stage.pointer(Move, 800.0, 500.0);
            assert!(
                stage.shown(),
                "pointer capture protects the active seek drag"
            );
            stage.pointer(end, 800.0, 500.0);
            assert!(
                !stage
                    .cx
                    .overlay_locks(stage.document, stage.bar.stable_id())
                    .dragging
            );
            assert!(
                !stage.shown(),
                "ending outside hides even though the captured target was inside"
            );
            stage.pointer(Move, 100.0, 100.0);
            assert!(stage.shown());
        }
    }

    #[test]
    fn auto_hide_preserves_keyboard_focus_and_keyboard_reveal() {
        use nana_ui_input::PointerPhase::{Down, Move};

        let mut stage = AutoHideStage::new(true);
        stage.key("Tab");
        assert!(
            stage
                .cx
                .world()
                .focus_visible(stage.document)
                .is_some_and(|focused| stage.cx.is_descendant(focused, stage.bar.stable_id()))
        );
        stage.pointer(Move, 800.0, 500.0);
        assert!(
            stage.shown(),
            "keyboard focus protects the controls while the pointer is away"
        );
        stage.pointer(Down, 800.0, 500.0);
        assert_eq!(stage.cx.world().focused(stage.document), None);
        assert!(
            !stage.shown(),
            "the controls hide once focus no longer protects them"
        );
        stage.key("KeyA");
        assert!(
            stage.shown(),
            "keyboard activity still reveals controls while the pointer is away"
        );
        let deadline = stage.input.now() + OVERLAY_IDLE;
        assert_eq!(stage.cx.next_animation_deadline(), Some(deadline));
        stage.cx.advance_animations(deadline);
        assert!(!stage.shown(), "keyboard reveal retains the idle timeout");
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn stays_visible_when_inactive_or_locked() {
        let mut vis = OverlayVisibility::new(t0());
        let now = t0();
        assert!(!vis.synchronize(now, false, OverlayLocks::default()));
        assert!(vis.visible());
        vis.synchronize(now, true, OverlayLocks::default());
        vis.tick(now + OVERLAY_IDLE);
        assert!(!vis.visible());
        vis.synchronize(
            now + OVERLAY_IDLE,
            true,
            OverlayLocks {
                dragging: true,
                ..OverlayLocks::default()
            },
        );
        assert!(vis.visible());
    }

    #[test]
    fn activity_resets_idle_deadline() {
        let mut vis = OverlayVisibility::new(t0());
        let now = t0();
        vis.synchronize(now, true, OverlayLocks::default());
        vis.tick(now + Duration::from_secs(2));
        assert!(vis.visible());
        vis.activity(now + Duration::from_secs(2));
        vis.tick(now + Duration::from_secs(4));
        assert!(vis.visible());
        vis.tick(now + Duration::from_secs(2) + OVERLAY_IDLE);
        assert!(!vis.visible());
    }

    #[test]
    fn hover_dwell_delays_reveal() {
        let now = t0();
        let mut vis = OverlayVisibility::with_config(
            now,
            OverlayVisibilityConfig {
                idle: OVERLAY_IDLE,
                hover_dwell: Duration::from_millis(150),
                ..OverlayVisibilityConfig::default()
            },
        );
        vis.synchronize(now, true, OverlayLocks::default());
        vis.tick(now + OVERLAY_IDLE);
        assert!(!vis.visible());
        assert!(!vis.set_pointer_inside(now + OVERLAY_IDLE, true));
        assert!(!vis.visible());
        vis.tick(now + OVERLAY_IDLE + Duration::from_millis(150));
        assert!(vis.visible());
    }

    #[test]
    fn conceal_hides_until_activity_whether_playing_or_not_and_waits_for_a_hold() {
        let now = t0();
        let mut vis = OverlayVisibility::new(now);
        vis.synchronize(now, false, OverlayLocks::default());
        assert!(
            vis.conceal(now),
            "paused chrome hides when the pointer leaves"
        );
        vis.synchronize(now, false, OverlayLocks::default());
        assert!(!vis.visible(), "and stays hidden while it is away");
        vis.activity(now);
        assert!(vis.visible());

        let held = OverlayLocks {
            menu_open: true,
            ..OverlayLocks::default()
        };
        vis.synchronize(now, true, held);
        assert!(!vis.conceal(now), "an open menu keeps the chrome");
        assert!(vis.visible());
        vis.synchronize(now, true, OverlayLocks::default());
        assert!(!vis.visible(), "the hold ends with the pointer still away");
    }

    const GRACE: Duration = Duration::from_millis(80);

    /// Chrome that follows the pointer: hidden until it comes, hidden again
    /// a grace after it leaves, never on a clock.
    fn chrome() -> OverlayVisibility {
        OverlayVisibility::hidden(OverlayVisibilityConfig {
            leave_grace: GRACE,
            ..OverlayVisibilityConfig::default()
        })
    }

    fn ms(start: Instant, value: u64) -> Instant {
        start + Duration::from_millis(value)
    }

    fn hold(vis: &mut OverlayVisibility, now: Instant, held: bool) -> bool {
        let locks = OverlayLocks {
            interaction: held,
            ..OverlayLocks::default()
        };
        vis.synchronize(now, false, locks)
    }

    #[test]
    fn pointer_chrome_reveals_and_is_shown_when_the_reveal_lands() {
        let start = t0();
        let mut vis = chrome();
        assert_eq!(vis.phase(), PresencePhase::Hidden);
        assert!(vis.set_pointer_inside(start, true));
        assert_eq!(vis.phase(), PresencePhase::Revealing);
        assert!(!vis.tick(ms(start, 100)));
        assert_eq!(vis.transition_finished(true), Some(PresenceSettled::Shown));
        assert_eq!(vis.phase(), PresencePhase::Shown);
        assert_eq!(vis.wakeup(), None);
    }

    #[test]
    fn pointer_chrome_conceals_after_the_grace_then_settles_hidden() {
        let start = t0();
        let mut vis = chrome();
        vis.set_pointer_inside(start, true);
        vis.transition_finished(true);
        assert!(!vis.conceal(ms(start, 300)));
        assert_eq!(vis.wakeup(), Some(ms(start, 380)));
        assert!(!vis.tick(ms(start, 370)));
        assert_eq!(vis.phase(), PresencePhase::Shown);
        assert!(vis.tick(ms(start, 380)));
        assert_eq!(vis.phase(), PresencePhase::Concealing);
        assert_eq!(vis.wakeup(), None, "the fade end is an event, not a timer");
        assert_eq!(
            vis.transition_finished(false),
            Some(PresenceSettled::Hidden)
        );
        assert_eq!(vis.phase(), PresencePhase::Hidden);
    }

    #[test]
    fn edge_jitter_inside_the_grace_never_conceals_or_moves_the_deadline_later() {
        let start = t0();
        let mut vis = chrome();
        vis.set_pointer_inside(start, true);
        vis.transition_finished(true);
        for step in 0..20 {
            let at = ms(start, 200 + step * 10);
            if step % 2 == 1 {
                assert!(!vis.set_pointer_inside(at, true));
            } else {
                assert!(!vis.conceal(at));
                assert!(!vis.conceal(at), "a second leave keeps the first deadline");
            }
            assert!(vis.wakeup().is_none_or(|due| due <= at + GRACE));
            assert!(!vis.tick(at));
        }
        assert_eq!(vis.phase(), PresencePhase::Shown);
        vis.tick(ms(start, 1_000));
        assert_eq!(vis.phase(), PresencePhase::Shown);
        assert_eq!(vis.wakeup(), None);
    }

    #[test]
    fn pointer_chrome_retargets_both_ways_and_ignores_stale_finishes() {
        let start = t0();
        let mut vis = chrome();
        vis.set_pointer_inside(start, true);
        // Leave while still revealing: the conceal starts from the reveal.
        vis.conceal(ms(start, 40));
        assert!(vis.tick(ms(start, 120)));
        // A reveal finish arriving now belongs to the replaced transition.
        assert_eq!(vis.transition_finished(true), None);
        assert_eq!(vis.phase(), PresencePhase::Concealing);
        // Back halfway through the conceal: the reveal replaces the settle.
        assert!(vis.set_pointer_inside(ms(start, 180), true));
        assert_eq!(vis.transition_finished(false), None);
        assert_eq!(vis.phase(), PresencePhase::Revealing);
        assert_eq!(vis.transition_finished(true), Some(PresenceSettled::Shown));
    }

    #[test]
    fn holds_keep_pointer_chrome_until_released_with_the_pointer_away() {
        let start = t0();
        let mut vis = chrome();
        vis.set_pointer_inside(start, true);
        hold(&mut vis, ms(start, 10), true);
        assert!(!vis.conceal(ms(start, 20)));
        assert_eq!(vis.wakeup(), None);
        assert!(!vis.tick(ms(start, 2_000)));
        assert!(vis.visible());
        assert!(!hold(&mut vis, ms(start, 2_000), false));
        assert!(vis.tick(ms(start, 2_080)));
        // A hold arriving during the conceal brings the chrome back.
        assert!(hold(&mut vis, ms(start, 2_100), true));
        assert_eq!(vis.phase(), PresencePhase::Revealing);
    }

    #[test]
    fn a_hold_released_with_the_pointer_inside_keeps_pointer_chrome() {
        let start = t0();
        let mut vis = chrome();
        vis.set_pointer_inside(start, true);
        hold(&mut vis, start, true);
        hold(&mut vis, ms(start, 500), false);
        assert_eq!(vis.wakeup(), None);
        assert!(!vis.tick(ms(start, 5_000)));
        assert!(vis.visible());
    }

    #[test]
    fn a_hold_does_not_bring_back_chrome_that_settled_hidden() {
        let start = t0();
        let mut vis = chrome();
        assert!(!hold(&mut vis, start, true));
        assert_eq!(vis.phase(), PresencePhase::Hidden);
        hold(&mut vis, ms(start, 10), false);
        vis.set_pointer_inside(ms(start, 20), true);
        vis.transition_finished(true);
        vis.conceal(ms(start, 30));
        vis.tick(ms(start, 110));
        vis.transition_finished(false);
        assert!(!hold(&mut vis, ms(start, 200), true));
        assert!(!vis.visible());
    }

    #[test]
    fn a_reveal_without_the_pointer_hides_again_when_a_hold_ends() {
        let start = t0();
        let mut vis = chrome();
        assert!(vis.reveal(start));
        vis.transition_finished(true);
        assert!(!hold(&mut vis, ms(start, 10), false), "no hold: it stays");
        assert_eq!(vis.wakeup(), None);
        hold(&mut vis, ms(start, 20), true);
        hold(&mut vis, ms(start, 500), false);
        assert_eq!(vis.wakeup(), Some(ms(start, 580)));
        assert!(vis.tick(ms(start, 580)));
        // Pointer activity is the pointer back: a hold ending then keeps it.
        vis.set_pointer_inside(ms(start, 600), true);
        hold(&mut vis, ms(start, 610), true);
        hold(&mut vis, ms(start, 620), false);
        assert_eq!(vis.wakeup(), None);
    }

    #[test]
    fn an_auto_hidden_bar_follows_routed_input_and_the_runtime_clock() {
        use crate::{
            AppContext, DocumentId, HeadlessInput, LayoutViewport, MediaTransportBar,
            MutationQueue, Stack,
        };
        use nana_ui_core::LengthSpec;
        use nana_ui_input::{InputPayload, PointerId, PointerPhase};
        use std::sync::{Arc, Mutex};

        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let window = cx
            .create_component(
                document,
                Stack::column(0.0).hittable().with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(640.0));
                    layout.height = Some(LengthSpec::Px(360.0));
                }),
            )
            .unwrap();
        let bar = cx
            .create_detached_component(document, MediaTransportBar::new().auto_hide(true))
            .unwrap();
        cx.append_child(window, bar).unwrap();
        cx.assemble_media_transport_bar(bar).unwrap();
        cx.layout_document(document, LayoutViewport::new(640.0, 360.0))
            .unwrap();
        cx.compat_world_mut().rebuild_hit_test(document);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let out = Arc::clone(&seen);
        cx.on(bar, move |_, event: &OverlayVisibilityChanged, _| {
            out.lock().unwrap().push(event.visible);
        })
        .unwrap();
        let shown = |cx: &AppContext| cx.read(bar, MediaTransportBar::shown).unwrap();
        let mut input = HeadlessInput::bind(&mut cx, document);

        assert_eq!(cx.next_animation_deadline(), None, "paused chrome stays");
        cx.update_component(bar, |bar, _| bar.playing = true)
            .unwrap();
        assert_eq!(cx.next_animation_deadline(), Some(OVERLAY_IDLE));
        cx.advance_animations(OVERLAY_IDLE);
        assert!(!shown(&cx), "idle while playing");

        input.set_now(Duration::from_secs(10));
        input
            .pointer(&mut cx, PointerPhase::Move, 100.0, 100.0)
            .unwrap();
        assert!(shown(&cx), "the pointer over the picture reveals it");
        assert_eq!(cx.next_animation_deadline(), Some(Duration::from_secs(13)));

        let seek = cx.read(bar, |bar| bar.seek()).unwrap().unwrap();
        let mut capture = MutationQueue::new();
        capture.capture_pointer(1, seek.stable_id());
        cx.commit_mutations(capture).unwrap();
        cx.advance_animations(Duration::from_secs(20));
        assert!(shown(&cx), "a seek drag holds it");
        let mut release = MutationQueue::new();
        release.release_pointer(1, seek.stable_id());
        cx.commit_mutations(release).unwrap();

        input.set_now(Duration::from_secs(21));
        input
            .route(
                &mut cx,
                InputPayload::PointerLeave {
                    pointer_id: PointerId(1),
                },
            )
            .unwrap();
        assert!(!shown(&cx), "leaving the window hides it at once");
        cx.update_component(bar, |bar, _| bar.playing = false)
            .unwrap();
        assert!(!shown(&cx), "pausing does not bring it back while away");

        input.set_now(Duration::from_secs(22));
        input
            .pointer(&mut cx, PointerPhase::Move, 100.0, 100.0)
            .unwrap();
        assert!(shown(&cx));
        cx.advance_animations(Duration::from_secs(60));
        assert!(shown(&cx), "paused chrome stays while the pointer is in");

        cx.update_component(bar, |bar, _| bar.playing = true)
            .unwrap();
        cx.advance_animations(Duration::from_secs(70));
        assert!(!shown(&cx));
        input.set_now(Duration::from_secs(80));
        input
            .press(
                &mut cx,
                nana_ui_input::KeyInput::named(
                    "KeyA",
                    "a",
                    nana_ui_input::KeyState::Pressed,
                    Default::default(),
                ),
                None,
                None,
            )
            .unwrap();
        assert!(shown(&cx), "a key press reveals it");
        assert_eq!(
            cx.next_animation_deadline(),
            Some(Duration::from_secs(83)),
            "idle counts from the key press"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec![false, true, false, true, false, true]
        );
    }

    #[test]
    fn an_auto_hidden_bar_takes_pointer_activity_at_its_deadline_and_follows_its_menus() {
        use crate::{
            AppContext, DocumentId, HeadlessInput, LayoutViewport, MediaTransportBar, Stack,
        };
        use nana_ui_core::LengthSpec;
        use nana_ui_input::PointerPhase;

        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let window = cx
            .create_component(
                document,
                Stack::column(0.0).hittable().with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(640.0));
                    layout.height = Some(LengthSpec::Px(360.0));
                }),
            )
            .unwrap();
        let bar = cx
            .create_detached_component(document, MediaTransportBar::new().auto_hide(true))
            .unwrap();
        cx.append_child(window, bar).unwrap();
        cx.assemble_media_transport_bar(bar).unwrap();
        cx.layout_document(document, LayoutViewport::new(640.0, 360.0))
            .unwrap();
        cx.compat_world_mut().rebuild_hit_test(document);
        let shown = |cx: &AppContext| cx.read(bar, MediaTransportBar::shown).unwrap();
        let mut input = HeadlessInput::bind(&mut cx, document);
        cx.update_component(bar, |bar, _| bar.playing = true)
            .unwrap();

        input.set_now(Duration::from_secs(1));
        input
            .pointer(&mut cx, PointerPhase::Move, 100.0, 100.0)
            .unwrap();
        assert_eq!(
            cx.next_animation_deadline(),
            Some(OVERLAY_IDLE),
            "a move over a bar counting down leaves the bar alone"
        );
        cx.advance_animations(OVERLAY_IDLE);
        assert!(shown(&cx), "the deadline takes the move");
        assert_eq!(cx.next_animation_deadline(), Some(Duration::from_secs(4)));

        let settings = cx.read(bar, |bar| bar.settings()).unwrap().unwrap();
        assert!(cx.toggle_action_menu(settings).unwrap());
        assert!(
            cx.overlay_wakeup(bar).unwrap().is_none(),
            "an open menu holds it"
        );
        cx.advance_animations(Duration::from_secs(10));
        assert!(shown(&cx));
        assert!(cx.dismiss_popovers_on_escape().unwrap());
        cx.advance_animations(Duration::from_secs(12));
        assert!(shown(&cx), "the idle timer restarts when the menu closes");
        cx.advance_animations(Duration::from_secs(13));
        assert!(!shown(&cx));
    }

    #[test]
    fn app_context_hides_the_bar_after_idle_and_holds_for_focus() {
        use crate::{AppContext, DocumentId, LayoutViewport, MediaTransportBar, MutationQueue};

        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let bar = cx
            .create_component(document, MediaTransportBar::new())
            .unwrap();
        cx.assemble_media_transport_bar(bar).unwrap();
        cx.layout_document(document, LayoutViewport::new(640.0, 360.0))
            .unwrap();
        let now = Instant::now();
        let wakeup = cx.sync_overlay_visibility(bar, now, true).unwrap();
        assert_eq!(wakeup, Some(now + OVERLAY_IDLE));
        assert!(!cx.read(bar, |bar| !bar.shown()).unwrap());

        cx.sync_overlay_visibility(bar, now + OVERLAY_IDLE, true)
            .unwrap();
        assert!(cx.read(bar, |bar| !bar.shown()).unwrap());

        let play = cx.read(bar, |bar| bar.play()).unwrap().unwrap();
        assert!(
            cx.reveal_overlay(bar, now + OVERLAY_IDLE)
                .expect("keyboard activity")
        );
        assert!(!cx.read(bar, |bar| !bar.shown()).unwrap());
        assert!(cx.focus_node(document, play.stable_id()).unwrap());
        cx.sync_overlay_visibility(bar, now + OVERLAY_IDLE, true)
            .unwrap();
        assert!(!cx.read(bar, |bar| !bar.shown()).unwrap());
        assert!(cx.overlay_wakeup(bar).unwrap().is_none());

        cx.clear_focus(document).unwrap();
        let mut mutations = MutationQueue::new();
        mutations.capture_pointer(1, play.stable_id());
        cx.commit_mutations(mutations).unwrap();
        cx.sync_overlay_visibility(bar, now + Duration::from_secs(40), true)
            .unwrap();
        assert!(!cx.read(bar, |bar| !bar.shown()).unwrap());

        let mut mutations = MutationQueue::new();
        mutations.release_pointer(1, play.stable_id());
        cx.commit_mutations(mutations).unwrap();
        cx.sync_overlay_visibility(bar, now + Duration::from_secs(40), true)
            .unwrap();
        assert_eq!(
            cx.overlay_wakeup(bar).unwrap(),
            Some(now + Duration::from_secs(43))
        );

        assert!(
            cx.reveal_overlay(bar, now + Duration::from_secs(41))
                .unwrap()
        );
        assert!(!cx.read(bar, |bar| !bar.shown()).unwrap());
        assert_eq!(
            cx.overlay_wakeup(bar).unwrap(),
            Some(now + Duration::from_secs(44)),
            "pointer activity should reset the idle deadline"
        );

        let child = cx
            .create_component(document, crate::Stack::row(0.0))
            .unwrap();
        cx.append_child(bar, child).unwrap();
        assert!(cx.is_descendant(child.stable_id(), bar.stable_id()));
        assert!(!cx.is_descendant(bar.stable_id(), child.stable_id()));
    }

    #[test]
    fn open_action_menu_holds_visibility_without_a_projected_visual() {
        use crate::{AppContext, DocumentId, LayoutViewport, MediaTransportBar};

        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let bar = cx
            .create_component(document, MediaTransportBar::new())
            .unwrap();
        cx.assemble_media_transport_bar(bar).unwrap();
        cx.layout_document(document, LayoutViewport::new(640.0, 360.0))
            .unwrap();
        let now = Instant::now();
        cx.sync_overlay_visibility(bar, now, true).unwrap();
        let settings = cx.read(bar, |bar| bar.settings()).unwrap().unwrap();
        assert!(cx.toggle_action_menu(settings).unwrap());
        cx.sync_overlay_visibility(bar, now + OVERLAY_IDLE, true)
            .unwrap();
        assert!(
            !cx.read(bar, |bar| !bar.shown()).unwrap(),
            "an open ActionMenu must lock the bar from the component open flag"
        );
        assert!(cx.overlay_wakeup(bar).unwrap().is_none());
        assert!(cx.dismiss_popovers_on_escape().unwrap());
        cx.sync_overlay_visibility(bar, now + Duration::from_secs(31), true)
            .unwrap();
        assert_eq!(
            cx.overlay_wakeup(bar).unwrap(),
            Some(now + Duration::from_secs(34))
        );
    }

    #[test]
    fn open_field_options_lock_the_overlay_like_a_menu() {
        use crate::{AppContext, ColorField, DocumentId, Dropdown, SearchDropdown, Select, Stack};

        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        let select = cx
            .create_detached_component(document, Select::new(None::<&str>))
            .unwrap();
        let dropdown = cx
            .create_detached_component(document, Dropdown::single(None::<&str>))
            .unwrap();
        let search = cx
            .create_detached_component(document, SearchDropdown::new(None::<&str>))
            .unwrap();
        let color = cx
            .create_detached_component(document, ColorField::new([1.0; 4]))
            .unwrap();
        cx.append_child(root, select).unwrap();
        cx.append_child(root, dropdown).unwrap();
        cx.append_child(root, search).unwrap();
        cx.append_child(root, color).unwrap();
        let locked = |cx: &AppContext| cx.overlay_locks(document, root.stable_id()).menu_open;
        assert!(!locked(&cx));
        cx.update_component(select, |view, _| view.opened = true)
            .unwrap();
        assert!(locked(&cx));
        cx.update_component(select, |view, _| view.opened = false)
            .unwrap();
        cx.update_component(dropdown, |view, _| view.opened = true)
            .unwrap();
        assert!(locked(&cx));
        cx.update_component(dropdown, |view, _| view.opened = false)
            .unwrap();
        cx.update_component(search, |view, _| view.opened = true)
            .unwrap();
        assert!(locked(&cx));
        cx.update_component(search, |view, _| view.opened = false)
            .unwrap();
        cx.update_component(color, |view, _| view.opened = true)
            .unwrap();
        assert!(locked(&cx));
        cx.update_component(color, |view, _| view.opened = false)
            .unwrap();
        assert!(!locked(&cx));
    }
}
