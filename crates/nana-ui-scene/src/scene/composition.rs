//! Scene composition projection.

use super::*;

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
        let mut custom_nodes: BTreeMap<Arc<str>, (PrimitiveId, CustomRenderNode)> = BTreeMap::new();
        for primitive in self.primitives() {
            let ScenePrimitiveKind::Custom { node: custom, .. } = &primitive.kind else {
                continue;
            };
            if let Some((_, previous)) = custom_nodes.get(&custom.resource)
                && (previous.revision != custom.revision || previous.renderer != custom.renderer)
            {
                return Err(GraphError::ConflictingExternalResource(
                    custom.resource.to_string(),
                ));
            }
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
