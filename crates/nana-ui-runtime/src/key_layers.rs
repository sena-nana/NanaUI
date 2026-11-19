//! Backend-neutral shortcut capture and keymap resolution.
//!
//! Host-facing command-palette `KeyStroke` / `Keymap` / `ActionRegistry` live in
//! `nana-ui::command`. This module uses [`CapturedStroke`] and a thin
//! enabled-state registry — not a second command palette. Live keys are
//! [`nana_ui_input::KeyInput`]; stored chords copy that event's logical key
//! and modifier bits. This file stays free of platform types.

use nana_ui_core::sort::CompactSort as _;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use nana_ui_core::{
    ActionId, ContextPredicate, KeyContext, LengthSpec, MenuBar, MenuEntry, MenuShortcut,
    SemanticColorRole, StyleModelRef,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentTextRegion, ComponentView, InteractionState,
    LayoutBox, MutationQueue, NodeKind, NodeStyle, StableNodeId, TextHorizontalAlignment,
    TextValue, UiWorld, WidgetPaintCtx, WidgetPaintOp, WidgetPaintOutput, WidgetQuad, WidgetVisual,
};

/// Focus-scope tag inserted by [`AppContext::document_key_context`](crate::AppContext::document_key_context)
/// when a blocking modal overlay (`Dialog` / `Drawer`) covers the document.
pub const CONTEXT_MODAL: &str = "modal";
/// Focus-scope tag inserted by [`AppContext::document_key_context`](crate::AppContext::document_key_context)
/// when an editable text field holds document focus.
pub const CONTEXT_TEXT_INPUT: &str = "text-input";
/// Focus-scope tag inserted by [`AppContext::document_key_context`](crate::AppContext::document_key_context)
/// when the focused node has an active IME preedit composition.
pub const CONTEXT_IME_COMPOSING: &str = "ime-composing";

/// The modifiers of a stored shortcut chord: the same bits a live key event
/// carries, so a chord copies them as they are.
pub type KeyModifiers = nana_ui_input::InputModifiers;

/// Chord key plus modifiers. Hosts copy these bits from platform input.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapturedStroke {
    pub key: Arc<str>,
    pub modifiers: KeyModifiers,
}

impl CapturedStroke {
    pub fn new(key: impl Into<Arc<str>>, modifiers: KeyModifiers) -> Self {
        Self {
            key: normalize_key_name(key.into().as_ref()),
            modifiers,
        }
    }

    /// Converts a canonical [`nana_ui_input::KeyInput`] into a normalized shortcut stroke.
    pub fn from_canonical(event: &nana_ui_input::KeyInput) -> Option<Self> {
        let stroke = Self::new(event.logical.0.as_ref(), event.modifiers);
        (!stroke.key.is_empty()).then_some(stroke)
    }

    /// Platform-conventional rendering, e.g. `Ctrl+Shift+P` or `⌘P`.
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
        if self.modifiers.meta {
            parts.push(if cfg!(target_os = "macos") {
                "⌘".to_owned()
            } else {
                "Super".to_owned()
            });
        }
        let key = if self.key.chars().count() == 1 {
            self.key.to_uppercase()
        } else {
            self.key.to_string()
        };
        parts.push(key);
        parts.join("+")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCaptureEvent {
    Captured(CapturedStroke),
    Cleared,
    Cancelled,
}

/// Records one shortcut while the application has enabled recording.
///
/// Policy object that also projects a small chrome leaf for snapshots.
/// Modifier-only presses stay pending; Tab is left to focus traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCaptureLayer {
    pub recording: bool,
    pub pending_modifiers: KeyModifiers,
}

impl Default for KeyCaptureLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyCaptureLayer {
    pub fn new() -> Self {
        Self {
            recording: false,
            pending_modifiers: KeyModifiers::empty(),
        }
    }

    pub fn recording(mut self, recording: bool) -> Self {
        self.set_recording(recording);
        self
    }

    pub fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
        if !recording {
            self.pending_modifiers = KeyModifiers::empty();
        }
    }

    /// True when a press must not reach content or the application keymap.
    pub fn should_consume(&self, event: &nana_ui_input::KeyInput) -> bool {
        self.recording && event.is_pressed() && !is_named(event.logical.0.as_ref(), "Tab")
    }

    pub fn handle_key(&mut self, event: &nana_ui_input::KeyInput) -> Option<KeyCaptureEvent> {
        if !self.recording {
            self.pending_modifiers = KeyModifiers::empty();
            return None;
        }
        let modifiers = event.modifiers;
        if !event.is_pressed() {
            if is_modifier_key(event.logical.0.as_ref()) {
                self.pending_modifiers = modifiers;
            }
            return None;
        }
        if event.repeat {
            return None;
        }
        if is_named(event.logical.0.as_ref(), "Tab") {
            return None;
        }
        if is_named(event.logical.0.as_ref(), "Escape") {
            self.pending_modifiers = KeyModifiers::empty();
            return Some(KeyCaptureEvent::Cancelled);
        }
        if is_named(event.logical.0.as_ref(), "Delete")
            || is_named(event.logical.0.as_ref(), "Backspace")
        {
            self.pending_modifiers = KeyModifiers::empty();
            return Some(KeyCaptureEvent::Cleared);
        }
        if is_modifier_key(event.logical.0.as_ref()) {
            self.pending_modifiers = modifiers;
            return None;
        }
        let stroke = CapturedStroke::from_canonical(event)?;
        self.pending_modifiers = KeyModifiers::empty();
        Some(KeyCaptureEvent::Captured(stroke))
    }
}

impl ComponentView for KeyCaptureLayer {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "key-capture-layer".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = KeyCaptureLayerVisual {
            recording: self.recording,
        };
        if world.widget_visual_as::<KeyCaptureLayerVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &layer_chrome_style(),
            InteractionState {
                pointer_events: false,
                focusable: self.recording,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::from(if self.recording {
                    "Recording shortcut"
                } else {
                    "Shortcut capture"
                })),
                ..AccessibilityState::default()
            },
        );
    }
}

/// One registered action: its stable id, when it is available, and the copy a
/// command palette shows for it.
///
/// This is the single action record. The keymap needs `id` / `enabled` / `when`;
/// a palette additionally reads `label` / `category` / `keywords`. A host that
/// only binds keys leaves the copy empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionDescriptor {
    pub id: ActionId,
    pub enabled: bool,
    pub when: ContextPredicate,
    /// Human-readable name. Empty for actions that never reach a palette.
    pub label: String,
    pub category: Option<String>,
    pub keywords: Vec<String>,
}

impl ActionDescriptor {
    pub fn new(id: impl Into<ActionId>) -> Self {
        Self {
            id: id.into(),
            enabled: true,
            when: ContextPredicate::always(),
            label: String::new(),
            category: None,
            keywords: Vec::new(),
        }
    }

    /// Action carrying palette copy.
    pub fn labeled(id: impl Into<ActionId>, label: impl Into<String>) -> Self {
        Self::new(id).label(label)
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn keywords(mut self, keywords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn when(mut self, when: ContextPredicate) -> Self {
        self.when = when;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionRegistryError {
    EmptyId,
    Duplicate { id: ActionId },
}

impl fmt::Display for ActionRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId => formatter.write_str("action id must not be empty"),
            Self::Duplicate { id } => write!(formatter, "action `{id}` is already registered"),
        }
    }
}

impl std::error::Error for ActionRegistryError {}

type DynamicEnabled = Arc<dyn Fn(&KeyContext) -> bool + Send + Sync>;
type DynamicLabel = Arc<dyn Fn(&KeyContext) -> String + Send + Sync>;

fn option_arc_ptr_eq<T: ?Sized>(left: &Option<Arc<T>>, right: &Option<Arc<T>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

#[derive(Clone)]
struct RegisteredAction {
    descriptor: ActionDescriptor,
    enabled_when: Option<DynamicEnabled>,
    dynamic_label: Option<DynamicLabel>,
    /// Registration order, so a palette lists actions the way the host declared
    /// them rather than alphabetically by id.
    order: u64,
}

impl fmt::Debug for RegisteredAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredAction")
            .field("descriptor", &self.descriptor)
            .field("has_enabled_binding", &self.enabled_when.is_some())
            .field("has_label_binding", &self.dynamic_label.is_some())
            .field("order", &self.order)
            .finish()
    }
}

impl PartialEq for RegisteredAction {
    fn eq(&self, other: &Self) -> bool {
        self.descriptor == other.descriptor
            && self.order == other.order
            && option_arc_ptr_eq(&self.enabled_when, &other.enabled_when)
            && option_arc_ptr_eq(&self.dynamic_label, &other.dynamic_label)
    }
}

impl Eq for RegisteredAction {}

impl RegisteredAction {
    fn is_available(&self, context: &KeyContext) -> bool {
        self.descriptor.enabled
            && self.descriptor.when.matches(context)
            && self
                .enabled_when
                .as_ref()
                .is_none_or(|check| check(context))
    }

    fn resolve_label(&self, context: &KeyContext) -> Cow<'_, str> {
        match &self.dynamic_label {
            Some(resolve) => Cow::Owned(resolve(context)),
            None => Cow::Borrowed(self.descriptor.label.as_str()),
        }
    }
}

/// Every action the application registers: the keymap reads `enabled` / `when`,
/// a command palette reads the copy and calls [`Self::search`]. One registry,
/// not one per consumer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        let Some(id) = descriptor.id.normalized() else {
            return Err(ActionRegistryError::EmptyId);
        };
        descriptor.id = id;
        if self.actions.contains_key(&descriptor.id) {
            return Err(ActionRegistryError::Duplicate { id: descriptor.id });
        }
        let order = self.next_order;
        self.next_order += 1;
        self.actions.insert(
            descriptor.id.clone(),
            RegisteredAction {
                descriptor,
                enabled_when: None,
                dynamic_label: None,
                order,
            },
        );
        Ok(())
    }

    pub fn unregister(&mut self, id: &ActionId) -> Option<ActionDescriptor> {
        let id = id.normalized()?;
        self.actions.remove(&id).map(|action| action.descriptor)
    }

    pub fn set_enabled(&mut self, id: &ActionId, enabled: bool) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        let Some(action) = self.actions.get_mut(&id) else {
            return false;
        };
        action.descriptor.enabled = enabled;
        true
    }

    /// Updates the static fallback label of a registered action.
    pub fn set_label(&mut self, id: &ActionId, label: impl Into<String>) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        let Some(action) = self.actions.get_mut(&id) else {
            return false;
        };
        action.descriptor.label = label.into();
        true
    }

    /// Binds a dynamic availability check evaluated alongside `enabled` and `when`.
    pub fn bind_enabled(
        &mut self,
        id: &ActionId,
        enabled: impl Fn(&KeyContext) -> bool + Send + Sync + 'static,
    ) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        let Some(action) = self.actions.get_mut(&id) else {
            return false;
        };
        action.enabled_when = Some(Arc::new(enabled));
        true
    }

    /// Removes any dynamic availability binding on `id`, restoring static evaluation.
    pub fn clear_enabled_binding(&mut self, id: &ActionId) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        let Some(action) = self.actions.get_mut(&id) else {
            return false;
        };
        action.enabled_when = None;
        true
    }

    /// Binds a dynamic label resolver (for example, `"撤销: 移动节点"`).
    pub fn bind_label(
        &mut self,
        id: &ActionId,
        label: impl Fn(&KeyContext) -> String + Send + Sync + 'static,
    ) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        let Some(action) = self.actions.get_mut(&id) else {
            return false;
        };
        action.dynamic_label = Some(Arc::new(label));
        true
    }

    /// Removes any dynamic label binding on `id`, falling back to `descriptor.label`.
    pub fn clear_label_binding(&mut self, id: &ActionId) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        let Some(action) = self.actions.get_mut(&id) else {
            return false;
        };
        action.dynamic_label = None;
        true
    }

    pub fn get(&self, id: &ActionId) -> Option<&ActionDescriptor> {
        let id = id.normalized()?;
        self.actions.get(&id).map(|action| &action.descriptor)
    }

    pub fn is_available(&self, id: &ActionId, context: &KeyContext) -> bool {
        let Some(id) = id.normalized() else {
            return false;
        };
        self.actions
            .get(&id)
            .is_some_and(|action| action.is_available(context))
    }

    /// Resolves an action's display label for `context` (dynamic binding if set,
    /// otherwise `descriptor.label`).
    pub fn resolved_label(&self, id: &ActionId, context: &KeyContext) -> Option<String> {
        let id = id.normalized()?;
        self.actions
            .get(&id)
            .map(|action| action.resolve_label(context).into_owned())
    }

    /// Enabled actions whose `when` and dynamic availability match, in registration order.
    pub fn available(&self, context: &KeyContext) -> Vec<&ActionDescriptor> {
        let mut actions = self
            .actions
            .values()
            .filter(|action| action.is_available(context))
            .collect::<Vec<_>>();
        actions.compact_sort_by_key(|action| action.order);
        actions
            .into_iter()
            .map(|action| &action.descriptor)
            .collect()
    }

    /// Fuzzy-matches `query` against resolved label, id, category and keywords.
    /// Best match first; ties keep registration order.
    pub fn search<'a>(&'a self, query: &str, context: &KeyContext) -> Vec<ActionMatch<'a>> {
        let query = query.trim().to_lowercase();
        let mut matches = self
            .actions
            .values()
            .filter(|action| action.is_available(context))
            .filter_map(|action| {
                let label = action.resolve_label(context);
                action_score(&action.descriptor, label.as_ref(), &query).map(|score| ActionMatch {
                    action: &action.descriptor,
                    score,
                    order: action.order,
                })
            })
            .collect::<Vec<_>>();
        matches.compact_sort_by_key(|matched| (matched.score, matched.order));
        matches
    }

    /// Official DTO adapter from this registry (plus an optional [`Keymap`]) to
    /// [`CommandPaletteItem`]s for [`CommandPalette`](crate::CommandPalette).
    pub fn palette_items(
        &self,
        query: &str,
        context: &KeyContext,
        keymap: Option<&Keymap>,
    ) -> Vec<nana_ui_core::CommandPaletteItem> {
        self.search(query, context)
            .into_iter()
            .map(|matched| {
                let label = self
                    .resolved_label(&matched.action.id, context)
                    .unwrap_or_else(|| matched.action.label.clone());
                let mut item =
                    nana_ui_core::CommandPaletteItem::new(matched.action.id.clone(), label);
                if let Some(category) = &matched.action.category {
                    item = item.category(category.clone());
                }
                if let Some(shortcut) = keymap
                    .and_then(|keymap| keymap.binding_label(&matched.action.id, context, self))
                {
                    item = item.shortcut(shortcut);
                }
                item
            })
            .collect()
    }

    /// Official DTO adapter projecting one registered action into an [`ActionMenuItem`](crate::ActionMenuItem).
    ///
    /// Reflects dynamic label, availability (`.disabled(...)`), and shortcut hint.
    pub fn action_menu_item(
        &self,
        id: &ActionId,
        context: &KeyContext,
        keymap: Option<&Keymap>,
    ) -> Option<crate::ActionMenuItem> {
        let id = id.normalized()?;
        let label = self.resolved_label(&id, context)?;
        let mut item = crate::ActionMenuItem::new(label).disabled(!self.is_available(&id, context));
        if let Some(shortcut) = keymap.and_then(|keymap| keymap.display_binding(&id, context, self))
        {
            item = item.hint(shortcut);
        }
        Some(item)
    }
}

/// One search hit from [`ActionRegistry::search`]; lower `score` is better.
#[derive(Debug, Clone, Copy)]
pub struct ActionMatch<'a> {
    pub action: &'a ActionDescriptor,
    pub score: u32,
    order: u64,
}

fn action_score(action: &ActionDescriptor, resolved_label: &str, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let candidates = std::iter::once(resolved_label)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub action: ActionId,
    pub sequence: Vec<CapturedStroke>,
    pub when: ContextPredicate,
}

impl KeyBinding {
    pub fn new(action: impl Into<ActionId>, stroke: CapturedStroke) -> Self {
        let action = action.into();
        let action = action.normalized().unwrap_or(action);
        Self {
            action,
            sequence: vec![stroke],
            when: ContextPredicate::always(),
        }
    }

    pub fn sequence(
        action: impl Into<ActionId>,
        sequence: impl IntoIterator<Item = CapturedStroke>,
    ) -> Self {
        let action = action.into();
        let action = action.normalized().unwrap_or(action);
        Self {
            action,
            sequence: sequence.into_iter().collect(),
            when: ContextPredicate::always(),
        }
    }

    pub fn when(mut self, when: ContextPredicate) -> Self {
        self.when = when;
        self
    }
    /// The whole sequence rendered for display, e.g. `Ctrl+K Ctrl+S`.
    pub fn display(&self) -> String {
        self.sequence
            .iter()
            .map(CapturedStroke::display)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeymapState {
    pending: Vec<CapturedStroke>,
}

impl KeymapState {
    pub fn pending(&self) -> &[CapturedStroke] {
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

impl KeymapMatch {
    pub fn consumed(&self) -> bool {
        !matches!(self, Self::NoMatch)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    bindings: Vec<KeyBinding>,
}

impl Keymap {
    pub fn new(bindings: impl IntoIterator<Item = KeyBinding>) -> Self {
        Self {
            bindings: bindings.into_iter().collect(),
        }
    }

    /// Shortcut a command palette shows beside `action`, if one is bound and
    /// currently available. The last matching binding wins, so a later
    /// registration overrides an earlier default.
    pub fn binding_label(
        &self,
        action: &ActionId,
        context: &KeyContext,
        registry: &ActionRegistry,
    ) -> Option<String> {
        let action = action.normalized()?;
        if !registry.is_available(&action, context) {
            return None;
        }
        self.bindings
            .iter()
            .rev()
            .find(|binding| {
                binding.action.normalized().as_ref() == Some(&action)
                    && !binding.sequence.is_empty()
                    && binding.when.matches(context)
            })
            .map(KeyBinding::display)
    }

    /// Shortcut text for display beside `action` even while the action is
    /// disabled, provided its binding and descriptor `when` predicates match.
    pub fn display_binding(
        &self,
        action: &ActionId,
        context: &KeyContext,
        registry: &ActionRegistry,
    ) -> Option<String> {
        let action = action.normalized()?;
        let descriptor = registry.get(&action)?;
        if !descriptor.when.matches(context) {
            return None;
        }
        self.bindings
            .iter()
            .rev()
            .find(|binding| {
                binding.action.normalized().as_ref() == Some(&action)
                    && !binding.sequence.is_empty()
                    && binding.when.matches(context)
            })
            .map(KeyBinding::display)
    }

    /// Converts the active single-stroke binding of `action` into a [`MenuShortcut`].
    pub fn menu_shortcut(
        &self,
        action: &ActionId,
        context: &KeyContext,
        registry: &ActionRegistry,
    ) -> Option<MenuShortcut> {
        let action = action.normalized()?;
        let descriptor = registry.get(&action)?;
        if !descriptor.when.matches(context) {
            return None;
        }
        self.bindings
            .iter()
            .rev()
            .find(|binding| {
                binding.action.normalized().as_ref() == Some(&action)
                    && binding.sequence.len() == 1
                    && binding.when.matches(context)
            })
            .and_then(|binding| stroke_to_menu_shortcut(&binding.sequence[0]))
    }

    pub fn push(&mut self, binding: KeyBinding) {
        self.bindings.push(binding);
    }

    pub fn resolve(
        &self,
        state: &mut KeymapState,
        stroke: CapturedStroke,
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

    fn resolve_pending(
        &self,
        state: &mut KeymapState,
        context: &KeyContext,
        registry: &ActionRegistry,
    ) -> KeymapMatch {
        let candidates = self.bindings.iter().rev().filter_map(|binding| {
            let action = binding.action.normalized()?;
            (binding.sequence.starts_with(&state.pending)
                && binding.when.matches(context)
                && registry.is_available(&action, context))
            .then_some((binding, action))
        });
        let mut has_prefix = false;
        for (binding, action) in candidates {
            if binding.sequence.len() == state.pending.len() {
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

fn stroke_to_menu_shortcut(stroke: &CapturedStroke) -> Option<MenuShortcut> {
    if stroke.key.chars().count() != 1 {
        return None;
    }
    let modifiers = stroke.modifiers;
    let (primary, non_primary_platform_mod) = if cfg!(target_os = "macos") {
        (modifiers.meta, modifiers.control)
    } else {
        (modifiers.control, modifiers.meta)
    };
    if non_primary_platform_mod {
        return None;
    }
    Some(MenuShortcut {
        key: stroke.key.to_string(),
        primary,
        shift: modifiers.shift,
        alt: modifiers.alt,
    })
}

/// Rejection reasons when binding numeric [`MenuEntry::Item`](nana_ui_core::MenuEntry::Item) ids to [`ActionId`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuActionMapError {
    EmptyActionId,
    DuplicateMenuId(u32),
    DuplicateActionId(ActionId),
}

impl fmt::Display for MenuActionMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyActionId => formatter.write_str("action id must not be empty"),
            Self::DuplicateMenuId(id) => write!(formatter, "menu id {id} is already mapped"),
            Self::DuplicateActionId(id) => {
                write!(formatter, "action `{id}` is already mapped to a menu id")
            }
        }
    }
}

impl std::error::Error for MenuActionMapError {}

/// Thin bidirectional conversion between numeric menu item ids (`u32`) and
/// normalized [`ActionId`]s.
///
/// Stores only the validated id bijection; action descriptors and availability
/// stay in [`ActionRegistry`], and handlers stay in [`AppContext`](crate::AppContext).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuActionMap {
    id_to_action: BTreeMap<u32, ActionId>,
    action_to_id: BTreeMap<ActionId, u32>,
}

impl MenuActionMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(
        &mut self,
        menu_id: u32,
        action: impl Into<ActionId>,
    ) -> Result<(), MenuActionMapError> {
        let Some(action) = action.into().normalized() else {
            return Err(MenuActionMapError::EmptyActionId);
        };
        if self.id_to_action.contains_key(&menu_id) {
            return Err(MenuActionMapError::DuplicateMenuId(menu_id));
        }
        if self.action_to_id.contains_key(&action) {
            return Err(MenuActionMapError::DuplicateActionId(action));
        }
        self.id_to_action.insert(menu_id, action.clone());
        self.action_to_id.insert(action, menu_id);
        Ok(())
    }

    pub fn unbind_menu_id(&mut self, menu_id: u32) -> Option<ActionId> {
        let action = self.id_to_action.remove(&menu_id)?;
        self.action_to_id.remove(&action);
        Some(action)
    }

    pub fn unbind_action(&mut self, action: &ActionId) -> Option<u32> {
        let action = action.normalized()?;
        let menu_id = self.action_to_id.remove(&action)?;
        self.id_to_action.remove(&menu_id);
        Some(menu_id)
    }

    pub fn action_for(&self, menu_id: u32) -> Option<&ActionId> {
        self.id_to_action.get(&menu_id)
    }

    pub fn menu_id_for(&self, action: &ActionId) -> Option<u32> {
        let action = action.normalized()?;
        self.action_to_id.get(&action).copied()
    }

    /// Projects a mapped action into a [`MenuEntry::Item`] using the live label,
    /// availability, and optional shortcut from `registry` / `keymap`.
    pub fn menu_entry(
        &self,
        action: &ActionId,
        registry: &ActionRegistry,
        context: &KeyContext,
        keymap: Option<&Keymap>,
    ) -> Option<MenuEntry> {
        let action = action.normalized()?;
        let menu_id = self.menu_id_for(&action)?;
        let label = registry.resolved_label(&action, context)?;
        let mut entry =
            MenuEntry::item(menu_id, label).enabled(registry.is_available(&action, context));
        if let Some(shortcut) =
            keymap.and_then(|keymap| keymap.menu_shortcut(&action, context, registry))
        {
            entry = entry.shortcut(shortcut);
        }
        Some(entry)
    }

    /// Synchronizes every mapped item inside `bar` (including nested submenus)
    /// against `registry` (label + enabled) and `keymap` (when provided).
    pub fn sync_menu_bar(
        &self,
        bar: &mut MenuBar,
        registry: &ActionRegistry,
        context: &KeyContext,
        keymap: Option<&Keymap>,
    ) {
        for menu in &mut bar.menus {
            self.sync_menu(menu, registry, context, keymap);
        }
    }

    fn sync_menu(
        &self,
        menu: &mut nana_ui_core::Menu,
        registry: &ActionRegistry,
        context: &KeyContext,
        keymap: Option<&Keymap>,
    ) {
        for entry in &mut menu.entries {
            match entry {
                MenuEntry::Item {
                    id,
                    label,
                    shortcut,
                    enabled,
                    ..
                } => {
                    let Some(action) = self.action_for(*id) else {
                        continue;
                    };
                    if let Some(resolved) = registry.resolved_label(action, context) {
                        *label = resolved;
                    }
                    *enabled = registry.is_available(action, context);
                    if let Some(keymap) = keymap {
                        *shortcut = keymap.menu_shortcut(action, context, registry);
                    }
                }
                MenuEntry::Submenu(submenu) => {
                    self.sync_menu(submenu, registry, context, keymap);
                }
                MenuEntry::Separator => {}
            }
        }
    }
}

/// Resolves application key bindings against context and enabled actions.
///
/// Chord prefix state. A hit is consumed; a miss is `None` so content can
/// handle the key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeymapLayer {
    pub keymap: Keymap,
    pub context: KeyContext,
    pub registry: ActionRegistry,
    state: KeymapState,
}

impl KeymapLayer {
    pub fn new(keymap: Keymap, context: KeyContext, registry: ActionRegistry) -> Self {
        Self {
            keymap,
            context,
            registry,
            state: KeymapState::default(),
        }
    }

    pub fn pending(&self) -> &[CapturedStroke] {
        self.state.pending()
    }

    pub fn cancel(&mut self) {
        self.state.cancel();
    }

    /// Resolve including chord [`KeymapMatch::Pending`].
    pub fn resolve_key(&mut self, event: &nana_ui_input::KeyInput) -> KeymapMatch {
        if !event.is_pressed() {
            return KeymapMatch::NoMatch;
        }
        let Some(stroke) = CapturedStroke::from_canonical(event) else {
            return KeymapMatch::NoMatch;
        };
        self.keymap
            .resolve(&mut self.state, stroke, &self.context, &self.registry)
    }

    pub fn handle_key(&mut self, event: &nana_ui_input::KeyInput) -> Option<ActionId> {
        match self.resolve_key(event) {
            KeymapMatch::Dispatch(action) => Some(action),
            KeymapMatch::Pending | KeymapMatch::NoMatch => None,
        }
    }
}

impl ComponentView for KeymapLayer {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "keymap-layer".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = KeymapLayerVisual;
        if world.widget_visual_as::<KeymapLayerVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &layer_chrome_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Resolved layout and surface tint for a mounted [`KeyCaptureLayer`].
#[derive(Debug, Clone, PartialEq)]
pub struct KeyCaptureLayerLayout {
    pub badge: ComponentTextRegion,
    pub background: Option<[f32; 4]>,
}

/// Widget visual projected by [`KeyCaptureLayer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCaptureLayerVisual {
    pub recording: bool,
}

impl KeyCaptureLayerVisual {
    fn layout_at(
        &self,
        content: LayoutBox,
        style_model: StyleModelRef,
        measure: crate::text_width::ChromeTextMeasure<'_>,
    ) -> KeyCaptureLayerLayout {
        let label = if self.recording { "Recording" } else { "Idle" };
        KeyCaptureLayerLayout {
            badge: key_badge_region(content, label, !self.recording, style_model, measure),
            background: Some(if self.recording {
                style_model
                    .color(SemanticColorRole::AccentSoft)
                    .as_rgba_array()
            } else {
                style_model.color(SemanticColorRole::Subtle).as_rgba_array()
            }),
        }
    }
}

impl WidgetVisual for KeyCaptureLayerVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn measures_text(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        let layout = self.layout_at(
            ctx.content,
            ctx.style_model,
            ctx.world.chrome_text_measure(ctx.id),
        );
        let mut out = WidgetPaintOutput {
            owns_node_text: true,
            suppress_root_surface: true,
            ..WidgetPaintOutput::new()
        };
        if let Some(background) = layout.background {
            out.push(
                10,
                WidgetPaintOp::Quad(WidgetQuad {
                    rect: layout.badge.bounds,
                    fill: Some(background),
                    fill_paint: ctx.matching_background_paint(background),
                    corner_radius: [ctx.chrome_radii.sm; 4],
                    ..WidgetQuad::default()
                }),
            );
        }
        out.push(
            11,
            WidgetPaintOp::Text(crate::widget_visual::widget_text_region(
                &layout.badge,
                ctx,
                TextHorizontalAlignment::Center,
                false,
            )),
        );
        out
    }
}

/// Resolved layout for a mounted [`KeymapLayer`].
#[derive(Debug, Clone, PartialEq)]
pub struct KeymapLayerLayout {
    pub badge: ComponentTextRegion,
}

/// Widget visual projected by [`KeymapLayer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeymapLayerVisual;

impl KeymapLayerVisual {
    fn layout_at(
        &self,
        content: LayoutBox,
        style_model: StyleModelRef,
        measure: crate::text_width::ChromeTextMeasure<'_>,
    ) -> KeymapLayerLayout {
        KeymapLayerLayout {
            badge: key_badge_region(content, "Keymap", false, style_model, measure),
        }
    }
}

impl WidgetVisual for KeymapLayerVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn measures_text(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        let layout = self.layout_at(
            ctx.content,
            ctx.style_model,
            ctx.world.chrome_text_measure(ctx.id),
        );
        let mut out = WidgetPaintOutput {
            owns_node_text: true,
            suppress_root_surface: true,
            ..WidgetPaintOutput::new()
        };
        out.push(
            10,
            WidgetPaintOp::Quad(WidgetQuad {
                rect: layout.badge.bounds,
                fill: ctx.style.background.or(layout
                    .badge
                    .color
                    .map(|color| [color[0], color[1], color[2], 0.12])),
                corner_radius: [ctx.chrome_radii.sm; 4],
                ..WidgetQuad::default()
            }),
        );
        out.push(
            11,
            WidgetPaintOp::Text(crate::widget_visual::widget_text_region(
                &layout.badge,
                ctx,
                TextHorizontalAlignment::Center,
                false,
            )),
        );
        out
    }
}

fn key_badge_region(
    origin: LayoutBox,
    label: &str,
    muted: bool,
    palette: StyleModelRef,
    measure: crate::text_width::ChromeTextMeasure<'_>,
) -> ComponentTextRegion {
    let metrics = palette.metrics;
    let height = nana_ui_core::ControlSize::Small.height_in(metrics);
    const PAD: f32 = nana_ui_core::space::MD;
    // Four of the widest spacing step: the narrowest badge still has to read
    // as a key cap rather than as a letter with a box round it.
    const MIN_WIDTH: f32 = nana_ui_core::space::XXXL * 4.0;
    let font_size = nana_ui_core::type_scale::META;
    const FONT_WEIGHT: u16 = 600;
    ComponentTextRegion {
        bounds: LayoutBox {
            x: origin.x,
            y: origin.y,
            width: (measure.width(label, font_size, Some(FONT_WEIGHT)) + PAD * 2.0).max(MIN_WIDTH),
            height: height.min(origin.height.max(height)),
        },
        content: TextValue::from(label),
        color: Some(if muted {
            palette.color(SemanticColorRole::Muted).as_rgba_array()
        } else {
            palette.color(SemanticColorRole::Text).as_rgba_array()
        }),
        font_size,
        font_weight: Some(FONT_WEIGHT),
    }
}

impl UiWorld {
    /// Resolves `id`'s key-capture badge region and surface tint when mounted as a [`KeyCaptureLayer`].
    #[must_use]
    pub fn key_capture_layer_layout(&self, id: StableNodeId) -> Option<KeyCaptureLayerLayout> {
        let visual = self.widget_visual_as::<KeyCaptureLayerVisual>(id)?;
        let content = self.component_content_box(id)?;
        let style_model = self.node_style_model(id);
        Some(visual.layout_at(content, style_model, self.chrome_text_measure(id)))
    }

    /// Resolves `id`'s keymap badge region when mounted as a [`KeymapLayer`].
    #[must_use]
    pub fn keymap_layer_layout(&self, id: StableNodeId) -> Option<KeymapLayerLayout> {
        let visual = self.widget_visual_as::<KeymapLayerVisual>(id)?;
        let content = self.component_content_box(id)?;
        let style_model = self.node_style_model(id);
        Some(visual.layout_at(content, style_model, self.chrome_text_measure(id)))
    }
}

fn layer_chrome_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.min_height = Some(LengthSpec::Px(28.0));
    style
}

fn normalize_key_name(value: &str) -> Arc<str> {
    let value = value.trim();
    if value.chars().count() == 1 {
        Arc::from(value.to_lowercase())
    } else {
        Arc::from(value)
    }
}

fn is_named(key: &str, name: &str) -> bool {
    key.trim().eq_ignore_ascii_case(name)
}

fn is_modifier_key(key: &str) -> bool {
    [
        "Alt", "AltGraph", "Control", "Ctrl", "Shift", "Meta", "Hyper", "Super",
    ]
    .iter()
    .any(|name| is_named(key, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;
    use nana_ui_input::{InputModifiers, KeyInput, KeyState, LogicalKey, PhysicalKey};
    use std::borrow::Cow;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn ctrl() -> KeyModifiers {
        KeyModifiers {
            control: true,
            ..KeyModifiers::empty()
        }
    }

    fn press(key: &'static str, modifiers: KeyModifiers) -> KeyInput {
        KeyInput::named(key, key, KeyState::Pressed, modifiers)
    }

    #[test]
    fn keymap_keeps_the_physical_key_and_copies_modifier_bits() {
        let event = KeyInput {
            physical: PhysicalKey(Cow::Borrowed("KeyK")),
            logical: LogicalKey(Cow::Borrowed("k")),
            state: KeyState::Pressed,
            repeat: true,
            modifiers: InputModifiers {
                alt: true,
                control: true,
                meta: true,
                shift: true,
            },
        };
        let stroke = CapturedStroke::from_canonical(&event).expect("logical key");
        assert_eq!(stroke.key.as_ref(), "k");
        assert_eq!(stroke.modifiers, event.modifiers);
        assert_eq!(event.physical.0.as_ref(), "KeyK");

        let release = KeyInput::named(
            "Escape",
            "Escape",
            KeyState::Released,
            InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
        );
        assert!(!release.is_pressed());
        assert_eq!(
            release.modifiers,
            KeyModifiers {
                control: true,
                ..KeyModifiers::empty()
            }
        );
    }

    fn palette_registry() -> ActionRegistry {
        let mut registry = ActionRegistry::new();
        registry
            .register(ActionDescriptor::new("workspace.palette"))
            .expect("action is valid");
        registry
    }

    #[test]
    fn modifier_only_does_not_emit() {
        let mut layer = KeyCaptureLayer::new().recording(true);
        assert_eq!(layer.handle_key(&press("Control", ctrl())), None);
        assert_eq!(layer.pending_modifiers, ctrl());
        assert!(layer.should_consume(&press("Control", ctrl())));
    }

    #[test]
    fn escape_cancels_delete_clears_and_tab_is_ignored() {
        let mut layer = KeyCaptureLayer::new().recording(true);
        assert_eq!(
            layer.handle_key(&press("Escape", KeyModifiers::empty())),
            Some(KeyCaptureEvent::Cancelled)
        );
        assert_eq!(
            layer.handle_key(&press("Delete", KeyModifiers::empty())),
            Some(KeyCaptureEvent::Cleared)
        );
        assert_eq!(
            layer.handle_key(&press("Backspace", KeyModifiers::empty())),
            Some(KeyCaptureEvent::Cleared)
        );
        assert_eq!(layer.handle_key(&press("Tab", KeyModifiers::empty())), None);
        assert!(!layer.should_consume(&press("Tab", KeyModifiers::empty())));
    }

    #[test]
    fn captures_ctrl_k_when_recording() {
        let mut layer = KeyCaptureLayer::new().recording(true);
        assert_eq!(
            layer.handle_key(&press("K", ctrl())),
            Some(KeyCaptureEvent::Captured(CapturedStroke::new("k", ctrl())))
        );

        layer.set_recording(false);
        assert_eq!(layer.handle_key(&press("K", ctrl())), None);
    }

    #[test]
    fn keymap_hits_bound_shortcut_and_misses_unbound() {
        let mut layer = KeymapLayer::new(
            Keymap::new([KeyBinding::new(
                "workspace.palette",
                CapturedStroke::new("k", ctrl()),
            )]),
            KeyContext::new(["workspace"]),
            palette_registry(),
        );
        assert_eq!(
            layer.handle_key(&press("k", ctrl())),
            Some(ActionId::new("workspace.palette"))
        );
        assert_eq!(
            layer.resolve_key(&press("k", ctrl())),
            KeymapMatch::Dispatch(ActionId::new("workspace.palette"))
        );
        assert_eq!(layer.handle_key(&press("x", KeyModifiers::empty())), None);
        assert_eq!(
            layer.resolve_key(&press("x", KeyModifiers::empty())),
            KeymapMatch::NoMatch
        );
        let mut typed = press("k", ctrl());
        typed.physical = PhysicalKey(std::borrow::Cow::Borrowed("KeyK"));
        assert_eq!(
            layer.handle_key(&typed),
            Some(ActionId::new("workspace.palette"))
        );
        typed.state = KeyState::Released;
        assert_eq!(layer.resolve_key(&typed), KeymapMatch::NoMatch);
    }

    #[test]
    fn disabled_action_is_not_dispatched() {
        let mut registry = palette_registry();
        registry.set_enabled(&ActionId::new("workspace.palette"), false);
        let mut layer = KeymapLayer::new(
            Keymap::new([KeyBinding::new(
                "workspace.palette",
                CapturedStroke::new("k", ctrl()),
            )]),
            KeyContext::default(),
            registry,
        );
        assert_eq!(layer.handle_key(&press("k", ctrl())), None);
        assert_eq!(layer.resolve_key(&press("k", ctrl())), KeymapMatch::NoMatch);
    }

    #[test]
    fn capture_layer_projects_recording_visual() {
        let mut context = AppContext::new();
        let idle = context
            .create_component(document(), KeyCaptureLayer::new())
            .unwrap();
        let idle_id = idle.stable_id();
        assert!(matches!(
            context.world().node(idle_id).map(|node| node.kind),
            Some(NodeKind::Element { tag }) if tag == "key-capture-layer"
        ));
        assert_eq!(
            context
                .world()
                .widget_visual_as::<KeyCaptureLayerVisual>(idle_id),
            Some(&KeyCaptureLayerVisual { recording: false })
        );
        assert_eq!(
            context.world().interaction(idle_id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        assert_eq!(
            context
                .world()
                .accessibility(idle_id)
                .and_then(|state| state.label.clone())
                .as_deref(),
            Some("Shortcut capture")
        );
        assert_eq!(
            context
                .world()
                .node_style(idle_id)
                .and_then(|style| style.layout.min_height),
            Some(LengthSpec::Px(28.0))
        );

        let recording = context
            .create_component(document(), KeyCaptureLayer::new().recording(true))
            .unwrap();
        let recording_id = recording.stable_id();
        assert_eq!(
            context
                .world()
                .widget_visual_as::<KeyCaptureLayerVisual>(recording_id),
            Some(&KeyCaptureLayerVisual { recording: true })
        );
        assert_eq!(
            context.world().interaction(recording_id),
            Some(InteractionState {
                pointer_events: false,
                focusable: true,
            })
        );
        assert_eq!(
            context
                .world()
                .accessibility(recording_id)
                .and_then(|state| state.label.clone())
                .as_deref(),
            Some("Recording shortcut")
        );
    }

    #[test]
    fn keymap_layer_projects_visual() {
        let mut context = AppContext::new();
        let layer = context
            .create_component(document(), KeymapLayer::default())
            .unwrap();
        let id = layer.stable_id();
        assert!(matches!(
            context.world().node(id).map(|node| node.kind),
            Some(NodeKind::Element { tag }) if tag == "keymap-layer"
        ));
        assert_eq!(
            context.world().widget_visual_as::<KeymapLayerVisual>(id),
            Some(&KeymapLayerVisual)
        );
        assert_eq!(
            context.world().interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        assert_eq!(
            context
                .world()
                .node_style(id)
                .and_then(|style| style.layout.min_height),
            Some(LengthSpec::Px(28.0))
        );
    }

    #[test]
    fn normalized_action_ids_match_across_registry_keymap_and_menu_map() {
        let mut registry = ActionRegistry::new();
        assert_eq!(
            registry.register(ActionDescriptor::new("   \t\n")),
            Err(ActionRegistryError::EmptyId)
        );
        registry
            .register(ActionDescriptor::labeled("  edit.undo \n", "撤销"))
            .unwrap();
        assert_eq!(
            registry.register(ActionDescriptor::new("edit.undo")),
            Err(ActionRegistryError::Duplicate {
                id: ActionId::new("edit.undo"),
            })
        );

        let untrimmed = ActionId::new("  edit.undo ");
        let canonical = ActionId::new("edit.undo");
        assert_eq!(
            registry.get(&untrimmed).map(|d| d.id.clone()),
            Some(canonical.clone())
        );
        assert!(registry.is_available(&untrimmed, &KeyContext::default()));
        assert!(registry.set_label(&untrimmed, "撤销操作"));
        assert_eq!(
            registry.resolved_label(&canonical, &KeyContext::default()),
            Some("撤销操作".to_owned())
        );

        let keymap = Keymap::new([KeyBinding::new(
            "  edit.undo ",
            CapturedStroke::new("z", ctrl()),
        )]);
        let mut state = KeymapState::default();
        assert_eq!(
            keymap.resolve(
                &mut state,
                CapturedStroke::new("z", ctrl()),
                &KeyContext::default(),
                &registry,
            ),
            KeymapMatch::Dispatch(canonical.clone())
        );
        assert_eq!(
            keymap.binding_label(&untrimmed, &KeyContext::default(), &registry),
            Some("Ctrl+Z".to_owned())
        );

        let mut menu_map = MenuActionMap::new();
        assert_eq!(
            menu_map.bind(1, "   "),
            Err(MenuActionMapError::EmptyActionId)
        );
        menu_map.bind(10, " edit.undo ").unwrap();
        assert_eq!(
            menu_map.bind(10, "edit.redo"),
            Err(MenuActionMapError::DuplicateMenuId(10))
        );
        assert_eq!(
            menu_map.bind(11, "edit.undo"),
            Err(MenuActionMapError::DuplicateActionId(canonical.clone()))
        );
        assert_eq!(menu_map.action_for(10), Some(&canonical));
        assert_eq!(menu_map.menu_id_for(&untrimmed), Some(10));
        assert!(registry.set_enabled(&untrimmed, false));
        assert!(!registry.is_available(&canonical, &KeyContext::default()));
        assert_eq!(menu_map.unbind_action(&untrimmed), Some(10));
        assert!(registry.unregister(&untrimmed).is_some());
    }

    #[test]
    fn dynamic_enabled_and_label_stay_consistent_across_palette_action_menu_and_menu_bar() {
        let can_undo = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let undo_target = Arc::new(std::sync::Mutex::new(String::from("移动节点")));
        let mut registry = ActionRegistry::new();
        registry
            .register(
                ActionDescriptor::labeled("edit.undo", "撤销")
                    .category("编辑")
                    .when(ContextPredicate::always().none_of([CONTEXT_MODAL])),
            )
            .unwrap();
        let flag = Arc::clone(&can_undo);
        assert!(
            registry.bind_enabled(&ActionId::new(" edit.undo "), move |_| {
                flag.load(std::sync::atomic::Ordering::Relaxed)
            })
        );
        let target = Arc::clone(&undo_target);
        assert!(
            registry.bind_label(&ActionId::new("edit.undo"), move |ctx| {
                let suffix = target.lock().unwrap().clone();
                if ctx.contains("canvas") {
                    format!("撤销: {suffix}")
                } else {
                    format!("撤销（全局）: {suffix}")
                }
            })
        );

        let primary = KeyModifiers::primary();
        let keymap = Keymap::new([KeyBinding::new(
            "edit.undo",
            CapturedStroke::new("z", primary),
        )]);
        let mut menu_map = MenuActionMap::new();
        menu_map.bind(101, "edit.undo").unwrap();

        let canvas = KeyContext::new(["canvas"]);
        let mut bar = nana_ui_core::MenuBar::new([nana_ui_core::Menu::new(
            "编辑",
            [
                nana_ui_core::MenuEntry::item(101, "占位"),
                nana_ui_core::MenuEntry::Submenu(nana_ui_core::Menu::new(
                    "更多",
                    [nana_ui_core::MenuEntry::item(101, "占位子项")],
                )),
            ],
        )]);

        // Initially disabled dynamically.
        assert!(!registry.is_available(&ActionId::new("edit.undo"), &canvas));
        assert!(
            registry
                .palette_items("", &canvas, Some(&keymap))
                .is_empty()
        );
        let menu_item = registry
            .action_menu_item(&ActionId::new("edit.undo"), &canvas, Some(&keymap))
            .unwrap();
        assert_eq!(menu_item.label.as_ref(), "撤销: 移动节点");
        assert!(menu_item.disabled);
        assert!(menu_item.hint.is_some());

        menu_map.sync_menu_bar(&mut bar, &registry, &canvas, Some(&keymap));
        match &bar.menus[0].entries[0] {
            nana_ui_core::MenuEntry::Item {
                label,
                enabled,
                shortcut,
                ..
            } => {
                assert_eq!(label, "撤销: 移动节点");
                assert!(!enabled);
                assert!(shortcut.is_some());
            }
            other => panic!("expected item, got {other:?}"),
        }

        // Enable dynamically and update dynamic target name.
        can_undo.store(true, std::sync::atomic::Ordering::Relaxed);
        *undo_target.lock().unwrap() = String::from("改色");
        assert!(registry.is_available(&ActionId::new("edit.undo"), &canvas));

        // Search matches the live dynamic label ("改色").
        let palette = registry.palette_items("改色", &canvas, Some(&keymap));
        assert_eq!(palette.len(), 1);
        assert_eq!(palette[0].label, "撤销: 改色");
        assert_eq!(palette[0].category.as_deref(), Some("编辑"));
        assert!(palette[0].shortcut.is_some());

        let menu_item = registry
            .action_menu_item(&ActionId::new("edit.undo"), &canvas, Some(&keymap))
            .unwrap();
        assert_eq!(menu_item.label.as_ref(), "撤销: 改色");
        assert!(!menu_item.disabled);

        // Nested submenu item also syncs.
        let global = KeyContext::default();
        menu_map.sync_menu_bar(&mut bar, &registry, &global, Some(&keymap));
        match &bar.menus[0].entries[1] {
            nana_ui_core::MenuEntry::Submenu(sub) => match &sub.entries[0] {
                nana_ui_core::MenuEntry::Item { label, enabled, .. } => {
                    assert_eq!(label, "撤销（全局）: 改色");
                    assert!(*enabled);
                }
                other => panic!("expected nested item, got {other:?}"),
            },
            other => panic!("expected submenu, got {other:?}"),
        }

        // Clearing bindings falls back to static descriptor fields.
        assert!(registry.clear_enabled_binding(&ActionId::new("edit.undo")));
        assert!(registry.clear_label_binding(&ActionId::new("edit.undo")));
        assert_eq!(
            registry.resolved_label(&ActionId::new("edit.undo"), &canvas),
            Some("撤销".to_owned())
        );
    }
}
