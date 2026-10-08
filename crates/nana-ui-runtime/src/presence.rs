//! One node that is shown and hidden by fading (and travelling) in and out:
//! a card, a strip of transient chrome, the content of a tool window.
//!
//! ```ignore
//! let mut card = PresenceLifecycle::new(card_entity, Transition::slide(0.0, 12.0, ms(180)));
//! card.set_shown(cx, false, now)?;           // conceal: fades out, input off, focus out
//! // every AnimationFrame of the node's document:
//! if card.animation_events(&frame.events) == Some(PresenceSettled::Hidden) {
//!     hide_native_window();
//! }
//! ```
//!
//! The enter half of the [`Transition`] is how a revealing node starts and the
//! leave half how a concealing one ends, both relative to the node's own
//! opacity and transform (as for rows and branches of a view). Both run on
//! the node's compositor tracks, so logical style is never written; a
//! reversal mid-way retargets from what is shown.
//!
//! While concealing and once hidden the node takes no pointer input, and
//! focus inside it is dropped as concealing begins, so keys cannot reach a
//! fading control. Once hidden it also leaves paint, hit testing and the
//! accessibility tree (`visibility: hidden`, which keeps its layout); a
//! reveal puts it back first.
//!
//! The settled state comes from the animation events of its own runs: every
//! run ends with exactly one `Finished` or `Cancelled`, so when the last run
//! it started has reported, the node is at rest — no deadline fallback of the
//! consumer's own. Under [`AppContext::reduced_motion`] the runs take no time
//! and travel nowhere, and still report. Pass every [`crate::AnimationFrame`]
//! of the node's document to [`PresenceLifecycle::animation_events`]; a node
//! that lives in its own window is no different, since the host advances a
//! window's animations whether or not it presents.

use std::time::Duration;

use nana_ui_core::{PaintTransform, PointerEventsSpec, VisibilitySpec};

use crate::view::{StyledComponent, Transition};
use crate::{
    AnimatableProperty, AnimationEvent, AnimationEventKind, AnimationId, AppContext, ComponentView,
    Entity, FrameworkError, MotionValue, MutationQueue, StableNodeId,
};

/// Where a [`PresenceLifecycle`] is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresencePhase {
    Hidden,
    Revealing,
    Shown,
    Concealing,
}

/// A [`PresenceLifecycle`] came to rest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresenceSettled {
    Shown,
    Hidden,
}

type StyleWriter = fn(
    &mut AppContext,
    StableNodeId,
    &mut dyn FnMut(&mut nana_ui_core::LayoutStyle),
) -> Result<(), FrameworkError>;

/// Shows and hides one node; see the module docs.
pub struct PresenceLifecycle {
    node: StableNodeId,
    transition: Transition,
    phase: PresencePhase,
    /// Ends still to be reported for the runs this lifecycle started.
    outstanding: u32,
    /// The pointer-events the node had before it first concealed.
    pointer_events: Option<Option<PointerEventsSpec>>,
    write: StyleWriter,
}

fn write_style<C: StyledComponent + ComponentView>(
    cx: &mut AppContext,
    node: StableNodeId,
    edit: &mut dyn FnMut(&mut nana_ui_core::LayoutStyle),
) -> Result<(), FrameworkError> {
    cx.update_component(Entity::<C>::from_stable_id(node), |component, _| {
        let layout = &mut component.node_style_mut().layout;
        let mut next = (**layout).clone();
        edit(&mut next);
        if next != **layout {
            *layout = std::sync::Arc::new(next);
        }
    })
}

impl PresenceLifecycle {
    /// A lifecycle for `node`, which is shown now (its own opacity and
    /// transform, nothing applied). [`Self::set_shown_at_once`] starts it
    /// hidden instead.
    pub fn new<C: StyledComponent + ComponentView>(
        node: Entity<C>,
        transition: Transition,
    ) -> Self {
        Self {
            node: node.stable_id(),
            transition,
            phase: PresencePhase::Shown,
            outstanding: 0,
            pointer_events: None,
            write: write_style::<C>,
        }
    }

    pub fn node(&self) -> StableNodeId {
        self.node
    }

    pub fn phase(&self) -> PresencePhase {
        self.phase
    }

    /// The logical target: shown, or on its way there.
    pub fn shown(&self) -> bool {
        matches!(self.phase, PresencePhase::Shown | PresencePhase::Revealing)
    }

    /// At rest, and which way.
    pub fn settled(&self) -> Option<PresenceSettled> {
        match self.phase {
            PresencePhase::Shown => Some(PresenceSettled::Shown),
            PresencePhase::Hidden => Some(PresenceSettled::Hidden),
            PresencePhase::Revealing | PresencePhase::Concealing => None,
        }
    }

    /// Reveal or conceal from wherever the node is, over the transition's
    /// durations. Returns whether the target changed.
    pub fn set_shown(
        &mut self,
        cx: &mut AppContext,
        shown: bool,
        now: Duration,
    ) -> Result<bool, FrameworkError> {
        if self.shown() == shown {
            return Ok(false);
        }
        self.start(cx, shown, now, false)?;
        Ok(true)
    }

    /// Land at `shown` with no animation (a node that mounts hidden, a jump
    /// the user should not watch). It still settles through
    /// [`Self::animation_events`], with the next advance.
    pub fn set_shown_at_once(
        &mut self,
        cx: &mut AppContext,
        shown: bool,
        now: Duration,
    ) -> Result<(), FrameworkError> {
        self.start(cx, shown, now, true)
    }

    /// Follow the node's animation events; `Some` when they bring it to
    /// rest. Pass every frame of its document.
    pub fn animation_events(&mut self, events: &[AnimationEvent]) -> Option<PresenceSettled> {
        let id = opacity_id(self.node)?;
        for event in events {
            if event.id == id
                && event.target == self.node
                && matches!(
                    event.kind,
                    AnimationEventKind::Finished | AnimationEventKind::Cancelled
                )
            {
                self.outstanding = self.outstanding.saturating_sub(1);
            }
        }
        if self.outstanding > 0 {
            return None;
        }
        match self.phase {
            PresencePhase::Revealing => {
                self.phase = PresencePhase::Shown;
                Some(PresenceSettled::Shown)
            }
            PresencePhase::Concealing => {
                self.phase = PresencePhase::Hidden;
                Some(PresenceSettled::Hidden)
            }
            PresencePhase::Shown | PresencePhase::Hidden => None,
        }
    }

    /// Settle at once in the state the node is heading for, painting it
    /// accordingly (a hidden node leaves paint). For a host that drops the
    /// node's events, e.g. while tearing its window down.
    pub fn settle(
        &mut self,
        cx: &mut AppContext,
    ) -> Result<Option<PresenceSettled>, FrameworkError> {
        self.outstanding = 0;
        let settled = self.animation_events(&[]);
        if settled == Some(PresenceSettled::Hidden) {
            self.set_painted(cx, false)?;
        }
        Ok(settled)
    }

    /// After [`Self::animation_events`] reported [`PresenceSettled::Hidden`]:
    /// take the node out of paint, hit testing and accessibility. Called for
    /// you by [`Self::apply_events`].
    fn set_painted(&self, cx: &mut AppContext, painted: bool) -> Result<(), FrameworkError> {
        let visibility = Some(if painted {
            VisibilitySpec::Visible
        } else {
            VisibilitySpec::Hidden
        });
        (self.write)(cx, self.node, &mut |layout| {
            layout.paint.visibility = visibility;
        })
    }

    /// [`Self::animation_events`], also taking a node that came to rest
    /// hidden out of paint. Prefer it whenever an [`AppContext`] is at hand.
    pub fn apply_events(
        &mut self,
        cx: &mut AppContext,
        events: &[AnimationEvent],
    ) -> Result<Option<PresenceSettled>, FrameworkError> {
        let settled = self.animation_events(events);
        if settled == Some(PresenceSettled::Hidden) {
            self.set_painted(cx, false)?;
        }
        Ok(settled)
    }

    fn start(
        &mut self,
        cx: &mut AppContext,
        shown: bool,
        now: Duration,
        at_once: bool,
    ) -> Result<(), FrameworkError> {
        if !cx.world().contains(self.node) {
            return Err(FrameworkError::MissingView(self.node));
        }
        let reduced = cx.reduced_motion();
        let half = if shown {
            self.transition.enter
        } else {
            self.transition.leave
        };
        let duration = match half {
            Some(half) if !at_once && !reduced => half.duration,
            _ => Duration::ZERO,
        };
        // Without the half nothing plays, so its curve does not matter; an
        // unset one is the theme's standard easing.
        let easing = cx.transition_easing(half.and_then(|half| half.easing));
        let transform_easing = half
            .and_then(|half| half.transform_easing)
            .unwrap_or(easing);
        let own_opacity = match cx
            .world()
            .logical_motion_value(self.node, AnimatableProperty::Opacity)
        {
            Some(MotionValue::Scalar(opacity)) => opacity,
            _ => 1.0,
        };
        let own_transform = match cx
            .world()
            .logical_motion_value(self.node, AnimatableProperty::Transform)
        {
            Some(MotionValue::Transform(transform)) => transform,
            _ => PaintTransform::default(),
        };
        // Shown is the node's own values; hidden is where the leave ends.
        let (opacity, transform) = if shown {
            (own_opacity, Some(own_transform))
        } else {
            let leave = self.transition.leave;
            (
                leave.and_then(|leave| leave.opacity).unwrap_or(0.0),
                // Reduced motion travels nowhere.
                leave
                    .and_then(|leave| leave.transform)
                    .filter(|_| !reduced)
                    .or(Some(own_transform)),
            )
        };
        let travels = self
            .transition
            .enter
            .and_then(|half| half.transform)
            .is_some()
            || self
                .transition
                .leave
                .and_then(|half| half.transform)
                .is_some();

        if shown {
            self.set_painted(cx, true)?;
            let restore = self.pointer_events.take().unwrap_or(None);
            (self.write)(cx, self.node, &mut |layout| layout.pointer_events = restore)?;
        } else {
            let mut before = None;
            (self.write)(cx, self.node, &mut |layout| {
                before = Some(layout.pointer_events);
                layout.pointer_events = Some(PointerEventsSpec::None);
            })?;
            if self.pointer_events.is_none() {
                self.pointer_events = before;
            }
            self.drop_focus(cx)?;
        }

        let mut queue = MutationQueue::new();
        // A reveal from rest starts where the enter half says; anything else
        // (a reversal mid-way, a conceal) starts from what is shown.
        let enter_from = (shown && self.phase == PresencePhase::Hidden)
            .then_some(self.transition.enter)
            .flatten();
        let mut run = |property: AnimatableProperty, from: Option<MotionValue>, to: MotionValue| {
            let easing = if property == AnimatableProperty::Transform {
                transform_easing
            } else {
                easing
            };
            match from {
                Some(from) => queue.start_animation(crate::motion_api::presence_spec(
                    self.node,
                    property,
                    from,
                    to,
                    now,
                    duration,
                    easing,
                    crate::AnimationFillMode::Forwards,
                )),
                None => {
                    let builder = queue.node(self.node, now).transition();
                    let builder = match to {
                        MotionValue::Transform(transform) => builder.transform(transform),
                        MotionValue::Scalar(opacity) => builder.opacity(opacity),
                        _ => builder,
                    };
                    builder.duration(duration).ease(easing).start();
                }
            }
        };
        run(
            AnimatableProperty::Opacity,
            enter_from
                .and_then(|enter| enter.opacity)
                .map(MotionValue::Scalar),
            MotionValue::Scalar(opacity),
        );
        if travels && let Some(transform) = transform {
            run(
                AnimatableProperty::Transform,
                enter_from
                    .and_then(|enter| enter.transform)
                    .filter(|_| !reduced)
                    .map(MotionValue::Transform),
                MotionValue::Transform(transform),
            );
        }
        cx.commit_mutations(queue)?;
        self.outstanding = self.outstanding.saturating_add(1);
        self.phase = if shown {
            PresencePhase::Revealing
        } else {
            PresencePhase::Concealing
        };
        Ok(())
    }

    fn drop_focus(&self, cx: &mut AppContext) -> Result<(), FrameworkError> {
        let Some(document) = cx.world().document_of(self.node) else {
            return Ok(());
        };
        if cx
            .world()
            .focused(document)
            .is_some_and(|focused| cx.world().is_descendant_or_self(focused, self.node))
        {
            cx.clear_focus(document)?;
        }
        Ok(())
    }
}

fn opacity_id(node: StableNodeId) -> Option<AnimationId> {
    crate::motion_api::user_animation_id(node, AnimatableProperty::Opacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Presence;
    use crate::{Button, DocumentId, Easing, Stack};

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn card_with_button() -> (AppContext, DocumentId, Entity<Stack>, Entity<Button>) {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        let card = cx.create_component(document, Stack::column(0.0)).unwrap();
        let button = cx.create_component(document, Button::new("Close")).unwrap();
        cx.append_child(root, card).unwrap();
        cx.append_child(card, button).unwrap();
        (cx, document, card, button)
    }

    fn slide() -> Transition {
        Transition::new()
            .enter(
                Presence::new(ms(200))
                    .opacity(0.0)
                    .translate(0.0, 12.0)
                    .ease_transform(Easing::CubicBezier([0.22, 1.3, 0.36, 1.0])),
            )
            .leave(Presence::new(ms(100)).opacity(0.0).translate(0.0, 12.0))
    }

    /// Advance the way the host does: at each deadline, until `until`.
    fn run(
        cx: &mut AppContext,
        card: &mut PresenceLifecycle,
        until: Duration,
    ) -> Vec<(Duration, PresenceSettled)> {
        let mut settled = Vec::new();
        while let Some(due) = cx.next_animation_deadline() {
            if due > until {
                break;
            }
            let at = due.max(cx.world().animation_now());
            let frame = cx.advance_animations(at);
            if let Some(state) = card.apply_events(cx, &frame.events).unwrap() {
                settled.push((at, state));
            }
        }
        settled
    }

    fn opacity(cx: &AppContext, node: StableNodeId, at: Duration) -> f32 {
        match cx
            .world()
            .presentation_motion_value(node, AnimatableProperty::Opacity, at)
        {
            Some(MotionValue::Scalar(value)) => value,
            other => panic!("{other:?}"),
        }
    }

    fn layout(cx: &AppContext, node: StableNodeId) -> std::sync::Arc<nana_ui_core::LayoutStyle> {
        cx.world().node_style(node).unwrap().layout.clone()
    }

    #[test]
    fn concealing_turns_input_and_focus_off_at_once_and_settles_hidden_from_its_events() {
        let (mut cx, document, card, button) = card_with_button();
        cx.advance_animations(ms(1000));
        cx.focus_node(document, button.stable_id()).unwrap();
        let mut presence = PresenceLifecycle::new(card, slide());

        assert!(presence.set_shown(&mut cx, false, ms(1000)).unwrap());
        assert_eq!(presence.phase(), PresencePhase::Concealing);
        assert_eq!(
            layout(&cx, card.stable_id()).pointer_events,
            Some(PointerEventsSpec::None)
        );
        assert_eq!(
            cx.world().focused(document),
            None,
            "no key reaches a fading control"
        );
        let fading = opacity(&cx, card.stable_id(), ms(1050));
        assert!(fading > 0.0 && fading < 1.0, "{fading}");

        let settled = run(&mut cx, &mut presence, ms(2000));
        assert_eq!(settled, vec![(ms(1100), PresenceSettled::Hidden)]);
        assert_eq!(
            layout(&cx, card.stable_id()).paint.visibility,
            Some(VisibilitySpec::Hidden),
            "out of paint, hit testing and accessibility"
        );

        assert!(presence.set_shown(&mut cx, true, ms(2000)).unwrap());
        assert_eq!(layout(&cx, card.stable_id()).pointer_events, None);
        assert_eq!(
            layout(&cx, card.stable_id()).paint.visibility,
            Some(VisibilitySpec::Visible)
        );
        assert_eq!(
            opacity(&cx, card.stable_id(), ms(2000)),
            0.0,
            "enters from the enter look"
        );
        let settled = run(&mut cx, &mut presence, ms(3000));
        assert_eq!(settled, vec![(ms(2200), PresenceSettled::Shown)]);
        assert_eq!(opacity(&cx, card.stable_id(), ms(2300)), 1.0);
    }

    #[test]
    fn a_reversal_mid_way_settles_only_at_the_end_of_the_last_run() {
        let (mut cx, _, card, _) = card_with_button();
        cx.advance_animations(ms(0));
        let mut presence = PresenceLifecycle::new(card, slide());
        presence.set_shown(&mut cx, false, ms(0)).unwrap();
        assert!(run(&mut cx, &mut presence, ms(50)).is_empty());
        let halfway = opacity(&cx, card.stable_id(), ms(50));

        presence.set_shown(&mut cx, true, ms(50)).unwrap();
        assert_eq!(
            opacity(&cx, card.stable_id(), ms(50)),
            halfway,
            "from what is shown"
        );
        // The cut conceal's Cancelled does not settle anything.
        let settled = run(&mut cx, &mut presence, ms(1000));
        assert_eq!(settled, vec![(ms(250), PresenceSettled::Shown)]);
        assert_eq!(presence.settled(), Some(PresenceSettled::Shown));
    }

    #[test]
    fn a_card_in_its_own_window_settles_from_deadlines_alone() {
        // A tool window's document: nothing lays it out or paints it, the host
        // only advances it at its deadlines (as it does while the window is
        // hidden or not presenting).
        let (mut cx, _, card, _) = card_with_button();
        let mut presence = PresenceLifecycle::new(card, slide());
        presence.set_shown_at_once(&mut cx, false, ms(0)).unwrap();
        assert_eq!(
            run(&mut cx, &mut presence, ms(0)),
            vec![(ms(0), PresenceSettled::Hidden)]
        );
        presence.set_shown(&mut cx, true, ms(500)).unwrap();
        presence.set_shown(&mut cx, false, ms(600)).unwrap();
        let settled = run(&mut cx, &mut presence, ms(5000));
        assert_eq!(settled, vec![(ms(700), PresenceSettled::Hidden)]);
    }

    #[test]
    fn reduced_motion_lands_at_once_without_travel_and_still_settles() {
        let (mut cx, _, card, _) = card_with_button();
        cx.advance_animations(ms(100));
        cx.set_reduced_motion(true);
        let mut presence = PresenceLifecycle::new(card, slide());
        presence.set_shown(&mut cx, false, ms(100)).unwrap();
        assert_eq!(opacity(&cx, card.stable_id(), ms(100)), 0.0);
        assert_eq!(
            cx.world().presentation_motion_value(
                card.stable_id(),
                AnimatableProperty::Transform,
                ms(100)
            ),
            Some(MotionValue::Transform(PaintTransform::default())),
            "no travel"
        );
        assert_eq!(
            run(&mut cx, &mut presence, ms(100)),
            vec![(ms(100), PresenceSettled::Hidden)]
        );
    }
}
