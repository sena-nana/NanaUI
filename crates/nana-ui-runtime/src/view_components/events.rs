use std::path::PathBuf;
use std::sync::Arc;

use crate::{RovingEdge, ScrollOffset, StableNodeId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activate;

/// A request for a node's context menu: a secondary (right) pointer press,
/// or its keyboard equivalent, delivered to the nearest handler at or above
/// the target.
///
/// The framework opens nothing: whether a menu appears, and what is in it, is
/// the application's call. For a pointer press `target` is the node actually
/// hit, which may be a descendant of the handler's own node, and `x` / `y`
/// are the window point.
///
/// The keyboard raises it on the focused node, with `keyboard` set and `x` /
/// `y` at the centre of that node's box: the `ContextMenu` key or Shift+F10
/// on any focused node, and ArrowUp / ArrowDown on a focused popup trigger
/// ([`AccessibilityState::has_popup`]), whenever the focused control did not
/// claim the key itself. An application that opens a menu for a keyboard
/// request moves focus into it; one opened by the pointer leaves focus where
/// it is.
///
/// `focus` says where that focus goes when the key named it, after the
/// WAI-ARIA menu button: ArrowDown asks for the first item
/// ([`RovingEdge::Start`]), ArrowUp for the last ([`RovingEdge::End`]). The
/// `ContextMenu` key, Shift+F10 and a host's own gesture leave it `None` and
/// the application picks (usually the first item); a pointer press always
/// leaves it `None`. [`crate::AppContext::focus_roving_edge`] focuses that
/// end of the menu's [`crate::RovingFocusGroup`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SecondaryPress {
    pub target: StableNodeId,
    pub x: f32,
    pub y: f32,
    /// Raised by the keyboard rather than a pointer button.
    pub keyboard: bool,
    /// The end of the menu the key asked to focus: `Start` for ArrowDown,
    /// `End` for ArrowUp on a popup trigger; `None` otherwise.
    pub focus: Option<RovingEdge>,
}

/// The pointer entered (`hovered`) or left this node's subtree.
///
/// Only nodes with a handler for it receive it, innermost first, so a row can
/// reveal its own tools on hover without becoming a component. Moving between
/// two descendants of the same node does not repeat it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerHoverChanged {
    pub hovered: bool,
}

/// The pointer moved over this node's subtree, at `x` / `y` in the node's
/// own layout px (its border box's top-left is `0, 0`).
///
/// Only nodes with a handler for it receive it, innermost first, the same
/// nodes [`PointerHoverChanged`] calls hovered; leaving is told by that event
/// alone. While a node holds the pointer's capture (a drag), the nodes
/// enclosing it keep hearing moves, and the point may lie outside them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerMoved {
    pub x: f32,
    pub y: f32,
}

/// Where a [`PointerDragged`] gesture is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerDragPhase {
    /// The press moved past the slop: the node holds the pointer's capture
    /// (or, when the node takes no pointer itself, the control the press
    /// landed on does), and that control is no longer pressed and will not
    /// activate. Carries the press point, with no delta.
    Started,
    /// The pointer moved while the drag holds it.
    Moved,
    /// The button was released.
    Ended,
    /// The press was cancelled or the node lost the capture; nothing the
    /// drag did should be kept as if it were finished.
    Cancelled,
}

/// A primary-button drag that began in this node's subtree.
///
/// Only the innermost node with a handler for it under the press hears it. A
/// press that never travels past the slop is an ordinary press: it focuses,
/// activates and opens what it always did, and raises nothing. One that does
/// becomes this node's drag: `Started`, a `Moved` per sample (the crossing
/// sample included), then `Ended` on release or `Cancelled`. A press another
/// control already took for a gesture of its own (a slider, a text caret, a
/// scrollbar) does not become a drag.
///
/// `x` / `y` are in the node's own layout px and may lie outside it while the
/// drag holds the pointer; `dx` / `dy` are the travel from the press point
/// in logical px as the window measures it, so a node that moves while it is
/// dragged (a handle following its value) does not change them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerDragged {
    pub phase: PointerDragPhase,
    pub x: f32,
    pub y: f32,
    pub dx: f32,
    pub dy: f32,
    pub modifiers: nana_ui_input::InputModifiers,
}

/// Keyboard focus entered (`focused`) or left this node's subtree.
///
/// Only nodes with a handler for it receive it, innermost first. Moving focus
/// between two descendants of the same node does not repeat it. A row that
/// reveals its tools on [`PointerHoverChanged`] reveals them on this too, so
/// Tab can reach them. A listener being updated when focus moves hears it
/// once its update finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusWithinChanged {
    pub focused: bool,
}

/// Platform file drag resolved onto a registered drop target.
///
/// The framework picks the innermost accepting node and paints hover chrome.
/// Opening, attaching, or rejecting the paths stays the application's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDropEvent {
    Hovered {
        paths: Arc<[PathBuf]>,
        effect: nana_ui_core::DropEffect,
    },
    Dropped {
        paths: Arc<[PathBuf]>,
        effect: nana_ui_core::DropEffect,
    },
    Left,
}

/// Enter submission from an editable single-line input, excluding IME preedit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSubmitted {
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChanged {
    pub value: crate::TextValue,
    pub selection: crate::TextSelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCellFocused {
    pub row: usize,
    pub column: usize,
    pub cell: StableNodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleChanged {
    pub checked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxes {
    Horizontal,
    Vertical,
    Both,
}

impl ScrollAxes {
    pub fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    pub fn vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    pub fn covers(self, axis: nana_ui_core::ScrollbarAxis) -> bool {
        match axis {
            nana_ui_core::ScrollbarAxis::Horizontal => self.horizontal(),
            nana_ui_core::ScrollbarAxis::Vertical => self.vertical(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollChanged {
    pub offset: crate::ScrollOffset,
}

/// A layout pass finished a `ScrollView` that asked to be told (rows that
/// measure themselves read their laid-out heights now).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollLaidOut;

/// A `Text` with a line clamp now shows all of its lines (`false`) or cut
/// some (`true`): a "more" control appears only while the text is clamped.
/// Sent after text shaping, when the answer changes, and only to texts that
/// listen for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextClamped {
    pub clamped: bool,
}

/// A node's laid-out box changed size: its first layout, a window resize, a
/// sibling that grew. Sent after the layout pass, only when the size changed,
/// and only to nodes that listen for it — the width a container was actually
/// given, for picking a two-column or a stacked arrangement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeChanged {
    pub width: f32,
    pub height: f32,
}

/// A layout pass gave a `ScrollView` a viewport of a new size: the first
/// layout, a window resize. Virtualized content widens or narrows its window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollViewportChanged {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayChanged {
    pub active: Option<StableNodeId>,
}

/// Logical dismissal starts now; the host retains this root until its exit finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayClosing {
    pub root: StableNodeId,
}

/// The user asked a modal surface to close: Escape, a press outside it, or
/// its close button (`trigger`). Emitted on the `Dialog`, `ConfirmDialog` or
/// `Drawer` itself, before its `close_policy` is applied. A policy that lets
/// that gesture close it closes it right after, and its host emits
/// [`OverlayClosing`]; one that does not leaves it open, for the application
/// to close or keep. [`nana_ui_core::DialogClosePolicy::requests_only`] leaves
/// every gesture to the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogCloseRequested {
    pub trigger: nana_ui_core::DialogCloseTrigger,
}

/// The host opened or closed a modal surface, and its `open` now says so.
/// Emitted on the `Dialog`, `ConfirmDialog` or `Drawer` itself when the
/// host made the change, not the application's own write of `open`: a close
/// gesture its policy allows, `dismiss_overlay`, `activate_overlay`, another
/// overlay of the host opening in its place, or the surface being parked. A
/// view's `.model(signal)` writes it back into the signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogToggled {
    pub open: bool,
}

/// Reported after the committed numeric value changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberChanged {
    pub value: f64,
}

/// Every value the range shows, including a drag or keyboard step in flight.
/// Observe this for live previews.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeInput {
    pub value: f64,
}

/// A pointer drag on the range began (`true`) or ended (`false`), after the
/// [`RangeChanged`] a release commits, or the [`RangeInput`] a cancel
/// restores. Keyboard and accessibility steps are not drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeDragging {
    pub dragging: bool,
}

/// A committed value: pointer release, a keyboard step, an accessibility
/// `SetValue` or [`crate::AppContext::set_range_value`]. A drag emits it once,
/// on release, and only when the value moved; a cancelled drag never does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeChanged {
    pub value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeAdjustment {
    Decrement,
    Increment,
    PageDecrement,
    PageIncrement,
    Minimum,
    Maximum,
}

/// A scroll offset changed through wheel or scrollbar input, never layout
/// clamping, anchoring, or programmatic scrolling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UserScroll {
    pub offset: ScrollOffset,
    /// Within two logical pixels of the vertical end in the current layout.
    pub at_end: bool,
}
