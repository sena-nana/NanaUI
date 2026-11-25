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
