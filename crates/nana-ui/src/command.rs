use std::collections::BTreeMap;
use std::fmt;

use iced::keyboard;
pub use nana_ui_core::{ActionId, ContextPredicate, KeyContext};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionDescriptor {
    pub id: ActionId,
    pub label: String,
    pub category: Option<String>,
    pub keywords: Vec<String>,
    pub when: ContextPredicate,
    pub enabled: bool,
}

impl ActionDescriptor {
    pub fn new(id: impl Into<ActionId>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: None,
            keywords: Vec::new(),
            when: ContextPredicate::always(),
            enabled: true,
        }
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn keywords(mut self, keywords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    pub fn when(mut self, when: ContextPredicate) -> Self {
        self.when = when;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionRegistryError {
    EmptyId,
    EmptyLabel { id: ActionId },
    Duplicate { id: ActionId },
}

impl fmt::Display for ActionRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId => formatter.write_str("action id must not be empty"),
            Self::EmptyLabel { id } => write!(formatter, "action `{id}` must have a label"),
            Self::Duplicate { id } => write!(formatter, "action `{id}` is already registered"),
        }
    }
}

impl std::error::Error for ActionRegistryError {}

#[derive(Debug, Clone)]
struct RegisteredAction {
    descriptor: ActionDescriptor,
    order: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ActionRegistry {
    actions: BTreeMap<ActionId, RegisteredAction>,
    next_order: u64,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        mut descriptor: ActionDescriptor,
    ) -> Result<(), ActionRegistryError> {
        descriptor.id = ActionId::new(descriptor.id.as_str().trim());
        descriptor.label = descriptor.label.trim().to_owned();
        if descriptor.id.as_str().is_empty() {
            return Err(ActionRegistryError::EmptyId);
        }
        if descriptor.label.is_empty() {
            return Err(ActionRegistryError::EmptyLabel { id: descriptor.id });
        }
        if self.actions.contains_key(&descriptor.id) {
            return Err(ActionRegistryError::Duplicate { id: descriptor.id });
        }
        let order = self.next_order;
        self.next_order = self.next_order.saturating_add(1);
        self.actions.insert(
            descriptor.id.clone(),
            RegisteredAction { descriptor, order },
        );
        Ok(())
    }

    pub fn unregister(&mut self, id: &ActionId) -> Option<ActionDescriptor> {
        self.actions.remove(id).map(|action| action.descriptor)
    }

    pub fn set_enabled(&mut self, id: &ActionId, enabled: bool) -> bool {
        let Some(action) = self.actions.get_mut(id) else {
            return false;
        };
        action.descriptor.enabled = enabled;
        true
    }

    pub fn get(&self, id: &ActionId) -> Option<&ActionDescriptor> {
        self.actions.get(id).map(|action| &action.descriptor)
    }

    pub fn is_available(&self, id: &ActionId, context: &KeyContext) -> bool {
        self.get(id)
            .is_some_and(|action| action.enabled && action.when.matches(context))
    }

    pub fn available(&self, context: &KeyContext) -> Vec<&ActionDescriptor> {
        let mut actions = self
            .actions
            .values()
            .filter(|action| action.descriptor.enabled && action.descriptor.when.matches(context))
            .collect::<Vec<_>>();
        actions.sort_by_key(|action| action.order);
        actions
            .into_iter()
            .map(|action| &action.descriptor)
            .collect()
    }

    pub fn search<'a>(&'a self, query: &str, context: &KeyContext) -> Vec<ActionMatch<'a>> {
        let query = query.trim().to_lowercase();
        let mut matches = self
            .actions
            .values()
            .filter(|action| action.descriptor.enabled && action.descriptor.when.matches(context))
            .filter_map(|action| {
                action_score(&action.descriptor, &query).map(|score| ActionMatch {
                    action: &action.descriptor,
                    score,
                    order: action.order,
                })
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|matched| (matched.score, matched.order));
        matches
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ActionMatch<'a> {
    pub action: &'a ActionDescriptor,
    pub score: u32,
    order: u64,
}

fn action_score(action: &ActionDescriptor, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let candidates = std::iter::once(action.label.as_str())
        .chain(std::iter::once(action.id.as_str()))
        .chain(action.category.as_deref())
        .chain(action.keywords.iter().map(String::as_str))
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    query
        .split_whitespace()
        .map(|term| {
            candidates
                .iter()
                .filter_map(|candidate| fuzzy_score(candidate, term))
                .min()
        })
        .try_fold(0_u32, |total, score| {
            score.map(|score| total.saturating_add(score))
        })
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<u32> {
    if candidate == query {
        return Some(0);
    }
    if candidate.starts_with(query) {
        return Some(
            10 + candidate
                .chars()
                .count()
                .saturating_sub(query.chars().count()) as u32,
        );
    }
    if let Some(position) = candidate.find(query) {
        return Some(100 + position as u32);
    }
    let mut candidate_chars = candidate.chars().enumerate();
    let mut last = 0_usize;
    let mut gap = 0_usize;
    for query_char in query.chars() {
        let (index, _) =
            candidate_chars.find(|(_, candidate_char)| *candidate_char == query_char)?;
        gap = gap.saturating_add(index.saturating_sub(last));
        last = index.saturating_add(1);
    }
    Some(200 + gap as u32)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyModifiers {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl KeyModifiers {
    pub const fn primary() -> Self {
        if cfg!(target_os = "macos") {
            Self {
                logo: true,
                control: false,
                alt: false,
                shift: false,
            }
        } else {
            Self {
                control: true,
                alt: false,
                shift: false,
                logo: false,
            }
        }
    }

    pub const fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }
}

impl From<keyboard::Modifiers> for KeyModifiers {
    fn from(value: keyboard::Modifiers) -> Self {
        Self {
            control: value.control(),
            alt: value.alt(),
            shift: value.shift(),
            logo: value.logo(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyStroke {
    pub key: String,
    pub modifiers: KeyModifiers,
}

impl KeyStroke {
    pub fn new(key: impl Into<String>, modifiers: KeyModifiers) -> Self {
        Self {
            key: normalize_key_name(&key.into()),
            modifiers,
        }
    }

    pub fn from_iced(key: &keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Self> {
        let key = match key.as_ref() {
            keyboard::Key::Named(named) => format!("{named:?}"),
            keyboard::Key::Character(character) => character.to_owned(),
            keyboard::Key::Unidentified => return None,
        };
        Some(Self::new(key, modifiers.into()))
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.control {
            parts.push("Ctrl".to_owned());
        }
        if self.modifiers.alt {
            parts.push("Alt".to_owned());
        }
        if self.modifiers.shift {
            parts.push("Shift".to_owned());
        }
        if self.modifiers.logo {
            parts.push(if cfg!(target_os = "macos") {
                "⌘".to_owned()
            } else {
                "Super".to_owned()
            });
        }
        let key = if self.key.chars().count() == 1 {
            self.key.to_uppercase()
        } else {
            self.key.clone()
        };
        parts.push(key);
        parts.join("+")
    }
}

fn normalize_key_name(value: &str) -> String {
    let value = value.trim();
    if value.chars().count() == 1 {
        value.to_lowercase()
    } else {
        value.to_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBinding {
    pub action: ActionId,
    pub sequence: Vec<KeyStroke>,
    pub when: ContextPredicate,
}

impl KeyBinding {
    pub fn new(action: impl Into<ActionId>, stroke: KeyStroke) -> Self {
        Self {
            action: action.into(),
            sequence: vec![stroke],
            when: ContextPredicate::always(),
        }
    }

    pub fn sequence(
        action: impl Into<ActionId>,
        sequence: impl IntoIterator<Item = KeyStroke>,
    ) -> Self {
        Self {
            action: action.into(),
            sequence: sequence.into_iter().collect(),
            when: ContextPredicate::always(),
        }
    }

    pub fn when(mut self, when: ContextPredicate) -> Self {
        self.when = when;
        self
    }

    pub fn display(&self) -> String {
        self.sequence
            .iter()
            .map(KeyStroke::display)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeymapState {
    pending: Vec<KeyStroke>,
}

impl KeymapState {
    pub fn pending(&self) -> &[KeyStroke] {
        &self.pending
    }

    pub fn cancel(&mut self) {
        self.pending.clear();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapMatch {
    Dispatch(ActionId),
    Pending,
    NoMatch,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keymap {
    bindings: Vec<KeyBinding>,
}

impl Keymap {
    pub fn new(bindings: impl IntoIterator<Item = KeyBinding>) -> Self {
        Self {
            bindings: bindings.into_iter().collect(),
        }
    }

    pub fn push(&mut self, binding: KeyBinding) {
        self.bindings.push(binding);
    }

    pub fn resolve(
        &self,
        state: &mut KeymapState,
        stroke: KeyStroke,
        context: &KeyContext,
        registry: &ActionRegistry,
    ) -> KeymapMatch {
        let had_pending = !state.pending.is_empty();
        state.pending.push(stroke.clone());
        let matched = self.resolve_pending(state, context, registry);
        if matched == KeymapMatch::NoMatch && had_pending {
            state.pending.clear();
            state.pending.push(stroke);
            return self.resolve_pending(state, context, registry);
        }
        matched
    }

    pub fn binding_label(
        &self,
        action: &ActionId,
        context: &KeyContext,
        registry: &ActionRegistry,
    ) -> Option<String> {
        self.bindings
            .iter()
            .rev()
            .find(|binding| {
                binding.action == *action
                    && !binding.sequence.is_empty()
                    && binding.when.matches(context)
                    && registry.is_available(action, context)
            })
            .map(KeyBinding::display)
    }

    fn resolve_pending(
        &self,
        state: &mut KeymapState,
        context: &KeyContext,
        registry: &ActionRegistry,
    ) -> KeymapMatch {
        let candidates = self.bindings.iter().rev().filter(|binding| {
            binding.sequence.starts_with(&state.pending)
                && binding.when.matches(context)
                && registry.is_available(&binding.action, context)
        });
        let mut has_prefix = false;
        for binding in candidates {
            if binding.sequence.len() == state.pending.len() {
                let action = binding.action.clone();
                state.pending.clear();
                return KeymapMatch::Dispatch(action);
            }
            has_prefix = true;
        }
        if has_prefix {
            KeymapMatch::Pending
        } else {
            state.pending.clear();
            KeymapMatch::NoMatch
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPickerSelection {
    pub action: ActionId,
    pub restore_focus: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionPickerNavigation {
    Previous,
    Next,
    First,
    Last,
    Confirm,
    Dismiss,
}

impl ActionPickerNavigation {
    pub fn from_iced_key(key: &keyboard::Key) -> Option<Self> {
        match key.as_ref() {
            keyboard::Key::Named(keyboard::key::Named::ArrowUp) => Some(Self::Previous),
            keyboard::Key::Named(keyboard::key::Named::ArrowDown) => Some(Self::Next),
            keyboard::Key::Named(keyboard::key::Named::Home) => Some(Self::First),
            keyboard::Key::Named(keyboard::key::Named::End) => Some(Self::Last),
            keyboard::Key::Named(keyboard::key::Named::Enter) => Some(Self::Confirm),
            keyboard::Key::Named(keyboard::key::Named::Escape) => Some(Self::Dismiss),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionPickerState {
    open: bool,
    query: String,
    selected: usize,
    restore_focus: Option<String>,
}

impl ActionPickerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn open(&mut self, restore_focus: Option<String>) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.restore_focus = restore_focus;
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.selected = 0;
    }

    pub fn move_selection(&mut self, offset: isize, result_count: usize) {
        if result_count == 0 {
            self.selected = 0;
            return;
        }
        self.selected =
            (self.selected as isize + offset).rem_euclid(result_count as isize) as usize;
    }

    pub fn navigate(&mut self, navigation: ActionPickerNavigation, result_count: usize) {
        match navigation {
            ActionPickerNavigation::Previous => self.move_selection(-1, result_count),
            ActionPickerNavigation::Next => self.move_selection(1, result_count),
            ActionPickerNavigation::First => self.selected = 0,
            ActionPickerNavigation::Last if result_count > 0 => self.selected = result_count - 1,
            ActionPickerNavigation::Last
            | ActionPickerNavigation::Confirm
            | ActionPickerNavigation::Dismiss => {}
        }
    }

    pub fn sync_results(&mut self, result_count: usize) {
        if result_count == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(result_count - 1);
        }
    }

    pub fn dismiss(&mut self) -> Option<String> {
        self.open = false;
        self.query.clear();
        self.selected = 0;
        self.restore_focus.take()
    }

    pub fn confirm(&mut self, action: ActionId) -> ActionPickerSelection {
        let restore_focus = self.dismiss();
        ActionPickerSelection {
            action,
            restore_focus,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ActionRegistry {
        let mut registry = ActionRegistry::new();
        registry
            .register(
                ActionDescriptor::new("workspace.open", "打开工作区")
                    .category("工作区")
                    .keywords(["folder", "project"]),
            )
            .unwrap();
        registry
            .register(
                ActionDescriptor::new("editor.save", "保存文件")
                    .category("编辑器")
                    .when(ContextPredicate::always().all_of(["editor"])),
            )
            .unwrap();
        registry
            .register(ActionDescriptor::new("editor.close", "关闭文件").enabled(false))
            .unwrap();
        registry
    }

    #[test]
    fn registry_exposes_only_enabled_actions_in_the_active_context() {
        let registry = registry();
        let global = registry.available(&KeyContext::default());
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].id.as_str(), "workspace.open");

        let editor = registry.available(&KeyContext::new(["editor"]));
        assert_eq!(editor.len(), 2);
        assert_eq!(editor[1].id.as_str(), "editor.save");
    }

    #[test]
    fn registry_search_ranks_labels_before_keyword_and_subsequence_matches() {
        let registry = registry();
        let context = KeyContext::new(["editor"]);
        let label = registry.search("保存", &context);
        assert_eq!(label[0].action.id.as_str(), "editor.save");
        let keyword = registry.search("folder", &context);
        assert_eq!(keyword[0].action.id.as_str(), "workspace.open");
        let subsequence = registry.search("dkq", &context);
        assert!(subsequence.is_empty());
    }

    #[test]
    fn keymap_resolves_chords_against_context_and_retries_after_a_broken_prefix() {
        let registry = registry();
        let context = KeyContext::new(["editor"]);
        let primary = KeyModifiers::primary();
        let keymap = Keymap::new([
            KeyBinding::sequence(
                "workspace.open",
                [KeyStroke::new("k", primary), KeyStroke::new("o", primary)],
            ),
            KeyBinding::new("editor.save", KeyStroke::new("s", primary))
                .when(ContextPredicate::always().all_of(["editor"])),
        ]);
        let mut state = KeymapState::default();
        assert_eq!(
            keymap.resolve(
                &mut state,
                KeyStroke::new("k", primary),
                &context,
                &registry,
            ),
            KeymapMatch::Pending
        );
        assert_eq!(
            keymap.resolve(
                &mut state,
                KeyStroke::new("s", primary),
                &context,
                &registry,
            ),
            KeymapMatch::Dispatch(ActionId::from("editor.save"))
        );
    }

    #[test]
    fn picker_wraps_selection_and_returns_the_prior_focus_on_confirm() {
        let mut picker = ActionPickerState::new();
        picker.open(Some("editor.body".to_owned()));
        picker.move_selection(-1, 3);
        assert_eq!(picker.selected(), 2);
        picker.set_query("save");
        assert_eq!(picker.selected(), 0);
        let selected = picker.confirm(ActionId::from("editor.save"));
        assert_eq!(selected.action.as_str(), "editor.save");
        assert_eq!(selected.restore_focus.as_deref(), Some("editor.body"));
        assert!(!picker.is_open());
    }
}
