//! Localized text and locale scopes (Issues #268, #269).
//!
//! Every localized node is indexed under the scope it reads its locale
//! from: the nearest subtree scope at or above it, else its window. A
//! locale change visits the nodes indexed under the scopes it reached and no
//! other node: not the literal text, not the windows it did not reach.
//!
//! A switch is one transaction. Each node it reaches resolves its message
//! in the new locale and shows the result only when it differs, so a
//! translation equal to the last one does no text or layout work; its
//! shaping language moves only when the locale it resolved in does; the
//! scope's root takes the new direction only when it changed. All of it
//! lands in the world before the frame reads it: no frame shows half a
//! switch.

use super::*;
use crate::i18n::message::{CompiledMessage, FormatIssue};
use crate::i18n::{
    Locale, LocaleFormatter, LocalizedText, MessageCatalog, MessageId, MissingMessage,
    default_formatter, fallback_chain,
};
use nana_text::font::LanguageTag;
use nana_ui_core::DirSpec;

/// The scope a localized node reads its locale from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocaleScope {
    /// A subtree scope: the node a locale was set on.
    Node(StableNodeId),
    /// A window: its own locale, else the application's.
    Document(DocumentId),
}

/// How many times each part of a scope's locale changed: its messages, the
/// language its text shapes in, its direction, its formatting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocaleGenerations {
    pub messages: u64,
    pub language: u64,
    pub direction: u64,
    pub formatting: u64,
}

struct Localized {
    text: LocalizedText,
    scope: LocaleScope,
    /// The locale the shown string came from, after fallback.
    resolved: Option<LanguageTag>,
}

#[derive(Default)]
pub(super) struct I18nIndex {
    catalog: Option<Arc<dyn MessageCatalog>>,
    /// Bumped each time a catalog is installed or updated.
    catalog_generation: u64,
    /// What messages format through; the default until one is installed.
    formatter: Option<Arc<dyn LocaleFormatter>>,
    /// The locale every chain ends in.
    fallback: Option<LanguageTag>,
    missing: MissingMessage,
    application: Option<Locale>,
    documents: HashMap<DocumentId, Locale>,
    scopes: HashMap<StableNodeId, Locale, BuildIdHasher>,
    /// Subtree scopes per window. A window with none has its localized nodes
    /// read the window, found without a walk.
    scoped_documents: HashMap<DocumentId, usize>,
    nodes: HashMap<StableNodeId, Localized, BuildIdHasher>,
    dependents: HashMap<LocaleScope, HashSet<StableNodeId, BuildIdHasher>>,
    /// Localized nodes by the message they say: what a catalog update of
    /// one message reaches.
    message_dependents: HashMap<MessageId, HashSet<StableNodeId, BuildIdHasher>>,
    /// Messages found, by locale asked and message: the locale each came
    /// from, compiled once. Emptied when the catalog or the fallback changes.
    lookups: HashMap<(LanguageTag, MessageId), Option<(LanguageTag, Arc<CompiledMessage>)>>,
    /// The output buffer formatting reuses.
    scratch: String,
    generations: HashMap<LocaleScope, LocaleGenerations>,
    /// Virtual lists: localized text resolved in the rows under one is a
    /// virtual row resolved.
    virtual_lists: HashSet<StableNodeId, BuildIdHasher>,
}

impl I18nIndex {
    /// Stop indexing localized node `id`: what it said, if it was one.
    fn forget(&mut self, id: StableNodeId) -> Option<Localized> {
        let entry = self.nodes.remove(&id)?;
        unlink(&mut self.dependents, entry.scope, id);
        unlink(&mut self.message_dependents, entry.text.message, id);
        Some(entry)
    }

    /// `id` stops being a locale scope of `document`: whether it was one.
    fn drop_scope(&mut self, id: StableNodeId, document: DocumentId) -> bool {
        if self.scopes.remove(&id).is_none() {
            return false;
        }
        if let Some(count) = self.scoped_documents.get_mut(&document) {
            *count -= 1;
            if *count == 0 {
                self.scoped_documents.remove(&document);
            }
        }
        self.generations.remove(&LocaleScope::Node(id));
        true
    }

    /// Entries held: localized nodes, their scope links, scopes and cached
    /// lookups. They follow what is registered, not what happened.
    #[cfg(test)]
    pub(super) fn footprint(&self) -> (usize, usize, usize, usize) {
        (
            self.nodes.len(),
            self.dependents.values().map(HashSet::len).sum(),
            self.scopes.len(),
            self.lookups.len(),
        )
    }
}

/// Index `id` under `key`.
fn link<K: std::hash::Hash + Eq>(index: &mut HashMap<K, NodeSet>, key: K, id: StableNodeId) {
    index.entry(key).or_default().insert(id);
}

/// Take `id` from under `key`, and `key` once nothing is under it.
fn unlink<K: std::hash::Hash + Eq>(index: &mut HashMap<K, NodeSet>, key: K, id: StableNodeId) {
    if let Some(nodes) = index.get_mut(&key) {
        nodes.remove(&id);
        if nodes.is_empty() {
            index.remove(&key);
        }
    }
}

impl UiWorld {
    // --- what the application sets ------------------------------------

    /// Install the catalog messages come from; `None` leaves only the
    /// missing-message policy. Every localized node resolves again.
    pub fn set_message_catalog(&mut self, catalog: Option<Arc<dyn MessageCatalog>>) {
        self.i18n.catalog = catalog;
        self.i18n.catalog_generation += 1;
        self.i18n.lookups.clear();
        let changed = self.resolve_every_localized();
        self.note_i18n_generation(changed);
    }

    /// Install `catalog`, in which only `changed` messages differ from the
    /// catalog it replaces: only the nodes saying those messages resolve
    /// again, found through an index, not a scan (a hot reload of one
    /// message, a downloaded translation of a few).
    pub fn update_message_catalog(
        &mut self,
        catalog: Arc<dyn MessageCatalog>,
        changed: &[MessageId],
    ) {
        self.i18n.catalog = Some(catalog);
        self.i18n.catalog_generation += 1;
        self.i18n
            .lookups
            .retain(|(_, message), _| !changed.contains(message));
        let mut ids: Vec<StableNodeId> = changed
            .iter()
            .filter_map(|message| self.i18n.message_dependents.get(message))
            .flat_map(|nodes| nodes.iter().copied())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let mut any_changed = false;
        for id in ids {
            any_changed |= self.resolve_localized(id, self.localized_language(id));
        }
        self.note_i18n_generation(any_changed);
    }

    /// How many catalogs were installed or updated.
    pub fn catalog_generation(&self) -> u64 {
        self.i18n.catalog_generation
    }

    /// Format messages through `formatter` instead of the default; every
    /// localized node resolves again.
    pub fn set_locale_formatter(&mut self, formatter: Arc<dyn LocaleFormatter>) {
        self.i18n.formatter = Some(formatter);
        let changed = self.resolve_every_localized();
        self.note_i18n_generation(changed);
    }

    /// The locale every fallback chain ends in, after the scope's own; the
    /// default is none.
    pub fn set_fallback_locale(&mut self, locale: Option<LanguageTag>) {
        if self.i18n.fallback == locale {
            return;
        }
        self.i18n.fallback = locale;
        self.i18n.lookups.clear();
        let changed = self.resolve_every_localized();
        self.note_i18n_generation(changed);
    }

    /// What a message no locale has shows; see [`MissingMessage`].
    pub fn set_missing_message(&mut self, policy: MissingMessage) {
        if self.i18n.missing == policy {
            return;
        }
        self.i18n.missing = policy;
        let changed = self.resolve_every_localized();
        self.note_i18n_generation(changed);
    }

    /// The application's locale, which every window without its own takes.
    pub fn set_default_locale(&mut self, locale: Option<Locale>) {
        if self.i18n.application == locale {
            return;
        }
        let before = self.i18n.application.clone();
        self.i18n.application = locale;
        // Windows with localized text, and windows whose roots take its
        // direction.
        let mut documents: Vec<DocumentId> = self
            .i18n
            .dependents
            .keys()
            .filter_map(|scope| match scope {
                LocaleScope::Document(document) => Some(*document),
                LocaleScope::Node(_) => None,
            })
            .chain(self.live_document_roots.keys().copied())
            .collect();
        documents.sort_unstable();
        documents.dedup();
        let reached = documents
            .into_iter()
            .filter(|document| !self.i18n.documents.contains_key(document))
            .map(|document| (LocaleScope::Document(document), before.clone()))
            .collect();
        let changed = self.switch_locale(reached);
        self.note_i18n_generation(changed);
    }

    /// `document`'s locale; `None` takes the application's again.
    pub fn set_document_locale(&mut self, document: DocumentId, locale: Option<Locale>) {
        if self.i18n.documents.get(&document) == locale.as_ref() {
            return;
        }
        let before = self.scope_locale(LocaleScope::Document(document)).cloned();
        match locale {
            Some(locale) => self.i18n.documents.insert(document, locale),
            None => self.i18n.documents.remove(&document),
        };
        let changed = self.switch_locale(vec![(LocaleScope::Document(document), before)]);
        self.note_i18n_generation(changed);
    }

    /// A change the application made outside a commit that changed what
    /// shows moves the world's generation, as a commit does, so a host that
    /// draws when the generation moves draws it.
    fn note_i18n_generation(&mut self, changed: bool) {
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub fn default_locale(&self) -> Option<&Locale> {
        self.i18n.application.as_ref()
    }

    pub fn document_locale(&self, document: DocumentId) -> Option<&Locale> {
        self.i18n.documents.get(&document)
    }

    /// The localized text `id` shows, if it shows one.
    pub fn localized_text(&self, id: StableNodeId) -> Option<&LocalizedText> {
        self.i18n.nodes.get(&id).map(|entry| &entry.text)
    }

    /// The scope `id` reads its locale from.
    pub fn locale_scope(&self, id: StableNodeId) -> Option<LocaleScope> {
        self.contains(id).then(|| self.locale_scope_of(id))
    }

    /// The locale `scope` reads: its own, else its window's, else the
    /// application's.
    pub fn scope_locale(&self, scope: LocaleScope) -> Option<&Locale> {
        match scope {
            LocaleScope::Node(id) => self.i18n.scopes.get(&id),
            LocaleScope::Document(document) => self
                .i18n
                .documents
                .get(&document)
                .or(self.i18n.application.as_ref()),
        }
    }

    pub fn locale_generations(&self, scope: LocaleScope) -> LocaleGenerations {
        self.i18n
            .generations
            .get(&scope)
            .copied()
            .unwrap_or_default()
    }

    // --- what mutations set ---------------------------------------------

    /// `id` shows `text` resolved in its scope's locale; `None` makes what
    /// it shows literal again, as it is.
    pub(super) fn set_localized_text(&mut self, id: StableNodeId, text: Option<LocalizedText>) {
        let Some(text) = text else {
            // Its language followed its locale; it is its own again.
            if self
                .i18n
                .forget(id)
                .is_some_and(|previous| previous.resolved.is_some())
            {
                self.mark(id, DirtyMask::STYLE | DirtyMask::TEXT);
            }
            return;
        };
        match self.i18n.nodes.get_mut(&id) {
            Some(entry) => {
                let said = std::mem::replace(&mut entry.text, text).message;
                let says = entry.text.message;
                if said != says {
                    unlink(&mut self.i18n.message_dependents, said, id);
                    link(&mut self.i18n.message_dependents, says, id);
                }
            }
            None => {
                let scope = self.locale_scope_of(id);
                link(&mut self.i18n.dependents, scope, id);
                link(&mut self.i18n.message_dependents, text.message, id);
                self.i18n.nodes.insert(
                    id,
                    Localized {
                        text,
                        scope,
                        resolved: None,
                    },
                );
            }
        }
        let _ = self.resolve_localized(id, self.localized_language(id));
    }

    /// `id` becomes a locale scope for its subtree, takes another locale, or
    /// with `None` stops being one.
    pub(super) fn set_scope_locale(&mut self, id: StableNodeId, locale: Option<Locale>) {
        let before = self.i18n.scopes.get(&id).cloned();
        if before == locale {
            return;
        }
        let Some(document) = self.document_of(id) else {
            return;
        };
        let scope = LocaleScope::Node(id);
        let outer = self.locale_scope_above(id);
        match (&before, locale) {
            (None, Some(locale)) => {
                self.i18n.scopes.insert(id, locale);
                *self.i18n.scoped_documents.entry(document).or_default() += 1;
                // The localized nodes in the subtree read the new scope now.
                self.adopt_dependents(outer, scope, id);
                // The root lays out in the scope's direction.
                self.restyle(id, None);
            }
            (Some(_), None) => {
                // Its nodes read the scope above it again, while its locale
                // is still the one they read before.
                self.adopt_dependents(scope, outer, id);
                self.i18n.drop_scope(id, document);
                // The root's direction was the scope's.
                self.restyle(id, None);
            }
            (Some(_), Some(locale)) => {
                self.i18n.scopes.insert(id, locale);
                self.switch_locale(vec![(scope, before)]);
            }
            (None, None) => {}
        }
    }

    /// `child` moved from `old_parent`: the localized nodes in it that read
    /// a scope above it read the one above its new place.
    pub(super) fn i18n_reparented(
        &mut self,
        child: StableNodeId,
        old_parent: Option<StableNodeId>,
    ) {
        let Some(document) = self.document_of(child) else {
            return;
        };
        if !self.i18n.scoped_documents.contains_key(&document) || self.i18n.nodes.is_empty() {
            return;
        }
        if self.i18n.scopes.contains_key(&child) {
            // A scope moves with its nodes.
            return;
        }
        let old = self.scope_at(document, old_parent);
        let new = self.locale_scope_above(child);
        if old == new {
            return;
        }
        self.adopt_dependents(old, new, child);
    }

    /// `list` is a virtual list: localized text resolved in the rows it
    /// mounts counts as virtual rows resolved.
    pub(crate) fn note_virtual_list(&mut self, list: StableNodeId) {
        self.i18n.virtual_lists.insert(list);
    }

    /// Forget a node of `document` that is gone: as localized text, as a
    /// scope and as a virtual list.
    pub(super) fn forget_i18n(&mut self, id: StableNodeId, document: DocumentId) {
        self.i18n.forget(id);
        self.i18n.virtual_lists.remove(&id);
        if self.i18n.drop_scope(id, document) {
            // Its subtree is despawned with it, each node in it forgotten in
            // turn.
            self.i18n.dependents.remove(&LocaleScope::Node(id));
        }
    }

    // --- what the pipeline reads ----------------------------------------

    /// The language localized text `id` shapes in: its locale's named
    /// language, else the locale its message came from. `None` for literal
    /// text, which keeps the language it is in.
    pub(super) fn localized_language(&self, id: StableNodeId) -> Option<LanguageTag> {
        let entry = self.i18n.nodes.get(&id)?;
        self.scope_locale(entry.scope)
            .and_then(Locale::language)
            .cloned()
            .or_else(|| entry.resolved.clone())
    }

    /// The direction a locale lays `id` out in: a subtree scope's at its
    /// root; a right-to-left window's, or application's, at the window's
    /// roots. `None` where no locale sets one.
    pub(super) fn locale_direction(&self, id: StableNodeId) -> Option<DirSpec> {
        if let Some(locale) = self.i18n.scopes.get(&id) {
            return Some(locale.direction());
        }
        if !self.locale_sets_direction() {
            return None;
        }
        let document = self.document_of(id)?;
        if !self
            .live_document_roots
            .get(&document)
            .is_some_and(|roots| roots.contains(&id))
        {
            return None;
        }
        let locale = self.scope_locale(LocaleScope::Document(document))?;
        // A window starts left to right; only a right-to-left locale moves it.
        (locale.direction() == DirSpec::Rtl).then_some(DirSpec::Rtl)
    }

    /// Whether any window or the application has a locale, which a window's
    /// roots may take a direction from.
    pub(super) fn locale_sets_direction(&self) -> bool {
        !self.i18n.documents.is_empty() || self.i18n.application.is_some()
    }

    // --- resolution -----------------------------------------------------

    /// The scope `id` reads: the nearest subtree scope at or above it, else
    /// its window.
    fn locale_scope_of(&self, id: StableNodeId) -> LocaleScope {
        let document = self.document_of(id).expect("a live node has a document");
        if !self.i18n.scoped_documents.contains_key(&document) {
            return LocaleScope::Document(document);
        }
        self.scope_at(document, Some(id))
    }

    /// The scope above `id`, itself not counted.
    fn locale_scope_above(&self, id: StableNodeId) -> LocaleScope {
        let document = self.document_of(id).expect("a live node has a document");
        self.scope_at(document, self.parent_id(id))
    }

    /// The scope a node at `from` in `document` reads: the nearest subtree
    /// scope at or above `from`, else the window.
    fn scope_at(&self, document: DocumentId, from: Option<StableNodeId>) -> LocaleScope {
        let mut current = from;
        while let Some(node) = current {
            if self.i18n.scopes.contains_key(&node) {
                return LocaleScope::Node(node);
            }
            current = self.parent_id(node);
        }
        LocaleScope::Document(document)
    }

    /// Move the nodes of `from` that lie in `root`'s subtree, `root`
    /// included, to `to`, and resolve them there: one transaction. Visits
    /// `from`'s nodes, never the subtree.
    fn adopt_dependents(&mut self, from: LocaleScope, to: LocaleScope, root: StableNodeId) {
        let before = self.scope_locale(from).cloned();
        let stop = match from {
            LocaleScope::Node(scope) => Some(scope),
            LocaleScope::Document(_) => None,
        };
        // A scope's own nodes all lie under it.
        let mut moved: Vec<StableNodeId> = self
            .i18n
            .dependents
            .get(&from)
            .map(|nodes| {
                nodes
                    .iter()
                    .copied()
                    .filter(|node| stop == Some(root) || self.lies_under(*node, root, stop))
                    .collect()
            })
            .unwrap_or_default();
        moved.sort_unstable();
        for &node in &moved {
            unlink(&mut self.i18n.dependents, from, node);
            link(&mut self.i18n.dependents, to, node);
            if let Some(entry) = self.i18n.nodes.get_mut(&node) {
                entry.scope = to;
            }
        }
        let _ = self.switch_scope(to, before, &moved, false);
    }

    /// Whether `node` is `root` or under it, walking up no further than
    /// `stop`.
    fn lies_under(
        &self,
        node: StableNodeId,
        root: StableNodeId,
        stop: Option<StableNodeId>,
    ) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            if id == root {
                return true;
            }
            if Some(id) == stop {
                return false;
            }
            current = self.parent_id(id);
        }
        false
    }

    /// Resolve every localized node again: the catalog, the fallback or the
    /// missing-message policy changed. One transaction.
    fn resolve_every_localized(&mut self) -> bool {
        let mut ids: Vec<StableNodeId> = self.i18n.nodes.keys().copied().collect();
        ids.sort_unstable();
        let mut changed = false;
        for id in ids {
            changed |= self.resolve_localized(id, self.localized_language(id));
        }
        changed
    }

    /// The locales of `scopes` changed from what each held before: one
    /// transaction over the localized nodes they hold and their roots'
    /// direction.
    fn switch_locale(&mut self, scopes: Vec<(LocaleScope, Option<Locale>)>) -> bool {
        let mut changed = false;
        for (scope, before) in scopes {
            let mut nodes: Vec<StableNodeId> = self
                .i18n
                .dependents
                .get(&scope)
                .map(|nodes| nodes.iter().copied().collect())
                .unwrap_or_default();
            nodes.sort_unstable();
            changed |= self.switch_scope(scope, before, &nodes, true);
        }
        changed
    }

    fn switch_scope(
        &mut self,
        scope: LocaleScope,
        before: Option<Locale>,
        nodes: &[StableNodeId],
        own_root: bool,
    ) -> bool {
        let after = self.scope_locale(scope).cloned();
        if before == after {
            return false;
        }
        let messages_moved =
            before.as_ref().map(Locale::messages) != after.as_ref().map(Locale::messages);
        let language_moved =
            before.as_ref().and_then(Locale::language) != after.as_ref().and_then(Locale::language);
        let formatting_moved =
            before.as_ref().map(Locale::formatting) != after.as_ref().map(Locale::formatting);
        let direction_moved =
            before.as_ref().map(Locale::direction) != after.as_ref().map(Locale::direction);
        let generations = self.i18n.generations.entry(scope).or_default();
        generations.messages += u64::from(messages_moved);
        generations.language += u64::from(language_moved);
        generations.formatting += u64::from(formatting_moved);
        generations.direction += u64::from(direction_moved);
        // What a node shaped in before: the language the old locale named,
        // else the locale its message came from.
        let named = before.as_ref().and_then(Locale::language);
        let mut changed = false;
        for &id in nodes {
            let language = named
                .cloned()
                .or_else(|| self.i18n.nodes.get(&id)?.resolved.clone());
            // Moved messages resolve every node; a formatting locale that
            // moved alone, only the ones that format a number, a date or a
            // plural.
            if messages_moved || (formatting_moved && self.localized_formats(id)) {
                changed |= self.resolve_localized(id, language);
            } else if language_moved && self.localized_language(id) != language {
                self.mark_localized_language(id);
                changed = true;
            }
        }
        if direction_moved && own_root {
            changed = true;
            match scope {
                LocaleScope::Node(root) => self.restyle(root, None),
                LocaleScope::Document(document) => {
                    let mut roots: Vec<StableNodeId> = self
                        .live_document_roots
                        .get(&document)
                        .map(|roots| roots.iter().copied().collect())
                        .unwrap_or_default();
                    roots.sort_unstable();
                    for root in roots {
                        self.restyle(root, None);
                    }
                }
            }
        }
        changed
    }

    /// `id`'s language follows its locale's, which moved: its style resolves
    /// again and its text shapes again.
    fn mark_localized_language(&mut self, id: StableNodeId) {
        self.mark(id, DirtyMask::STYLE | DirtyMask::TEXT);
    }

    /// Resolve `id`'s message in its scope's locale and show the result when
    /// it differs from what the node shows. `language` is what it shaped in
    /// before; its language is marked when that moved.
    fn resolve_localized(&mut self, id: StableNodeId, language: Option<LanguageTag>) -> bool {
        let Some(entry) = self.i18n.nodes.get(&id) else {
            return false;
        };
        let message = entry.text.message;
        let args = entry.text.args.clone();
        let scope = entry.scope;
        let locale = self.scope_locale(scope).cloned();
        let asked = locale
            .as_ref()
            .map(|locale| locale.messages().clone())
            .or_else(|| self.i18n.fallback.clone());
        let found = asked.and_then(|asked| self.lookup_message(&asked, message));
        let mut shown = std::mem::take(&mut self.i18n.scratch);
        shown.clear();
        let wrote = match &found {
            Some((resolved, compiled)) => {
                // Numbers and dates in the scope's formatting locale, else
                // the locale the message came from.
                let formatting = locale
                    .as_ref()
                    .map(|locale| locale.formatting().clone())
                    .unwrap_or_else(|| resolved.clone());
                let formatter =
                    Arc::clone(self.i18n.formatter.get_or_insert_with(default_formatter));
                let mut issues = Vec::new();
                compiled.format(
                    &formatting,
                    &args,
                    formatter.as_ref(),
                    &mut shown,
                    &mut issues,
                );
                // The output names each argument it lacks; the event says
                // which node.
                for FormatIssue::Argument(_) in issues {
                    nana_diagnostics::event!(
                        nana_diagnostics::framework::text::I18N_ARGUMENT_INVALID,
                        node = id.get()
                    );
                }
                true
            }
            None => match self.i18n.missing {
                MissingMessage::Key => {
                    shown.push_str(&message.key());
                    true
                }
                MissingMessage::KeepPrevious => false,
            },
        };
        if let Some(entry) = self.i18n.nodes.get_mut(&id) {
            entry.resolved = found.map(|(locale, _)| locale);
        }
        let mut changed = false;
        if self.localized_language(id) != language {
            self.mark_localized_language(id);
            changed = true;
        }
        if wrote && self.nodes.text_content(id).value.as_str() != shown {
            self.replace_text_content(
                id,
                TextContent {
                    value: shown.as_str().into(),
                },
            );
            self.mark(
                id,
                DirtyMask::TEXT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
            );
            changed = true;
        }
        self.i18n.scratch = shown;
        changed
    }

    /// Whether `id`'s message, as last resolved, formats a number, a date or
    /// a plural: whether a formatting locale reaches it.
    fn localized_formats(&self, id: StableNodeId) -> bool {
        let Some(entry) = self.i18n.nodes.get(&id) else {
            return false;
        };
        let Some(locale) = self.scope_locale(entry.scope) else {
            return true;
        };
        self.i18n
            .lookups
            .get(&(locale.messages().clone(), entry.text.message))
            .is_none_or(|found| {
                found
                    .as_ref()
                    .is_some_and(|(_, compiled)| compiled.formats())
            })
    }

    /// The pattern of `message` for a scope asking `asked`: along its
    /// fallback chain, then the configured fallback's. Cached per asked
    /// locale and message.
    fn lookup_message(
        &mut self,
        asked: &LanguageTag,
        message: MessageId,
    ) -> Option<(LanguageTag, Arc<CompiledMessage>)> {
        let key = (asked.clone(), message);
        if let Some(found) = self.i18n.lookups.get(&key) {
            return found.clone();
        }
        let pattern = self.i18n.catalog.as_ref().and_then(|catalog| {
            let fallback = self.i18n.fallback.iter().flat_map(fallback_chain);
            fallback_chain(asked)
                .chain(fallback)
                .find_map(|tag| catalog.pattern(&tag, message).map(|pattern| (tag, pattern)))
        });
        let found = pattern.map(|(tag, pattern)| {
            let compiled = CompiledMessage::parse(&pattern).unwrap_or_else(|error| {
                nana_diagnostics::fault!(
                    nana_diagnostics::framework::text::I18N_PATTERN_INVALID,
                    at = error.at as u64;
                    "message {} in {} is not a pattern at byte {}",
                    message.key(),
                    tag.as_str(),
                    error.at
                );
                CompiledMessage::literal(&pattern)
            });
            (tag, Arc::new(compiled))
        });
        if found.is_none() {
            nana_diagnostics::event!(nana_diagnostics::framework::text::I18N_MESSAGE_MISSING);
        }
        self.i18n.lookups.insert(key, found.clone());
        found
    }
}
