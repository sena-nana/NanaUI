//! Typed internal drag-and-drop sessions (`DragSource` -> `DropTarget`),
//! activating [`DropKind::Custom`] with `Copy`/`Move`/`Link` negotiation and
//! lifecycle-safe cancellation.

use std::any::TypeId;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use nana_ui_core::{
    DataTransfer, DragCancelReason, DragSessionId, DropAccepts, DropEffect, DropEffects, DropKind,
    TransferLifetime, TransferOrigin,
};
use nana_ui_input::{CursorIcon, InputModifiers, PointerType};

use super::super::emit_erased;
use super::super::touch_scroll::TOUCH_SLOP;
use super::gestures::DRAG_SLOP;
use crate::{
    AppContext, DocumentId, DragDropEvent, DragSourceEvent, Entity, FrameworkError, MutationQueue,
    StableNodeId, View,
};

type DragPayloadProducer = Box<dyn FnMut(&AppContext, StableNodeId) -> Option<DataTransfer> + Send>;

/// Drag source behavior attached to a node (`AppContext::set_drag_source` / `El::drag_source`).
pub struct DragSource {
    pub(crate) kind: DropKind,
    pub(crate) allowed_effects: DropEffects,
    pub(crate) produce: DragPayloadProducer,
}

impl std::fmt::Debug for DragSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DragSource")
            .field("kind", &self.kind)
            .field("allowed_effects", &self.allowed_effects)
            .finish_non_exhaustive()
    }
}

impl DragSource {
    /// Create a drag source for `kind` with default `Copy | Move` allowed actions.
    pub fn new(
        kind: DropKind,
        produce: impl FnMut(&AppContext, StableNodeId) -> Option<DataTransfer> + Send + 'static,
    ) -> Self {
        Self {
            kind,
            allowed_effects: DropEffects::COPY | DropEffects::MOVE,
            produce: Box::new(produce),
        }
    }

    /// Create a drag source for a [`DropKind::Custom`] payload name.
    pub fn custom(
        name: impl Into<Arc<str>>,
        produce: impl FnMut(&AppContext, StableNodeId) -> Option<DataTransfer> + Send + 'static,
    ) -> Self {
        Self::new(DropKind::custom(name), produce)
    }

    /// Override the [`DropEffects`] this source allows.
    pub fn allowed_effects(mut self, effects: DropEffects) -> Self {
        self.allowed_effects = effects;
        self
    }

    /// Include [`DropEffect::Link`] in the source's allowed actions.
    pub fn allow_link(mut self) -> Self {
        self.allowed_effects.insert(DropEffect::Link);
        self
    }
}

/// Renderer-neutral snapshot of the currently active internal drag session,
/// intended for drag preview/feedback hooks (never payload authority).
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveDragSession {
    pub session: DragSessionId,
    pub document: DocumentId,
    pub source: StableNodeId,
    pub kind: DropKind,
    pub allowed_effects: DropEffects,
    pub target: Option<StableNodeId>,
    pub effect: Option<DropEffect>,
    pub origin: (f32, f32),
    pub position: (f32, f32),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArmedInternalDrag {
    source: StableNodeId,
    holder: Option<StableNodeId>,
    origin: (f32, f32),
    slop: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct InternalDragState {
    pub(crate) session: DragSessionId,
    pub(crate) document: DocumentId,
    pub(crate) source: StableNodeId,
    pub(crate) holder: Option<StableNodeId>,
    pub(crate) pointer_id: Option<u64>,
    pub(crate) kind: DropKind,
    pub(crate) allowed_effects: DropEffects,
    pub(crate) transfer: DataTransfer,
    pub(crate) origin: (f32, f32),
    pub(crate) position: (f32, f32),
    pub(crate) hovered_target: Option<(StableNodeId, DropEffect)>,
}

/// Map held keyboard modifiers onto a proposed [`DropEffect`] override.
pub(crate) fn proposed_drop_effect(modifiers: InputModifiers) -> Option<DropEffect> {
    if (modifiers.control && modifiers.shift) || (modifiers.meta && modifiers.alt) {
        Some(DropEffect::Link)
    } else if (modifiers.control || modifiers.alt)
        && !modifiers.shift
        && !modifiers.meta
        && !(modifiers.control && modifiers.alt)
    {
        Some(DropEffect::Copy)
    } else if (modifiers.shift || modifiers.meta)
        && !modifiers.control
        && !modifiers.alt
        && !(modifiers.shift && modifiers.meta)
    {
        Some(DropEffect::Move)
    } else {
        None
    }
}

impl AppContext {
    /// Register `source` as an internal drag source on `entity`.
    pub fn set_drag_source<V: View>(
        &mut self,
        entity: Entity<V>,
        source: DragSource,
    ) -> Result<(), FrameworkError> {
        self.set_drag_source_node(entity.id, source)
    }

    /// Register `source` as an internal drag source on `id`.
    pub fn set_drag_source_node(
        &mut self,
        id: StableNodeId,
        source: DragSource,
    ) -> Result<(), FrameworkError> {
        if !self.world.contains(id) {
            return Err(FrameworkError::MissingView(id));
        }
        self.input.drag_sources.insert(id, source);
        Ok(())
    }

    /// Remove any [`DragSource`] registration from `entity`.
    pub fn clear_drag_source<V: View>(&mut self, entity: Entity<V>) -> bool {
        self.clear_drag_source_node(entity.id)
    }

    /// Remove any [`DragSource`] registration from `id`.
    pub fn clear_drag_source_node(&mut self, id: StableNodeId) -> bool {
        let removed = self.input.drag_sources.remove(&id).is_some();
        self.input
            .armed_internal_drags
            .retain(|_, armed| armed.source != id);
        if self
            .input
            .active_internal_drag
            .as_ref()
            .is_some_and(|drag| drag.source == id)
        {
            let _ = self.abort_active_internal_drag(DragCancelReason::Programmatic, false);
        }
        removed
    }

    /// Deepest visible drop target under `(x, y)` accepting `(kind, transfer)`
    /// with a non-empty negotiated effect.
    #[allow(clippy::too_many_arguments)]
    pub fn drop_target_for_transfer(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
        kind: &DropKind,
        transfer: &DataTransfer,
        allowed_effects: DropEffects,
        requested: Option<DropEffect>,
    ) -> Option<(StableNodeId, DropEffect)> {
        let depth_of = |mut id: StableNodeId| {
            let mut depth = 0_u32;
            while let Some(parent) = self.world.parent_id(id) {
                depth += 1;
                id = parent;
            }
            depth
        };
        self.world
            .drop_target_ids()
            .filter(|id| {
                self.world
                    .node(*id)
                    .is_some_and(|node| node.document == document)
            })
            .filter(|id| {
                self.world
                    .computed_style(*id)
                    .is_some_and(|style| style.box_visible)
            })
            .filter(|id| {
                self.world
                    .drop_target(*id)
                    .is_some_and(|accepts| accepts.accepts_transfer(kind, transfer))
            })
            .filter(|id| {
                self.world.component_layout_box(*id).is_some_and(|bounds| {
                    x >= bounds.x
                        && x < bounds.x + bounds.width
                        && y >= bounds.y
                        && y < bounds.y + bounds.height
                })
            })
            .filter_map(|id| {
                let accepts: &DropAccepts = self.world.drop_target(id)?;
                let effect = accepts.negotiate_effect(allowed_effects, requested)?;
                Some((id, effect, depth_of(id)))
            })
            .max_by_key(|(_, _, depth)| *depth)
            .map(|(id, effect, _)| (id, effect))
    }

    /// Start an internal drag session programmatically from `source`.
    ///
    /// Rejects non-internal transfers (`TransferOrigin::External`) and missing
    /// or cross-document sources.
    pub fn begin_internal_drag(
        &mut self,
        document: DocumentId,
        source: StableNodeId,
        kind: DropKind,
        transfer: DataTransfer,
        allowed_effects: DropEffects,
        origin: (f32, f32),
        pointer_id: Option<u64>,
    ) -> Result<DragSessionId, FrameworkError> {
        let Some(node) = self.world.node(source) else {
            return Err(FrameworkError::MissingView(source));
        };
        if node.document != document
            || transfer.origin() != TransferOrigin::Internal
            || !transfer.lifetime().is_alive()
            || allowed_effects.is_empty()
        {
            return Err(FrameworkError::InvalidInput);
        }
        if self.input.active_internal_drag.is_some() {
            let _ = self.abort_active_internal_drag(DragCancelReason::Programmatic, false)?;
        }
        let session = self.next_drag_session_id();
        self.register_source_transfer_lifetime(source, transfer.lifetime().clone());
        let holder = pointer_id
            .filter(|_| self.world.accepts_pointer(document, source))
            .map(|_| source);
        if let (Some(pid), Some(holder_id)) = (pointer_id, holder) {
            let mut capture = MutationQueue::new();
            capture.capture_pointer(pid, holder_id);
            self.commit_mutations(capture)?;
        }
        self.input.active_internal_drag = Some(InternalDragState {
            session,
            document,
            source,
            holder,
            pointer_id,
            kind: kind.clone(),
            allowed_effects,
            transfer,
            origin,
            position: origin,
            hovered_target: None,
        });
        self.emit_drag_source_event(
            source,
            DragSourceEvent::Started {
                session,
                kind,
                allowed_effects,
            },
        )?;
        Ok(session)
    }

    /// Programmatically cancel the active internal drag session in `document`.
    pub fn cancel_internal_drag(&mut self, document: DocumentId) -> Result<bool, FrameworkError> {
        if self
            .input
            .active_internal_drag
            .as_ref()
            .is_some_and(|drag| drag.document == document)
        {
            self.abort_active_internal_drag(DragCancelReason::Programmatic, false)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Renderer-neutral snapshot of the active internal drag in `document`.
    pub fn active_drag_session(&self, document: DocumentId) -> Option<ActiveDragSession> {
        let drag = self.input.active_internal_drag.as_ref()?;
        if drag.document != document {
            return None;
        }
        Some(ActiveDragSession {
            session: drag.session,
            document: drag.document,
            source: drag.source,
            kind: drag.kind.clone(),
            allowed_effects: drag.allowed_effects,
            target: drag.hovered_target.map(|(id, _)| id),
            effect: drag.hovered_target.map(|(_, effect)| effect),
            origin: drag.origin,
            position: drag.position,
        })
    }

    pub(crate) fn next_drag_session_id(&mut self) -> DragSessionId {
        let id = self.input.next_drag_session;
        self.input.next_drag_session = id.wrapping_add(1).max(1);
        DragSessionId(id)
    }

    fn register_source_transfer_lifetime(
        &mut self,
        source: StableNodeId,
        lifetime: TransferLifetime,
    ) {
        let entry = self
            .input
            .source_transfer_lifetimes
            .entry(source)
            .or_default();
        entry.retain(TransferLifetime::is_alive);
        entry.push(lifetime);
    }

    pub(in crate::framework::input) fn drag_source_at(
        &self,
        document: DocumentId,
        hit: Option<StableNodeId>,
        x: f32,
        y: f32,
    ) -> Option<StableNodeId> {
        if self.input.drag_sources.is_empty() {
            return None;
        }
        let mut current = hit;
        while let Some(id) = current {
            if self.input.drag_sources.contains_key(&id) {
                return Some(id);
            }
            current = self.world.parent_id(id);
        }
        let depth_of = |mut id: StableNodeId| {
            let mut depth = 0_u32;
            while let Some(parent) = self.world.parent_id(id) {
                depth += 1;
                id = parent;
            }
            depth
        };
        self.input
            .drag_sources
            .keys()
            .copied()
            .filter(|id| {
                self.world
                    .node(*id)
                    .is_some_and(|node| node.document == document)
            })
            .filter(|id| {
                self.world
                    .computed_style(*id)
                    .is_some_and(|style| style.box_visible)
            })
            .filter(|id| {
                self.world.component_layout_box(*id).is_some_and(|bounds| {
                    x >= bounds.x
                        && x < bounds.x + bounds.width
                        && y >= bounds.y
                        && y < bounds.y + bounds.height
                })
            })
            .max_by_key(|id| depth_of(*id))
    }

    pub(in crate::framework::input) fn arm_internal_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        pointer_type: PointerType,
        x: f32,
        y: f32,
    ) {
        let key = (document, pointer_id);
        self.input.armed_internal_drags.remove(&key);
        if self.input.drag_sources.is_empty()
            || self.world.pointer_capture(document, pointer_id).is_some()
            || self
                .text_edit
                .document_select_drag
                .is_some_and(|(drag, ..)| drag == pointer_id)
        {
            return;
        }
        let pressed = self.world.pointer_press(document, pointer_id);
        let Some(source) = self.drag_source_at(document, pressed, x, y) else {
            return;
        };
        let slop = match pointer_type {
            PointerType::Touch => TOUCH_SLOP,
            _ => DRAG_SLOP,
        };
        let holder = if self.world.accepts_pointer(document, source) {
            Some(source)
        } else {
            pressed.filter(|p| self.world.accepts_pointer(document, *p))
        };
        self.input.armed_internal_drags.insert(
            key,
            ArmedInternalDrag {
                source,
                holder,
                origin: (x, y),
                slop,
            },
        );
    }

    pub(in crate::framework::input) fn move_internal_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        modifiers: InputModifiers,
        _now: Duration,
    ) -> Result<bool, FrameworkError> {
        let key = (document, pointer_id);
        if let Some(active) = self.input.active_internal_drag.as_ref()
            && active.document == document
            && active.pointer_id == Some(pointer_id)
        {
            self.update_active_internal_drag(x, y, modifiers)?;
            return Ok(true);
        }

        let Some(armed) = self.input.armed_internal_drags.get(&key).copied() else {
            return Ok(false);
        };
        if self.input.pressed_buttons.get(&key) != Some(&0)
            || self.world.pointer_capture(document, pointer_id).is_some()
            || !self.world.contains(armed.source)
            || !self.input.drag_sources.contains_key(&armed.source)
        {
            self.input.armed_internal_drags.remove(&key);
            return Ok(false);
        }
        if (x - armed.origin.0).hypot(y - armed.origin.1) <= armed.slop {
            return Ok(false);
        }
        self.input.armed_internal_drags.remove(&key);
        self.input.drags.remove(&key);

        let Some(mut source_cfg) = self.input.drag_sources.remove(&armed.source) else {
            return Ok(false);
        };
        let produced = (source_cfg.produce)(self, armed.source);
        if self.world.contains(armed.source) && !self.input.drag_sources.contains_key(&armed.source)
        {
            self.input.drag_sources.insert(armed.source, source_cfg);
        }
        let Some(transfer) = produced else {
            return Ok(false);
        };
        if transfer.origin() != TransferOrigin::Internal
            || !transfer.lifetime().is_alive()
            || !self.input.drag_sources.contains_key(&armed.source)
        {
            return Ok(false);
        }
        let (kind, allowed_effects) = self
            .input
            .drag_sources
            .get(&armed.source)
            .map(|cfg| (cfg.kind.clone(), cfg.allowed_effects))
            .ok_or(FrameworkError::MissingView(armed.source))?;
        if allowed_effects.is_empty() {
            return Ok(false);
        }

        self.release_pointer(document, pointer_id);
        let holder = armed
            .holder
            .filter(|h| self.world.accepts_pointer(document, *h));
        if let Some(holder_id) = holder {
            let mut capture = MutationQueue::new();
            capture.capture_pointer(pointer_id, holder_id);
            self.commit_mutations(capture)?;
        }

        let session = self.next_drag_session_id();
        self.register_source_transfer_lifetime(armed.source, transfer.lifetime().clone());
        self.input.active_internal_drag = Some(InternalDragState {
            session,
            document,
            source: armed.source,
            holder,
            pointer_id: Some(pointer_id),
            kind: kind.clone(),
            allowed_effects,
            transfer,
            origin: armed.origin,
            position: (x, y),
            hovered_target: None,
        });
        self.emit_drag_source_event(
            armed.source,
            DragSourceEvent::Started {
                session,
                kind,
                allowed_effects,
            },
        )?;
        self.update_active_internal_drag(x, y, modifiers)?;
        Ok(true)
    }

    fn update_active_internal_drag(
        &mut self,
        x: f32,
        y: f32,
        modifiers: InputModifiers,
    ) -> Result<(), FrameworkError> {
        let Some((document, source, session, kind, allowed_effects, transfer, prev_hover)) =
            self.input.active_internal_drag.as_ref().map(|drag| {
                (
                    drag.document,
                    drag.source,
                    drag.session,
                    drag.kind.clone(),
                    drag.allowed_effects,
                    drag.transfer.clone(),
                    drag.hovered_target,
                )
            })
        else {
            return Ok(());
        };
        if !transfer.lifetime().is_alive() {
            return self.abort_active_internal_drag(DragCancelReason::SourceDestroyed, true);
        }
        let requested = proposed_drop_effect(modifiers);
        let next_hover = self.drop_target_for_transfer(
            document,
            x,
            y,
            &kind,
            &transfer,
            allowed_effects,
            requested,
        );
        if let Some(active) = self.input.active_internal_drag.as_mut() {
            active.position = (x, y);
            active.hovered_target = next_hover;
        }
        self.world.set_drop_hover(next_hover);

        let prev_id = prev_hover.map(|(id, _)| id);
        let next_id = next_hover.map(|(id, _)| id);
        if prev_id != next_id
            && let Some(old_id) = prev_id
        {
            self.emit_drag_drop_event(old_id, DragDropEvent::Left { session })?;
        }
        if let Some((target_id, effect)) = next_hover {
            let payload = if prev_id != Some(target_id) {
                DragDropEvent::Entered {
                    session,
                    kind,
                    transfer,
                    effect,
                    x,
                    y,
                    modifiers,
                }
            } else {
                DragDropEvent::Moved {
                    session,
                    kind,
                    transfer,
                    effect,
                    x,
                    y,
                    modifiers,
                }
            };
            self.emit_drag_drop_event(target_id, payload)?;
        }
        self.emit_drag_source_event(
            source,
            DragSourceEvent::Updated {
                session,
                target: next_hover.map(|(id, _)| id),
                effect: next_hover.map(|(_, effect)| effect),
                x,
                y,
            },
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::framework::input) fn end_internal_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        modifiers: InputModifiers,
        cancelled: bool,
        cancel_reason: DragCancelReason,
    ) -> Result<bool, FrameworkError> {
        let key = (document, pointer_id);
        self.input.armed_internal_drags.remove(&key);
        let matches_active = self
            .input
            .active_internal_drag
            .as_ref()
            .is_some_and(|drag| drag.document == document && drag.pointer_id == Some(pointer_id));
        if !matches_active {
            return Ok(false);
        }
        if cancelled {
            self.abort_active_internal_drag(cancel_reason, false)?;
            return Ok(true);
        }

        // Re-resolve drop target at the release position & modifiers.
        let Some((source, session, kind, allowed_effects, transfer, prev_hover)) =
            self.input.active_internal_drag.as_ref().map(|drag| {
                (
                    drag.source,
                    drag.session,
                    drag.kind.clone(),
                    drag.allowed_effects,
                    drag.transfer.clone(),
                    drag.hovered_target,
                )
            })
        else {
            return Ok(false);
        };
        if !transfer.lifetime().is_alive() {
            self.abort_active_internal_drag(DragCancelReason::SourceDestroyed, true)?;
            return Ok(true);
        }

        let requested = proposed_drop_effect(modifiers);
        let final_target = self.drop_target_for_transfer(
            document,
            x,
            y,
            &kind,
            &transfer,
            allowed_effects,
            requested,
        );
        let active = self
            .input
            .active_internal_drag
            .take()
            .expect("checked above");
        self.release_drag_capture(&active)?;
        self.world.set_drop_hover(None);

        if let Some((old_id, _)) = prev_hover
            && final_target.map(|(id, _)| id) != Some(old_id)
        {
            self.emit_drag_drop_event(old_id, DragDropEvent::Left { session })?;
        }

        if let Some((target_id, effect)) = final_target {
            self.emit_drag_drop_event(
                target_id,
                DragDropEvent::Dropped {
                    session,
                    kind,
                    transfer,
                    effect,
                    x,
                    y,
                    modifiers,
                },
            )?;
            self.emit_drag_source_event(
                source,
                DragSourceEvent::Finished {
                    session,
                    target: target_id,
                    effect,
                },
            )?;
        } else {
            transfer.lifetime().cancel();
            self.emit_drag_source_event(
                source,
                DragSourceEvent::Cancelled {
                    session,
                    reason: cancel_reason,
                },
            )?;
        }
        Ok(true)
    }

    pub(in crate::framework::input) fn cancel_internal_drag_on_escape(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        if self
            .input
            .active_internal_drag
            .as_ref()
            .is_some_and(|drag| drag.document == document)
        {
            self.abort_active_internal_drag(DragCancelReason::Escape, false)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(in crate::framework::input) fn cancel_source_internal_drag(
        &mut self,
        source: super::InputSourceId,
        document: DocumentId,
        reason: DragCancelReason,
    ) -> Result<bool, FrameworkError> {
        self.input
            .armed_internal_drags
            .retain(|(doc, _), _| *doc != document);
        let should_cancel = {
            let Some(active) = self.input.active_internal_drag.as_ref() else {
                return Ok(false);
            };
            if active.document != document {
                return Ok(false);
            }
            match active.pointer_id {
                None => true,
                Some(pid) => self
                    .input
                    .sources
                    .get(&source)
                    .is_some_and(|state| state.pointers.values().any(|p| p.local == pid)),
            }
        };
        if should_cancel {
            self.abort_active_internal_drag(reason, false)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(in crate::framework) fn clear_drop_hover_for_removed_target(
        &mut self,
        target: StableNodeId,
    ) -> Result<(), FrameworkError> {
        let Some((session, source, x, y)) =
            self.input.active_internal_drag.as_mut().and_then(|drag| {
                if drag.hovered_target.is_some_and(|(id, _)| id == target) {
                    drag.hovered_target = None;
                    Some((drag.session, drag.source, drag.position.0, drag.position.1))
                } else {
                    None
                }
            })
        else {
            return Ok(());
        };
        if self.world.contains(target) {
            self.emit_drag_drop_event(target, DragDropEvent::Left { session })?;
        }
        self.emit_drag_source_event(
            source,
            DragSourceEvent::Updated {
                session,
                target: None,
                effect: None,
                x,
                y,
            },
        )?;
        Ok(())
    }

    pub(crate) fn forget_drag_nodes(&mut self, removed: &HashSet<StableNodeId>) {
        if removed.is_empty() {
            return;
        }
        for id in removed {
            self.input.drag_sources.remove(id);
            if let Some(lifetimes) = self.input.source_transfer_lifetimes.remove(id) {
                for lifetime in lifetimes {
                    lifetime.mark_source_destroyed();
                }
            }
        }
        self.input.armed_internal_drags.retain(|_, armed| {
            !removed.contains(&armed.source) && !armed.holder.is_some_and(|h| removed.contains(&h))
        });

        if let Some(active) = self.input.active_internal_drag.as_ref() {
            if removed.contains(&active.source) {
                let _ = self.abort_active_internal_drag(DragCancelReason::SourceDestroyed, true);
            } else if active
                .hovered_target
                .is_some_and(|(target, _)| removed.contains(&target))
            {
                let target = active.hovered_target.unwrap().0;
                let _ = self.clear_drop_hover_for_removed_target(target);
            }
        }
    }

    pub(in crate::framework::input) fn active_drag_cursor(
        &self,
        document: DocumentId,
    ) -> Option<CursorIcon> {
        let active = self.input.active_internal_drag.as_ref()?;
        if active.document != document {
            return None;
        }
        Some(match active.hovered_target.map(|(_, effect)| effect) {
            Some(DropEffect::Copy) => CursorIcon::Grabbing,
            Some(DropEffect::Move) => CursorIcon::Move,
            Some(DropEffect::Link) => CursorIcon::Pointer,
            None => CursorIcon::NotAllowed,
        })
    }

    fn abort_active_internal_drag(
        &mut self,
        reason: DragCancelReason,
        source_destroyed: bool,
    ) -> Result<(), FrameworkError> {
        let Some(active) = self.input.active_internal_drag.take() else {
            return Ok(());
        };
        if source_destroyed {
            active.transfer.lifetime().mark_source_destroyed();
        } else {
            active.transfer.lifetime().cancel();
        }
        let _ = self.release_drag_capture(&active);
        self.world.set_drop_hover(None);
        if let Some((target_id, _)) = active.hovered_target
            && self.world.contains(target_id)
        {
            self.emit_drag_drop_event(
                target_id,
                DragDropEvent::Left {
                    session: active.session,
                },
            )?;
        }
        if !source_destroyed && self.world.contains(active.source) {
            self.emit_drag_source_event(
                active.source,
                DragSourceEvent::Cancelled {
                    session: active.session,
                    reason,
                },
            )?;
        }
        Ok(())
    }

    fn release_drag_capture(&mut self, active: &InternalDragState) -> Result<(), FrameworkError> {
        let (Some(pid), Some(holder)) = (active.pointer_id, active.holder) else {
            return Ok(());
        };
        if self.world.pointer_capture(active.document, pid) == Some(holder) {
            let mut release = MutationQueue::new();
            release.release_pointer(pid, holder);
            self.commit_mutations(release)?;
        }
        Ok(())
    }

    pub(crate) fn emit_drag_drop_event(
        &mut self,
        id: StableNodeId,
        event: DragDropEvent,
    ) -> Result<(), FrameworkError> {
        self.emit_typed_drag_event(id, event)
    }

    fn emit_drag_source_event(
        &mut self,
        id: StableNodeId,
        event: DragSourceEvent,
    ) -> Result<(), FrameworkError> {
        self.emit_typed_drag_event(id, event)
    }

    fn emit_typed_drag_event<E: Send + 'static>(
        &mut self,
        id: StableNodeId,
        event: E,
    ) -> Result<(), FrameworkError> {
        let Some(emit) = self
            .view_type_id(id)
            .and_then(|type_id| self.store.emitters.get(&type_id))
            .copied()
        else {
            return Ok(());
        };
        // Avoid allocating or doing work if no handler listens for `E` on `id`.
        if !self
            .lifecycle
            .event_handlers
            .contains_key(&(id, TypeId::of::<E>()))
        {
            return Ok(());
        }
        emit_erased(self, emit, id, event)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use nana_ui_core::{
        DataFormat, DataTransfer, DragCancelReason, DropAccepts, DropEffect, DropEffects, DropKind,
        FileDragKind, LengthSpec, TransferCachePolicy, TransferError, TransferOrigin,
    };
    use nana_ui_input::{InputModifiers, InputPayload, KeyInput, KeyState, PointerPhase};

    use crate::{
        AppContext, DocumentId, DragDropEvent, DragSource, DragSourceEvent, Entity, FileDropEvent,
        HeadlessInput, LayoutViewport, NodeStyle, Stack, TextInput,
    };

    fn sized_box(width: f32, height: f32) -> Stack {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(width));
        layout.height = Some(LengthSpec::Px(height));
        Stack::column(0.0).style(style)
    }

    fn sized_row(width: f32, height: f32) -> Stack {
        Stack::row(0.0).with_layout(|layout| {
            layout.width = Some(LengthSpec::Px(width));
            layout.height = Some(LengthSpec::Px(height));
            layout.align_items = nana_ui_core::AlignSpec::Start;
        })
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct NodeToken(u64);

    fn setup_source_and_targets(
        context: &mut AppContext,
        document: DocumentId,
    ) -> (Entity<Stack>, Entity<Stack>, Entity<Stack>, Entity<Stack>) {
        let root = context
            .create_component(document, sized_row(400.0, 200.0))
            .unwrap();
        let source = context
            .create_component(document, sized_box(80.0, 80.0))
            .unwrap();
        let outer = context
            .create_component(document, sized_box(160.0, 160.0))
            .unwrap();
        let inner = context
            .create_component(document, sized_box(80.0, 80.0))
            .unwrap();
        context
            .attach_child(root.stable_id(), source.stable_id())
            .unwrap();
        context
            .attach_child(root.stable_id(), outer.stable_id())
            .unwrap();
        context
            .attach_child(outer.stable_id(), inner.stable_id())
            .unwrap();
        context
            .layout_document(document, LayoutViewport::new(400.0, 200.0))
            .unwrap();
        (root, source, outer, inner)
    }

    #[test]
    fn custom_internal_drag_negotiates_actions_nested_targets_and_lazy_payload() {
        let mut context = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let (_, source, outer, inner) = setup_source_and_targets(&mut context, doc);
        let editor = context
            .create_component(doc, TextInput::new("keep-focus"))
            .unwrap();
        context.focus_node(doc, editor.stable_id()).unwrap();

        let produce_calls = Arc::new(AtomicUsize::new(0));
        let lazy_calls = Arc::new(AtomicUsize::new(0));
        let produce_ref = Arc::clone(&produce_calls);
        let lazy_ref = Arc::clone(&lazy_calls);

        let custom_mime = DataFormat::custom("application/x-nana-asset").unwrap();
        let custom_mime_inner = custom_mime.clone();

        context
            .set_drag_source(
                source,
                DragSource::custom("asset-card", move |_, id| {
                    produce_ref.fetch_add(1, Ordering::Relaxed);
                    let mut transfer = DataTransfer::internal();
                    let _ = transfer.set_text("card-7");
                    let _ = transfer
                        .set_internal_payload(DropKind::custom("asset-card"), NodeToken(id.get()));
                    let lazy_ref = Arc::clone(&lazy_ref);
                    transfer.set_lazy_fn(
                        custom_mime_inner.clone(),
                        Some(4),
                        TransferCachePolicy::Cache,
                        move |_| {
                            lazy_ref.fetch_add(1, Ordering::Relaxed);
                            Ok(vec![1, 2, 3, 4])
                        },
                    );
                    Some(transfer)
                })
                .allowed_effects(DropEffects::COPY | DropEffects::MOVE | DropEffects::LINK),
            )
            .unwrap();

        context
            .set_drop_target(
                outer,
                DropAccepts::custom("asset-card")
                    .effect(DropEffect::Copy)
                    .effects(DropEffects::COPY | DropEffects::LINK),
            )
            .unwrap();
        context
            .set_drop_target(
                inner,
                DropAccepts::custom("asset-card")
                    .with_formats([custom_mime.clone()])
                    .effect(DropEffect::Move)
                    .effects(DropEffects::MOVE | DropEffects::LINK),
            )
            .unwrap();

        let source_events = Arc::new(Mutex::new(Vec::<String>::new()));
        let outer_events = Arc::new(Mutex::new(Vec::<String>::new()));
        let inner_events = Arc::new(Mutex::new(Vec::<String>::new()));
        let dropped_token = Arc::new(Mutex::new(None));
        let dropped_lazy = Arc::new(Mutex::new(None));

        let s_log = Arc::clone(&source_events);
        context
            .on(source, move |_, event: &DragSourceEvent, _| match event {
                DragSourceEvent::Started { .. } => s_log.lock().unwrap().push("started".into()),
                DragSourceEvent::Updated { effect, .. } => {
                    s_log.lock().unwrap().push(format!("updated:{effect:?}"));
                }
                DragSourceEvent::Finished { effect, .. } => {
                    s_log.lock().unwrap().push(format!("finished:{effect:?}"));
                }
                DragSourceEvent::Cancelled { reason, .. } => {
                    s_log.lock().unwrap().push(format!("cancelled:{reason:?}"));
                }
            })
            .unwrap();

        let o_log = Arc::clone(&outer_events);
        context
            .on(outer, move |_, event: &DragDropEvent, _| match event {
                DragDropEvent::Entered { effect, .. } => {
                    o_log.lock().unwrap().push(format!("enter:{effect:?}"));
                }
                DragDropEvent::Moved { effect, .. } => {
                    o_log.lock().unwrap().push(format!("move:{effect:?}"));
                }
                DragDropEvent::Left { .. } => o_log.lock().unwrap().push("leave".into()),
                DragDropEvent::Dropped { effect, .. } => {
                    o_log.lock().unwrap().push(format!("drop:{effect:?}"));
                }
            })
            .unwrap();

        let i_log = Arc::clone(&inner_events);
        let tok_out = Arc::clone(&dropped_token);
        let lazy_out = Arc::clone(&dropped_lazy);
        let mime_read = custom_mime.clone();
        context
            .on(inner, move |_, event: &DragDropEvent, _| match event {
                DragDropEvent::Entered { effect, .. } => {
                    i_log.lock().unwrap().push(format!("enter:{effect:?}"));
                }
                DragDropEvent::Moved { effect, .. } => {
                    i_log.lock().unwrap().push(format!("move:{effect:?}"));
                }
                DragDropEvent::Left { .. } => i_log.lock().unwrap().push("leave".into()),
                DragDropEvent::Dropped {
                    transfer, effect, ..
                } => {
                    i_log.lock().unwrap().push(format!("drop:{effect:?}"));
                    *tok_out.lock().unwrap() =
                        transfer.read_internal::<NodeToken>().unwrap().cloned();
                    *lazy_out.lock().unwrap() =
                        transfer.read_bytes(&mime_read).unwrap().map(|b| b.to_vec());
                }
            })
            .unwrap();

        let mut input = HeadlessInput::bind(&mut context, doc);
        // Short click within slop does not trigger drag or produce payload.
        input
            .pointer(&mut context, PointerPhase::Down, 20.0, 20.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, 21.0, 20.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Up, 21.0, 20.0)
            .unwrap();
        assert_eq!(produce_calls.load(Ordering::Relaxed), 0);
        assert!(context.active_drag_session(doc).is_none());

        // Drag from source (x=20,y=20) into inner (x=100,y=20), then outer-only (x=100,y=110),
        // then back into inner with Ctrl+Shift (Link) and release.
        input
            .pointer(&mut context, PointerPhase::Down, 20.0, 20.0)
            .unwrap();
        // Move past slop onto inner box [80..160, 0..80].
        input
            .pointer(&mut context, PointerPhase::Move, 100.0, 20.0)
            .unwrap();
        assert_eq!(produce_calls.load(Ordering::Relaxed), 1);
        assert_eq!(lazy_calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            context.drop_hover(),
            Some((inner.stable_id(), DropEffect::Move))
        );
        assert_eq!(
            context.world().focused(doc),
            Some(editor.stable_id()),
            "internal drag must not steal keyboard focus"
        );

        // Move into outer-only region [80..240, 80..160] -> inner gets Left, outer gets Entered(Copy).
        input
            .pointer(&mut context, PointerPhase::Move, 100.0, 110.0)
            .unwrap();
        assert_eq!(
            context.drop_hover(),
            Some((outer.stable_id(), DropEffect::Copy))
        );

        // Move back into inner with Link modifier (Ctrl+Shift).
        let link_mods = InputModifiers {
            control: true,
            shift: true,
            ..InputModifiers::default()
        };
        input
            .route(
                &mut context,
                InputPayload::Pointer(nana_ui_input::PointerInput {
                    modifiers: link_mods,
                    ..nana_ui_input::PointerInput::mouse(PointerPhase::Move, 100.0, 20.0)
                }),
            )
            .unwrap();
        assert_eq!(
            context.drop_hover(),
            Some((inner.stable_id(), DropEffect::Link))
        );
        assert_eq!(lazy_calls.load(Ordering::Relaxed), 0);

        // Drop with Link modifier.
        input
            .route(
                &mut context,
                InputPayload::Pointer(nana_ui_input::PointerInput {
                    modifiers: link_mods,
                    ..nana_ui_input::PointerInput::mouse(PointerPhase::Up, 100.0, 20.0)
                }),
            )
            .unwrap();
        assert!(context.active_drag_session(doc).is_none());
        assert!(context.drop_hover().is_none());
        assert_eq!(lazy_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            *dropped_token.lock().unwrap(),
            Some(NodeToken(source.stable_id().get()))
        );
        assert_eq!(*dropped_lazy.lock().unwrap(), Some(vec![1, 2, 3, 4]));
        assert_eq!(
            outer_events.lock().unwrap().as_slice(),
            ["enter:Copy", "leave"]
        );
        assert_eq!(
            inner_events.lock().unwrap().as_slice(),
            ["enter:Move", "leave", "enter:Link", "drop:Link"]
        );
        assert_eq!(
            source_events.lock().unwrap().last().map(String::as_str),
            Some("finished:Link")
        );
    }

    #[test]
    fn internal_drag_handles_rejection_escape_blur_disconnect_and_source_destruction() {
        let mut context = AppContext::new();
        let doc = DocumentId::new(2).unwrap();
        let (_, source, outer, _) = setup_source_and_targets(&mut context, doc);
        let one_gib = 1024 * 1024 * 1024;
        let lazy_invoked = Arc::new(AtomicUsize::new(0));
        let lazy_ref = Arc::clone(&lazy_invoked);

        context
            .set_drag_source(
                source,
                DragSource::custom("blob", move |_, _| {
                    let mut transfer = DataTransfer::internal();
                    let lazy_ref = Arc::clone(&lazy_ref);
                    transfer.set_lazy_fn(
                        DataFormat::OctetStream,
                        Some(one_gib),
                        TransferCachePolicy::Cache,
                        move |_| {
                            lazy_ref.fetch_add(1, Ordering::Relaxed);
                            Ok(vec![42])
                        },
                    );
                    Some(transfer)
                })
                .allowed_effects(DropEffects::COPY),
            )
            .unwrap();

        // Target wants `Move` only -> disjoint with source (`Copy`) -> rejected!
        context
            .set_drop_target(outer, DropAccepts::custom("blob").effect(DropEffect::Move))
            .unwrap();

        let source_cancels = Arc::new(Mutex::new(Vec::<DragCancelReason>::new()));
        let cancel_log = Arc::clone(&source_cancels);
        context
            .on(source, move |_, event: &DragSourceEvent, _| {
                if let DragSourceEvent::Cancelled { reason, .. } = event {
                    cancel_log.lock().unwrap().push(*reason);
                }
            })
            .unwrap();

        let mut input = HeadlessInput::bind(&mut context, doc);
        input
            .pointer(&mut context, PointerPhase::Down, 20.0, 20.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, 100.0, 20.0)
            .unwrap();
        assert!(context.active_drag_session(doc).is_some());
        assert_eq!(context.drop_hover(), None);
        assert_eq!(lazy_invoked.load(Ordering::Relaxed), 0);
        assert_eq!(
            input.services().cursor(),
            nana_ui_input::CursorIcon::NotAllowed
        );

        // Escape cancels active drag.
        input
            .route(
                &mut context,
                InputPayload::Key(KeyInput::named(
                    "Escape",
                    "Escape",
                    KeyState::Pressed,
                    InputModifiers::default(),
                )),
            )
            .unwrap();
        assert!(context.active_drag_session(doc).is_none());
        assert_eq!(
            source_cancels.lock().unwrap().as_slice(),
            [DragCancelReason::Escape]
        );

        // Now make outer accept `Copy` and test window blur cancellation.
        context
            .set_drop_target(outer, DropAccepts::custom("blob").effect(DropEffect::Copy))
            .unwrap();
        let outer_events = Arc::new(Mutex::new(Vec::<String>::new()));
        let o_log = Arc::clone(&outer_events);
        let held_transfer = Arc::new(Mutex::new(None::<DataTransfer>));
        let held_ref = Arc::clone(&held_transfer);
        context
            .on(outer, move |_, event: &DragDropEvent, _| match event {
                DragDropEvent::Entered { transfer, .. } => {
                    *held_ref.lock().unwrap() = Some(transfer.clone());
                    o_log.lock().unwrap().push("enter".into());
                }
                DragDropEvent::Left { .. } => o_log.lock().unwrap().push("leave".into()),
                _ => {}
            })
            .unwrap();

        input
            .pointer(&mut context, PointerPhase::Down, 20.0, 20.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, 100.0, 20.0)
            .unwrap();
        assert_eq!(
            context.drop_hover(),
            Some((outer.stable_id(), DropEffect::Copy))
        );
        input
            .route(&mut context, InputPayload::Focus { focused: false })
            .unwrap();
        assert!(context.active_drag_session(doc).is_none());
        assert!(context.drop_hover().is_none());
        assert_eq!(
            held_transfer
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .read_bytes(&DataFormat::OctetStream),
            Err(TransferError::Cancelled)
        );
        assert_eq!(
            source_cancels.lock().unwrap().as_slice(),
            [DragCancelReason::Escape, DragCancelReason::WindowBlur]
        );

        // Test source node destruction mid-drag marks lifetime SourceDestroyed and sends Left.
        input
            .route(&mut context, InputPayload::Focus { focused: true })
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Down, 20.0, 20.0)
            .unwrap();
        input
            .pointer(&mut context, PointerPhase::Move, 100.0, 20.0)
            .unwrap();
        assert_eq!(
            context.drop_hover(),
            Some((outer.stable_id(), DropEffect::Copy))
        );
        context.remove_view(source).unwrap();
        assert!(context.active_drag_session(doc).is_none());
        assert!(context.drop_hover().is_none());
        assert_eq!(
            held_transfer
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .read_bytes(&DataFormat::OctetStream),
            Err(TransferError::SourceDestroyed)
        );
        assert_eq!(
            outer_events.lock().unwrap().as_slice(),
            ["enter", "leave", "enter", "leave"]
        );
        assert_eq!(lazy_invoked.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn external_file_drag_emits_unified_drag_drop_event_without_reading_files() {
        let mut context = AppContext::new();
        let doc = DocumentId::new(3).unwrap();
        let target = context
            .create_component(doc, sized_box(200.0, 120.0))
            .unwrap();
        context
            .layout_document(doc, LayoutViewport::new(300.0, 200.0))
            .unwrap();
        context
            .set_drop_target(target, DropAccepts::files())
            .unwrap();

        let file_events = Arc::new(Mutex::new(Vec::<FileDropEvent>::new()));
        let drag_events = Arc::new(Mutex::new(Vec::<String>::new()));
        let f_log = Arc::clone(&file_events);
        let d_log = Arc::clone(&drag_events);
        context
            .on(target, move |_, ev: &FileDropEvent, _| {
                f_log.lock().unwrap().push(ev.clone());
            })
            .unwrap();
        context
            .on(target, move |_, ev: &DragDropEvent, _| match ev {
                DragDropEvent::Entered { transfer, .. } => {
                    assert_eq!(transfer.origin(), TransferOrigin::External);
                    assert_eq!(
                        transfer.read_file_paths().unwrap(),
                        Some(vec![PathBuf::from("/nonexistent-file-120/note.md")])
                    );
                    d_log.lock().unwrap().push("enter".into());
                }
                DragDropEvent::Dropped { transfer, .. } => {
                    assert_eq!(transfer.origin(), TransferOrigin::External);
                    d_log.lock().unwrap().push("drop".into());
                }
                DragDropEvent::Left { .. } => d_log.lock().unwrap().push("leave".into()),
                DragDropEvent::Moved { .. } => d_log.lock().unwrap().push("move".into()),
            })
            .unwrap();

        let mut input = HeadlessInput::bind(&mut context, doc);
        let paths = vec![PathBuf::from("/nonexistent-file-120/note.md")];
        input
            .route(
                &mut context,
                InputPayload::FileDrag(nana_ui_input::FileDragInput {
                    kind: FileDragKind::Hover,
                    paths: paths.clone(),
                    position: Some((20.0, 20.0)),
                    modifiers: InputModifiers::default(),
                }),
            )
            .unwrap();
        input
            .route(
                &mut context,
                InputPayload::FileDrag(nana_ui_input::FileDragInput {
                    kind: FileDragKind::Drop,
                    paths,
                    position: Some((20.0, 20.0)),
                    modifiers: InputModifiers::default(),
                }),
            )
            .unwrap();
        assert_eq!(file_events.lock().unwrap().len(), 2);
        assert_eq!(drag_events.lock().unwrap().as_slice(), ["enter", "drop"]);
    }
}
