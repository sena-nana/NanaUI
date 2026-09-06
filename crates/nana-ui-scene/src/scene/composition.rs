//! Scene composition projection.

use super::*;

/// Structural frame program. Resource contents and bindings are deliberately
/// resolved at consumption time, so a texture update does not compile a graph.
#[derive(Debug)]
pub struct FramePlan {
    pub operations: Arc<[RenderOperation]>,
    pub preparations: Arc<[PrimitiveId]>,
    pub custom_nodes: Arc<[PrimitiveId]>,
}

impl UiScene {
    pub fn frame_plan(&self) -> Result<Arc<FramePlan>, GraphError> {
        if let Some(plan) = self.frame_plan.get() {
            self.validate_plan_resources(plan)?;
            return Ok(Arc::clone(plan));
        }
        let graph = self.frame_graph(ResourceId(1))?;
        let mut preparations = Vec::new();
        let mut custom_nodes = Vec::new();
        let mut operations = Vec::new();
        for operation in graph.passes.into_iter().flat_map(|pass| pass.operations) {
            match operation {
                RenderOperation::PrepareExternal(id) => {
                    preparations.push(id);
                    continue;
                }
                RenderOperation::InvokeCustom(id) => custom_nodes.push(id),
                RenderOperation::Draw(_) => {}
            }
            operations.push(operation);
        }
        let plan = Arc::new(FramePlan {
            operations: operations.into(),
            preparations: preparations.into(),
            custom_nodes: custom_nodes.into(),
        });
        let _ = self.frame_plan.set(Arc::clone(&plan));
        Ok(plan)
    }

    /// The first host-texture slot two nodes claim with different revisions
    /// (or different renderers), if any.
    ///
    /// Such a frame is rejected before anything is drawn — `frame_graph` fails
    /// and the painter draws nothing, so a single mismatched view blanks the
    /// whole frame rather than just itself. Hosts that bind one slot from
    /// several views (the same cover as an avatar and as a plain texture view,
    /// say) can assert this stays `None` without depending on the render graph.
    pub fn conflicting_external_resource(&self) -> Option<Arc<str>> {
        let mut seen: BTreeMap<&Arc<str>, (&Arc<str>, u64)> = BTreeMap::new();
        for primitive in self.primitives() {
            let ScenePrimitiveKind::Custom { node: custom, .. } = &primitive.kind else {
                continue;
            };
            let claim = (&custom.renderer, custom.revision);
            match seen.get(&custom.resource) {
                Some(previous) if *previous != claim => {
                    return Some(Arc::clone(&custom.resource));
                }
                Some(_) => {}
                None => {
                    seen.insert(&custom.resource, claim);
                }
            }
        }
        None
    }

    fn validate_plan_resources(&self, plan: &FramePlan) -> Result<(), GraphError> {
        let mut revisions = HashMap::new();
        for id in plan.custom_nodes.iter() {
            let Some(ScenePrimitive {
                kind: ScenePrimitiveKind::Custom { node, .. },
                ..
            }) = self.primitive(*id)
            else {
                continue;
            };
            if let Some(previous) =
                revisions.insert(&node.resource, (&node.renderer, node.revision))
                && previous != (&node.renderer, node.revision)
            {
                return Err(GraphError::ConflictingExternalResource(
                    node.resource.to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl UiScene {
    /// Build the default frame pass. Custom operations remain in exact scene
    /// order and split standard draw segments, allowing a backend extension to
    /// encode a real pass between ordinary UI items. Opaque custom resources
    /// are explicit external graph inputs rather than hidden backend state.
    pub fn frame_graph(&self, target: ResourceId) -> Result<CompiledRenderGraph, GraphError> {
        let mut graph = RenderGraph::new();
        graph.add_resource(RenderResource {
            id: target,
            label: "ui-target".into(),
            external: true,
        })?;
        let mut next_resource = 1_u64;
        // One source of truth for the invariant: the public query and the graph
        // must never disagree about which frames are rejected.
        if let Some(resource) = self.conflicting_external_resource() {
            return Err(GraphError::ConflictingExternalResource(
                resource.to_string(),
            ));
        }
        let mut custom_nodes: BTreeMap<Arc<str>, (PrimitiveId, CustomRenderNode)> = BTreeMap::new();
        for primitive in self.primitives() {
            let ScenePrimitiveKind::Custom { node: custom, .. } = &primitive.kind else {
                continue;
            };
            custom_nodes
                .entry(custom.resource.clone())
                .or_insert((primitive.id, custom.clone()));
        }
        let custom_resources = custom_nodes
            .into_iter()
            .map(|(resource, (representative, _))| {
                while ResourceId(next_resource) == target {
                    next_resource += 1;
                }
                let id = ResourceId(next_resource);
                next_resource += 1;
                (resource, (id, representative))
            })
            .collect::<HashMap<_, _>>();
        for (label, (id, _)) in &custom_resources {
            graph.add_resource(RenderResource {
                id: *id,
                label: label.to_string(),
                external: true,
            })?;
        }
        let mut pass_id = 1_u64;
        let mut ordered_resources = custom_resources.iter().collect::<Vec<_>>();
        ordered_resources.sort_by_key(|(label, _)| *label);
        for (label, (resource, representative)) in ordered_resources {
            graph.add_pass(RenderPass {
                id: PassId(pass_id),
                label: format!("prepare:{label}"),
                dependencies: Vec::new(),
                resources: vec![ResourceAccess {
                    resource: *resource,
                    mode: AccessMode::Write,
                }],
                operations: vec![RenderOperation::PrepareExternal(*representative)],
            })?;
            pass_id += 1;
        }
        let mut standard = Vec::new();
        let flush_standard = |graph: &mut RenderGraph,
                              pass_id: &mut u64,
                              standard: &mut Vec<RenderOperation>|
         -> Result<(), GraphError> {
            if standard.is_empty() {
                return Ok(());
            }
            graph.add_pass(RenderPass {
                id: PassId(*pass_id),
                label: "ui-standard".into(),
                dependencies: Vec::new(),
                resources: vec![ResourceAccess {
                    resource: target,
                    mode: AccessMode::ReadWrite,
                }],
                operations: std::mem::take(standard),
            })?;
            *pass_id += 1;
            Ok(())
        };
        for primitive in self.primitives() {
            match &primitive.kind {
                ScenePrimitiveKind::Custom { node: custom, .. } => {
                    flush_standard(&mut graph, &mut pass_id, &mut standard)?;
                    let resource = custom_resources[&custom.resource].0;
                    graph.add_pass(RenderPass {
                        id: PassId(pass_id),
                        label: format!("custom:{}", custom.renderer),
                        dependencies: Vec::new(),
                        resources: vec![
                            ResourceAccess {
                                resource: target,
                                mode: AccessMode::ReadWrite,
                            },
                            ResourceAccess {
                                resource,
                                mode: AccessMode::Read,
                            },
                        ],
                        operations: vec![RenderOperation::InvokeCustom(primitive.id)],
                    })?;
                    pass_id += 1;
                }
                _ => standard.push(RenderOperation::Draw(primitive.id)),
            }
        }
        flush_standard(&mut graph, &mut pass_id, &mut standard)?;
        graph.compile()
    }
}
