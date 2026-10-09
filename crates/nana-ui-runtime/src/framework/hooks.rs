//! Per-type hooks: behavior the input router reaches only through a type
//! that needs it.
//!
//! A type installs its hooks with its first node
//! ([`TypeBehavior::hooks`]), so a program that never creates the
//! type links none of the code behind them. Until then a router call does
//! what it always did for a document without such a node: nothing
//! ([`Absent`]), so the order and outcome of the routing chain are unchanged.

use std::time::Duration;

use super::*;

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
}

/// A wheel turn over a node, in the node's own layout px. Deltas are px,
/// positive when the content would move up / left (the wheel's own sign).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeWheel {
    pub pointer_id: u64,
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub ctrl: bool,
}

/// Pointer handling a component owns over its whole box: hover tracking,
/// drags it captures and wheel turns. Built-ins install it through
/// [`TypeBehavior::pointer`], extensions through
/// `ExtensionRegistrar::register_pointer`.
pub struct NodePointerHooks<C: View> {
    /// The pointer moved over the node, or anywhere while the node holds
    /// its capture.
    pub moved: fn(&mut AppContext, Entity<C>, NodePointer) -> Handled,
    /// The pointer left the node, or the node stopped being reachable.
    pub left: EntityFn<C, ()>,
    /// A primary press on the node. `Ok(true)` captures the pointer for the
    /// node until the release and consumes the press.
    pub pressed: fn(&mut AppContext, Entity<C>, NodePointer) -> Handled,
    /// The release of a press the node captured.
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
        let view = self.views.get(&id)?;
        self.behaviors.get(&view.as_ref().type_id()).copied()
    }

    /// The node's pointer hooks: its type's, or an extension's.
    pub(crate) fn pointer_hooks(&self, id: StableNodeId) -> Option<&'static dyn ErasedPointer> {
        let view = self.views.get(&id)?;
        let type_id = view.as_ref().type_id();
        self.behaviors
            .get(&type_id)
            .and_then(|behavior| behavior.pointer)
            .or_else(|| self.pointer_extensions.get(&type_id).copied())
    }

    /// The pointer at `(x, y)` in `id`'s own layout px.
    pub(crate) fn node_pointer(
        &self,
        id: StableNodeId,
        pointer_id: u64,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Option<NodePointer> {
        let (x, y) = self.world.pointer_layout_position(id, x, y)?;
        let bounds = self.world.component_layout_box(id)?;
        let document = self.world.document_of(id)?;
        Some(NodePointer {
            pointer_id,
            x: x - bounds.x,
            y: y - bounds.y,
            captured: self.world.pointer_capture(document, pointer_id) == Some(id),
            shift,
        })
    }

    /// A primary press on `target`: its pointer hooks may take it, and with
    /// it the pointer's capture until the release.
    pub(super) fn press_pointer_hook(
        &mut self,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Handled {
        let Some(hooks) = self.pointer_hooks(target) else {
            return Ok(false);
        };
        let Some(pointer) = self.node_pointer(target, pointer_id, x, y, shift) else {
            return Ok(false);
        };
        if !hooks.pressed(self, target, pointer)? {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.capture_pointer(pointer_id, target);
        self.commit_mutations(mutations)?;
        Ok(true)
    }

    /// The release of a press a node with pointer hooks captured.
    pub(super) fn release_pointer_hook(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Handled {
        let Some(owner) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let Some(hooks) = self.pointer_hooks(owner) else {
            return Ok(false);
        };
        let pointer = self.node_pointer(owner, pointer_id, x, y, shift);
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
        if let Some(pointer) = self.node_pointer(owner, pointer_id, x, y, false) {
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
}

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
