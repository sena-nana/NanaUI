//! [`ToastStack`]: the notices of one corner of a window. The newest toast
//! stands at the anchored edge and older ones above it; a toast shows once it
//! is measured and there is room for it, and waits its turn otherwise.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, FlexDirection, JustifySpec, LengthSpec, PaintTransform, PointerEventsSpec,
    PopoverAlignment, PositionSpec, ToastRecipe, VisibilitySpec,
};

use crate::panel::{origin, positive};
use crate::view::Presence;
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AnimatableProperty, AppContext, ComponentView,
    DocumentId, Entity, FocusWithinChanged, FrameworkError, InteractionState, LayoutBox,
    MotionValue, MutationQueue, NodeKind, NodeStyle, PanelInsets, PointerHoverChanged,
    StableNodeId, Toast, UiWorld,
};

/// A column of toasts in one corner of a window, newest at the bottom.
///
/// Toasts go in with [`AppContext::push_toast`], which assembles each one
/// ([`AppContext::assemble_toast`]), and leave with
/// [`AppContext::remove_toast`]. A new toast is laid out unseen first; it
/// shows once it is measured, newest first, while fewer than
/// [`Self::max_visible`] are on screen and their heights and the gaps between
/// them fit the stack's height ([`Self::place_in`]). The rest wait, hidden
/// and with their timers held, and come back when there is room. Toasts come
/// and go with the theme's [`ToastRecipe::motion`]. Each toast in the column
/// keeps where it rests (its bottom edge's height above the stack's), and
/// whatever moves it — a toast joining or leaving below it, one below growing
/// or shrinking — slides it there from where it is seen; with reduced motion
/// they appear, go and move at once.
///
/// While the pointer is over a toast of the stack, or focus is inside it,
/// every timer in the stack holds and later goes on with the time it had
/// left. The stack itself takes no pointer input, so the gaps between toasts
/// and the room above them pass through to whatever is underneath, and the
/// toasts in it keep taking presses while a modal dialog is open, when the
/// application puts the stack above its modal layer (`z_index`). Removing a
/// toast — after [`crate::ToastDismissed`] or [`crate::ToastAction`] — is the
/// application's call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToastStack {
    pub style: NodeStyle,
    /// Most toasts on screen at once, a leaving one not counted; newer ones
    /// push older ones into the queue. 3 by default.
    pub max_visible: usize,
    /// The region's accessible name; `None` says the framework's
    /// (`toast.region`).
    pub label: Option<Arc<str>>,
}

impl Default for ToastStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastStack {
    pub fn new() -> Self {
        Self {
            style: NodeStyle::default(),
            max_visible: 3,
            label: None,
        }
    }

    pub fn max_visible(mut self, max_visible: usize) -> Self {
        self.max_visible = max_visible;
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Pin the stack into the space `insets` leaves free in `viewport`, the
    /// reservation contract of [`crate::Toast::place_in`] and
    /// [`crate::Panel::viewport`]: `align` places it across that free width
    /// and its toasts within it (`End` keeps them on the right edge),
    /// `max_width` caps its width and `max_height` the height its toasts may
    /// take, and it rests on the reserved bottom edge, growing upwards.
    /// `viewport` is the positioned host's own box. Geometry only.
    pub fn place_in(
        &mut self,
        viewport: LayoutBox,
        align: PopoverAlignment,
        max_width: f32,
        insets: PanelInsets,
        max_height: f32,
    ) {
        let insets = insets.clamp_into(viewport);
        let free = positive(viewport.width) - insets.left - insets.right;
        let free_height = positive(viewport.height) - insets.top - insets.bottom;
        let width = positive(max_width).min(free);
        let leading = match align {
            PopoverAlignment::Start => 0.0,
            PopoverAlignment::Center => (free - width) / 2.0,
            PopoverAlignment::End => free - width,
        };
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.position = PositionSpec::Absolute;
        layout.offset_left = Some(LengthSpec::Px(origin(viewport.x) + insets.left + leading));
        layout.offset_right = None;
        layout.offset_top = None;
        layout.offset_bottom = Some(LengthSpec::Px(insets.bottom));
        layout.width = Some(LengthSpec::Px(width));
        layout.max_height = Some(LengthSpec::Px(positive(max_height).min(free_height)));
        layout.align_items = match align {
            PopoverAlignment::Start => AlignSpec::Start,
            PopoverAlignment::Center => AlignSpec::Center,
            PopoverAlignment::End => AlignSpec::End,
        };
    }

    /// The height the stack's toasts may take: the `max_height` it was
    /// placed with, or no limit.
    fn height_budget(&self) -> f32 {
        match self.style.layout.max_height {
            Some(LengthSpec::Px(height)) => positive(height),
            _ => f32::INFINITY,
        }
    }

    fn effective_style(&self, world: &UiWorld) -> NodeStyle {
        let recipe = world.theme().recipes().toast();
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        // The containing block of the slot a toast is measured in, also
        // before the stack is placed in its viewport.
        if !layout.position.establishes_containing_block() {
            layout.position = PositionSpec::Relative;
        }
        // Resting on its bottom edge: what does not fit spills upwards.
        layout.justify_content = JustifySpec::End;
        layout.gap = Some(LengthSpec::Px(recipe.stack_gap));
        style
    }
}

impl ComponentView for ToastStack {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(AppContext::reconcile_toast_stack),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "toast-stack".into(),
        }
    }

    /// The recipe's stack gap is authored onto the node.
    fn wants_recipe_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let label = self
            .label
            .clone()
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| Arc::clone(&world.framework_strings().toast_region));
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(world),
            // Only the toasts take the pointer: the gaps and the room above
            // them reach whatever is underneath.
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Region,
                label: Some(label),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Where a toast of a stack is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// Laid out unseen and out of flow, so the toasts on screen stay put.
    Measuring,
    Shown,
    /// Waiting for room: not laid out, not heard, its timer held.
    Queued,
    /// Playing its exit in its place, out of reach of the pointer and
    /// assistive technology; it goes when the exit ends.
    Leaving,
}

impl Slot {
    /// Whether the slot takes room in the column.
    const fn flows(self) -> bool {
        matches!(self, Self::Shown | Self::Leaving)
    }
}

/// The box a stack holds each toast in. What the stack measures, hides and
/// moves is the slot, so the toast keeps its own style.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToastSlot {
    pub(crate) slot: Slot,
}

impl ComponentView for ToastSlot {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "toast-slot".into(),
        }
    }

    /// The recipe's scaling origin is authored onto the node.
    fn wants_recipe_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        // What scales is the slot: its entrance, exit and change in place
        // grow out of the point the theme names.
        layout.transform_origin = Some(world.theme().recipes().toast().motion.origin);
        // As wide as its toast, never wider than the stack: the stack's
        // alignment places it.
        layout.width = Some(LengthSpec::FitContent);
        layout.max_width = Some(LengthSpec::Percent(100.0));
        layout.flex_shrink = Some(0.0);
        match self.slot {
            Slot::Measuring => {
                layout.position = PositionSpec::Absolute;
                layout.offset_left = Some(LengthSpec::Px(0.0));
                layout.offset_top = Some(LengthSpec::Px(0.0));
                layout.paint.visibility = Some(VisibilitySpec::Hidden);
                layout.pointer_events = Some(PointerEventsSpec::None);
            }
            Slot::Queued => layout.hidden = true,
            Slot::Leaving => layout.pointer_events = Some(PointerEventsSpec::None),
            Slot::Shown => {}
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                hidden: self.slot != Slot::Shown,
                ..AccessibilityState::default()
            },
        );
    }
}

/// One toast a stack holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StackEntry {
    pub(crate) toast: StableNodeId,
    pub(crate) slot: StableNodeId,
    pub(crate) status: Slot,
    /// Its height when last measured or laid out; `None` until measured.
    pub(crate) height: Option<f32>,
    /// The layout pass it started measuring after: a later pass measured it.
    pub(crate) measure_after: usize,
    /// Where it rests in the column: how far its bottom edge stands above
    /// the stack's, as last laid out in flow. `None` until then, and again
    /// once it leaves the flow.
    pub(crate) rest: Option<f32>,
}

/// What a stack holds, kept by the context so a stack written whole keeps it.
#[derive(Debug, Default)]
pub(crate) struct StackState {
    /// Oldest first, as the slots stand in the column.
    pub(crate) entries: Vec<StackEntry>,
    /// Slots whose toast something else despawned; they go next time the
    /// stack fits.
    pub(crate) orphans: Vec<StableNodeId>,
    /// Whether the stack hears the pointer and focus come and go.
    pub(crate) listening: bool,
}

impl StackState {
    fn entry(&self, toast: StableNodeId) -> Option<&StackEntry> {
        self.entries.iter().find(|entry| entry.toast == toast)
    }
}

/// The toast's entrance: fading in from transparent and, when the recipe
/// moves it, from its offset and scale. Played back, its exit.
fn presence(recipe: &ToastRecipe, transition: nana_ui_core::DialogTransition) -> Presence {
    let motion = recipe.motion;
    let presence = Presence::new(transition.duration())
        .opacity(0.0)
        .ease(transition.easing);
    if motion.moves() {
        presence.transform(PaintTransform {
            a: motion.enter_scale,
            b: 0.0,
            c: 0.0,
            d: motion.enter_scale,
            e: 0.0,
            f: motion.enter_offset_y,
        })
    } else {
        presence
    }
}

impl AppContext {
    /// Assemble `toast` ([`Self::assemble_toast`]) and add it to `stack` as
    /// the newest. It is laid out unseen first and shows once measured and
    /// there is room; until then its timer does not count. Never moves
    /// focus. The entity is the toast's, for its events and updates.
    pub fn push_toast(
        &mut self,
        stack: Entity<ToastStack>,
        toast: Toast,
    ) -> Result<Entity<Toast>, FrameworkError> {
        self.read(stack, |_| ())?;
        let id = stack.stable_id();
        let document = self
            .world()
            .node(id)
            .ok_or(FrameworkError::MissingView(id))?
            .document;
        self.listen_to_toast_stack(stack)?;
        let slot = self.create_detached_component(
            document,
            ToastSlot {
                slot: Slot::Measuring,
            },
        )?;
        let toast = self.create_detached_component(document, toast)?;
        self.assemble_toast(toast)?;
        self.append_child(slot, toast)?;
        let measure_after = self.layout_invocations();
        self.toast_stacks_mut()
            .entry(id)
            .or_default()
            .entries
            .push(StackEntry {
                toast: toast.stable_id(),
                slot: slot.stable_id(),
                status: Slot::Measuring,
                height: None,
                measure_after,
                rest: None,
            });
        self.append_child(stack, slot)?;
        self.sync_toast_timer(toast.stable_id())?;
        Ok(toast)
    }

    /// Take `toast` out of `stack`. One on screen plays its exit in its
    /// place, then goes, and the rest close up behind it; one still waiting
    /// or being measured goes at once, and with reduced motion every one
    /// does. `Ok(false)` when the stack does not hold it.
    pub fn remove_toast(
        &mut self,
        stack: Entity<ToastStack>,
        toast: Entity<Toast>,
    ) -> Result<bool, FrameworkError> {
        self.read(stack, |_| ())?;
        let Some(entry) = self
            .toast_stacks()
            .get(&stack.stable_id())
            .and_then(|state| state.entry(toast.stable_id()))
            .copied()
        else {
            return Ok(false);
        };
        match entry.status {
            Slot::Leaving => return Ok(true),
            Slot::Shown => {
                self.set_slot(stack.stable_id(), entry.slot, Slot::Leaving)?;
                self.sync_toast_timer(entry.toast)?;
                let recipe = *self.world().theme().recipes().toast();
                let motion = recipe.motion;
                let exit = presence(&recipe, motion.exit);
                // The rest close up when the layout after it goes moves them
                // ([`Self::slide_toast_stack`]).
                if !self.reduced_motion() && self.begin_leave(entry.slot, &exit, None)? {
                    return Ok(true);
                }
            }
            Slot::Measuring | Slot::Queued => {}
        }
        let mut mutations = MutationQueue::new();
        mutations.despawn_subtree(entry.slot);
        self.commit_mutations(mutations)?;
        Ok(true)
    }

    /// Answer `toast`'s copy changing in place: one on screen springs back
    /// to rest from the theme's [`nana_ui_core::ToastMotion::change_scale`]
    /// about its origin, without fading. Nothing plays for a toast that is
    /// not on screen, while it is still entering or sliding, with reduced
    /// motion, or when the theme's change scale is 1. `Ok(false)` when
    /// nothing plays.
    pub fn pulse_toast(
        &mut self,
        stack: Entity<ToastStack>,
        toast: Entity<Toast>,
    ) -> Result<bool, FrameworkError> {
        self.read(stack, |_| ())?;
        let Some(entry) = self
            .toast_stacks()
            .get(&stack.stable_id())
            .and_then(|state| state.entry(toast.stable_id()))
            .copied()
        else {
            return Ok(false);
        };
        let motion = self.world().theme().recipes().toast().motion;
        if entry.status != Slot::Shown || !motion.changes() || self.reduced_motion() {
            return Ok(false);
        }
        // Its entrance or a slide owns the transform until it lands.
        let moving = [
            crate::motion_api::user_animation_id(entry.slot, AnimatableProperty::Transform),
            crate::component_animation_id(crate::component_animation_kinds::FLIP, entry.slot),
        ]
        .into_iter()
        .flatten()
        .any(|id| self.world().animation_is_active(id));
        if moving {
            return Ok(false);
        }
        let change = Presence::new(motion.change.duration())
            .scale(motion.change_scale)
            .ease(motion.change.easing);
        self.begin_enter(&[entry.slot], &change)?;
        Ok(true)
    }

    /// Take every toast out of `stack`, as [`Self::remove_toast`] does.
    pub fn clear_toasts(&mut self, stack: Entity<ToastStack>) -> Result<(), FrameworkError> {
        self.read(stack, |_| ())?;
        let toasts = self
            .toast_stacks()
            .get(&stack.stable_id())
            .map(|state| {
                state
                    .entries
                    .iter()
                    .map(|entry| entry.toast)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for toast in toasts {
            self.remove_toast(stack, Entity::from_stable_id(toast))?;
        }
        Ok(())
    }

    /// The toasts `stack` has on screen, newest first: not those waiting or
    /// still being measured, and not one leaving.
    pub fn shown_toasts(&self, stack: Entity<ToastStack>) -> Vec<Entity<Toast>> {
        self.toast_stacks()
            .get(&stack.stable_id())
            .map(|state| {
                state
                    .entries
                    .iter()
                    .rev()
                    .filter(|entry| entry.status == Slot::Shown)
                    .map(|entry| Entity::from_stable_id(entry.toast))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The stack that holds `toast`, and where in it the toast is.
    pub(crate) fn toast_stack_place(&self, toast: StableNodeId) -> Option<(StableNodeId, Slot)> {
        self.toast_stacks()
            .iter()
            .find_map(|(stack, state)| state.entry(toast).map(|entry| (*stack, entry.status)))
    }

    /// [`ToastStack`]'s assembler: fit the stack again and hold or run its
    /// timers, after a write to it or the pointer or focus coming or going.
    pub(crate) fn reconcile_toast_stack(
        &mut self,
        stack: Entity<ToastStack>,
    ) -> Result<bool, FrameworkError> {
        let id = stack.stable_id();
        if !self.toast_stacks().contains_key(&id) {
            return Ok(false);
        }
        self.fit_toast_stack(id)?;
        let toasts: Vec<StableNodeId> = self
            .toast_stacks()
            .get(&id)
            .map(|state| state.entries.iter().map(|entry| entry.toast).collect())
            .unwrap_or_default();
        for toast in toasts {
            self.sync_toast_timer(toast)?;
        }
        Ok(true)
    }

    /// After a layout pass of `document`: measure the stacks' new toasts
    /// and fit what shows.
    pub(crate) fn reconcile_toast_stacks(
        &mut self,
        document: DocumentId,
    ) -> Result<(), FrameworkError> {
        if self.toast_stacks().is_empty() {
            return Ok(());
        }
        let stacks = self
            .toast_stacks()
            .keys()
            .copied()
            .filter(|stack| self.world().document_of(*stack) == Some(document))
            .collect::<Vec<_>>();
        for stack in stacks {
            self.fit_toast_stack(stack)?;
        }
        Ok(())
    }

    /// Hear the pointer and focus come and go over `stack`'s toasts: the
    /// write that delivers either fits the stack and syncs its timers.
    fn listen_to_toast_stack(&mut self, stack: Entity<ToastStack>) -> Result<(), FrameworkError> {
        if self
            .toast_stacks()
            .get(&stack.stable_id())
            .is_some_and(|state| state.listening)
        {
            return Ok(());
        }
        self.on(stack, |_, _: &PointerHoverChanged, _| {})?;
        self.on(stack, |_, _: &FocusWithinChanged, _| {})?;
        self.toast_stacks_mut()
            .entry(stack.stable_id())
            .or_default()
            .listening = true;
        Ok(())
    }

    fn set_slot(
        &mut self,
        stack: StableNodeId,
        slot: StableNodeId,
        status: Slot,
    ) -> Result<(), FrameworkError> {
        if let Some(entry) = self
            .toast_stacks_mut()
            .get_mut(&stack)
            .and_then(|state| state.entries.iter_mut().find(|entry| entry.slot == slot))
        {
            entry.status = status;
        }
        self.update_component(Entity::<ToastSlot>::from_stable_id(slot), |view, _| {
            view.slot = status;
        })
    }

    /// Measure what was laid out unseen and decide what shows: newest first,
    /// while fewer than `max_visible` are on screen and their heights and
    /// gaps fit the stack's height. The newest always shows. A toast that
    /// waited comes back through measuring.
    fn fit_toast_stack(&mut self, stack: StableNodeId) -> Result<(), FrameworkError> {
        let orphans = self
            .toast_stacks_mut()
            .get_mut(&stack)
            .map(|state| std::mem::take(&mut state.orphans))
            .unwrap_or_default();
        let orphans = orphans
            .into_iter()
            .filter(|slot| self.world().contains(*slot))
            .collect::<Vec<_>>();
        if !orphans.is_empty() {
            let mut mutations = MutationQueue::new();
            for slot in orphans {
                mutations.despawn_subtree(slot);
            }
            self.commit_mutations(mutations)?;
        }
        let Ok(view) = self.read(Entity::<ToastStack>::from_stable_id(stack), Clone::clone) else {
            return Ok(());
        };
        if !self.world().is_mounted(stack) {
            return Ok(());
        }
        let Some(entries) = self
            .toast_stacks()
            .get(&stack)
            .map(|state| state.entries.clone())
        else {
            return Ok(());
        };
        let pass = self.layout_invocations();
        let recipe = *self.world().theme().recipes().toast();
        let mut next = entries.clone();
        for entry in &mut next {
            let laid_out = match entry.status {
                Slot::Measuring => pass > entry.measure_after,
                Slot::Shown | Slot::Leaving => true,
                Slot::Queued => false,
            };
            if laid_out && let Some(bounds) = self.world().layout_box(entry.slot) {
                entry.height = Some(bounds.height);
            }
        }
        let measuring = next
            .iter()
            .any(|entry| entry.status == Slot::Measuring && pass <= entry.measure_after);
        if !measuring {
            let budget = view.height_budget();
            let (mut count, mut used, mut placed, mut fitting) = (0, 0.0_f32, false, true);
            for entry in next.iter_mut().rev() {
                let height = entry.height.unwrap_or(0.0);
                let take = if placed {
                    recipe.stack_gap + height
                } else {
                    height
                };
                if entry.status == Slot::Leaving {
                    used += take;
                    placed = true;
                    continue;
                }
                let room =
                    fitting && count < view.max_visible && (count == 0 || used + take <= budget);
                if !room {
                    fitting = false;
                    entry.status = Slot::Queued;
                    continue;
                }
                count += 1;
                used += take;
                placed = true;
                if entry.status == Slot::Queued {
                    // It comes back measured: the rest wait for that.
                    entry.status = Slot::Measuring;
                    entry.measure_after = pass;
                    fitting = false;
                } else {
                    entry.status = Slot::Shown;
                }
            }
        }
        let changes = entries
            .iter()
            .zip(&next)
            .filter(|(before, after)| before.status != after.status)
            .map(|(before, after)| (after.slot, before.status, after.status))
            .collect::<Vec<_>>();
        let slides = self.slide_toast_stack(stack, &entries, &mut next);
        if let Some(state) = self.toast_stacks_mut().get_mut(&stack) {
            state.entries = next.clone();
        }
        for (slot, _, status) in &changes {
            self.update_component(Entity::<ToastSlot>::from_stable_id(*slot), |view, _| {
                view.slot = *status;
            })?;
        }
        let reduced = self.reduced_motion();
        for (slot, before, after) in &changes {
            if *after == Slot::Shown && *before != Slot::Shown && !reduced {
                self.begin_enter(&[*slot], &presence(&recipe, recipe.motion.enter))?;
            }
        }
        if !slides.is_empty() {
            self.commit_mutations(slides)?;
        }
        if !changes.is_empty() {
            for entry in &next {
                self.sync_toast_timer(entry.toast)?;
            }
        }
        Ok(())
    }

    /// Take where each toast in the column rests now and slide the ones that
    /// moved, from where they are seen, to their new place: a toast joining
    /// or leaving below them, one below growing or shrinking (its words
    /// changing, or wrapping anew at another width). A toast's rest is its
    /// bottom edge's height above the stack's, so one whose own height
    /// changes stays put and only those above it move. `before` is the
    /// stack's entries as they stood, `next` as they stand now; a rest is
    /// taken only from a toast already in the column before, whose box is its
    /// place in it.
    fn slide_toast_stack(
        &self,
        stack: StableNodeId,
        before: &[StackEntry],
        next: &mut [StackEntry],
    ) -> MutationQueue {
        let mut slides = MutationQueue::new();
        let Some(stack_box) = self.world().layout_box(stack) else {
            return slides;
        };
        let bottom = stack_box.y + stack_box.height;
        let moves = !self.reduced_motion();
        let stack_move = self.world().theme().recipes().toast().motion.stack_move;
        let now = self.animation_now();
        for (was, entry) in before.iter().zip(next.iter_mut()) {
            if !(was.status.flows() && entry.status.flows()) {
                entry.rest = None;
                continue;
            }
            let Some(last) = self.flip_first(entry.slot) else {
                continue;
            };
            let rest = bottom - (last.y + last.height);
            let previous = entry.rest.replace(rest);
            let Some(previous) = previous.filter(|previous| (rest - previous).abs() > 0.5) else {
                continue;
            };
            if !moves {
                continue;
            }
            // A slide still running carries on from where it has got to.
            let flip =
                crate::component_animation_id(crate::component_animation_kinds::FLIP, entry.slot);
            let sliding = flip
                .filter(|id| self.world().animation_is_active(*id))
                .and_then(|_| {
                    self.world().presentation_motion_value(
                        entry.slot,
                        AnimatableProperty::Transform,
                        now,
                    )
                })
                .map_or(0.0, |value| match value {
                    MotionValue::Transform(transform) => transform.f,
                    _ => 0.0,
                });
            // A toast still entering or springing back slides from where it
            // stands: the slide owns its transform from here, so it ends at
            // rest.
            if let Some(enter) =
                crate::motion_api::user_animation_id(entry.slot, AnimatableProperty::Transform)
                && self.world().animation_is_active(enter)
            {
                slides.stop_animation(enter);
            }
            let first = nana_ui_core::FlipRect {
                y: last.y + rest - previous + sliding,
                ..last
            };
            slides.start_layout_flip(
                entry.slot,
                first,
                last,
                now,
                stack_move.duration(),
                stack_move.easing,
            );
        }
        slides
    }

    /// The parts of stacks despawned in a commit: a stack with its toasts;
    /// a toast with its slot; and a slot whose toast went alone, which goes
    /// next time the stack fits.
    pub(crate) fn forget_toast_stack_nodes(
        &mut self,
        removed: &std::collections::HashSet<StableNodeId>,
    ) {
        let stacks = self.toast_stacks_mut();
        stacks.retain(|stack, _| !removed.contains(stack));
        for state in stacks.values_mut() {
            let mut orphans = Vec::new();
            state.entries.retain(|entry| {
                if removed.contains(&entry.slot) {
                    return false;
                }
                if removed.contains(&entry.toast) {
                    orphans.push(entry.slot);
                    return false;
                }
                true
            });
            state.orphans.extend(orphans);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use nana_ui_core::{DialogTransition, Easing, ThemeDefinition, ToastMotion};
    use nana_ui_input::PointerPhase;

    use super::*;
    use crate::{
        Activate, Button, Dialog, DialogCloseRequested, LayoutViewport, MotionValue, OverlayHost,
        OverlayKey, OverlayPointerPhase, Stack, ToastAction, ToastDismissed, ToastTone,
    };

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    const WINDOW: LayoutBox = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    const INSETS: PanelInsets = PanelInsets {
        top: 16.0,
        right: 16.0,
        bottom: 16.0,
        left: 16.0,
    };

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// A window: a stage the pointer lands on, and a stack in its
    /// bottom-right corner, 360 wide and `max_height` tall at most.
    fn window(
        context: &mut AppContext,
        max_visible: usize,
        max_height: f32,
    ) -> (Entity<ToastStack>, Entity<Stack>) {
        let stage = context
            .create_component(
                document(),
                Stack::from_layout(nana_ui_core::LayoutStyle {
                    position: PositionSpec::Relative,
                    width: Some(LengthSpec::Px(WINDOW.width)),
                    height: Some(LengthSpec::Px(WINDOW.height)),
                    ..nana_ui_core::LayoutStyle::default()
                }),
            )
            .unwrap();
        let mut stack = ToastStack::new().max_visible(max_visible);
        stack.place_in(WINDOW, PopoverAlignment::End, 360.0, INSETS, max_height);
        let stack = context
            .create_detached_component(document(), stack)
            .unwrap();
        context.append_child(stage, stack).unwrap();
        (stack, stage)
    }

    /// A host frame: styles, text, layout, again until nothing is left to
    /// do, then the hit index.
    fn frame(context: &mut AppContext) {
        let viewport = LayoutViewport::new(WINDOW.width, WINDOW.height);
        let mut shaper = crate::MeasureTextShaper;
        for _ in 0..8 {
            let work = context.take_system_work();
            if work.is_empty() {
                break;
            }
            context.resolve_styles(&work.style).unwrap();
            context.shape_text(&work.text, &mut shaper).unwrap();
            context.layout_document(document(), viewport).unwrap();
            context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap();
            context.layout_document(document(), viewport).unwrap();
        }
        context.rebuild_hit_test(document());
    }

    /// A host's frame as `RuntimeDocument::flush` drives it: the first
    /// layout for the viewport, then incremental passes from the seeds each
    /// batch of work left, until nothing is left to do.
    fn host_frame(context: &mut AppContext, first: bool) {
        let viewport = LayoutViewport::new(WINDOW.width, WINDOW.height);
        let mut shaper = crate::MeasureTextShaper;
        let mut force = first;
        for _ in 0..8 {
            let work = context.take_system_work();
            if work.is_empty() && !force {
                break;
            }
            context.resolve_styles(&work.style).unwrap();
            context.shape_text(&work.text, &mut shaper).unwrap();
            let mut seeds = work.layout_frontier_seeds.clone();
            seeds.extend(context.take_layout_frontier_seeds(document()));
            if force || !seeds.is_empty() {
                if force {
                    context
                        .layout_document_for_viewport(document(), viewport, &seeds)
                        .unwrap();
                } else {
                    context
                        .layout_document_with_frontier(document(), viewport, &seeds)
                        .unwrap();
                }
                let mut scope = context.take_last_layout_scope();
                for _ in 0..4 {
                    if !context
                        .shape_text_for_layout_scoped(&scope, &mut shaper)
                        .unwrap()
                    {
                        break;
                    }
                    let seeds = context.take_layout_frontier_seeds(document());
                    if seeds.is_empty() {
                        break;
                    }
                    context
                        .layout_document_with_frontier(document(), viewport, &seeds)
                        .unwrap();
                    scope = context.take_last_layout_scope();
                }
            }
            force = false;
        }
        context.rebuild_hit_test(document());
    }

    fn slot_of(context: &AppContext, toast: Entity<Toast>) -> StableNodeId {
        context.world().parent_id(toast.stable_id()).unwrap()
    }

    fn bounds(context: &AppContext, id: StableNodeId) -> LayoutBox {
        context.world().layout_box(id).unwrap()
    }

    fn status(context: &AppContext, toast: Entity<Toast>) -> Option<Slot> {
        context
            .toast_stack_place(toast.stable_id())
            .map(|(_, slot)| slot)
    }

    fn spoken(context: &AppContext, id: StableNodeId) -> bool {
        context
            .world()
            .project_accessibility(document())
            .iter()
            .any(|node| node.id == id)
    }

    fn notice(title: &str) -> Toast {
        Toast::new(title, ToastTone::Info)
    }

    fn translate_y(context: &AppContext, id: StableNodeId, now: Duration) -> f32 {
        match context
            .world()
            .presentation_motion_value(id, AnimatableProperty::Transform, now)
        {
            Some(MotionValue::Transform(transform)) => transform.f,
            other => panic!("transform of {id:?}: {other:?}"),
        }
    }

    fn counter<E: Send + 'static>(
        context: &mut AppContext,
        toast: Entity<Toast>,
    ) -> Arc<Mutex<u32>> {
        let count = Arc::new(Mutex::new(0));
        let heard = Arc::clone(&count);
        context
            .on(toast, move |_, _: &E, _| *heard.lock().unwrap() += 1)
            .unwrap();
        count
    }

    fn heard(count: &Arc<Mutex<u32>>) -> u32 {
        *count.lock().unwrap()
    }

    /// `place_in` pins the stack into what the insets leave free, on the
    /// bottom inset, capped in width and height, its toasts aligned as
    /// asked; a tiny window keeps it inside.
    #[test]
    fn place_in_pins_the_stack_into_the_free_corner() {
        let mut stack = ToastStack::new();
        let insets = PanelInsets {
            top: 48.0,
            right: 416.0,
            bottom: 100.0,
            left: 16.0,
        };
        stack.place_in(WINDOW, PopoverAlignment::End, 360.0, insets, 1000.0);
        let layout = &stack.style.layout;
        assert_eq!(layout.position, PositionSpec::Absolute);
        assert_eq!(layout.width, Some(LengthSpec::Px(360.0)));
        assert_eq!(
            layout.offset_left,
            Some(LengthSpec::Px(800.0 - 416.0 - 360.0))
        );
        assert_eq!(layout.offset_bottom, Some(LengthSpec::Px(100.0)));
        assert_eq!(layout.offset_top, None);
        assert_eq!(
            layout.max_height,
            Some(LengthSpec::Px(600.0 - 48.0 - 100.0)),
            "never taller than the free height"
        );
        assert_eq!(layout.align_items, AlignSpec::End);

        stack.place_in(WINDOW, PopoverAlignment::Start, 200.0, insets, 300.0);
        let layout = &stack.style.layout;
        assert_eq!(layout.offset_left, Some(LengthSpec::Px(16.0)));
        assert_eq!(layout.max_height, Some(LengthSpec::Px(300.0)));
        assert_eq!(layout.align_items, AlignSpec::Start);

        let tiny = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        stack.place_in(tiny, PopoverAlignment::Center, 360.0, insets, 300.0);
        let layout = &stack.style.layout;
        assert_eq!(layout.width, Some(LengthSpec::Px(0.0)));
        assert_eq!(layout.max_height, Some(LengthSpec::Px(0.0)));
    }

    /// A pushed toast is assembled and laid out unseen and out of flow,
    /// timer held; the pass that measures it shows it, and the next one puts
    /// it in its place at the anchored edge. Pushing never moves focus.
    #[test]
    fn a_pushed_toast_is_measured_unseen_then_shown() {
        let mut context = AppContext::new();
        let (stack, stage) = window(&mut context, 3, 400.0);
        let button = context
            .create_detached_component(document(), Button::new("Import"))
            .unwrap();
        context.append_child(stage, button).unwrap();
        frame(&mut context);
        context.focus_node(document(), button.stable_id()).unwrap();

        let toast = context
            .push_toast(stack, notice("Optimizing").timeout(Duration::from_secs(4)))
            .unwrap();
        assert!(context.read(toast, |toast| toast.parts.is_some()).unwrap());
        let slot = slot_of(&context, toast);
        assert_eq!(status(&context, toast), Some(Slot::Measuring));
        assert!(context.shown_toasts(stack).is_empty());
        let style = context.world().node_style(slot).unwrap();
        assert_eq!(style.layout.position, PositionSpec::Absolute);
        assert_eq!(style.layout.paint.visibility, Some(VisibilitySpec::Hidden));
        context.advance_animations(Duration::from_secs(1));
        assert_eq!(
            context.next_animation_deadline(),
            None,
            "its timer waits until it shows"
        );

        let viewport = LayoutViewport::new(WINDOW.width, WINDOW.height);
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        assert!(!spoken(&context, toast.stable_id()), "measured unheard");
        context.layout_document(document(), viewport).unwrap();
        assert_eq!(context.shown_toasts(stack), vec![toast], "measured, shown");

        frame(&mut context);
        let stack_box = bounds(&context, stack.stable_id());
        let slot_box = bounds(&context, slot);
        assert!(slot_box.height > 0.0);
        assert!((stack_box.y + stack_box.height - (WINDOW.height - INSETS.bottom)).abs() < 0.01);
        assert!((slot_box.y + slot_box.height - (stack_box.y + stack_box.height)).abs() < 0.01);
        assert!(
            (slot_box.x + slot_box.width - (WINDOW.width - INSETS.right)).abs() < 0.01,
            "on the right edge: {slot_box:?}"
        );
        assert!(spoken(&context, toast.stable_id()));
        assert!(
            context.next_animation_deadline().is_some(),
            "its timer runs"
        );
        assert_eq!(
            context.world().focused(document()),
            Some(button.stable_id()),
            "a notice never takes focus"
        );
    }

    /// A host lays out incrementally. A stack in flow in a padded list —
    /// the shape the gallery mounts — stays as tall as the toasts it shows
    /// while they are measured, shown, queued, brought back, removed and
    /// cleared, and the layout verifier finds every incremental pass equal to
    /// a full layout: a slot leaving or joining the flow re-measures the
    /// stack.
    #[test]
    fn incremental_host_frames_keep_a_stack_in_flow_as_tall_as_its_toasts() {
        let mut context = AppContext::new();
        let mut root_style = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut root_style.layout);
            layout.width = Some(LengthSpec::Percent(100.0));
            layout.height = Some(LengthSpec::Percent(100.0));
            layout.padding_left = Some(LengthSpec::Px(20.0));
            layout.padding_right = Some(LengthSpec::Px(20.0));
            layout.padding_top = Some(LengthSpec::Px(20.0));
            layout.padding_bottom = Some(LengthSpec::Px(20.0));
        }
        let root = context
            .create_component(document(), crate::List::new().style(root_style))
            .unwrap();
        let mut style = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Percent(100.0));
            layout.align_items = AlignSpec::End;
        }
        let stack = context
            .create_component(document(), ToastStack::new().max_visible(2).style(style))
            .unwrap();
        let toasts = ["First", "Second", "Third"]
            .map(|title| context.push_toast(stack, notice(title)).unwrap());
        let mut hierarchy = MutationQueue::new();
        hierarchy.insert(root.stable_id(), stack.stable_id(), None);
        context.commit_mutations(hierarchy).unwrap();
        host_frame(&mut context, true);

        let gap = ToastRecipe::DEFAULT.stack_gap;
        let as_tall_as_its_toasts = |context: &AppContext| {
            let shown = context.shown_toasts(stack);
            let heights: f32 = shown
                .iter()
                .map(|toast| bounds(context, slot_of(context, *toast)).height)
                .sum();
            let expected = heights + gap * shown.len().saturating_sub(1) as f32;
            let height = bounds(context, stack.stable_id()).height;
            assert!(
                (height - expected).abs() < 0.01,
                "{height} for {} toasts, {expected} expected",
                shown.len()
            );
            shown
        };
        assert_eq!(as_tall_as_its_toasts(&context), vec![toasts[2], toasts[1]]);

        context.remove_toast(stack, toasts[2]).unwrap();
        host_frame(&mut context, false);
        context.advance_animations(Duration::from_secs(1));
        host_frame(&mut context, false);
        assert_eq!(as_tall_as_its_toasts(&context), vec![toasts[1], toasts[0]]);

        context.clear_toasts(stack).unwrap();
        host_frame(&mut context, false);
        context.advance_animations(Duration::from_secs(2));
        host_frame(&mut context, false);
        assert!(as_tall_as_its_toasts(&context).is_empty());
    }

    /// Newer toasts stand below older ones, a stack gap apart.
    #[test]
    fn newer_toasts_stand_below_older_ones() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 3, 400.0);
        let older = context.push_toast(stack, notice("Older")).unwrap();
        let newer = context.push_toast(stack, notice("Newer")).unwrap();
        frame(&mut context);
        assert_eq!(context.shown_toasts(stack), vec![newer, older]);
        let top = bounds(&context, slot_of(&context, older));
        let bottom = bounds(&context, slot_of(&context, newer));
        let gap = ToastRecipe::DEFAULT.stack_gap;
        assert!(
            (top.y + top.height + gap - bottom.y).abs() < 0.01,
            "{top:?} {bottom:?}"
        );
    }

    /// More toasts than `max_visible`: the newest show and the oldest waits
    /// out of sight; when one leaves, it comes back through measuring.
    #[test]
    fn max_visible_queues_the_oldest_and_a_removal_brings_it_back() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 2, 400.0);
        let first = context.push_toast(stack, notice("First")).unwrap();
        let second = context.push_toast(stack, notice("Second")).unwrap();
        let third = context.push_toast(stack, notice("Third")).unwrap();
        frame(&mut context);
        assert_eq!(context.shown_toasts(stack), vec![third, second]);
        assert_eq!(status(&context, first), Some(Slot::Queued));
        let waiting = slot_of(&context, first);
        assert!(context.world().node_style(waiting).unwrap().layout.hidden);
        assert!(!spoken(&context, first.stable_id()));

        assert!(context.remove_toast(stack, third).unwrap());
        assert_eq!(status(&context, third), Some(Slot::Leaving));
        frame(&mut context);
        assert_eq!(context.shown_toasts(stack), vec![second, first]);
        context.advance_animations(Duration::from_secs(1));
        frame(&mut context);
        assert_eq!(status(&context, third), None, "gone once its exit ended");
        assert_eq!(context.shown_toasts(stack), vec![second, first]);
        let top = bounds(&context, slot_of(&context, first));
        let bottom = bounds(&context, slot_of(&context, second));
        assert!(top.y < bottom.y);
    }

    /// Toasts that do not fit the stack's height wait, newest first; the
    /// newest shows even when it alone is taller. A new placement fits the
    /// stack again.
    #[test]
    fn what_does_not_fit_the_height_waits_and_a_new_placement_refits() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 5, 400.0);
        let first = context.push_toast(stack, notice("First")).unwrap();
        frame(&mut context);
        let height = bounds(&context, slot_of(&context, first)).height;
        let gap = ToastRecipe::DEFAULT.stack_gap;
        let second = context.push_toast(stack, notice("Second")).unwrap();
        let third = context.push_toast(stack, notice("Third")).unwrap();
        frame(&mut context);
        assert_eq!(context.shown_toasts(stack), vec![third, second, first]);

        let place = |context: &mut AppContext, max_height: f32| {
            context
                .update_component(stack, |stack, _| {
                    stack.place_in(WINDOW, PopoverAlignment::End, 360.0, INSETS, max_height)
                })
                .unwrap();
            frame(context);
        };
        place(&mut context, 2.0 * height + gap + 0.5);
        assert_eq!(context.shown_toasts(stack), vec![third, second]);
        assert_eq!(status(&context, first), Some(Slot::Queued));

        place(&mut context, height / 2.0);
        assert_eq!(
            context.shown_toasts(stack),
            vec![third],
            "the newest shows even when it alone is too tall"
        );

        place(&mut context, 400.0);
        assert_eq!(context.shown_toasts(stack), vec![third, second, first]);
    }

    /// A removed toast plays its exit in its place, out of reach of the
    /// pointer; when it ends the toast goes and the rest slide down into
    /// the room it leaves.
    #[test]
    fn a_removed_toast_leaves_in_place_then_the_rest_close_up() {
        let mut context = AppContext::new();
        let (stack, stage) = window(&mut context, 3, 400.0);
        let older = context.push_toast(stack, notice("Older")).unwrap();
        let newer = context.push_toast(stack, notice("Newer")).unwrap();
        frame(&mut context);
        let start = Duration::from_secs(1);
        context.advance_animations(start);
        let above = slot_of(&context, older);
        let leaving = slot_of(&context, newer);
        let was = bounds(&context, above);
        let place = bounds(&context, leaving);

        assert!(context.remove_toast(stack, newer).unwrap());
        assert!(
            context.remove_toast(stack, newer).unwrap(),
            "already leaving"
        );
        frame(&mut context);
        assert_eq!(bounds(&context, leaving), place, "it keeps its slot");
        assert_eq!(bounds(&context, above), was);
        assert_eq!(context.shown_toasts(stack), vec![older]);
        let centre = (place.x + place.width / 2.0, place.y + place.height / 2.0);
        assert_eq!(
            context.world().hit_test(document(), centre.0, centre.1),
            Some(stage.stable_id()),
            "a leaving toast takes no pointer"
        );
        let opacity = match context.world().presentation_motion_value(
            leaving,
            AnimatableProperty::Opacity,
            start + ms(70),
        ) {
            Some(MotionValue::Scalar(opacity)) => opacity,
            other => panic!("{other:?}"),
        };
        assert!(opacity > 0.0 && opacity < 1.0, "fading: {opacity}");

        let ended = start + ms(200);
        context.advance_animations(ended);
        assert!(!context.world().contains(leaving));
        assert!(!context.world().contains(newer.stable_id()));
        frame(&mut context);
        let now = bounds(&context, above);
        assert!(now.y > was.y, "closed up: {was:?} -> {now:?}");
        let offset = translate_y(&context, above, ended);
        assert!(
            (offset - (was.y - now.y)).abs() < 0.5,
            "slides from where it was: {offset}"
        );
        let settled = translate_y(&context, above, ended + ms(400));
        assert!(settled.abs() < 0.01, "{settled}");
    }

    /// `remove_toast` answers whether the stack held the toast; one waiting
    /// or still being measured goes at once. `clear_toasts` takes them all.
    #[test]
    fn remove_and_clear_take_toasts_out() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 1, 400.0);
        let (other, _) = window(&mut context, 1, 400.0);
        let waiting = context.push_toast(stack, notice("Waiting")).unwrap();
        let shown = context.push_toast(stack, notice("Shown")).unwrap();
        frame(&mut context);
        assert_eq!(status(&context, waiting), Some(Slot::Queued));
        assert!(
            !context.remove_toast(other, shown).unwrap(),
            "not that stack's"
        );
        let slot = slot_of(&context, waiting);
        assert!(context.remove_toast(stack, waiting).unwrap());
        assert!(
            !context.world().contains(slot),
            "a waiting toast goes at once"
        );
        assert!(
            !context.remove_toast(stack, waiting).unwrap(),
            "no longer held"
        );

        let measuring = context.push_toast(stack, notice("Measuring")).unwrap();
        let slot = slot_of(&context, measuring);
        context.clear_toasts(stack).unwrap();
        assert!(!context.world().contains(slot));
        assert_eq!(status(&context, shown), Some(Slot::Leaving));
        assert!(context.shown_toasts(stack).is_empty());
        context.advance_animations(Duration::from_secs(1));
        assert!(!context.world().contains(shown.stable_id()));
        assert!(
            context
                .world()
                .node(stack.stable_id())
                .unwrap()
                .children
                .is_empty()
        );
    }

    /// The pointer over any toast of a stack holds every timer in it, and
    /// so does focus inside it; each goes on with the time it had left. A
    /// toast that waits for room does not count down at all.
    #[test]
    fn hover_and_focus_hold_every_timer_and_waiting_toasts_do_not_count() {
        let seconds = Duration::from_secs;
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 2, 400.0);
        let waiting = context
            .push_toast(stack, notice("Waiting").timeout(seconds(2)))
            .unwrap();
        let older = context
            .push_toast(stack, notice("Older").timeout(seconds(4)).dismissible(true))
            .unwrap();
        let newer = context
            .push_toast(stack, notice("Newer").timeout(seconds(4)))
            .unwrap();
        let counts =
            [waiting, older, newer].map(|toast| counter::<ToastDismissed>(&mut context, toast));
        frame(&mut context);
        assert_eq!(status(&context, waiting), Some(Slot::Queued));

        // Counting from 10s; a second in, the pointer comes over the older.
        context.advance_animations(seconds(10));
        context.advance_animations(seconds(11));
        context
            .set_pointer_hover_at(document(), 1, Some(older.stable_id()), seconds(11))
            .unwrap();
        context.advance_animations(seconds(20));
        assert_eq!(counts.each_ref().map(heard), [0, 0, 0], "held");

        // Three seconds were left, counted from the frame after it leaves.
        context
            .set_pointer_hover_at(document(), 1, None, seconds(20))
            .unwrap();
        context.advance_animations(seconds(20) + ms(16));
        context.advance_animations(seconds(22) + ms(16));
        // Focus inside the stack holds them a second before the end.
        let close = context
            .read(older, |toast| toast.parts)
            .unwrap()
            .unwrap()
            .close;
        context.focus_node(document(), close).unwrap();
        context.advance_animations(seconds(40));
        assert_eq!(counts.each_ref().map(heard), [0, 0, 0], "held by focus");
        context.clear_focus(document()).unwrap();
        context.advance_animations(seconds(40) + ms(16));
        context.advance_animations(seconds(41));
        assert_eq!(counts.each_ref().map(heard), [0, 0, 0]);
        context.advance_animations(seconds(41) + ms(16));
        assert_eq!(counts.each_ref().map(heard), [0, 1, 1]);
        assert_eq!(
            status(&context, waiting),
            Some(Slot::Queued),
            "never counted"
        );

        // Room at last: it shows and counts its whole timeout.
        context.remove_toast(stack, newer).unwrap();
        frame(&mut context);
        assert_eq!(context.shown_toasts(stack), vec![older, waiting]);
        context.advance_animations(seconds(50));
        context.advance_animations(seconds(51));
        assert_eq!(heard(&counts[0]), 0);
        context.advance_animations(seconds(52));
        assert_eq!(heard(&counts[0]), 1);
    }

    /// With motion running nothing else, the stack wakes the host only for
    /// its earliest timer.
    #[test]
    fn an_idle_stack_wakes_only_for_its_earliest_timer() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 3, 400.0);
        context
            .push_toast(stack, notice("Later").timeout(Duration::from_secs(8)))
            .unwrap();
        context
            .push_toast(stack, notice("Sooner").timeout(Duration::from_secs(3)))
            .unwrap();
        context.push_toast(stack, notice("Sticky")).unwrap();
        frame(&mut context);
        // The frame the timers arm for starts their counts; the next one
        // samples them.
        let start = Duration::from_secs(10);
        context.advance_animations(start);
        context.advance_animations(start);
        assert_eq!(
            context.next_animation_deadline(),
            Some(start + Duration::from_secs(3))
        );
    }

    /// Pressing a toast's action and close buttons reaches the application
    /// as `ToastAction` and `ToastDismissed` on the toast.
    #[test]
    fn pressing_a_toast_s_buttons_reaches_the_application() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 3, 400.0);
        let toast = context
            .push_toast(stack, notice("Optimized").action("Open").dismissible(true))
            .unwrap();
        frame(&mut context);
        context.advance_animations(Duration::from_secs(1));
        let actions = counter::<ToastAction>(&mut context, toast);
        let dismissals = counter::<ToastDismissed>(&mut context, toast);
        let parts = context.read(toast, |toast| toast.parts).unwrap().unwrap();
        let mut input = crate::HeadlessInput::bind(&mut context, document());
        for part in [parts.action, parts.close] {
            let target = bounds(&context, part);
            let (x, y) = (
                target.x + target.width / 2.0,
                target.y + target.height / 2.0,
            );
            for phase in [PointerPhase::Move, PointerPhase::Down, PointerPhase::Up] {
                input.pointer(&mut context, phase, x, y).unwrap();
            }
        }
        assert_eq!((heard(&actions), heard(&dismissals)), (1, 1));
    }

    /// A window with a dialog open in an overlay host and a stack at
    /// `z_index` holding a toast with an action and a close button; focus on
    /// the dialog's Save. The dialog's frame paints at `z_index` 1000.
    struct ModalScene {
        stack: Entity<ToastStack>,
        host: Entity<OverlayHost>,
        dialog: Entity<Dialog>,
        cancel: Entity<Button>,
        save: Entity<Button>,
        toast: Entity<Toast>,
        /// Close gestures the dialog heard: a press outside it is one.
        requests: Arc<Mutex<u32>>,
    }

    fn modal_scene(context: &mut AppContext, z_index: i32) -> ModalScene {
        let (stack, stage) = window(context, 3, 400.0);
        context
            .update_component(stack, |stack, _| {
                Arc::make_mut(&mut stack.style.layout).z_index = Some(z_index);
            })
            .unwrap();
        let host = context
            .create_detached_component(document(), OverlayHost::new())
            .unwrap();
        context.append_child(stage, host).unwrap();
        let dialog = context
            .create_detached_component(document(), Dialog::new("Settings"))
            .unwrap();
        let cancel = context
            .create_detached_component(document(), Button::new("Cancel"))
            .unwrap();
        let save = context
            .create_detached_component(document(), Button::new("Save"))
            .unwrap();
        context
            .set_modal_slots(
                dialog,
                crate::ModalSlots {
                    actions: vec![cancel.stable_id(), save.stable_id()],
                    ..Default::default()
                },
            )
            .unwrap();
        context.append_child(host, dialog).unwrap();
        let toast = context
            .push_toast(stack, notice("Optimized").action("Open").dismissible(true))
            .unwrap();
        frame(context);
        assert!(context.activate_overlay(host, dialog).unwrap());
        frame(context);
        context.advance_animations(Duration::from_secs(1));
        frame(context);
        context.focus_node(document(), save.stable_id()).unwrap();
        let requests = Arc::new(Mutex::new(0));
        let count = Arc::clone(&requests);
        context
            .on(dialog, move |_, _: &DialogCloseRequested, _| {
                *count.lock().unwrap() += 1
            })
            .unwrap();
        ModalScene {
            stack,
            host,
            dialog,
            cancel,
            save,
            toast,
            requests,
        }
    }

    fn press(context: &mut AppContext, part: StableNodeId) {
        let target = bounds(context, part);
        let (x, y) = (
            target.x + target.width / 2.0,
            target.y + target.height / 2.0,
        );
        let mut input = crate::HeadlessInput::bind(context, document());
        for phase in [PointerPhase::Move, PointerPhase::Down, PointerPhase::Up] {
            input.pointer(context, phase, x, y).unwrap();
        }
    }

    fn open_overlay(context: &AppContext, host: Entity<OverlayHost>) -> Option<StableNodeId> {
        context
            .world()
            .overlay_host(host.stable_id())
            .and_then(|state| state.active)
    }

    /// The application puts the stack above its modal layer: with a dialog
    /// open, a press on a toast reaches it and is no press outside the
    /// dialog, focus stays in the dialog and Tab still cycles only its
    /// controls.
    #[test]
    fn a_stack_above_a_modal_keeps_its_presses_and_the_modal_stays_open() {
        let mut context = AppContext::new();
        let scene = modal_scene(&mut context, 1_100);
        let actions = counter::<ToastAction>(&mut context, scene.toast);
        let dismissals = counter::<ToastDismissed>(&mut context, scene.toast);
        let saved = Arc::new(Mutex::new(0));
        let count = Arc::clone(&saved);
        context
            .on(scene.save, move |_, _: &Activate, _| {
                *count.lock().unwrap() += 1
            })
            .unwrap();
        let parts = context
            .read(scene.toast, |toast| toast.parts)
            .unwrap()
            .unwrap();

        press(&mut context, parts.action);
        assert_eq!(heard(&actions), 1, "the press reached the toast");
        assert_eq!(
            open_overlay(&context, scene.host),
            Some(scene.dialog.stable_id()),
            "no press outside"
        );
        assert_eq!(
            context.world().focused(document()),
            Some(scene.save.stable_id()),
            "focus stays in the dialog"
        );
        press(&mut context, parts.close);
        assert_eq!(heard(&dismissals), 1);
        assert!(!context.covered_by_blocking_overlay(document(), parts.action));

        // The toast's own surface keeps its press too.
        let surface = bounds(&context, scene.toast.stable_id());
        let (x, y) = (surface.x + 4.0, surface.y + 4.0);
        let down = context
            .route_overlay_pointer(document(), 7, OverlayPointerPhase::PrimaryDown, x, y)
            .unwrap();
        assert_eq!(down.target, Some(scene.toast.stable_id()));
        let up = context
            .route_overlay_pointer(document(), 7, OverlayPointerPhase::PrimaryUp, x, y)
            .unwrap();
        assert!(!up.dismissed);
        assert_eq!(
            open_overlay(&context, scene.host),
            Some(scene.dialog.stable_id())
        );

        // Tab cycles the dialog's controls only.
        let mut seen = Vec::new();
        for _ in 0..4 {
            context
                .route_overlay_key(document(), OverlayKey::Tab { reverse: false })
                .unwrap();
            seen.push(context.world().focused(document()).unwrap());
        }
        assert!(
            seen.iter().all(|focused| {
                *focused == scene.cancel.stable_id() || *focused == scene.save.stable_id()
            }),
            "{seen:?}"
        );
        assert_eq!(*saved.lock().unwrap(), 0);
        assert_eq!(
            *scene.requests.lock().unwrap(),
            0,
            "no close gesture reached the dialog"
        );
        let _ = scene.stack;
    }

    /// Below the modal layer a stack is covered like the rest of the
    /// window: the dialog takes the press, as one outside it.
    #[test]
    fn a_stack_below_a_modal_is_covered_by_it() {
        let mut context = AppContext::new();
        let scene = modal_scene(&mut context, 10);
        let actions = counter::<ToastAction>(&mut context, scene.toast);
        let parts = context
            .read(scene.toast, |toast| toast.parts)
            .unwrap()
            .unwrap();
        press(&mut context, parts.action);
        assert_eq!(heard(&actions), 0, "the modal takes it");
        assert_eq!(
            *scene.requests.lock().unwrap(),
            1,
            "a press outside the dialog"
        );
        let _ = scene.stack;
    }

    /// With reduced motion a toast shows and goes at once: no fade, no move.
    #[test]
    fn reduced_motion_shows_and_removes_at_once() {
        let mut context = AppContext::new();
        context.set_reduced_motion(true);
        let moving = ThemeDefinition::NANA_DARK.with_toast(ToastRecipe {
            motion: ToastMotion {
                enter_offset_y: 8.0,
                enter_scale: 0.96,
                ..ToastMotion::DEFAULT
            },
            ..ToastRecipe::DEFAULT
        });
        assert!(context.set_theme_definition(&moving).unwrap());
        let (stack, _) = window(&mut context, 3, 400.0);
        let older = context.push_toast(stack, notice("Older")).unwrap();
        frame(&mut context);
        let newer = context.push_toast(stack, notice("Newer")).unwrap();
        frame(&mut context);
        for toast in [older, newer] {
            let slot = slot_of(&context, toast);
            for property in [AnimatableProperty::Opacity, AnimatableProperty::Transform] {
                assert!(
                    !context
                        .world()
                        .presentation_motion_value(slot, property, Duration::ZERO)
                        .is_some_and(|value| match value {
                            MotionValue::Scalar(opacity) => opacity != 1.0,
                            MotionValue::Transform(transform) => {
                                transform != nana_ui_core::PaintTransform::default()
                            }
                            _ => false,
                        }),
                    "{property:?} of {toast:?} is at rest"
                );
            }
        }
        let slot = slot_of(&context, newer);
        assert!(context.remove_toast(stack, newer).unwrap());
        assert!(!context.world().contains(slot), "gone at once");
    }

    /// A toast still entering when a newer one pushes it up slides from
    /// where it stands and comes to rest in its place, untransformed.
    #[test]
    fn a_toast_entering_while_it_slides_ends_at_rest() {
        let mut context = AppContext::new();
        let moving = ThemeDefinition::NANA_DARK.with_toast(ToastRecipe {
            motion: ToastMotion {
                enter: DialogTransition::new(300, Easing::EaseOutCubic),
                enter_offset_y: 8.0,
                enter_scale: 0.96,
                ..ToastMotion::DEFAULT
            },
            ..ToastRecipe::DEFAULT
        });
        assert!(context.set_theme_definition(&moving).unwrap());
        let (stack, _) = window(&mut context, 3, 400.0);
        let start = Duration::from_secs(1);
        context.advance_animations(start);
        let older = context.push_toast(stack, notice("Older")).unwrap();
        frame(&mut context);
        let slot = slot_of(&context, older);
        let entering = translate_y(&context, slot, start + ms(10));
        assert!(entering > 0.0, "it enters from below: {entering}");

        let midway = start + ms(100);
        context.advance_animations(midway);
        let was = bounds(&context, slot);
        context.push_toast(stack, notice("Newer")).unwrap();
        frame(&mut context);
        let now = bounds(&context, slot);
        assert!(now.y < was.y, "pushed up");
        let sliding = translate_y(&context, slot, midway);
        assert!(
            (sliding - (was.y - now.y)).abs() < 0.5,
            "slides from where it stood: {sliding}"
        );

        let rest = midway + Duration::from_secs(1);
        context.advance_animations(rest);
        assert_eq!(
            context
                .world()
                .presentation_motion_value(slot, AnimatableProperty::Transform, rest),
            Some(MotionValue::Transform(
                nana_ui_core::PaintTransform::default()
            )),
        );
        assert_eq!(
            context
                .world()
                .presentation_motion_value(slot, AnimatableProperty::Opacity, rest),
            Some(MotionValue::Scalar(1.0)),
        );
    }

    /// Whether `id` is drawn untransformed at `now`.
    fn at_rest(context: &AppContext, id: StableNodeId, now: Duration) -> bool {
        match context
            .world()
            .presentation_motion_value(id, AnimatableProperty::Transform, now)
        {
            None => true,
            Some(MotionValue::Transform(transform)) => {
                transform == nana_ui_core::PaintTransform::default()
            }
            other => panic!("transform of {id:?}: {other:?}"),
        }
    }

    /// Two toasts at rest, `start` + 1s on the animation clock: the older
    /// above the newer.
    fn two_at_rest(
        context: &mut AppContext,
        stack: Entity<ToastStack>,
        start: Duration,
    ) -> (Entity<Toast>, Entity<Toast>, Duration) {
        context.advance_animations(start);
        let older = context.push_toast(stack, notice("Older")).unwrap();
        let newer = context.push_toast(stack, notice("Newer")).unwrap();
        frame(context);
        let settled = start + Duration::from_secs(1);
        context.advance_animations(settled);
        frame(context);
        (older, newer, settled)
    }

    /// A toast that grows in place keeps its bottom edge where it was; the
    /// ones above it slide up from where they stood, and come to rest.
    #[test]
    fn a_toast_growing_below_slides_the_ones_above_it() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 3, 400.0);
        let (older, newer, settled) = two_at_rest(&mut context, stack, Duration::from_secs(1));
        let above = slot_of(&context, older);
        let below = slot_of(&context, newer);
        let was = bounds(&context, above);
        let base = bounds(&context, below);

        context
            .update_component(newer, |toast, _| {
                toast.description = Some("Saved to the library, ready to place.".into());
            })
            .unwrap();
        frame(&mut context);
        let grown = bounds(&context, below);
        assert!(grown.height > base.height, "{base:?} -> {grown:?}");
        assert!(
            (grown.y + grown.height - (base.y + base.height)).abs() < 0.01,
            "its bottom edge stays"
        );
        assert!(at_rest(&context, below, settled), "it does not slide");
        let now = bounds(&context, above);
        assert!(now.y < was.y, "pushed up: {was:?} -> {now:?}");
        let offset = translate_y(&context, above, settled);
        assert!(
            (offset - (was.y - now.y)).abs() < 0.5,
            "slides from where it stood: {offset}"
        );
        let landed = settled + ms(400);
        context.advance_animations(landed);
        assert!(at_rest(&context, above, landed));

        // Shrinking back slides it down again.
        context
            .update_component(newer, |toast, _| toast.description = None)
            .unwrap();
        frame(&mut context);
        let back = bounds(&context, above);
        assert!((back.y - was.y).abs() < 0.01);
        let offset = translate_y(&context, above, landed);
        assert!(
            (offset - (now.y - back.y)).abs() < 0.5,
            "slides down from where it stood: {offset}"
        );
    }

    /// A toast already sliding when the column moves again carries on from
    /// where it has got to, never from its old place.
    #[test]
    fn a_toast_moved_again_while_sliding_carries_on_from_where_it_is() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 3, 400.0);
        let start = Duration::from_secs(1);
        context.advance_animations(start);
        let first = context.push_toast(stack, notice("First")).unwrap();
        frame(&mut context);
        let joined = start + Duration::from_secs(1);
        context.advance_animations(joined);
        context.push_toast(stack, notice("Second")).unwrap();
        frame(&mut context);
        let slot = slot_of(&context, first);
        let midway = joined + ms(60);
        context.advance_animations(midway);
        let seen = translate_y(&context, slot, midway);
        assert!(seen > 0.5, "still sliding: {seen}");
        let was = bounds(&context, slot);

        context.push_toast(stack, notice("Third")).unwrap();
        frame(&mut context);
        let now = bounds(&context, slot);
        assert!(now.y < was.y);
        let offset = translate_y(&context, slot, midway);
        assert!(
            (offset - (was.y - now.y + seen)).abs() < 0.5,
            "from where it was seen: {offset}, {seen} + {}",
            was.y - now.y
        );
        assert!(at_rest(&context, slot, midway + ms(400)));
    }

    /// A narrower stack wraps a toast taller: the ones above it slide up
    /// as they do for any change of height below them.
    #[test]
    fn a_toast_wrapping_taller_at_a_new_width_slides_the_ones_above_it() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 3, 400.0);
        let (older, newer, settled) = two_at_rest(&mut context, stack, Duration::from_secs(1));
        context
            .update_component(newer, |toast, _| {
                toast.description =
                    Some("A description long enough to wrap once the stack narrows.".into());
            })
            .unwrap();
        frame(&mut context);
        let later = settled + Duration::from_secs(1);
        context.advance_animations(later);
        frame(&mut context);
        let above = slot_of(&context, older);
        let was = bounds(&context, above);

        context
            .update_component(stack, |stack, _| {
                stack.place_in(WINDOW, PopoverAlignment::End, 180.0, INSETS, 400.0)
            })
            .unwrap();
        frame(&mut context);
        let now = bounds(&context, above);
        assert!(now.y < was.y, "the newer wraps taller: {was:?} -> {now:?}");
        let offset = translate_y(&context, above, later);
        assert!(
            (offset - (was.y - now.y)).abs() < 0.5,
            "slides from where it stood: {offset}"
        );
    }

    /// The slot scales about the theme's origin: a stack in the
    /// bottom-right corner grows its toasts out of that corner.
    #[test]
    fn a_slot_scales_about_the_theme_s_origin() {
        let mut context = AppContext::new();
        let corner = nana_ui_core::TransformOrigin {
            x: LengthSpec::Percent(100.0),
            y: LengthSpec::Percent(100.0),
        };
        let cornered = ThemeDefinition::NANA_DARK.with_toast(ToastRecipe {
            motion: ToastMotion {
                origin: corner,
                ..ToastMotion::DEFAULT
            },
            ..ToastRecipe::DEFAULT
        });
        assert!(context.set_theme_definition(&cornered).unwrap());
        let (stack, _) = window(&mut context, 3, 400.0);
        let toast = context.push_toast(stack, notice("Saved")).unwrap();
        frame(&mut context);
        let slot = slot_of(&context, toast);
        assert_eq!(
            context
                .world()
                .node_style(slot)
                .and_then(|style| style.layout.transform_origin),
            Some(corner),
        );
    }

    /// A toast on screen whose copy changes springs back to rest from the
    /// theme's change scale without fading; nothing plays while it is still
    /// entering, or when the theme's change scale is 1.
    #[test]
    fn a_toast_on_screen_springs_back_when_its_copy_changes() {
        let mut context = AppContext::new();
        let springy = ThemeDefinition::NANA_DARK.with_toast(ToastRecipe {
            motion: ToastMotion {
                enter: DialogTransition::new(200, Easing::EaseOutCubic),
                enter_scale: 0.96,
                change: DialogTransition::new(200, Easing::EaseOutCubic),
                change_scale: 0.9,
                ..ToastMotion::DEFAULT
            },
            ..ToastRecipe::DEFAULT
        });
        assert!(context.set_theme_definition(&springy).unwrap());
        let (stack, _) = window(&mut context, 3, 400.0);
        let start = Duration::from_secs(1);
        context.advance_animations(start);
        let toast = context.push_toast(stack, notice("Optimising")).unwrap();
        frame(&mut context);
        assert!(
            !context.pulse_toast(stack, toast).unwrap(),
            "its entrance owns the motion"
        );

        let settled = start + Duration::from_secs(1);
        context.advance_animations(settled);
        frame(&mut context);
        assert!(context.pulse_toast(stack, toast).unwrap());
        let slot = slot_of(&context, toast);
        let scale = |context: &AppContext, now| match context.world().presentation_motion_value(
            slot,
            AnimatableProperty::Transform,
            now,
        ) {
            Some(MotionValue::Transform(transform)) => transform.a,
            other => panic!("transform of {slot:?}: {other:?}"),
        };
        let early = scale(&context, settled + ms(10));
        assert!((0.9..1.0).contains(&early), "starts small: {early}");
        assert_eq!(
            context.world().presentation_motion_value(
                slot,
                AnimatableProperty::Opacity,
                settled + ms(10)
            ),
            Some(MotionValue::Scalar(1.0)),
            "without fading"
        );
        let rest = settled + Duration::from_secs(1);
        context.advance_animations(rest);
        assert_eq!(scale(&context, rest), 1.0);

        let still = ThemeDefinition::NANA_DARK.with_toast(ToastRecipe::DEFAULT);
        assert!(context.set_theme_definition(&still).unwrap());
        frame(&mut context);
        assert!(!context.pulse_toast(stack, toast).unwrap());
    }

    /// A toast the application despawns itself leaves the stack, and its
    /// empty slot goes the next time the stack fits.
    #[test]
    fn a_toast_despawned_by_the_application_leaves_no_slot_behind() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 3, 400.0);
        let kept = context.push_toast(stack, notice("Kept")).unwrap();
        let gone = context.push_toast(stack, notice("Gone")).unwrap();
        frame(&mut context);
        let slot = slot_of(&context, gone);
        let mut mutations = MutationQueue::new();
        mutations.despawn_subtree(gone.stable_id());
        context.commit_mutations(mutations).unwrap();
        assert_eq!(context.shown_toasts(stack), vec![kept]);
        frame(&mut context);
        assert!(!context.world().contains(slot));
        assert_eq!(
            context
                .world()
                .node(stack.stable_id())
                .unwrap()
                .children
                .to_vec(),
            vec![slot_of(&context, kept)]
        );
    }

    /// The stack is a region named by its label, `toast.region` unless the
    /// stack names it; a toast being measured or waiting is not heard.
    #[test]
    fn the_stack_is_a_named_region_that_speaks_only_what_shows() {
        let mut context = AppContext::new();
        let (stack, _) = window(&mut context, 1, 400.0);
        let waiting = context.push_toast(stack, notice("Waiting")).unwrap();
        let shown = context.push_toast(stack, notice("Shown")).unwrap();
        frame(&mut context);
        let region = context
            .world()
            .project_accessibility(document())
            .into_iter()
            .find(|node| node.id == stack.stable_id())
            .expect("the stack is heard");
        assert_eq!(region.role, AccessibilityRole::Region);
        assert_eq!(region.label.as_deref(), Some("通知"));
        assert!(spoken(&context, shown.stable_id()));
        assert!(!spoken(&context, waiting.stable_id()));
        let measuring = context.push_toast(stack, notice("Measuring")).unwrap();
        assert!(!spoken(&context, measuring.stable_id()));

        context
            .update_component(stack, |stack, _| stack.label = Some(Arc::from("Notices")))
            .unwrap();
        assert_eq!(
            context
                .world()
                .accessibility(stack.stable_id())
                .and_then(|state| state.label.clone())
                .as_deref(),
            Some("Notices")
        );
    }

    /// The stack takes no pointer of its own: beside a narrow toast and in
    /// the gap between two, the press reaches the stage underneath.
    #[test]
    fn the_stack_has_no_hit_area_of_its_own() {
        let mut context = AppContext::new();
        let (stack, stage) = window(&mut context, 3, 400.0);
        let older = context.push_toast(stack, notice("A")).unwrap();
        let newer = context.push_toast(stack, notice("B")).unwrap();
        frame(&mut context);
        let area = bounds(&context, stack.stable_id());
        let top = bounds(&context, slot_of(&context, older));
        let bottom = bounds(&context, slot_of(&context, newer));
        assert!(top.x > area.x + 8.0, "narrow toasts: {top:?} in {area:?}");
        let hit = |x: f32, y: f32| context.world().hit_test(document(), x, y);
        assert_eq!(
            hit(area.x + 4.0, top.y + top.height / 2.0),
            Some(stage.stable_id())
        );
        let gap = (top.y + top.height + bottom.y) / 2.0;
        assert_eq!(hit(top.x + top.width / 2.0, gap), Some(stage.stable_id()));
        assert_eq!(
            hit(top.x + 4.0, top.y + 4.0),
            Some(older.stable_id()),
            "the toast takes its own"
        );
    }
}
