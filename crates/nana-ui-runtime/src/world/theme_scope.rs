//! Subtree `ThemeScope`, `ThemePatch` O(1) resolution cache, and per-document root themes (#104).

use std::collections::{HashMap, VecDeque};

use super::*;
use nana_ui_core::{
    ChromeRadii, ColorOverrides, CompiledTheme, ScopeGeneration, StyleModelRef,
    ThemeCategoryGenerations, ThemeDependencyMask, ThemePatch, ThemeScopeId, ThemeWorkCounters,
};

const MAX_SCOPE_CACHE: usize = 256;
const MAX_SCOPE_CHAIN_DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub(crate) struct ScopeEntry {
    pub(crate) id: ThemeScopeId,
    pub(crate) owner: StableNodeId,
    pub(crate) document: DocumentId,
    pub(crate) parent_scope: ThemeScopeId,
    pub(crate) patch: Arc<ThemePatch>,
    pub(crate) resolved: Arc<CompiledTheme>,
    pub(crate) generation: ScopeGeneration,
    pub(crate) invalid_patch: bool,
}

#[derive(Clone)]
struct ScopeCacheEntry {
    parent: Arc<CompiledTheme>,
    patch: Arc<ThemePatch>,
    resolved: Arc<CompiledTheme>,
}

/// Authoritative registry and O(1) resolution cache for live `ThemeScope` nodes.
#[derive(Default)]
pub(crate) struct ThemeScopeRegistry {
    next_id: u32,
    scopes: HashMap<ThemeScopeId, ScopeEntry, BuildIdHasher>,
    owner_to_scope: HashMap<StableNodeId, ThemeScopeId, BuildIdHasher>,
    cache: HashMap<(usize, u32), Vec<ScopeCacheEntry>>,
    cache_order: VecDeque<(usize, u32)>,
    cache_count: usize,
}

/// Heap-boxed theme runtime bookkeeping to keep `UiWorld` compact on the stack.
pub(crate) struct ThemeRuntimeState {
    pub(crate) theme_work: ThemeWorkCounters,
    pub(crate) theme_frame_work: ThemeWorkCounters,
    pub(crate) theme_generations: ThemeCategoryGenerations,
    pub(crate) pending_theme_eval: HashMap<StableNodeId, ThemeDependencyMask, BuildIdHasher>,
    pub(crate) theme_scopes: ThemeScopeRegistry,
    pub(crate) installed_document_themes: HashMap<DocumentId, Arc<CompiledTheme>, BuildIdHasher>,
    pub(crate) document_themes: HashMap<DocumentId, Arc<CompiledTheme>, BuildIdHasher>,
    pub(crate) pending_scope_reprojects: Vec<(StableNodeId, ThemeDependencyMask)>,
    pub(crate) pending_scope_theme_work: ThemeWorkCounters,
    pub(crate) last_theme_invalidation: Option<crate::world::theme_inspect::LastThemeInvalidation>,
}

impl Default for ThemeRuntimeState {
    fn default() -> Self {
        Self {
            theme_work: ThemeWorkCounters::default(),
            theme_frame_work: ThemeWorkCounters::default(),
            theme_generations: ThemeCategoryGenerations::INITIAL,
            pending_theme_eval: HashMap::default(),
            theme_scopes: ThemeScopeRegistry::default(),
            installed_document_themes: HashMap::default(),
            document_themes: HashMap::default(),
            pending_scope_reprojects: Vec::new(),
            pending_scope_theme_work: ThemeWorkCounters::default(),
            last_theme_invalidation: None,
        }
    }
}

impl ThemeScopeRegistry {
    pub(crate) fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.scopes.len()
    }

    pub(crate) fn scope_for_owner(&self, owner: StableNodeId) -> Option<ThemeScopeId> {
        self.owner_to_scope.get(&owner).copied()
    }

    pub(crate) fn entry(&self, scope: ThemeScopeId) -> Option<&ScopeEntry> {
        self.scopes.get(&scope)
    }

    pub(crate) fn theme(&self, scope: ThemeScopeId) -> Option<&Arc<CompiledTheme>> {
        self.scopes.get(&scope).map(|entry| &entry.resolved)
    }

    pub(crate) fn generation(&self, scope: ThemeScopeId) -> ScopeGeneration {
        if scope.is_root() {
            ScopeGeneration::INITIAL
        } else {
            self.scopes
                .get(&scope)
                .map_or(ScopeGeneration::INITIAL, |entry| entry.generation)
        }
    }

    pub(crate) fn parent_scope_of(&self, scope: ThemeScopeId) -> ThemeScopeId {
        self.scopes
            .get(&scope)
            .map_or(ThemeScopeId::ROOT, |entry| entry.parent_scope)
    }

    /// Ancestor-to-leaf chain of scopes ending at `scope` (excluding root).
    pub(crate) fn scope_chain(&self, mut scope: ThemeScopeId) -> Vec<ThemeScopeId> {
        let mut chain = Vec::new();
        let mut depth = 0usize;
        while !scope.is_root() && depth < MAX_SCOPE_CHAIN_DEPTH {
            let Some(entry) = self.scopes.get(&scope) else {
                break;
            };
            chain.push(scope);
            scope = entry.parent_scope;
            depth += 1;
        }
        chain.reverse();
        chain
    }

    pub(crate) fn remove_owner(&mut self, owner: StableNodeId) -> Option<ScopeEntry> {
        let scope = self.owner_to_scope.remove(&owner)?;
        self.scopes.remove(&scope)
    }

    fn resolve_with_cache(
        &mut self,
        owner: StableNodeId,
        parent_theme: &Arc<CompiledTheme>,
        patch: &Arc<ThemePatch>,
        work: &mut ThemeWorkCounters,
    ) -> (Arc<CompiledTheme>, bool) {
        if patch.is_empty() {
            return (Arc::clone(parent_theme), false);
        }
        let key = (
            Arc::as_ptr(parent_theme) as usize,
            patch.affected_mask().bits(),
        );
        if let Some(bucket) = self.cache.get(&key) {
            for cached in bucket {
                if Arc::ptr_eq(&cached.parent, parent_theme)
                    && (Arc::ptr_eq(&cached.patch, patch) || *cached.patch == **patch)
                {
                    work.scope_cache_hits += 1;
                    return (Arc::clone(&cached.resolved), false);
                }
            }
        }
        work.scope_cache_misses += 1;
        match patch.apply_to_compiled(parent_theme) {
            Ok(compiled) => {
                let resolved = Arc::new(compiled);
                self.insert_cache(
                    key,
                    ScopeCacheEntry {
                        parent: Arc::clone(parent_theme),
                        patch: Arc::clone(patch),
                        resolved: Arc::clone(&resolved),
                    },
                );
                (resolved, false)
            }
            Err(_) => {
                nana_diagnostics::event!(
                    nana_diagnostics::framework::runtime::THEME_SCOPE_INVALID_PATCH,
                    node = owner.get()
                );
                (Arc::clone(parent_theme), true)
            }
        }
    }

    fn insert_cache(&mut self, key: (usize, u32), entry: ScopeCacheEntry) {
        while self.cache_count >= MAX_SCOPE_CACHE {
            let Some(oldest_key) = self.cache_order.pop_front() else {
                break;
            };
            if let Some(bucket) = self.cache.get_mut(&oldest_key)
                && !bucket.is_empty()
            {
                bucket.remove(0);
                self.cache_count = self.cache_count.saturating_sub(1);
                if bucket.is_empty() {
                    self.cache.remove(&oldest_key);
                }
            }
        }
        self.cache.entry(key).or_default().push(entry);
        self.cache_order.push_back(key);
        self.cache_count += 1;
    }

    fn allocate_id(&mut self) -> ThemeScopeId {
        loop {
            self.next_id = self.next_id.wrapping_add(1);
            if self.next_id == 0 {
                self.next_id = 1;
            }
            let id = ThemeScopeId(self.next_id);
            if !self.scopes.contains_key(&id) {
                return id;
            }
        }
    }

    /// Create or update `owner`'s scope entry. Returns `(scope_id, next_theme, next_gen, changed_generation)`.
    pub(crate) fn upsert_scope(
        &mut self,
        owner: StableNodeId,
        document: DocumentId,
        parent_scope: ThemeScopeId,
        parent_theme: &Arc<CompiledTheme>,
        patch: Arc<ThemePatch>,
        work: &mut ThemeWorkCounters,
    ) -> (ThemeScopeId, Arc<CompiledTheme>, ScopeGeneration, bool) {
        let (resolved, invalid_patch) = self.resolve_with_cache(owner, parent_theme, &patch, work);
        if let Some(scope_id) = self.owner_to_scope.get(&owner).copied()
            && let Some(entry) = self.scopes.get_mut(&scope_id)
        {
            let changed_gen = *entry.resolved != *resolved;
            if changed_gen {
                entry.generation = entry.generation.next();
                work.scope_generation_changes += 1;
            }
            entry.document = document;
            entry.parent_scope = parent_scope;
            entry.patch = patch;
            entry.resolved = Arc::clone(&resolved);
            entry.invalid_patch = invalid_patch;
            return (scope_id, resolved, entry.generation, changed_gen);
        }
        let id = self.allocate_id();
        work.scope_generation_changes += 1;
        let entry = ScopeEntry {
            id,
            owner,
            document,
            parent_scope,
            patch,
            resolved: Arc::clone(&resolved),
            generation: ScopeGeneration::INITIAL,
            invalid_patch,
        };
        self.owner_to_scope.insert(owner, id);
        self.scopes.insert(id, entry);
        (id, resolved, ScopeGeneration::INITIAL, true)
    }

    pub(crate) fn refresh_scope(
        &mut self,
        scope_id: ThemeScopeId,
        parent_scope: ThemeScopeId,
        parent_theme: &Arc<CompiledTheme>,
        work: &mut ThemeWorkCounters,
    ) -> Option<(Arc<CompiledTheme>, Arc<CompiledTheme>, ScopeGeneration)> {
        let (owner, patch) = {
            let entry = self.scopes.get(&scope_id)?;
            (entry.owner, Arc::clone(&entry.patch))
        };
        let (resolved, invalid_patch) = self.resolve_with_cache(owner, parent_theme, &patch, work);
        let entry = self.scopes.get_mut(&scope_id)?;
        let old_resolved = Arc::clone(&entry.resolved);
        if *old_resolved != *resolved {
            entry.generation = entry.generation.next();
            work.scope_generation_changes += 1;
        }
        entry.parent_scope = parent_scope;
        entry.resolved = Arc::clone(&resolved);
        entry.invalid_patch = invalid_patch;
        Some((old_resolved, resolved, entry.generation))
    }

    /// Scopes present anywhere under `document`.
    pub(crate) fn scopes_in_document(&self, document: DocumentId) -> Vec<ThemeScopeId> {
        self.scopes
            .values()
            .filter(|entry| entry.document == document)
            .map(|entry| entry.id)
            .collect()
    }

    /// Documents that own at least one registered `ThemeScope`.
    pub(crate) fn documents(&self) -> Vec<DocumentId> {
        let mut docs: Vec<DocumentId> = self.scopes.values().map(|entry| entry.document).collect();
        docs.sort_by_key(|doc| doc.get());
        docs.dedup();
        docs
    }
}

impl UiWorld {
    /// Number of currently active non-root [`ThemeScope`](nana_ui_core::ThemeScopeId)s.
    pub fn active_theme_scopes(&self) -> usize {
        self.theme_state.theme_scopes.len()
    }

    /// Whether `document` overrides the workspace root theme via [`Self::set_document_theme`].
    pub fn has_document_theme(&self, document: DocumentId) -> bool {
        self.theme_state.document_themes.contains_key(&document)
    }

    /// Root theme active for `document` (either a per-document override or the
    /// application root theme).
    pub fn document_theme(&self, document: DocumentId) -> &Arc<CompiledTheme> {
        self.theme_state
            .document_themes
            .get(&document)
            .unwrap_or(&self.theme)
    }

    /// Compute high-contrast rendition of an arbitrary installed theme.
    pub(super) fn contrast_rendition_for(
        &self,
        installed: &Arc<CompiledTheme>,
    ) -> Arc<CompiledTheme> {
        if !self.high_contrast {
            return Arc::clone(installed);
        }
        let mut model = installed.style_model();
        model.palette = model.palette.for_system_contrast(true);
        model.titlebar = model.palette.background;
        Arc::new((**installed).clone().with_style_model(model))
    }

    pub(crate) fn theme_arc_for_scope(
        &self,
        document: DocumentId,
        scope: ThemeScopeId,
    ) -> Arc<CompiledTheme> {
        if !scope.is_root()
            && let Some(themed) = self.theme_state.theme_scopes.theme(scope)
        {
            return Arc::clone(themed);
        }
        Arc::clone(self.document_theme(document))
    }

    pub(crate) fn theme_for_scope(
        &self,
        document: DocumentId,
        scope: ThemeScopeId,
    ) -> &CompiledTheme {
        if !scope.is_root()
            && let Some(themed) = self.theme_state.theme_scopes.theme(scope)
        {
            return themed;
        }
        self.document_theme(document)
    }

    /// Effective theme for `id` (O(1): own/inherited `ThemeScope`, else document
    /// root theme, else world theme). Falls back to world theme if `id` is not
    /// committed yet.
    pub fn node_theme(&self, id: StableNodeId) -> &CompiledTheme {
        let Some(record) = self.nodes.get(id) else {
            return &self.theme;
        };
        let scope = self.nodes.node_scope(id);
        self.theme_for_scope(record.document, scope)
    }

    /// Arc clone of `node_theme(id)`.
    pub fn node_theme_arc(&self, id: StableNodeId) -> Arc<CompiledTheme> {
        let Some(record) = self.nodes.get(id) else {
            return Arc::clone(&self.theme);
        };
        let scope = self.nodes.node_scope(id);
        self.theme_arc_for_scope(record.document, scope)
    }

    pub(crate) fn node_chrome_radii(&self, id: StableNodeId) -> ChromeRadii {
        self.node_theme(id).chrome_radii()
    }

    /// Effective style model for `(document, scope)` overlaid with optional
    /// `color_overrides`.
    pub(crate) fn style_model_for_scope(
        &self,
        document: DocumentId,
        scope: ThemeScopeId,
        color_overrides: Option<&ColorOverrides>,
    ) -> StyleModelRef {
        let base = if scope.is_root() && !self.theme_state.document_themes.contains_key(&document) {
            self.style_model
        } else {
            self.theme_for_scope(document, scope).style_model()
        };
        match color_overrides {
            Some(overrides) => base.with_color_overrides(overrides),
            None => base,
        }
    }

    /// Effective style model for `id` (scoped `ThemePatch` + inherited `color_overrides`).
    pub fn node_style_model(&self, id: StableNodeId) -> StyleModelRef {
        let Some(record) = self.nodes.get(id) else {
            return self.style_model;
        };
        let scope = self.nodes.node_scope(id);
        self.style_model_for_scope(
            record.document,
            scope,
            record.resolved.0.color_scope.as_deref(),
        )
    }

    pub(crate) fn node_style_model_with(
        &self,
        id: StableNodeId,
        color_overrides: Option<&ColorOverrides>,
    ) -> StyleModelRef {
        let Some(record) = self.nodes.get(id) else {
            return self.scoped_style_model(color_overrides);
        };
        let scope = self.nodes.node_scope(id);
        self.style_model_for_scope(record.document, scope, color_overrides)
    }

    /// Structural nearest `ThemeScopeId` at or above `id`.
    pub(crate) fn structural_scope_for(&self, id: StableNodeId) -> ThemeScopeId {
        if self.theme_state.theme_scopes.is_empty() {
            return ThemeScopeId::ROOT;
        }
        let mut cursor = Some(id);
        while let Some(node) = cursor {
            if let Some(scope) = self.theme_state.theme_scopes.scope_for_owner(node) {
                return scope;
            }
            cursor = self.nodes.get(node).and_then(|r| r.hierarchy.parent);
        }
        ThemeScopeId::ROOT
    }

    /// Sync `id`'s `ThemeScope` ownership against its `NodeStyle::theme_patch`.
    pub(crate) fn sync_node_theme_scope(&mut self, id: StableNodeId) {
        let Some(record) = self.nodes.get(id) else {
            return;
        };
        let document = record.document;
        let parent = record.hierarchy.parent;
        let patch = record.style.theme_patch.clone();
        let prev_scope = self.nodes.node_scope(id);
        let parent_scope = parent
            .map(|p| self.structural_scope_for(p))
            .unwrap_or(ThemeScopeId::ROOT);
        let parent_theme = self.theme_arc_for_scope(document, parent_scope);

        let mut work = ThemeWorkCounters::default();
        match patch {
            Some(patch) if !patch.is_empty() => {
                let prev_entry_theme = self
                    .theme_state
                    .theme_scopes
                    .scope_for_owner(id)
                    .and_then(|scope| self.theme_state.theme_scopes.theme(scope))
                    .map(Arc::clone)
                    .unwrap_or_else(|| self.theme_arc_for_scope(document, prev_scope));
                let (scope_id, next_theme, scope_generation, _) = self
                    .theme_state
                    .theme_scopes
                    .upsert_scope(id, document, parent_scope, &parent_theme, patch, &mut work);
                let changed = ThemeCategoryGenerations::diff_themes(&prev_entry_theme, &next_theme);
                self.nodes.set_node_scope(id, scope_id);
                if changed.is_empty() {
                    if *prev_entry_theme != *next_theme {
                        self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);
                    }
                    self.rebind_descendant_scopes(id, scope_id, &next_theme, false, &mut work);
                    work.theme_scopes_active = self.theme_state.theme_scopes.len();
                    self.record_scope_pending_work(work);
                    return;
                }
                if changed == ThemeDependencyMask::MOTION_POLICY {
                    self.rebind_descendant_scopes(id, scope_id, &next_theme, false, &mut work);
                    work.style_motion_policy_invalidations = 1;
                    work.theme_scopes_active = self.theme_state.theme_scopes.len();
                    self.record_theme_invalidation(
                        crate::world::theme_inspect::ThemeInvalidationTrigger::ScopePatch {
                            owner: id,
                            scope_id,
                            generation: scope_generation,
                        },
                        changed,
                        &work,
                        0,
                        true,
                    );
                    self.record_scope_pending_work(work);
                    return;
                }
                self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);
                let seeds = self.propagate_theme_delta_on_roots(
                    &[id],
                    scope_id,
                    &prev_entry_theme,
                    &next_theme,
                    changed,
                    true,
                    true,
                    true,
                    &mut work,
                );
                work.theme_scopes_active = self.theme_state.theme_scopes.len();
                self.record_theme_invalidation(
                    crate::world::theme_inspect::ThemeInvalidationTrigger::ScopePatch {
                        owner: id,
                        scope_id,
                        generation: scope_generation,
                    },
                    changed,
                    &work,
                    seeds,
                    false,
                );
                self.record_scope_pending_work(work);
            }
            _ => {
                let Some(removed) = self.theme_state.theme_scopes.remove_owner(id) else {
                    return;
                };
                let prev_theme = removed.resolved;
                let changed = ThemeCategoryGenerations::diff_themes(&prev_theme, &parent_theme);
                self.nodes.set_node_scope(id, parent_scope);
                if changed.is_empty() {
                    self.rebind_descendant_scopes(
                        id,
                        parent_scope,
                        &parent_theme,
                        false,
                        &mut work,
                    );
                    work.theme_scopes_active = self.theme_state.theme_scopes.len();
                    self.record_scope_pending_work(work);
                    return;
                }
                if changed == ThemeDependencyMask::MOTION_POLICY {
                    self.rebind_descendant_scopes(
                        id,
                        parent_scope,
                        &parent_theme,
                        false,
                        &mut work,
                    );
                    work.style_motion_policy_invalidations = 1;
                    work.theme_scopes_active = self.theme_state.theme_scopes.len();
                    self.record_theme_invalidation(
                        crate::world::theme_inspect::ThemeInvalidationTrigger::ScopeRemoved {
                            owner: id,
                            parent_scope,
                        },
                        changed,
                        &work,
                        0,
                        true,
                    );
                    self.record_scope_pending_work(work);
                    return;
                }
                self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);
                let seeds = self.propagate_theme_delta_on_roots(
                    &[id],
                    parent_scope,
                    &prev_theme,
                    &parent_theme,
                    changed,
                    true,
                    true,
                    true,
                    &mut work,
                );
                work.theme_scopes_active = self.theme_state.theme_scopes.len();
                self.record_theme_invalidation(
                    crate::world::theme_inspect::ThemeInvalidationTrigger::ScopeRemoved {
                        owner: id,
                        parent_scope,
                    },
                    changed,
                    &work,
                    seeds,
                    false,
                );
                self.record_scope_pending_work(work);
            }
        }
    }

    /// Update scopes and invalidate descendants when `child` moves from
    /// `old_parent` to its current parent (or detaches to `None`).
    pub(crate) fn theme_scope_reparented(
        &mut self,
        child: StableNodeId,
        old_parent: Option<StableNodeId>,
    ) {
        if self.theme_state.theme_scopes.is_empty() && self.theme_state.document_themes.is_empty() {
            return;
        }
        let Some(document) = self.document_of(child) else {
            return;
        };
        let new_parent_scope = self
            .parent_id(child)
            .map(|p| self.structural_scope_for(p))
            .unwrap_or(ThemeScopeId::ROOT);
        let prev_parent_scope = match old_parent {
            Some(old) => self.structural_scope_for(old),
            None => match self.theme_state.theme_scopes.scope_for_owner(child) {
                Some(own) => self.theme_state.theme_scopes.parent_scope_of(own),
                None => self.nodes.node_scope(child),
            },
        };
        if new_parent_scope == prev_parent_scope {
            return;
        }
        let old_parent_theme = self.theme_arc_for_scope(document, prev_parent_scope);
        let new_parent_theme = self.theme_arc_for_scope(document, new_parent_scope);
        let changed = ThemeCategoryGenerations::diff_themes(&old_parent_theme, &new_parent_theme);
        let mut work = ThemeWorkCounters::default();
        if !changed.is_empty() && changed != ThemeDependencyMask::MOTION_POLICY {
            self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);
        }
        let seeds = self.propagate_theme_delta_on_roots(
            &[child],
            new_parent_scope,
            &old_parent_theme,
            &new_parent_theme,
            changed,
            false,
            true,
            true,
            &mut work,
        );
        work.theme_scopes_active = self.theme_state.theme_scopes.len();
        if !changed.is_empty() {
            self.record_theme_invalidation(
                crate::world::theme_inspect::ThemeInvalidationTrigger::ScopeReparented {
                    root: child,
                    old_scope: prev_parent_scope,
                    new_scope: new_parent_scope,
                },
                changed,
                &work,
                seeds,
                changed == ThemeDependencyMask::MOTION_POLICY,
            );
        }
        self.record_scope_pending_work(work);
    }

    /// Install or clear a per-document root theme override (independent of other documents).
    pub fn set_document_theme(
        &mut self,
        document: DocumentId,
        theme: Option<Arc<CompiledTheme>>,
    ) -> ThemeDependencyMask {
        let old_effective = Arc::clone(self.document_theme(document));
        let next_effective = match theme {
            Some(installed) => {
                let effective = self.contrast_rendition_for(&installed);
                self.theme_state
                    .installed_document_themes
                    .insert(document, installed);
                self.theme_state
                    .document_themes
                    .insert(document, Arc::clone(&effective));
                effective
            }
            None => {
                self.theme_state.installed_document_themes.remove(&document);
                self.theme_state.document_themes.remove(&document);
                Arc::clone(&self.theme)
            }
        };
        if *old_effective == *next_effective {
            return ThemeDependencyMask::NONE;
        }
        let changed = ThemeCategoryGenerations::diff_themes(&old_effective, &next_effective);
        if changed.is_empty() {
            return ThemeDependencyMask::NONE;
        }
        self.theme_state.theme_generations.advance(changed);
        if changed == ThemeDependencyMask::MOTION_POLICY {
            let mut work = ThemeWorkCounters::default();
            self.refresh_document_scopes(document, &next_effective, &mut work);
            work.style_motion_policy_invalidations = 1;
            work.theme_scopes_active = self.theme_state.theme_scopes.len();
            let doc_identity = next_effective.identity();
            self.record_theme_invalidation(
                crate::world::theme_inspect::ThemeInvalidationTrigger::DocumentTheme {
                    document,
                    theme_id: doc_identity.id,
                    generation: doc_identity.generation,
                },
                changed,
                &work,
                0,
                true,
            );
            self.record_theme_work(work);
            return changed;
        }
        self.palette_epoch = self.palette_epoch.wrapping_add(1).max(1);
        let hover_ids: Vec<StableNodeId> = self
            .hover_transitions
            .keys()
            .copied()
            .filter(|&id| self.contains(id) && self.record(id).document == document)
            .collect();
        for id in hover_ids {
            self.cancel_hover_transition(id);
        }
        let roots: Vec<StableNodeId> = self
            .live_document_roots
            .get(&document)
            .map(|r| r.iter().copied().collect())
            .unwrap_or_default();
        let mut work = ThemeWorkCounters::default();
        let seeds = if !roots.is_empty() {
            self.propagate_theme_delta_on_roots(
                &roots,
                ThemeScopeId::ROOT,
                &old_effective,
                &next_effective,
                changed,
                false,
                false,
                true,
                &mut work,
            )
        } else {
            0
        };
        // Keep any parked/detached scopes belonging to `document` aligned with
        // the new document root theme.
        let unvisited_doc_scopes = self.theme_state.theme_scopes.scopes_in_document(document);
        for scope_id in unvisited_doc_scopes {
            if let Some(entry) = self.theme_state.theme_scopes.entry(scope_id)
                && entry.parent_scope.is_root()
                && *entry.resolved != *next_effective
            {
                let owner = entry.owner;
                if !roots.iter().any(|&r| self.is_descendant_or_self(owner, r)) {
                    let _ = self.theme_state.theme_scopes.refresh_scope(
                        scope_id,
                        ThemeScopeId::ROOT,
                        &next_effective,
                        &mut work,
                    );
                }
            }
        }
        work.theme_scopes_active = self.theme_state.theme_scopes.len();
        let doc_identity = next_effective.identity();
        self.record_theme_invalidation(
            crate::world::theme_inspect::ThemeInvalidationTrigger::DocumentTheme {
                document,
                theme_id: doc_identity.id,
                generation: doc_identity.generation,
            },
            changed,
            &work,
            seeds,
            false,
        );
        self.record_theme_work(work);
        changed
    }

    /// Re-apply root theme changes only to documents not overridden by
    /// `document_themes`, refreshing any `ThemeScope`s along the way.
    pub(super) fn apply_root_theme_with_scopes(
        &mut self,
        old_root: &Arc<CompiledTheme>,
        next_root: &Arc<CompiledTheme>,
        changed: ThemeDependencyMask,
    ) {
        if changed == ThemeDependencyMask::MOTION_POLICY {
            let mut work = ThemeWorkCounters::default();
            let mut docs: Vec<DocumentId> = self
                .live_document_roots
                .keys()
                .copied()
                .filter(|doc| !self.theme_state.document_themes.contains_key(doc))
                .collect();
            for doc in self.theme_state.theme_scopes.documents() {
                if !self.theme_state.document_themes.contains_key(&doc) && !docs.contains(&doc) {
                    docs.push(doc);
                }
            }
            for &doc in &docs {
                self.refresh_document_scopes(doc, next_root, &mut work);
            }
            if !docs.is_empty() || self.live_document_roots.is_empty() {
                work.style_motion_policy_invalidations = 1;
            }
            work.theme_scopes_active = self.theme_state.theme_scopes.len();
            let root_identity = next_root.identity();
            self.record_theme_invalidation(
                crate::world::theme_inspect::ThemeInvalidationTrigger::RootTheme {
                    theme_id: root_identity.id,
                    generation: root_identity.generation,
                },
                changed,
                &work,
                0,
                true,
            );
            self.record_theme_work(work);
            return;
        }
        let roots: Vec<StableNodeId> = self
            .live_document_roots
            .iter()
            .filter(|(doc, _)| !self.theme_state.document_themes.contains_key(doc))
            .flat_map(|(_, r)| r.iter().copied())
            .collect();
        let mut work = ThemeWorkCounters::default();
        let seeds = if !roots.is_empty() {
            self.propagate_theme_delta_on_roots(
                &roots,
                ThemeScopeId::ROOT,
                old_root,
                next_root,
                changed,
                false,
                false,
                false,
                &mut work,
            )
        } else {
            0
        };
        work.theme_scopes_active = self.theme_state.theme_scopes.len();
        let root_identity = next_root.identity();
        self.record_theme_invalidation(
            crate::world::theme_inspect::ThemeInvalidationTrigger::RootTheme {
                theme_id: root_identity.id,
                generation: root_identity.generation,
            },
            changed,
            &work,
            seeds,
            false,
        );
        self.record_theme_work(work);
    }

    fn refresh_document_scopes(
        &mut self,
        document: DocumentId,
        doc_theme: &Arc<CompiledTheme>,
        work: &mut ThemeWorkCounters,
    ) {
        if self.theme_state.theme_scopes.is_empty() {
            return;
        }
        let doc_scopes = self.theme_state.theme_scopes.scopes_in_document(document);
        if doc_scopes.is_empty() {
            return;
        }
        let mut resolved_parents: std::collections::HashMap<ThemeScopeId, Arc<CompiledTheme>> =
            std::collections::HashMap::with_capacity(doc_scopes.len() + 1);
        resolved_parents.insert(ThemeScopeId::ROOT, Arc::clone(doc_theme));
        let mut remaining = doc_scopes;
        while !remaining.is_empty() {
            let before = remaining.len();
            let mut next_remaining = Vec::new();
            for scope_id in remaining {
                let Some(parent_scope) = self
                    .theme_state
                    .theme_scopes
                    .entry(scope_id)
                    .map(|entry| entry.parent_scope)
                else {
                    continue;
                };
                if let Some(parent_theme) = resolved_parents.get(&parent_scope).cloned() {
                    if let Some((_, next_resolved, _)) = self
                        .theme_state
                        .theme_scopes
                        .refresh_scope(scope_id, parent_scope, &parent_theme, work)
                    {
                        resolved_parents.insert(scope_id, next_resolved);
                    }
                } else {
                    next_remaining.push(scope_id);
                }
            }
            if next_remaining.len() == before {
                for scope_id in next_remaining {
                    if let Some(parent_scope) = self
                        .theme_state
                        .theme_scopes
                        .entry(scope_id)
                        .map(|entry| entry.parent_scope)
                    {
                        let _ = self.theme_state.theme_scopes.refresh_scope(
                            scope_id,
                            parent_scope,
                            doc_theme,
                            work,
                        );
                    }
                }
                break;
            }
            remaining = next_remaining;
        }
    }

    fn rebind_descendant_scopes(
        &mut self,
        id: StableNodeId,
        incoming_scope: ThemeScopeId,
        incoming_theme: &Arc<CompiledTheme>,
        include_self: bool,
        work: &mut ThemeWorkCounters,
    ) {
        if !self.contains(id) {
            return;
        }
        let (eff_scope, eff_theme) = if !include_self {
            (incoming_scope, Arc::clone(incoming_theme))
        } else if let Some(own_scope) = self.theme_state.theme_scopes.scope_for_owner(id) {
            if let Some((_, next_resolved, _)) = self.theme_state.theme_scopes.refresh_scope(
                own_scope,
                incoming_scope,
                incoming_theme,
                work,
            ) {
                self.nodes.set_node_scope(id, own_scope);
                (own_scope, next_resolved)
            } else {
                self.nodes.set_node_scope(id, incoming_scope);
                (incoming_scope, Arc::clone(incoming_theme))
            }
        } else {
            self.nodes.set_node_scope(id, incoming_scope);
            (incoming_scope, Arc::clone(incoming_theme))
        };
        let children = Arc::clone(&self.record(id).hierarchy.children);
        for &child in children.iter() {
            self.rebind_descendant_scopes(child, eff_scope, &eff_theme, true, work);
        }
    }

    fn record_scope_pending_work(&mut self, work: ThemeWorkCounters) {
        self.theme_state.pending_scope_theme_work.accumulate(work);
    }

    #[allow(clippy::too_many_arguments)]
    fn propagate_theme_delta_on_roots(
        &mut self,
        roots: &[StableNodeId],
        incoming_scope: ThemeScopeId,
        old_parent_theme: &Arc<CompiledTheme>,
        new_parent_theme: &Arc<CompiledTheme>,
        parent_changed: ThemeDependencyMask,
        root_scope_already_updated: bool,
        count_as_scope: bool,
        queue_reprojects: bool,
        work: &mut ThemeWorkCounters,
    ) -> usize {
        let seeds_before = self.layout_seeds_created;
        let mut modal_frames = Vec::new();
        let mut part_readers = Vec::new();
        let mut intent_candidates = Vec::new();
        let mut default_roots = Vec::new();
        let mut touched_nodes = 0usize;
        let mut any_intent_relevant = false;

        for &root in roots {
            self.visit_theme_subtree(
                root,
                root_scope_already_updated,
                incoming_scope,
                old_parent_theme,
                new_parent_theme,
                parent_changed,
                count_as_scope,
                queue_reprojects,
                work,
                &mut touched_nodes,
                &mut any_intent_relevant,
                &mut modal_frames,
                &mut part_readers,
                &mut intent_candidates,
                &mut default_roots,
            );
        }

        if touched_nodes > 0 {
            work.record_allocation(
                1,
                touched_nodes.saturating_mul(std::mem::size_of::<StableNodeId>()),
            );
        }
        let mut dependents = 0;
        if any_intent_relevant {
            dependents = self.reresolve_layout_intent_for(&intent_candidates);
            for root in default_roots {
                self.mark(root, DirtyMask::STYLE | DirtyMask::TEXT);
                let classified =
                    self.classify_layout_change(root, nana_ui_core::LayoutStyleChange::FONT);
                self.record_layout_invalidation(
                    root,
                    nana_ui_core::LayoutInvalidation {
                        source: nana_ui_core::LayoutInvalidationSource::Resource,
                        reason: nana_ui_core::InvalidationReason::RESOURCE,
                        ..classified
                    },
                );
                dependents += 1;
            }
            work.record_layout_invalidation(dependents);
        }
        work.record_layout_invalidation(modal_frames.len() + part_readers.len());
        for id in modal_frames {
            self.nodes
                .invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
            self.mark(id, DirtyMask::TEXT);
            self.record_layout_invalidation(
                id,
                nana_ui_core::LayoutInvalidation::new(
                    nana_ui_core::LayoutInvalidationSource::Resource,
                    nana_ui_core::InvalidationReason::RESOURCE,
                    nana_ui_core::InvalidationKind::MEASURE
                        .union(nana_ui_core::InvalidationKind::PLACEMENT),
                    nana_ui_core::LayoutFieldMask::ALL,
                    nana_ui_core::LayoutDependencyFootprint::ALL,
                ),
            );
        }
        for id in part_readers {
            if self.text(id).is_some_and(|text| !text.is_empty()) {
                self.nodes
                    .invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
                self.mark(id, DirtyMask::TEXT);
            }
            self.record_layout_invalidation(
                id,
                nana_ui_core::LayoutInvalidation::new(
                    nana_ui_core::LayoutInvalidationSource::Resource,
                    nana_ui_core::InvalidationReason::RESOURCE,
                    nana_ui_core::InvalidationKind::MEASURE
                        .union(nana_ui_core::InvalidationKind::PLACEMENT),
                    nana_ui_core::LayoutFieldMask::INTRINSIC
                        .union(nana_ui_core::LayoutFieldMask::FLOW),
                    nana_ui_core::LayoutDependencyFootprint::intrinsic_container(),
                ),
            );
        }
        let seeds = (self.layout_seeds_created - seeds_before) as usize;
        let counts = &mut self.counters.pending_drain_counts;
        if any_intent_relevant {
            counts.theme_to_layout_seeds += seeds;
            counts.theme_metric_dependents_invalidated += dependents;
        } else {
            counts.theme_palette_layout_invalidations += seeds;
        }
        seeds
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_theme_subtree(
        &mut self,
        id: StableNodeId,
        skip_scope_refresh: bool,
        incoming_scope: ThemeScopeId,
        old_incoming_theme: &Arc<CompiledTheme>,
        new_incoming_theme: &Arc<CompiledTheme>,
        incoming_changed: ThemeDependencyMask,
        count_as_scope: bool,
        queue_reprojects: bool,
        work: &mut ThemeWorkCounters,
        touched_nodes: &mut usize,
        any_intent_relevant: &mut bool,
        modal_frames: &mut Vec<StableNodeId>,
        part_readers: &mut Vec<StableNodeId>,
        intent_candidates: &mut Vec<StableNodeId>,
        default_roots: &mut Vec<StableNodeId>,
    ) {
        if !self.contains(id) {
            return;
        }
        let (eff_scope, old_theme, new_theme, changed) = if skip_scope_refresh {
            let scope = self.nodes.node_scope(id);
            (
                scope,
                Arc::clone(old_incoming_theme),
                Arc::clone(new_incoming_theme),
                incoming_changed,
            )
        } else if let Some(own_scope) = self.theme_state.theme_scopes.scope_for_owner(id) {
            if let Some((old_resolved, new_resolved, _)) = self
                .theme_state
                .theme_scopes
                .refresh_scope(own_scope, incoming_scope, new_incoming_theme, work)
            {
                let own_changed =
                    ThemeCategoryGenerations::diff_themes(&old_resolved, &new_resolved);
                (own_scope, old_resolved, new_resolved, own_changed)
            } else {
                (
                    incoming_scope,
                    Arc::clone(old_incoming_theme),
                    Arc::clone(new_incoming_theme),
                    incoming_changed,
                )
            }
        } else {
            (
                incoming_scope,
                Arc::clone(old_incoming_theme),
                Arc::clone(new_incoming_theme),
                incoming_changed,
            )
        };

        self.nodes.set_node_scope(id, eff_scope);

        if changed == ThemeDependencyMask::MOTION_POLICY {
            if work.style_motion_policy_invalidations == 0 {
                work.style_motion_policy_invalidations = 1;
            }
        } else if !changed.is_empty() {
            *touched_nodes += 1;
            let dep = self.nodes.theme_dependency(id);
            if dep.intersects(changed) {
                let hit = dep.intersection(changed);
                work.theme_revision_candidates += 1;
                if count_as_scope {
                    work.scope_nodes_invalidated += 1;
                }
                if hit.intersects(ThemeDependencyMask::RECIPE_ANY) {
                    work.component_recipe_nodes_invalidated += 1;
                }
                self.theme_state
                    .pending_theme_eval
                    .entry(id)
                    .and_modify(|prev| *prev = prev.union(hit))
                    .or_insert(hit);
                self.mark(id, DirtyMask::STYLE);
            } else {
                work.theme_dependency_skipped += 1;
            }

            if self.paint_recordings.get_mut().contains_key(&id) && dep.intersects(changed) {
                self.mark_repaint(id);
            }
            if self.modal_surface_recordings.get_mut().contains_key(&id)
                && changed.intersects(
                    ThemeDependencyMask::PALETTE
                        .union(ThemeDependencyMask::ACCENT)
                        .union(ThemeDependencyMask::EFFECT)
                        .union(ThemeDependencyMask::RECIPE_DIALOG)
                        .union(ThemeDependencyMask::METRICS)
                        .union(ThemeDependencyMask::RECIPE_SHAPE),
                )
            {
                self.mark_repaint(id);
            }

            let metrics_changed = !old_theme.metrics().same_layout(&new_theme.metrics())
                || old_theme.typography() != new_theme.typography()
                || old_theme.spacing() != new_theme.spacing();
            let token_layout_changed = old_theme.spacing() != new_theme.spacing()
                || old_theme.typography() != new_theme.typography()
                || old_theme.border() != new_theme.border()
                || old_theme.metrics() != new_theme.metrics();
            let dialog_changed =
                changed.intersects(ThemeDependencyMask::RECIPE_DIALOG) || token_layout_changed;
            let toast_changed = changed.intersects(ThemeDependencyMask::RECIPE_TOAST);
            let shape_changed = changed.intersects(ThemeDependencyMask::RECIPE_SHAPE);
            let menu_changed = changed.intersects(ThemeDependencyMask::RECIPE_MENU);
            let intent_relevant = metrics_changed
                || token_layout_changed
                || shape_changed
                || menu_changed
                || changed.intersects(ThemeDependencyMask::TYPOGRAPHY_ANY);

            if dialog_changed
                && (self
                    .widget_visual_as::<crate::ModalFrameVisual>(id)
                    .is_some()
                    || self
                        .widget_visual_as::<crate::EmptyStateVisual>(id)
                        .is_some())
            {
                modal_frames.push(id);
            }
            if token_layout_changed || toast_changed {
                let widget_visual = self.nodes.widget_visual(id);
                if (token_layout_changed
                    && widget_visual.is_some_and(|w| w.visual_parts_read_theme()))
                    || (toast_changed && widget_visual.is_some_and(|w| w.reads_toast_recipe()))
                {
                    part_readers.push(id);
                }
            }
            if intent_relevant {
                *any_intent_relevant = true;
                if self.intent_nodes.contains(&id) {
                    intent_candidates.push(id);
                }
                let body_changed = old_theme.typography().body != new_theme.typography().body;
                let regular_changed =
                    old_theme.typography().regular != new_theme.typography().regular;
                let line_ratio_changed = old_theme.typography().line / old_theme.typography().body
                    != new_theme.typography().line / new_theme.typography().body;
                let record = self.record(id);
                if record.hierarchy.parent.is_none() {
                    let layout = &record.resolved_layout;
                    if (body_changed && layout.font_size.is_none())
                        || (regular_changed && layout.font_weight.is_none())
                        || (line_ratio_changed && layout.line_height.is_none())
                    {
                        default_roots.push(id);
                    }
                }
            }
            if queue_reprojects {
                let border_changed = old_theme.border() != new_theme.border();
                let reproject_mask = if border_changed {
                    changed.union(ThemeDependencyMask::RECIPE_ANY)
                } else {
                    changed
                };
                if reproject_mask.intersects(
                    ThemeDependencyMask::METRICS
                        .union(ThemeDependencyMask::TYPOGRAPHY_ANY)
                        .union(ThemeDependencyMask::RECIPE_ANY),
                ) && !matches!(self.record(id).kind.as_ref(), NodeKind::Text)
                {
                    self.theme_state
                        .pending_scope_reprojects
                        .push((id, reproject_mask));
                }
            }
        }

        let children = Arc::clone(&self.record(id).hierarchy.children);
        for &child in children.iter() {
            self.visit_theme_subtree(
                child,
                false,
                eff_scope,
                &old_theme,
                &new_theme,
                changed,
                count_as_scope,
                queue_reprojects,
                work,
                touched_nodes,
                any_intent_relevant,
                modal_frames,
                part_readers,
                intent_candidates,
                default_roots,
            );
        }
    }
}
