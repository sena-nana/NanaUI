//! Business agnostic retained composition declarations.
//!
//! Applications describe structure with stable ids and hand the validated
//! tree to their renderer/host.  This module deliberately contains no
//! product or widget types.

use crate::{
    AppContext, ComponentView, DocumentId, Entity, FrameworkError, MutationQueue, StableNodeId,
};
use std::collections::{HashMap, HashSet};

/// Stable identity for a node in a composition tree.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CompositionId(String);

impl CompositionId {
    pub fn new(value: impl Into<String>) -> Result<Self, CompositionError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(CompositionError::EmptyId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&'static str> for CompositionId {
    fn from(value: &'static str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for CompositionId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Structural role.  Rendering and widget selection remain application-owned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionNodeKind {
    Page,
    Pane,
    Group,
    Option,
    Slot,
    Extension,
}

/// One immutable node in a composition declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionNode {
    pub id: CompositionId,
    pub kind: CompositionNodeKind,
    pub children: Vec<CompositionNode>,
}

impl CompositionNode {
    pub fn leaf(id: impl Into<CompositionId>, kind: CompositionNodeKind) -> Self {
        Self {
            id: id.into(),
            kind,
            children: Vec::new(),
        }
    }

    pub fn with_children(
        id: impl Into<CompositionId>,
        kind: CompositionNodeKind,
        children: impl IntoIterator<Item = CompositionNode>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            children: children.into_iter().collect(),
        }
    }
}

/// A complete composition tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionSpec {
    pub root: CompositionNode,
}

impl CompositionSpec {
    pub fn new(root: CompositionNode) -> Self {
        Self { root }
    }

    pub fn validate(&self) -> Result<CompositionIndex, CompositionError> {
        if self.root.kind != CompositionNodeKind::Page {
            return Err(CompositionError::RootMustBePage);
        }
        let mut index = CompositionIndex {
            nodes: HashMap::new(),
        };
        let mut path = HashSet::new();
        visit(&self.root, None, &mut path, &mut index)?;
        Ok(index)
    }
}

fn visit(
    node: &CompositionNode,
    parent: Option<CompositionId>,
    path: &mut HashSet<CompositionId>,
    index: &mut CompositionIndex,
) -> Result<(), CompositionError> {
    if node.id.as_str().trim().is_empty() {
        return Err(CompositionError::EmptyId);
    }
    if !path.insert(node.id.clone()) {
        return Err(CompositionError::Cycle(node.id.clone()));
    }
    if index.nodes.contains_key(&node.id) {
        return Err(CompositionError::DuplicateId(node.id.clone()));
    }
    for child in &node.children {
        if !valid_child(node.kind, child.kind) {
            return Err(CompositionError::InvalidChild {
                parent: node.kind,
                child: child.kind,
            });
        }
        visit(child, Some(node.id.clone()), path, index)?;
    }
    path.remove(&node.id);
    index.nodes.insert(
        node.id.clone(),
        CompositionEntry {
            parent,
            kind: node.kind,
        },
    );
    Ok(())
}

fn valid_child(parent: CompositionNodeKind, child: CompositionNodeKind) -> bool {
    match parent {
        CompositionNodeKind::Page => matches!(
            child,
            CompositionNodeKind::Pane | CompositionNodeKind::Group | CompositionNodeKind::Slot
        ),
        CompositionNodeKind::Pane => matches!(
            child,
            CompositionNodeKind::Group | CompositionNodeKind::Option | CompositionNodeKind::Slot
        ),
        CompositionNodeKind::Group => matches!(
            child,
            CompositionNodeKind::Group
                | CompositionNodeKind::Option
                | CompositionNodeKind::Slot
                | CompositionNodeKind::Extension
        ),
        CompositionNodeKind::Option | CompositionNodeKind::Extension => false,
        CompositionNodeKind::Slot => matches!(
            child,
            CompositionNodeKind::Group
                | CompositionNodeKind::Option
                | CompositionNodeKind::Slot
                | CompositionNodeKind::Extension
        ),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionEntry {
    pub parent: Option<CompositionId>,
    pub kind: CompositionNodeKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompositionIndex {
    nodes: HashMap<CompositionId, CompositionEntry>,
}

impl CompositionIndex {
    pub fn get(&self, id: &CompositionId) -> Option<&CompositionEntry> {
        self.nodes.get(id)
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositionError {
    EmptyId,
    AlreadyMounted,
    MissingRenderer(CompositionId),
    DuplicateRenderer(CompositionId),
    Runtime(String),
    RootMustBePage,
    DuplicateId(CompositionId),
    Cycle(CompositionId),
    InvalidChild {
        parent: CompositionNodeKind,
        child: CompositionNodeKind,
    },
    NotSlot(CompositionId),
    MissingParent(StableNodeId),
    InvalidSlotParent {
        slot: CompositionId,
        parent: StableNodeId,
    },
    CrossHostParent {
        parent: StableNodeId,
        owner: StableNodeId,
    },
}

/// A typed component factory. Component creation still goes through the
/// Runtime component registry; this table only binds declaration identities
/// to their component values.
type ComponentFactory =
    Box<dyn Fn(&mut AppContext, DocumentId) -> Result<StableNodeId, FrameworkError>>;

#[derive(Default)]
pub struct CompositionRegistry {
    factories: HashMap<CompositionId, ComponentFactory>,
}

impl CompositionRegistry {
    /// Register a built-in or extension component without exposing a raw
    /// RuntimeDocument tree builder to the declaring page.
    pub fn register<C: ComponentView + Clone>(
        &mut self,
        id: impl Into<CompositionId>,
        component: C,
    ) -> Result<(), CompositionError> {
        let id = id.into();
        if id.as_str().trim().is_empty() {
            return Err(CompositionError::EmptyId);
        }
        if self.factories.contains_key(&id) {
            return Err(CompositionError::DuplicateRenderer(id));
        }
        self.factories.insert(
            id,
            Box::new(move |cx, document| {
                cx.create_detached_component(document, component.clone())
                    .map(|entity| entity.stable_id())
            }),
        );
        Ok(())
    }
}

/// Owns one real Runtime subtree and its identity-to-entity mapping.
/// A host must be explicitly unmounted using the same AppContext.
#[derive(Default)]
pub struct CompositionHost {
    index: Option<CompositionIndex>,
    entities: HashMap<CompositionId, StableNodeId>,
    root: Option<StableNodeId>,
}

fn runtime_error(error: FrameworkError) -> CompositionError {
    CompositionError::Runtime(error.to_string())
}

impl CompositionHost {
    pub fn mount(
        &mut self,
        cx: &mut AppContext,
        document: DocumentId,
        parent: StableNodeId,
        spec: &CompositionSpec,
        registry: &CompositionRegistry,
    ) -> Result<StableNodeId, CompositionError> {
        if self.root.is_some() {
            return Err(CompositionError::AlreadyMounted);
        }
        if let Some(owner) = cx.composition_owner(parent) {
            return Err(CompositionError::CrossHostParent { parent, owner });
        }
        let index = spec.validate()?;
        // Validate the complete registry before creating any Runtime node.
        for id in index.nodes.keys() {
            if !registry.factories.contains_key(id) {
                return Err(CompositionError::MissingRenderer(id.clone()));
            }
        }
        let mut entities = HashMap::new();
        for id in index.nodes.keys() {
            match registry.factories[id](cx, document) {
                Ok(entity) => {
                    entities.insert(id.clone(), entity);
                }
                Err(error) => {
                    let mut cleanup = MutationQueue::new();
                    for entity in entities.values() {
                        cleanup.despawn_subtree(*entity);
                    }
                    cx.commit_mutations(cleanup).map_err(runtime_error)?;
                    return Err(runtime_error(error));
                }
            }
        }
        let root = entities[&spec.root.id];
        let mut queue = MutationQueue::new();
        fn connect(
            node: &CompositionNode,
            entities: &HashMap<CompositionId, StableNodeId>,
            queue: &mut MutationQueue,
        ) {
            for child in &node.children {
                queue.insert(entities[&node.id], entities[&child.id], None);
                connect(child, entities, queue);
            }
        }
        connect(&spec.root, &entities, &mut queue);
        queue.insert(parent, root, None);
        if let Err(error) = cx.commit_mutations(queue) {
            let mut cleanup = MutationQueue::new();
            for entity in entities.values() {
                cleanup.despawn_subtree(*entity);
            }
            cx.commit_mutations(cleanup).map_err(runtime_error)?;
            return Err(runtime_error(error));
        }
        self.root = Some(root);
        self.entities = entities;
        cx.register_composition_nodes(root, self.entities.values().copied());
        self.index = Some(index);
        Ok(root)
    }

    /// Move a declared slot and its subtree to an application-owned runtime
    /// parent. Only nodes declared as `Slot` may cross a layout boundary.
    pub fn bind_slot(
        &mut self,
        cx: &mut AppContext,
        id: &CompositionId,
        parent: StableNodeId,
    ) -> Result<StableNodeId, CompositionError> {
        let index = self
            .index
            .as_ref()
            .ok_or(CompositionError::AlreadyMounted)?;
        let entry = index
            .get(id)
            .ok_or_else(|| CompositionError::MissingRenderer(id.clone()))?;
        if entry.kind != CompositionNodeKind::Slot {
            return Err(CompositionError::NotSlot(id.clone()));
        }
        if !cx.world().contains(parent) {
            return Err(CompositionError::MissingParent(parent));
        }
        if let Some(owner) = cx.composition_owner(parent)
            && self.root != Some(owner)
        {
            return Err(CompositionError::CrossHostParent { parent, owner });
        }
        let slot = self
            .entities
            .get(id)
            .copied()
            .ok_or_else(|| CompositionError::MissingRenderer(id.clone()))?;
        let slot_document = cx
            .world()
            .node(slot)
            .ok_or_else(|| CompositionError::MissingRenderer(id.clone()))?;
        let parent_document = cx
            .world()
            .node(parent)
            .ok_or(CompositionError::MissingParent(parent))?;
        if slot_document.document != parent_document.document {
            return Err(CompositionError::InvalidSlotParent {
                slot: id.clone(),
                parent,
            });
        }
        let mut ancestor = Some(parent);
        while let Some(candidate) = ancestor {
            if candidate == slot {
                return Err(CompositionError::InvalidSlotParent {
                    slot: id.clone(),
                    parent,
                });
            }
            ancestor = cx.world().node(candidate).and_then(|node| node.parent);
        }
        if cx.world().node(slot).and_then(|node| node.parent) == Some(parent) {
            return Ok(slot);
        }
        let mut queue = MutationQueue::new();
        queue.insert(parent, slot, None);
        cx.commit_mutations(queue).map_err(runtime_error)?;
        Ok(slot)
    }

    pub fn node(&self, id: &CompositionId) -> Option<StableNodeId> {
        self.entities.get(id).copied()
    }

    /// Resolve a typed binding and check its actual registered component type.
    pub fn entity<C: ComponentView>(
        &self,
        cx: &AppContext,
        id: &CompositionId,
    ) -> Result<Entity<C>, CompositionError> {
        let node = self
            .node(id)
            .ok_or_else(|| CompositionError::MissingRenderer(id.clone()))?;
        let entity = Entity::<C>::from_stable_id(node);
        cx.read(entity, |_| ()).map_err(runtime_error)?;
        Ok(entity)
    }

    pub fn unmount(&mut self, cx: &mut AppContext) -> Result<(), CompositionError> {
        if let Some(root) = self.root {
            cx.unregister_composition_nodes(root, self.entities.values().copied());
            let mut queue = MutationQueue::new();
            queue.despawn_subtree(root);
            cx.commit_mutations(queue).map_err(runtime_error)?;
        }
        self.root = None;
        self.index = None;
        self.entities.clear();
        Ok(())
    }

    pub fn index(&self) -> Option<&CompositionIndex> {
        self.index.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(children: Vec<CompositionNode>) -> CompositionSpec {
        CompositionSpec::new(CompositionNode::with_children(
            "page",
            CompositionNodeKind::Page,
            children,
        ))
    }

    #[test]
    fn validates_stable_tree_and_host_lifecycle() {
        let spec = page(vec![CompositionNode::with_children(
            "pane",
            CompositionNodeKind::Pane,
            [CompositionNode::leaf("group", CompositionNodeKind::Group)],
        )]);
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let mut registry = CompositionRegistry::default();
        for id in ["page", "pane", "group"] {
            registry.register(id, crate::Stack::column(0.0)).unwrap();
        }
        let mut host = CompositionHost::default();
        let root = host
            .mount(&mut cx, document, parent.stable_id(), &spec, &registry)
            .unwrap();
        assert_eq!(host.index().unwrap().len(), 3);
        let group = host.node(&CompositionId::from("group")).unwrap();
        let pane = host.node(&CompositionId::from("pane")).unwrap();
        assert_eq!(cx.world().node(pane).unwrap().children, vec![group]);
        assert!(cx.world().contains(root));
        host.unmount(&mut cx).unwrap();
        assert!(!cx.world().contains(group));
        assert!(!cx.world().contains(root));
        assert!(cx.world().contains(parent.stable_id()));
    }

    #[test]
    fn rejects_duplicate_ids_and_invalid_children() {
        let duplicate = page(vec![
            CompositionNode::leaf("same", CompositionNodeKind::Group),
            CompositionNode::leaf("same", CompositionNodeKind::Group),
        ]);
        assert!(matches!(
            duplicate.validate(),
            Err(CompositionError::DuplicateId(_))
        ));
        let invalid = page(vec![CompositionNode::leaf(
            "bad",
            CompositionNodeKind::Option,
        )]);
        assert!(matches!(
            invalid.validate(),
            Err(CompositionError::InvalidChild { .. })
        ));
    }

    #[test]
    fn mount_rejects_missing_renderer_before_creating_runtime_nodes() {
        let spec = page(vec![CompositionNode::leaf(
            "missing",
            CompositionNodeKind::Group,
        )]);
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let mut registry = CompositionRegistry::default();
        registry
            .register("page", crate::Stack::column(0.0))
            .unwrap();
        let mut host = CompositionHost::default();
        assert!(matches!(
            host.mount(&mut cx, document, parent.stable_id(), &spec, &registry),
            Err(CompositionError::MissingRenderer(id)) if id.as_str() == "missing"
        ));
        assert!(host.index().is_none());
        assert_eq!(
            cx.world().node(parent.stable_id()).unwrap().children.len(),
            0
        );
    }

    #[test]
    fn reordering_declarations_keeps_the_same_identity_set() {
        let first = page(vec![CompositionNode::with_children(
            "group",
            CompositionNodeKind::Group,
            [
                CompositionNode::leaf("camera", CompositionNodeKind::Option),
                CompositionNode::leaf("microphone", CompositionNodeKind::Option),
            ],
        )]);
        let reordered = page(vec![CompositionNode::with_children(
            "group",
            CompositionNodeKind::Group,
            [
                CompositionNode::leaf("microphone", CompositionNodeKind::Option),
                CompositionNode::leaf("camera", CompositionNodeKind::Option),
            ],
        )]);
        let first = first.validate().unwrap();
        let reordered = reordered.validate().unwrap();
        for id in ["page", "group", "camera", "microphone"] {
            let id = CompositionId::from(id);
            assert_eq!(first.get(&id), reordered.get(&id));
        }
    }

    #[test]
    fn hiding_a_group_through_its_own_style_removes_it_from_layout() {
        let spec = page(vec![CompositionNode::with_children(
            "pane",
            CompositionNodeKind::Pane,
            [
                CompositionNode::leaf("first", CompositionNodeKind::Group),
                CompositionNode::leaf("second", CompositionNodeKind::Group),
            ],
        )]);
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let mut registry = CompositionRegistry::default();
        for id in ["page", "pane"] {
            registry.register(id, crate::Stack::column(0.0)).unwrap();
        }
        for id in ["first", "second"] {
            registry
                .register(
                    id,
                    crate::Stack::column(0.0)
                        .with_layout(|layout| layout.height = Some(crate::LengthSpec::Px(40.0))),
                )
                .unwrap();
        }
        let mut host = CompositionHost::default();
        host.mount(&mut cx, document, parent.stable_id(), &spec, &registry)
            .unwrap();
        let viewport = crate::LayoutViewport::new(320.0, 240.0);
        let first_id = CompositionId::from("first");
        let second = host.node(&CompositionId::from("second")).unwrap();
        cx.layout_document(document, viewport).unwrap();
        let first_top = cx
            .world()
            .layout_box(host.node(&first_id).unwrap())
            .unwrap()
            .y;
        assert_eq!(cx.world().layout_box(second).unwrap().y, first_top + 40.0);

        let first = host.entity::<crate::Stack>(&cx, &first_id).unwrap();
        cx.update_component(first, |stack, _| {
            *stack = stack.clone().with_layout(|layout| layout.hidden = true);
        })
        .unwrap();
        cx.layout_document(document, viewport).unwrap();
        assert!(
            cx.world()
                .node_style(first.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert_eq!(cx.world().layout_box(second).unwrap().y, first_top);
        host.unmount(&mut cx).unwrap();
    }

    #[test]
    fn declared_slot_can_bind_only_to_a_live_parent() {
        let spec = page(vec![CompositionNode::with_children(
            "pane",
            CompositionNodeKind::Pane,
            [CompositionNode::with_children(
                "content-slot",
                CompositionNodeKind::Slot,
                [CompositionNode::leaf("group", CompositionNodeKind::Group)],
            )],
        )]);
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let body = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let mut registry = CompositionRegistry::default();
        for id in ["page", "pane", "content-slot", "group"] {
            registry.register(id, crate::Stack::column(0.0)).unwrap();
        }
        let mut host = CompositionHost::default();
        host.mount(&mut cx, document, root.stable_id(), &spec, &registry)
            .unwrap();
        let slot = CompositionId::from("content-slot");
        let group = host.node(&CompositionId::from("group")).unwrap();
        host.bind_slot(&mut cx, &slot, body.stable_id()).unwrap();
        assert_eq!(
            cx.world().node(host.node(&slot).unwrap()).unwrap().parent,
            Some(body.stable_id())
        );
        assert_eq!(
            cx.world().node(body.stable_id()).unwrap().children,
            [host.node(&slot).unwrap()]
        );
        assert_eq!(
            cx.world().node(host.node(&slot).unwrap()).unwrap().children,
            [group]
        );
        assert!(matches!(
            host.bind_slot(&mut cx, &CompositionId::from("group"), root.stable_id()),
            Err(CompositionError::NotSlot(id)) if id.as_str() == "group"
        ));
        assert!(matches!(
            host.bind_slot(&mut cx, &slot, StableNodeId::new(999_999).unwrap()),
            Err(CompositionError::MissingParent(_))
        ));
        assert!(matches!(
            host.bind_slot(&mut cx, &slot, host.node(&slot).unwrap()),
            Err(CompositionError::InvalidSlotParent { .. })
        ));
        let root_two = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let mut host_two = CompositionHost::default();
        host_two
            .mount(&mut cx, document, root_two.stable_id(), &spec, &registry)
            .unwrap();
        let foreign_parent = host_two.node(&CompositionId::from("page")).unwrap();
        assert!(matches!(
            host.bind_slot(&mut cx, &slot, foreign_parent),
            Err(CompositionError::CrossHostParent { .. })
        ));
        host_two.unmount(&mut cx).unwrap();
        let body_two = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        host.bind_slot(&mut cx, &slot, body_two.stable_id())
            .unwrap();
        let mut destroy = MutationQueue::new();
        destroy.despawn_subtree(body_two.stable_id());
        cx.commit_mutations(destroy).unwrap();
        assert!(matches!(
            host.bind_slot(&mut cx, &slot, body_two.stable_id()),
            Err(CompositionError::MissingParent(_)) | Err(CompositionError::MissingRenderer(_))
        ));
        host.unmount(&mut cx).unwrap();
        assert!(host.node(&slot).is_none());
    }

    #[test]
    fn mounts_a_large_declared_tree_with_measured_host_work() {
        let leaves: Vec<_> = (0..128)
            .map(|index| {
                CompositionNode::leaf(format!("option-{index}"), CompositionNodeKind::Option)
            })
            .collect();
        let spec = CompositionSpec::new(CompositionNode::with_children(
            "page",
            CompositionNodeKind::Page,
            [CompositionNode::with_children(
                "group",
                CompositionNodeKind::Group,
                leaves,
            )],
        ));
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = cx
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let mut registry = CompositionRegistry::default();
        registry
            .register("page", crate::Stack::column(0.0))
            .unwrap();
        registry
            .register("group", crate::Stack::column(0.0))
            .unwrap();
        for index in 0..128 {
            registry
                .register(format!("option-{index}"), crate::Stack::column(0.0))
                .unwrap();
        }
        let start = std::time::Instant::now();
        let mut host = CompositionHost::default();
        host.mount(&mut cx, document, parent.stable_id(), &spec, &registry)
            .unwrap();
        let elapsed = start.elapsed();
        eprintln!("composition_mount_130_nodes_us={}", elapsed.as_micros());
        assert_eq!(host.index().unwrap().len(), 130);
        host.unmount(&mut cx).unwrap();
    }
}
