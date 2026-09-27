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
        CompositionNodeKind::Option
        | CompositionNodeKind::Slot
        | CompositionNodeKind::Extension => false,
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
        self.index = Some(index);
        Ok(root)
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
