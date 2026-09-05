//! Scene order projection.

use super::*;

impl UiScene {
    pub(super) fn sort_primitives(&mut self) {
        // The group prefix is a per-node parent walk; a node usually owns
        // several primitives, so walk once and reuse it.
        let mut prefixes: HashMap<StableNodeId, Vec<(i32, usize)>> = HashMap::new();
        let keys: Vec<SceneOrderKey> = self
            .primitives
            .values()
            .map(|primitive| {
                let prefix = prefixes
                    .entry(primitive.node)
                    .or_insert_with(|| group_prefix(&self.nodes, &self.node_order, primitive.node));
                order_key_from_prefix(&self.nodes, prefix, primitive)
            })
            .collect();
        self.ordered.clear();
        self.ordered.extend(keys);
    }
}

impl UiScene {
    pub(super) fn visit_order(
        &mut self,
        id: StableNodeId,
        visited: &mut HashSet<StableNodeId>,
        order: &mut usize,
    ) {
        if !visited.insert(id) {
            return;
        }
        self.node_order.insert(id, *order);
        *order += 1;
        let children = self
            .nodes
            .get(&id)
            .map(|node| Arc::clone(&node.children))
            .unwrap_or_else(|| Arc::new(Vec::new()));
        for child in children.iter().copied() {
            self.visit_order(child, visited, order);
        }
    }
}

impl UiScene {
    pub(super) fn rebuild_document_order(&mut self) {
        self.node_order.clear();
        let mut roots = self
            .nodes
            .values()
            .filter(|node| node.parent.is_none() || !self.nodes.contains_key(&node.parent.unwrap()))
            .map(|node| node.id)
            .collect::<Vec<_>>();
        roots.sort_unstable();
        let mut visited = HashSet::new();
        let mut order = 0;
        for root in roots {
            self.visit_order(root, &mut visited, &mut order);
        }
        // Malformed/incomplete deltas must not make retained nodes disappear.
        let mut detached = self.nodes.keys().copied().collect::<Vec<_>>();
        detached.sort_unstable();
        for id in detached {
            if !visited.contains(&id) {
                self.visit_order(id, &mut visited, &mut order);
            }
        }
    }
}
