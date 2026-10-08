//! Time-based overlay show/hide policy. Hosts supply clocks and lock flags
//! through [`crate::AppContext::sync_overlay_visibility`], or let the runtime
//! drive a bar's policy from routed input and its own clock
//! ([`crate::MediaTransportBar::auto_hide`]). The policy does not own windows
//! or media, and is not a leaf control.

use std::time::{Duration, Instant};

/// Idle auto-hide after this long while the overlay is not locked.
pub const OVERLAY_IDLE: Duration = Duration::from_secs(3);

/// Pointer / menu / drag locks that keep chrome visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayLocks {
    pub focused: bool,
    pub dragging: bool,
    pub menu_open: bool,
}

impl OverlayLocks {
    pub fn held(self) -> bool {
        self.focused || self.dragging || self.menu_open
    }
}

/// Timing knobs. Zero hover dwell reveals immediately; zero startup skips the
/// initial forced-visible window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayVisibilityConfig {
    pub idle: Duration,
    pub hover_dwell: Duration,
    pub startup: Duration,
}

impl Default for OverlayVisibilityConfig {
    fn default() -> Self {
        Self {
            idle: OVERLAY_IDLE,
            hover_dwell: Duration::ZERO,
            startup: Duration::ZERO,
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

/// Auto-hide machine for media chrome and stage HUDs. Not a leaf control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayVisibility {
    config: OverlayVisibilityConfig,
    visible: bool,
    active: bool,
    held: bool,
    /// Hidden by [`Self::conceal`] until the next activity.
    concealed: bool,
    deadline: Option<Instant>,
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
            visible: true,
            active: false,
            held: false,
            concealed: false,
            deadline: None,
            startup_until,
            hot_since: None,
        }
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Up, not concealed, and counting down to an idle hide: activity now
    /// would only move that deadline later.
    pub(crate) fn counting_down(&self) -> bool {
        self.visible && !self.concealed && self.deadline.is_some()
    }

    pub fn wakeup(&self) -> Option<Instant> {
        let mut next = self.deadline;
        if let Some(startup) = self.startup_until {
            next = Some(next.map_or(startup, |deadline| deadline.min(startup)));
        }
        if let Some(hot) = self.hot_since {
            let dwell = hot + self.config.hover_dwell;
            next = Some(next.map_or(dwell, |deadline| deadline.min(dwell)));
        }
        next
    }

    /// `active` means the surface is in a mode that may hide chrome (playing
    /// media, idle stage). Loading / paused / empty pass false and stay up.
    pub fn synchronize(&mut self, now: Instant, active: bool, locks: OverlayLocks) -> bool {
        let old = self.visible;
        let held = locks.held();
        if self.startup_until.is_some_and(|until| now >= until) {
            self.startup_until = None;
        }
        if held || (!self.concealed && (!active || self.startup_until.is_some())) {
            self.visible = true;
            self.deadline = None;
        } else if self.concealed {
            self.visible = false;
            self.deadline = None;
        } else if !self.active || self.held || (self.visible && self.deadline.is_none()) {
            self.deadline = Some(now + self.config.idle);
        }
        self.active = active;
        self.held = held;
        old != self.visible
    }

    /// Immediate reveal (pointer / keyboard activity).
    pub fn activity(&mut self, now: Instant) -> bool {
        let changed = !self.visible;
        self.concealed = false;
        self.visible = true;
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

    /// Hide now, playing or not, until the next [`Self::activity`]: the
    /// pointer left the window. A held overlay (focus, drag, open menu) stays
    /// until the hold ends, then hides.
    pub fn conceal(&mut self) -> bool {
        self.concealed = true;
        self.hot_since = None;
        if self.held {
            return false;
        }
        self.deadline = None;
        let changed = self.visible;
        self.visible = false;
        changed
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
        if self.deadline.is_some_and(|deadline| deadline <= now) {
            self.deadline = None;
            let changed = self.visible;
            self.visible = false;
            return changed;
        }
        false
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
                || open::<crate::ColorField>(self, id, |field| field.opened)
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
                bar.visibility.conceal();
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
    /// See [`OverlayVisibility::conceal`].
    pub fn conceal_overlay(
        &mut self,
        bar: crate::Entity<crate::MediaTransportBar>,
    ) -> Result<bool, crate::FrameworkError> {
        self.update_component(bar, |bar, cx| {
            let shown = bar.visibility.visible();
            let changed = bar.visibility.conceal();
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
/// longer `before`.
pub(crate) fn report_visibility(
    before: bool,
    bar: &crate::MediaTransportBar,
    cx: &mut crate::ViewContext<'_, crate::MediaTransportBar>,
) {
    let visible = bar.visibility.visible();
    if visible != before {
        cx.emit(OverlayVisibilityChanged { visible });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                startup: Duration::ZERO,
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
        assert!(vis.conceal(), "paused chrome hides when the pointer leaves");
        vis.synchronize(now, false, OverlayLocks::default());
        assert!(!vis.visible(), "and stays hidden while it is away");
        vis.activity(now);
        assert!(vis.visible());

        let held = OverlayLocks {
            menu_open: true,
            ..OverlayLocks::default()
        };
        vis.synchronize(now, true, held);
        assert!(!vis.conceal(), "an open menu keeps the chrome");
        assert!(vis.visible());
        vis.synchronize(now, true, OverlayLocks::default());
        assert!(!vis.visible(), "the hold ends with the pointer still away");
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
