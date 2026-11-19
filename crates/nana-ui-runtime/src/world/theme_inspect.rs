//! On-demand theme provenance, dependency summary, and last invalidation trace (#109).

use std::collections::BTreeMap;

use super::*;
use nana_ui_core::{
    ScopeGeneration, SemanticColor, SemanticColorRole, ThemeAppearance, ThemeCategoryGenerations,
    ThemeDependencyMask, ThemeGeneration, ThemeId, ThemePackageProvenance, ThemeRegistry,
    ThemeScopeId, ThemeWorkCounters,
};

/// One layer in the resolved [`ThemeScopeId`] chain covering a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeScopeChainEntry {
    pub scope_id: ThemeScopeId,
    pub owner: StableNodeId,
    pub generation: ScopeGeneration,
    pub invalid_patch: bool,
    pub overrides: Vec<(String, String)>,
}

/// What triggered the most recent theme invalidation pass on `UiWorld`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeInvalidationTrigger {
    RootTheme {
        theme_id: ThemeId,
        generation: ThemeGeneration,
    },
    DocumentTheme {
        document: DocumentId,
        theme_id: ThemeId,
        generation: ThemeGeneration,
    },
    ScopePatch {
        owner: StableNodeId,
        scope_id: ThemeScopeId,
        generation: ScopeGeneration,
    },
    ScopeRemoved {
        owner: StableNodeId,
        parent_scope: ThemeScopeId,
    },
    ScopeReparented {
        root: StableNodeId,
        old_scope: ThemeScopeId,
        new_scope: ThemeScopeId,
    },
}

/// Summary of the most recent theme/scope invalidation pass (#109).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastThemeInvalidation {
    pub trigger: ThemeInvalidationTrigger,
    pub changed_mask: ThemeDependencyMask,
    pub category_generations: ThemeCategoryGenerations,
    pub candidates_invalidated: usize,
    pub nodes_skipped_by_dependency: usize,
    pub scope_nodes_invalidated: usize,
    pub recipe_nodes_invalidated: usize,
    pub layout_seeds: usize,
    pub motion_policy_only: bool,
}

/// On-demand inspection snapshot of one node's resolved theme state (#109).
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeNodeInspection {
    pub node: StableNodeId,
    pub document: DocumentId,
    pub effective_theme_id: ThemeId,
    pub effective_theme_generation: ThemeGeneration,
    pub appearance: ThemeAppearance,
    pub active_scope: ThemeScopeId,
    pub scope_generation: ScopeGeneration,
    pub document_theme_override: Option<ThemeId>,
    pub scope_chain: Vec<ThemeScopeChainEntry>,
    pub color_overrides: Vec<(SemanticColorRole, SemanticColor)>,
    pub dependency_mask: ThemeDependencyMask,
    pub package_provenance: Option<ThemePackageProvenance>,
    pub last_invalidation: Option<LastThemeInvalidation>,
}

/// Aggregate distribution of tracked node dependency categories across `UiWorld`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeDependencySummary {
    pub total_tracked_nodes: usize,
    pub category_counts: Vec<(&'static str, usize)>,
}

impl UiWorld {
    pub(crate) fn record_theme_invalidation(
        &mut self,
        trigger: ThemeInvalidationTrigger,
        changed_mask: ThemeDependencyMask,
        work: &ThemeWorkCounters,
        layout_seeds: usize,
        motion_policy_only: bool,
    ) {
        self.theme_state.last_theme_invalidation = Some(LastThemeInvalidation {
            trigger,
            changed_mask,
            category_generations: self.theme_state.theme_generations,
            candidates_invalidated: work.theme_revision_candidates,
            nodes_skipped_by_dependency: work.theme_dependency_skipped,
            scope_nodes_invalidated: work.scope_nodes_invalidated,
            recipe_nodes_invalidated: work.component_recipe_nodes_invalidated,
            layout_seeds,
            motion_policy_only,
        });
    }

    /// Inspect the effective theme, scope chain, color overrides, dependency mask,
    /// and last invalidation trace for `id`.
    pub fn inspect_node_theme(&self, id: StableNodeId) -> Option<ThemeNodeInspection> {
        self.inspect_node_theme_with_registry(id, None)
    }

    /// Like [`Self::inspect_node_theme`], also attaching [`ThemePackageProvenance`]
    /// when `registry` holds provenance for the node's effective base theme.
    pub fn inspect_node_theme_with_registry(
        &self,
        id: StableNodeId,
        registry: Option<&ThemeRegistry>,
    ) -> Option<ThemeNodeInspection> {
        let record = self.nodes.get(id)?;
        let document = record.document;
        let active_scope = self.nodes.node_scope(id);
        let effective = self.theme_for_scope(document, active_scope);
        let identity = effective.identity();
        let scope_generation = self.theme_state.theme_scopes.generation(active_scope);
        let document_theme_override = self
            .theme_state
            .document_themes
            .get(&document)
            .map(|theme| theme.id());

        let scope_chain = self
            .theme_state
            .theme_scopes
            .scope_chain(active_scope)
            .into_iter()
            .filter_map(|scope_id| {
                let entry = self.theme_state.theme_scopes.entry(scope_id)?;
                Some(ThemeScopeChainEntry {
                    scope_id,
                    owner: entry.owner,
                    generation: entry.generation,
                    invalid_patch: entry.invalid_patch,
                    overrides: entry.patch.describe_overrides(),
                })
            })
            .collect();

        let color_overrides = record
            .resolved
            .0
            .color_scope
            .as_deref()
            .or(record.style.color_overrides.as_deref())
            .map(|overrides| overrides.iter().collect())
            .unwrap_or_default();

        let package_provenance = registry.and_then(|reg| {
            reg.provenance(&identity.id)
                .or_else(|| reg.provenance(&self.document_theme(document).id()))
                .cloned()
        });

        Some(ThemeNodeInspection {
            node: id,
            document,
            effective_theme_id: identity.id,
            effective_theme_generation: identity.generation,
            appearance: effective.appearance(),
            active_scope,
            scope_generation,
            document_theme_override,
            scope_chain,
            color_overrides,
            dependency_mask: self.nodes.theme_dependency(id),
            package_provenance,
            last_invalidation: self.theme_state.last_theme_invalidation.clone(),
        })
    }

    /// Summary of the most recent theme or scope invalidation pass across `UiWorld`.
    pub fn last_theme_invalidation(&self) -> Option<&LastThemeInvalidation> {
        self.theme_state.last_theme_invalidation.as_ref()
    }

    /// Aggregate count of live tracked nodes and per-category dependency usage.
    pub fn theme_dependency_summary(&self) -> ThemeDependencySummary {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut total_tracked_nodes = 0usize;
        for (id, mask) in self.nodes.theme_dependency_entries() {
            if !self.contains(id) {
                continue;
            }
            total_tracked_nodes += 1;
            for name in mask.names() {
                *counts.entry(name).or_default() += 1;
            }
        }
        ThemeDependencySummary {
            total_tracked_nodes,
            category_counts: counts.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, Button, LayoutViewport, MeasureTextShaper, Stack, Switch};
    use nana_ui_core::{
        ButtonKind, ColorOverrides, MotionPatch, SemanticColor, SemanticColorRole, ThemeDefinition,
        ThemeId, ThemePackage, ThemePatch, ThemeRegistry, builtin_theme, builtin_theme_arc,
    };

    fn settle(context: &mut AppContext, document: DocumentId) {
        let viewport = LayoutViewport::new(320.0, 200.0);
        context.layout_document(document, viewport).unwrap();
        let work = context.compat_world_mut().take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        if !work.text.is_empty() {
            context
                .compat_world_mut()
                .shape_text(&work.text, &mut MeasureTextShaper)
                .unwrap();
            context.layout_document(document, viewport).unwrap();
            let follow_up = context.compat_world_mut().take_system_work();
            context
                .compat_world_mut()
                .resolve_styles(&follow_up.style)
                .unwrap();
        }
    }

    #[test]
    fn inspect_node_theme_reports_nested_scopes_colors_and_provenance() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();

        let mut registry = ThemeRegistry::default();
        let pkg_json = ThemePackage::new(ThemeId::from_owned("acme.sunset"), "Acme Sunset")
            .with_base(ThemeDefinition::NANA_DARK.id)
            .with_patch(
                ThemePatch::new()
                    .with_color(SemanticColorRole::Accent, SemanticColor::rgb8(192, 80, 208)),
            )
            .to_json_pretty()
            .unwrap();
        let compiled = registry.register_package_json(&pkg_json).unwrap();
        context.set_theme_tokens(Arc::clone(&compiled)).unwrap();

        let mut outer = Stack::column(0.0);
        outer.style_mut().theme_patch = Some(Arc::new(ThemePatch::default().with_color(
            SemanticColorRole::Warning,
            SemanticColor::rgb8(240, 180, 40),
        )));
        let outer = context.create_component(document, outer).unwrap();

        let mut inner = Stack::column(0.0);
        inner.style_mut().theme_patch = Some(Arc::new(
            ThemePatch::default()
                .with_color(SemanticColorRole::Danger, SemanticColor::rgb8(220, 60, 70)),
        ));
        inner.style_mut().color_overrides = Some(Arc::new(
            ColorOverrides::new()
                .with(SemanticColorRole::Accent, SemanticColor::rgb8(10, 200, 150)),
        ));
        let inner = context.create_component(document, inner).unwrap();

        let button = context
            .create_component(document, Button::new("Save").kind(ButtonKind::Primary))
            .unwrap();
        let doc_only = context
            .create_component(document, Switch::new("Doc", true))
            .unwrap();

        context.append_child(outer, inner).unwrap();
        context.append_child(inner, button).unwrap();
        settle(&mut context, document);

        let inspected = context
            .inspect_node_theme_with_registry(button.stable_id(), Some(&registry))
            .expect("node inspection");
        assert_eq!(inspected.effective_theme_id.as_str(), "acme.sunset");
        assert_eq!(inspected.appearance, ThemeAppearance::Dark);
        assert!(!inspected.active_scope.is_root());
        assert_eq!(inspected.scope_chain.len(), 2);
        assert_eq!(inspected.scope_chain[0].owner, outer.stable_id());
        assert_eq!(inspected.scope_chain[1].owner, inner.stable_id());
        assert!(
            inspected.scope_chain[0]
                .overrides
                .iter()
                .any(|(k, _)| k == "colors.Warning"),
            "{:?}",
            inspected.scope_chain[0].overrides
        );
        assert!(
            inspected.scope_chain[1]
                .overrides
                .iter()
                .any(|(k, _)| k == "colors.Danger"),
            "{:?}",
            inspected.scope_chain[1].overrides
        );
        assert_eq!(
            inspected.color_overrides,
            vec![(SemanticColorRole::Accent, SemanticColor::rgb8(10, 200, 150))]
        );
        let provenance = inspected.package_provenance.expect("package provenance");
        assert_eq!(
            provenance.base_chain,
            vec![
                ThemeId::new("nana.dark"),
                ThemeId::from_owned("acme.sunset")
            ]
        );
        assert!(
            inspected
                .dependency_mask
                .intersects(ThemeDependencyMask::RECIPE_BUTTON)
        );

        let root_inspected = context
            .inspect_node_theme(doc_only.stable_id())
            .expect("root node inspection");
        assert!(root_inspected.active_scope.is_root());
        assert!(root_inspected.scope_chain.is_empty());

        let summary = context.theme_dependency_summary();
        assert!(summary.total_tracked_nodes >= 2);
        assert!(
            summary
                .category_counts
                .iter()
                .any(|(name, count)| *name == "RECIPE_BUTTON" && *count >= 1),
            "{summary:?}"
        );
    }

    #[test]
    fn last_theme_invalidation_tracks_root_motion_document_scope_and_reparent() {
        let mut context = AppContext::new();
        let doc1 = DocumentId::new(1).unwrap();
        let doc2 = DocumentId::new(2).unwrap();

        let switch1 = context
            .create_component(doc1, Switch::new("Root", true))
            .unwrap();
        let switch2 = context
            .create_component(doc2, Switch::new("OtherDoc", true))
            .unwrap();
        let mut scoped_host = Stack::column(0.0);
        scoped_host.style_mut().theme_patch = Some(Arc::new(
            ThemePatch::default()
                .with_color(SemanticColorRole::Accent, SemanticColor::rgb8(90, 40, 200)),
        ));
        let scoped_host = context.create_component(doc1, scoped_host).unwrap();
        let movable = context
            .create_component(doc1, Switch::new("Movable", false))
            .unwrap();
        settle(&mut context, doc1);
        settle(&mut context, doc2);

        // 1. Scope patch invalidation already recorded (or re-patch to test trigger).
        let mut q = MutationQueue::new();
        q.set_style(
            scoped_host.stable_id(),
            crate::NodeStyle {
                theme_patch: Some(Arc::new(
                    ThemePatch::default()
                        .with_color(SemanticColorRole::Accent, SemanticColor::rgb8(40, 160, 220)),
                )),
                ..crate::NodeStyle::default()
            },
        );
        context.commit_mutations(q).unwrap();
        let patch_inv = context.last_theme_invalidation().unwrap();
        assert!(matches!(
            patch_inv.trigger,
            ThemeInvalidationTrigger::ScopePatch { owner, .. } if owner == scoped_host.stable_id()
        ));
        assert!(
            patch_inv
                .changed_mask
                .intersects(ThemeDependencyMask::ACCENT)
        );

        // 2. Reparent into scope updates trigger.
        context.append_child(scoped_host, movable).unwrap();
        let reparent_inv = context.last_theme_invalidation().unwrap();
        assert!(matches!(
            reparent_inv.trigger,
            ThemeInvalidationTrigger::ScopeReparented { root, .. } if root == movable.stable_id()
        ));

        // 3. Remove scope updates trigger.
        let mut q = MutationQueue::new();
        q.set_style(scoped_host.stable_id(), crate::NodeStyle::default());
        context.commit_mutations(q).unwrap();
        let removed_inv = context.last_theme_invalidation().unwrap();
        assert!(matches!(
            removed_inv.trigger,
            ThemeInvalidationTrigger::ScopeRemoved { owner, .. } if owner == scoped_host.stable_id()
        ));

        // 4. Document theme override.
        let light = builtin_theme_arc(ThemeAppearance::Light);
        context.set_document_theme(doc2, Some(light)).unwrap();
        let doc_inv = context.last_theme_invalidation().unwrap();
        assert!(matches!(
            doc_inv.trigger,
            ThemeInvalidationTrigger::DocumentTheme { document, .. } if document == doc2
        ));
        assert_eq!(
            context
                .inspect_node_theme(switch2.stable_id())
                .unwrap()
                .document_theme_override,
            Some(ThemeDefinition::NANA_LIGHT.id)
        );
        assert!(
            context
                .inspect_node_theme(switch1.stable_id())
                .unwrap()
                .document_theme_override
                .is_none()
        );

        // 5. Root theme change.
        context.set_preset_theme(ThemeAppearance::Light).unwrap();
        let root_inv = context.last_theme_invalidation().unwrap();
        assert!(matches!(
            root_inv.trigger,
            ThemeInvalidationTrigger::RootTheme { .. }
        ));
        assert!(!root_inv.motion_policy_only);

        // 6. Motion-policy-only fast path when unscoped.
        context.set_document_theme(doc2, None).unwrap();
        let motion_patch = ThemePatch::default().with_motion_patch(MotionPatch {
            hover_color_ms: Some(123),
            ..Default::default()
        });
        let motion_only = Arc::new(
            motion_patch
                .apply_to_compiled(&builtin_theme(ThemeAppearance::Light))
                .unwrap(),
        );
        context.set_theme_tokens(motion_only).unwrap();
        let motion_inv = context.last_theme_invalidation().unwrap();
        assert!(motion_inv.motion_policy_only);
        assert_eq!(motion_inv.changed_mask, ThemeDependencyMask::MOTION_POLICY);
    }
}
