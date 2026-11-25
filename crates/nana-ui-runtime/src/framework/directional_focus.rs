//! 2D directional focus navigation (explicit 4-way neighbors + lazy spatial-grid
//! geometric fallback) and unified Keyboard/Gamepad navigation intent routing.

use super::*;
use crate::{DirectionalFocusScope, FocusCounters, FocusDirection, FocusNeighbors, FocusPolicy};
use nana_ui_input::{InputDisposition, NavigationInput, NavigationIntent, NavigationSource};

const CONSUMED: InputDisposition = InputDisposition {
    handled: true,
    prevent_default: true,
};

impl AppContext {
    /// Set or clear an explicit [`FocusPolicy`] on `id`.
    pub fn set_focus_policy(
        &mut self,
        id: StableNodeId,
        policy: Option<FocusPolicy>,
    ) -> Result<(), FrameworkError> {
        if !self.world.contains(id) {
            return Err(FrameworkError::MissingView(id));
        }
        let mut mutations = MutationQueue::new();
        mutations.set_focus_policy(id, policy);
        self.commit_mutations(mutations)?;
        Ok(())
    }

    /// Set explicit 4-way [`FocusNeighbors`] on `id`.
    pub fn set_focus_neighbors(
        &mut self,
        id: StableNodeId,
        neighbors: FocusNeighbors,
    ) -> Result<(), FrameworkError> {
        if !self.world.contains(id) {
            return Err(FrameworkError::MissingView(id));
        }
        let mut mutations = MutationQueue::new();
        mutations.set_focus_neighbors(id, neighbors);
        self.commit_mutations(mutations)?;
        Ok(())
    }

    /// Set or clear a single directional neighbor of `id`.
    pub fn set_focus_neighbor(
        &mut self,
        id: StableNodeId,
        direction: FocusDirection,
        target: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        let mut neighbors = self.world.focus_neighbors(id);
        neighbors.set(direction, target);
        self.set_focus_neighbors(id, neighbors)
    }

    /// Declare or remove a [`DirectionalFocusScope`] on `id`.
    pub fn set_directional_focus(
        &mut self,
        id: StableNodeId,
        scope: Option<DirectionalFocusScope>,
    ) -> Result<(), FrameworkError> {
        if !self.world.contains(id) {
            return Err(FrameworkError::MissingView(id));
        }
        let mut mutations = MutationQueue::new();
        mutations.set_directional_focus(id, scope);
        self.commit_mutations(mutations)?;
        Ok(())
    }

    /// Cumulative focus and directional navigation diagnostic counters.
    pub fn focus_counters(&self) -> FocusCounters {
        self.world.focus_counters()
    }

    /// Move focus in `direction` using explicit neighbors first, then 2D
    /// spatial geometric fallback.
    pub fn navigate_directional_focus(
        &mut self,
        document: DocumentId,
        direction: FocusDirection,
    ) -> Result<bool, FrameworkError> {
        let modal_root = self
            .active_runtime_overlays(document)
            .into_iter()
            .find(|overlay| overlay.kind.traps_focus())
            .map(|overlay| overlay.root);

        let current = self.world.focused(document).filter(|id| {
            self.world.is_mounted(*id) && self.world.document_of(*id) == Some(document)
        });

        let Some(origin) = current else {
            let index = self.world.take_spatial_focus_index(document, modal_root);
            let (first, considered) = index.first_matching(|entry| {
                self.is_roving_tab_stop(entry.id)
                    && !self.confirm_busy_action_subtree(entry.id)
                    && entry.clipped_by_scroll.is_none()
            });
            self.world.put_spatial_focus_index(document, index);
            if let Some(target) = first {
                self.world.record_focus_navigation_query(considered);
                return self.focus_directional_target(document, target);
            }
            let candidates = self.sequential_focus_candidates(document);
            let mut scanned = considered;
            for id in candidates {
                scanned += 1;
                if modal_root.is_none_or(|root| self.overlay_descendant(root, id)) {
                    self.world.record_focus_navigation_query(scanned);
                    return self.focus_directional_target(document, id);
                }
            }
            self.world.record_focus_navigation_query(scanned);
            return Ok(false);
        };

        let scope_root = self.confining_directional_scope(origin, modal_root);

        // 1. Explicit neighbor chain (with cycle guard, max 16 hops).
        let mut hops = 0u64;
        let mut cursor = origin;
        let mut visited = [origin; 17];
        let mut visited_len = 1usize;
        while hops < 16 {
            let Some(next) = self.world.focus_neighbors(cursor).get(direction) else {
                break;
            };
            if visited[..visited_len].contains(&next) {
                break;
            }
            visited[visited_len] = next;
            visited_len += 1;
            hops += 1;
            if self.explicit_directional_candidate(document, next, modal_root, scope_root) {
                self.world.record_focus_navigation_query(hops);
                return self.focus_directional_target(document, next);
            }
            cursor = next;
        }

        // 2. 2D geometric fallback using lazy spatial grid index.
        let origin_rect = self
            .world
            .viewport_layout_box(origin)
            .or_else(|| self.world.canonical_layout_box(origin));
        let Some(origin_rect) = origin_rect else {
            self.world.record_focus_navigation_query(hops);
            return Ok(false);
        };

        let mut origin_scroll_ancestors = Vec::new();
        let mut ancestor = self.world.parent_id(origin);
        while let Some(id) = ancestor {
            if self.world.is_scroll_container(id) {
                origin_scroll_ancestors.push(id);
            }
            ancestor = self.world.parent_id(id);
        }

        let mut index = self.world.take_spatial_focus_index(document, modal_root);
        let (target, considered) = index.query(origin, origin_rect, direction, |entry| {
            if let Some(scope) = scope_root
                && !self.world.is_descendant_or_self(entry.id, scope)
            {
                return false;
            }
            if let Some(scroll_id) = entry.clipped_by_scroll
                && !origin_scroll_ancestors.contains(&scroll_id)
            {
                return false;
            }
            self.is_roving_tab_stop(entry.id) && !self.confirm_busy_action_subtree(entry.id)
        });
        self.world.put_spatial_focus_index(document, index);
        self.world.record_focus_navigation_query(hops + considered);

        match target {
            Some(target) => self.focus_directional_target(document, target),
            None => Ok(false),
        }
    }

    fn focus_directional_target(
        &mut self,
        document: DocumentId,
        target: StableNodeId,
    ) -> Result<bool, FrameworkError> {
        if self.focus_node(document, target)? {
            if let Some(select_entry) = self
                .behavior(target)
                .and_then(|b| b.pointer_ops)
                .and_then(|p| p.select_on_focus_entry)
            {
                select_entry(self, document, target, None)?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn confining_directional_scope(
        &self,
        origin: StableNodeId,
        modal_root: Option<StableNodeId>,
    ) -> Option<StableNodeId> {
        let mut current = Some(origin);
        while let Some(id) = current {
            if self
                .world
                .directional_focus_scope(id)
                .is_some_and(|scope| scope.contain)
            {
                return Some(id);
            }
            if modal_root == Some(id) {
                break;
            }
            current = self.world.parent_id(id);
        }
        None
    }

    fn explicit_directional_candidate(
        &self,
        document: DocumentId,
        id: StableNodeId,
        modal_root: Option<StableNodeId>,
        scope_root: Option<StableNodeId>,
    ) -> bool {
        self.world.is_mounted(id)
            && self.world.document_of(id) == Some(document)
            && self.world.focus_policy(id).allows_directional()
            && !self.world.is_inert(id)
            && !self.world.motion_blocks_input(id)
            && !self.confirm_busy_action_subtree(id)
            && self.world.is_overlay_reachable(id)
            && modal_root.is_none_or(|root| self.overlay_descendant(root, id))
            && scope_root.is_none_or(|scope| self.world.is_descendant_or_self(id, scope))
    }

    /// Keyboard arrow fallback for directional navigation: only activates when
    /// the focused node has an explicit neighbor in that direction or any
    /// [`DirectionalFocusScope`] is registered in the tree (zero cost otherwise).
    pub(super) fn keyboard_directional_focus_key(
        &mut self,
        document: DocumentId,
        key: &str,
    ) -> Result<bool, FrameworkError> {
        let direction = match key {
            "ArrowUp" => FocusDirection::Up,
            "ArrowDown" => FocusDirection::Down,
            "ArrowLeft" => FocusDirection::Left,
            "ArrowRight" => FocusDirection::Right,
            _ => return Ok(false),
        };
        let has_explicit = self
            .world
            .focused(document)
            .is_some_and(|focused| self.world.focus_neighbors(focused).get(direction).is_some());
        if !has_explicit && !self.world.has_directional_focus_scopes() {
            return Ok(false);
        }
        if !has_explicit {
            // Check whether current focus (or any node in this document when
            // unfocused) sits inside a declared DirectionalFocusScope.
            let in_scope = match self.world.focused(document) {
                Some(focused) => {
                    let mut cursor = Some(focused);
                    let mut found = false;
                    while let Some(id) = cursor {
                        if self.world.directional_focus_scope(id).is_some() {
                            found = true;
                            break;
                        }
                        cursor = self.world.parent_id(id);
                    }
                    found
                }
                None => true,
            };
            if !in_scope {
                return Ok(false);
            }
        }
        self.navigate_directional_focus(document, direction)
    }

    /// Dispatch a canonical [`NavigationInput`] intent (Keyboard or Gamepad).
    pub(super) fn dispatch_navigation(
        &mut self,
        document: DocumentId,
        nav: NavigationInput,
    ) -> Result<InputDisposition, FrameworkError> {
        let modality = match nav.source {
            NavigationSource::Gamepad => crate::InputModality::Gamepad,
            NavigationSource::Keyboard => crate::InputModality::Keyboard,
        };
        self.world.note_input_modality(document, modality);
        if self.editor_composing(document) {
            return Ok(CONSUMED);
        }
        let keyboard_barrier = self.has_blocking_runtime_overlay(document);
        let handled = match nav.intent {
            NavigationIntent::Move(direction) => {
                self.navigate_directional_focus(document, direction)?
            }
            NavigationIntent::Next | NavigationIntent::Previous => {
                let reverse = matches!(nav.intent, NavigationIntent::Previous);
                if self.route_overlay_key(document, OverlayKey::Tab { reverse })? {
                    true
                } else {
                    self.navigate_sequential_focus(document, reverse)?
                }
            }
            NavigationIntent::Confirm => {
                if nav.repeat {
                    false
                } else if let Some(target) = self.world.focused(document) {
                    let allowed = self.world.is_overlay_reachable(target)
                        && !self.world.is_inert(target)
                        && !self.world.motion_blocks_input(target);
                    if allowed {
                        self.activate_node(target)?
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            NavigationIntent::Cancel => {
                if nav.repeat {
                    false
                } else if self.route_overlay_key(document, OverlayKey::Escape)? {
                    true
                } else if let Some(popover) = self.capabilities.popover
                    && (popover.dismiss_on_escape)(self)?
                {
                    true
                } else {
                    let mut cancelled = false;
                    for i in 0..self.capabilities.edit_cancellers.len() {
                        let cancel = self.capabilities.edit_cancellers[i];
                        if cancel(self, document)? {
                            cancelled = true;
                            break;
                        }
                    }
                    cancelled
                }
            }
        };
        Ok(InputDisposition {
            handled,
            prevent_default: handled || keyboard_barrier,
        })
    }
}
