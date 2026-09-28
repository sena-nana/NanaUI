//! Declarative composition over assembly keys.
//!
//! A [`CompositionSpec`] declares a retained subtree as keyed nodes; mounting
//! it is one [`AppContext::build_detached`] batch, so node identity is the
//! framework's single stable-identity contract, the assembly key
//! ([`AppContext::assembly_path`], [`AppContext::resolve_assembly_path`]).
//! The host keeps no identity table of its own: it holds its root and the
//! nodes its declaration allows to move. Structural roles are the
//! application's: `K` is whatever kind it tags nodes with, and the rule it
//! passes to [`CompositionSpec::validate`] decides which children a parent may
//! have. This module contains no product or widget types.

use std::collections::{HashMap, HashSet};

use crate::{
    AppContext, ComponentView, DocumentId, Entity, FrameworkError, MutationQueue, StableNodeId,
    UiBuilder,
};

/// Separates assembly keys in a declared path.
pub const COMPOSITION_PATH_SEPARATOR: char = '/';

/// One node of a declaration: its assembly key, the application's kind for
/// it, and whether [`CompositionHost::bind_slot`] may place it elsewhere.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionNode<K = ()> {
    pub key: String,
    pub kind: K,
    pub rebindable: bool,
    pub children: Vec<CompositionNode<K>>,
}

impl<K> CompositionNode<K> {
    pub fn leaf(key: impl Into<String>, kind: K) -> Self {
        Self {
            key: key.into(),
            kind,
            rebindable: false,
            children: Vec::new(),
        }
    }

    pub fn with_children(
        key: impl Into<String>,
        kind: K,
        children: impl IntoIterator<Item = CompositionNode<K>>,
    ) -> Self {
        Self {
            key: key.into(),
            kind,
            rebindable: false,
            children: children.into_iter().collect(),
        }
    }

    /// Let [`CompositionHost::bind_slot`] place this node (and its subtree)
    /// under an application-owned parent.
    pub fn rebindable(mut self) -> Self {
        self.rebindable = true;
        self
    }
}

/// A complete declaration. Paths name nodes by their keys from the root:
/// `"page"`, `"page/content"`, `"page/content/list"`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionSpec<K = ()> {
    pub root: CompositionNode<K>,
}

impl<K> CompositionSpec<K> {
    pub fn new(root: CompositionNode<K>) -> Self {
        Self { root }
    }

    /// Every node with its path, parents before children and siblings in
    /// declaration order.
    pub fn nodes(&self) -> Vec<(String, &CompositionNode<K>)> {
        let mut nodes = Vec::new();
        let mut stack = vec![(self.root.key.clone(), &self.root)];
        while let Some((path, node)) = stack.pop() {
            for child in node.children.iter().rev() {
                stack.push((child_path(&path, &child.key), child));
            }
            nodes.push((path, node));
        }
        nodes
    }

    /// Checks keys and structure before anything is created: every key is
    /// non-empty and free of the path separator, siblings have distinct keys,
    /// and `rule` accepts every parent/child pair (its `Err` is the reason).
    pub fn validate(
        &self,
        rule: impl Fn(&CompositionNode<K>, &CompositionNode<K>) -> Result<(), String>,
    ) -> Result<(), CompositionError> {
        check_key(&self.root.key, &self.root.key)?;
        let mut stack = vec![(self.root.key.clone(), &self.root)];
        while let Some((path, node)) = stack.pop() {
            let mut siblings = HashSet::with_capacity(node.children.len());
            for child in &node.children {
                let path_of_child = child_path(&path, &child.key);
                check_key(&child.key, &path_of_child)?;
                if !siblings.insert(child.key.as_str()) {
                    return Err(CompositionError::DuplicateKey(path_of_child));
                }
                rule(node, child).map_err(|reason| CompositionError::InvalidChild {
                    parent: path.clone(),
                    child: path_of_child.clone(),
                    reason,
                })?;
                stack.push((path_of_child, child));
            }
        }
        Ok(())
    }
}

fn child_path(parent: &str, key: &str) -> String {
    let mut path = String::with_capacity(parent.len() + 1 + key.len());
    path.push_str(parent);
    path.push(COMPOSITION_PATH_SEPARATOR);
    path.push_str(key);
    path
}

fn check_key(key: &str, path: &str) -> Result<(), CompositionError> {
    if key.trim().is_empty() || key.contains(COMPOSITION_PATH_SEPARATOR) {
        Err(CompositionError::InvalidKey(path.to_owned()))
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositionError {
    /// Empty, blank, or containing [`COMPOSITION_PATH_SEPARATOR`].
    InvalidKey(String),
    /// Two siblings share a key; the path is the second one.
    DuplicateKey(String),
    InvalidChild {
        parent: String,
        child: String,
        reason: String,
    },
    MissingRenderer(String),
    DuplicateRenderer(String),
    AlreadyMounted,
    NotMounted,
    /// No declared node at this path.
    UnknownPath(String),
    /// The node was not declared [`CompositionNode::rebindable`].
    NotRebindable(String),
    MissingParent(StableNodeId),
    /// The parent belongs to another composition.
    CrossHostParent {
        parent: StableNodeId,
        owner: StableNodeId,
    },
    Runtime(String),
}

fn runtime_error(error: FrameworkError) -> CompositionError {
    CompositionError::Runtime(error.to_string())
}

/// Creates the node at one path: builds it keyed under the current builder
/// parent, then runs `children` nested inside it. Returns its id.
type ComponentFactory =
    Box<dyn Fn(&mut UiBuilder<'_>, String, &mut dyn FnMut(&mut UiBuilder<'_>)) -> StableNodeId>;

/// The component each declared path mounts. Creation goes through the
/// Runtime component registry like any other build.
#[derive(Default)]
pub struct CompositionRegistry {
    factories: HashMap<String, ComponentFactory>,
}

impl CompositionRegistry {
    pub fn register<C: ComponentView + Clone>(
        &mut self,
        path: impl Into<String>,
        component: C,
    ) -> Result<(), CompositionError> {
        let path = path.into();
        if path
            .split(COMPOSITION_PATH_SEPARATOR)
            .any(|key| key.trim().is_empty())
        {
            return Err(CompositionError::InvalidKey(path));
        }
        if self.factories.contains_key(&path) {
            return Err(CompositionError::DuplicateRenderer(path));
        }
        self.factories.insert(
            path,
            Box::new(move |builder, key, children| {
                let entity = builder.child(key, component.clone());
                builder.nest(entity, |builder| children(builder));
                entity.stable_id()
            }),
        );
        Ok(())
    }

    pub fn contains(&self, path: &str) -> bool {
        self.factories.contains_key(path)
    }
}

/// Mounts one declaration and answers for it until unmounted.
///
/// The mounted root is unkeyed under the application's parent, so the
/// parent's own keyed children are not disturbed; every node below it is
/// keyed by its declaration. Unmount explicitly with the same `AppContext`;
/// nodes [`Self::bind_slot`] placed elsewhere go with the root.
#[derive(Debug, Default)]
pub struct CompositionHost {
    root: Option<StableNodeId>,
    root_key: String,
    /// Nodes the declaration marked rebindable.
    rebindable: HashSet<StableNodeId>,
}

impl CompositionHost {
    /// Validate `spec` with `rule`, check every path has a renderer, then
    /// build the whole tree in one batch and insert it under `parent`. On any
    /// failure the world is left as it was.
    pub fn mount<K>(
        &mut self,
        cx: &mut AppContext,
        parent: StableNodeId,
        spec: &CompositionSpec<K>,
        registry: &CompositionRegistry,
        rule: impl Fn(&CompositionNode<K>, &CompositionNode<K>) -> Result<(), String>,
    ) -> Result<StableNodeId, CompositionError> {
        if self.root.is_some() {
            return Err(CompositionError::AlreadyMounted);
        }
        spec.validate(rule)?;
        let nodes = spec.nodes();
        if let Some((path, _)) = nodes.iter().find(|(path, _)| !registry.contains(path)) {
            return Err(CompositionError::MissingRenderer(path.clone()));
        }
        let document = cx
            .world()
            .node(parent)
            .ok_or(CompositionError::MissingParent(parent))?
            .document;
        if let Some(owner) = cx.assembly_owner(parent) {
            return Err(CompositionError::CrossHostParent { parent, owner });
        }
        let root = build_tree(cx, document, spec, registry).map_err(runtime_error)?;
        let mut insert = MutationQueue::new();
        insert.insert(parent, root, None);
        if let Err(error) = cx.commit_mutations(insert) {
            let mut cleanup = MutationQueue::new();
            cleanup.despawn_subtree(root);
            cx.commit_mutations(cleanup).map_err(runtime_error)?;
            return Err(runtime_error(error));
        }
        cx.register_composition_root(root);
        self.rebindable = nodes
            .iter()
            .filter(|(_, node)| node.rebindable)
            .filter_map(|(path, _)| self.resolve_in(cx, root, &spec.root.key, path))
            .collect();
        self.root = Some(root);
        self.root_key = spec.root.key.clone();
        Ok(root)
    }

    pub fn root(&self) -> Option<StableNodeId> {
        self.root
    }

    /// The node declared at `path`, wherever it is placed.
    pub fn node(&self, cx: &AppContext, path: &str) -> Option<StableNodeId> {
        self.resolve_in(cx, self.root?, &self.root_key, path)
    }

    /// Resolve a typed binding, checking the node's actual component type.
    pub fn entity<C: ComponentView>(
        &self,
        cx: &AppContext,
        path: &str,
    ) -> Result<Entity<C>, CompositionError> {
        let node = self
            .node(cx, path)
            .ok_or_else(|| CompositionError::UnknownPath(path.to_owned()))?;
        let entity = Entity::<C>::from_stable_id(node);
        cx.read(entity, |_| ()).map_err(runtime_error)?;
        Ok(entity)
    }

    /// Place a rebindable node and its subtree under an application-owned
    /// `parent`, keeping its declared identity
    /// ([`AppContext::place_assembled`]). A parent inside another
    /// composition, a dead parent, and a parent inside the node itself are
    /// rejected before anything moves.
    pub fn bind_slot(
        &mut self,
        cx: &mut AppContext,
        path: &str,
        parent: StableNodeId,
    ) -> Result<StableNodeId, CompositionError> {
        let root = self.root.ok_or(CompositionError::NotMounted)?;
        let node = self
            .node(cx, path)
            .ok_or_else(|| CompositionError::UnknownPath(path.to_owned()))?;
        if !self.rebindable.contains(&node) {
            return Err(CompositionError::NotRebindable(path.to_owned()));
        }
        if !cx.world().contains(parent) {
            return Err(CompositionError::MissingParent(parent));
        }
        if let Some(owner) = cx.assembly_owner(parent)
            && owner != root
        {
            return Err(CompositionError::CrossHostParent { parent, owner });
        }
        cx.place_assembled(node, parent).map_err(runtime_error)?;
        Ok(node)
    }

    /// Despawn the mounted tree, including nodes placed elsewhere. The host is
    /// empty afterwards even if the despawn fails, and can mount again.
    pub fn unmount(&mut self, cx: &mut AppContext) -> Result<(), CompositionError> {
        let Some(root) = self.root.take() else {
            return Ok(());
        };
        self.root_key.clear();
        self.rebindable.clear();
        cx.unregister_composition_root(root);
        if !cx.world().contains(root) {
            return Ok(());
        }
        let mut queue = MutationQueue::new();
        queue.despawn_subtree(root);
        cx.commit_mutations(queue).map_err(runtime_error)?;
        Ok(())
    }

    fn resolve_in(
        &self,
        cx: &AppContext,
        root: StableNodeId,
        root_key: &str,
        path: &str,
    ) -> Option<StableNodeId> {
        let rest = path.strip_prefix(root_key)?;
        if rest.is_empty() {
            return Some(root);
        }
        let rest = rest.strip_prefix(COMPOSITION_PATH_SEPARATOR)?;
        cx.resolve_assembly_path(root, rest)
    }
}

/// One detached build of the whole declaration; its root is parked until the
/// caller inserts it.
fn build_tree<K>(
    cx: &mut AppContext,
    document: DocumentId,
    spec: &CompositionSpec<K>,
    registry: &CompositionRegistry,
) -> Result<StableNodeId, FrameworkError> {
    fn build_node<K>(
        builder: &mut UiBuilder<'_>,
        node: &CompositionNode<K>,
        path: &str,
        registry: &CompositionRegistry,
    ) -> StableNodeId {
        let factory = &registry.factories[path];
        factory(builder, node.key.clone(), &mut |builder| {
            for child in &node.children {
                build_node(builder, child, &child_path(path, &child.key), registry);
            }
        })
    }
    cx.build_detached(document, |builder| {
        build_node(builder, &spec.root, &spec.root.key, registry)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LayoutViewport, LengthSpec, Stack};

    fn any(_: &CompositionNode, _: &CompositionNode) -> Result<(), String> {
        Ok(())
    }

    fn stack() -> Stack {
        Stack::column(0.0)
    }

    fn registry_for<K>(spec: &CompositionSpec<K>) -> CompositionRegistry {
        let mut registry = CompositionRegistry::default();
        for (path, _) in spec.nodes() {
            registry.register(path, stack()).unwrap();
        }
        registry
    }

    fn setup() -> (AppContext, DocumentId, StableNodeId) {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let parent = cx.create_component(document, stack()).unwrap();
        (cx, document, parent.stable_id())
    }

    fn page(children: Vec<CompositionNode>) -> CompositionSpec {
        CompositionSpec::new(CompositionNode::with_children("page", (), children))
    }

    fn slotted() -> CompositionSpec {
        page(vec![CompositionNode::with_children(
            "pane",
            (),
            [
                CompositionNode::with_children("slot", (), [CompositionNode::leaf("group", ())])
                    .rebindable(),
            ],
        )])
    }

    #[test]
    fn mounts_keyed_nodes_that_resolve_by_path_and_assembly_path() {
        let spec = page(vec![CompositionNode::with_children(
            "pane",
            (),
            [CompositionNode::leaf("group", ())],
        )]);
        let (mut cx, _, parent) = setup();
        let mut host = CompositionHost::default();
        let root = host
            .mount(&mut cx, parent, &spec, &registry_for(&spec), any)
            .unwrap();
        let pane = host.node(&cx, "page/pane").unwrap();
        let group = host.node(&cx, "page/pane/group").unwrap();
        assert_eq!(host.node(&cx, "page"), Some(root));
        assert_eq!(cx.world().node(root).unwrap().parent, Some(parent));
        assert_eq!(cx.world().node(pane).unwrap().children, vec![group]);
        assert_eq!(cx.assembly_path(group).as_deref(), Some("pane/group"));
        assert_eq!(cx.assembly_key(pane, group), Some("group"));
        assert!(host.entity::<Stack>(&cx, "page/pane/group").is_ok());
        assert!(host.node(&cx, "page/missing").is_none());
        assert!(host.node(&cx, "other/pane").is_none());
        host.unmount(&mut cx).unwrap();
        assert!(!cx.world().contains(group));
        assert!(!cx.world().contains(root));
        assert!(cx.world().contains(parent));
    }

    #[test]
    fn validation_rejects_keys_duplicates_and_rule_violations_with_paths() {
        let duplicate = page(vec![
            CompositionNode::leaf("same", ()),
            CompositionNode::leaf("same", ()),
        ]);
        assert_eq!(
            duplicate.validate(any),
            Err(CompositionError::DuplicateKey("page/same".into()))
        );
        let separator = page(vec![CompositionNode::leaf("a/b", ())]);
        assert_eq!(
            separator.validate(any),
            Err(CompositionError::InvalidKey("page/a/b".into()))
        );
        let blank = page(vec![CompositionNode::leaf(" ", ())]);
        assert!(matches!(
            blank.validate(any),
            Err(CompositionError::InvalidKey(_))
        ));
        // Same key under different parents is fine: keys are per parent.
        let cousins = page(vec![
            CompositionNode::with_children("a", (), [CompositionNode::leaf("x", ())]),
            CompositionNode::with_children("b", (), [CompositionNode::leaf("x", ())]),
        ]);
        assert_eq!(cousins.validate(any), Ok(()));
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Kind {
            Page,
            Option,
        }
        let typed = CompositionSpec::new(CompositionNode::with_children(
            "page",
            Kind::Page,
            [CompositionNode::with_children(
                "option",
                Kind::Option,
                [CompositionNode::leaf("nested", Kind::Option)],
            )],
        ));
        let rule = |parent: &CompositionNode<Kind>, _: &CompositionNode<Kind>| {
            if parent.kind == Kind::Option {
                Err("options are leaves".to_owned())
            } else {
                Ok(())
            }
        };
        assert_eq!(
            typed.validate(rule),
            Err(CompositionError::InvalidChild {
                parent: "page/option".into(),
                child: "page/option/nested".into(),
                reason: "options are leaves".into(),
            })
        );
    }

    #[test]
    fn a_missing_renderer_leaves_the_world_untouched() {
        let spec = page(vec![CompositionNode::leaf("missing", ())]);
        let (mut cx, _, parent) = setup();
        let mut registry = CompositionRegistry::default();
        registry.register("page", stack()).unwrap();
        let before = cx.world().len();
        let mut host = CompositionHost::default();
        assert_eq!(
            host.mount(&mut cx, parent, &spec, &registry, any),
            Err(CompositionError::MissingRenderer("page/missing".into()))
        );
        assert_eq!(cx.world().len(), before);
        assert!(host.root().is_none());
        let spec = page(Vec::new());
        host.mount(&mut cx, parent, &spec, &registry_for(&spec), any)
            .unwrap();
    }

    #[test]
    fn nodes_are_created_in_declaration_order() {
        let spec = page(
            (0..16)
                .map(|index| CompositionNode::leaf(format!("option-{index}"), ()))
                .collect(),
        );
        let ids = |spec: &CompositionSpec| {
            let (mut cx, _, parent) = setup();
            let mut host = CompositionHost::default();
            host.mount(&mut cx, parent, spec, &registry_for(spec), any)
                .unwrap();
            (0..16)
                .map(|index| {
                    host.node(&cx, &format!("page/option-{index}"))
                        .unwrap()
                        .get()
                })
                .collect::<Vec<_>>()
        };
        let first = ids(&spec);
        assert!(first.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(first, ids(&spec));
    }

    #[test]
    fn a_bound_slot_keeps_its_identity_and_leaves_with_the_host() {
        let spec = slotted();
        let (mut cx, document, parent) = setup();
        let body = cx.create_component(document, stack()).unwrap().stable_id();
        let mut host = CompositionHost::default();
        host.mount(&mut cx, parent, &spec, &registry_for(&spec), any)
            .unwrap();
        let slot = host.bind_slot(&mut cx, "page/pane/slot", body).unwrap();
        let group = host.node(&cx, "page/pane/slot/group").unwrap();
        assert_eq!(cx.world().node(slot).unwrap().parent, Some(body));
        assert_eq!(cx.world().node(body).unwrap().children, vec![slot]);
        assert_eq!(cx.world().node(slot).unwrap().children, vec![group]);
        // Identity is the declaration, not the placement.
        assert_eq!(host.node(&cx, "page/pane/slot"), Some(slot));
        assert_eq!(cx.assembly_path(group).as_deref(), Some("pane/slot/group"));
        host.unmount(&mut cx).unwrap();
        assert!(!cx.world().contains(slot));
        assert!(!cx.world().contains(group));
        assert!(cx.world().contains(body));
        assert!(cx.world().node(body).unwrap().children.is_empty());
    }

    #[test]
    fn bind_slot_rejects_undeclared_foreign_dead_and_cyclic_parents() {
        let spec = slotted();
        let (mut cx, document, parent) = setup();
        let mut host = CompositionHost::default();
        host.mount(&mut cx, parent, &spec, &registry_for(&spec), any)
            .unwrap();
        assert_eq!(
            host.bind_slot(&mut cx, "page/pane", parent),
            Err(CompositionError::NotRebindable("page/pane".into()))
        );
        assert_eq!(
            host.bind_slot(&mut cx, "page/nope", parent),
            Err(CompositionError::UnknownPath("page/nope".into()))
        );
        let dead = StableNodeId::new(999_999).unwrap();
        assert_eq!(
            host.bind_slot(&mut cx, "page/pane/slot", dead),
            Err(CompositionError::MissingParent(dead))
        );
        let group = host.node(&cx, "page/pane/slot/group").unwrap();
        assert!(matches!(
            host.bind_slot(&mut cx, "page/pane/slot", group),
            Err(CompositionError::Runtime(_))
        ));
        // Another host's nodes, including unkeyed ones created below them,
        // belong to that host.
        let other_parent = cx.create_component(document, stack()).unwrap().stable_id();
        let mut other = CompositionHost::default();
        let other_root = other
            .mount(&mut cx, other_parent, &spec, &registry_for(&spec), any)
            .unwrap();
        let foreign = other.node(&cx, "page/pane/slot/group").unwrap();
        let projected = cx.create_component(document, stack()).unwrap().stable_id();
        let mut attach = MutationQueue::new();
        attach.insert(foreign, projected, None);
        cx.commit_mutations(attach).unwrap();
        for target in [foreign, projected] {
            assert_eq!(
                host.bind_slot(&mut cx, "page/pane/slot", target),
                Err(CompositionError::CrossHostParent {
                    parent: target,
                    owner: other_root,
                })
            );
        }
        // Nothing moved.
        let slot = host.node(&cx, "page/pane/slot").unwrap();
        let pane = host.node(&cx, "page/pane").unwrap();
        assert_eq!(cx.world().node(slot).unwrap().parent, Some(pane));
        // Mounting inside another host is rejected too.
        let mut third = CompositionHost::default();
        assert!(matches!(
            third.mount(&mut cx, foreign, &spec, &registry_for(&spec), any),
            Err(CompositionError::CrossHostParent { .. })
        ));
    }

    #[test]
    fn a_despawned_declared_parent_takes_its_placed_child() {
        let spec = slotted();
        let (mut cx, document, parent) = setup();
        let body = cx.create_component(document, stack()).unwrap().stable_id();
        let mut host = CompositionHost::default();
        let root = host
            .mount(&mut cx, parent, &spec, &registry_for(&spec), any)
            .unwrap();
        let slot = host.bind_slot(&mut cx, "page/pane/slot", body).unwrap();
        // The application tears down the page parent without unmounting.
        let mut teardown = MutationQueue::new();
        teardown.despawn_subtree(parent);
        cx.commit_mutations(teardown).unwrap();
        assert!(!cx.world().contains(root));
        assert!(!cx.world().contains(slot));
        assert!(cx.world().contains(body));
        // The host is empty afterwards and can mount again.
        host.unmount(&mut cx).unwrap();
        let parent = cx.create_component(document, stack()).unwrap().stable_id();
        host.mount(&mut cx, parent, &spec, &registry_for(&spec), any)
            .unwrap();
    }

    #[test]
    fn placing_a_slot_back_under_its_declared_parent_ends_the_placement() {
        let spec = slotted();
        let (mut cx, document, parent) = setup();
        let body = cx.create_component(document, stack()).unwrap().stable_id();
        let mut host = CompositionHost::default();
        host.mount(&mut cx, parent, &spec, &registry_for(&spec), any)
            .unwrap();
        let slot = host.bind_slot(&mut cx, "page/pane/slot", body).unwrap();
        let pane = host.node(&cx, "page/pane").unwrap();
        host.bind_slot(&mut cx, "page/pane/slot", pane).unwrap();
        assert_eq!(cx.world().node(slot).unwrap().parent, Some(pane));
        // Despawning the former placement parent no longer touches the slot.
        let mut teardown = MutationQueue::new();
        teardown.despawn_subtree(body);
        cx.commit_mutations(teardown).unwrap();
        assert!(cx.world().contains(slot));
    }

    #[test]
    fn hiding_a_declared_node_through_its_own_style_removes_it_from_layout() {
        let spec = page(vec![CompositionNode::with_children(
            "pane",
            (),
            [
                CompositionNode::leaf("first", ()),
                CompositionNode::leaf("second", ()),
            ],
        )]);
        let (mut cx, document, parent) = setup();
        let mut registry = CompositionRegistry::default();
        registry.register("page", stack()).unwrap();
        registry.register("page/pane", stack()).unwrap();
        for path in ["page/pane/first", "page/pane/second"] {
            registry
                .register(
                    path,
                    stack().with_layout(|layout| layout.height = Some(LengthSpec::Px(40.0))),
                )
                .unwrap();
        }
        let mut host = CompositionHost::default();
        host.mount(&mut cx, parent, &spec, &registry, any).unwrap();
        let viewport = LayoutViewport::new(320.0, 240.0);
        let first = host.entity::<Stack>(&cx, "page/pane/first").unwrap();
        let second = host.node(&cx, "page/pane/second").unwrap();
        cx.layout_document(document, viewport).unwrap();
        let first_top = cx.world().layout_box(first.stable_id()).unwrap().y;
        assert_eq!(cx.world().layout_box(second).unwrap().y, first_top + 40.0);
        cx.update_component(first, |stack, _| {
            *stack = stack.clone().with_layout(|layout| layout.hidden = true);
        })
        .unwrap();
        cx.layout_document(document, viewport).unwrap();
        assert_eq!(cx.world().layout_box(second).unwrap().y, first_top);
    }

    #[test]
    fn unmount_despawn_forgets_views_and_assembly_records() {
        let spec = slotted();
        let (mut cx, _, parent) = setup();
        let mut host = CompositionHost::default();
        host.mount(&mut cx, parent, &spec, &registry_for(&spec), any)
            .unwrap();
        let group = host.node(&cx, "page/pane/slot/group").unwrap();
        let pane = host.node(&cx, "page/pane").unwrap();
        host.unmount(&mut cx).unwrap();
        assert!(cx.view_entity::<Stack>(group).is_none());
        assert!(cx.assembly_path(group).is_none());
        assert!(cx.assembled_child(pane, "slot").is_none());
        assert!(cx.assembly_owner(parent).is_none());
    }
}
