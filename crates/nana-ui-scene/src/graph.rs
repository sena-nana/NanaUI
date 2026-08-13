use std::collections::{BTreeSet, HashMap};
use std::fmt;

use crate::PrimitiveId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PassId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderResource {
    pub id: ResourceId,
    pub label: String,
    /// External resources are supplied by the host and need no graph producer.
    pub external: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    Read,
    Write,
    ReadWrite,
}

impl AccessMode {
    fn writes(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceAccess {
    pub resource: ResourceId,
    pub mode: AccessMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderOperation {
    Draw(PrimitiveId),
    InvokeCustom(PrimitiveId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPass {
    pub id: PassId,
    pub label: String,
    pub dependencies: Vec<PassId>,
    pub resources: Vec<ResourceAccess>,
    pub operations: Vec<RenderOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    DuplicateResource(ResourceId),
    DuplicatePass(PassId),
    UnknownResource { pass: PassId, resource: ResourceId },
    UnknownDependency { pass: PassId, dependency: PassId },
    UninitializedRead { pass: PassId, resource: ResourceId },
    Cycle,
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateResource(id) => write!(formatter, "duplicate render resource {}", id.0),
            Self::DuplicatePass(id) => write!(formatter, "duplicate render pass {}", id.0),
            Self::UnknownResource { pass, resource } => write!(
                formatter,
                "render pass {} references unknown resource {}",
                pass.0, resource.0
            ),
            Self::UnknownDependency { pass, dependency } => write!(
                formatter,
                "render pass {} depends on unknown pass {}",
                pass.0, dependency.0
            ),
            Self::UninitializedRead { pass, resource } => write!(
                formatter,
                "render pass {} reads transient resource {} before it is written",
                pass.0, resource.0
            ),
            Self::Cycle => formatter.write_str("render graph contains a dependency cycle"),
        }
    }
}

impl std::error::Error for GraphError {}

#[derive(Debug, Clone, Default)]
pub struct RenderGraph {
    resources: Vec<RenderResource>,
    passes: Vec<RenderPass>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRenderGraph {
    pub resources: Vec<RenderResource>,
    pub passes: Vec<RenderPass>,
}

impl RenderGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_resource(&mut self, resource: RenderResource) -> Result<(), GraphError> {
        if self.resources.iter().any(|item| item.id == resource.id) {
            return Err(GraphError::DuplicateResource(resource.id));
        }
        self.resources.push(resource);
        Ok(())
    }

    pub fn add_pass(&mut self, pass: RenderPass) -> Result<(), GraphError> {
        if self.passes.iter().any(|item| item.id == pass.id) {
            return Err(GraphError::DuplicatePass(pass.id));
        }
        self.passes.push(pass);
        Ok(())
    }

    /// Compile explicit dependencies plus deterministic resource hazards.
    ///
    /// When two insertion-ordered passes access the same resource and either
    /// access writes, the later pass depends on the earlier one. This preserves
    /// a simple authoring model without hiding ambiguous concurrent writes.
    pub fn compile(&self) -> Result<CompiledRenderGraph, GraphError> {
        let resource_ids = self
            .resources
            .iter()
            .map(|resource| resource.id)
            .collect::<BTreeSet<_>>();
        let pass_indices = self
            .passes
            .iter()
            .enumerate()
            .map(|(index, pass)| (pass.id, index))
            .collect::<HashMap<_, _>>();
        for pass in &self.passes {
            for dependency in &pass.dependencies {
                if !pass_indices.contains_key(dependency) {
                    return Err(GraphError::UnknownDependency {
                        pass: pass.id,
                        dependency: *dependency,
                    });
                }
            }
            for access in &pass.resources {
                if !resource_ids.contains(&access.resource) {
                    return Err(GraphError::UnknownResource {
                        pass: pass.id,
                        resource: access.resource,
                    });
                }
            }
        }

        let mut initialized = self
            .resources
            .iter()
            .map(|resource| (resource.id, resource.external))
            .collect::<HashMap<_, _>>();
        for pass in &self.passes {
            for access in &pass.resources {
                let ready = initialized[&access.resource];
                if !ready && matches!(access.mode, AccessMode::Read | AccessMode::ReadWrite) {
                    return Err(GraphError::UninitializedRead {
                        pass: pass.id,
                        resource: access.resource,
                    });
                }
                if access.mode.writes() {
                    initialized.insert(access.resource, true);
                }
            }
        }

        let mut dependencies = self
            .passes
            .iter()
            .map(|pass| pass.dependencies.iter().copied().collect::<BTreeSet<_>>())
            .collect::<Vec<_>>();
        for (later, later_dependencies) in dependencies.iter_mut().enumerate() {
            for earlier in 0..later {
                if has_hazard(&self.passes[earlier], &self.passes[later]) {
                    later_dependencies.insert(self.passes[earlier].id);
                }
            }
        }

        let mut ready = BTreeSet::new();
        let mut emitted = BTreeSet::new();
        for (index, deps) in dependencies.iter().enumerate() {
            if deps.is_empty() {
                ready.insert(index);
            }
        }
        let mut ordered = Vec::with_capacity(self.passes.len());
        while let Some(index) = ready.pop_first() {
            if !emitted.insert(self.passes[index].id) {
                continue;
            }
            let mut pass = self.passes[index].clone();
            pass.dependencies = dependencies[index].iter().copied().collect();
            ordered.push(pass);
            for (candidate, deps) in dependencies.iter().enumerate() {
                if !emitted.contains(&self.passes[candidate].id)
                    && deps.iter().all(|dependency| emitted.contains(dependency))
                {
                    ready.insert(candidate);
                }
            }
        }
        if ordered.len() != self.passes.len() {
            return Err(GraphError::Cycle);
        }
        Ok(CompiledRenderGraph {
            resources: self.resources.clone(),
            passes: ordered,
        })
    }
}

fn has_hazard(earlier: &RenderPass, later: &RenderPass) -> bool {
    earlier.resources.iter().any(|left| {
        later.resources.iter().any(|right| {
            left.resource == right.resource && (left.mode.writes() || right.mode.writes())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(id: u64, dependencies: &[u64], mode: AccessMode) -> RenderPass {
        RenderPass {
            id: PassId(id),
            label: format!("pass-{id}"),
            dependencies: dependencies.iter().copied().map(PassId).collect(),
            resources: vec![ResourceAccess {
                resource: ResourceId(1),
                mode,
            }],
            operations: Vec::new(),
        }
    }

    #[test]
    fn resource_hazards_preserve_pass_order() {
        let mut graph = RenderGraph::new();
        graph
            .add_resource(RenderResource {
                id: ResourceId(1),
                label: "surface".into(),
                external: true,
            })
            .unwrap();
        graph.add_pass(pass(1, &[], AccessMode::Write)).unwrap();
        graph.add_pass(pass(2, &[], AccessMode::ReadWrite)).unwrap();
        let compiled = graph.compile().unwrap();
        assert_eq!(compiled.passes[1].dependencies, vec![PassId(1)]);
    }

    #[test]
    fn explicit_cycle_is_rejected() {
        let mut graph = RenderGraph::new();
        graph
            .add_resource(RenderResource {
                id: ResourceId(1),
                label: "surface".into(),
                external: true,
            })
            .unwrap();
        graph.add_pass(pass(1, &[2], AccessMode::Read)).unwrap();
        graph.add_pass(pass(2, &[1], AccessMode::Read)).unwrap();
        assert_eq!(graph.compile(), Err(GraphError::Cycle));
    }

    #[test]
    fn transient_resource_must_be_written_before_read() {
        let mut graph = RenderGraph::new();
        graph
            .add_resource(RenderResource {
                id: ResourceId(1),
                label: "temporary".into(),
                external: false,
            })
            .unwrap();
        graph.add_pass(pass(1, &[], AccessMode::Read)).unwrap();
        assert_eq!(
            graph.compile(),
            Err(GraphError::UninitializedRead {
                pass: PassId(1),
                resource: ResourceId(1),
            })
        );
    }
}
