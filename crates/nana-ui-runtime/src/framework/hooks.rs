//! Per-type hooks: behavior the input router reaches only through a type
//! that needs it.
//!
//! A type installs its hooks with its first node
//! ([`TypeBehavior::hooks`]), so a program that never creates the
//! type links none of the code behind them. Until then a router call does
//! what it always did for a document without such a node: nothing
//! ([`Absent`]), so the order and outcome of the routing chain are unchanged.

use std::time::Duration;

use nana_ui_input::{
    CompositionInput, HostServices, InputModifiers, PointerInput, PointerPhase, PointerType,
};

use super::*;
#[allow(unused_imports)]
use crate::*;

type Handled = Result<bool, FrameworkError>;
type NearHit = fn(&AppContext, DocumentId, f32, f32, Option<StableNodeId>) -> Option<StableNodeId>;

/// Dock split handles and item drags.
pub(crate) struct DockHooks {
    pub(crate) handle_near_hit: NearHit,
    pub(crate) tab_strip_near_hit: NearHit,
    pub(crate) is_handle: fn(&AppContext, StableNodeId) -> bool,
    pub(crate) is_item_source: fn(&AppContext, StableNodeId) -> bool,
    pub(crate) begin_split: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Handled,
    pub(crate) update_split: fn(&mut AppContext, DocumentId, u64, f32, f32) -> Handled,
    pub(crate) end_split: fn(&mut AppContext, DocumentId, u64, bool) -> Handled,
    pub(crate) begin_item: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Handled,
    pub(crate) update_item: fn(&mut AppContext, DocumentId, u64, f32, f32) -> Handled,
    pub(crate) end_item: fn(&mut AppContext, DocumentId, u64, f32, f32, bool) -> Handled,
    pub(crate) adjust_focused_split: fn(&mut AppContext, DocumentId, f32) -> Handled,
}

/// Workspace region resize handles.
pub(crate) struct WorkspaceHooks {
    pub(crate) handle_near_hit: NearHit,
    pub(crate) is_resize_handle: fn(&AppContext, StableNodeId) -> bool,
    pub(crate) begin_resize:
        fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32, Duration) -> Handled,
    pub(crate) update_resize: fn(&mut AppContext, DocumentId, u64, f32, f32, Duration) -> Handled,
    pub(crate) end_resize: fn(&mut AppContext, DocumentId, u64, Duration) -> Handled,
}

/// A focused option list's keyboard: move the highlight, commit it.
pub(crate) struct ChoiceHooks {
    pub(crate) adjust: fn(&mut AppContext, DocumentId, i32) -> Handled,
    pub(crate) commit: fn(&mut AppContext, DocumentId) -> Handled,
}

/// A focused picker's keyboard navigation.
pub(crate) struct NavigateHooks<N: 'static> {
    pub(crate) navigate: fn(&mut AppContext, DocumentId, N) -> Handled,
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
    modifiers: InputModifiers,
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
    /// The router hooks this type needs.
    pub hooks: Option<fn(&mut TypeHooks)>,
    /// Text editing, for an editor or a composite with a search field.
    pub editable: Option<&'static super::text_edit::EditableHooks>,
    /// Hover, drags and wheel over the whole node.
    pub pointer: Option<&'static NodePointerHooks<C>>,
}

impl<C: View> TypeBehavior<C> {
    pub const NONE: Self = Self {
        activation: None,
        assembler: None,
        slot_assembler: None,
        activate_at: None,
        close_options: None,
        lifecycle: None,
        hooks: None,
        editable: None,
        pointer: None,
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
        }
    }
}

impl AppContext {
    /// The behavior of the node's type, if the node has a view.
    pub(crate) fn behavior(&self, id: StableNodeId) -> Option<ErasedBehavior> {
        let type_id = self.view_type_id(id)?;
        self.behaviors.get(&type_id).copied()
    }

    /// The node's pointer hooks: its type's, or an extension's.
    pub(crate) fn pointer_hooks(&self, id: StableNodeId) -> Option<&'static dyn ErasedPointer> {
        let type_id = self.view_type_id(id)?;
        self.behaviors
            .get(&type_id)
            .and_then(|behavior| behavior.pointer)
            .or_else(|| self.pointer_extensions.get(&type_id).copied())
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

/// The hooks installed so far. See the module docs.
#[doc(hidden)]
#[derive(Clone, Copy, Default)]
pub struct TypeHooks {
    pub(crate) dock: Option<&'static DockHooks>,
    pub(crate) workspace: Option<&'static WorkspaceHooks>,
    pub(crate) select: Option<&'static ChoiceHooks>,
    pub(crate) dropdown: Option<&'static ChoiceHooks>,
    pub(crate) search_dropdown: Option<&'static ChoiceHooks>,
    pub(crate) command_palette: Option<&'static NavigateHooks<ActionPickerNavigation>>,
    pub(crate) tree: Option<&'static NavigateHooks<crate::TreeNavigation>>,
    pub(crate) calls: RoutedCalls,
}

/// Declares [`RoutedCalls`]: a field per call, and the calls' names for the
/// check that each has a type to bring it.
macro_rules! routed_calls {
    ($($name:ident: $call:ty,)*) => {
        /// Calls the router makes into one type's own handling, each reached
        /// through the type that needs it ([`via`]). A field is named after
        /// the `AppContext` method it stands for and holds that method once a
        /// type whose nodes it serves has one; until then the call answers
        /// [`Absent`], which is what the method answers for a document
        /// without such a node.
        #[doc(hidden)]
        #[derive(Clone, Copy, Default)]
        pub struct RoutedCalls {
            $(pub(crate) $name: Option<$call>,)*
        }

        impl RoutedCalls {
            /// Every call, by the name of its method.
            #[cfg(test)]
            pub(crate) const NAMES: &'static [&'static str] = &[$(stringify!($name)),*];

            /// The calls a type has brought so far.
            #[cfg(test)]
            pub(crate) fn installed(&self) -> Vec<&'static str> {
                let mut installed = Vec::new();
                $(if self.$name.is_some() {
                    installed.push(stringify!($name));
                })*
                installed
            }
        }
    };
}

routed_calls! {
    set_terminal_preedit: fn(&mut AppContext, DocumentId, &str) -> Result<bool, FrameworkError>,
    focused_terminal: fn(&AppContext, DocumentId) -> Option<Entity<TerminalView>>,
    terminal_pointer: fn(&mut AppContext, DocumentId, Option<StableNodeId>, u64, u8, f32, f32) -> Result<bool, FrameworkError>,
    terminal_key: fn(&mut AppContext, DocumentId, &str, Option<&str>, bool, bool, bool) -> Result<bool, FrameworkError>,
    terminal_input: fn(&mut AppContext, DocumentId, Vec<u8>) -> Result<bool, FrameworkError>,
    paste_terminal: fn(&mut AppContext, DocumentId, &str) -> Result<bool, FrameworkError>,
    resize_terminal_view: fn(&mut AppContext, Entity<TerminalView>, f32, f32) -> Result<bool, FrameworkError>,
    end_xy_pad_drag: fn(&mut AppContext, DocumentId, u64, bool) -> Result<bool, FrameworkError>,
    update_xy_pad_drag: fn(&mut AppContext, DocumentId, u64, f32, f32, bool) -> Result<bool, FrameworkError>,
    begin_xy_pad_drag: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Result<bool, FrameworkError>,
    is_xy_pad: fn(&AppContext, StableNodeId) -> bool,
    adjust_focused_xy_pad: fn(&mut AppContext, crate::DocumentId, XYPadAdjustment) -> Result<bool, FrameworkError>,
    end_xy_pad_drag_on: fn(&mut AppContext, StableNodeId, u64, bool) -> Result<bool, FrameworkError>,
    end_range_drag: fn(&mut AppContext, DocumentId, u64, bool) -> Result<bool, FrameworkError>,
    update_range_drag_at: fn(&mut AppContext, DocumentId, u64, f32, f32, bool) -> Result<bool, FrameworkError>,
    begin_range_drag_at: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Result<bool, FrameworkError>,
    adjust_focused_range: fn(&mut AppContext, DocumentId, RangeAdjustment) -> Result<bool, FrameworkError>,
    is_range_field: fn(&AppContext, StableNodeId) -> bool,
    end_focused_range_edit: fn(&mut AppContext, DocumentId) -> Result<bool, FrameworkError>,
    end_range_drag_on: fn(&mut AppContext, StableNodeId, u64, bool) -> Result<bool, FrameworkError>,
    adjust_range: fn(&mut AppContext, Entity<RangeField>, RangeAdjustment) -> Result<bool, FrameworkError>,
    set_range_value: fn(&mut AppContext, Entity<RangeField>, f64) -> Result<bool, FrameworkError>,
    step_focused_number_input: fn(&mut AppContext, DocumentId, i32) -> Result<bool, FrameworkError>,
    revert_focused_number_input: fn(&mut AppContext, DocumentId) -> Result<bool, FrameworkError>,
    press_number_stepper: fn(&mut AppContext, StableNodeId, f32, f32) -> Result<bool, FrameworkError>,
    commit_focused_number_input: fn(&mut AppContext, DocumentId) -> Result<bool, FrameworkError>,
    submit_focused_number_input: fn(&mut AppContext, DocumentId) -> Result<bool, FrameworkError>,
    cancel_focused_number_input: fn(&mut AppContext, DocumentId) -> Result<bool, FrameworkError>,
    select_focused_number_on_entry:
        fn(&mut AppContext, DocumentId, StableNodeId, Option<(f32, f32)>) -> Result<bool, FrameworkError>,
    settle_focused_number_input: fn(&mut AppContext, DocumentId) -> Result<bool, FrameworkError>,
    step_number_input: fn(&mut AppContext, Entity<NumberInput>, i32) -> Result<bool, FrameworkError>,
    set_number_text: fn(&mut AppContext, Entity<NumberInput>, &str) -> Result<bool, FrameworkError>,
    end_split_resize: fn(&mut AppContext, crate::DocumentId, u64, bool) -> Result<bool, FrameworkError>,
    update_split_resize: fn(&mut AppContext, crate::DocumentId, u64, f32, f32) -> Result<bool, FrameworkError>,
    sync_split_handle_hover_near_hit: fn(&mut AppContext, crate::DocumentId, f32, f32, Option<StableNodeId>, std::time::Duration) -> Result<bool, FrameworkError>,
    sync_split_handle_hover: fn(&mut AppContext, crate::DocumentId, Option<StableNodeId>) -> Result<bool, FrameworkError>,
    is_split_handle: fn(&AppContext, StableNodeId) -> bool,
    begin_split_resize: fn(&mut AppContext, crate::DocumentId, u64, StableNodeId, f32, f32) -> Result<bool, FrameworkError>,
    adjust_focused_split: fn(&mut AppContext, crate::DocumentId, f32) -> Result<bool, FrameworkError>,
    end_range_span_drag: fn(&mut AppContext, DocumentId, u64, bool) -> Result<bool, FrameworkError>,
    end_focused_range_span_edit: fn(&mut AppContext, DocumentId) -> Result<bool, FrameworkError>,
    update_range_span_drag_at: fn(&mut AppContext, DocumentId, u64, f32, f32) -> Result<bool, FrameworkError>,
    range_span_target: fn(&AppContext, StableNodeId) -> Option<(Entity<RangeSpanField>, Option<RangeSpanThumb>)>,
    begin_range_span_drag_at: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Result<bool, FrameworkError>,
    adjust_focused_range_span: fn(&mut AppContext, DocumentId, RangeAdjustment) -> Result<bool, FrameworkError>,
    end_range_span_drag_on: fn(&mut AppContext, StableNodeId, u64, bool) -> Result<bool, FrameworkError>,
    adjust_range_span: fn(&mut AppContext, Entity<RangeSpanField>, RangeSpanThumb, RangeAdjustment) -> Result<bool, FrameworkError>,
    set_range_span_thumb: fn(&mut AppContext, Entity<RangeSpanField>, RangeSpanThumb, f64) -> Result<bool, FrameworkError>,
    end_curve_drag: fn(&mut AppContext, DocumentId, u64, bool) -> Result<bool, FrameworkError>,
    update_curve_drag_at: fn(&mut AppContext, DocumentId, u64, f32, f32) -> Result<bool, FrameworkError>,
    curve_target: fn(&AppContext, StableNodeId) -> Option<(Entity<CurveField>, Option<usize>)>,
    begin_curve_drag_at: fn(&mut AppContext, DocumentId, u64, StableNodeId, f32, f32) -> Result<bool, FrameworkError>,
    focus_curve_neighbour: fn(&mut AppContext, DocumentId, bool) -> Result<bool, FrameworkError>,
    adjust_focused_curve: fn(&mut AppContext, DocumentId, RangeAdjustment) -> Result<bool, FrameworkError>,
    end_curve_drag_on: fn(&mut AppContext, StableNodeId, u64, bool) -> Result<bool, FrameworkError>,
    adjust_curve_point: fn(&mut AppContext, Entity<CurveField>, usize, RangeAdjustment) -> Result<bool, FrameworkError>,
    set_curve_point: fn(&mut AppContext, Entity<CurveField>, usize, f64) -> Result<bool, FrameworkError>,
    end_focused_scrub_edit: fn(&mut AppContext, DocumentId) -> Result<bool, FrameworkError>,
    adjust_focused_scrub_field: fn(&mut AppContext, DocumentId, RangeAdjustment) -> Result<bool, FrameworkError>,
    step_scrub_field: fn(&mut AppContext, Entity<ScrubField>, i32) -> Result<bool, FrameworkError>,
    focused_tree_key: fn(&mut AppContext, DocumentId, &str, bool) -> Result<bool, FrameworkError>,
    end_focused_tree_rename: fn(&mut AppContext, DocumentId, bool) -> Result<bool, FrameworkError>,
    apply_tree_item_action: fn(&mut AppContext, DocumentId, StableNodeId, u16, &crate::AccessibilityAction) -> Result<bool, FrameworkError>,
    apply_tree_toggle_action: fn(&mut AppContext, StableNodeId, u16, usize, &crate::AccessibilityAction) -> Result<bool, FrameworkError>,
    request_focused_timeline_clip_delete: fn(&mut AppContext, crate::DocumentId) -> Result<bool, FrameworkError>,
    rich_editor_pointer: fn(&mut AppContext, DocumentId, Option<StableNodeId>, u64, u8, f32, f32, bool, std::time::Duration) -> Result<bool, FrameworkError>,
    rich_editor_key: fn(&mut AppContext, DocumentId, &str, Option<&str>, InputModifiers, &mut dyn HostServices) -> Result<Option<bool>, FrameworkError>,
    rich_editor_composition: fn(&mut AppContext, DocumentId, &CompositionInput) -> Result<Option<bool>, FrameworkError>,
    prepare_rich_editor_context_menu: fn(&mut AppContext, Entity<RichTextEditor>, f32, f32) -> Result<(), FrameworkError>,
    advance_rich_drag: fn(&mut AppContext, std::time::Duration) -> Option<StableNodeId>,
    note_rich_editor_scroll: fn(&mut AppContext, StableNodeId, ScrollOffset) -> Result<(), FrameworkError>,
    navigate_focused_segmented: fn(&mut AppContext, DocumentId, RovingFocusIntent) -> Result<bool, FrameworkError>,
    is_segmented_option_node: fn(&AppContext, StableNodeId) -> bool,
    focus_segmented_option: fn(&mut AppContext, DocumentId, StableNodeId) -> Result<bool, FrameworkError>,
    segmented_option_is_tab_stop: fn(&AppContext, StableNodeId) -> bool,
    navigate_focused_table: fn(&mut AppContext, DocumentId, crate::TableNavigation, usize) -> Result<bool, FrameworkError>,
    end_text_area_resize: fn(&mut AppContext, DocumentId, u64, bool) -> Result<bool, FrameworkError>,
    update_text_area_resize: fn(&mut AppContext, DocumentId, u64, f32, f32) -> Result<bool, FrameworkError>,
    begin_text_area_resize: fn(&mut AppContext, u64, StableNodeId, f32, f32) -> Result<bool, FrameworkError>,
    refresh_text_area_compact_layout: fn(&mut AppContext, StableNodeId) -> Result<bool, FrameworkError>,
    note_text_area_scroll: fn(&mut AppContext, StableNodeId, ScrollOffset) -> Result<(), FrameworkError>,
    text_editor_minimap_navigate: fn(&mut AppContext, StableNodeId, super::text_edit::TextEditorKind, u64, ScrollOffset) -> Result<bool, FrameworkError>,
    close_focused_text_atom: fn(&mut AppContext, DocumentId, &str) -> Result<bool, FrameworkError>,
    note_modal_open: fn(&mut AppContext, StableNodeId, bool) -> Result<(), FrameworkError>,
    sync_modal_open_at: fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>,
    announce_close_request: fn(&mut AppContext, StableNodeId, nana_ui_core::DialogCloseTrigger) -> Result<(), FrameworkError>,
    announce_confirm_intent: fn(&mut AppContext, StableNodeId, crate::ConfirmIntent) -> Result<(), FrameworkError>,
    note_anchored_menu_dismissed: fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>,
    navigate_open_anchored_menu: fn(&mut AppContext, DocumentId, &str) -> Result<bool, FrameworkError>,
    sync_sidebar_section_hover: fn(&mut AppContext, Option<StableNodeId>, Option<StableNodeId>) -> Result<(), FrameworkError>,
    set_sidebar_section_progress: fn(&mut AppContext, StableNodeId, f32) -> bool,
    activate_select_option: fn(&mut AppContext, StableNodeId, u16) -> Result<Option<bool>, FrameworkError>,
    set_dropdown_opened: fn(&mut AppContext, Entity<Dropdown>, bool) -> Result<bool, FrameworkError>,
    close_hover_card: fn(&mut AppContext, StableNodeId) -> Result<bool, FrameworkError>,
    sweep_hover_cards: fn(&mut AppContext),
    open_hover_card: fn(&mut AppContext, StableNodeId) -> Result<bool, FrameworkError>,
    dismiss_popovers_on_escape: fn(&mut AppContext) -> Result<bool, FrameworkError>,
    close_open_popovers_within: fn(&mut AppContext, Option<StableNodeId>, Option<StableNodeId>, fn(&Popover) -> bool,) -> Result<bool, FrameworkError>,
    dismiss_popovers_outside: fn(&mut AppContext, Option<StableNodeId>) -> Result<bool, FrameworkError>,
    navigate_open_action_menu: fn(&mut AppContext, DocumentId, &str) -> Result<bool, FrameworkError>,
    reconcile_toast_stacks: fn(&mut AppContext, DocumentId) -> Result<(), FrameworkError>,
    refresh_settings_row_layout: fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>,
    advance_settings_row_flash: fn(&mut AppContext, StableNodeId, crate::AnimatableProperty, std::time::Duration) -> Result<(), FrameworkError>,
    set_switch_loading_phase: fn(&mut AppContext, StableNodeId, f32) -> bool,
    set_card_loading_phase: fn(&mut AppContext, StableNodeId, f32) -> bool,
    set_spinner_phase: fn(&mut AppContext, StableNodeId, f32) -> bool,
    expire_toast: fn(&mut AppContext, StableNodeId, Duration),
    advance_workspace_animation: fn(&mut AppContext, StableNodeId, Duration) -> bool,
    drive_due_auto_overlays: fn(&mut AppContext) -> Vec<StableNodeId>,
    sync_media_bound_bars: fn(&mut AppContext) -> Vec<StableNodeId>,
    drive_auto_overlay: fn(&mut AppContext, Entity<MediaTransportBar>, Option<bool>) -> Result<bool, FrameworkError>,
    release_scrollbar_holds: fn(&mut AppContext, Duration) -> Vec<StableNodeId>,
    hold_scrollbars: fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>,
    apply_scroll_view_to: fn(&mut AppContext, StableNodeId, ScrollOffset) -> Result<bool, FrameworkError>,
    sync_scrollbar_hot: fn(&mut AppContext, DocumentId, u64, Option<(StableNodeId, nana_ui_core::ScrollbarAxis)>) -> Result<(), FrameworkError>,
    scrollbar_target_near_hit: fn(&AppContext, f32, f32, Option<StableNodeId>) -> Option<(StableNodeId, nana_ui_core::ScrollbarAxis)>,
    begin_scrollbar_drag: fn(&mut AppContext, u64, StableNodeId, nana_ui_core::ScrollbarAxis, f32, f32) -> Result<bool, FrameworkError>,
    update_scrollbar_drag: fn(&mut AppContext, DocumentId, u64, f32, f32) -> Result<bool, FrameworkError>,
    end_scrollbar_drag: fn(&mut AppContext, DocumentId, u64, bool) -> Result<bool, FrameworkError>,
    set_scroll_view_hover: fn(&mut AppContext, StableNodeId, bool) -> Result<(), FrameworkError>,
    settle_menu_aims: fn(&mut AppContext, Duration) -> Vec<StableNodeId>,
    toggle_menu_bar_keyboard: fn(&mut AppContext, Entity<AppMenuBar>) -> Result<bool, FrameworkError>,
    route_menu_bar_key: fn(&mut AppContext, DocumentId, &str, bool, bool, InputModifiers) -> Result<bool, FrameworkError>,
    cancel_menu_bar_alt: fn(&mut AppContext),
    sync_row_tools: fn(&mut AppContext, Option<StableNodeId>, Option<bool>, Option<bool>) -> Result<(), FrameworkError>,
    set_select_opened: fn(&mut AppContext, Entity<Select>, bool) -> Result<bool, FrameworkError>,
    activate_context_menu_index: fn(&mut AppContext, Entity<ContextMenu>, usize) -> Result<bool, FrameworkError>,
    focus_context_menu_index: fn(&mut AppContext, Entity<ContextMenu>, usize) -> Result<bool, FrameworkError>,
    navigate_open_context_menu: fn(&mut AppContext, DocumentId, &str, bool) -> Result<bool, FrameworkError>,
    dismiss_context_menu: fn(&mut AppContext, Entity<ContextMenu>) -> Result<bool, FrameworkError>,
    set_search_dropdown_opened: fn(&mut AppContext, Entity<SearchDropdown>, bool) -> Result<bool, FrameworkError>,
    mark_deferred_settings_groups: fn(&mut AppContext) -> Result<(), FrameworkError>,
    note_icon_button_tooltip: fn(&mut AppContext, StableNodeId, bool) -> Result<(), FrameworkError>,
    set_title_bars_maximized: fn(&mut AppContext, crate::DocumentId, bool),
}

/// Fills fields of [`RoutedCalls`] with the `AppContext` methods of the same
/// names: `route!(hooks; end_split_resize, ...)`.
macro_rules! route {
    ($hooks:expr; $($method:ident),* $(,)?) => {
        $( $hooks.calls.$method = Some(crate::AppContext::$method); )*
    };
}
pub(crate) use route;

/// `via!(self.method(args))`: `AppContext::method` through [`RoutedCalls`],
/// or [`Absent`] while no type has installed it.
macro_rules! via {
    ($cx:ident . $method:ident ( $($arg:expr),* $(,)? )) => {
        match $cx.type_hooks.calls.$method {
            Some(call) => call($cx, $($arg),*),
            None => $crate::framework::hooks::Absent::absent(),
        }
    };
}
pub(crate) use via;

/// What a hooked call answers while its type has no node.
pub(crate) trait Absent {
    fn absent() -> Self;
}

impl Absent for Handled {
    fn absent() -> Self {
        Ok(false)
    }
}

impl<T> Absent for Option<T> {
    fn absent() -> Self {
        None
    }
}

impl Absent for bool {
    fn absent() -> Self {
        false
    }
}

impl Absent for Result<(), FrameworkError> {
    fn absent() -> Self {
        Ok(())
    }
}

impl<T> Absent for Result<Option<T>, FrameworkError> {
    fn absent() -> Self {
        Ok(None)
    }
}

impl Absent for () {
    fn absent() -> Self {}
}

impl<T> Absent for Vec<T> {
    fn absent() -> Self {
        Vec::new()
    }
}

/// Call `$call` with the hook table in `$field` bound to `$hooks`, or answer
/// [`Absent`] when that table is not installed.
macro_rules! hooked {
    ($cx:expr, $field:ident, |$hooks:ident| $call:expr) => {
        match $cx.type_hooks.$field {
            Some($hooks) => $call,
            None => $crate::framework::hooks::Absent::absent(),
        }
    };
}
pub(crate) use hooked;
