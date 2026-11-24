//! Per-type behavior and lazy capabilities.
//!
//! A type registers its node-level behavior (`PointerOps`, `A11yBehavior`,
//! `OverlayBehavior`, `FocusBehavior`, `post_layout`, `animation_sample`)
//! and context-level [`Capabilities`] (`TypeBehavior::capabilities`) once
//! when its first node is created, so feature-gated or unused controls link
//! none of their code until instantiated.

use std::time::Duration;

use nana_ui_input::{
    CompositionInput, HostServices, InputModifiers, PointerInput, PointerPhase, PointerType,
};

use super::*;
#[allow(unused_imports)]
use crate::*;

type Handled = Result<bool, FrameworkError>;
pub(crate) type NearHit =
    fn(&AppContext, DocumentId, f32, f32, Option<StableNodeId>) -> Option<StableNodeId>;

/// Drag lifecycle callbacks registered on `InputState::active_drags` when a
/// pointer down begins (or arms) a drag, allowing `O(1)` Move/Up/Cancel dispatch.
#[derive(Clone, Copy)]
pub(crate) struct ActiveDragOps {
    pub(crate) pre_gesture: bool,
    pub(crate) cancel_on_touch_scroll: bool,
    pub(crate) update:
        fn(&mut AppContext, DocumentId, StableNodeId, u64, f32, f32, Duration) -> Handled,
    pub(crate) end:
        fn(&mut AppContext, DocumentId, StableNodeId, u64, f32, f32, Duration, bool) -> Handled,
}

/// One in-flight or armed pointer drag.
#[derive(Clone, Copy)]
pub(crate) struct ActiveDrag {
    pub(crate) document: DocumentId,
    pub(crate) pointer_id: u64,
    pub(crate) node: StableNodeId,
    pub(crate) ops: &'static ActiveDragOps,
}

impl std::fmt::Debug for ActiveDrag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveDrag")
            .field("document", &self.document)
            .field("pointer_id", &self.pointer_id)
            .field("node", &self.node)
            .finish()
    }
}

type EntityFn<C, R = bool> = fn(&mut AppContext, Entity<C>) -> Result<R, FrameworkError>;

/// The pointer over, pressed on or captured by a node, in the node's own
/// layout px (its border box's top-left is `0, 0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodePointer {
    pub pointer_id: u64,
    pub x: f32,
    pub y: f32,
    /// The node holds the pointer's capture (a drag it started).
    pub captured: bool,
    pub shift: bool,
    /// The button this press or release is about, numbered like DOM
    /// `PointerEvent.button`: 0 primary, 1 middle, 2 secondary, 3 back,
    /// 4 forward. `-1` when no button changed (moves, leaves).
    pub button: i16,
    /// The buttons held, as DOM `PointerEvent.buttons` bits: 1 primary,
    /// 2 secondary, 4 middle, 8 back, 16 forward.
    pub buttons: u16,
    /// All modifier keys held (`shift` repeats `modifiers.shift`).
    pub modifiers: InputModifiers,
    pub pointer_type: PointerType,
    /// Pen or touch pressure in `0..=1`; a mouse reports 0.5 while a button
    /// is held and 0 otherwise.
    pub pressure: f32,
    /// Pen tilt in degrees, `-90..=90`, 0 for a mouse.
    pub tilt_x: i16,
    pub tilt_y: i16,
    /// Presses of the same button in quick succession at about the same
    /// spot, counting this one: 1 for a single press, 2 for a double click.
    /// Carried by the press and the moves and release after it.
    pub click_count: u8,
}

/// The latest sample's details for a pointer: what [`NodePointer`] carries
/// beyond its position.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PointerDetail {
    buttons: u16,
    pub(super) modifiers: InputModifiers,
    pointer_type: PointerType,
    pressure: f32,
    tilt_x: i16,
    tilt_y: i16,
    click_count: u8,
    /// The last press: when, where and which button (for `click_count`).
    last_press: Option<(Duration, f32, f32, i16)>,
}

/// Presses this close in time and space count as one multi-click.
const MULTI_CLICK_WINDOW: Duration = Duration::from_millis(500);
const MULTI_CLICK_SLOP: f32 = 4.0;

/// A wheel turn over a node, in the node's own layout px. Deltas are px,
/// positive when the content would move up / left (the wheel's own sign).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeWheel {
    pub pointer_id: u64,
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    /// Control or Meta held (the zoom modifier).
    pub ctrl: bool,
    /// All modifier keys held.
    pub modifiers: InputModifiers,
}

/// Pointer handling a component owns over its whole box: hover tracking,
/// drags it captures and wheel turns. Built-ins install it through
/// [`TypeBehavior::pointer`], extensions through
/// `ExtensionRegistrar::register_pointer`.
pub struct NodePointerHooks<C: View> {
    /// The pointer moved over the node, or anywhere while the node holds
    /// its capture.
    pub moved: fn(&mut AppContext, Entity<C>, NodePointer) -> Handled,
    /// The pointer left the node, or the node stopped being reachable, or a
    /// press it captured was cancelled before its release.
    pub left: EntityFn<C, ()>,
    /// A press of the primary, middle or secondary button on the node
    /// ([`NodePointer::button`]). `Ok(true)` captures the pointer for the
    /// node until that button's release and consumes the press; a hook
    /// that only handles the primary button returns `Ok(false)` for the
    /// others.
    pub pressed: fn(&mut AppContext, Entity<C>, NodePointer) -> Handled,
    /// The release of the button whose press the node captured.
    pub released: fn(&mut AppContext, Entity<C>, NodePointer) -> Handled,
    /// A wheel turn over the node. `Ok(false)` lets it scroll whatever is
    /// under the node.
    pub wheel: fn(&mut AppContext, Entity<C>, NodeWheel) -> Handled,
}

/// [`NodePointerHooks`] without its type.
pub(crate) trait ErasedPointer: Sync {
    fn moved(&self, cx: &mut AppContext, id: StableNodeId, pointer: NodePointer) -> Handled;
    fn left(&self, cx: &mut AppContext, id: StableNodeId) -> Result<(), FrameworkError>;
    fn pressed(&self, cx: &mut AppContext, id: StableNodeId, pointer: NodePointer) -> Handled;
    fn released(&self, cx: &mut AppContext, id: StableNodeId, pointer: NodePointer) -> Handled;
    fn wheel(&self, cx: &mut AppContext, id: StableNodeId, wheel: NodeWheel) -> Handled;
}

impl<C: View> ErasedPointer for NodePointerHooks<C> {
    fn moved(&self, cx: &mut AppContext, id: StableNodeId, pointer: NodePointer) -> Handled {
        (self.moved)(cx, Entity::from_stable_id(id), pointer)
    }

    fn left(&self, cx: &mut AppContext, id: StableNodeId) -> Result<(), FrameworkError> {
        (self.left)(cx, Entity::from_stable_id(id))
    }

    fn pressed(&self, cx: &mut AppContext, id: StableNodeId, pointer: NodePointer) -> Handled {
        (self.pressed)(cx, Entity::from_stable_id(id), pointer)
    }

    fn released(&self, cx: &mut AppContext, id: StableNodeId, pointer: NodePointer) -> Handled {
        (self.released)(cx, Entity::from_stable_id(id), pointer)
    }

    fn wheel(&self, cx: &mut AppContext, id: StableNodeId, wheel: NodeWheel) -> Handled {
        (self.wheel)(cx, Entity::from_stable_id(id), wheel)
    }
}

/// What a built-in type brings with its first node
/// ([`ComponentView::BEHAVIOR`]), so a program links only that of the types
/// it creates.
#[doc(hidden)]
pub struct TypeBehavior<C: View> {
    /// Activation: pointer release, Enter, Space. Plugins register theirs
    /// with `ExtensionRegistrar::register_activation`.
    pub activation: Option<EntityFn<C>>,
    /// Assembler of a leaf composite whose children follow purely from its
    /// own props, run after each write. Shell / Workspace / Dock / SplitPane
    /// / PaneSection must not have one: they reconcile application-owned
    /// slots, and assembling on every write would break "an idle projection
    /// does not dirty the world". Callers run their `assemble_*` instead.
    pub assembler: Option<EntityFn<C>>,
    /// Assembler of a composite that places application-owned slots (a
    /// shell's regions). The view layer runs it after it builds such a node
    /// and after a binding changes one, so a view never calls `assemble_*`;
    /// other writes leave it to the caller, as for [`Self::assembler`].
    pub slot_assembler: Option<EntityFn<C>>,
    /// Activation at a point inside inner geometry: an open list's option,
    /// a tree row.
    pub activate_at: Option<fn(&mut AppContext, Entity<C>, f32, f32) -> Handled>,
    /// Close detached options (Escape, a press elsewhere), keeping the
    /// value; answers whether anything closed.
    pub close_options: Option<EntityFn<C>>,
    /// Upkeep after each create or write: a tab strip's options, a
    /// terminal's rows, a workspace's transition.
    pub lifecycle: Option<EntityFn<C, ()>>,
    /// Context-level lazy capabilities registered once when the first node of this type is created.
    pub(crate) capabilities: Option<fn(&mut Capabilities)>,
    /// Text editing, for an editor or a composite with a search field.
    pub editable: Option<&'static super::text_edit::EditableHooks>,
    /// Hover, drags and wheel over the whole node.
    pub pointer: Option<&'static NodePointerHooks<C>>,
    /// Node-level pointer/scroll dispatch table.
    pub(crate) pointer_ops: Option<&'static PointerOps>,
    /// Node-level accessibility action dispatch table.
    pub(crate) a11y: Option<&'static A11yBehavior>,
    /// Node-level modal/menu/tooltip/hover-card overlay behavior.
    pub(crate) overlay: Option<&'static OverlayBehavior>,
    /// Node-level focus settlement/loss behavior.
    pub(crate) focus: Option<&'static FocusBehavior>,
    /// Post-layout adjustment hook for instances of this type.
    pub(crate) post_layout: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
    /// Animation sample / expiry hook for instances of this type.
    pub(crate) animation_sample:
        Option<fn(&mut AppContext, StableNodeId, &crate::AnimationSample, Duration) -> bool>,
    /// Extra projection over framework-owned assembled slots stored on `AppContext`.
    #[doc(hidden)]
    pub slot_projector: Option<fn(&AppContext, StableNodeId, &C, &UiWorld, &mut MutationQueue)>,
}

impl<C: View> TypeBehavior<C> {
    pub const NONE: Self = Self {
        activation: None,
        assembler: None,
        slot_assembler: None,
        activate_at: None,
        close_options: None,
        lifecycle: None,
        capabilities: None,
        editable: None,
        pointer: None,
        pointer_ops: None,
        a11y: None,
        overlay: None,
        focus: None,
        post_layout: None,
        animation_sample: None,
        slot_projector: None,
    };
}

/// [`TypeBehavior`] without its type, per type created so far.
#[derive(Clone, Copy)]
pub(crate) struct ErasedBehavior {
    pub(crate) assembler: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
    pub(crate) slot_assembler: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
    pub(crate) activate_at: Option<fn(&mut AppContext, StableNodeId, f32, f32) -> Handled>,
    pub(crate) close_options: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
    pub(crate) lifecycle: Option<fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>>,
    pub(crate) editable: Option<&'static super::text_edit::EditableHooks>,
    pub(crate) pointer: Option<&'static dyn ErasedPointer>,
    pub(crate) pointer_ops: Option<&'static PointerOps>,
    pub(crate) a11y: Option<&'static A11yBehavior>,
    pub(crate) overlay: Option<&'static OverlayBehavior>,
    pub(crate) focus: Option<&'static FocusBehavior>,
    pub(crate) post_layout: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
    pub(crate) animation_sample:
        Option<fn(&mut AppContext, StableNodeId, &crate::AnimationSample, Duration) -> bool>,
}

impl ErasedBehavior {
    pub(crate) fn of<C: ComponentView>() -> Self {
        let behavior = &C::BEHAVIOR;
        Self {
            assembler: behavior.assembler.map(|_| {
                (|cx: &mut AppContext, node| {
                    (C::BEHAVIOR.assembler.unwrap())(cx, Entity::from_stable_id(node))
                }) as _
            }),
            slot_assembler: behavior.slot_assembler.map(|_| {
                (|cx: &mut AppContext, node| {
                    (C::BEHAVIOR.slot_assembler.unwrap())(cx, Entity::from_stable_id(node))
                }) as _
            }),
            activate_at: behavior.activate_at.map(|_| {
                (|cx: &mut AppContext, node, x, y| {
                    (C::BEHAVIOR.activate_at.unwrap())(cx, Entity::from_stable_id(node), x, y)
                }) as _
            }),
            close_options: behavior.close_options.map(|_| {
                (|cx: &mut AppContext, node| {
                    (C::BEHAVIOR.close_options.unwrap())(cx, Entity::from_stable_id(node))
                }) as _
            }),
            lifecycle: behavior.lifecycle.map(|_| {
                (|cx: &mut AppContext, node| {
                    (C::BEHAVIOR.lifecycle.unwrap())(cx, Entity::from_stable_id(node))
                }) as _
            }),
            editable: behavior.editable,
            pointer: behavior
                .pointer
                .map(|hooks| hooks as &'static dyn ErasedPointer),
            pointer_ops: behavior.pointer_ops,
            a11y: behavior.a11y,
            overlay: behavior.overlay,
            focus: behavior.focus,
            post_layout: behavior.post_layout,
            animation_sample: behavior.animation_sample,
        }
    }
}

impl AppContext {
    /// The behavior of the node's type, if the node has a view.
    pub(crate) fn behavior(&self, id: StableNodeId) -> Option<ErasedBehavior> {
        let type_id = self.view_type_id(id)?;
        self.store.behaviors.get(&type_id).copied()
    }

    /// The node's pointer hooks: its type's, or an extension's.
    pub(crate) fn pointer_hooks(&self, id: StableNodeId) -> Option<&'static dyn ErasedPointer> {
        let type_id = self.view_type_id(id)?;
        self.store
            .behaviors
            .get(&type_id)
            .and_then(|behavior| behavior.pointer)
            .or_else(|| self.store.pointer_extensions.get(&type_id).copied())
    }

    /// Record a pointer sample's details for the [`NodePointer`]s built
    /// from it and after it; a press counts towards `click_count`.
    pub(super) fn note_pointer_detail(
        &mut self,
        document: DocumentId,
        pointer: &PointerInput,
        now: Duration,
    ) {
        let detail = self
            .input
            .pointer_details
            .entry((document, pointer.pointer_id.0))
            .or_insert(PointerDetail {
                buttons: 0,
                modifiers: InputModifiers::empty(),
                pointer_type: pointer.pointer_type,
                pressure: 0.0,
                tilt_x: 0,
                tilt_y: 0,
                click_count: 0,
                last_press: None,
            });
        if pointer.phase == PointerPhase::Down {
            let repeat = detail.last_press.is_some_and(|(at, x, y, button)| {
                button == pointer.button
                    && now.saturating_sub(at) <= MULTI_CLICK_WINDOW
                    && (x - pointer.x).abs() < MULTI_CLICK_SLOP
                    && (y - pointer.y).abs() < MULTI_CLICK_SLOP
            });
            detail.click_count = if repeat {
                detail.click_count.saturating_add(1)
            } else {
                1
            };
            detail.last_press = Some((now, pointer.x, pointer.y, pointer.button));
        }
        detail.buttons = pointer.buttons;
        detail.modifiers = pointer.modifiers;
        detail.pointer_type = pointer.pointer_type;
        detail.pressure = pointer.pressure;
        detail.tilt_x = pointer.tilt_x;
        detail.tilt_y = pointer.tilt_y;
    }

    /// The pointer at `(x, y)` in `id`'s own layout px. `button` is the
    /// button the event is about, `-1` for none.
    pub(crate) fn node_pointer(
        &self,
        id: StableNodeId,
        pointer_id: u64,
        x: f32,
        y: f32,
        shift: bool,
        button: i16,
    ) -> Option<NodePointer> {
        let (x, y) = self.world.pointer_layout_position(id, x, y)?;
        let bounds = self.world.component_layout_box(id)?;
        let document = self.world.document_of(id)?;
        let detail = self.input.pointer_details.get(&(document, pointer_id));
        Some(NodePointer {
            pointer_id,
            x: x - bounds.x,
            y: y - bounds.y,
            captured: self.world.pointer_capture(document, pointer_id) == Some(id),
            shift,
            button,
            buttons: detail.map_or(0, |d| d.buttons),
            modifiers: detail.map_or(
                InputModifiers {
                    shift,
                    ..InputModifiers::empty()
                },
                |d| d.modifiers,
            ),
            pointer_type: detail.map_or(PointerType::Mouse, |d| d.pointer_type),
            pressure: detail.map_or(0.0, |d| d.pressure),
            tilt_x: detail.map_or(0, |d| d.tilt_x),
            tilt_y: detail.map_or(0, |d| d.tilt_y),
            click_count: detail.map_or(0, |d| d.click_count),
        })
    }

    /// A press of `button` on `target`: its pointer hooks may take it, and
    /// with it the pointer's capture until that button's release.
    pub(super) fn press_pointer_hook(
        &mut self,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
        shift: bool,
        button: i16,
    ) -> Handled {
        let Some(hooks) = self.pointer_hooks(target) else {
            return Ok(false);
        };
        let Some(pointer) = self.node_pointer(target, pointer_id, x, y, shift, button) else {
            return Ok(false);
        };
        if !hooks.pressed(self, target, pointer)? {
            return Ok(false);
        }
        if let Some(document) = self.world.document_of(target) {
            self.input
                .hook_buttons
                .insert((document, pointer_id), button);
        }
        let mut mutations = MutationQueue::new();
        mutations.capture_pointer(pointer_id, target);
        self.commit_mutations(mutations)?;
        Ok(true)
    }

    /// A cancelled pointer whose press a node with pointer hooks captured:
    /// the capture ends with no release, and the node hears the pointer
    /// leave once the hover is cleared.
    pub(super) fn cancel_pointer_hook(&mut self, document: DocumentId, pointer_id: u64) -> Handled {
        let Some(owner) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if self.pointer_hooks(owner).is_none() {
            return Ok(false);
        }
        self.input.hook_buttons.remove(&(document, pointer_id));
        let mut mutations = MutationQueue::new();
        mutations.release_pointer(pointer_id, owner);
        self.commit_mutations(mutations)?;
        Ok(true)
    }

    /// The release of `button` when a node with pointer hooks captured its
    /// press. Another button's release leaves the capture alone.
    pub(super) fn release_pointer_hook(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        shift: bool,
        button: i16,
    ) -> Handled {
        let Some(owner) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(hooks) = self.pointer_hooks(owner) else {
            return Ok(false);
        };
        let key = (document, pointer_id);
        if self
            .input
            .hook_buttons
            .get(&key)
            .is_some_and(|b| *b != button)
        {
            return Ok(false);
        }
        self.input.hook_buttons.remove(&key);
        let pointer = self.node_pointer(owner, pointer_id, x, y, shift, button);
        let handled = match pointer {
            Some(pointer) => hooks.released(self, owner, pointer)?,
            None => false,
        };
        let mut mutations = MutationQueue::new();
        mutations.release_pointer(pointer_id, owner);
        self.commit_mutations(mutations)?;
        Ok(handled || pointer.is_some())
    }

    /// Hover moved from `previous` to `target`: the one left hears it, and
    /// whichever node now has the pointer (or its capture) hears where.
    pub(super) fn sync_pointer_hooks(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        previous: Option<StableNodeId>,
        target: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        if previous != target
            && let Some(previous) = previous
            && let Some(hooks) = self.pointer_hooks(previous)
            && self.world.pointer_capture(document, pointer_id) != Some(previous)
        {
            hooks.left(self, previous)?;
        }
        let owner = self
            .world
            .pointer_capture(document, pointer_id)
            .filter(|owner| self.pointer_hooks(*owner).is_some())
            .or(target);
        let Some(owner) = owner else {
            return Ok(());
        };
        let Some(hooks) = self.pointer_hooks(owner) else {
            return Ok(());
        };
        let Some(&(x, y)) = self
            .component_lifecycle
            .pointer_positions
            .get(&(document, pointer_id))
        else {
            return Ok(());
        };
        let shift = self
            .input
            .pointer_details
            .get(&(document, pointer_id))
            .is_some_and(|d| d.modifiers.shift);
        if let Some(pointer) = self.node_pointer(owner, pointer_id, x, y, shift, -1) {
            hooks.moved(self, owner, pointer)?;
        }
        Ok(())
    }
}

/// Per-node pointer, scroll, drag-end and focus-entry dispatch behavior.
#[derive(Clone, Copy, Default)]
pub(crate) struct PointerOps {
    pub(crate) press:
        Option<fn(&mut AppContext, DocumentId, StableNodeId, u64, i16, f32, f32) -> Handled>,
    pub(crate) early_press: bool,
    pub(crate) skips_press_pointer: bool,
    pub(crate) skip_on_activation_click: bool,
    pub(crate) releases_pointer_on_press: bool,
    pub(crate) corner_press: Option<fn(&mut AppContext, u64, StableNodeId, f32, f32) -> Handled>,
    pub(crate) prepare_context_menu:
        Option<fn(&mut AppContext, StableNodeId, f32, f32) -> Result<(), FrameworkError>>,
    pub(crate) select_on_focus_entry:
        Option<fn(&mut AppContext, DocumentId, StableNodeId, Option<(f32, f32)>) -> Handled>,
    pub(crate) column_edge_at: Option<fn(&AppContext, StableNodeId, f32, f32) -> bool>,
    pub(crate) wheel_pre_overlay:
        Option<fn(&mut AppContext, StableNodeId, f32, f32, f32) -> Handled>,
    pub(crate) wheel_post_overlay:
        Option<fn(&mut AppContext, DocumentId, StableNodeId, f32, f32, f32, bool) -> Handled>,
    pub(crate) is_value_range: bool,
    pub(crate) end_value_drag: Option<fn(&mut AppContext, StableNodeId, u64, bool) -> Handled>,
    pub(crate) is_segmented_option: bool,
    pub(crate) segmented_option_tab_stop: Option<fn(&AppContext, StableNodeId) -> bool>,
    pub(crate) focus_segmented_option:
        Option<fn(&mut AppContext, DocumentId, StableNodeId) -> Handled>,
    pub(crate) apply_scroll: Option<fn(&mut AppContext, StableNodeId, ScrollOffset) -> Handled>,
    pub(crate) note_scroll:
        Option<fn(&mut AppContext, StableNodeId, ScrollOffset) -> Result<(), FrameworkError>>,
    pub(crate) hold_scrollbars:
        Option<fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>>,
    pub(crate) minimap_navigate: Option<
        fn(
            &mut AppContext,
            StableNodeId,
            super::text_edit::TextEditorKind,
            u64,
            ScrollOffset,
        ) -> Handled,
    >,
    pub(crate) close_text_atom: Option<fn(&mut AppContext, DocumentId, &str) -> Handled>,
}

impl PointerOps {
    pub(crate) const EMPTY: Self = Self {
        press: None,
        early_press: false,
        skips_press_pointer: false,
        skip_on_activation_click: false,
        releases_pointer_on_press: false,
        corner_press: None,
        prepare_context_menu: None,
        select_on_focus_entry: None,
        column_edge_at: None,
        wheel_pre_overlay: None,
        wheel_post_overlay: None,
        is_value_range: false,
        end_value_drag: None,
        is_segmented_option: false,
        segmented_option_tab_stop: None,
        focus_segmented_option: None,
        apply_scroll: None,
        note_scroll: None,
        hold_scrollbars: None,
        minimap_navigate: None,
        close_text_atom: None,
    };
}

/// Per-node accessibility action dispatch behavior.
#[derive(Clone, Copy, Default)]
pub(crate) struct A11yBehavior {
    pub(crate) activate_context_menu_index:
        Option<fn(&mut AppContext, StableNodeId, usize) -> Handled>,
    pub(crate) focus_context_menu_index:
        Option<fn(&mut AppContext, StableNodeId, usize) -> Handled>,
    pub(crate) activate_select_option:
        Option<fn(&mut AppContext, StableNodeId, u16) -> Result<Option<bool>, FrameworkError>>,
    pub(crate) adjust_range: Option<fn(&mut AppContext, StableNodeId, RangeAdjustment) -> Handled>,
    pub(crate) step: Option<fn(&mut AppContext, StableNodeId, i32) -> Handled>,
    pub(crate) set_expanded: Option<fn(&mut AppContext, StableNodeId, bool) -> Handled>,
    pub(crate) set_text: Option<fn(&mut AppContext, StableNodeId, &str) -> Handled>,
    pub(crate) set_range_value: Option<fn(&mut AppContext, StableNodeId, f64) -> Handled>,
}

impl A11yBehavior {
    pub(crate) const EMPTY: Self = Self {
        activate_context_menu_index: None,
        focus_context_menu_index: None,
        activate_select_option: None,
        adjust_range: None,
        step: None,
        set_expanded: None,
        set_text: None,
        set_range_value: None,
    };
}

/// Per-node overlay/modal/menu/tooltip behavior.
#[derive(Clone, Copy, Default)]
pub(crate) struct OverlayBehavior {
    pub(crate) note_modal_open:
        Option<fn(&mut AppContext, StableNodeId, bool) -> Result<(), FrameworkError>>,
    pub(crate) sync_modal_open:
        Option<fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>>,
    pub(crate) announce_close_request: Option<
        fn(
            &mut AppContext,
            StableNodeId,
            nana_ui_core::DialogCloseTrigger,
        ) -> Result<(), FrameworkError>,
    >,
    pub(crate) announce_confirm_intent: Option<
        fn(&mut AppContext, StableNodeId, crate::ConfirmIntent) -> Result<(), FrameworkError>,
    >,
    pub(crate) note_anchored_menu_dismissed:
        Option<fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>>,
    pub(crate) dismiss_context_menu: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
    pub(crate) note_tooltip:
        Option<fn(&mut AppContext, StableNodeId, bool) -> Result<(), FrameworkError>>,
    pub(crate) open_hover_card: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
    pub(crate) close_hover_card: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
}

impl OverlayBehavior {
    pub(crate) const EMPTY: Self = Self {
        note_modal_open: None,
        sync_modal_open: None,
        announce_close_request: None,
        announce_confirm_intent: None,
        note_anchored_menu_dismissed: None,
        dismiss_context_menu: None,
        note_tooltip: None,
        open_hover_card: None,
        close_hover_card: None,
    };
}

/// Per-node focus change/settle behavior.
#[derive(Clone, Copy, Default)]
pub(crate) struct FocusBehavior {
    pub(crate) on_blur_settle: Option<fn(&mut AppContext, DocumentId) -> Handled>,
    pub(crate) on_focus_lost: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
}

#[derive(Clone, Copy)]
pub(crate) struct DockCapabilities {
    pub(crate) handle_near_hit: NearHit,
    pub(crate) tab_strip_near_hit: NearHit,
    pub(crate) is_handle: fn(&AppContext, StableNodeId) -> bool,
    pub(crate) is_item_source: fn(&AppContext, StableNodeId) -> bool,
    pub(crate) begin_split: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Handled,
    pub(crate) begin_item: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Handled,
    pub(crate) adjust_focused_split: fn(&mut AppContext, DocumentId, f32) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct WorkspaceCapabilities {
    pub(crate) handle_near_hit: NearHit,
    pub(crate) is_resize_handle: fn(&AppContext, StableNodeId) -> bool,
    pub(crate) begin_resize:
        fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32, Duration) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct SplitPaneCapabilities {
    pub(crate) handle_near_hit: NearHit,
    pub(crate) is_handle: fn(&AppContext, StableNodeId) -> bool,
    pub(crate) begin_resize:
        fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Handled,
    pub(crate) adjust_focused: fn(&mut AppContext, DocumentId, f32) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct ScrollbarCapabilities {
    pub(crate) near_hit: fn(
        &AppContext,
        f32,
        f32,
        Option<StableNodeId>,
    ) -> Option<(StableNodeId, nana_ui_core::ScrollbarAxis)>,
    pub(crate) hot_sync: fn(
        &mut AppContext,
        DocumentId,
        u64,
        Option<(StableNodeId, nana_ui_core::ScrollbarAxis)>,
    ) -> Result<(), FrameworkError>,
    pub(crate) begin_drag:
        fn(&mut AppContext, u64, StableNodeId, nana_ui_core::ScrollbarAxis, f32, f32) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct ReorderListCapabilities {
    pub(crate) begin_pointer:
        fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Handled,
    pub(crate) cancel_drag: fn(&mut AppContext, DocumentId) -> Handled,
    pub(crate) focused_key: fn(&mut AppContext, DocumentId, &str) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct RangeSpanCapabilities {
    pub(crate) target:
        fn(&AppContext, StableNodeId) -> Option<(StableNodeId, Option<RangeSpanThumb>)>,
    pub(crate) begin_drag: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Handled,
    pub(crate) adjust_thumb:
        fn(&mut AppContext, StableNodeId, RangeSpanThumb, RangeAdjustment) -> Handled,
    pub(crate) set_thumb: fn(&mut AppContext, StableNodeId, RangeSpanThumb, f64) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct CurveCapabilities {
    pub(crate) target: fn(&AppContext, StableNodeId) -> Option<(StableNodeId, Option<usize>)>,
    pub(crate) begin_drag: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Handled,
    pub(crate) focus_neighbour: fn(&mut AppContext, DocumentId, bool) -> Handled,
    pub(crate) adjust_point: fn(&mut AppContext, StableNodeId, usize, RangeAdjustment) -> Handled,
    pub(crate) set_point: fn(&mut AppContext, StableNodeId, usize, f64) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct TerminalCapabilities {
    pub(crate) pointer:
        fn(&mut AppContext, DocumentId, Option<StableNodeId>, u64, u8, f32, f32) -> Handled,
    pub(crate) focused_node: fn(&AppContext, DocumentId) -> Option<StableNodeId>,
    pub(crate) accepts_input: fn(&AppContext, DocumentId) -> bool,
    pub(crate) caret_bounds: fn(&AppContext, DocumentId) -> Option<LayoutBox>,
    pub(crate) selected_text: fn(&AppContext, DocumentId) -> Option<String>,
    pub(crate) paste: fn(&mut AppContext, DocumentId, &str) -> Handled,
    pub(crate) key:
        fn(&mut AppContext, DocumentId, &str, Option<&str>, bool, bool, bool) -> Handled,
    pub(crate) set_preedit: fn(&mut AppContext, DocumentId, &str) -> Handled,
    pub(crate) input_bytes: fn(&mut AppContext, DocumentId, Vec<u8>) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct RichEditorCapabilities {
    pub(crate) pointer: fn(
        &mut AppContext,
        DocumentId,
        Option<StableNodeId>,
        u64,
        u8,
        f32,
        f32,
        bool,
        Duration,
    ) -> Handled,
    pub(crate) key: fn(
        &mut AppContext,
        DocumentId,
        &str,
        Option<&str>,
        InputModifiers,
        &mut dyn HostServices,
    ) -> Result<Option<bool>, FrameworkError>,
    pub(crate) composition:
        fn(&mut AppContext, DocumentId, &CompositionInput) -> Result<Option<bool>, FrameworkError>,
}

#[derive(Clone, Copy)]
pub(crate) struct MenuBarCapabilities {
    pub(crate) toggle_keyboard: fn(&mut AppContext, StableNodeId) -> Handled,
    pub(crate) route_key:
        fn(&mut AppContext, DocumentId, &str, bool, bool, InputModifiers) -> Handled,
    pub(crate) cancel_alt: fn(&mut AppContext),
}

#[derive(Clone, Copy)]
pub(crate) struct PopoverCapabilities {
    pub(crate) dismiss_on_escape: fn(&mut AppContext) -> Handled,
    pub(crate) close_within: fn(
        &mut AppContext,
        Option<StableNodeId>,
        Option<StableNodeId>,
        fn(&Popover) -> bool,
    ) -> Handled,
    pub(crate) dismiss_outside: fn(&mut AppContext, Option<StableNodeId>) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct TreeCapabilities {
    pub(crate) focused_key:
        fn(&mut AppContext, DocumentId, &str, nana_ui_core::SelectionModifiers) -> Handled,
    pub(crate) end_rename: fn(&mut AppContext, DocumentId, bool) -> Handled,
    pub(crate) navigate: fn(&mut AppContext, DocumentId, crate::TreeNavigation) -> Handled,
    pub(crate) apply_item_action:
        fn(&mut AppContext, DocumentId, StableNodeId, u16, &crate::AccessibilityAction) -> Handled,
    pub(crate) apply_toggle_action:
        fn(&mut AppContext, StableNodeId, u16, usize, &crate::AccessibilityAction) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct TableCapabilities {
    pub(crate) navigate: fn(&mut AppContext, DocumentId, crate::TableNavigation, usize) -> Handled,
    pub(crate) selection_key:
        fn(&mut AppContext, DocumentId, &str, nana_ui_core::SelectionModifiers) -> Handled,
}

#[derive(Clone, Copy)]
pub(crate) struct ChoiceKeyHooks {
    pub(crate) adjust: fn(&mut AppContext, DocumentId, i32) -> Handled,
    pub(crate) commit: fn(&mut AppContext, DocumentId) -> Handled,
    pub(crate) commit_on_space: bool,
}

#[derive(Default)]
pub(crate) struct Capabilities {
    pub(crate) dock: Option<&'static DockCapabilities>,
    pub(crate) workspace: Option<&'static WorkspaceCapabilities>,
    pub(crate) split_pane: Option<&'static SplitPaneCapabilities>,
    pub(crate) scrollbar: Option<&'static ScrollbarCapabilities>,
    pub(crate) reorder_list: Option<&'static ReorderListCapabilities>,
    pub(crate) range_span: Option<&'static RangeSpanCapabilities>,
    pub(crate) curve_field: Option<&'static CurveCapabilities>,
    pub(crate) terminal: Option<&'static TerminalCapabilities>,
    pub(crate) rich_editor: Option<&'static RichEditorCapabilities>,
    pub(crate) menu_bar: Option<&'static MenuBarCapabilities>,
    pub(crate) popover: Option<&'static PopoverCapabilities>,
    pub(crate) tree: Option<&'static TreeCapabilities>,
    pub(crate) data_table: Option<&'static TableCapabilities>,
    pub(crate) auto_overlay_drive:
        Option<fn(&mut AppContext, StableNodeId, Option<bool>) -> Handled>,
    pub(crate) close_hover_card: Option<fn(&mut AppContext, StableNodeId) -> Handled>,
    pub(crate) set_title_bars_maximized: Option<fn(&mut AppContext, DocumentId, bool)>,
    pub(crate) sync_sidebar_section_hover: Option<
        fn(
            &mut AppContext,
            Option<StableNodeId>,
            Option<StableNodeId>,
        ) -> Result<(), FrameworkError>,
    >,
    pub(crate) sync_row_tools: Option<
        fn(
            &mut AppContext,
            Option<StableNodeId>,
            Option<bool>,
            Option<bool>,
        ) -> Result<(), FrameworkError>,
    >,
    pub(crate) hover_moves:
        Vec<fn(&mut AppContext, DocumentId, Option<StableNodeId>, f32, f32, Duration) -> Handled>,
    pub(crate) hover_clears: Vec<fn(&mut AppContext, DocumentId) -> Handled>,
    pub(crate) post_layout_docs: Vec<fn(&mut AppContext, DocumentId) -> Result<(), FrameworkError>>,
    pub(crate) frame_tickers: Vec<fn(&mut AppContext, Duration) -> Vec<StableNodeId>>,
    pub(crate) open_menu_navigators: Vec<fn(&mut AppContext, DocumentId, &str, bool) -> Handled>,
    pub(crate) choice_hooks: Vec<&'static ChoiceKeyHooks>,
    pub(crate) segmented_navigate:
        Option<fn(&mut AppContext, DocumentId, RovingFocusIntent) -> Handled>,
    pub(crate) timeline_clip_delete: Option<fn(&mut AppContext, DocumentId) -> Handled>,
    pub(crate) image_viewer_step: Option<fn(&mut AppContext, DocumentId, bool) -> Handled>,
    pub(crate) graph_canvas_key: Option<fn(&mut AppContext, DocumentId, &str) -> Handled>,
    pub(crate) command_palette_navigate:
        Option<fn(&mut AppContext, DocumentId, ActionPickerNavigation) -> Handled>,
    pub(crate) step_adjusters: Vec<fn(&mut AppContext, DocumentId, RangeAdjustment) -> Handled>,
    pub(crate) adjust_xy_pad: Option<fn(&mut AppContext, DocumentId, XYPadAdjustment) -> Handled>,
    pub(crate) edit_cancellers: Vec<fn(&mut AppContext, DocumentId) -> Handled>,
    pub(crate) edit_enders: Vec<fn(&mut AppContext, DocumentId) -> Handled>,
    pub(crate) number_step: Option<fn(&mut AppContext, DocumentId, i32) -> Handled>,
    pub(crate) number_submit: Option<fn(&mut AppContext, DocumentId) -> Handled>,
    pub(crate) rich_text_pointer:
        Option<fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Handled>,
}

impl Capabilities {
    #[inline]
    pub(crate) fn push_unique<T: Copy>(list: &mut Vec<T>, item: T, eq: fn(T, T) -> bool) {
        if !list.iter().any(|&existing| eq(existing, item)) {
            list.push(item);
        }
    }

    pub(crate) fn add_hover_move(
        &mut self,
        f: fn(&mut AppContext, DocumentId, Option<StableNodeId>, f32, f32, Duration) -> Handled,
    ) {
        Self::push_unique(&mut self.hover_moves, f, |a, b| a as usize == b as usize);
    }

    pub(crate) fn add_hover_clear(&mut self, f: fn(&mut AppContext, DocumentId) -> Handled) {
        Self::push_unique(&mut self.hover_clears, f, |a, b| a as usize == b as usize);
    }

    pub(crate) fn add_post_layout_doc(
        &mut self,
        f: fn(&mut AppContext, DocumentId) -> Result<(), FrameworkError>,
    ) {
        Self::push_unique(&mut self.post_layout_docs, f, |a, b| {
            a as usize == b as usize
        });
    }

    pub(crate) fn add_frame_ticker(
        &mut self,
        f: fn(&mut AppContext, Duration) -> Vec<StableNodeId>,
    ) {
        Self::push_unique(&mut self.frame_tickers, f, |a, b| a as usize == b as usize);
    }

    pub(crate) fn add_open_menu_navigator(
        &mut self,
        f: fn(&mut AppContext, DocumentId, &str, bool) -> Handled,
    ) {
        Self::push_unique(&mut self.open_menu_navigators, f, |a, b| {
            a as usize == b as usize
        });
    }

    pub(crate) fn add_choice_hook(&mut self, hook: &'static ChoiceKeyHooks) {
        Self::push_unique(&mut self.choice_hooks, hook, |a, b| std::ptr::eq(a, b));
    }

    pub(crate) fn add_step_adjuster(
        &mut self,
        f: fn(&mut AppContext, DocumentId, RangeAdjustment) -> Handled,
    ) {
        Self::push_unique(&mut self.step_adjusters, f, |a, b| a as usize == b as usize);
    }

    pub(crate) fn add_edit_canceller(&mut self, f: fn(&mut AppContext, DocumentId) -> Handled) {
        Self::push_unique(&mut self.edit_cancellers, f, |a, b| {
            a as usize == b as usize
        });
    }

    pub(crate) fn add_edit_ender(&mut self, f: fn(&mut AppContext, DocumentId) -> Handled) {
        Self::push_unique(&mut self.edit_enders, f, |a, b| a as usize == b as usize);
    }
}
