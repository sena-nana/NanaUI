//! Touch scrolling. A finger that travels past the slop scrolls the nearest
//! container that can still move on the gesture's axis; lifting hands the
//! measured velocity to the platform's fling and, past an edge, a bounce.
//! ScrollOffset remains the effective position; the gesture and the fling are
//! transient input state advanced by the animation deadline, never a second
//! layout or transform.

use super::*;
use nana_ui_core::motion::{SpringParams, damped_harmonic};
use nana_ui_input::{InputDisposition, PointerInput, PointerPhase};

/// How far (logical px) a finger travels before a press becomes a scroll.
pub(crate) const TOUCH_SLOP: f32 = 8.0;
/// The release velocity is fitted over the finger's last 100 ms.
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
const INTERVAL: Duration = Duration::from_millis(8);
/// Closer than this to rest (logical px) a fling or bounce ends exactly there.
const REST_DISTANCE: f32 = 0.25;
/// px/s; a bounce slower than this at its rest distance is done.
const REST_VELOCITY: f32 = 10.0;

const CONSUMED: InputDisposition = InputDisposition {
    handled: true,
    prevent_default: true,
};

/// Which platform's touch scrolling the runtime imitates. The host sets it
/// with [`AppContext::set_scroll_physics`]; the default is [`Self::Ios`].
/// Wheel, scrollbar and programmatic scrolling are the same under both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScrollPhysics {
    /// `UIScrollView`: the release velocity decays by 0.998 per millisecond,
    /// a drag past an edge is rubber-banded, and the container springs back
    /// to the edge, critically damped, once the finger lifts.
    #[default]
    Ios,
    /// AOSP `OverScroller`: the fling follows `SplineOverScroller`'s spline
    /// and stops at the edge. Neither a drag nor a fling goes past an edge.
    Android,
}

/// iOS deceleration, rubber band and bounce, in logical px and seconds.
pub(crate) mod ios {
    use super::*;

    /// Velocity kept per millisecond (`UIScrollView.DecelerationRate.normal`).
    pub(crate) const DECELERATION_RATE: f32 = 0.998;
    /// The rubber band's stiffness coefficient.
    pub(crate) const RUBBER_BAND: f32 = 0.55;
    /// The bounce back to an edge: critically damped, settling in ~0.5 s.
    pub(crate) const BOUNCE: SpringParams = SpringParams::new(250.0, 31.622_776, 1.0);

    /// Displacement and velocity `t` seconds into a fling released at
    /// `velocity` px/s: `v·(r^ms − 1)/ln r` with `v` in px/ms.
    pub(crate) fn decay(velocity: f32, t: f32) -> (f32, f32) {
        let ms = t.max(0.0) * 1_000.0;
        let keep = DECELERATION_RATE.powf(ms);
        let distance = velocity / 1_000.0 * (keep - 1.0) / DECELERATION_RATE.ln();
        (distance, velocity * keep)
    }

    /// Where a fling at `velocity` px/s comes to rest.
    pub(crate) fn decay_distance(velocity: f32) -> f32 {
        -velocity / 1_000.0 / DECELERATION_RATE.ln()
    }

    /// Seconds until a fling from `velocity` covers `distance` (same sign),
    /// or `None` when it comes to rest first.
    pub(crate) fn decay_time_to(velocity: f32, distance: f32) -> Option<f32> {
        let keep = 1.0 + distance * DECELERATION_RATE.ln() * 1_000.0 / velocity;
        (keep > 0.0 && keep <= 1.0).then(|| keep.ln() / DECELERATION_RATE.ln() / 1_000.0)
    }

    /// How far content shows past an edge when the finger has dragged it
    /// `overscroll` past, in a viewport `dimension` long:
    /// `(1 − 1/(x·0.55/d + 1))·d`. Never reaches `d`.
    pub(crate) fn rubber_band(overscroll: f32, dimension: f32) -> f32 {
        if dimension <= 0.0 {
            return 0.0;
        }
        let x = overscroll.abs();
        (1.0 - 1.0 / (x * RUBBER_BAND / dimension + 1.0)) * dimension * overscroll.signum()
    }

    /// The finger travel [`rubber_band`] shows as `shown`, so a container
    /// caught mid-bounce continues from where it is drawn.
    pub(crate) fn rubber_band_inverse(shown: f32, dimension: f32) -> f32 {
        if dimension <= 0.0 {
            return 0.0;
        }
        let ratio = (shown.abs() / dimension).min(0.999);
        dimension / RUBBER_BAND * (1.0 / (1.0 - ratio) - 1.0) * shown.signum()
    }

    /// Displacement from the edge and velocity `t` seconds into a bounce.
    pub(crate) fn bounce(from: f32, velocity: f32, t: f32) -> (f32, f32) {
        damped_harmonic(from, velocity, BOUNCE, t)
    }
}

/// AOSP `SplineOverScroller`, in logical px (dp) and seconds.
pub(crate) mod android {
    use std::sync::OnceLock;

    /// `ViewConfiguration.getScrollFriction()`.
    pub(crate) const FRICTION: f32 = 0.015;
    pub(crate) const INFLEXION: f32 = 0.35;
    const START_TENSION: f32 = 0.5;
    const END_TENSION: f32 = 1.0;
    const P1: f32 = START_TENSION * INFLEXION;
    const P2: f32 = 1.0 - END_TENSION * (1.0 - INFLEXION);
    pub(crate) const SAMPLES: usize = 100;
    /// `ViewConfiguration` minimum and maximum fling velocity, dp/s.
    pub(crate) const MIN_FLING_VELOCITY: f32 = 50.0;
    pub(crate) const MAX_FLING_VELOCITY: f32 = 8_000.0;

    /// `ln(0.78) / ln(0.9)`.
    pub(crate) fn deceleration_rate() -> f64 {
        0.78f64.ln() / 0.9f64.ln()
    }

    /// `GRAVITY_EARTH · 39.37 in/m · ppi · 0.84`, with 160 ppi per dp.
    pub(crate) fn physical_coefficient() -> f64 {
        9.806_65 * 39.37 * 160.0 * 0.84
    }

    fn spline_deceleration(velocity: f32) -> f64 {
        (f64::from(INFLEXION) * f64::from(velocity.abs())
            / (f64::from(FRICTION) * physical_coefficient()))
        .ln()
    }

    /// `getSplineFlingDuration`, in seconds.
    pub(crate) fn fling_duration(velocity: f32) -> f32 {
        let l = spline_deceleration(velocity);
        (l / (deceleration_rate() - 1.0)).exp() as f32
    }

    /// `getSplineFlingDistance`, unsigned.
    pub(crate) fn fling_distance(velocity: f32) -> f32 {
        let l = spline_deceleration(velocity);
        let rate = deceleration_rate();
        (f64::from(FRICTION) * physical_coefficient() * (rate / (rate - 1.0) * l).exp()) as f32
    }

    /// `SPLINE_POSITION`: the share of the distance covered at each
    /// hundredth of the duration, built as AOSP builds it.
    pub(crate) fn spline_position() -> &'static [f32; SAMPLES + 1] {
        static TABLE: OnceLock<[f32; SAMPLES + 1]> = OnceLock::new();
        TABLE.get_or_init(|| {
            let mut table = [0.0; SAMPLES + 1];
            let mut x_min = 0.0f32;
            for (i, slot) in table.iter_mut().take(SAMPLES).enumerate() {
                let alpha = i as f32 / SAMPLES as f32;
                let mut x_max = 1.0f32;
                let (mut x, mut coef);
                loop {
                    x = x_min + (x_max - x_min) / 2.0;
                    coef = 3.0 * x * (1.0 - x);
                    let tx = coef * ((1.0 - x) * P1 + x * P2) + x * x * x;
                    if (tx - alpha).abs() < 1e-5 {
                        break;
                    }
                    if tx > alpha {
                        x_max = x;
                    } else {
                        x_min = x;
                    }
                }
                *slot = coef * ((1.0 - x) * START_TENSION + x) + x * x * x;
            }
            table[SAMPLES] = 1.0;
            table
        })
    }

    /// The share of the distance covered and the velocity coefficient at
    /// `t` (a share of the duration), as `SplineOverScroller.update` reads it.
    pub(crate) fn spline(t: f32) -> (f32, f32) {
        let table = spline_position();
        let index = (SAMPLES as f32 * t.max(0.0)) as usize;
        if index >= SAMPLES {
            return (1.0, 0.0);
        }
        let t_inf = index as f32 / SAMPLES as f32;
        let t_sup = (index + 1) as f32 / SAMPLES as f32;
        let (d_inf, d_sup) = (table[index], table[index + 1]);
        let velocity = (d_sup - d_inf) / (t_sup - t_inf);
        (d_inf + (t - t_inf) * velocity, velocity)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    fn of(offset: ScrollOffset, axis: Axis) -> f32 {
        match axis {
            Axis::Horizontal => offset.x,
            Axis::Vertical => offset.y,
        }
    }

    fn with(self, offset: ScrollOffset, value: f32) -> ScrollOffset {
        match self {
            Axis::Horizontal => ScrollOffset { x: value, ..offset },
            Axis::Vertical => ScrollOffset { y: value, ..offset },
        }
    }

    fn pick(self, x: f32, y: f32) -> f32 {
        match self {
            Axis::Horizontal => x,
            Axis::Vertical => y,
        }
    }
}

/// One container a touch drag holds.
#[derive(Debug, Clone, Copy)]
struct TouchDrag {
    target: StableNodeId,
    axis: Axis,
    /// The finger position on the axis the drag measures from, and the
    /// unbounded offset it stood for: past an edge, the finger travel before
    /// the rubber band.
    anchor: f32,
    anchor_offset: f32,
}

/// One finger from press to lift.
#[derive(Debug, Clone)]
pub(super) struct TouchGesture {
    origin: (f32, f32),
    /// Where the press landed, as the ordinary pointer path resolved it.
    hit: Option<StableNodeId>,
    /// The press stopped this container's fling, so it activates nothing.
    caught: Option<StableNodeId>,
    drag: Option<TouchDrag>,
    /// Past slop with nothing to scroll, or a control kept the press.
    declined: bool,
    samples: Vec<(Duration, f32, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Motion {
    /// iOS deceleration from `from` at `velocity` px/s.
    Decay { from: f32, velocity: f32 },
    /// AOSP spline from `from` over `distance` in `duration` seconds.
    Spline {
        from: f32,
        distance: f32,
        duration: f32,
    },
    /// iOS spring back to `edge` from `displacement` at `velocity` px/s.
    Bounce {
        edge: f32,
        displacement: f32,
        velocity: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TouchFling {
    axis: Axis,
    motion: Motion,
    started: Duration,
    next: Duration,
    /// What the fling last wrote; any other position means something else
    /// moved the container, which ends the fling.
    last: ScrollOffset,
}

#[derive(Debug, Default)]
pub(super) struct TouchScrolls {
    physics: ScrollPhysics,
    gestures: HashMap<(DocumentId, u64), TouchGesture, crate::BuildIdHasher>,
    flings: HashMap<StableNodeId, TouchFling, crate::BuildIdHasher>,
    /// Reused across samples; callbacks may end flings while we walk.
    due: Vec<StableNodeId>,
}

impl AppContext {
    /// Choose the touch scrolling the runtime imitates. Flings already
    /// running finish under the physics they started with.
    pub fn set_scroll_physics(&mut self, physics: ScrollPhysics) {
        self.component_lifecycle.touch_scroll.physics = physics;
    }

    pub fn scroll_physics(&self) -> ScrollPhysics {
        self.component_lifecycle.touch_scroll.physics
    }

    pub(super) fn touch_scroll_deadline(&self) -> Option<Duration> {
        self.component_lifecycle
            .touch_scroll
            .flings
            .iter()
            .filter(|(id, _)| self.touch_scroll_live(**id))
            .map(|(_, fling)| fling.next)
            .min()
    }

    /// Stop a fling or bounce where it is drawn; the next write wins.
    pub(super) fn stop_touch_fling(&mut self, id: StableNodeId) {
        self.component_lifecycle.touch_scroll.flings.remove(&id);
    }

    fn touch_scroll_live(&self, id: StableNodeId) -> bool {
        self.world.is_mounted(id)
            && self.world.is_overlay_reachable(id)
            && !self
                .views
                .get(&id)
                .is_some_and(|view| view.is::<TextArea>() || view.is::<crate::RichTextEditor>())
            && (self.is_scroll_view(id) || self.overflow_scrolls(id))
    }

    /// The axes `id` scrolls on as a touch target.
    fn touch_axes(&self, id: StableNodeId) -> Option<(bool, bool)> {
        if !self.touch_scroll_live(id) {
            return None;
        }
        if self.is_scroll_view(id) {
            return self
                .read(Entity::<ScrollView>::from_stable_id(id), |scroll| {
                    (
                        matches!(scroll.axes, ScrollAxes::Horizontal | ScrollAxes::Both),
                        matches!(scroll.axes, ScrollAxes::Vertical | ScrollAxes::Both),
                    )
                })
                .ok();
        }
        self.overflow_axes(id)
    }

    /// `[min, max]` offset of `id` on `axis`, and its viewport length.
    fn touch_range(&self, id: StableNodeId, axis: Axis) -> Option<(f32, f32, f32)> {
        let metrics = self.world.scroll_metrics(id)?;
        let (min, max) = (metrics.min_offset(), metrics.max_offset());
        Some(match axis {
            Axis::Horizontal => (min.x, max.x, metrics.viewport_width),
            Axis::Vertical => (min.y, max.y, metrics.viewport_height),
        })
    }

    /// Before the ordinary pointer path: `Some` when the gesture took the
    /// sample. A press is only noted here; [`Self::note_touch_target`] learns
    /// where it landed.
    pub(super) fn touch_scroll_pointer(
        &mut self,
        document: DocumentId,
        pointer: &PointerInput,
        now: Duration,
        landed: &mut Option<StableNodeId>,
    ) -> Result<Option<InputDisposition>, FrameworkError> {
        let key = (document, pointer.pointer_id.0);
        let (x, y) = (pointer.x, pointer.y);
        match pointer.phase {
            PointerPhase::Down => {
                if !pointer.is_primary || pointer.button != 0 {
                    return Ok(None);
                }
                self.forget_lifted_touches(document)?;
                let caught = self.catch_touch_fling(document, x, y);
                // A caught press skips the ordinary path, which would have
                // resolved where it landed.
                let hit = caught.and_then(|_| self.world.hit_test(document, x, y));
                self.component_lifecycle.touch_scroll.gestures.insert(
                    key,
                    TouchGesture {
                        origin: (x, y),
                        hit,
                        caught,
                        drag: None,
                        declined: false,
                        samples: vec![(now, x, y)],
                    },
                );
                if caught.is_some() {
                    // The tap that stops a moving list stops it, nothing more.
                    *landed = None;
                    self.set_pointer_location(document, key.1, Some((x, y)));
                    return Ok(Some(CONSUMED));
                }
                Ok(None)
            }
            PointerPhase::Move => {
                let Some(gesture) = self.component_lifecycle.touch_scroll.gestures.get_mut(&key)
                else {
                    return Ok(None);
                };
                gesture.samples.push((now, x, y));
                gesture
                    .samples
                    .retain(|(at, _, _)| now.saturating_sub(*at) <= VELOCITY_WINDOW);
                if gesture.declined {
                    return Ok(None);
                }
                if gesture.drag.is_none() {
                    let (dx, dy) = (x - gesture.origin.0, y - gesture.origin.1);
                    if dx.hypot(dy) <= TOUCH_SLOP {
                        return Ok(None);
                    }
                    if !self.begin_touch_drag(document, key.1, x, y, now)? {
                        if let Some(gesture) =
                            self.component_lifecycle.touch_scroll.gestures.get_mut(&key)
                        {
                            gesture.declined = true;
                        }
                        return Ok(None);
                    }
                }
                let Some(drag) = self
                    .component_lifecycle
                    .touch_scroll
                    .gestures
                    .get(&key)
                    .and_then(|gesture| gesture.drag)
                else {
                    return Ok(None);
                };
                *landed = Some(drag.target);
                self.set_pointer_location(document, key.1, Some((x, y)));
                self.drag_touch_scroll(drag, drag.axis.pick(x, y))?;
                Ok(Some(CONSUMED))
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                if pointer.phase == PointerPhase::Up && !pointer.is_primary {
                    return Ok(None);
                }
                let Some(mut gesture) = self.component_lifecycle.touch_scroll.gestures.remove(&key)
                else {
                    return Ok(None);
                };
                let cancel = pointer.phase == PointerPhase::Cancel;
                if !cancel {
                    gesture.samples.push((now, x, y));
                }
                let Some(drag) = gesture.drag else {
                    // A tap: a container it caught past an edge goes back.
                    if let Some(caught) = gesture.caught {
                        self.settle_touch_scroll(caught, None, 0.0, now)?;
                    }
                    return Ok(None);
                };
                let velocity = if cancel {
                    0.0
                } else {
                    // Content moves against the finger.
                    -release_velocity(&gesture.samples, drag.axis, now)
                };
                let mut mutations = MutationQueue::new();
                mutations.release_pointer(key.1, drag.target);
                self.commit_mutations(mutations)?;
                self.settle_touch_scroll(drag.target, Some(drag.axis), velocity, now)?;
                if cancel {
                    // Let the ordinary cancel clear what else the pointer held.
                    return Ok(None);
                }
                self.input.pressed_buttons.remove(&key);
                *landed = Some(drag.target);
                Ok(Some(CONSUMED))
            }
        }
    }

    /// Where the ordinary pointer path landed a touch press.
    pub(super) fn note_touch_target(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: Option<StableNodeId>,
    ) {
        if let Some(gesture) = self
            .component_lifecycle
            .touch_scroll
            .gestures
            .get_mut(&(document, pointer_id))
            && gesture.caught.is_none()
        {
            gesture.hit = target;
        }
    }

    /// A lifecycle cancel only reaches a pointer that held something, so a
    /// finger that pressed nothing can leave its gesture behind; it has no
    /// position any more. Drop it, and bounce back what it left past an edge.
    fn forget_lifted_touches(&mut self, document: DocumentId) -> Result<(), FrameworkError> {
        let stale: Vec<_> = self
            .component_lifecycle
            .touch_scroll
            .gestures
            .keys()
            .filter(|(owner, pointer)| {
                *owner == document && self.pointer_position(document, *pointer).is_none()
            })
            .copied()
            .collect();
        for key in stale {
            if let Some(gesture) = self.component_lifecycle.touch_scroll.gestures.remove(&key)
                && let Some(id) = gesture.drag.map(|drag| drag.target).or(gesture.caught)
            {
                let now = self.component_lifecycle.now;
                self.settle_touch_scroll(id, None, 0.0, now)?;
            }
        }
        Ok(())
    }

    /// A finger that lands on a moving container stops it. Returns that
    /// container when it was moving, which makes the press a catch.
    fn catch_touch_fling(&mut self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        if self.component_lifecycle.touch_scroll.flings.is_empty() {
            return None;
        }
        let mut current = self.world.hit_test(document, x, y);
        while let Some(id) = current {
            if self
                .component_lifecycle
                .touch_scroll
                .flings
                .remove(&id)
                .is_some()
            {
                return Some(id);
            }
            current = self.world.parent_id(id);
        }
        None
    }

    /// The finger left the slop: lock the dominant axis and hand the
    /// gesture to the innermost container that can still move that way, or,
    /// with none, to the innermost one that scrolls on the axis at all (so
    /// iOS can rubber-band it). Returns `false` when there is none, or a
    /// control holds the press for itself.
    fn begin_touch_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        now: Duration,
    ) -> Result<bool, FrameworkError> {
        let key = (document, pointer_id);
        let Some(gesture) = self.component_lifecycle.touch_scroll.gestures.get(&key) else {
            return Ok(false);
        };
        let (dx, dy) = (x - gesture.origin.0, y - gesture.origin.1);
        let axis = if dx.abs() > dy.abs() {
            Axis::Horizontal
        } else {
            Axis::Vertical
        };
        // Content moves against the finger.
        let toward = -axis.pick(dx, dy);
        let Some(target) = self.touch_scroll_target(document, gesture.hit, axis, toward) else {
            return Ok(false);
        };
        if !self.release_touch_press(document, pointer_id, axis, x, y)? {
            return Ok(false);
        }
        self.stop_scroll_motion(target);
        let current = Axis::of(self.world.scroll_offset(target).unwrap_or_default(), axis);
        let anchor_offset = match (self.scroll_physics(), self.touch_range(target, axis)) {
            (ScrollPhysics::Ios, Some((min, _, dimension))) if current < min => {
                min - ios::rubber_band_inverse(min - current, dimension)
            }
            (ScrollPhysics::Ios, Some((_, max, dimension))) if current > max => {
                max + ios::rubber_band_inverse(current - max, dimension)
            }
            _ => current,
        };
        let mut mutations = MutationQueue::new();
        mutations.capture_pointer(pointer_id, target);
        self.commit_mutations(mutations)?;
        // A scrolling finger hovers nothing it passes over.
        self.set_pointer_hover_at(document, pointer_id, None, now)?;
        if let Some(gesture) = self.component_lifecycle.touch_scroll.gestures.get_mut(&key) {
            gesture.drag = Some(TouchDrag {
                target,
                axis,
                anchor: axis.pick(x, y),
                anchor_offset,
            });
        }
        Ok(true)
    }

    fn touch_scroll_target(
        &self,
        document: DocumentId,
        hit: Option<StableNodeId>,
        axis: Axis,
        toward: f32,
    ) -> Option<StableNodeId> {
        let hit = hit?;
        // A blocking overlay bounds the walk, as it bounds the wheel.
        let root = self
            .active_runtime_overlays(document)
            .into_iter()
            .find(|overlay| overlay.kind.blocks_pointer())
            .map(|overlay| overlay.root)
            .filter(|root| self.overlay_descendant(*root, hit));
        let mut fallback = None;
        let mut current = Some(hit);
        while let Some(id) = current {
            let scrolls = self.touch_axes(id).is_some_and(|(x, y)| match axis {
                Axis::Horizontal => x,
                Axis::Vertical => y,
            });
            if scrolls && let Some((min, max, _)) = self.touch_range(id, axis) {
                let offset = Axis::of(self.world.scroll_offset(id).unwrap_or_default(), axis);
                if max - min > 0.5 {
                    let room = if toward > 0.0 {
                        max - offset
                    } else {
                        offset - min
                    };
                    if room > 0.5 {
                        return Some(id);
                    }
                    fallback = fallback.or(Some(id));
                }
            }
            if Some(id) == root
                || self
                    .world
                    .node_style(id)
                    .is_some_and(|style| style.layout.position == nana_ui_core::PositionSpec::Fixed)
            {
                break;
            }
            current = self.world.parent_id(id);
        }
        fallback
    }

    /// Take the press back from whatever the finger first pressed, so
    /// nothing activates on release. A horizontal range keeps a sideways
    /// drag; text selection gives way, as on touch platforms selection is a
    /// long press. Any other capture (a canvas, a handle, a pointer hook)
    /// keeps the gesture.
    fn release_touch_press(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        axis: Axis,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if let Some(owner) = self.world.pointer_capture(document, pointer_id) {
            let range = self.is_range_field(owner) || self.range_span_target(owner).is_some();
            let text = self
                .text_edit
                .text_pointer_drag
                .is_some_and(|(drag, node, _)| drag == pointer_id && node == owner);
            #[cfg(feature = "rich-text")]
            let text = text
                || self
                    .component_lifecycle
                    .rich_text_presses
                    .get(&(document, pointer_id))
                    == Some(&owner);
            if range && axis == Axis::Vertical {
                self.end_range_drag(document, pointer_id, true)?;
                self.end_range_span_drag(document, pointer_id, true)?;
            } else if text {
                self.text_editor_pointer_release(pointer_id);
                #[cfg(feature = "rich-text")]
                self.end_rich_text_pointer(document, pointer_id, x, y, true)?;
            } else {
                return Ok(false);
            }
        }
        let _ = (x, y);
        self.document_text_pointer_release(pointer_id);
        self.release_pointer(document, pointer_id);
        Ok(true)
    }

    fn drag_touch_scroll(&mut self, drag: TouchDrag, finger: f32) -> Result<bool, FrameworkError> {
        if !self.touch_scroll_live(drag.target) {
            return Ok(false);
        }
        let raw = drag.anchor_offset - (finger - drag.anchor);
        let shown = match self.touch_range(drag.target, drag.axis) {
            Some((min, max, dimension)) => match self.scroll_physics() {
                ScrollPhysics::Ios if raw < min => min - ios::rubber_band(min - raw, dimension),
                ScrollPhysics::Ios if raw > max => max + ios::rubber_band(raw - max, dimension),
                _ => raw.clamp(min, max),
            },
            None => raw,
        };
        let current = self.world.scroll_offset(drag.target).unwrap_or_default();
        self.apply_touch_scroll_to(drag.target, drag.axis.with(current, shown))
    }

    /// The finger lifted from `id` at `velocity` px/s on `axis` (content
    /// direction): fling, bounce back past an edge, or rest. With no axis
    /// known (a catch released, a gesture dropped) only an edge is settled.
    fn settle_touch_scroll(
        &mut self,
        id: StableNodeId,
        axis: Option<Axis>,
        velocity: f32,
        now: Duration,
    ) -> Result<(), FrameworkError> {
        if !self.touch_scroll_live(id) {
            return Ok(());
        }
        let current = self.world.scroll_offset(id).unwrap_or_default();
        let axes = match axis {
            Some(axis) => vec![axis],
            None => vec![Axis::Vertical, Axis::Horizontal],
        };
        for axis in axes {
            let Some((min, max, _)) = self.touch_range(id, axis) else {
                continue;
            };
            let position = Axis::of(current, axis);
            let edge = position.clamp(min, max);
            let motion = if self.reduced_motion() {
                None
            } else if position != edge {
                // Only iOS goes past an edge.
                Some(Motion::Bounce {
                    edge,
                    displacement: position - edge,
                    velocity,
                })
            } else {
                match self.scroll_physics() {
                    ScrollPhysics::Ios => (ios::decay_distance(velocity).abs() >= REST_DISTANCE)
                        .then_some(Motion::Decay {
                            from: position,
                            velocity,
                        }),
                    ScrollPhysics::Android => {
                        let speed = velocity.abs();
                        (speed >= android::MIN_FLING_VELOCITY).then(|| {
                            let speed = speed.min(android::MAX_FLING_VELOCITY);
                            Motion::Spline {
                                from: position,
                                distance: android::fling_distance(speed) * velocity.signum(),
                                duration: android::fling_duration(speed),
                            }
                        })
                    }
                }
            };
            match motion {
                Some(motion) => {
                    self.component_lifecycle.touch_scroll.flings.insert(
                        id,
                        TouchFling {
                            axis,
                            motion,
                            started: now,
                            next: now.saturating_add(INTERVAL),
                            last: current,
                        },
                    );
                    self.hold_scrollbars(id)?;
                    return Ok(());
                }
                None if position != edge => {
                    self.apply_touch_scroll_to(id, axis.with(current, edge))?;
                    return Ok(());
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Where a fling stands `elapsed` seconds in, against the edges `min`
    /// and `max`. A deceleration that crosses an edge turns into a bounce
    /// at the moment it crosses, so samples land where they would at any
    /// frame rate. `None` motion means the fling is over.
    fn fling_position(
        motion: Motion,
        started: Duration,
        now: Duration,
        min: f32,
        max: f32,
    ) -> (f32, Option<(Motion, Duration)>) {
        let elapsed = now.saturating_sub(started).as_secs_f32();
        match motion {
            Motion::Decay { from, velocity } => {
                let edge = if velocity > 0.0 { max } else { min };
                if let Some(at) = ios::decay_time_to(velocity, edge - from)
                    && at <= elapsed
                {
                    let (_, speed) = ios::decay(velocity, at);
                    let crossed = started + Duration::from_secs_f32(at);
                    let bounce = Motion::Bounce {
                        edge,
                        displacement: 0.0,
                        velocity: speed,
                    };
                    return Self::fling_position(bounce, crossed, now, min, max);
                }
                let (distance, speed) = ios::decay(velocity, elapsed);
                if ios::decay_distance(speed).abs() < REST_DISTANCE {
                    return ((from + ios::decay_distance(velocity)).clamp(min, max), None);
                }
                (from + distance, Some((motion, started)))
            }
            Motion::Spline {
                from,
                distance,
                duration,
            } => {
                if duration <= 0.0 || elapsed >= duration {
                    return ((from + distance).clamp(min, max), None);
                }
                let (share, _) = android::spline(elapsed / duration);
                let position = from + share * distance;
                // OverScroller with no overscroll stops at the edge.
                if (distance > 0.0 && position >= max) || (distance < 0.0 && position <= min) {
                    return (position.clamp(min, max), None);
                }
                (position, Some((motion, started)))
            }
            Motion::Bounce {
                edge,
                displacement,
                velocity,
            } => {
                let (x, v) = ios::bounce(displacement, velocity, elapsed);
                if x.abs() < REST_DISTANCE && v.abs() < REST_VELOCITY {
                    return (edge, None);
                }
                (edge + x, Some((motion, started)))
            }
        }
    }

    fn sample_touch_fling(
        &mut self,
        id: StableNodeId,
        now: Duration,
    ) -> Result<bool, FrameworkError> {
        let Some(mut fling) = self
            .component_lifecycle
            .touch_scroll
            .flings
            .get(&id)
            .copied()
        else {
            return Ok(false);
        };
        let current = self.world.scroll_offset(id).unwrap_or_default();
        let range = self
            .touch_scroll_live(id)
            .then(|| self.touch_range(id, fling.axis))
            .flatten()
            .filter(|_| current == fling.last);
        let Some((min, max, _)) = range else {
            self.component_lifecycle.touch_scroll.flings.remove(&id);
            // A container hidden mid-bounce comes back inside its range.
            let clamped = self.world.clamp_scroll_offset(id, current);
            if current == fling.last && clamped != current && self.world.is_mounted(id) {
                let mut mutations = MutationQueue::new();
                mutations.set_scroll_overscroll(id, clamped);
                self.world.commit(mutations)?;
            }
            return Ok(false);
        };
        let (position, next) = Self::fling_position(fling.motion, fling.started, now, min, max);
        let offset = fling.axis.with(current, position);
        match next {
            Some((motion, started)) => {
                fling.motion = motion;
                fling.started = started;
                fling.next = now.saturating_add(INTERVAL);
                fling.last = offset;
                self.component_lifecycle
                    .touch_scroll
                    .flings
                    .insert(id, fling);
            }
            None => {
                self.component_lifecycle.touch_scroll.flings.remove(&id);
            }
        }
        let changed = self.apply_touch_scroll_to(id, offset)?;
        // An event observer may have moved or removed the container. Never
        // resurrect the fling after that callback.
        if changed
            && let Some(state) = self.component_lifecycle.touch_scroll.flings.get_mut(&id)
            && state.next == fling.next
        {
            state.last = self.world.scroll_offset(id).unwrap_or(offset);
        }
        Ok(changed)
    }

    pub(super) fn advance_touch_scrolls(&mut self, now: Duration, frame: &mut AnimationFrame) {
        let mut due = std::mem::take(&mut self.component_lifecycle.touch_scroll.due);
        due.extend(
            self.component_lifecycle
                .touch_scroll
                .flings
                .iter()
                .filter(|(id, fling)| fling.next <= now || !self.touch_scroll_live(**id))
                .map(|(id, _)| *id),
        );
        for id in due.drain(..) {
            match self.sample_touch_fling(id, now) {
                Ok(true) => frame.component_updates.push(id),
                Ok(false) => {}
                Err(_) => {
                    self.component_lifecycle.touch_scroll.flings.remove(&id);
                }
            }
        }
        self.component_lifecycle.touch_scroll.due = due;
    }

    /// Reduced motion lands every fling at once: past an edge on the edge,
    /// otherwise where it is.
    pub(super) fn finish_touch_scrolls(&mut self) {
        let flings = std::mem::take(&mut self.component_lifecycle.touch_scroll.flings);
        for id in flings.into_keys() {
            if !self.touch_scroll_live(id) {
                continue;
            }
            let current = self.world.scroll_offset(id).unwrap_or_default();
            let _ = self.apply_touch_scroll_to(id, self.world.clamp_scroll_offset(id, current));
        }
    }
}

/// The finger's velocity on `axis` in px/s: a least-squares line through the
/// samples of the last [`VELOCITY_WINDOW`]. A finger that rested before it
/// lifted has one sample left and no velocity.
fn release_velocity(samples: &[(Duration, f32, f32)], axis: Axis, now: Duration) -> f32 {
    let recent: Vec<(f32, f32)> = samples
        .iter()
        .filter(|(at, _, _)| now.saturating_sub(*at) <= VELOCITY_WINDOW)
        .map(|(at, x, y)| (at.as_secs_f32(), axis.pick(*x, *y)))
        .collect();
    if recent.len() < 2 {
        return 0.0;
    }
    let n = recent.len() as f32;
    let t0 = recent[0].0;
    let mean_t = recent.iter().map(|(t, _)| t - t0).sum::<f32>() / n;
    let mean_p = recent.iter().map(|(_, p)| p).sum::<f32>() / n;
    let (mut covariance, mut variance) = (0.0, 0.0);
    for (t, p) in &recent {
        let dt = t - t0 - mean_t;
        covariance += dt * (p - mean_p);
        variance += dt * dt;
    }
    if variance <= f32::EPSILON {
        return 0.0;
    }
    covariance / variance
}
