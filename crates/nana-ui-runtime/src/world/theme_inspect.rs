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

/// Active pointer/focus/accessibility interaction state observed on a node (#109).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThemeInteractionState {
    pub hovered: bool,
    pub active: bool,
    pub focused: bool,
    pub focus_visible: bool,
    pub disabled: bool,
    pub selected: bool,
    pub checked: bool,
}

/// One resolved token value active for an inspected node (#109).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeResolvedToken {
    pub category: &'static str,
    pub name: &'static str,
    pub value: String,
}

/// Effective motion/transition timing policy resolved for an inspected node (#109).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeTransitionSource {
    pub property: &'static str,
    pub duration_ms: u32,
    pub easing: String,
    pub reduced_motion: bool,
}

/// One layer in the node's causal theme override stack (#109).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeTraceLayer {
    pub layer_kind: &'static str,
    pub source: String,
    pub theme_id: Option<String>,
    pub scope_id: Option<ThemeScopeId>,
    pub overridden_categories: Vec<&'static str>,
}

/// Causal explanation of whether the last invalidation affected a given node (#109).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeNodeInvalidationStatus {
    pub affected: bool,
    pub reasons: Vec<String>,
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
    pub target_document: Option<DocumentId>,
    pub scope_root: Option<StableNodeId>,
    pub candidates_invalidated: usize,
    pub nodes_skipped_by_dependency: usize,
    pub scope_nodes_invalidated: usize,
    pub recipe_nodes_invalidated: usize,
    pub layout_seeds: usize,
    pub motion_policy_only: bool,
    pub resolved_style_changed: usize,
    pub resolved_style_unchanged: usize,
    pub paint_nodes_from_style: usize,
    pub text_nodes_from_style: usize,
    pub style_text_shape_invalidations: usize,
    pub style_text_layout_invalidations: usize,
    pub layout_nodes_from_style: usize,
    pub style_motion_policy_invalidations: usize,
    pub triggered_stages: Vec<&'static str>,
    pub stage_reasons: Vec<String>,
}

/// On-demand inspection snapshot of one node's resolved theme state (#109).
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeNodeInspection {
    pub node: StableNodeId,
    pub document: DocumentId,
    pub component: String,
    pub effective_theme_id: ThemeId,
    pub effective_theme_generation: ThemeGeneration,
    pub appearance: ThemeAppearance,
    pub high_contrast: bool,
    pub active_scope: ThemeScopeId,
    pub scope_generation: ScopeGeneration,
    pub document_theme_override: Option<ThemeId>,
    pub scope_chain: Vec<ThemeScopeChainEntry>,
    pub color_overrides: Vec<(SemanticColorRole, SemanticColor)>,
    pub dependency_mask: ThemeDependencyMask,
    pub recipe_families: Vec<&'static str>,
    pub interaction_state: ThemeInteractionState,
    pub resolved_tokens: Vec<ThemeResolvedToken>,
    pub transitions: Vec<ThemeTransitionSource>,
    pub resolution_trace: Vec<ThemeTraceLayer>,
    pub package_provenance: Option<ThemePackageProvenance>,
    pub last_invalidation: Option<LastThemeInvalidation>,
    pub last_invalidation_status: Option<ThemeNodeInvalidationStatus>,
}

/// Aggregate distribution of tracked node dependency categories across `UiWorld`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeDependencySummary {
    pub total_tracked_nodes: usize,
    pub category_counts: Vec<(&'static str, usize)>,
}

fn format_rgba(color: [f32; 4]) -> String {
    format!(
        "rgba({:.3}, {:.3}, {:.3}, {:.3})",
        color[0], color[1], color[2], color[3]
    )
}

fn categories_from_override_paths(overrides: &[(String, String)]) -> Vec<&'static str> {
    let mut categories = Vec::new();
    let mut push = |name: &'static str| {
        if !categories.contains(&name) {
            categories.push(name);
        }
    };
    for (path, _) in overrides {
        if path.starts_with("colors.") || path == "titlebar" {
            push("PALETTE");
        } else if path.starts_with("accent.") {
            push("ACCENT");
        } else if path.starts_with("metrics.") || path.starts_with("spacing.") {
            push("METRICS");
        } else if path.starts_with("typography.") {
            push("TYPOGRAPHY");
        } else if path.starts_with("motion.") {
            push("MOTION_POLICY");
        } else if path.starts_with("effects.") || path.starts_with("opacity.") {
            push("EFFECT");
        } else if path.starts_with("recipes.") || path.starts_with("surfaces.") {
            push("RECIPE");
        } else if path == "appearance" {
            push("APPEARANCE");
        }
    }
    categories
}

fn build_triggered_stages(inv: &LastThemeInvalidation) -> (Vec<&'static str>, Vec<String>) {
    let mut stages = Vec::new();
    let mut reasons = Vec::new();
    if inv.motion_policy_only || inv.style_motion_policy_invalidations > 0 {
        stages.push("motion_policy");
        reasons.push(format!(
            "motion_policy: {} invalidation(s) handled without style/layout pass",
            inv.style_motion_policy_invalidations.max(1)
        ));
    }
    if inv.candidates_invalidated > 0
        || inv.resolved_style_changed > 0
        || inv.resolved_style_unchanged > 0
    {
        stages.push("style");
        reasons.push(format!(
            "style: {} candidate(s), {} changed, {} unchanged, {} skipped",
            inv.candidates_invalidated,
            inv.resolved_style_changed,
            inv.resolved_style_unchanged,
            inv.nodes_skipped_by_dependency
        ));
    }
    let expects_paint = !inv.motion_policy_only
        && inv.candidates_invalidated > 0
        && inv.changed_mask.intersects(
            ThemeDependencyMask::PALETTE
                .union(ThemeDependencyMask::ACCENT)
                .union(ThemeDependencyMask::EFFECT)
                .union(ThemeDependencyMask::RECIPE_ANY),
        );
    if inv.paint_nodes_from_style > 0 || expects_paint {
        stages.push("paint");
        reasons.push(format!(
            "paint: {} node(s) scheduled for render from style delta",
            inv.paint_nodes_from_style
        ));
    }
    if inv.style_text_shape_invalidations > 0 {
        stages.push("text_shape");
        reasons.push(format!(
            "text_shape: {} node(s) require text reshaping",
            inv.style_text_shape_invalidations
        ));
    }
    if inv.style_text_layout_invalidations > 0 || inv.text_nodes_from_style > 0 {
        stages.push("text_layout");
        reasons.push(format!(
            "text_layout: {} node(s) marked dirty (layout-only={})",
            inv.text_nodes_from_style, inv.style_text_layout_invalidations
        ));
    }
    if inv.layout_seeds > 0 || inv.layout_nodes_from_style > 0 {
        stages.push("layout");
        reasons.push(format!(
            "layout: {} frontier seed(s), {} node(s) invalidated from style",
            inv.layout_seeds, inv.layout_nodes_from_style
        ));
    }
    (stages, reasons)
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
        let (target_document, scope_root) = match &trigger {
            ThemeInvalidationTrigger::RootTheme { .. } => (None, None),
            ThemeInvalidationTrigger::DocumentTheme { document, .. } => (Some(*document), None),
            ThemeInvalidationTrigger::ScopePatch { owner, .. }
            | ThemeInvalidationTrigger::ScopeRemoved { owner, .. } => {
                (self.document_of(*owner), Some(*owner))
            }
            ThemeInvalidationTrigger::ScopeReparented { root, .. } => {
                (self.document_of(*root), Some(*root))
            }
        };
        let mut record = LastThemeInvalidation {
            trigger,
            changed_mask,
            category_generations: self.invalidation.theme_state.theme_generations,
            target_document,
            scope_root,
            candidates_invalidated: work.theme_revision_candidates,
            nodes_skipped_by_dependency: work.theme_dependency_skipped,
            scope_nodes_invalidated: work.scope_nodes_invalidated,
            recipe_nodes_invalidated: work.component_recipe_nodes_invalidated,
            layout_seeds,
            motion_policy_only,
            resolved_style_changed: work.resolved_style_changed,
            resolved_style_unchanged: work.resolved_style_unchanged,
            paint_nodes_from_style: work.paint_nodes_from_style,
            text_nodes_from_style: work.text_nodes_from_style,
            style_text_shape_invalidations: work.style_text_shape_invalidations,
            style_text_layout_invalidations: work.style_text_layout_invalidations,
            layout_nodes_from_style: work.layout_nodes_from_style,
            style_motion_policy_invalidations: work.style_motion_policy_invalidations,
            triggered_stages: Vec::new(),
            stage_reasons: Vec::new(),
        };
        (record.triggered_stages, record.stage_reasons) = build_triggered_stages(&record);
        self.invalidation.theme_state.last_theme_invalidation = Some(record);
    }

    pub(super) fn fold_theme_invalidation_resolution(&mut self, work: &ThemeWorkCounters) {
        let Some(inv) = self
            .invalidation
            .theme_state
            .last_theme_invalidation
            .as_mut()
        else {
            return;
        };
        inv.resolved_style_changed = inv
            .resolved_style_changed
            .saturating_add(work.resolved_style_changed);
        inv.resolved_style_unchanged = inv
            .resolved_style_unchanged
            .saturating_add(work.resolved_style_unchanged);
        inv.paint_nodes_from_style = inv
            .paint_nodes_from_style
            .saturating_add(work.paint_nodes_from_style);
        inv.text_nodes_from_style = inv
            .text_nodes_from_style
            .saturating_add(work.text_nodes_from_style);
        inv.style_text_shape_invalidations = inv
            .style_text_shape_invalidations
            .saturating_add(work.style_text_shape_invalidations);
        inv.style_text_layout_invalidations = inv
            .style_text_layout_invalidations
            .saturating_add(work.style_text_layout_invalidations);
        inv.layout_nodes_from_style = inv
            .layout_nodes_from_style
            .saturating_add(work.layout_nodes_from_style);
        inv.style_motion_policy_invalidations = inv
            .style_motion_policy_invalidations
            .max(work.style_motion_policy_invalidations);
        (inv.triggered_stages, inv.stage_reasons) = build_triggered_stages(inv);
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
        let accessibility = self.nodes.accessibility(id);
        let component = match record.kind.as_ref() {
            NodeKind::Document => "document".to_owned(),
            NodeKind::Text => "text".to_owned(),
            NodeKind::Comment => "comment".to_owned(),
            NodeKind::Element { tag } => format!("{tag} ({:?})", accessibility.role),
        };
        let active_scope = self.nodes.node_scope(id);
        let effective = self.theme_for_scope(document, active_scope);
        let identity = effective.identity();
        let scope_generation = self
            .invalidation
            .theme_state
            .theme_scopes
            .generation(active_scope);
        let document_theme_override = self
            .invalidation
            .theme_state
            .document_themes
            .get(&document)
            .map(|theme| theme.id());

        let scope_chain: Vec<ThemeScopeChainEntry> = self
            .invalidation
            .theme_state
            .theme_scopes
            .scope_chain(active_scope)
            .into_iter()
            .filter_map(|scope_id| {
                let entry = self.invalidation.theme_state.theme_scopes.entry(scope_id)?;
                Some(ThemeScopeChainEntry {
                    scope_id,
                    owner: entry.owner,
                    generation: entry.generation,
                    invalid_patch: entry.invalid_patch,
                    overrides: entry.patch.describe_overrides(),
                })
            })
            .collect();

        let color_overrides: Vec<(SemanticColorRole, SemanticColor)> = record
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

        let dependency_mask = self.nodes.theme_dependency(id);
        let recipe_families: Vec<&'static str> = dependency_mask
            .names()
            .into_iter()
            .filter(|name| name.starts_with("RECIPE_"))
            .collect();

        let paint_state = self.paint_state(id);
        let interaction_state = ThemeInteractionState {
            hovered: paint_state.hovered,
            active: paint_state.pressed,
            focused: self.focused(document) == Some(id),
            focus_visible: paint_state.focused,
            disabled: accessibility.disabled,
            selected: accessibility.selected == Some(true),
            checked: accessibility.checked == Some(true) || accessibility.mixed,
        };

        let computed = &record.resolved.0;
        let metrics = effective.metrics();
        let reduced_motion = self.reduced_motion();
        let mut resolved_tokens = Vec::with_capacity(9);
        if let Some(c) = computed.color {
            resolved_tokens.push(ThemeResolvedToken {
                category: "color",
                name: "color",
                value: format_rgba(c),
            });
        }
        if let Some(bg) = computed.background {
            resolved_tokens.push(ThemeResolvedToken {
                category: "color",
                name: "background",
                value: format_rgba(bg),
            });
        }
        if let Some(border) = computed.border_color {
            resolved_tokens.push(ThemeResolvedToken {
                category: "color",
                name: "border_color",
                value: format_rgba(border),
            });
        }
        if let Some(ring) = computed.ring_color {
            resolved_tokens.push(ThemeResolvedToken {
                category: "color",
                name: "ring_color",
                value: format_rgba(ring),
            });
        }
        resolved_tokens.push(ThemeResolvedToken {
            category: "typography",
            name: "font_size",
            value: format!("{:.1}px", computed.font_size),
        });
        let font_weight = computed
            .font_weight
            .unwrap_or_else(|| effective.typography().regular);
        resolved_tokens.push(ThemeResolvedToken {
            category: "typography",
            name: "font_weight",
            value: font_weight.to_string(),
        });
        resolved_tokens.push(ThemeResolvedToken {
            category: "metrics",
            name: "control_height",
            value: format!("{:.1}px", metrics.control_height),
        });
        resolved_tokens.push(ThemeResolvedToken {
            category: "metrics",
            name: "radius_md",
            value: format!("{:.1}px", metrics.radius_md),
        });
        resolved_tokens.push(ThemeResolvedToken {
            category: "motion",
            name: "reduced_motion",
            value: reduced_motion.to_string(),
        });

        let motion = effective.motion();
        let mut transitions = vec![
            ThemeTransitionSource {
                property: "hover_color",
                duration_ms: u32::from(motion.hover_color_ms),
                easing: motion.standard_easing.to_string(),
                reduced_motion,
            },
            ThemeTransitionSource {
                property: "overlay_fade",
                duration_ms: u32::from(motion.overlay_fade_ms),
                easing: motion.standard_easing.to_string(),
                reduced_motion,
            },
            ThemeTransitionSource {
                property: "menu_pop",
                duration_ms: u32::from(motion.menu_pop_ms),
                easing: motion.emphasized_easing.to_string(),
                reduced_motion,
            },
        ];
        if dependency_mask.intersects(ThemeDependencyMask::RECIPE_SCROLLBAR) {
            let sm = effective.recipes().scrollbar().motion;
            transitions.push(ThemeTransitionSource {
                property: "scrollbar_expand",
                duration_ms: u32::from(sm.expand.millis),
                easing: sm.expand.easing.to_string(),
                reduced_motion,
            });
        }
        if dependency_mask.intersects(ThemeDependencyMask::RECIPE_DIALOG) {
            let dm = effective.recipes().dialog().motion;
            transitions.push(ThemeTransitionSource {
                property: "dialog_card_fade",
                duration_ms: u32::from(dm.card_fade.millis),
                easing: dm.card_fade.easing.to_string(),
                reduced_motion,
            });
        }
        if dependency_mask.intersects(ThemeDependencyMask::RECIPE_TOAST) {
            let tm = effective.recipes().toast().motion;
            transitions.push(ThemeTransitionSource {
                property: "toast_enter",
                duration_ms: u32::from(tm.enter.millis),
                easing: tm.enter.easing.to_string(),
                reduced_motion,
            });
        }

        let mut resolution_trace = Vec::new();
        let root_id = self.invalidation.theme.id();
        resolution_trace.push(ThemeTraceLayer {
            layer_kind: "root_theme",
            source: "application root".to_owned(),
            theme_id: Some(root_id.as_str().to_owned()),
            scope_id: None,
            overridden_categories: Vec::new(),
        });
        if let Some(prov) = package_provenance.as_ref() {
            for (pkg_id, overrides) in &prov.patch_entries {
                resolution_trace.push(ThemeTraceLayer {
                    layer_kind: "package_layer",
                    source: format!("package {}", pkg_id.as_str()),
                    theme_id: Some(pkg_id.as_str().to_owned()),
                    scope_id: None,
                    overridden_categories: categories_from_override_paths(overrides),
                });
            }
        }
        if let Some(doc_theme_id) = document_theme_override.as_ref() {
            resolution_trace.push(ThemeTraceLayer {
                layer_kind: "document_theme",
                source: format!("document {}", document.get()),
                theme_id: Some(doc_theme_id.as_str().to_owned()),
                scope_id: None,
                overridden_categories: vec!["DOCUMENT_OVERRIDE"],
            });
        }
        for entry in &scope_chain {
            resolution_trace.push(ThemeTraceLayer {
                layer_kind: "theme_scope",
                source: format!("node {}", entry.owner.get()),
                theme_id: None,
                scope_id: Some(entry.scope_id),
                overridden_categories: categories_from_override_paths(&entry.overrides),
            });
        }
        if !color_overrides.is_empty() {
            resolution_trace.push(ThemeTraceLayer {
                layer_kind: "color_overrides",
                source: format!("node {}", id.get()),
                theme_id: None,
                scope_id: None,
                overridden_categories: vec!["COLOR_OVERRIDES"],
            });
        }
        if record.style.background.is_some()
            || record.style.foreground.is_some()
            || record.style.border.is_some()
            || !record.style.interaction.is_shared_default()
        {
            resolution_trace.push(ThemeTraceLayer {
                layer_kind: "node_style",
                source: format!("node {}", id.get()),
                theme_id: None,
                scope_id: None,
                overridden_categories: dependency_mask.names(),
            });
        }

        let last_invalidation = self
            .invalidation
            .theme_state
            .last_theme_invalidation
            .clone();
        let last_invalidation_status = last_invalidation.as_ref().map(|inv| {
            let mut reasons = Vec::new();
            let affected = if inv.target_document.is_some_and(|d| d != document) {
                reasons.push(format!(
                    "isolated: invalidation targeted document {}, node is in document {}",
                    inv.target_document.unwrap().get(),
                    document.get()
                ));
                false
            } else if matches!(inv.trigger, ThemeInvalidationTrigger::RootTheme { .. })
                && document_theme_override.is_some()
            {
                reasons.push("isolated: document has an independent theme override".to_owned());
                false
            } else if inv
                .scope_root
                .is_some_and(|root| !self.is_descendant_or_self(id, root))
            {
                reasons.push(format!(
                    "isolated: outside invalidated subtree root {}",
                    inv.scope_root.unwrap().get()
                ));
                false
            } else if inv.motion_policy_only {
                reasons.push(
                    "motion_policy fast path: motion updated without style/layout".to_owned(),
                );
                true
            } else {
                let matched = dependency_mask.intersection(inv.changed_mask);
                if !matched.is_empty() {
                    reasons.push(format!(
                        "invalidated via dependency categories: {}",
                        matched.names().join(", ")
                    ));
                    true
                } else {
                    reasons.push(
                        "skipped: node dependency mask does not intersect changed categories"
                            .to_owned(),
                    );
                    false
                }
            };
            ThemeNodeInvalidationStatus { affected, reasons }
        });

        Some(ThemeNodeInspection {
            node: id,
            document,
            component,
            effective_theme_id: identity.id,
            effective_theme_generation: identity.generation,
            appearance: effective.appearance(),
            high_contrast: self.invalidation.high_contrast,
            active_scope,
            scope_generation,
            document_theme_override,
            scope_chain,
            color_overrides,
            dependency_mask,
            recipe_families,
            interaction_state,
            resolved_tokens,
            transitions,
            resolution_trace,
            package_provenance,
            last_invalidation,
            last_invalidation_status,
        })
    }

    /// Summary of the most recent theme or scope invalidation pass across `UiWorld`.
    pub fn last_theme_invalidation(&self) -> Option<&LastThemeInvalidation> {
        self.invalidation
            .theme_state
            .last_theme_invalidation
            .as_ref()
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
        assert!(
            inspected.component.contains("Button"),
            "{}",
            inspected.component
        );
        assert!(inspected.recipe_families.contains(&"RECIPE_BUTTON"));
        assert!(!inspected.high_contrast);
        assert!(!inspected.resolved_tokens.is_empty());
        assert!(!inspected.transitions.is_empty());
        assert!(
            inspected
                .resolution_trace
                .iter()
                .any(|layer| layer.layer_kind == "root_theme")
        );
        assert!(
            inspected
                .resolution_trace
                .iter()
                .any(|layer| layer.layer_kind == "package_layer")
        );
        assert!(
            inspected
                .resolution_trace
                .iter()
                .any(|layer| layer.layer_kind == "theme_scope")
        );
        assert!(
            inspected
                .resolution_trace
                .iter()
                .any(|layer| layer.layer_kind == "color_overrides")
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
        settle(&mut context, doc1);
        let root_inv = context.last_theme_invalidation().unwrap();
        assert!(matches!(
            root_inv.trigger,
            ThemeInvalidationTrigger::RootTheme { .. }
        ));
        assert!(!root_inv.motion_policy_only);
        assert!(root_inv.target_document.is_none());
        assert!(root_inv.resolved_style_changed > 0);
        assert!(root_inv.triggered_stages.contains(&"style"));
        let switch2_isolated = context
            .inspect_node_theme(switch2.stable_id())
            .unwrap()
            .last_invalidation_status
            .expect("invalidation status");
        assert!(!switch2_isolated.affected);
        let switch1_affected = context
            .inspect_node_theme(switch1.stable_id())
            .unwrap()
            .last_invalidation_status
            .expect("invalidation status");
        assert!(switch1_affected.affected);

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
        assert!(motion_inv.triggered_stages.contains(&"motion_policy"));
    }
}
