//! Keyboard patterns any container or control opts into: roving focus
//! groups ([`RovingFocusGroup`], the WAI-ARIA toolbar / menu arrows) and the
//! keyboard's context-menu request (the `ContextMenu` key, Shift+F10, and the
//! arrows on a popup trigger, raising [`SecondaryPress`]).
//!
//! Both run after the focused control has had the key: a text field's caret
//! keys, a slider's steps, a table's cells and a tree's rows stay theirs.

use super::*;
use crate::{
    RovingEdge, RovingFocusEdge, RovingFocusGroup, RovingFocusPolicy, SelectionOrientation,
};

/// Emit [`RovingFocusEdge`] on a group whose concrete type the caller no
/// longer knows; recorded with the group, which is declared typed.
type RovingEdgeFn =
    fn(&mut AppContext, StableNodeId, RovingFocusEdge) -> Result<(), FrameworkError>;

#[derive(Clone, Copy)]
pub(super) struct RovingGroupEntry {
    group: RovingFocusGroup,
    emit_edge: RovingEdgeFn,
}

fn emit_roving_edge<V: View>(
    cx: &mut AppContext,
    id: StableNodeId,
    event: RovingFocusEdge,
) -> Result<(), FrameworkError> {
    cx.update(Entity::<V>::from_stable_id(id), |_, cx| cx.emit(event))
}

impl AppContext {
    /// Make `entity` a [`RovingFocusGroup`], or, with `None`, no longer one.
    /// Its [`RovingFocusEdge`] events are emitted on it. The declaration goes
    /// with the node.
    pub fn set_roving_focus_group<V: View>(
        &mut self,
        entity: Entity<V>,
        group: Option<RovingFocusGroup>,
    ) -> Result<(), FrameworkError> {
        self.read(entity, |_| ())?;
        match group {
            Some(group) => {
                self.roving_focus_groups.insert(
                    entity.id,
                    RovingGroupEntry {
                        group,
                        emit_edge: emit_roving_edge::<V>,
                    },
                );
            }
            None => {
                self.roving_focus_groups.remove(&entity.id);
            }
        }
        Ok(())
    }

    /// The group `id` was declared as, if any.
    pub fn roving_focus_group(&self, id: StableNodeId) -> Option<RovingFocusGroup> {
        self.roving_focus_groups.get(&id).map(|entry| entry.group)
    }

    /// The nearest group strictly above `id`.
    fn enclosing_roving_group(&self, id: StableNodeId) -> Option<StableNodeId> {
        let mut current = self.world.parent_id(id);
        while let Some(candidate) = current {
            if self.roving_focus_groups.contains_key(&candidate) {
                return Some(candidate);
            }
            current = self.world.parent_id(candidate);
        }
        None
    }

    /// `group`'s items in document order: focusable, enabled and reachable
    /// nodes under it whose nearest group it is. A nested group's container
    /// can be an item; what is under it is not.
    fn roving_focus_items(&self, document: DocumentId, group: StableNodeId) -> Vec<StableNodeId> {
        let mut items = Vec::new();
        let mut stack = self
            .world
            .node(group)
            .map(|node| node.children.iter().rev().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        while let Some(id) = stack.pop() {
            if !self.world.motion_blocks_input(id)
                && self.sequential_focus_candidate(document, id)
                && !self
                    .world
                    .accessibility(id)
                    .is_some_and(|state| state.disabled)
                && self.world.is_overlay_reachable(id)
            {
                items.push(id);
            }
            if self.roving_focus_groups.contains_key(&id) {
                continue;
            }
            if let Some(node) = self.world.node(id) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        items
    }

    /// An unmodified arrow, Home or End for the group the focused item is
    /// in. Returns whether the group took the key: it moved focus, or met
    /// an end and emitted [`RovingFocusEdge`].
    pub(super) fn roving_focus_key(
        &mut self,
        document: DocumentId,
        key: &str,
    ) -> Result<bool, FrameworkError> {
        if self.roving_focus_groups.is_empty() {
            return Ok(false);
        }
        let Some(focused) = self.world.focused(document) else {
            return Ok(false);
        };
        let Some(group_id) = self.enclosing_roving_group(focused) else {
            return Ok(false);
        };
        let Some(&RovingGroupEntry { group, emit_edge }) = self.roving_focus_groups.get(&group_id)
        else {
            return Ok(false);
        };
        let writing = self.world.layout_writing(group_id);
        let reversed = !writing.is_vertical() && writing.inline_reversed();
        let intent = match (group.orientation, key) {
            (SelectionOrientation::Vertical, "ArrowUp") => RovingFocusIntent::Previous,
            (SelectionOrientation::Vertical, "ArrowDown") => RovingFocusIntent::Next,
            (SelectionOrientation::Horizontal, "ArrowLeft") if reversed => RovingFocusIntent::Next,
            (SelectionOrientation::Horizontal, "ArrowLeft") => RovingFocusIntent::Previous,
            (SelectionOrientation::Horizontal, "ArrowRight") if reversed => {
                RovingFocusIntent::Previous
            }
            (SelectionOrientation::Horizontal, "ArrowRight") => RovingFocusIntent::Next,
            (_, "Home") => RovingFocusIntent::First,
            (_, "End") => RovingFocusIntent::Last,
            _ => return Ok(false),
        };
        let items = self.roving_focus_items(document, group_id);
        let Some(index) = items.iter().position(|id| *id == focused) else {
            return Ok(false);
        };
        if !group.wrap {
            let edge = match intent {
                RovingFocusIntent::Previous if index == 0 => Some(RovingEdge::Start),
                RovingFocusIntent::Next if index + 1 == items.len() => Some(RovingEdge::End),
                _ => None,
            };
            if let Some(edge) = edge {
                emit_edge(
                    self,
                    group_id,
                    RovingFocusEdge {
                        edge,
                        item: focused,
                    },
                )?;
                return Ok(true);
            }
        }
        let enabled = items.iter().map(|id| (*id, true)).collect::<Vec<_>>();
        let policy = RovingFocusPolicy { wrap: group.wrap };
        if let Some(target) = policy.resolve(&enabled, Some(focused), intent)
            && target != focused
        {
            self.focus_node(document, target)?;
        }
        Ok(true)
    }

    /// The keyboard's context-menu request on the focused node: the
    /// `ContextMenu` key or Shift+F10 anywhere, and an unmodified ArrowUp /
    /// ArrowDown on a popup trigger. A held key's repeats raise nothing.
    /// Returns whether a handler took it.
    pub(super) fn keyboard_menu_key(
        &mut self,
        document: DocumentId,
        key: &str,
        repeat: bool,
        modifiers: nana_ui_input::InputModifiers,
    ) -> Result<bool, FrameworkError> {
        if repeat || modifiers.alt || modifiers.control || modifiers.meta {
            return Ok(false);
        }
        let requested = match key {
            "ContextMenu" => true,
            "F10" => modifiers.shift,
            "ArrowUp" | "ArrowDown" if !modifiers.shift => {
                self.world.focused(document).is_some_and(|focused| {
                    self.world
                        .accessibility(focused)
                        .is_some_and(|state| state.has_popup && !state.disabled)
                })
            }
            _ => false,
        };
        if !requested {
            return Ok(false);
        }
        Ok(self.secondary_press_focused(document)?.is_some())
    }
}
