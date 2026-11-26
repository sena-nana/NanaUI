//! Focus policy, explicit directional neighbors, directional scopes, and
//! focus navigation diagnostics.

use crate::StableNodeId;
use nana_ui_input::FocusDirection;

/// Controls which input modalities can move focus onto a node.
///
/// Nodes without an explicit `FocusPolicy` inherit their default from
/// [`InteractionState::focusable`](crate::InteractionState::focusable):
/// `true` maps to [`FocusPolicy::Strong`], `false` maps to [`FocusPolicy::None`].
/// Disabled nodes always resolve to [`FocusPolicy::None`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FocusPolicy {
    /// Cannot receive focus from any source.
    #[default]
    None,
    /// Focusable only via pointer interaction or programmatic focus requests.
    Click,
    /// Focusable only via sequential (`Tab`), directional, or programmatic focus.
    Tab,
    /// Focusable via pointer, sequential (`Tab`), directional, and programmatic focus.
    Strong,
    /// Focusable only via explicit programmatic requests (`focus_node` / `RequestFocus`).
    Programmatic,
}

impl FocusPolicy {
    #[inline]
    pub const fn allows_programmatic(self) -> bool {
        !matches!(self, Self::None)
    }

    #[inline]
    pub const fn allows_pointer(self) -> bool {
        matches!(self, Self::Click | Self::Strong)
    }

    #[inline]
    pub const fn allows_keyboard(self) -> bool {
        matches!(self, Self::Tab | Self::Strong)
    }

    #[inline]
    pub const fn allows_directional(self) -> bool {
        matches!(self, Self::Tab | Self::Strong)
    }
}

/// Explicit 4-way directional focus neighbors for a node.
///
/// Configured neighbors take precedence over 2D geometric spatial fallback.
/// If a configured neighbor is missing, disabled, hidden, inert, or outside
/// the active modal / directional boundary, navigation follows its outgoing
/// neighbor in the same direction (with cycle detection) before falling back to
/// 2D geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FocusNeighbors {
    pub up: Option<StableNodeId>,
    pub down: Option<StableNodeId>,
    pub left: Option<StableNodeId>,
    pub right: Option<StableNodeId>,
}

impl FocusNeighbors {
    pub const fn new() -> Self {
        Self {
            up: None,
            down: None,
            left: None,
            right: None,
        }
    }

    pub const fn up(mut self, target: StableNodeId) -> Self {
        self.up = Some(target);
        self
    }

    pub const fn down(mut self, target: StableNodeId) -> Self {
        self.down = Some(target);
        self
    }

    pub const fn left(mut self, target: StableNodeId) -> Self {
        self.left = Some(target);
        self
    }

    pub const fn right(mut self, target: StableNodeId) -> Self {
        self.right = Some(target);
        self
    }

    #[inline]
    pub const fn get(&self, direction: FocusDirection) -> Option<StableNodeId> {
        match direction {
            FocusDirection::Up => self.up,
            FocusDirection::Down => self.down,
            FocusDirection::Left => self.left,
            FocusDirection::Right => self.right,
        }
    }

    #[inline]
    pub fn set(&mut self, direction: FocusDirection, target: Option<StableNodeId>) {
        match direction {
            FocusDirection::Up => self.up = target,
            FocusDirection::Down => self.down = target,
            FocusDirection::Left => self.left = target,
            FocusDirection::Right => self.right = target,
        }
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.up.is_none() && self.down.is_none() && self.left.is_none() && self.right.is_none()
    }
}

/// Declares a container as participating in keyboard 2D directional focus
/// navigation (`ArrowUp`/`ArrowDown`/`ArrowLeft`/`ArrowRight`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DirectionalFocusScope {
    /// When `true`, 2D directional navigation stays within this subtree.
    pub contain: bool,
}

impl DirectionalFocusScope {
    pub const fn new() -> Self {
        Self { contain: false }
    }

    pub const fn contained() -> Self {
        Self { contain: true }
    }

    pub const fn contain(mut self, contain: bool) -> Self {
        self.contain = contain;
        self
    }
}

/// Programmatic focus containment scope rooted at any container node,
/// independent of [`Dialog`](crate::Dialog) or [`OverlayHost`](crate::OverlayHost).
///
/// Use [`FocusScope::trap`] to keep sequential (`Tab` / `Shift+Tab`) and
/// directional arrow navigation within `root` (for guided tours, floating
/// panels, inline wizards, or cardless modal sheets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FocusScope {
    /// Root node whose mounted descendants participate in this focus scope.
    pub root: StableNodeId,
    /// Whether sequential and directional focus navigation is trapped inside `root`.
    pub trap: bool,
    /// Whether deactivating this scope should restore the previously focused node.
    pub restore_focus: bool,
}

impl FocusScope {
    /// Create a non-trapping focus scope rooted at `root`.
    pub const fn new(root: StableNodeId) -> Self {
        Self {
            root,
            trap: false,
            restore_focus: true,
        }
    }

    /// Enable or disable programmatic focus trapping inside `self.root`.
    pub const fn trap(mut self, trap: bool) -> Self {
        self.trap = trap;
        self
    }

    /// Control whether releasing the scope restores the node focused when the
    /// scope was entered.
    pub const fn restore_focus(mut self, restore_focus: bool) -> Self {
        self.restore_focus = restore_focus;
        self
    }

    #[inline]
    pub const fn is_trapped(self) -> bool {
        self.trap
    }
}

/// Cumulative focus and directional navigation diagnostic counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FocusCounters {
    pub focus_changes: u64,
    pub trap_enters: u64,
    pub trap_exits: u64,
    pub traps_active: u64,
    pub navigation_queries: u64,
    pub navigation_candidates_considered: u64,
    pub spatial_index_builds: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, Button, DocumentId, LayoutViewport, Stack};

    #[test]
    fn focus_scope_trap_contains_tab_and_restores_focus_without_dialog() {
        let doc = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx.create_component(doc, Stack::column(0.0)).unwrap();
        let outside = cx.create_component(doc, Button::new("Outside")).unwrap();
        let container = cx.create_component(doc, Stack::column(0.0)).unwrap();
        let inside_a = cx.create_component(doc, Button::new("Inside A")).unwrap();
        let inside_b = cx.create_component(doc, Button::new("Inside B")).unwrap();

        cx.append_child(root, outside).unwrap();
        cx.append_child(root, container).unwrap();
        cx.append_child(container, inside_a).unwrap();
        cx.append_child(container, inside_b).unwrap();
        cx.layout_document(doc, LayoutViewport::new(400.0, 300.0))
            .unwrap();

        cx.focus_node(doc, outside.stable_id()).unwrap();
        assert_eq!(cx.world().focused(doc), Some(outside.stable_id()));

        let scope = FocusScope::new(container.stable_id()).trap(true);
        let prev = cx.enter_focus_scope(doc, scope).unwrap();
        assert_eq!(prev, Some(outside.stable_id()));
        assert_eq!(cx.world().focused(doc), Some(inside_a.stable_id()));

        // Forward Tab stays trapped inside container (A -> B -> A).
        assert!(cx.navigate_focus_scope(doc, scope, false).unwrap());
        assert_eq!(cx.world().focused(doc), Some(inside_b.stable_id()));
        assert!(cx.navigate_focus_scope(doc, scope, false).unwrap());
        assert_eq!(cx.world().focused(doc), Some(inside_a.stable_id()));

        // Reverse Shift+Tab wraps A -> B without escaping to `outside`.
        assert!(cx.navigate_focus_scope(doc, scope, true).unwrap());
        assert_eq!(cx.world().focused(doc), Some(inside_b.stable_id()));

        // `focus_last_in` targets B.
        cx.focus_node(doc, inside_a.stable_id()).unwrap();
        assert!(cx.focus_last_in(doc, container.stable_id()).unwrap());
        assert_eq!(cx.world().focused(doc), Some(inside_b.stable_id()));

        // Exiting the scope restores focus to `outside`.
        cx.exit_focus_scope(doc, scope, prev).unwrap();
        assert_eq!(cx.world().focused(doc), Some(outside.stable_id()));
    }
}
