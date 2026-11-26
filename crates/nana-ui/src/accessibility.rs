use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

#[cfg(all(feature = "hosted", not(target_os = "android")))]
use std::collections::VecDeque;

#[derive(Default)]
struct IdHasher(u64);

impl Hasher for IdHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(byte)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

type FastHashMap<K, V> = HashMap<K, V, BuildHasherDefault<IdHasher>>;
type FastHashSet<K> = HashSet<K, BuildHasherDefault<IdHasher>>;

use accesskit::ActionData;
use accesskit::{
    Action, Affine, HasPopup, Invalid, Live, Node, NodeId, Orientation, Rect, Role, ScrollUnit,
    TextDirection, TextPosition, TextSelection as AccessKitTextSelection, Toggled, TreeId,
    TreeInfo, TreeUpdate,
};
#[cfg(all(feature = "hosted", not(target_os = "android")))]
use nana_ui_runtime::AccessibilityUpdate;
use nana_ui_runtime::{
    AccessibilityDelta, AccessibilityLive, AccessibilityNode, AccessibilityRole,
    AccessibilityScrollDirection, SelectionOrientation, StableNodeId,
};

use accesskit::ActionRequest;
#[cfg(all(
    feature = "hosted",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
use accesskit::DeactivationHandler;
#[cfg(all(feature = "hosted", not(target_os = "android")))]
use accesskit::{ActionHandler, ActivationHandler};
#[cfg(all(feature = "hosted", not(target_os = "android")))]
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Stateful conversion from NanaUI's backend-neutral semantics to AccessKit.
///
/// The retained cache is necessary because AccessKit removals are expressed by
/// replacing a parent's child list, while NanaUI deltas also carry explicit
/// tombstones. The cache lets one runtime transaction become one coherent
/// platform update without leaking AccessKit into `nana-ui-runtime`.
pub(crate) struct AccessibilityProjector {
    nodes: FastHashMap<StableNodeId, AccessibilityNode>,
    text_runs: FastHashMap<StableNodeId, NodeId>,
    text_runs_by_run_id: FastHashMap<NodeId, StableNodeId>,
    next_text_run_id: u64,
    roots: BTreeSet<StableNodeId>,
    focused: BTreeSet<StableNodeId>,
    tree_links_exact: bool,
    interactive: bool,
    scale_factor: f32,
    generation: Option<u64>,
    window_root: bool,
    #[cfg(test)]
    full_updates: std::cell::Cell<usize>,
    #[cfg(test)]
    global_reconciliations: std::cell::Cell<usize>,
}

impl AccessibilityProjector {
    #[cfg(test)]
    fn new(
        nodes: Vec<AccessibilityNode>,
        interactive: bool,
        scale_factor: f32,
    ) -> (Self, TreeUpdate) {
        Self::new_at_generation(nodes, interactive, scale_factor, None)
    }

    #[cfg(test)]
    fn new_at_generation(
        nodes: Vec<AccessibilityNode>,
        interactive: bool,
        scale_factor: f32,
        generation: Option<u64>,
    ) -> (Self, TreeUpdate) {
        let projector = Self::retain(nodes, interactive, scale_factor, generation, false);
        let update = projector.full_update();
        (projector, update)
    }

    pub(crate) fn retain(
        nodes: Vec<AccessibilityNode>,
        interactive: bool,
        scale_factor: f32,
        generation: Option<u64>,
        window_root: bool,
    ) -> Self {
        let mut node_map = FastHashMap::with_capacity_and_hasher(nodes.len(), Default::default());
        for node in nodes {
            node_map.insert(node.id, node);
        }
        let roots = runtime_roots(&node_map);
        let focused = focused_nodes(&node_map);
        let tree_links_exact = check_tree_links_exact(&node_map, &roots);
        let mut projector = Self {
            nodes: node_map,
            text_runs: FastHashMap::default(),
            text_runs_by_run_id: FastHashMap::default(),
            next_text_run_id: u64::MAX,
            roots,
            focused,
            tree_links_exact,
            interactive,
            scale_factor: scale_factor.max(0.01),
            generation,
            window_root,
            #[cfg(test)]
            full_updates: std::cell::Cell::new(0),
            #[cfg(test)]
            global_reconciliations: std::cell::Cell::new(0),
        };
        projector.reconcile_text_runs();
        projector
    }

    pub(crate) fn apply_delta(&mut self, delta: AccessibilityDelta) -> Option<TreeUpdate> {
        if self
            .generation
            .is_some_and(|generation| delta.generation <= generation)
        {
            return None;
        }
        self.generation = Some(delta.generation);
        Some(self.apply(delta))
    }

    pub(crate) fn synchronize_full(
        &mut self,
        nodes: Vec<AccessibilityNode>,
        scale_factor: f32,
        generation: Option<u64>,
    ) -> Option<TreeUpdate> {
        let scale_factor = scale_factor.max(0.01);
        if generation.is_some_and(|next| self.generation.is_some_and(|current| next <= current)) {
            if (scale_factor - self.scale_factor).abs() > f32::EPSILON {
                self.scale_factor = scale_factor;
                return Some(self.full_update());
            }
            return None;
        }
        if generation.is_some() {
            self.generation = generation;
        }
        self.synchronize(nodes, scale_factor)
    }

    fn apply_property_only(&mut self, updated: Vec<AccessibilityNode>) -> TreeUpdate {
        if updated.len() == 1 {
            let node = updated.into_iter().next().expect("single updated node");
            if node.focused {
                self.focused.insert(node.id);
            } else {
                self.focused.remove(&node.id);
            }
            let id = node.id;
            let is_text_input = node.role == AccessibilityRole::TextInput;
            let slot = self.nodes.get_mut(&id).expect("existing semantic node");
            *slot = node;
            let mut out = Vec::with_capacity(if is_text_input { 2 } else { 1 });
            let text_run_id = is_text_input
                .then(|| self.text_runs.get(&id).copied())
                .flatten();
            project_node_into(
                slot,
                text_run_id,
                self.interactive,
                self.scale_factor,
                &mut out,
            );
            return TreeUpdate {
                nodes: out,
                tree: None,
                tree_id: TreeId::ROOT,
                focus: self.focused_node_id(),
            };
        }

        let mut seen = FastHashSet::with_capacity_and_hasher(updated.len(), Default::default());
        let mut unique_ids = Vec::with_capacity(updated.len());
        for node in updated {
            if node.focused {
                self.focused.insert(node.id);
            } else {
                self.focused.remove(&node.id);
            }
            let id = node.id;
            if seen.insert(id) {
                unique_ids.push(id);
            }
            *self.nodes.get_mut(&id).expect("existing semantic node") = node;
        }
        let mut out = Vec::with_capacity(unique_ids.len());
        for id in unique_ids {
            if let Some(node) = self.nodes.get(&id) {
                self.project_entries_into(node, &mut out);
            }
        }
        TreeUpdate {
            nodes: out,
            tree: None,
            tree_id: TreeId::ROOT,
            focus: self.focused_node_id(),
        }
    }

    pub(crate) fn apply(&mut self, delta: AccessibilityDelta) -> TreeUpdate {
        if delta.removed.is_empty() {
            if delta.updated.is_empty() {
                return TreeUpdate {
                    nodes: Vec::new(),
                    tree: None,
                    tree_id: TreeId::ROOT,
                    focus: self.focused_node_id(),
                };
            }
            let property_only = delta.updated.iter().all(|node| {
                self.nodes.get(&node.id).is_some_and(|previous| {
                    previous.parent == node.parent
                        && previous.children == node.children
                        && (previous.role == AccessibilityRole::TextInput)
                            == (node.role == AccessibilityRole::TextInput)
                })
            });
            if property_only {
                return self.apply_property_only(delta.updated);
            }
        }

        let mut incoming =
            FastHashMap::with_capacity_and_hasher(delta.updated.len(), Default::default());
        for node in delta.updated {
            incoming.insert(node.id, node);
        }
        let mut removed = FastHashSet::default();
        if !delta.removed.is_empty() {
            removed.reserve(delta.removed.len());
            for id in delta.removed {
                if !incoming.contains_key(&id) {
                    removed.insert(id);
                }
            }
        }

        let structure_changed = !removed.is_empty()
            || incoming.values().any(|node| {
                self.nodes.get(&node.id).is_none_or(|previous| {
                    previous.parent != node.parent || previous.children != node.children
                })
            });
        let text_roles_changed = structure_changed
            || incoming.values().any(|node| {
                self.nodes.get(&node.id).is_none_or(|previous| {
                    (previous.role == AccessibilityRole::TextInput)
                        != (node.role == AccessibilityRole::TextInput)
                })
            });

        let previous_roots = structure_changed.then(|| self.roots.clone());
        let incoming_ids: FastHashSet<StableNodeId> = incoming.keys().copied().collect();
        let incoming_old_parents: FastHashMap<StableNodeId, Option<StableNodeId>> =
            if structure_changed {
                incoming_ids
                    .iter()
                    .filter_map(|&id| self.nodes.get(&id).map(|node| (id, node.parent)))
                    .collect()
            } else {
                FastHashMap::default()
            };
        let mut changed = FastHashSet::with_capacity_and_hasher(
            incoming.len() + removed.len() + 2,
            Default::default(),
        );
        changed.extend(incoming_ids.iter().copied());

        let mut unlinked_from_parent: FastHashMap<StableNodeId, Vec<StableNodeId>> =
            FastHashMap::default();
        let mut dropped_edges: Vec<(StableNodeId, StableNodeId)> = Vec::new();
        let mut added_edges: FastHashSet<(StableNodeId, StableNodeId)> = FastHashSet::default();
        let mut candidates: FastHashSet<StableNodeId> = FastHashSet::default();
        let mut reparented_or_new: Vec<(
            StableNodeId,
            Option<StableNodeId>,
            Option<StableNodeId>,
            bool,
        )> = Vec::new();

        for &id in &removed {
            if let Some(old_node) = self.nodes.remove(&id) {
                self.focused.remove(&id);
                self.roots.remove(&id);
                self.release_text_run(id);
                changed.remove(&id);
                if let Some(parent_id) = old_node.parent
                    && !removed.contains(&parent_id)
                {
                    changed.insert(parent_id);
                    unlinked_from_parent.entry(parent_id).or_default().push(id);
                }
                for child_id in old_node.children {
                    if !removed.contains(&child_id) {
                        dropped_edges.push((id, child_id));
                        candidates.insert(child_id);
                    }
                }
            }
        }

        for (id, node) in incoming {
            let (was_new, old_parent) = match self.nodes.get(&id) {
                Some(previous) => {
                    let old_parent = previous.parent;
                    if structure_changed && previous.children != node.children {
                        let (dropped, added, has_duplicates) =
                            diff_children(&previous.children, &node.children);
                        if has_duplicates {
                            self.tree_links_exact = false;
                        }
                        let mut dropped_local = FastHashSet::default();
                        for child_id in dropped {
                            dropped_local.insert(child_id);
                            if !removed.contains(&child_id) {
                                dropped_edges.push((id, child_id));
                                candidates.insert(child_id);
                            }
                        }
                        for child_id in added {
                            if !added_edges.insert((id, child_id)) {
                                self.tree_links_exact = false;
                            } else if self.tree_links_exact && !dropped_local.contains(&child_id) {
                                let child_old_parent =
                                    if let Some(&old) = incoming_old_parents.get(&child_id) {
                                        old
                                    } else if incoming_ids.contains(&child_id) {
                                        None
                                    } else {
                                        self.nodes.get(&child_id).and_then(|node| node.parent)
                                    };
                                if child_old_parent == Some(id) {
                                    // `child_id` was already a child of `id` in the unchanged prefix/suffix,
                                    // meaning `node.children` contains a duplicate entry.
                                    self.tree_links_exact = false;
                                }
                            }
                        }
                    }
                    (false, old_parent)
                }
                None => {
                    if structure_changed {
                        for &child_id in &node.children {
                            if !added_edges.insert((id, child_id)) {
                                self.tree_links_exact = false;
                            }
                        }
                    }
                    (true, None)
                }
            };

            if was_new {
                if node.parent.is_none() {
                    self.roots.insert(id);
                }
            } else if old_parent.is_none() && node.parent.is_some() {
                self.roots.remove(&id);
                candidates.insert(id);
            } else if old_parent.is_some() && node.parent.is_none() {
                self.roots.insert(id);
            }

            if structure_changed && (was_new || old_parent != node.parent) {
                reparented_or_new.push((id, old_parent, node.parent, was_new));
                if node.parent.is_some() {
                    candidates.insert(id);
                }
                if let Some(old_p) = old_parent
                    && !removed.contains(&old_p)
                {
                    changed.insert(old_p);
                    if !incoming_ids.contains(&old_p) {
                        unlinked_from_parent.entry(old_p).or_default().push(id);
                    }
                }
            }

            if node.focused {
                self.focused.insert(id);
            } else {
                self.focused.remove(&id);
            }
            self.nodes.insert(id, node);
        }

        if structure_changed {
            for (parent_id, to_unlink) in unlinked_from_parent {
                if let Some(parent) = self.nodes.get_mut(&parent_id) {
                    let before = parent.children.len();
                    if to_unlink.len() == 1 {
                        let target = to_unlink[0];
                        parent.children.retain(|child| *child != target);
                    } else {
                        let unlink_set: FastHashSet<_> = to_unlink.into_iter().collect();
                        parent.children.retain(|child| !unlink_set.contains(child));
                    }
                    if parent.children.len() != before {
                        changed.insert(parent_id);
                    }
                }
            }

            for &(child_id, old_parent, new_parent, was_new) in &reparented_or_new {
                if let Some(old_p) = old_parent
                    && Some(old_p) != new_parent
                    && incoming_ids.contains(&old_p)
                    && !dropped_edges.contains(&(old_p, child_id))
                    && let Some(parent) = self.nodes.get_mut(&old_p)
                {
                    let before = parent.children.len();
                    parent.children.retain(|child| *child != child_id);
                    if parent.children.len() != before {
                        changed.insert(old_p);
                        dropped_edges.push((old_p, child_id));
                    }
                }
                if let Some(parent_id) = new_parent
                    && !incoming_ids.contains(&parent_id)
                    && let Some(parent) = self.nodes.get_mut(&parent_id)
                {
                    let already_present = if was_new && self.tree_links_exact {
                        false
                    } else {
                        parent.children.contains(&child_id)
                    };
                    if !already_present {
                        parent.children.push(child_id);
                        changed.insert(parent_id);
                        added_edges.insert((parent_id, child_id));
                    }
                }
            }

            // Strip dangling children referenced by added edges and verify tree link exactness.
            let mut dangling_by_parent: FastHashMap<StableNodeId, Vec<StableNodeId>> =
                FastHashMap::default();
            for &(parent_id, child_id) in &added_edges {
                match self.nodes.get(&child_id) {
                    None => {
                        dangling_by_parent
                            .entry(parent_id)
                            .or_default()
                            .push(child_id);
                    }
                    Some(child) if child.parent != Some(parent_id) => {
                        self.tree_links_exact = false;
                    }
                    Some(_) => {}
                }
            }
            for (parent_id, dangling) in dangling_by_parent {
                if let Some(parent) = self.nodes.get_mut(&parent_id) {
                    let before = parent.children.len();
                    if dangling.len() == 1 {
                        let target = dangling[0];
                        parent.children.retain(|child| *child != target);
                    } else {
                        let dangling_set: FastHashSet<_> = dangling.into_iter().collect();
                        parent
                            .children
                            .retain(|child| !dangling_set.contains(child));
                    }
                    if parent.children.len() != before {
                        changed.insert(parent_id);
                    }
                }
            }

            if self.tree_links_exact {
                self.drop_unreachable_local(
                    &candidates,
                    &dropped_edges,
                    &reparented_or_new,
                    &incoming_ids,
                    &added_edges,
                    &mut changed,
                );
            } else {
                self.drop_unreachable_global(&mut changed);
            }
        }

        if text_roles_changed {
            self.reconcile_text_runs_incremental(&incoming_ids, &mut changed);
        }

        if let Some(previous_roots) = previous_roots
            && self.roots != previous_roots
        {
            return self.full_update();
        }

        // Updating a parent removes stale AccessKit subtrees. Do not also ship
        // unreachable descendants: AccessKit would keep their children with a
        // dangling parent_and_index.
        let mut out = Vec::with_capacity(changed.len() + 2);
        for id in changed {
            if let Some(node) = self.nodes.get(&id) {
                self.project_entries_into(node, &mut out);
            }
        }
        TreeUpdate {
            nodes: out,
            tree: None,
            tree_id: TreeId::ROOT,
            focus: self.focused_node_id(),
        }
    }

    fn is_reachable_local(
        &self,
        start: StableNodeId,
        broken_parent_link: &FastHashSet<StableNodeId>,
        known_reachable: &mut FastHashSet<StableNodeId>,
        known_unreachable: &mut FastHashSet<StableNodeId>,
        walk_path: &mut Vec<StableNodeId>,
        walk_set: &mut FastHashSet<StableNodeId>,
    ) -> bool {
        walk_path.clear();
        walk_set.clear();
        let mut cur = start;
        let reachable = loop {
            if known_reachable.contains(&cur) {
                break true;
            }
            if known_unreachable.contains(&cur) {
                break false;
            }
            let in_cycle = if walk_path.len() < 16 {
                walk_path.contains(&cur)
            } else {
                walk_set.contains(&cur)
            };
            if in_cycle {
                break false;
            }
            let Some(node) = self.nodes.get(&cur) else {
                break false;
            };
            match node.parent {
                None => break true,
                Some(parent) => {
                    if broken_parent_link.contains(&cur) {
                        break false;
                    }
                    if walk_path.len() == 16 {
                        walk_set.extend(walk_path.iter().copied());
                    }
                    if walk_path.len() >= 16 {
                        walk_set.insert(cur);
                    }
                    walk_path.push(cur);
                    cur = parent;
                }
            }
        };
        if reachable {
            known_reachable.extend(walk_path.iter().copied());
        } else {
            known_unreachable.extend(walk_path.iter().copied());
        }
        reachable
    }

    fn drop_unreachable_local(
        &mut self,
        candidates: &FastHashSet<StableNodeId>,
        dropped_edges: &[(StableNodeId, StableNodeId)],
        reparented_or_new: &[(
            StableNodeId,
            Option<StableNodeId>,
            Option<StableNodeId>,
            bool,
        )],
        incoming_ids: &FastHashSet<StableNodeId>,
        added_edges: &FastHashSet<(StableNodeId, StableNodeId)>,
        changed: &mut FastHashSet<StableNodeId>,
    ) {
        if candidates.is_empty() {
            return;
        }
        let mut broken_parent_link = FastHashSet::default();
        for &(old_parent, child_id) in dropped_edges {
            if self
                .nodes
                .get(&child_id)
                .is_some_and(|child| child.parent == Some(old_parent))
            {
                broken_parent_link.insert(child_id);
            }
        }
        for &(child_id, _old_parent, new_parent, _was_new) in reparented_or_new {
            if let Some(parent_id) = new_parent
                && (!self.nodes.contains_key(&parent_id)
                    || (incoming_ids.contains(&parent_id)
                        && !added_edges.contains(&(parent_id, child_id))))
            {
                broken_parent_link.insert(child_id);
            }
        }

        let mut known_reachable = FastHashSet::default();
        let mut known_unreachable = FastHashSet::default();
        let mut prune_stack = Vec::new();
        let mut walk_path = Vec::new();
        let mut walk_set = FastHashSet::default();
        for &cand in candidates {
            if !self.nodes.contains_key(&cand) {
                continue;
            }
            if !self.is_reachable_local(
                cand,
                &broken_parent_link,
                &mut known_reachable,
                &mut known_unreachable,
                &mut walk_path,
                &mut walk_set,
            ) {
                prune_stack.push(cand);
            }
        }
        while let Some(id) = prune_stack.pop() {
            let Some(pruned) = self.nodes.remove(&id) else {
                continue;
            };
            self.focused.remove(&id);
            self.roots.remove(&id);
            self.release_text_run(id);
            changed.remove(&id);
            for child_id in pruned.children {
                if self
                    .nodes
                    .get(&child_id)
                    .is_some_and(|child| child.parent == Some(id))
                {
                    prune_stack.push(child_id);
                }
            }
        }
    }

    fn drop_unreachable_global(&mut self, changed: &mut FastHashSet<StableNodeId>) {
        #[cfg(test)]
        self.global_reconciliations
            .set(self.global_reconciliations.get() + 1);
        let mut keep = FastHashSet::with_capacity_and_hasher(self.nodes.len(), Default::default());
        let mut stack: Vec<StableNodeId> = self.roots.iter().copied().collect();
        while let Some(id) = stack.pop() {
            if !keep.insert(id) {
                continue;
            }
            if let Some(node) = self.nodes.get(&id) {
                for &child in &node.children {
                    if self.nodes.contains_key(&child) {
                        stack.push(child);
                    }
                }
            }
        }
        if keep.len() != self.nodes.len() {
            let mut pruned = Vec::with_capacity(self.nodes.len() - keep.len());
            for &id in self.nodes.keys() {
                if !keep.contains(&id) {
                    pruned.push(id);
                }
            }
            for id in pruned {
                self.nodes.remove(&id);
                self.focused.remove(&id);
                self.roots.remove(&id);
                self.release_text_run(id);
                changed.remove(&id);
            }
        }
        for node in self.nodes.values_mut() {
            let before = node.children.len();
            node.children.retain(|child| keep.contains(child));
            if node.children.len() != before {
                changed.insert(node.id);
            }
        }
        self.tree_links_exact = check_tree_links_exact(&self.nodes, &self.roots);
    }

    fn synchronize(
        &mut self,
        nodes: Vec<AccessibilityNode>,
        scale_factor: f32,
    ) -> Option<TreeUpdate> {
        let mut next = FastHashMap::with_capacity_and_hasher(nodes.len(), Default::default());
        let mut roots = BTreeSet::new();
        for node in nodes {
            let id = node.id;
            let is_root = node.parent.is_none();
            if is_root {
                roots.insert(id);
            } else {
                roots.remove(&id);
            }
            next.insert(id, node);
        }
        let scale_factor = scale_factor.max(0.01);
        if (scale_factor - self.scale_factor).abs() > f32::EPSILON || roots != self.roots {
            self.nodes = next;
            self.focused = focused_nodes(&self.nodes);
            self.scale_factor = scale_factor;
            self.roots = roots;
            self.tree_links_exact = check_tree_links_exact(&self.nodes, &self.roots);
            self.reconcile_text_runs();
            return Some(self.full_update());
        }
        let removed = self
            .nodes
            .keys()
            .filter(|id| !next.contains_key(id))
            .copied()
            .collect::<Vec<_>>();
        let updated = next
            .into_values()
            .filter(|node| self.nodes.get(&node.id) != Some(node))
            .collect::<Vec<_>>();
        if updated.is_empty() && removed.is_empty() {
            return None;
        }
        Some(self.apply(AccessibilityDelta {
            generation: 0,
            updated,
            removed,
        }))
    }

    pub(crate) fn full_update(&self) -> TreeUpdate {
        #[cfg(test)]
        self.full_updates.set(self.full_updates.get() + 1);
        let has_synthetic_root = self.window_root || self.roots.len() != 1;
        let mut nodes = Vec::with_capacity(
            self.nodes.len() + self.text_runs.len() + usize::from(has_synthetic_root),
        );
        for node in self.nodes.values() {
            self.project_entries_into(node, &mut nodes);
        }
        if has_synthetic_root {
            let mut root = Node::new(if self.window_root {
                Role::Window
            } else {
                Role::GenericContainer
            });
            if !self.roots.is_empty() {
                root.set_children(self.roots.iter().copied().map(node_id).collect::<Vec<_>>());
            }
            nodes.push((FOREST_ROOT_ID, root));
        }
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(self.tree_root_id())),
            tree_id: TreeId::ROOT,
            focus: self.focused_node_id(),
        }
    }

    fn tree_root_id(&self) -> NodeId {
        if !self.window_root && self.roots.len() == 1 {
            node_id(*self.roots.first().expect("single root"))
        } else {
            FOREST_ROOT_ID
        }
    }

    fn focused_node_id(&self) -> NodeId {
        // AccessKit has one focus id. A context-menu row is a virtual
        // descendant: keyboard focus stays on the trigger, and the highlighted
        // row is also marked focused. Virtual ids sort after retained ids, so
        // the lowest id would leave the platform focus on the trigger. A tree
        // row is the same under its focused tree; an open menu wins over it.
        let id = self
            .focused
            .iter()
            .rev()
            .find(|id| nana_ui_runtime::scene_contract::decode_virtual_menu_item(**id).is_some())
            .or_else(|| {
                self.focused.iter().rev().find(|id| {
                    nana_ui_runtime::scene_contract::decode_virtual_tree_item(**id).is_some()
                })
            })
            .or_else(|| self.focused.first())
            .copied();
        id.map(node_id).unwrap_or_else(|| {
            if self.window_root {
                FOREST_ROOT_ID
            } else {
                self.roots.first().copied().map_or(FOREST_ROOT_ID, node_id)
            }
        })
    }

    fn allocate_text_run(&mut self, id: StableNodeId) -> NodeId {
        loop {
            let candidate = self.next_text_run_id;
            self.next_text_run_id = candidate.wrapping_sub(1);
            if candidate == 0 {
                continue;
            }
            let run_id = NodeId(candidate);
            let collides_with_node =
                StableNodeId::new(candidate).is_some_and(|stable| self.nodes.contains_key(&stable));
            if !collides_with_node && !self.text_runs_by_run_id.contains_key(&run_id) {
                self.text_runs.insert(id, run_id);
                self.text_runs_by_run_id.insert(run_id, id);
                return run_id;
            }
        }
    }

    fn release_text_run(&mut self, id: StableNodeId) -> Option<NodeId> {
        let run_id = self.text_runs.remove(&id)?;
        self.text_runs_by_run_id.remove(&run_id);
        Some(run_id)
    }

    fn reconcile_text_runs_incremental(
        &mut self,
        incoming_ids: &FastHashSet<StableNodeId>,
        changed: &mut FastHashSet<StableNodeId>,
    ) {
        let mut needs_text_run = Vec::new();
        for &id in incoming_ids {
            let Some(is_text_input) = self
                .nodes
                .get(&id)
                .map(|node| node.role == AccessibilityRole::TextInput)
            else {
                continue;
            };
            if !is_text_input {
                self.release_text_run(id);
            }
            if let Some(owner_id) = self.text_runs_by_run_id.remove(&node_id(id)) {
                self.text_runs.remove(&owner_id);
                needs_text_run.push(owner_id);
            }
            if is_text_input && !self.text_runs.contains_key(&id) {
                needs_text_run.push(id);
            }
        }
        for id in needs_text_run {
            if self
                .nodes
                .get(&id)
                .is_some_and(|node| node.role == AccessibilityRole::TextInput)
                && !self.text_runs.contains_key(&id)
            {
                self.allocate_text_run(id);
                changed.insert(id);
            }
        }
    }

    fn reconcile_text_runs(&mut self) {
        #[cfg(test)]
        self.global_reconciliations
            .set(self.global_reconciliations.get() + 1);
        self.text_runs.retain(|id, text_run| {
            let valid = self
                .nodes
                .get(id)
                .is_some_and(|node| node.role == AccessibilityRole::TextInput)
                && StableNodeId::new(text_run.0)
                    .is_none_or(|stable| !self.nodes.contains_key(&stable));
            if !valid {
                self.text_runs_by_run_id.remove(text_run);
            }
            valid
        });
        let missing = self
            .nodes
            .values()
            .filter(|node| {
                node.role == AccessibilityRole::TextInput && !self.text_runs.contains_key(&node.id)
            })
            .map(|node| node.id)
            .collect::<Vec<_>>();
        for id in missing {
            self.allocate_text_run(id);
        }
    }

    fn project_entries_into(&self, node: &AccessibilityNode, out: &mut Vec<(NodeId, Node)>) {
        let text_run_id = (node.role == AccessibilityRole::TextInput)
            .then(|| self.text_runs.get(&node.id).copied())
            .flatten();
        project_node_into(node, text_run_id, self.interactive, self.scale_factor, out);
    }

    pub(crate) fn project_action_request(
        &self,
        request: ActionRequest,
    ) -> Option<nana_ui_runtime::AccessibilityActionRequest> {
        if request.target_tree != TreeId::ROOT {
            return None;
        }
        let target = StableNodeId::new(request.target_node.0)?;
        let node = self.nodes.get(&target)?;
        if !self.interactive || node.disabled || node.inert {
            return None;
        }
        if let Some((menu, slot)) =
            nana_ui_runtime::scene_contract::decode_virtual_menu_item(target)
        {
            if !is_menu_row(node.role) {
                return None;
            }
            let action = match request.action {
                Action::Click => {
                    nana_ui_runtime::AccessibilityAction::ActivateMenuItem { menu, slot }
                }
                Action::Focus => nana_ui_runtime::AccessibilityAction::Focus,
                _ => return None,
            };
            return Some(nana_ui_runtime::AccessibilityActionRequest { target, action });
        }
        if nana_ui_runtime::scene_contract::decode_virtual_tree_item(target).is_some() {
            if node.role != AccessibilityRole::TreeItem {
                return None;
            }
            let action = match request.action {
                Action::Click => nana_ui_runtime::AccessibilityAction::Click,
                Action::Focus => nana_ui_runtime::AccessibilityAction::Focus,
                Action::Expand if supports_expand(node) => {
                    nana_ui_runtime::AccessibilityAction::Expand
                }
                Action::Collapse if supports_collapse(node) => {
                    nana_ui_runtime::AccessibilityAction::Collapse
                }
                _ => return None,
            };
            return Some(nana_ui_runtime::AccessibilityActionRequest { target, action });
        }
        if nana_ui_runtime::scene_contract::decode_virtual_tree_toggle(target).is_some() {
            if node.role != AccessibilityRole::Checkbox || request.action != Action::Click {
                return None;
            }
            return Some(nana_ui_runtime::AccessibilityActionRequest {
                target,
                action: nana_ui_runtime::AccessibilityAction::Click,
            });
        }
        let action = match request.action {
            Action::Click if supports_click(node.role) && !node.display_only => {
                nana_ui_runtime::AccessibilityAction::Click
            }
            Action::Focus if supports_focus(node.role) && !node.display_only => {
                nana_ui_runtime::AccessibilityAction::Focus
            }
            Action::Increment if supports_numeric_step(node) => {
                nana_ui_runtime::AccessibilityAction::Increment
            }
            Action::Decrement if supports_numeric_step(node) => {
                nana_ui_runtime::AccessibilityAction::Decrement
            }
            Action::Expand if supports_expand(node) => nana_ui_runtime::AccessibilityAction::Expand,
            Action::Collapse if supports_collapse(node) => {
                nana_ui_runtime::AccessibilityAction::Collapse
            }
            Action::ScrollDown if node.scroll_y.is_some() => {
                nana_ui_runtime::AccessibilityAction::Scroll(
                    AccessibilityScrollDirection::Down,
                    scroll_unit(request.data.as_ref()),
                )
            }
            Action::ScrollUp if node.scroll_y.is_some() => {
                nana_ui_runtime::AccessibilityAction::Scroll(
                    AccessibilityScrollDirection::Up,
                    scroll_unit(request.data.as_ref()),
                )
            }
            Action::ScrollLeft if node.scroll_x.is_some() => {
                nana_ui_runtime::AccessibilityAction::Scroll(
                    AccessibilityScrollDirection::Left,
                    scroll_unit(request.data.as_ref()),
                )
            }
            Action::ScrollRight if node.scroll_x.is_some() => {
                nana_ui_runtime::AccessibilityAction::Scroll(
                    AccessibilityScrollDirection::Right,
                    scroll_unit(request.data.as_ref()),
                )
            }
            Action::SetScrollOffset if node.scroll_x.is_some() || node.scroll_y.is_some() => {
                let Some(ActionData::SetScrollOffset(point)) = request.data else {
                    return None;
                };
                if !point.x.is_finite() || !point.y.is_finite() {
                    return None;
                }
                nana_ui_runtime::AccessibilityAction::SetScrollOffset(
                    nana_ui_runtime::ScrollOffset {
                        x: point.x as f32,
                        y: point.y as f32,
                    },
                )
            }
            Action::SetValue if supports_set_value(node) => match request.data {
                Some(ActionData::Value(value)) => {
                    nana_ui_runtime::AccessibilityAction::SetValue(value.into())
                }
                Some(ActionData::NumericValue(value)) if value.is_finite() => {
                    nana_ui_runtime::AccessibilityAction::SetValue(value.to_string())
                }
                _ => return None,
            },
            Action::SetTextSelection => {
                let Some(ActionData::SetTextSelection(selection)) = request.data else {
                    return None;
                };
                let text_run = self.text_runs.get(&target)?;
                projected_text_selection(node, *text_run)?;
                if selection.anchor.node != *text_run || selection.focus.node != *text_run {
                    return None;
                }
                let value = node.value.as_deref().unwrap_or_default();
                nana_ui_runtime::AccessibilityAction::SetSelection(
                    nana_ui_runtime::TextSelection::new(
                        character_index_to_byte(value, selection.anchor.character_index)?,
                        character_index_to_byte(value, selection.focus.character_index)?,
                    ),
                )
            }
            _ => return None,
        };
        Some(nana_ui_runtime::AccessibilityActionRequest { target, action })
    }
}

const FOREST_ROOT_ID: NodeId = NodeId(0);

fn focused_nodes(nodes: &FastHashMap<StableNodeId, AccessibilityNode>) -> BTreeSet<StableNodeId> {
    nodes
        .values()
        .filter(|node| node.focused)
        .map(|node| node.id)
        .collect()
}

fn runtime_roots(nodes: &FastHashMap<StableNodeId, AccessibilityNode>) -> BTreeSet<StableNodeId> {
    nodes
        .values()
        .filter(|node| node.parent.is_none())
        .map(|node| node.id)
        .collect()
}

fn check_tree_links_exact(
    nodes: &FastHashMap<StableNodeId, AccessibilityNode>,
    roots: &BTreeSet<StableNodeId>,
) -> bool {
    let mut visited = FastHashSet::with_capacity_and_hasher(nodes.len(), Default::default());
    let mut stack = Vec::with_capacity(roots.len().max(16));
    for &root_id in roots {
        if !visited.insert(root_id) {
            return false;
        }
        stack.push(root_id);
    }
    while let Some(id) = stack.pop() {
        let Some(node) = nodes.get(&id) else {
            return false;
        };
        for &child_id in &node.children {
            let Some(child) = nodes.get(&child_id) else {
                return false;
            };
            if child.parent != Some(id) || !visited.insert(child_id) {
                return false;
            }
            stack.push(child_id);
        }
    }
    visited.len() == nodes.len()
}

fn diff_children(
    prev: &[StableNodeId],
    next: &[StableNodeId],
) -> (Vec<StableNodeId>, Vec<StableNodeId>, bool) {
    let prefix = prev
        .iter()
        .zip(next.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let prev_rem = &prev[prefix..];
    let next_rem = &next[prefix..];
    let suffix = prev_rem
        .iter()
        .rev()
        .zip(next_rem.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let prev_mid = &prev_rem[..prev_rem.len() - suffix];
    let next_mid = &next_rem[..next_rem.len() - suffix];
    if prev_mid.is_empty() {
        return (Vec::new(), next_mid.to_vec(), false);
    }
    if next_mid.is_empty() {
        return (prev_mid.to_vec(), Vec::new(), false);
    }
    if prev_mid.len() <= 8 && next_mid.len() <= 8 {
        let dropped = prev_mid
            .iter()
            .copied()
            .filter(|id| !next_mid.contains(id))
            .collect::<Vec<_>>();
        let added = next_mid
            .iter()
            .copied()
            .filter(|id| !prev_mid.contains(id))
            .collect::<Vec<_>>();
        let has_duplicates = next_mid.len() != prev_mid.len() - dropped.len() + added.len();
        return (dropped, added, has_duplicates);
    }
    let next_set: FastHashSet<_> = next_mid.iter().copied().collect();
    let prev_set: FastHashSet<_> = prev_mid.iter().copied().collect();
    let dropped = prev_mid
        .iter()
        .copied()
        .filter(|id| !next_set.contains(id))
        .collect::<Vec<_>>();
    let added = next_mid
        .iter()
        .copied()
        .filter(|id| !prev_set.contains(id))
        .collect::<Vec<_>>();
    let has_duplicates = next_set.len() != next_mid.len();
    (dropped, added, has_duplicates)
}

#[cfg(all(feature = "hosted", not(target_os = "android")))]
#[derive(Clone)]
struct CurrentTree {
    projector: Arc<Mutex<AccessibilityProjector>>,
    activated: Arc<AtomicBool>,
    window: Option<Arc<dyn winit::window::Window>>,
}

#[cfg(all(feature = "hosted", not(target_os = "android")))]
impl ActivationHandler for CurrentTree {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.activated.store(true, Ordering::Release);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Some(self.projector.lock().ok()?.full_update())
    }
}

#[cfg(all(feature = "hosted", not(target_os = "android")))]
impl CurrentTree {
    fn unpublished(
        interactive: bool,
        scale_factor: f32,
        window: Option<Arc<dyn winit::window::Window>>,
    ) -> Self {
        Self {
            projector: Arc::new(Mutex::new(AccessibilityProjector::retain(
                Vec::new(),
                interactive,
                scale_factor,
                None,
                true,
            ))),
            activated: Arc::new(AtomicBool::new(false)),
            window,
        }
    }

    #[cfg(test)]
    fn from_projector(projector: Arc<Mutex<AccessibilityProjector>>) -> Self {
        Self {
            projector,
            activated: Arc::new(AtomicBool::new(false)),
            window: None,
        }
    }

    fn synchronize(&self, update: AccessibilityUpdate, scale_factor: f32) -> Option<TreeUpdate> {
        let mut projector = self.projector.lock().expect("accessibility projector");
        match update {
            AccessibilityUpdate::Full { generation, nodes } => {
                projector.synchronize_full(nodes, scale_factor, generation)
            }
            AccessibilityUpdate::Delta(delta) => {
                debug_assert!(
                    (projector.scale_factor - scale_factor.max(0.01)).abs() <= f32::EPSILON
                );
                projector.apply_delta(delta)
            }
        }
    }
}

#[cfg(all(feature = "hosted", not(target_os = "android")))]
struct QueuedActions {
    requests: Arc<Mutex<VecDeque<ActionRequest>>>,
    has_pending: Arc<AtomicBool>,
    window: Arc<dyn winit::window::Window>,
}

#[cfg(all(feature = "hosted", not(target_os = "android")))]
const MAX_PENDING_ACCESSIBILITY_ACTIONS: usize = 256;

#[cfg(all(feature = "hosted", not(target_os = "android")))]
fn enqueue_accessibility_action(
    requests: &mut VecDeque<ActionRequest>,
    request: ActionRequest,
) -> bool {
    if requests.len() >= MAX_PENDING_ACCESSIBILITY_ACTIONS {
        return false;
    }
    requests.push_back(request);
    true
}

#[cfg(all(feature = "hosted", not(target_os = "android")))]
impl ActionHandler for QueuedActions {
    fn do_action(&mut self, request: ActionRequest) {
        if let Ok(mut requests) = self.requests.lock()
            && enqueue_accessibility_action(&mut requests, request)
        {
            self.has_pending.store(true, Ordering::Release);
            self.window.request_redraw();
        }
    }
}

#[cfg(all(feature = "hosted", target_os = "windows"))]
fn native_adapter(
    window: &Arc<dyn winit::window::Window>,
    activation: CurrentTree,
    action: QueuedActions,
) -> accesskit_windows::SubclassingAdapter {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window
        .window_handle()
        .expect("AccessKit requires a live window handle");
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        panic!("hosted AccessKit on Windows requires an HWND");
    };
    accesskit_windows::SubclassingAdapter::new(
        accesskit_windows::HWND(handle.hwnd.get() as *mut _),
        activation,
        action,
    )
}

#[cfg(all(feature = "hosted", target_os = "macos"))]
fn native_adapter(
    window: &Arc<dyn winit::window::Window>,
    activation: CurrentTree,
    action: QueuedActions,
) -> accesskit_macos::SubclassingAdapter {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window
        .window_handle()
        .expect("AccessKit requires a live window handle");
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        panic!("hosted AccessKit on macOS requires an NSView");
    };
    unsafe { accesskit_macos::SubclassingAdapter::new(handle.ns_view.as_ptr(), activation, action) }
}

#[cfg(all(
    feature = "hosted",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
struct UnixDeactivation {
    activated: Arc<AtomicBool>,
}

#[cfg(all(
    feature = "hosted",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
impl DeactivationHandler for UnixDeactivation {
    fn deactivate_accessibility(&mut self) {
        self.activated.store(false, Ordering::Release);
    }
}

/// Wayland cannot report outer position; X11 can. `surface_position` is relative
/// to the window frame, so inner desktop origin is outer + surface origin.
#[cfg(all(
    feature = "hosted",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
fn unix_root_window_bounds(
    window: &dyn winit::window::Window,
    outer_position: winit::dpi::PhysicalPosition<i32>,
    inner_size: winit::dpi::PhysicalSize<u32>,
) -> (Rect, Rect) {
    let surface_position = window.surface_position();
    let inner_position = winit::dpi::PhysicalPosition::new(
        outer_position.x + surface_position.x,
        outer_position.y + surface_position.y,
    );
    let outer_origin: (f64, f64) = outer_position.cast::<f64>().into();
    let outer_size: (f64, f64) = window.outer_size().cast::<f64>().into();
    let inner_origin: (f64, f64) = inner_position.cast::<f64>().into();
    let inner_size: (f64, f64) = inner_size.cast::<f64>().into();
    (
        Rect::from_origin_size(outer_origin, outer_size),
        Rect::from_origin_size(inner_origin, inner_size),
    )
}

/// AT-SPI attaches over D-Bus; there is no HWND/NSView-style window handle.
#[cfg(all(
    feature = "hosted",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
fn native_adapter(
    _window: &Arc<dyn winit::window::Window>,
    activation: CurrentTree,
    action: QueuedActions,
) -> accesskit_unix::Adapter {
    let deactivation = UnixDeactivation {
        activated: Arc::clone(&activation.activated),
    };
    accesskit_unix::Adapter::new(activation, action, deactivation)
}

#[cfg(all(
    feature = "hosted",
    not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "android",
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn native_adapter(
    _window: &Arc<dyn winit::window::Window>,
    _activation: CurrentTree,
    _action: QueuedActions,
) -> Option<()> {
    None
}

#[cfg(all(feature = "hosted", any(target_os = "windows", target_os = "macos")))]
fn raise_accesskit_update<A>(adapter: &mut A, update: TreeUpdate)
where
    A: AccessKitUpdate,
{
    if let Some(events) = adapter.update_if_active(|| update) {
        events.raise();
    }
}

#[cfg(all(
    feature = "hosted",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
fn raise_accesskit_update(adapter: &mut accesskit_unix::Adapter, update: TreeUpdate) {
    adapter.update_if_active(|| update);
}

#[cfg(all(
    feature = "hosted",
    not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "android",
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn raise_accesskit_update(_adapter: &mut Option<()>, _update: TreeUpdate) {}

#[cfg(all(feature = "hosted", any(target_os = "windows", target_os = "macos")))]
trait AccessKitUpdate {
    type Events: RaiseEvents;
    fn update_if_active(
        &mut self,
        update_factory: impl FnOnce() -> TreeUpdate,
    ) -> Option<Self::Events>;
}

#[cfg(all(feature = "hosted", any(target_os = "windows", target_os = "macos")))]
trait RaiseEvents {
    fn raise(self);
}

#[cfg(all(feature = "hosted", target_os = "windows"))]
impl AccessKitUpdate for accesskit_windows::SubclassingAdapter {
    type Events = accesskit_windows::QueuedEvents;
    fn update_if_active(
        &mut self,
        update_factory: impl FnOnce() -> TreeUpdate,
    ) -> Option<Self::Events> {
        accesskit_windows::SubclassingAdapter::update_if_active(self, update_factory)
    }
}

#[cfg(all(feature = "hosted", target_os = "windows"))]
impl RaiseEvents for accesskit_windows::QueuedEvents {
    fn raise(self) {
        accesskit_windows::QueuedEvents::raise(self);
    }
}

#[cfg(all(feature = "hosted", target_os = "macos"))]
impl AccessKitUpdate for accesskit_macos::SubclassingAdapter {
    type Events = accesskit_macos::QueuedEvents;
    fn update_if_active(
        &mut self,
        update_factory: impl FnOnce() -> TreeUpdate,
    ) -> Option<Self::Events> {
        accesskit_macos::SubclassingAdapter::update_if_active(self, update_factory)
    }
}

#[cfg(all(feature = "hosted", target_os = "macos"))]
impl RaiseEvents for accesskit_macos::QueuedEvents {
    fn raise(self) {
        accesskit_macos::QueuedEvents::raise(self);
    }
}

/// Per-window native adapter. Only actions with a backend-neutral hosted path
/// are advertised and accepted.
#[cfg(all(feature = "hosted", not(target_os = "android")))]
pub(crate) struct HostedAccessibility {
    #[cfg(all(feature = "hosted", target_os = "windows"))]
    adapter: accesskit_windows::SubclassingAdapter,
    #[cfg(all(feature = "hosted", target_os = "macos"))]
    adapter: accesskit_macos::SubclassingAdapter,
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    adapter: accesskit_unix::Adapter,
    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    adapter: Option<()>,
    current_tree: CurrentTree,
    requests: Arc<Mutex<VecDeque<ActionRequest>>>,
    has_pending_actions: Arc<AtomicBool>,
}

#[cfg(all(feature = "hosted", not(target_os = "android")))]
impl HostedAccessibility {
    pub(crate) fn new(
        window: Arc<dyn winit::window::Window>,
        interactive: bool,
        scale_factor: f32,
    ) -> Self {
        // A native window must keep its own exposed root when focus moves into
        // a child. Generic Runtime roots are otherwise filtered by AccessKit,
        // and mounting/replacing documents must not replace the HWND provider.
        // A prepared document is not necessarily a presented document. Keep
        // activation limited to the native root until the first publication.
        let current_tree =
            CurrentTree::unpublished(interactive, scale_factor, Some(Arc::clone(&window)));
        let requests = Arc::new(Mutex::new(VecDeque::new()));
        let has_pending_actions = Arc::new(AtomicBool::new(false));
        let actions = QueuedActions {
            requests: Arc::clone(&requests),
            has_pending: Arc::clone(&has_pending_actions),
            window: Arc::clone(&window),
        };
        let adapter = native_adapter(&window, current_tree.clone(), actions);
        Self {
            adapter,
            current_tree,
            requests,
            has_pending_actions,
        }
    }

    pub(crate) fn is_activated(&self) -> bool {
        self.current_tree.activated.load(Ordering::Acquire)
    }

    pub(crate) fn sync_state(&self, scale_factor: f32) -> (bool, Option<u64>) {
        let projector = self
            .current_tree
            .projector
            .lock()
            .expect("accessibility projector");
        let scale_factor_changed =
            (projector.scale_factor - scale_factor.max(0.01)).abs() > f32::EPSILON;
        (scale_factor_changed, projector.generation)
    }

    pub(crate) fn process_event(
        &mut self,
        window: &dyn winit::window::Window,
        event: &winit::event::WindowEvent,
    ) {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        match event {
            winit::event::WindowEvent::Moved(outer_position) => {
                let (outer, inner) =
                    unix_root_window_bounds(window, *outer_position, window.surface_size());
                self.adapter.set_root_window_bounds(outer, inner);
            }
            winit::event::WindowEvent::SurfaceResized(inner_size) => {
                let outer_position = window.outer_position().unwrap_or_default();
                let (outer, inner) = unix_root_window_bounds(window, outer_position, *inner_size);
                self.adapter.set_root_window_bounds(outer, inner);
            }
            winit::event::WindowEvent::Focused(is_focused) => {
                self.adapter.update_window_focus_state(*is_focused);
            }
            _ => {}
        }
        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        {
            let _ = (window, event);
        }
    }

    pub(crate) fn synchronize(&mut self, update: AccessibilityUpdate, scale_factor: f32) {
        // Drop the projector lock before entering native UIA/AccessKit code:
        // raising events can synchronously request a fresh activation tree.
        if let Some(update) = self.current_tree.synchronize(update, scale_factor) {
            raise_accesskit_update(&mut self.adapter, update);
        }
    }

    pub(crate) fn take_actions(&self) -> Vec<nana_ui_runtime::AccessibilityActionRequest> {
        if !self.has_pending_actions.load(Ordering::Acquire) {
            return Vec::new();
        }
        let requests = {
            let Ok(mut requests) = self.requests.lock() else {
                return Vec::new();
            };
            self.has_pending_actions.store(false, Ordering::Release);
            std::mem::take(&mut *requests)
        };
        if requests.is_empty() {
            return Vec::new();
        }
        let projector = self
            .current_tree
            .projector
            .lock()
            .expect("accessibility projector");
        requests
            .into_iter()
            .filter_map(|request| projector.project_action_request(request))
            .collect()
    }
}

#[cfg(test)]
fn project_node(
    node: &AccessibilityNode,
    text_run_id: Option<NodeId>,
    interactive: bool,
    scale_factor: f32,
) -> Vec<(NodeId, Node)> {
    let mut out = Vec::with_capacity(if text_run_id.is_some() { 2 } else { 1 });
    project_node_into(node, text_run_id, interactive, scale_factor, &mut out);
    out
}

fn project_node_into(
    node: &AccessibilityNode,
    text_run_id: Option<NodeId>,
    interactive: bool,
    scale_factor: f32,
    out: &mut Vec<(NodeId, Node)>,
) {
    // An inert node keeps its place and what it says, and offers nothing.
    let interactive = interactive && !node.inert;
    let mut projected = Node::new(projected_role(node));
    // AccessKit Label nodes expose their text through value (including the
    // native UIA Name property), unlike controls whose accessible name is label.
    if node.role != AccessibilityRole::Text
        && let Some(label) = node.label.as_deref()
    {
        projected.set_label(label);
    }
    let value: Option<&str> = if node.role == AccessibilityRole::Text {
        node.label.as_deref().or(node.value.as_deref())
    } else {
        node.value.as_deref()
    };
    if let Some(value) = value {
        projected.set_value(value);
    }
    if let Some(description) = node.description.as_deref() {
        projected.set_description(description);
    }
    if let Some(role_description) = node.role_description.as_deref() {
        projected.set_role_description(role_description);
    }
    if !node.children.is_empty() || text_run_id.is_some() {
        let mut children =
            Vec::with_capacity(node.children.len() + usize::from(text_run_id.is_some()));
        children.extend(node.children.iter().copied().map(node_id));
        if let Some(text_run_id) = text_run_id {
            children.push(text_run_id);
        }
        projected.set_children(children);
    }
    // Runtime bounds are relative to the parent's children's space: the
    // offset, after the node's own turn when it has one, then the move by
    // its scroll, is this node's transform, so its children's offsets start
    // from where its content scrolled to, and its own box is its local
    // bounds moved back by the scroll. A box that moved with its parent, or
    // that its parent scrolled, keeps both and is not sent again. Physical
    // px scale every length: the turn's linear part stays, its move scales.
    let (x, y) = (
        f64::from(node.bounds.x * scale_factor),
        f64::from(node.bounds.y * scale_factor),
    );
    let (sx, sy) = (
        f64::from(node.scroll[0] * scale_factor),
        f64::from(node.scroll[1] * scale_factor),
    );
    let scrolled = sx != 0.0 || sy != 0.0;
    let scroll = Affine::translate((-sx, -sy));
    match node.transform {
        Some(turn) => {
            let placed = Affine::translate((x, y))
                * Affine::new([
                    f64::from(turn.a),
                    f64::from(turn.b),
                    f64::from(turn.c),
                    f64::from(turn.d),
                    f64::from(turn.e * scale_factor),
                    f64::from(turn.f * scale_factor),
                ]);
            projected.set_transform(if scrolled { placed * scroll } else { placed });
        }
        None if scrolled => {
            projected.set_transform(Affine::translate((x, y)) * scroll);
        }
        None if x != 0.0 || y != 0.0 => projected.set_transform(Affine::translate((x, y))),
        None => {}
    }
    projected.set_bounds(Rect {
        x0: sx,
        y0: sy,
        x1: sx + f64::from(node.bounds.width * scale_factor),
        y1: sy + f64::from(node.bounds.height * scale_factor),
    });
    if node.disabled {
        projected.set_disabled();
    }
    if node.modal {
        projected.set_modal();
    }
    if node.busy {
        projected.set_busy();
    }
    if node.invalid {
        projected.set_invalid(Invalid::True);
    }
    if let Some(live) = node.live {
        projected.set_live(match live {
            AccessibilityLive::Off => Live::Off,
            AccessibilityLive::Polite => Live::Polite,
            AccessibilityLive::Assertive => Live::Assertive,
        });
    }
    if let Some(value) = node.numeric_value {
        projected.set_numeric_value(value);
    }
    if let Some(minimum) = node.numeric_minimum {
        projected.set_min_numeric_value(minimum);
    }
    if let Some(maximum) = node.numeric_maximum {
        projected.set_max_numeric_value(maximum);
    }
    if let Some(step) = node.numeric_step {
        projected.set_numeric_value_step(step);
    }
    if let Some(scroll) = node.scroll_x {
        projected.set_scroll_x(scroll.value);
        projected.set_scroll_x_min(scroll.minimum);
        projected.set_scroll_x_max(scroll.maximum);
    }
    if let Some(scroll) = node.scroll_y {
        projected.set_scroll_y(scroll.value);
        projected.set_scroll_y_min(scroll.minimum);
        projected.set_scroll_y_max(scroll.maximum);
    }
    if interactive && !node.disabled {
        if node.scroll_y.is_some() {
            projected.add_action(Action::ScrollDown);
            projected.add_action(Action::ScrollUp);
        }
        if node.scroll_x.is_some() {
            projected.add_action(Action::ScrollLeft);
            projected.add_action(Action::ScrollRight);
        }
        if node.scroll_x.is_some() || node.scroll_y.is_some() {
            projected.add_action(Action::SetScrollOffset);
        }
    }
    if node.role == AccessibilityRole::TextInput && !node.editable {
        projected.set_read_only();
    }
    // A tree row and a list box option are always selectable, so their
    // unselected state is said too.
    match node.selected {
        Some(true) => projected.set_selected(true),
        Some(false)
            if matches!(
                node.role,
                AccessibilityRole::TreeItem | AccessibilityRole::ListBoxOption
            ) =>
        {
            projected.set_selected(false)
        }
        _ => {}
    }
    if let Some(descendant) = node.active_descendant {
        projected.set_active_descendant(node_id(descendant));
    }
    // AccessKit counts positions from zero, `aria-posinset` from one.
    if let Some(position) = node.position_in_set.filter(|position| *position > 0) {
        projected.set_position_in_set(position as usize - 1);
    }
    if let Some(size) = node.set_size {
        projected.set_size_of_set(size as usize);
    }
    if node.multiselectable {
        projected.set_multiselectable();
    }
    if let Some(level) = node.level.filter(|level| *level > 0) {
        // AccessKit counts levels from zero, `aria-level` from one.
        projected.set_level(level as usize - 1);
    }
    if let Some(expanded) = node.expanded {
        projected.set_expanded(expanded);
    }
    if node.has_popup {
        projected.set_has_popup(HasPopup::Menu);
    }
    if node.mixed {
        projected.set_toggled(Toggled::Mixed);
    } else if let Some(checked) = node.checked {
        projected.set_toggled(if checked {
            Toggled::True
        } else {
            Toggled::False
        });
    }
    if let Some(orientation) = node.orientation {
        projected.set_orientation(match orientation {
            SelectionOrientation::Horizontal => Orientation::Horizontal,
            SelectionOrientation::Vertical => Orientation::Vertical,
        });
    }
    // A node that only shows something offers neither: it is still the role
    // it says, a row of a list, just not one that acts.
    let actionable = interactive && !node.disabled && !node.display_only;
    if actionable && supports_click(node.role) {
        projected.add_action(Action::Click);
    }
    // A tree row's toggle only flips; the focus stays with the tree's rows.
    if actionable
        && supports_focus(node.role)
        && nana_ui_runtime::scene_contract::decode_virtual_tree_toggle(node.id).is_none()
    {
        projected.add_action(Action::Focus);
    }
    if interactive && !node.disabled && supports_set_value(node) {
        projected.add_action(Action::SetValue);
    }
    if interactive && !node.disabled && supports_numeric_step(node) {
        projected.add_action(Action::Increment);
        projected.add_action(Action::Decrement);
    }
    if interactive && !node.disabled && supports_expand(node) {
        projected.add_action(Action::Expand);
    }
    if interactive && !node.disabled && supports_collapse(node) {
        projected.add_action(Action::Collapse);
    }
    if let Some(selection) = text_run_id.and_then(|id| projected_text_selection(node, id)) {
        projected.set_text_selection(selection);
        if interactive && !node.disabled {
            projected.add_action(Action::SetTextSelection);
        }
    }

    out.push((node_id(node.id), projected));
    if let Some(text_run_id) = text_run_id {
        let value = node.value.as_deref().unwrap_or_default();
        let mut text_run = Node::new(Role::TextRun);
        text_run.set_value(value);
        text_run.set_text_direction(text_direction(node.writing));
        let lengths = if value.is_ascii() {
            vec![1u8; value.len()]
        } else {
            let mut lengths = Vec::with_capacity(value.len());
            for character in value.chars() {
                lengths.push(character.len_utf8() as u8);
            }
            lengths
        };
        text_run.set_character_lengths(lengths);
        out.push((text_run_id, text_run));
    }
}

/// Which way a text run reads, from its inline-start edge: a vertical column
/// reads top to bottom, or bottom to top under `direction: rtl` (#59).
const fn text_direction(writing: nana_ui_core::WritingContext) -> TextDirection {
    match writing.inline_start() {
        nana_ui_core::PhysicalEdge::Left => TextDirection::LeftToRight,
        nana_ui_core::PhysicalEdge::Right => TextDirection::RightToLeft,
        nana_ui_core::PhysicalEdge::Top => TextDirection::TopToBottom,
        nana_ui_core::PhysicalEdge::Bottom => TextDirection::BottomToTop,
    }
}

/// Commands stay menu items. Checkable rows are checkboxes (Toggle) and the
/// current choice in a group is a radio (SelectionItem). AccessKit does not
/// put SelectionItem on a plain menu item.
const fn is_menu_row(role: AccessibilityRole) -> bool {
    matches!(
        role,
        AccessibilityRole::MenuItem
            | AccessibilityRole::Checkbox
            | AccessibilityRole::Radio
            | AccessibilityRole::ListBoxOption
    )
}

/// The step a scroll request asks for: a line (UIA's small increment) or a
/// page, which is also what a request without one means.
fn scroll_unit(data: Option<&ActionData>) -> nana_ui_runtime::AccessibilityScrollUnit {
    match data {
        Some(ActionData::ScrollUnit(ScrollUnit::Item)) => {
            nana_ui_runtime::AccessibilityScrollUnit::Item
        }
        _ => nana_ui_runtime::AccessibilityScrollUnit::Page,
    }
}

const fn supports_click(role: AccessibilityRole) -> bool {
    matches!(
        role,
        AccessibilityRole::Button
            | AccessibilityRole::Checkbox
            | AccessibilityRole::Switch
            | AccessibilityRole::ComboBox
            | AccessibilityRole::TextInput
            | AccessibilityRole::Tab
            | AccessibilityRole::ListItem
            | AccessibilityRole::TreeItem
            | AccessibilityRole::MenuItem
            | AccessibilityRole::ListBoxOption
            | AccessibilityRole::Radio
    )
}

const fn supports_focus(role: AccessibilityRole) -> bool {
    matches!(
        role,
        AccessibilityRole::Button
            | AccessibilityRole::Checkbox
            | AccessibilityRole::Switch
            | AccessibilityRole::ListItem
            | AccessibilityRole::Tree
            | AccessibilityRole::TreeItem
            | AccessibilityRole::TextInput
            | AccessibilityRole::Slider
            | AccessibilityRole::ComboBox
            | AccessibilityRole::Tab
            | AccessibilityRole::MenuItem
            | AccessibilityRole::Radio
    )
}

const fn supports_set_value(node: &AccessibilityNode) -> bool {
    (matches!(node.role, AccessibilityRole::TextInput) && node.editable)
        || matches!(node.role, AccessibilityRole::Slider)
}

fn supports_numeric_step(node: &AccessibilityNode) -> bool {
    node.role == AccessibilityRole::Slider
        && node.numeric_step.is_some()
        && node.numeric_minimum.is_some()
        && node.numeric_maximum.is_some()
}

fn supports_expand(node: &AccessibilityNode) -> bool {
    match node.role {
        AccessibilityRole::ComboBox => !combo_box_open(node),
        AccessibilityRole::TreeItem => node.expanded == Some(false),
        _ => false,
    }
}

fn supports_collapse(node: &AccessibilityNode) -> bool {
    match node.role {
        AccessibilityRole::ComboBox => combo_box_open(node),
        AccessibilityRole::TreeItem => node.expanded == Some(true),
        _ => false,
    }
}

/// A combobox says it is open through `expanded`; one that only sets
/// `selected` (the state it paints with) is read the same way.
fn combo_box_open(node: &AccessibilityNode) -> bool {
    node.expanded.unwrap_or(node.selected == Some(true))
}

fn projected_text_selection(
    node: &AccessibilityNode,
    text_run_id: NodeId,
) -> Option<AccessKitTextSelection> {
    let selection = node.selection?;
    let value = node.value.as_deref()?;
    Some(AccessKitTextSelection {
        anchor: TextPosition {
            node: text_run_id,
            character_index: byte_to_character_index(value, selection.anchor)?,
        },
        focus: TextPosition {
            node: text_run_id,
            character_index: byte_to_character_index(value, selection.focus)?,
        },
    })
}

fn byte_to_character_index(value: &str, byte_offset: usize) -> Option<usize> {
    if byte_offset > value.len() || !value.is_char_boundary(byte_offset) {
        return None;
    }
    let prefix = &value[..byte_offset];
    if prefix.is_ascii() {
        return Some(byte_offset);
    }
    Some(prefix.chars().count())
}

fn character_index_to_byte(value: &str, character_index: usize) -> Option<usize> {
    if value.is_ascii() {
        return (character_index <= value.len()).then_some(character_index);
    }
    let mut count = 0usize;
    for (byte_index, _) in value.char_indices() {
        if count == character_index {
            return Some(byte_index);
        }
        count += 1;
    }
    (character_index == count).then_some(value.len())
}

const fn node_id(id: StableNodeId) -> NodeId {
    NodeId(id.get())
}

/// AccessKit's platform filter drops `GenericContainer` before it asks about
/// ScrollPattern. A generic box that can actually move has to stay in the
/// tree as `ScrollView`. A role that already names a control keeps that role,
/// and a generic box with no positive range stays generic.
fn projected_role(node: &AccessibilityNode) -> Role {
    let role = project_role(node.role, node.multiline);
    if role == Role::GenericContainer && (node.scroll_x.is_some() || node.scroll_y.is_some()) {
        Role::ScrollView
    } else {
        role
    }
}

const fn project_role(role: AccessibilityRole, multiline: bool) -> Role {
    match role {
        AccessibilityRole::Document => Role::Document,
        AccessibilityRole::Text => Role::Label,
        AccessibilityRole::Button => Role::Button,
        AccessibilityRole::Checkbox => Role::CheckBox,
        AccessibilityRole::Switch => Role::Switch,
        AccessibilityRole::TextInput if multiline => Role::MultilineTextInput,
        AccessibilityRole::TextInput => Role::TextInput,
        AccessibilityRole::Slider => Role::Slider,
        AccessibilityRole::ComboBox => Role::ComboBox,
        AccessibilityRole::ProgressIndicator => Role::ProgressIndicator,
        AccessibilityRole::List => Role::List,
        AccessibilityRole::ListItem => Role::ListItem,
        AccessibilityRole::ListBox => Role::ListBox,
        AccessibilityRole::ListBoxOption => Role::ListBoxOption,
        AccessibilityRole::Tree => Role::Tree,
        AccessibilityRole::TreeItem => Role::TreeItem,
        AccessibilityRole::Table => Role::Table,
        AccessibilityRole::Row => Role::Row,
        AccessibilityRole::Cell => Role::Cell,
        AccessibilityRole::ColumnHeader => Role::ColumnHeader,
        AccessibilityRole::TabList => Role::TabList,
        AccessibilityRole::Tab => Role::Tab,
        AccessibilityRole::RadioGroup => Role::RadioGroup,
        AccessibilityRole::Radio => Role::RadioButton,
        AccessibilityRole::Separator => Role::Splitter,
        AccessibilityRole::Dialog => Role::Dialog,
        AccessibilityRole::AlertDialog => Role::AlertDialog,
        AccessibilityRole::Menu => Role::Menu,
        AccessibilityRole::MenuItem => Role::MenuItem,
        AccessibilityRole::Tooltip => Role::Tooltip,
        AccessibilityRole::Status => Role::Status,
        AccessibilityRole::Toolbar => Role::Toolbar,
        AccessibilityRole::Image => Role::Image,
        AccessibilityRole::Main => Role::Main,
        AccessibilityRole::Navigation => Role::Navigation,
        AccessibilityRole::Banner => Role::Banner,
        AccessibilityRole::ContentInfo => Role::ContentInfo,
        AccessibilityRole::Complementary => Role::Complementary,
        AccessibilityRole::Region => Role::Region,
        AccessibilityRole::Search => Role::Search,
        AccessibilityRole::Form => Role::Form,
        AccessibilityRole::Generic => Role::GenericContainer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_runtime::LayoutBox;

    #[test]
    fn static_text_exports_its_content_as_accesskit_value_after_updates() {
        let mut text = node(1, None, &[]);
        text.role = AccessibilityRole::Text;
        text.label = Some("Initial text".into());
        let (mut projector, initial) = AccessibilityProjector::new(vec![text.clone()], true, 1.0);
        assert_eq!(initial.nodes[0].1.role(), Role::Label);
        assert_eq!(initial.nodes[0].1.value(), Some("Initial text"));
        text.label = Some("Updated text".into());
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 1,
                updated: vec![text.clone()],
                removed: vec![],
            })
            .unwrap();
        assert_eq!(update.nodes.len(), 1);
        assert_eq!(update.nodes[0].1.value(), Some("Updated text"));
        text.label = None;
        text.value = Some("Value-only text".into());
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 2,
                updated: vec![text],
                removed: vec![],
            })
            .unwrap();
        assert_eq!(update.nodes[0].1.value(), Some("Value-only text"));
    }

    #[test]
    fn switch_role_description_reaches_the_accesskit_node() {
        let mut control = node(2, Some(1), &[]);
        control.role = AccessibilityRole::Switch;
        control.label = Some("辉光".into());
        control.role_description = Some("开关".into());
        control.checked = Some(false);
        let (_projector, initial) =
            AccessibilityProjector::new(vec![node(1, None, &[2]), control], true, 1.0);
        let switch = initial
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .map(|(_, node)| node)
            .expect("switch node");
        assert_eq!(switch.role(), Role::Switch);
        assert_eq!(switch.role_description(), Some("开关"));
        assert_eq!(switch.label(), Some("辉光"));
    }

    #[cfg(all(feature = "hosted", not(target_os = "android")))]
    #[test]
    fn unpublished_window_exposes_only_its_root_until_the_first_publication() {
        let mut current = CurrentTree::unpublished(true, 1.0, None);
        assert!(!current.activated.load(Ordering::Acquire));
        let initial = current.request_initial_tree().unwrap();
        assert!(current.activated.load(Ordering::Acquire));
        assert_eq!(initial.nodes.len(), 1);
        assert_eq!(initial.nodes[0].0, FOREST_ROOT_ID);
        assert_eq!(initial.nodes[0].1.role(), Role::Window);
        assert!(initial.nodes[0].1.children().is_empty());
        assert_eq!(initial.focus, FOREST_ROOT_ID);
        assert_eq!(current.projector.lock().unwrap().generation, None);

        let mut control = node(2, Some(1), &[]);
        control.role = AccessibilityRole::Button;
        control.label = Some("Published button".into());
        current
            .synchronize(
                AccessibilityUpdate::Full {
                    generation: Some(7),
                    nodes: vec![node(1, None, &[2]), control],
                },
                1.0,
            )
            .unwrap();
        let published = current.request_initial_tree().unwrap();
        assert_eq!(published.nodes.len(), 3);
        assert_eq!(published.tree.unwrap().root, FOREST_ROOT_ID);
        assert!(
            published
                .nodes
                .iter()
                .any(|(id, value)| *id == NodeId(2) && value.label() == Some("Published button"))
        );
        assert_eq!(current.projector.lock().unwrap().generation, Some(7));
    }

    fn node(id: u64, parent: Option<u64>, children: &[u64]) -> AccessibilityNode {
        AccessibilityNode {
            id: StableNodeId::new(id).unwrap(),
            parent: parent.and_then(StableNodeId::new),
            children: children
                .iter()
                .copied()
                .filter_map(StableNodeId::new)
                .collect(),
            role: AccessibilityRole::Generic,
            label: None,
            description: None,
            role_description: None,
            value: None,
            disabled: false,
            display_only: false,
            inert: false,
            checked: None,
            mixed: false,
            orientation: None,
            selected: None,
            level: None,
            expanded: None,
            active_descendant: None,
            position_in_set: None,
            set_size: None,
            multiselectable: false,
            has_popup: false,
            multiline: false,
            editable: false,
            selection: None,
            modal: false,
            busy: false,
            invalid: false,
            live: None,
            numeric_minimum: None,
            numeric_maximum: None,
            numeric_step: None,
            numeric_value: None,
            scroll_x: None,
            scroll_y: None,
            focused: false,
            bounds: LayoutBox::default(),
            transform: None,
            scroll: [0.0, 0.0],
            clip: None,
            writing: Default::default(),
        }
    }

    #[test]
    fn semantic_focus_updates_keep_the_index_in_sync_without_tree_reconciliation() {
        let mut first = node(2, Some(1), &[]);
        first.focused = true;
        let mut second = node(3, Some(1), &[]);
        let mut projector = AccessibilityProjector::retain(
            vec![node(1, None, &[2, 3]), first.clone(), second.clone()],
            true,
            1.0,
            Some(0),
            true,
        );
        for generation in 1..=8 {
            first.focused = generation % 2 == 0;
            second.focused = !first.focused;
            let update = projector
                .apply_delta(AccessibilityDelta {
                    generation,
                    updated: vec![first.clone(), second.clone()],
                    removed: vec![],
                })
                .unwrap();
            assert_eq!(update.focus, NodeId(if first.focused { 2 } else { 3 }));
            assert!(update.tree.is_none());
            assert_eq!(update.nodes.len(), 2);
        }
        assert_eq!(projector.global_reconciliations.get(), 1);
        let removed = projector
            .apply_delta(AccessibilityDelta {
                generation: 9,
                updated: vec![],
                removed: vec![StableNodeId::new(1).unwrap()],
            })
            .unwrap();
        assert_eq!(removed.focus, FOREST_ROOT_ID);
        assert_eq!(removed.nodes.len(), 1);
        assert!(projector.focused.is_empty());
    }

    #[test]
    fn virtual_menu_row_wins_platform_focus_over_its_trigger() {
        let menu_id = 5u64;
        let row = nana_ui_runtime::scene_contract::virtual_menu_item_id(
            StableNodeId::new(menu_id).unwrap(),
            1,
        )
        .unwrap();
        let mut trigger = node(4, Some(1), &[]);
        trigger.role = AccessibilityRole::ListItem;
        trigger.label = Some("窗口".into());
        trigger.focused = true;
        let mut menu = node(menu_id, Some(1), &[row.get()]);
        menu.role = AccessibilityRole::Menu;
        let mut item = node(row.get(), Some(menu_id), &[]);
        item.role = AccessibilityRole::MenuItem;
        item.label = Some("取消窗口置顶".into());
        item.focused = true;
        item.selected = Some(true);
        let mut projector = AccessibilityProjector::retain(
            vec![
                node(1, None, &[4, menu_id]),
                trigger.clone(),
                menu,
                item.clone(),
            ],
            true,
            1.0,
            Some(0),
            true,
        );
        assert_eq!(projector.full_update().focus, NodeId(row.get()));

        item.focused = false;
        item.selected = Some(false);
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 1,
                updated: vec![item],
                removed: vec![],
            })
            .unwrap();
        assert_eq!(update.focus, NodeId(4));
        assert!(trigger.focused);
    }

    #[test]
    fn native_window_root_survives_focus_and_document_replacement() {
        let (mut projector, _) = AccessibilityProjector::new(Vec::new(), true, 1.0);
        projector.window_root = true;
        let initial = projector.full_update();
        assert_eq!(initial.tree.unwrap().root, FOREST_ROOT_ID);
        assert_eq!(initial.focus, FOREST_ROOT_ID);
        assert_eq!(initial.nodes[0].1.role(), Role::Window);

        let root = node(1, None, &[2]);
        let mut editor = node(2, Some(1), &[]);
        editor.role = AccessibilityRole::TextInput;
        editor.editable = true;
        projector.synchronize(vec![root.clone(), editor.clone()], 1.0);
        editor.focused = true;
        editor.value = Some("Edited".into());
        let focused = projector.synchronize(vec![root, editor], 1.0).unwrap();
        assert!(focused.tree.is_none());
        assert_eq!(focused.focus, NodeId(2));
        let full = projector.full_update();
        assert_eq!(full.tree.unwrap().root, FOREST_ROOT_ID);
        let window = &full
            .nodes
            .iter()
            .find(|(id, _)| *id == FOREST_ROOT_ID)
            .unwrap()
            .1;
        assert_eq!(window.role(), Role::Window);
        assert_eq!(window.children(), &[NodeId(1)]);

        for nodes in [vec![node(3, None, &[]), node(4, None, &[])], vec![]] {
            let updated = projector.synchronize(nodes, 1.0).unwrap();
            assert_eq!(updated.tree.unwrap().root, FOREST_ROOT_ID);
            assert_eq!(updated.focus, FOREST_ROOT_ID);
            assert_eq!(updated.nodes.last().unwrap().1.role(), Role::Window);
        }
        assert_eq!(projector.full_update().nodes.len(), 1);
    }

    #[test]
    fn landmark_roles_project_to_accesskit_landmarks() {
        let cases = [
            (AccessibilityRole::Main, Role::Main),
            (AccessibilityRole::Navigation, Role::Navigation),
            (AccessibilityRole::Banner, Role::Banner),
            (AccessibilityRole::ContentInfo, Role::ContentInfo),
            (AccessibilityRole::Complementary, Role::Complementary),
            (AccessibilityRole::Region, Role::Region),
            (AccessibilityRole::Search, Role::Search),
            (AccessibilityRole::Form, Role::Form),
        ];
        for (role, expected) in cases {
            assert_eq!(project_role(role, false), expected);
            assert!(!supports_click(role));
            assert!(!supports_focus(role));
        }
    }

    #[test]
    fn full_projection_uses_stable_ids_and_runtime_focus() {
        let root = node(1, None, &[2]);
        let mut input = node(2, Some(1), &[]);
        input.role = AccessibilityRole::TextInput;
        input.multiline = true;
        input.focused = true;
        input.label = Some("Message".into());
        input.description = Some("Required details".into());
        input.value = Some("hello".into());
        input.bounds = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 40.0,
        };

        let (_, update) = AccessibilityProjector::new(vec![root, input], true, 1.0);

        assert_eq!(update.tree.unwrap().root, NodeId(1));
        assert_eq!(update.focus, NodeId(2));
        let (_, input) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .unwrap();
        assert_eq!(input.role(), Role::MultilineTextInput);
        assert_eq!(input.label(), Some("Message"));
        assert_eq!(input.description(), Some("Required details"));
        assert_eq!(input.value(), Some("hello"));
        // The root has no offset, so the input's is its transform.
        assert_eq!(placed(input).x0, 10.0);
        assert_eq!(placed(input).x1, 110.0);
        assert_eq!(input.bounds().unwrap().x1, 100.0);
    }

    /// Where `node`'s bounds stand in its parent's coordinates: its
    /// transform applied to them.
    fn placed(node: &Node) -> accesskit::Rect {
        let bounds = node.bounds().unwrap();
        node.transform()
            .map_or(bounds, |transform| transform.transform_rect_bbox(bounds))
    }

    #[test]
    fn a_child_offset_is_its_transform_from_the_parent_origin() {
        let mut root = node(1, None, &[2]);
        root.bounds = LayoutBox {
            x: 5.0,
            y: 7.0,
            width: 200.0,
            height: 100.0,
        };
        let mut child = node(2, Some(1), &[]);
        child.bounds = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        };
        let (_, update) = AccessibilityProjector::new(vec![root, child], false, 2.0);
        let projected = |id| &update.nodes.iter().find(|(node, _)| *node == id).unwrap().1;
        let root = projected(NodeId(1));
        let child = projected(NodeId(2));
        assert_eq!(
            root.transform().unwrap().as_coeffs(),
            [1.0, 0.0, 0.0, 1.0, 10.0, 14.0]
        );
        assert_eq!(
            root.bounds().unwrap(),
            accesskit::Rect::new(0.0, 0.0, 400.0, 200.0)
        );
        assert_eq!(
            child.transform().unwrap().as_coeffs(),
            [1.0, 0.0, 0.0, 1.0, 20.0, 40.0]
        );
        assert_eq!(placed(child), accesskit::Rect::new(20.0, 40.0, 80.0, 120.0));
    }

    /// A scrolled node's transform is its offset after the move by its
    /// scroll, so its children's offsets start from where its content
    /// scrolled to; its own box is its bounds moved back by the scroll, and
    /// stands where it did. A scroll changes the scroller's node alone.
    #[test]
    fn a_scrolled_node_moves_its_children_by_its_transform() {
        let mut root = node(1, None, &[2]);
        root.bounds = LayoutBox {
            x: 5.0,
            y: 7.0,
            width: 200.0,
            height: 100.0,
        };
        root.scroll = [3.0, 40.0];
        let mut child = node(2, Some(1), &[]);
        child.bounds = LayoutBox {
            x: 10.0,
            y: 60.0,
            width: 30.0,
            height: 40.0,
        };
        let (_, update) = AccessibilityProjector::new(vec![root, child], false, 2.0);
        let projected = |id| &update.nodes.iter().find(|(node, _)| *node == id).unwrap().1;
        let root = projected(NodeId(1));
        let child = projected(NodeId(2));
        assert_eq!(
            root.transform().unwrap().as_coeffs(),
            [1.0, 0.0, 0.0, 1.0, 4.0, -66.0]
        );
        assert_eq!(
            root.bounds().unwrap(),
            accesskit::Rect::new(6.0, 80.0, 406.0, 280.0)
        );
        // Its own box stands at its offset, as an unscrolled one does.
        assert_eq!(placed(root), accesskit::Rect::new(10.0, 14.0, 410.0, 214.0));
        // The child, 60 px down its content scrolled by 40, stands 20 px
        // below the root's top: through both transforms.
        let through = *root.transform().unwrap() * *child.transform().unwrap();
        assert_eq!(
            through.transform_rect_bbox(child.bounds().unwrap()),
            accesskit::Rect::new(24.0, 54.0, 84.0, 134.0)
        );
    }

    /// A turned node's transform is its offset after its own turn, in
    /// physical px: the turn's linear part as it is, its move scaled.
    #[test]
    fn a_turned_node_hands_its_turn_after_its_offset() {
        let root = node(1, None, &[2]);
        let mut child = node(2, Some(1), &[]);
        child.bounds = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        };
        child.transform = Some(nana_ui_core::PaintTransform {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            e: 40.0,
            f: 0.0,
        });
        let (_, update) = AccessibilityProjector::new(vec![root, child], false, 2.0);
        let child = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .unwrap()
            .1;
        assert_eq!(
            child.transform().unwrap().as_coeffs(),
            [0.0, 1.0, -1.0, 0.0, 100.0, 40.0]
        );
        assert_eq!(
            child.bounds().unwrap(),
            accesskit::Rect::new(0.0, 0.0, 60.0, 80.0)
        );
        // A quarter turn about the box's origin, moved back by its height:
        // the turned box stands at the offset, 80 wide and 60 tall.
        assert_eq!(
            placed(child),
            accesskit::Rect::new(20.0, 40.0, 100.0, 100.0)
        );
    }

    #[test]
    fn focus_delta_moves_accesskit_focus_from_generic_root_to_text_input() {
        let root = node(1, None, &[2]);
        let mut input = node(2, Some(1), &[]);
        input.role = AccessibilityRole::TextInput;
        let (mut projector, initial) =
            AccessibilityProjector::new(vec![root, input.clone()], true, 1.0);
        assert_eq!(initial.focus, NodeId(1));

        input.focused = true;
        let update = projector.apply(AccessibilityDelta {
            generation: 2,
            updated: vec![input],
            removed: vec![],
        });
        assert!(update.tree.is_none());
        assert_eq!(update.focus, NodeId(2));
    }

    #[test]
    fn new_child_delta_reattaches_through_the_cached_parent() {
        let root = node(1, None, &[2]);
        let child = node(2, Some(1), &[]);
        let (mut projector, _) = AccessibilityProjector::new(vec![root, child], false, 1.0);

        let update = projector.apply(AccessibilityDelta {
            generation: 2,
            updated: vec![node(3, Some(1), &[])],
            removed: vec![],
        });

        assert!(update.tree.is_none());
        let (_, parent) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .expect("parent must ship with a newly attached child");
        assert!(parent.children().contains(&NodeId(3)));
        assert!(update.nodes.iter().any(|(id, _)| *id == NodeId(3)));
    }

    #[test]
    fn tombstone_reprojects_parent_without_retaining_ghost_child() {
        let root = node(1, None, &[2]);
        let child = node(2, Some(1), &[]);
        let (mut projector, _) = AccessibilityProjector::new(vec![root, child], false, 1.0);

        let update = projector.apply(AccessibilityDelta {
            generation: 2,
            updated: vec![],
            removed: vec![StableNodeId::new(2).unwrap()],
        });

        assert!(update.tree.is_none());
        assert_eq!(update.focus, NodeId(1));
        let (_, root) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .unwrap();
        assert!(root.children().is_empty());
        assert!(update.nodes.iter().all(|(id, _)| *id != NodeId(2)));
    }

    #[test]
    fn tombstone_parent_drops_cached_descendants_from_incremental_update() {
        let root = node(1, None, &[2]);
        let parent = node(2, Some(1), &[3]);
        let grandchild = node(3, Some(2), &[]);
        let (mut projector, _) =
            AccessibilityProjector::new(vec![root, parent, grandchild.clone()], false, 1.0);

        let update = projector.apply(AccessibilityDelta {
            generation: 2,
            updated: vec![grandchild],
            removed: vec![StableNodeId::new(2).unwrap()],
        });

        assert!(update.tree.is_none());
        assert!(update.nodes.iter().all(|(id, _)| *id != NodeId(2)));
        assert!(update.nodes.iter().all(|(id, _)| *id != NodeId(3)));
        let (_, root_node) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .unwrap();
        assert!(root_node.children().is_empty());
        assert!(!projector.nodes.contains_key(&StableNodeId::new(3).unwrap()));
        assert!(
            projector
                .synchronize(vec![node(1, None, &[])], 1.0)
                .is_none()
        );
    }

    #[test]
    fn tombstone_parent_can_reparent_a_child_onto_a_surviving_uncle() {
        let root = node(1, None, &[2, 3]);
        let parent = node(2, Some(1), &[4]);
        let uncle = node(3, Some(1), &[]);
        let child = node(4, Some(2), &[]);
        let (mut projector, _) = AccessibilityProjector::new(
            vec![root, parent, uncle.clone(), child.clone()],
            false,
            1.0,
        );

        let mut moved = child;
        moved.parent = Some(StableNodeId::new(3).unwrap());
        let mut next_uncle = uncle;
        next_uncle.children = vec![StableNodeId::new(4).unwrap()];
        let update = projector.apply(AccessibilityDelta {
            generation: 2,
            updated: vec![moved, next_uncle],
            removed: vec![StableNodeId::new(2).unwrap()],
        });

        assert!(update.tree.is_none());
        assert!(update.nodes.iter().any(|(id, _)| *id == NodeId(4)));
        assert!(update.nodes.iter().all(|(id, _)| *id != NodeId(2)));
        let (_, uncle_node) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(3))
            .unwrap();
        assert!(uncle_node.children().contains(&NodeId(4)));
        let (_, root_node) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .unwrap();
        assert_eq!(root_node.children(), [NodeId(3)].as_slice());
    }

    #[test]
    fn synchronize_emits_only_changed_nodes() {
        let root = node(1, None, &[2]);
        let child = node(2, Some(1), &[]);
        let (mut projector, _) =
            AccessibilityProjector::new(vec![root.clone(), child.clone()], false, 1.0);
        assert!(
            projector
                .synchronize(vec![root.clone(), child.clone()], 1.0)
                .is_none()
        );

        let mut changed = child;
        changed.label = Some("changed".into());
        let update = projector.synchronize(vec![root, changed], 1.0).unwrap();
        assert_eq!(update.nodes.len(), 1);
        assert_eq!(update.nodes[0].0, NodeId(2));
    }

    #[test]
    fn stale_delta_cannot_roll_back_the_platform_tree() {
        let root = node(1, None, &[2]);
        let child = node(2, Some(1), &[]);
        let (mut projector, _) =
            AccessibilityProjector::new(vec![root.clone(), child.clone()], false, 1.0);

        let mut current = child.clone();
        current.label = Some("current".into());
        current.bounds.x = 10.0;
        current.bounds.width = 5.0;
        assert!(
            projector
                .apply_delta(AccessibilityDelta {
                    generation: 2,
                    updated: vec![current],
                    removed: vec![],
                })
                .is_some()
        );

        let mut stale = child;
        stale.label = Some("stale".into());
        stale.bounds.x = 100.0;
        assert!(
            projector
                .apply_delta(AccessibilityDelta {
                    generation: 1,
                    updated: vec![stale.clone()],
                    removed: vec![],
                })
                .is_none()
        );
        assert_eq!(
            projector.nodes[&StableNodeId::new(2).unwrap()]
                .label
                .as_deref(),
            Some("current")
        );

        let update = projector
            .synchronize_full(vec![root, stale], 2.0, Some(1))
            .expect("DPI change must reproject retained current semantics");
        let projected = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .unwrap()
            .1;
        assert_eq!(projected.label(), Some("current"));
        assert_eq!(placed(projected).x0, 20.0);
        assert_eq!(placed(projected).x1, 30.0);
    }

    #[test]
    fn empty_snapshot_uses_platform_forest_until_a_new_root_arrives() {
        let (mut projector, _) = AccessibilityProjector::new(vec![node(1, None, &[])], false, 1.0);
        let empty = projector
            .synchronize_full(Vec::new(), 1.0, Some(2))
            .unwrap();
        assert_eq!(empty.tree.unwrap().root, FOREST_ROOT_ID);
        assert_eq!(empty.focus, FOREST_ROOT_ID);
        assert_eq!(projector.generation, Some(2));

        let update = projector
            .synchronize_full(vec![node(9, None, &[])], 1.0, Some(3))
            .unwrap();
        assert_eq!(update.tree.unwrap().root, NodeId(9));
        assert_eq!(update.focus, NodeId(9));
        assert_eq!(projector.generation, Some(3));
    }

    #[test]
    fn explicitly_enabled_empty_projector_accepts_the_first_async_root() {
        let (mut projector, initial) = AccessibilityProjector::new(Vec::new(), false, 1.0);
        assert_eq!(initial.tree.unwrap().root, FOREST_ROOT_ID);
        assert_eq!(initial.focus, FOREST_ROOT_ID);

        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 1,
                updated: vec![node(7, None, &[])],
                removed: vec![],
            })
            .unwrap();
        assert_eq!(update.tree.unwrap().root, NodeId(7));
        assert_eq!(update.focus, NodeId(7));
    }

    #[test]
    fn multiple_runtime_roots_use_a_collision_free_platform_forest_root() {
        let first = node(1, None, &[]);
        let second = node(9, None, &[]);
        let (mut projector, _) = AccessibilityProjector::new(vec![first], false, 1.0);
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 1,
                updated: vec![second],
                removed: vec![],
            })
            .unwrap();
        assert_eq!(update.tree.unwrap().root, FOREST_ROOT_ID);
        let forest = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == FOREST_ROOT_ID)
            .unwrap()
            .1;
        assert_eq!(forest.children(), [NodeId(1), NodeId(9)]);

        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 2,
                updated: vec![],
                removed: vec![StableNodeId::new(9).unwrap()],
            })
            .unwrap();
        assert_eq!(update.tree.unwrap().root, NodeId(1));
        assert!(update.nodes.iter().all(|(id, _)| *id != FOREST_ROOT_ID));

        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 3,
                updated: vec![],
                removed: vec![StableNodeId::new(1).unwrap()],
            })
            .unwrap();
        assert_eq!(update.tree.unwrap().root, FOREST_ROOT_ID);
        assert_eq!(update.focus, FOREST_ROOT_ID);
        assert!(
            update
                .nodes
                .iter()
                .find(|(id, _)| *id == FOREST_ROOT_ID)
                .unwrap()
                .1
                .children()
                .is_empty()
        );
    }

    #[test]
    fn a_display_only_row_offers_no_action_and_is_not_disabled() {
        let root = node(1, None, &[2]);
        let mut row = node(2, Some(1), &[]);
        row.role = AccessibilityRole::ListItem;
        row.display_only = true;
        let (projector, update) = AccessibilityProjector::new(vec![root, row], true, 1.0);
        let projected = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .unwrap()
            .1;
        assert_eq!(projected.role(), Role::ListItem);
        assert!(!projected.is_disabled());
        assert!(!projected.supports_action(Action::Click));
        assert!(!projected.supports_action(Action::Focus));
        for action in [Action::Click, Action::Focus] {
            assert!(
                projector
                    .project_action_request(ActionRequest {
                        action,
                        target_tree: TreeId::ROOT,
                        target_node: NodeId(2),
                        data: None,
                    })
                    .is_none()
            );
        }
    }

    #[test]
    fn an_inert_node_keeps_its_role_and_offers_no_action() {
        let root = node(1, None, &[2, 3]);
        let mut slider = node(2, Some(1), &[]);
        slider.role = AccessibilityRole::Slider;
        slider.numeric_minimum = Some(0.0);
        slider.numeric_maximum = Some(1.0);
        slider.numeric_value = Some(0.5);
        slider.inert = true;
        let mut button = node(3, Some(1), &[]);
        button.role = AccessibilityRole::Button;
        button.inert = true;
        let (projector, update) =
            AccessibilityProjector::new(vec![root, slider, button], true, 1.0);
        let projected = |id| {
            &update
                .nodes
                .iter()
                .find(|(node, _)| *node == NodeId(id))
                .unwrap()
                .1
        };
        assert_eq!(projected(2).role(), Role::Slider);
        assert_eq!(projected(3).role(), Role::Button);
        assert!(!projected(3).is_disabled(), "inert is not disabled");
        for (id, action) in [
            (2, Action::Increment),
            (2, Action::Decrement),
            (2, Action::SetValue),
            (3, Action::Click),
            (3, Action::Focus),
        ] {
            assert!(!projected(id).supports_action(action), "{id} {action:?}");
            assert!(
                projector
                    .project_action_request(ActionRequest {
                        action,
                        target_tree: TreeId::ROOT,
                        target_node: NodeId(id),
                        data: None,
                    })
                    .is_none(),
                "{id} {action:?}"
            );
        }
    }

    #[test]
    fn actions_are_only_advertised_for_enabled_interactive_nodes() {
        let root = node(1, None, &[2, 3]);
        let mut enabled = node(2, Some(1), &[]);
        enabled.role = AccessibilityRole::Button;
        let mut disabled = node(3, Some(1), &[]);
        disabled.role = AccessibilityRole::Button;
        disabled.disabled = true;

        let (_, update) = AccessibilityProjector::new(vec![root, enabled, disabled], true, 1.0);
        let enabled = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .unwrap()
            .1;
        let disabled = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(3))
            .unwrap()
            .1;
        assert!(enabled.supports_action(Action::Click));
        assert!(enabled.supports_action(Action::Focus));
        assert!(!enabled.supports_action(Action::SetValue));
        assert!(!disabled.supports_action(Action::Click));
        assert!(!disabled.supports_action(Action::Focus));
        assert!(!disabled.supports_action(Action::SetValue));

        let mut input = node(4, Some(1), &[]);
        input.role = AccessibilityRole::TextInput;
        input.editable = true;
        let (_, input) = project_node(&input, Some(NodeId(40)), true, 1.0)
            .into_iter()
            .next()
            .unwrap();
        assert!(input.supports_action(Action::Click));
        assert!(input.supports_action(Action::Focus));
        assert!(input.supports_action(Action::SetValue));

        let mut read_only = node(5, Some(1), &[]);
        read_only.role = AccessibilityRole::TextInput;
        let (_, read_only) = project_node(&read_only, Some(NodeId(50)), true, 1.0)
            .into_iter()
            .next()
            .unwrap();
        assert!(read_only.is_read_only());
        assert!(!read_only.supports_action(Action::SetValue));

        let mut malformed = node(6, Some(1), &[]);
        malformed.editable = true;
        let (_, malformed) = project_node(&malformed, None, true, 1.0)
            .into_iter()
            .next()
            .unwrap();
        assert!(!malformed.supports_action(Action::SetValue));

        let mut range = node(7, Some(1), &[]);
        range.role = AccessibilityRole::Slider;
        range.busy = true;
        range.invalid = true;
        range.numeric_minimum = Some(-1.0);
        range.numeric_maximum = Some(1.0);
        range.numeric_step = Some(0.25);
        range.numeric_value = Some(0.5);
        let (_, range) = project_node(&range, None, true, 1.0)
            .into_iter()
            .next()
            .unwrap();
        assert!(range.supports_action(Action::SetValue));
        assert!(range.is_busy());
        assert_eq!(range.invalid(), Some(Invalid::True));
        assert_eq!(range.min_numeric_value(), Some(-1.0));
        assert_eq!(range.max_numeric_value(), Some(1.0));
        assert_eq!(range.numeric_value_step(), Some(0.25));
        assert_eq!(range.numeric_value(), Some(0.5));

        let mut group = node(8, Some(1), &[9]);
        group.role = AccessibilityRole::RadioGroup;
        group.orientation = Some(SelectionOrientation::Horizontal);
        let (_, group) = project_node(&group, None, true, 1.0)
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(group.role(), Role::RadioGroup);
        assert_eq!(group.orientation(), Some(Orientation::Horizontal));
        assert!(!group.supports_action(Action::Click));

        let mut radio = node(9, Some(8), &[]);
        radio.role = AccessibilityRole::Radio;
        radio.checked = Some(true);
        let (_, radio) = project_node(&radio, None, true, 1.0)
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(radio.role(), Role::RadioButton);
        assert_eq!(radio.toggled(), Some(Toggled::True));
        assert!(radio.supports_action(Action::Click));
        assert!(radio.supports_action(Action::Focus));
    }

    #[test]
    fn a_popup_trigger_projects_as_a_menu_button() {
        use nana_ui_runtime::{AppContext, DocumentId, ListItem, ListItemRole};
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let entry = cx
            .create_component(
                document,
                ListItem::new("模型")
                    .role(ListItemRole::ToggleButton)
                    .has_popup(true),
            )
            .unwrap();
        let plain = cx
            .create_component(document, ListItem::new("设置").role(ListItemRole::Button))
            .unwrap();
        let project = |id| {
            let node = cx
                .world()
                .project_accessibility_nodes(&[id])
                .pop()
                .expect("projected");
            project_node(&node, None, true, 1.0)
                .into_iter()
                .next()
                .unwrap()
                .1
        };
        assert_eq!(project(entry.stable_id()).has_popup(), Some(HasPopup::Menu));
        assert_eq!(project(plain.stable_id()).has_popup(), None);
    }

    /// A live region carries its politeness to AccessKit as `aria-live`
    /// does; a node no composite made live says nothing, so the platform's
    /// default for its role stands.
    #[test]
    fn a_live_region_projects_its_politeness() {
        for (live, expected) in [
            (AccessibilityLive::Off, Live::Off),
            (AccessibilityLive::Polite, Live::Polite),
            (AccessibilityLive::Assertive, Live::Assertive),
        ] {
            let mut region = node(1, None, &[]);
            region.live = Some(live);
            let (_, projected) = project_node(&region, None, true, 1.0)
                .into_iter()
                .next()
                .unwrap();
            assert_eq!(projected.live(), Some(expected));
        }
        let (_, unset) = project_node(&node(1, None, &[]), None, true, 1.0)
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(unset.live(), None);

        use nana_ui_runtime::{AppContext, DocumentId, MutationQueue, Toast, ToastTone};
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let toast = cx
            .create_component(document, Toast::new("Saved", ToastTone::Success))
            .unwrap();
        let mut live = MutationQueue::new();
        live.set_live(toast.stable_id(), Some(AccessibilityLive::Assertive));
        cx.commit_mutations(live).unwrap();
        let node = cx
            .world()
            .project_accessibility_nodes(&[toast.stable_id()])
            .pop()
            .expect("projected");
        let (_, projected) = project_node(&node, None, true, 1.0)
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(projected.live(), Some(Live::Assertive));
    }

    #[test]
    fn a_list_item_declared_a_toggle_button_projects_as_a_pressed_button() {
        use nana_ui_runtime::{AppContext, DocumentId, ListItem, ListItemRole};
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let dock = cx
            .create_component(
                document,
                ListItem::new("设置")
                    .selected(true)
                    .role(ListItemRole::ToggleButton),
            )
            .unwrap();
        let capsule = cx
            .create_component(document, ListItem::new("Hiyori").role(ListItemRole::Button))
            .unwrap();
        let row = cx
            .create_component(document, ListItem::new("文件").selected(true))
            .unwrap();
        let project = |id| {
            let node = cx
                .world()
                .project_accessibility_nodes(&[id])
                .pop()
                .expect("projected");
            project_node(&node, None, true, 1.0)
                .into_iter()
                .next()
                .unwrap()
                .1
        };

        let open_card = project(dock.stable_id());
        assert_eq!(open_card.role(), Role::Button);
        assert_eq!(open_card.label(), Some("设置"));
        assert_eq!(open_card.toggled(), Some(Toggled::True), "pressed");
        assert_eq!(open_card.is_selected(), None, "not \"selected\"");
        assert!(open_card.supports_action(Action::Click));

        let model = project(capsule.stable_id());
        assert_eq!(model.role(), Role::Button);
        assert_eq!(model.toggled(), None);
        assert_eq!(model.is_selected(), None);

        let list_row = project(row.stable_id());
        assert_eq!(list_row.role(), Role::ListItem);
        assert_eq!(list_row.is_selected(), Some(true));
    }

    #[test]
    fn menu_item_marks_project_name_role_and_selection() {
        let project = |role, label: &str, checked, selected| {
            let mut item = node(2, Some(1), &[]);
            item.role = role;
            item.label = Some(label.into());
            item.checked = checked;
            item.selected = selected;
            project_node(&item, None, true, 1.0)
                .into_iter()
                .next()
                .unwrap()
                .1
        };

        let pinned = project(
            AccessibilityRole::Checkbox,
            "取消窗口置顶",
            Some(true),
            None,
        );
        assert_eq!(pinned.role(), Role::CheckBox);
        assert_eq!(pinned.label(), Some("取消窗口置顶"));
        assert_eq!(pinned.toggled(), Some(Toggled::True));
        assert_eq!(pinned.is_selected(), None);

        let current = project(AccessibilityRole::Radio, "场景 1", Some(true), Some(true));
        assert_eq!(current.role(), Role::RadioButton);
        assert_eq!(current.label(), Some("场景 1"));
        assert_eq!(current.toggled(), Some(Toggled::True));
        assert_eq!(current.is_selected(), Some(true));

        let other = project(AccessibilityRole::Radio, "场景 2", Some(false), None);
        assert_eq!(other.role(), Role::RadioButton);
        assert_eq!(other.toggled(), Some(Toggled::False));
        assert_eq!(other.is_selected(), None);

        let command = project(AccessibilityRole::MenuItem, "输出 Spout", None, None);
        assert_eq!(command.role(), Role::MenuItem);
        assert_eq!(command.label(), Some("输出 Spout"));
        assert_eq!(command.toggled(), None);
        assert_eq!(command.is_selected(), None);
    }

    #[test]
    fn virtual_menu_items_project_click_and_focus_actions() {
        let menu = StableNodeId::new(2).unwrap();
        let item = nana_ui_runtime::scene_contract::virtual_menu_item_id(menu, 1).unwrap();
        let mut virtual_item = node(item.get(), Some(menu.get()), &[]);
        virtual_item.role = AccessibilityRole::MenuItem;
        virtual_item.label = Some("打开".into());
        let (projector, update) = AccessibilityProjector::new(
            vec![node(menu.get(), None, &[item.get()]), virtual_item],
            true,
            1.0,
        );
        let projected = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(item.get()))
            .expect("virtual menu item projection")
            .1;
        assert!(projected.supports_action(Action::Click));
        assert!(projected.supports_action(Action::Focus));
        for action in [Action::Click, Action::Focus] {
            let request = projector
                .project_action_request(ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId(item.get()),
                    data: None,
                })
                .expect("virtual menu action")
                .action;
            assert!(matches!(
                request,
                nana_ui_runtime::AccessibilityAction::ActivateMenuItem { .. }
                    | nana_ui_runtime::AccessibilityAction::Focus
            ));
        }
    }

    #[test]
    fn tree_rows_project_as_accesskit_tree_items_with_platform_focus() {
        use nana_ui_runtime::{AppContext, DocumentId, TreeNode, TreeView};
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let tree = cx
            .create_component(
                document,
                TreeView::new([
                    TreeNode::branch(
                        "scene".into(),
                        "场景",
                        true,
                        [TreeNode::leaf("model".into(), "模型")],
                    ),
                    TreeNode::branch("light".into(), "灯光", false, []),
                ])
                .selection({
                    let mut selection = nana_ui_runtime::SelectionState::multiple();
                    selection.select_only("model".into());
                    selection
                })
                .label("大纲"),
            )
            .unwrap();
        cx.layout_document(document, nana_ui_runtime::LayoutViewport::new(300.0, 300.0))
            .unwrap();
        cx.focus_node(document, tree.stable_id()).unwrap();
        let row = |slot| {
            nana_ui_runtime::scene_contract::virtual_tree_item_id(tree.stable_id(), slot).unwrap()
        };
        let (projector, update) =
            AccessibilityProjector::new(cx.world().project_accessibility(document), true, 1.0);
        let node = |id: StableNodeId| {
            &update
                .nodes
                .iter()
                .find(|(node, _)| *node == NodeId(id.get()))
                .expect("projected node")
                .1
        };
        let root = node(tree.stable_id());
        assert_eq!(root.role(), Role::Tree);
        assert_eq!(root.label(), Some("大纲"));
        assert!(root.is_multiselectable());
        assert_eq!(
            root.children(),
            [
                NodeId(row(1).get()),
                NodeId(row(2).get()),
                NodeId(row(3).get())
            ]
        );

        let (scene, model, light) = (node(row(1)), node(row(2)), node(row(3)));
        assert!(
            [scene, model, light]
                .iter()
                .all(|row| row.role() == Role::TreeItem)
        );
        assert_eq!(scene.label(), Some("场景"));
        // AccessKit counts levels from zero
        assert_eq!(
            (scene.level(), model.level(), light.level()),
            (Some(0), Some(1), Some(0))
        );
        assert_eq!(scene.is_expanded(), Some(true));
        assert_eq!(model.is_expanded(), None);
        assert_eq!(light.is_expanded(), Some(false));
        assert_eq!(model.is_selected(), Some(true));
        assert_eq!(scene.is_selected(), Some(false));
        // the platform focus is on the active row, not on the tree
        assert_eq!(update.focus, NodeId(row(2).get()));

        assert!(model.supports_action(Action::Click));
        assert!(model.supports_action(Action::Focus));
        assert!(!model.supports_action(Action::Expand));
        assert!(!model.supports_action(Action::Collapse));
        assert!(scene.supports_action(Action::Collapse));
        assert!(!scene.supports_action(Action::Expand));
        assert!(light.supports_action(Action::Expand));
        let request = |target: StableNodeId, action| {
            projector
                .project_action_request(ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId(target.get()),
                    data: None,
                })
                .map(|request| request.action)
        };
        assert_eq!(
            request(row(2), Action::Click),
            Some(nana_ui_runtime::AccessibilityAction::Click)
        );
        assert_eq!(
            request(row(2), Action::Focus),
            Some(nana_ui_runtime::AccessibilityAction::Focus)
        );
        assert_eq!(
            request(row(1), Action::Collapse),
            Some(nana_ui_runtime::AccessibilityAction::Collapse)
        );
        assert_eq!(
            request(row(3), Action::Expand),
            Some(nana_ui_runtime::AccessibilityAction::Expand)
        );
        assert_eq!(request(row(1), Action::Expand), None);
        assert_eq!(request(row(2), Action::Expand), None);
        assert_eq!(request(row(2), Action::ScrollDown), None);
    }

    #[test]
    fn tree_row_toggles_project_as_accesskit_checkboxes() {
        use nana_ui_runtime::{AppContext, DocumentId, TreeNode, TreeToggleColumn, TreeView};
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let tree = cx
            .create_component(
                document,
                TreeView::new([
                    TreeNode::leaf("model".into(), "模型").toggles([true, false]),
                    TreeNode::leaf("light".into(), "灯光")
                        .toggles([true, true])
                        .disabled(true),
                ])
                .columns([
                    TreeToggleColumn::new(nana_ui_core::Icon::Eye, "显示"),
                    TreeToggleColumn::new(nana_ui_core::Icon::ShieldCheck, "锁定"),
                ])
                .label("大纲"),
            )
            .unwrap();
        cx.layout_document(document, nana_ui_runtime::LayoutViewport::new(300.0, 300.0))
            .unwrap();
        let row = |slot| {
            nana_ui_runtime::scene_contract::virtual_tree_item_id(tree.stable_id(), slot).unwrap()
        };
        let toggle = |slot, column| {
            nana_ui_runtime::scene_contract::virtual_tree_toggle_id(tree.stable_id(), slot, column)
                .unwrap()
        };
        let (projector, update) =
            AccessibilityProjector::new(cx.world().project_accessibility(document), true, 1.0);
        let node = |id: StableNodeId| {
            &update
                .nodes
                .iter()
                .find(|(node, _)| *node == NodeId(id.get()))
                .expect("projected node")
                .1
        };
        assert_eq!(
            node(row(1)).children(),
            [NodeId(toggle(1, 0).get()), NodeId(toggle(1, 1).get())]
        );
        let (visible, locked) = (node(toggle(1, 0)), node(toggle(1, 1)));
        assert_eq!(visible.role(), Role::CheckBox);
        assert_eq!(locked.role(), Role::CheckBox);
        assert_eq!(visible.label(), Some("显示"));
        assert_eq!(locked.label(), Some("锁定"));
        assert_eq!(visible.toggled(), Some(Toggled::True));
        assert_eq!(locked.toggled(), Some(Toggled::False));
        assert!(visible.supports_action(Action::Click));
        assert!(
            !visible.supports_action(Action::Focus),
            "a toggle flips; the focus stays with the rows"
        );
        let disabled = node(toggle(2, 1));
        assert!(disabled.is_disabled());
        assert_eq!(disabled.toggled(), Some(Toggled::True));
        assert!(!disabled.supports_action(Action::Click));

        let request = |target: StableNodeId, action| {
            projector
                .project_action_request(ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId(target.get()),
                    data: None,
                })
                .map(|request| (request.target, request.action))
        };
        assert_eq!(
            request(toggle(1, 1), Action::Click),
            Some((toggle(1, 1), nana_ui_runtime::AccessibilityAction::Click))
        );
        assert_eq!(request(toggle(1, 1), Action::Focus), None);
        assert_eq!(request(toggle(1, 1), Action::Expand), None);
        assert_eq!(request(toggle(2, 0), Action::Click), None, "disabled");

        // the translated click flips the toggle in the runtime
        let (target, action) = request(toggle(1, 1), Action::Click).unwrap();
        assert!(
            cx.apply_accessibility_action(
                document,
                nana_ui_runtime::AccessibilityActionRequest { target, action }
            )
            .unwrap()
        );
        assert_eq!(
            cx.read(tree, |tree| tree.nodes[0].toggles.clone()).unwrap(),
            [true, true]
        );
    }

    #[test]
    fn an_open_menu_row_wins_platform_focus_over_a_tree_row() {
        let menu_id = StableNodeId::new(5).unwrap();
        let tree_id = StableNodeId::new(4).unwrap();
        let menu_row = nana_ui_runtime::scene_contract::virtual_menu_item_id(menu_id, 1).unwrap();
        let tree_row = nana_ui_runtime::scene_contract::virtual_tree_item_id(tree_id, 1).unwrap();
        let mut tree = node(tree_id.get(), Some(1), &[tree_row.get()]);
        tree.role = AccessibilityRole::Tree;
        tree.focused = true;
        let mut row = node(tree_row.get(), Some(tree_id.get()), &[]);
        row.role = AccessibilityRole::TreeItem;
        row.focused = true;
        let mut menu = node(menu_id.get(), Some(1), &[menu_row.get()]);
        menu.role = AccessibilityRole::Menu;
        let mut item = node(menu_row.get(), Some(menu_id.get()), &[]);
        item.role = AccessibilityRole::MenuItem;
        item.focused = true;
        let (_, update) = AccessibilityProjector::new(
            vec![
                node(1, None, &[tree_id.get(), menu_id.get()]),
                tree.clone(),
                row.clone(),
                menu.clone(),
                item,
            ],
            true,
            1.0,
        );
        assert_eq!(update.focus, NodeId(menu_row.get()));
        let (_, update) = AccessibilityProjector::new(
            vec![node(1, None, &[tree_id.get()]), tree, row],
            true,
            1.0,
        );
        assert_eq!(update.focus, NodeId(tree_row.get()));
    }

    #[test]
    fn a_checkable_virtual_menu_row_still_activates_on_click() {
        let menu = StableNodeId::new(2).unwrap();
        let item = nana_ui_runtime::scene_contract::virtual_menu_item_id(menu, 1).unwrap();
        let mut virtual_item = node(item.get(), Some(menu.get()), &[]);
        virtual_item.role = AccessibilityRole::Checkbox;
        virtual_item.label = Some("取消窗口置顶".into());
        virtual_item.checked = Some(true);
        let (projector, _) = AccessibilityProjector::new(
            vec![node(menu.get(), None, &[item.get()]), virtual_item],
            true,
            1.0,
        );
        let request = projector
            .project_action_request(ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: NodeId(item.get()),
                data: None,
            })
            .expect("checkbox menu row click");
        assert!(matches!(
            request.action,
            nana_ui_runtime::AccessibilityAction::ActivateMenuItem { slot: 1, .. }
        ));
    }

    #[test]
    fn an_open_combo_box_projects_its_list_box_and_activates_an_option() {
        let field = StableNodeId::new(2).unwrap();
        let listbox = nana_ui_runtime::scene_contract::virtual_listbox_id(field).unwrap();
        let first = nana_ui_runtime::scene_contract::virtual_menu_item_id(field, 0x8001).unwrap();
        let second = nana_ui_runtime::scene_contract::virtual_menu_item_id(field, 0x8002).unwrap();
        let mut combo = node(field.get(), Some(1), &[listbox.get()]);
        combo.role = AccessibilityRole::ComboBox;
        combo.focused = true;
        combo.expanded = Some(true);
        combo.active_descendant = Some(second);
        let mut list = node(
            listbox.get(),
            Some(field.get()),
            &[first.get(), second.get()],
        );
        list.role = AccessibilityRole::ListBox;
        list.set_size = Some(2);
        list.multiselectable = true;
        let option = |id: StableNodeId, position: u32, selected: bool| {
            let mut option = node(id.get(), Some(listbox.get()), &[]);
            option.role = AccessibilityRole::ListBoxOption;
            option.position_in_set = Some(position);
            option.set_size = Some(2);
            option.selected = Some(selected);
            option
        };
        let (projector, update) = AccessibilityProjector::new(
            vec![
                node(1, None, &[field.get()]),
                combo,
                list,
                option(first, 1, true),
                option(second, 2, false),
            ],
            true,
            1.0,
        );
        let projected = |id: StableNodeId| {
            update
                .nodes
                .iter()
                .find(|(node_id, _)| *node_id == NodeId(id.get()))
                .unwrap()
                .1
                .clone()
        };
        // The focus stays on the field; the option the keyboard is on is its
        // active descendant, and the field says it is open, not selected.
        assert_eq!(update.focus, NodeId(field.get()));
        let combo = projected(field);
        assert_eq!(combo.active_descendant(), Some(NodeId(second.get())));
        assert_eq!(combo.is_expanded(), Some(true));
        assert_eq!(combo.is_selected(), None);
        assert!(combo.supports_action(Action::Collapse));
        assert!(!combo.supports_action(Action::Expand));
        let list = projected(listbox);
        assert_eq!(list.role(), Role::ListBox);
        assert_eq!(list.size_of_set(), Some(2));
        assert!(list.is_multiselectable());
        let (chosen, other) = (projected(first), projected(second));
        assert_eq!(chosen.role(), Role::ListBoxOption);
        assert_eq!(chosen.is_selected(), Some(true));
        assert_eq!(other.is_selected(), Some(false));
        // AccessKit counts positions from zero.
        assert_eq!(
            (chosen.position_in_set(), other.position_in_set()),
            (Some(0), Some(1))
        );
        assert!(other.supports_action(Action::Click));
        let request = projector
            .project_action_request(ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: NodeId(second.get()),
                data: None,
            })
            .expect("option click");
        assert_eq!(
            request.action,
            nana_ui_runtime::AccessibilityAction::ActivateMenuItem {
                menu: field,
                slot: 0x8002,
            }
        );
    }

    #[test]
    fn list_items_and_combo_boxes_project_runtime_actions() {
        let root = node(1, None, &[2, 3]);
        let mut list_item = node(2, Some(1), &[]);
        list_item.role = AccessibilityRole::ListItem;
        list_item.label = Some("素材".into());
        let mut combo_box = node(3, Some(1), &[]);
        combo_box.role = AccessibilityRole::ComboBox;
        combo_box.label = Some("主题".into());

        let (projector, update) =
            AccessibilityProjector::new(vec![root, list_item, combo_box], true, 1.0);
        for (id, actions) in [
            (NodeId(2), [Action::Click, Action::Focus]),
            (NodeId(3), [Action::Click, Action::Focus]),
        ] {
            let projected = &update
                .nodes
                .iter()
                .find(|(node_id, _)| *node_id == id)
                .expect("projected interactive node")
                .1;
            for action in actions {
                assert!(
                    projected.supports_action(action),
                    "{action:?} should be advertised for {id:?}"
                );
                let request = projector
                    .project_action_request(ActionRequest {
                        action,
                        target_tree: TreeId::ROOT,
                        target_node: id,
                        data: None,
                    })
                    .expect("projected action request");
                assert_eq!(request.target, StableNodeId::new(id.0).unwrap());
                assert_eq!(
                    request.action,
                    match action {
                        Action::Click => nana_ui_runtime::AccessibilityAction::Click,
                        Action::Focus => nana_ui_runtime::AccessibilityAction::Focus,
                        _ => unreachable!(),
                    }
                );
            }
        }
    }

    #[test]
    fn sliders_and_combos_project_step_and_expansion_actions() {
        let root = node(1, None, &[2, 3]);
        let mut slider = node(2, Some(1), &[]);
        slider.role = AccessibilityRole::Slider;
        slider.numeric_minimum = Some(0.0);
        slider.numeric_maximum = Some(10.0);
        slider.numeric_step = Some(1.0);
        let mut opened = node(3, Some(1), &[]);
        opened.role = AccessibilityRole::ComboBox;
        opened.selected = Some(true);
        let (projector, update) =
            AccessibilityProjector::new(vec![root, slider, opened], true, 1.0);
        let slider_node = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .unwrap()
            .1
            .clone();
        assert!(slider_node.supports_action(Action::Increment));
        assert!(slider_node.supports_action(Action::Decrement));
        for action in [Action::Increment, Action::Decrement] {
            let request = projector
                .project_action_request(ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId(2),
                    data: None,
                })
                .unwrap();
            assert_eq!(
                request.action,
                match action {
                    Action::Increment => nana_ui_runtime::AccessibilityAction::Increment,
                    Action::Decrement => nana_ui_runtime::AccessibilityAction::Decrement,
                    _ => unreachable!(),
                }
            );
        }
        let combo_node = update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(3))
            .unwrap()
            .1
            .clone();
        assert!(!combo_node.supports_action(Action::Expand));
        assert!(combo_node.supports_action(Action::Collapse));
        let request = projector
            .project_action_request(ActionRequest {
                action: Action::Collapse,
                target_tree: TreeId::ROOT,
                target_node: NodeId(3),
                data: None,
            })
            .unwrap();
        assert_eq!(
            request.action,
            nana_ui_runtime::AccessibilityAction::Collapse
        );
    }

    #[test]
    fn scrollable_nodes_project_range_and_directional_actions() {
        let root = node(1, None, &[2]);
        let mut scroll = node(2, Some(1), &[]);
        scroll.scroll_y = Some(nana_ui_runtime::AccessibilityScrollAxis {
            value: 12.0,
            minimum: 0.0,
            maximum: 240.0,
        });
        let vertical = scroll.scroll_y;
        let (projector, update) = AccessibilityProjector::new(vec![root, scroll], true, 1.0);
        let projected = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .expect("projected scroll container")
            .1;
        assert_eq!(projected.role(), Role::ScrollView);
        assert_eq!(projected.scroll_y(), Some(12.0));
        assert_eq!(projected.scroll_y_min(), Some(0.0));
        assert_eq!(projected.scroll_y_max(), Some(240.0));
        assert!(projected.scroll_y_max().unwrap() > projected.scroll_y_min().unwrap());
        assert!(projected.supports_action(Action::ScrollUp));
        assert!(projected.supports_action(Action::ScrollDown));
        let plain = node(4, Some(1), &[]);
        let mut list = node(5, Some(1), &[]);
        list.role = AccessibilityRole::List;
        list.scroll_y = vertical;
        let (_projector, kept) =
            AccessibilityProjector::new(vec![node(1, None, &[4, 5]), plain, list], true, 1.0);
        let role_of = |id: u64| {
            kept.nodes
                .iter()
                .find(|(node_id, _)| *node_id == NodeId(id))
                .expect("projected node")
                .1
                .role()
        };
        assert_eq!(role_of(4), Role::GenericContainer);
        assert_eq!(role_of(5), Role::List);
        for (action, expected) in [
            (
                Action::ScrollUp,
                nana_ui_runtime::AccessibilityScrollDirection::Up,
            ),
            (
                Action::ScrollDown,
                nana_ui_runtime::AccessibilityScrollDirection::Down,
            ),
        ] {
            let request = projector
                .project_action_request(ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId(2),
                    data: None,
                })
                .expect("scroll action request");
            assert_eq!(
                request.action,
                nana_ui_runtime::AccessibilityAction::Scroll(
                    expected,
                    nana_ui_runtime::AccessibilityScrollUnit::Page
                )
            );
        }
        // A small increment scrolls a line, not a page.
        let request = projector
            .project_action_request(ActionRequest {
                action: Action::ScrollDown,
                target_tree: TreeId::ROOT,
                target_node: NodeId(2),
                data: Some(ActionData::ScrollUnit(ScrollUnit::Item)),
            })
            .expect("line scroll request");
        assert_eq!(
            request.action,
            nana_ui_runtime::AccessibilityAction::Scroll(
                nana_ui_runtime::AccessibilityScrollDirection::Down,
                nana_ui_runtime::AccessibilityScrollUnit::Item
            )
        );
        // Set scroll percent arrives as an offset.
        let request = projector
            .project_action_request(ActionRequest {
                action: Action::SetScrollOffset,
                target_tree: TreeId::ROOT,
                target_node: NodeId(2),
                data: Some(ActionData::SetScrollOffset(accesskit::Point::new(
                    0.0, 120.0,
                ))),
            })
            .expect("set scroll offset request");
        assert_eq!(
            request.action,
            nana_ui_runtime::AccessibilityAction::SetScrollOffset(nana_ui_runtime::ScrollOffset {
                x: 0.0,
                y: 120.0
            })
        );
    }

    #[cfg(all(feature = "hosted", not(target_os = "android")))]
    #[test]
    fn a_text_run_reads_the_way_its_writing_mode_runs() {
        use nana_ui_core::{DirSpec, WritingContext, WritingModeSpec};
        let direction = |writing| {
            let root = node(1, None, &[2]);
            let mut input = node(2, Some(1), &[]);
            input.role = AccessibilityRole::TextInput;
            input.editable = true;
            input.value = Some("縦書き".into());
            input.writing = writing;
            let (projector, update) = AccessibilityProjector::new(vec![root, input], true, 1.0);
            let text_run_id = projector.text_runs[&StableNodeId::new(2).unwrap()];
            let text_run = &update
                .nodes
                .iter()
                .find(|(id, _)| *id == text_run_id)
                .unwrap()
                .1;
            text_run.text_direction()
        };
        let context = WritingContext::new;
        assert_eq!(
            direction(context(WritingModeSpec::HorizontalTb, DirSpec::Ltr)),
            Some(TextDirection::LeftToRight)
        );
        assert_eq!(
            direction(context(WritingModeSpec::HorizontalTb, DirSpec::Rtl)),
            Some(TextDirection::RightToLeft)
        );
        assert_eq!(
            direction(context(WritingModeSpec::VerticalRl, DirSpec::Ltr)),
            Some(TextDirection::TopToBottom)
        );
        assert_eq!(
            direction(context(WritingModeSpec::VerticalLr, DirSpec::Rtl)),
            Some(TextDirection::BottomToTop)
        );
    }

    #[test]
    fn unicode_text_run_selection_round_trips_without_byte_index_loss() {
        let root = node(1, None, &[2]);
        let mut input = node(2, Some(1), &[]);
        input.role = AccessibilityRole::TextInput;
        input.editable = true;
        input.value = Some("你a".into());
        input.selection = Some(nana_ui_runtime::TextSelection::new(0, "你".len()));
        let (projector, update) = AccessibilityProjector::new(vec![root, input], true, 1.0);
        let text_run_id = projector.text_runs[&StableNodeId::new(2).unwrap()];
        let input = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .unwrap()
            .1;
        assert!(input.children().contains(&text_run_id));
        assert!(input.supports_action(Action::SetTextSelection));
        let selection = input.text_selection().unwrap();
        assert_eq!(selection.anchor.character_index, 0);
        assert_eq!(selection.focus.character_index, 1);

        let text_run = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == text_run_id)
            .unwrap()
            .1;
        assert_eq!(text_run.role(), Role::TextRun);
        assert_eq!(text_run.value(), Some("你a"));
        assert_eq!(text_run.character_lengths(), [3, 1]);

        let projected = projector
            .project_action_request(ActionRequest {
                action: Action::SetTextSelection,
                target_tree: TreeId::ROOT,
                target_node: NodeId(2),
                data: Some(ActionData::SetTextSelection(AccessKitTextSelection {
                    anchor: TextPosition {
                        node: text_run_id,
                        character_index: 2,
                    },
                    focus: TextPosition {
                        node: text_run_id,
                        character_index: 1,
                    },
                })),
            })
            .unwrap();
        assert_eq!(
            projected.action,
            nana_ui_runtime::AccessibilityAction::SetSelection(
                nana_ui_runtime::TextSelection::new("你a".len(), "你".len())
            )
        );

        for (node, character_index) in [(text_run_id, 3), (NodeId(2), 0)] {
            assert!(
                projector
                    .project_action_request(ActionRequest {
                        action: Action::SetTextSelection,
                        target_tree: TreeId::ROOT,
                        target_node: NodeId(2),
                        data: Some(ActionData::SetTextSelection(AccessKitTextSelection {
                            anchor: TextPosition {
                                node,
                                character_index,
                            },
                            focus: TextPosition {
                                node: text_run_id,
                                character_index: 0,
                            },
                        })),
                    })
                    .is_none()
            );
        }
    }

    #[test]
    fn synthetic_text_run_rekeys_if_a_runtime_id_collides() {
        let root = node(1, None, &[2]);
        let mut input = node(2, Some(1), &[]);
        input.role = AccessibilityRole::TextInput;
        let (mut projector, _) =
            AccessibilityProjector::new(vec![root.clone(), input.clone()], false, 1.0);
        let previous = projector.text_runs[&StableNodeId::new(2).unwrap()];
        assert_eq!(previous, NodeId(u64::MAX));

        let mut next_root = root;
        next_root
            .children
            .push(StableNodeId::new(u64::MAX).unwrap());
        let update = projector
            .synchronize(vec![next_root, input, node(u64::MAX, Some(1), &[])], 1.0)
            .unwrap();
        let replacement = projector.text_runs[&StableNodeId::new(2).unwrap()];
        assert_ne!(replacement, previous);
        assert_ne!(replacement, NodeId(u64::MAX));
        let input = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .unwrap()
            .1;
        assert!(input.children().contains(&replacement));
        assert!(!input.children().contains(&previous));
    }

    #[test]
    fn text_run_lifecycle_tracks_input_role_and_removal() {
        let root = node(1, None, &[2]);
        let mut input = node(2, Some(1), &[]);
        input.role = AccessibilityRole::TextInput;
        input.value = Some("".into());
        input.selection = Some(nana_ui_runtime::TextSelection::caret(0));
        let (mut projector, _) =
            AccessibilityProjector::new(vec![root.clone(), input.clone()], false, 1.0);
        let first_text_run = projector.text_runs[&StableNodeId::new(2).unwrap()];

        let mut generic = input.clone();
        generic.role = AccessibilityRole::Generic;
        generic.selection = None;
        let update = projector
            .synchronize(vec![root.clone(), generic], 1.0)
            .unwrap();
        let projected = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .unwrap()
            .1;
        assert!(!projected.children().contains(&first_text_run));
        assert!(projector.text_runs.is_empty());

        let update = projector
            .synchronize(vec![root.clone(), input], 1.0)
            .unwrap();
        let second_text_run = projector.text_runs[&StableNodeId::new(2).unwrap()];
        assert_ne!(second_text_run, first_text_run);
        assert!(update.nodes.iter().any(|(id, _)| *id == second_text_run));

        let update = projector
            .synchronize(vec![node(1, None, &[])], 1.0)
            .unwrap();
        let projected_root = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .unwrap()
            .1;
        assert!(projected_root.children().is_empty());
        assert!(projector.text_runs.is_empty());
    }

    #[test]
    fn scale_change_reprojects_logical_bounds_in_physical_pixels() {
        let mut root = node(1, None, &[]);
        root.bounds = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        };
        let (mut projector, _) = AccessibilityProjector::new(vec![root.clone()], false, 1.0);

        let update = projector.synchronize(vec![root], 2.0).unwrap();
        let bounds = placed(&update.nodes[0].1);
        assert_eq!(bounds.x0, 20.0);
        assert_eq!(bounds.y0, 40.0);
        assert_eq!(bounds.x1, 80.0);
        assert_eq!(bounds.y1, 120.0);
        assert!(update.tree.is_some());
    }

    #[cfg(all(feature = "hosted", not(target_os = "android")))]
    #[test]
    fn native_activation_materializes_current_state_only_when_requested() {
        let children = (2..=10_001).collect::<Vec<_>>();
        let mut nodes = vec![node(1, None, &children)];
        nodes.extend(children.iter().map(|id| node(*id, Some(1), &[])));
        let mut editor = nodes[1].clone();
        editor.role = AccessibilityRole::TextInput;
        editor.editable = true;
        editor.value = Some("Before".into());
        nodes[1] = editor.clone();
        let current = CurrentTree::from_projector(Arc::new(Mutex::new(
            AccessibilityProjector::retain(nodes, true, 1.0, Some(0), true),
        )));
        let mut activation = current.clone();
        assert_eq!(current.projector.lock().unwrap().full_updates.get(), 0);
        for generation in 1..=4 {
            editor.value = Some(format!("Edited {generation}").into());
            editor.focused = true;
            let update = current
                .synchronize(
                    AccessibilityUpdate::Delta(AccessibilityDelta {
                        generation,
                        updated: vec![editor.clone()],
                        removed: vec![],
                    }),
                    1.0,
                )
                .unwrap();
            assert_eq!(update.focus, NodeId(2));
            assert_eq!(update.nodes.len(), 2); // Editor and its text run.
            assert!(update.tree.is_none());
            assert!(current.projector.try_lock().is_ok()); // Native callbacks may reenter now.
        }
        assert_eq!(current.projector.lock().unwrap().full_updates.get(), 0);
        let before = activation.request_initial_tree().unwrap();
        assert_eq!(
            current
                .projector
                .lock()
                .unwrap()
                .global_reconciliations
                .get(),
            1
        );
        assert_eq!(before.tree.as_ref().unwrap().root, FOREST_ROOT_ID);
        assert_eq!(
            before
                .nodes
                .iter()
                .find(|(id, _)| *id == NodeId(2))
                .unwrap()
                .1
                .value(),
            Some("Edited 4")
        );
        current
            .synchronize(
                AccessibilityUpdate::Delta(AccessibilityDelta {
                    generation: 5,
                    updated: vec![],
                    removed: vec![StableNodeId::new(2).unwrap()],
                }),
                1.0,
            )
            .unwrap();
        assert_eq!(current.projector.lock().unwrap().full_updates.get(), 1);
        let after = activation.request_initial_tree().unwrap();
        assert_eq!(after.focus, FOREST_ROOT_ID);
        assert!(after.nodes.iter().all(|(id, _)| *id != NodeId(2)));
        assert!(before.nodes.iter().any(|(id, _)| *id == NodeId(2)));
        assert_eq!(after.nodes.len(), 10_001); // Runtime nodes plus the window, no orphan text run.
        assert_eq!(current.projector.lock().unwrap().full_updates.get(), 2);
    }

    #[cfg(all(feature = "hosted", not(target_os = "android")))]
    #[test]
    fn activation_always_returns_the_latest_complete_tree() {
        let (initial, _) = AccessibilityProjector::new(vec![node(1, None, &[])], false, 1.0);
        let current = Arc::new(Mutex::new(initial));
        let mut activation = CurrentTree::from_projector(Arc::clone(&current));

        assert_eq!(
            activation
                .request_initial_tree()
                .unwrap()
                .tree
                .unwrap()
                .root,
            NodeId(1)
        );
        assert_eq!(
            activation
                .request_initial_tree()
                .unwrap()
                .tree
                .unwrap()
                .root,
            NodeId(1)
        );

        let (replacement, _) = AccessibilityProjector::new(vec![node(9, None, &[])], false, 1.0);
        *current.lock().unwrap() = replacement;
        assert_eq!(
            activation
                .request_initial_tree()
                .unwrap()
                .tree
                .unwrap()
                .root,
            NodeId(9)
        );
    }

    #[cfg(all(feature = "hosted", not(target_os = "android")))]
    #[test]
    fn action_queue_is_bounded_and_preserves_accepted_fifo_order() {
        let mut requests = VecDeque::new();
        for index in 0..=MAX_PENDING_ACCESSIBILITY_ACTIONS {
            let accepted = enqueue_accessibility_action(
                &mut requests,
                ActionRequest {
                    action: Action::Click,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId(index as u64 + 1),
                    data: None,
                },
            );
            assert_eq!(accepted, index < MAX_PENDING_ACCESSIBILITY_ACTIONS);
        }
        assert_eq!(requests.len(), MAX_PENDING_ACCESSIBILITY_ACTIONS);
        assert_eq!(requests.front().unwrap().target_node, NodeId(1));
        assert_eq!(
            requests.back().unwrap().target_node,
            NodeId(MAX_PENDING_ACCESSIBILITY_ACTIONS as u64)
        );
    }

    #[cfg(all(feature = "hosted", not(target_os = "android")))]
    #[test]
    fn set_value_action_requires_a_text_payload() {
        let root = node(1, None, &[7]);
        let mut input = node(7, Some(1), &[]);
        input.role = AccessibilityRole::TextInput;
        input.editable = true;
        input.value = Some("旧值".into());
        input.selection = Some(nana_ui_runtime::TextSelection::caret("旧值".len()));
        let (projector, _) = AccessibilityProjector::new(vec![root, input], true, 1.0);

        let projected = projector
            .project_action_request(ActionRequest {
                action: Action::SetValue,
                target_tree: TreeId::ROOT,
                target_node: NodeId(7),
                data: Some(ActionData::Value("新的值".into())),
            })
            .unwrap();
        assert_eq!(projected.target, StableNodeId::new(7).unwrap());
        assert_eq!(
            projected.action,
            nana_ui_runtime::AccessibilityAction::SetValue("新的值".into())
        );

        assert!(
            projector
                .project_action_request(ActionRequest {
                    action: Action::SetValue,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId(7),
                    data: None,
                })
                .is_none()
        );
    }

    #[cfg(all(feature = "hosted", not(target_os = "android")))]
    #[test]
    fn range_set_value_accepts_only_finite_numeric_payloads() {
        let root = node(1, None, &[8]);
        let mut range = node(8, Some(1), &[]);
        range.role = AccessibilityRole::Slider;
        range.numeric_minimum = Some(0.0);
        range.numeric_maximum = Some(10.0);
        range.numeric_step = Some(0.5);
        range.numeric_value = Some(4.0);
        let (projector, _) = AccessibilityProjector::new(vec![root, range], true, 1.0);

        let projected = projector
            .project_action_request(ActionRequest {
                action: Action::SetValue,
                target_tree: TreeId::ROOT,
                target_node: NodeId(8),
                data: Some(ActionData::NumericValue(4.5)),
            })
            .unwrap();
        assert_eq!(
            projected.action,
            nana_ui_runtime::AccessibilityAction::SetValue("4.5".into())
        );

        assert!(
            projector
                .project_action_request(ActionRequest {
                    action: Action::SetValue,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId(8),
                    data: Some(ActionData::NumericValue(f64::NAN)),
                })
                .is_none()
        );
    }

    #[cfg(all(feature = "hosted", not(target_os = "android")))]
    #[test]
    fn multiselectable_list_table_and_aria_selected_project_to_accesskit() {
        let mut root = node(1, None, &[2, 4]);

        let mut list = node(2, Some(1), &[3]);
        list.role = AccessibilityRole::List;
        list.multiselectable = true;
        let mut item = node(3, Some(2), &[]);
        item.role = AccessibilityRole::ListItem;
        item.selected = Some(true);

        let mut table = node(4, Some(1), &[5]);
        table.role = AccessibilityRole::Table;
        table.multiselectable = true;
        let mut card = node(5, Some(4), &[]);
        card.role = AccessibilityRole::Generic;
        card.selected = Some(true);

        root.focused = false;
        let (_, update) =
            AccessibilityProjector::new(vec![root, list, item, table, card], true, 1.0);
        let projected = |id: u64| {
            update
                .nodes
                .iter()
                .find(|(node_id, _)| *node_id == NodeId(id))
                .unwrap()
                .1
                .clone()
        };
        assert!(projected(2).is_multiselectable());
        assert_eq!(projected(3).is_selected(), Some(true));
        assert!(projected(4).is_multiselectable());
        assert_eq!(projected(5).is_selected(), Some(true));
    }

    #[test]
    fn incremental_text_runs_handle_collision_role_flip_and_unreachable_collider_without_global_scan()
     {
        let root = node(1, None, &[2]);
        let mut input = node(2, Some(1), &[]);
        input.role = AccessibilityRole::TextInput;
        input.value = Some("hello".into());
        let (mut projector, _) =
            AccessibilityProjector::new(vec![root.clone(), input.clone()], true, 1.0);
        assert_eq!(projector.global_reconciliations.get(), 1);
        let first_run = projector.text_runs[&StableNodeId::new(2).unwrap()];
        assert_eq!(first_run, NodeId(u64::MAX));
        assert_eq!(
            projector.text_runs_by_run_id.get(&first_run).copied(),
            Some(StableNodeId::new(2).unwrap())
        );

        // An unreachable node whose ID collides with `first_run` is pruned and must not disturb `first_run`.
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 1,
                updated: vec![node(u64::MAX, Some(999), &[])],
                removed: vec![],
            })
            .unwrap();
        assert!(update.tree.is_none());
        assert_eq!(
            projector.text_runs[&StableNodeId::new(2).unwrap()],
            first_run
        );
        assert!(
            !projector
                .nodes
                .contains_key(&StableNodeId::new(u64::MAX).unwrap())
        );
        assert_eq!(projector.global_reconciliations.get(), 1);

        // A reachable node with ID `u64::MAX` collides and rekeys node 2's text run incrementally.
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 2,
                updated: vec![node(u64::MAX, Some(1), &[])],
                removed: vec![],
            })
            .unwrap();
        assert!(update.tree.is_none());
        let second_run = projector.text_runs[&StableNodeId::new(2).unwrap()];
        assert_ne!(second_run, first_run);
        assert_ne!(second_run, NodeId(u64::MAX));
        assert_eq!(projector.text_runs.len(), 1);
        assert_eq!(projector.text_runs_by_run_id.len(), 1);
        assert_eq!(
            projector.text_runs_by_run_id.get(&second_run).copied(),
            Some(StableNodeId::new(2).unwrap())
        );
        let projected_input = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(2))
            .expect("rekeyed input must be emitted")
            .1;
        assert!(projected_input.children().contains(&second_run));
        assert!(!projected_input.children().contains(&first_run));
        assert_eq!(projector.global_reconciliations.get(), 1);

        // Role flip away from TextInput releases both maps; removing a TextInput subtree releases it too.
        let mut flipped = input.clone();
        flipped.role = AccessibilityRole::Generic;
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 3,
                updated: vec![flipped],
                removed: vec![],
            })
            .unwrap();
        assert!(update.tree.is_none());
        assert!(projector.text_runs.is_empty());
        assert!(projector.text_runs_by_run_id.is_empty());
        assert_eq!(projector.global_reconciliations.get(), 1);

        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 4,
                updated: vec![input],
                removed: vec![],
            })
            .unwrap();
        assert!(update.tree.is_none());
        let third_run = projector.text_runs[&StableNodeId::new(2).unwrap()];
        assert_ne!(third_run, second_run);
        assert_eq!(projector.text_runs_by_run_id.len(), 1);
        assert_eq!(projector.global_reconciliations.get(), 1);

        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 5,
                updated: vec![],
                removed: vec![StableNodeId::new(2).unwrap()],
            })
            .unwrap();
        assert!(update.tree.is_none());
        assert!(projector.text_runs.is_empty());
        assert!(projector.text_runs_by_run_id.is_empty());
        assert_eq!(projector.global_reconciliations.get(), 1);
    }

    #[test]
    fn incremental_roots_track_promotion_demotion_and_multi_root_forest_order() {
        let root = node(5, None, &[2]);
        let child = node(2, Some(5), &[]);
        let (mut projector, initial) = AccessibilityProjector::new(vec![root, child], false, 1.0);
        assert_eq!(initial.tree.unwrap().root, NodeId(5));
        assert_eq!(projector.global_reconciliations.get(), 1);

        // Promote child 2 to a second root; forest root lists roots in ascending StableNodeId order.
        let promoted = projector
            .apply_delta(AccessibilityDelta {
                generation: 1,
                updated: vec![node(5, None, &[]), node(2, None, &[])],
                removed: vec![],
            })
            .unwrap();
        assert_eq!(promoted.tree.unwrap().root, FOREST_ROOT_ID);
        let forest = &promoted
            .nodes
            .iter()
            .find(|(id, _)| *id == FOREST_ROOT_ID)
            .unwrap()
            .1;
        assert_eq!(forest.children(), [NodeId(2), NodeId(5)]);
        assert_eq!(projector.global_reconciliations.get(), 1);

        // Demote root 5 under root 2: back to single root 2.
        let demoted = projector
            .apply_delta(AccessibilityDelta {
                generation: 2,
                updated: vec![node(2, None, &[5]), node(5, Some(2), &[])],
                removed: vec![],
            })
            .unwrap();
        assert_eq!(demoted.tree.unwrap().root, NodeId(2));
        assert!(demoted.nodes.iter().all(|(id, _)| *id != FOREST_ROOT_ID));
        assert_eq!(projector.global_reconciliations.get(), 1);
    }

    #[test]
    fn localized_pruning_drops_detached_subtrees_cycles_and_self_loops_without_global_scan() {
        let root = node(1, None, &[2, 6]);
        let branch = node(2, Some(1), &[3]);
        let mut nested_input = node(3, Some(2), &[4]);
        nested_input.role = AccessibilityRole::TextInput;
        nested_input.focused = true;
        let leaf = node(4, Some(3), &[]);
        let survivor = node(6, Some(1), &[]);
        let (mut projector, _) = AccessibilityProjector::new(
            vec![root, branch, nested_input, leaf, survivor],
            true,
            1.0,
        );
        assert_eq!(projector.global_reconciliations.get(), 1);

        // Unlinking branch 2 from root 1 drops 2, 3 (and its text_run + focus), and 4 locally.
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 1,
                updated: vec![node(1, None, &[6])],
                removed: vec![],
            })
            .unwrap();
        assert!(update.tree.is_none());
        assert_eq!(projector.nodes.len(), 2);
        assert!(projector.text_runs.is_empty());
        assert!(projector.text_runs_by_run_id.is_empty());
        assert!(projector.focused.is_empty());
        assert_eq!(projector.global_reconciliations.get(), 1);

        // Reattach a 2-node branch (10 -> 11), then detach them into a cycle (10 <-> 11).
        let _ = projector
            .apply_delta(AccessibilityDelta {
                generation: 2,
                updated: vec![
                    node(1, None, &[6, 10]),
                    node(10, Some(1), &[11]),
                    node(11, Some(10), &[]),
                ],
                removed: vec![],
            })
            .unwrap();
        assert_eq!(projector.nodes.len(), 4);
        assert_eq!(projector.global_reconciliations.get(), 1);

        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 3,
                updated: vec![
                    node(1, None, &[6]),
                    node(10, Some(11), &[11]),
                    node(11, Some(10), &[10]),
                ],
                removed: vec![],
            })
            .unwrap();
        assert!(update.tree.is_none());
        assert!(
            !projector
                .nodes
                .contains_key(&StableNodeId::new(10).unwrap())
        );
        assert!(
            !projector
                .nodes
                .contains_key(&StableNodeId::new(11).unwrap())
        );
        assert_eq!(projector.nodes.len(), 2);
        assert_eq!(projector.global_reconciliations.get(), 1);

        // Self-loop and orphan subtree with missing parent are both pruned locally.
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 4,
                updated: vec![
                    node(20, Some(20), &[20]),
                    node(21, Some(999), &[22]),
                    node(22, Some(21), &[]),
                ],
                removed: vec![],
            })
            .unwrap();
        assert!(update.tree.is_none());
        assert_eq!(projector.nodes.len(), 2);
        assert_eq!(projector.global_reconciliations.get(), 1);

        // Reparenting a child onto a cached surviving parent without the surviving parent in delta.
        let _ = projector
            .apply_delta(AccessibilityDelta {
                generation: 5,
                updated: vec![
                    node(1, None, &[6, 30]),
                    node(30, Some(1), &[31]),
                    node(31, Some(30), &[]),
                ],
                removed: vec![],
            })
            .unwrap();
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 6,
                updated: vec![node(31, Some(6), &[])],
                removed: vec![StableNodeId::new(30).unwrap()],
            })
            .unwrap();
        assert!(update.tree.is_none());
        assert!(
            projector
                .nodes
                .contains_key(&StableNodeId::new(31).unwrap())
        );
        assert_eq!(
            projector.nodes[&StableNodeId::new(6).unwrap()].children,
            vec![StableNodeId::new(31).unwrap()]
        );
        assert_eq!(projector.global_reconciliations.get(), 1);
    }

    #[test]
    fn non_tree_mismatched_parent_links_fall_back_safely_to_exact_reachability() {
        let root = node(1, None, &[2, 3]);
        let a = node(2, Some(1), &[4]);
        let b = node(3, Some(1), &[]);
        let shared = node(4, Some(2), &[]);
        let (mut projector, _) = AccessibilityProjector::new(vec![root, a, b, shared], false, 1.0);
        assert!(projector.tree_links_exact);

        // Node 3 lists 4 even though 4.parent is still Some(2), while 2 is removed.
        // Forward reachability from root 1 still reaches 4 via 3 -> 4!
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 1,
                updated: vec![node(3, Some(1), &[4])],
                removed: vec![StableNodeId::new(2).unwrap()],
            })
            .unwrap();
        assert!(update.tree.is_none());
        assert!(projector.nodes.contains_key(&StableNodeId::new(4).unwrap()));
        assert_eq!(
            projector.nodes[&StableNodeId::new(3).unwrap()].children,
            vec![StableNodeId::new(4).unwrap()]
        );
        assert!(projector.global_reconciliations.get() >= 2);
    }

    #[test]
    fn repeated_parent_and_new_child_inserts_stay_local_regardless_of_hash_order() {
        let root = node(1, None, &[2]);
        let child = node(2, Some(1), &[]);
        let (mut projector, _) = AccessibilityProjector::new(vec![root, child], true, 1.0);
        assert_eq!(projector.global_reconciliations.get(), 1);
        for offset in 0..32u64 {
            let new_id = 100 + offset;
            let update = projector
                .apply_delta(AccessibilityDelta {
                    generation: offset * 2 + 1,
                    updated: vec![node(1, None, &[2, new_id]), node(new_id, Some(1), &[])],
                    removed: vec![],
                })
                .unwrap();
            assert!(update.tree.is_none());
            assert!(projector.tree_links_exact);
            let update = projector
                .apply_delta(AccessibilityDelta {
                    generation: offset * 2 + 2,
                    updated: vec![],
                    removed: vec![StableNodeId::new(new_id).unwrap()],
                })
                .unwrap();
            assert!(update.tree.is_none());
            assert!(projector.tree_links_exact);
        }
        assert_eq!(projector.global_reconciliations.get(), 1);
    }

    #[test]
    fn removed_child_is_stripped_even_when_parent_is_in_updated_with_stale_children() {
        let root = node(1, None, &[2, 3]);
        let first = node(2, Some(1), &[]);
        let second = node(3, Some(1), &[]);
        let (mut projector, _) = AccessibilityProjector::new(vec![root, first, second], true, 1.0);
        assert_eq!(projector.global_reconciliations.get(), 1);

        // Case 1: parent is in `updated` (e.g. label changed) with unchanged `children = [2, 3]`,
        // while child 2 is in `removed`.
        let mut parent_prop_update = node(1, None, &[2, 3]);
        parent_prop_update.label = Some("Renamed".into());
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 1,
                updated: vec![parent_prop_update],
                removed: vec![StableNodeId::new(2).unwrap()],
            })
            .unwrap();
        assert!(update.tree.is_none());
        let projected_root = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .expect("root emitted")
            .1;
        assert_eq!(projected_root.children(), [NodeId(3)].as_slice());
        assert_eq!(projected_root.label(), Some("Renamed"));
        assert_eq!(
            projector.nodes[&StableNodeId::new(1).unwrap()].children,
            vec![StableNodeId::new(3).unwrap()]
        );
        assert!(projector.tree_links_exact);
        assert_eq!(projector.global_reconciliations.get(), 1);

        // Case 2: parent adds sibling 4 in `updated` while still listing removed child 3 in its prefix.
        let update = projector
            .apply_delta(AccessibilityDelta {
                generation: 2,
                updated: vec![node(1, None, &[3, 4]), node(4, Some(1), &[])],
                removed: vec![StableNodeId::new(3).unwrap()],
            })
            .unwrap();
        assert!(update.tree.is_none());
        let projected_root = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .expect("root emitted")
            .1;
        assert_eq!(projected_root.children(), [NodeId(4)].as_slice());
        assert_eq!(
            projector.nodes[&StableNodeId::new(1).unwrap()].children,
            vec![StableNodeId::new(4).unwrap()]
        );
        assert!(projector.tree_links_exact);
        assert_eq!(projector.global_reconciliations.get(), 1);
    }

    #[test]
    fn text_index_conversion_handles_ascii_and_multibyte_boundaries() {
        assert_eq!(byte_to_character_index("", 0), Some(0));
        assert_eq!(byte_to_character_index("", 1), None);
        assert_eq!(character_index_to_byte("", 0), Some(0));
        assert_eq!(character_index_to_byte("", 1), None);

        assert_eq!(byte_to_character_index("hello", 0), Some(0));
        assert_eq!(byte_to_character_index("hello", 3), Some(3));
        assert_eq!(byte_to_character_index("hello", 5), Some(5));
        assert_eq!(byte_to_character_index("hello", 6), None);
        assert_eq!(character_index_to_byte("hello", 0), Some(0));
        assert_eq!(character_index_to_byte("hello", 3), Some(3));
        assert_eq!(character_index_to_byte("hello", 5), Some(5));
        assert_eq!(character_index_to_byte("hello", 6), None);

        let mixed = "a你🦀z";
        assert_eq!(byte_to_character_index(mixed, 0), Some(0));
        assert_eq!(byte_to_character_index(mixed, 1), Some(1));
        assert_eq!(byte_to_character_index(mixed, 2), None); // inside '你'
        assert_eq!(byte_to_character_index(mixed, 4), Some(2));
        assert_eq!(byte_to_character_index(mixed, 8), Some(3));
        assert_eq!(byte_to_character_index(mixed, 9), Some(4));
        assert_eq!(byte_to_character_index(mixed, 10), None);
        assert_eq!(character_index_to_byte(mixed, 0), Some(0));
        assert_eq!(character_index_to_byte(mixed, 1), Some(1));
        assert_eq!(character_index_to_byte(mixed, 2), Some(4));
        assert_eq!(character_index_to_byte(mixed, 3), Some(8));
        assert_eq!(character_index_to_byte(mixed, 4), Some(9));
        assert_eq!(character_index_to_byte(mixed, 5), None);
    }
}
