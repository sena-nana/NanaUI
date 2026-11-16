//! Pointer gestures a view hears without becoming a component: where the
//! pointer is over a node ([`PointerMoved`]) and a press dragged past the
//! slop ([`PointerDragged`]).
//!
//! Both cost nothing until some node listens for them. A drag takes over a
//! press only once it travels past [`DRAG_SLOP`]: until then the press is the
//! ordinary one, focusing and activating what it always did. From the drag's
//! start the press is released, so the control it began on shows no pressed
//! state and the release activates nothing, and the pointer is captured: by
//! the listening node, or, when that node takes no pointer itself (a layout
//! `Stack` that is not hittable), by the node the press landed on, since
//! routing makes the capture owner the hovered node on every sample.

use std::any::TypeId;

use nana_ui_input::{InputModifiers, PointerType};

use super::super::emit_erased;
use super::super::touch_scroll::TOUCH_SLOP;
use crate::{
    AppContext, DocumentId, FrameworkError, MutationQueue, PointerDragPhase, PointerDragged,
    PointerMoved, StableNodeId,
};

/// How far a mouse or pen press travels before it is a drag, in logical px.
/// A finger uses the touch scrolling slop, so a scroller under it still gets
/// the gesture first.
pub(crate) const DRAG_SLOP: f32 = nana_ui_core::space::XS;

/// Which gesture events have ever had a handler.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GestureListeners {
    pub(crate) moved: bool,
    pub(crate) dragged: bool,
}

/// A primary press on a node listening for [`PointerDragged`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewDrag {
    node: StableNodeId,
    /// What holds the pointer's capture once the drag starts.
    holder: StableNodeId,
    /// The press point, in window coordinates.
    origin: (f32, f32),
    slop: f32,
    started: bool,
}

impl AppContext {
    /// Note a handler for `event`, so routing starts raising it.
    pub(in crate::framework) fn note_gesture_listener(&mut self, event: TypeId) {
        if event == TypeId::of::<PointerMoved>() {
            self.input.gesture_listeners.moved = true;
        } else if event == TypeId::of::<PointerDragged>() {
            self.input.gesture_listeners.dragged = true;
        }
    }

    /// `(x, y)` in `id`'s own layout px, its border box's top-left `0, 0`.
    fn node_local(&self, id: StableNodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        let (x, y) = self.world.pointer_layout_position(id, x, y)?;
        let bounds = self.world.component_layout_box(id)?;
        Some((x - bounds.x, y - bounds.y))
    }

    fn emit_to<E: Send + 'static>(
        &mut self,
        id: StableNodeId,
        event: E,
    ) -> Result<(), FrameworkError> {
        let emit = self
            .view_type_id(id)
            .and_then(|type_id| self.emitters.get(&type_id))
            .copied();
        match emit {
            Some(emit) => emit_erased(self, emit, id, event),
            None => Ok(()),
        }
    }

    /// Tell the [`PointerMoved`] listeners at and above `target` (the node
    /// the pointer is over, or its capture owner) where it is.
    pub(super) fn emit_pointer_moved(
        &mut self,
        target: Option<StableNodeId>,
        x: f32,
        y: f32,
    ) -> Result<(), FrameworkError> {
        if !self.input.gesture_listeners.moved {
            return Ok(());
        }
        let mut listeners = Vec::new();
        let mut current = target;
        while let Some(id) = current {
            if self
                .event_handlers
                .contains_key(&(id, TypeId::of::<PointerMoved>()))
            {
                listeners.push(id);
            }
            current = self.world.parent_id(id);
        }
        for id in listeners {
            if let Some((x, y)) = self.node_local(id, x, y) {
                self.emit_to(id, PointerMoved { x, y })?;
            }
        }
        Ok(())
    }

    /// After a primary press was routed: if it is an ordinary press (no
    /// control captured it or began a text selection with it) inside a node
    /// listening for [`PointerDragged`], the innermost such node may turn it
    /// into a drag.
    pub(super) fn arm_view_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        pointer_type: PointerType,
        x: f32,
        y: f32,
    ) {
        let key = (document, pointer_id);
        self.input.drags.remove(&key);
        if !self.input.gesture_listeners.dragged
            || self.world.pointer_capture(document, pointer_id).is_some()
            || self
                .text_edit
                .document_select_drag
                .is_some_and(|(drag, ..)| drag == pointer_id)
        {
            return;
        }
        let Some(pressed) = self.world.pointer_press(document, pointer_id) else {
            return;
        };
        let mut current = Some(pressed);
        while let Some(id) = current {
            if self
                .event_handlers
                .contains_key(&(id, TypeId::of::<PointerDragged>()))
            {
                let slop = match pointer_type {
                    PointerType::Touch => TOUCH_SLOP,
                    _ => DRAG_SLOP,
                };
                self.input.drags.insert(
                    key,
                    ViewDrag {
                        node: id,
                        holder: if self.world.accepts_pointer(document, id) {
                            id
                        } else {
                            pressed
                        },
                        origin: (x, y),
                        slop,
                        started: false,
                    },
                );
                return;
            }
            current = self.world.parent_id(id);
        }
    }

    /// A move of a pointer whose press may be a drag. Answers whether the
    /// drag took the sample; a press still within the slop leaves it to the
    /// ordinary move.
    pub(super) fn move_view_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        modifiers: InputModifiers,
        now: std::time::Duration,
    ) -> Result<bool, FrameworkError> {
        let key = (document, pointer_id);
        let Some(drag) = self.input.drags.get(&key).copied() else {
            return Ok(false);
        };
        if drag.started {
            if self.world.pointer_capture(document, pointer_id) != Some(drag.holder) {
                // Something else took the pointer, or the node went away.
                self.input.drags.remove(&key);
                self.emit_drag(drag, PointerDragPhase::Cancelled, x, y, modifiers)?;
                return Ok(false);
            }
            self.emit_drag(drag, PointerDragPhase::Moved, x, y, modifiers)?;
            return Ok(true);
        }
        // Whatever ended or took the press since (a scroller, a cancel, the
        // button let go unseen) leaves nothing to drag.
        if self.input.pressed_buttons.get(&key) != Some(&0)
            || self.world.pointer_press(document, pointer_id).is_none()
            || self.world.pointer_capture(document, pointer_id).is_some()
            || !self.world.contains(drag.node)
            || !self.world.accepts_pointer(document, drag.holder)
        {
            self.input.drags.remove(&key);
            return Ok(false);
        }
        if (x - drag.origin.0).hypot(y - drag.origin.1) <= drag.slop {
            return Ok(false);
        }
        // The press is no longer one: nothing shows pressed, the release
        // activates nothing, and the drag holds the pointer.
        self.release_pointer(document, pointer_id);
        let mut capture = MutationQueue::new();
        capture.capture_pointer(pointer_id, drag.holder);
        self.commit_mutations(capture)?;
        self.set_pointer_hover_at(document, pointer_id, Some(drag.holder), now)?;
        let drag = ViewDrag {
            started: true,
            ..drag
        };
        self.input.drags.insert(key, drag);
        let (ox, oy) = drag.origin;
        self.emit_drag(drag, PointerDragPhase::Started, ox, oy, modifiers)?;
        if self.input.drags.get(&key).is_some_and(|d| d.started) {
            self.emit_drag(drag, PointerDragPhase::Moved, x, y, modifiers)?;
        }
        Ok(true)
    }

    /// A release (`cancelled` false) or cancel of a pointer whose press may
    /// be a drag. Answers whether a drag ended, which consumes the release.
    pub(super) fn end_view_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        modifiers: InputModifiers,
        cancelled: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(drag) = self.input.drags.remove(&(document, pointer_id)) else {
            return Ok(false);
        };
        if !drag.started {
            return Ok(false);
        }
        let held = self.world.pointer_capture(document, pointer_id) == Some(drag.holder);
        if held {
            let mut release = MutationQueue::new();
            release.release_pointer(pointer_id, drag.holder);
            self.commit_mutations(release)?;
        }
        let phase = if cancelled || !held {
            PointerDragPhase::Cancelled
        } else {
            PointerDragPhase::Ended
        };
        self.emit_drag(drag, phase, x, y, modifiers)?;
        Ok(true)
    }

    fn emit_drag(
        &mut self,
        drag: ViewDrag,
        phase: PointerDragPhase,
        x: f32,
        y: f32,
        modifiers: InputModifiers,
    ) -> Result<(), FrameworkError> {
        if !self.world.contains(drag.node) {
            return Ok(());
        }
        let Some((local_x, local_y)) = self.node_local(drag.node, x, y) else {
            return Ok(());
        };
        let (dx, dy) = (x - drag.origin.0, y - drag.origin.1);
        self.emit_to(
            drag.node,
            PointerDragged {
                phase,
                x: local_x,
                y: local_y,
                dx,
                dy,
                modifiers,
            },
        )
    }
}
