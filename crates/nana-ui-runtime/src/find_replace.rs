//! Generic find and replace toolbar.
//!
//! The bar owns only editing chrome and emits [`FindReplaceEvent`] requests.
//! The host remains responsible for searching documents and applying edits.
//! Query and replacement fields are retained `TextInput` children, so IME,
//! selection and accessibility follow the regular input path.

use std::sync::Arc;

use nana_ui_core::{ButtonKind, ControlSize, FlexDirection, LengthSpec};

use crate::component_registry::{RegisterableComponent, SemanticSpec};
use crate::view_components::{Activate, Button, Stack, Text, TextChanged, TextInput};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, Entity, FrameworkError,
    InteractionState, MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
    view_components::project_common,
};

/// Requests emitted by [`FindReplaceBar`].
///
/// Search and replacement are host-owned operations. The component emits the
/// current draft for text changes and an intent for each toolbar action; it
/// never reads or mutates a document itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindReplaceEvent {
    QueryChanged(String),
    ReplacementChanged(String),
    Previous,
    Next,
    Replace,
    ReplaceAll,
    Expanded(bool),
}

/// Find-only / find-and-replace toolbar.
///
/// `read_only` keeps navigation available while disabling replacement input
/// and actions. `expanded` controls whether the replacement row is visible.
/// Call [`AppContext::assemble_find_replace_bar`] after creation when using a
/// detached component or after manually binding one into an existing node.
#[derive(Debug, Clone, PartialEq, Default)]
struct RetainedChildren {
    toggle: Option<StableNodeId>,
    panel: Option<StableNodeId>,
    query_row: Option<StableNodeId>,
    replacement_row: Option<StableNodeId>,
    query_input: Option<StableNodeId>,
    replacement_input: Option<StableNodeId>,
    feedback: Option<StableNodeId>,
    previous: Option<StableNodeId>,
    next: Option<StableNodeId>,
    replace: Option<StableNodeId>,
    replace_all: Option<StableNodeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FindReplaceBar {
    pub query: String,
    pub replacement: String,
    pub expanded: bool,
    pub read_only: bool,
    pub disabled: bool,
    pub feedback: Arc<str>,
    pub query_placeholder: Arc<str>,
    pub replacement_placeholder: Arc<str>,
    pub label: Arc<str>,
    pub style: NodeStyle,
    retained: RetainedChildren,
}

impl FindReplaceBar {
    pub fn new() -> Self {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Column);
        layout.gap = Some(LengthSpec::Px(nana_ui_core::space::XS));
        layout.width = Some(LengthSpec::Fill);
        layout.min_width = Some(LengthSpec::Px(0.0));
        Self {
            query: String::new(),
            replacement: String::new(),
            expanded: false,
            read_only: false,
            disabled: false,
            feedback: Arc::from(""),
            query_placeholder: Arc::from("查找"),
            replacement_placeholder: Arc::from("替换为"),
            label: Arc::from("查找替换"),
            style,
            retained: RetainedChildren::default(),
        }
    }

    pub fn query(mut self, query: impl Into<String>) -> Self {
        self.query = query.into();
        self
    }

    pub fn replacement(mut self, replacement: impl Into<String>) -> Self {
        self.replacement = replacement.into();
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn feedback(mut self, feedback: impl Into<Arc<str>>) -> Self {
        self.feedback = feedback.into();
        self
    }

    pub fn query_placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.query_placeholder = placeholder.into();
        self
    }

    pub fn replacement_placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.replacement_placeholder = placeholder.into();
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = label.into();
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn toggle_id(&self) -> Option<StableNodeId> {
        self.retained.toggle
    }

    pub fn query_input_id(&self) -> Option<StableNodeId> {
        self.retained.query_input
    }

    fn toggle_label(&self) -> &'static str {
        if self.expanded {
            "收起查找"
        } else if self.read_only {
            "查找"
        } else {
            "查找替换"
        }
    }
}

impl Default for FindReplaceBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Short alias for consumers that expose this control as a text search bar.
pub type TextSearchBar = FindReplaceBar;

impl ComponentView for FindReplaceBar {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(crate::AppContext::assemble_find_replace_bar),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        next.retained = self.retained.clone();
        *self = next;
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "find-replace-bar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Toolbar,
                label: Some(Arc::clone(&self.label)),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

impl RegisterableComponent for FindReplaceBar {
    const TYPE_ID: &'static str = crate::component_descriptors::FIND_REPLACE_BAR.type_id;
    const TAGS: &'static [&'static str] = crate::component_descriptors::FIND_REPLACE_BAR.tags;
    const RETAIN_SEMANTIC_STATE: bool = true;

    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut bar = Self::new()
            .query(spec.value)
            .read_only(spec.read_only)
            .disabled(spec.disabled)
            .expanded(spec.active);
        if !spec.placeholder.is_empty() {
            bar.query_placeholder = Arc::from(spec.placeholder);
        }
        if !spec.label.is_empty() {
            bar.label = Arc::from(spec.label);
        }
        if let Some(replacement) = spec.attr("replacement") {
            bar.replacement = replacement.to_owned();
        }
        if let Some(placeholder) = spec.attr("replacement-placeholder") {
            bar.replacement_placeholder = Arc::from(placeholder);
        }
        bar.style.layout = Arc::clone(spec.layout);
        bar
    }

    fn reconcile_semantic(spec: &SemanticSpec<'_>, previous: Option<&Self>) -> Self {
        let mut next = Self::from_semantic(spec);
        if let Some(previous) = previous {
            next.retained = previous.retained.clone();
        }
        next
    }

    fn finish_semantic(
        context: &mut AppContext,
        entity: Entity<Self>,
    ) -> Result<(), FrameworkError> {
        context.assemble_find_replace_bar(entity).map(|_| ())
    }
}

impl AppContext {
    /// Idempotently build the bar's retained children and synchronize them.
    pub fn assemble_find_replace_bar(
        &mut self,
        bar: Entity<FindReplaceBar>,
    ) -> Result<bool, FrameworkError> {
        let document = self
            .world()
            .node(bar.stable_id())
            .ok_or(FrameworkError::MissingView(bar.stable_id()))?
            .document;
        let snapshot = self.read(bar, Clone::clone)?;
        let created = snapshot.retained.toggle.is_none();

        let (toggle, toggle_created) =
            self.ensure_child(document, snapshot.retained.toggle, || {
                Button::new(snapshot.toggle_label())
                    .kind(ButtonKind::Text)
                    .size(ControlSize::Small)
                    .disabled(snapshot.disabled)
            })?;
        let (panel, panel_created) =
            self.ensure_child(document, snapshot.retained.panel, || {
                let mut style = NodeStyle::default();
                let layout = Arc::make_mut(&mut style.layout);
                layout.direction = Some(FlexDirection::Column);
                layout.gap = Some(LengthSpec::Px(nana_ui_core::space::XS));
                layout.width = Some(LengthSpec::Fill);
                layout.min_width = Some(LengthSpec::Px(0.0));
                Stack::from_layout(style.layout.as_ref().clone())
            })?;
        let (query_row, query_row_created) =
            self.ensure_child(document, snapshot.retained.query_row, || Stack::bar(4.0))?;
        let (replacement_row, replacement_row_created) =
            self.ensure_child(document, snapshot.retained.replacement_row, || {
                Stack::bar(4.0)
            })?;
        let (query, query_created) =
            self.ensure_child(document, snapshot.retained.query_input, || {
                search_input(&snapshot.query, &snapshot.query_placeholder)
                    .label(snapshot.query_placeholder.as_ref())
            })?;
        let (replacement, replacement_created) =
            self.ensure_child(document, snapshot.retained.replacement_input, || {
                search_input(&snapshot.replacement, &snapshot.replacement_placeholder)
                    .label(snapshot.replacement_placeholder.as_ref())
            })?;
        let (feedback, feedback_created) =
            self.ensure_child(document, snapshot.retained.feedback, || {
                Text::new(snapshot.feedback.as_ref())
            })?;
        let (previous, previous_created) =
            self.ensure_button(document, snapshot.retained.previous, "上一处")?;
        let (next, next_created) =
            self.ensure_button(document, snapshot.retained.next, "下一处")?;
        let (replace, replace_created) =
            self.ensure_button(document, snapshot.retained.replace, "替换")?;
        let (replace_all, replace_all_created) =
            self.ensure_button(document, snapshot.retained.replace_all, "全部替换")?;

        if query_created {
            self.observe(query, bar, |bar, event: &TextChanged, cx| {
                let value = event.value.to_string();
                bar.query = value.clone();
                cx.emit(FindReplaceEvent::QueryChanged(value));
            })?;
        }
        if replacement_created {
            self.observe(replacement, bar, |bar, event: &TextChanged, cx| {
                let value = event.value.to_string();
                bar.replacement = value.clone();
                cx.emit(FindReplaceEvent::ReplacementChanged(value));
            })?;
        }
        if toggle_created {
            self.observe(toggle, bar, |bar, _: &Activate, cx| {
                if bar.disabled {
                    return;
                }
                bar.expanded = !bar.expanded;
                if let Some(panel) = bar.retained.panel {
                    let parent = cx.entity().stable_id();
                    if bar.expanded {
                        cx.mutations().insert(parent, panel, None);
                    } else {
                        cx.mutations().park_subtree(panel);
                    }
                }
                cx.reassemble();
                cx.emit(FindReplaceEvent::Expanded(bar.expanded));
            })?;
        }
        if previous_created {
            self.observe(previous, bar, |bar, _: &Activate, cx| {
                if !bar.disabled {
                    cx.emit(FindReplaceEvent::Previous);
                }
            })?;
        }
        if next_created {
            self.observe(next, bar, |bar, _: &Activate, cx| {
                if !bar.disabled {
                    cx.emit(FindReplaceEvent::Next);
                }
            })?;
        }
        if replace_created {
            self.observe(replace, bar, |bar, _: &Activate, cx| {
                if !bar.disabled && !bar.read_only {
                    cx.emit(FindReplaceEvent::Replace);
                }
            })?;
        }
        if replace_all_created {
            self.observe(replace_all, bar, |bar, _: &Activate, cx| {
                if !bar.disabled && !bar.read_only {
                    cx.emit(FindReplaceEvent::ReplaceAll);
                }
            })?;
        }

        self.update_component(toggle, |button, _| {
            *button = Button::new(snapshot.toggle_label())
                .kind(ButtonKind::Text)
                .size(ControlSize::Small)
                .disabled(snapshot.disabled);
        })?;
        self.update_component(query, |input, _| {
            if input.state.value.as_ref() != snapshot.query.as_str() {
                input.state.replace_value(snapshot.query.clone());
            }
            input.placeholder = Arc::clone(&snapshot.query_placeholder);
            input.disabled = snapshot.disabled;
            input.read_only = false;
        })?;
        self.update_component(replacement, |input, _| {
            if input.state.value.as_ref() != snapshot.replacement.as_str() {
                input.state.replace_value(snapshot.replacement.clone());
            }
            input.placeholder = Arc::clone(&snapshot.replacement_placeholder);
            input.disabled = snapshot.disabled || snapshot.read_only;
            input.read_only = snapshot.read_only;
        })?;
        for (button, label, disabled) in [
            (previous, "上一处", snapshot.disabled),
            (next, "下一处", snapshot.disabled),
            (replace, "替换", snapshot.disabled || snapshot.read_only),
            (
                replace_all,
                "全部替换",
                snapshot.disabled || snapshot.read_only,
            ),
        ] {
            self.update_component(button, |button, _| {
                *button = Button::new(label)
                    .kind(ButtonKind::Subtle)
                    .size(ControlSize::Small)
                    .disabled(disabled);
            })?;
        }
        self.update_component(feedback, |text, _| {
            *text = Text::new(snapshot.feedback.as_ref());
        })?;

        self.update_component(bar, |bar, _| {
            bar.retained = RetainedChildren {
                toggle: Some(toggle.stable_id()),
                panel: Some(panel.stable_id()),
                query_row: Some(query_row.stable_id()),
                replacement_row: Some(replacement_row.stable_id()),
                query_input: Some(query.stable_id()),
                replacement_input: Some(replacement.stable_id()),
                feedback: Some(feedback.stable_id()),
                previous: Some(previous.stable_id()),
                next: Some(next.stable_id()),
                replace: Some(replace.stable_id()),
                replace_all: Some(replace_all.stable_id()),
            };
        })?;

        self.append_children(bar.stable_id(), &[toggle.stable_id(), panel.stable_id()])?;
        self.append_children(
            panel.stable_id(),
            &[
                query_row.stable_id(),
                replacement_row.stable_id(),
                feedback.stable_id(),
            ],
        )?;
        self.append_children(
            query_row.stable_id(),
            &[query.stable_id(), previous.stable_id(), next.stable_id()],
        )?;
        self.append_children(
            replacement_row.stable_id(),
            &[
                replacement.stable_id(),
                replace.stable_id(),
                replace_all.stable_id(),
            ],
        )?;
        let mut visibility = MutationQueue::new();
        if snapshot.expanded {
            visibility.insert(bar.stable_id(), panel.stable_id(), None);
        } else {
            visibility.park_subtree(panel.stable_id());
        }
        self.commit_mutations(visibility)?;
        Ok(created
            || toggle_created
            || panel_created
            || query_row_created
            || replacement_row_created
            || query_created
            || replacement_created
            || feedback_created
            || previous_created
            || next_created
            || replace_created
            || replace_all_created)
    }

    fn ensure_child<C: ComponentView>(
        &mut self,
        document: crate::DocumentId,
        existing: Option<StableNodeId>,
        build: impl FnOnce() -> C,
    ) -> Result<(Entity<C>, bool), FrameworkError> {
        match existing.filter(|id| self.world().contains(*id)) {
            Some(id) => Ok((Entity::from_stable_id(id), false)),
            None => Ok((self.create_detached_component(document, build())?, true)),
        }
    }

    fn ensure_button(
        &mut self,
        document: crate::DocumentId,
        existing: Option<StableNodeId>,
        label: &str,
    ) -> Result<(Entity<Button>, bool), FrameworkError> {
        self.ensure_child(document, existing, || {
            Button::new(label)
                .kind(ButtonKind::Subtle)
                .size(ControlSize::Small)
        })
    }
}

fn search_input(value: &str, placeholder: &str) -> TextInput {
    let mut input = TextInput::new(value.to_owned()).placeholder(placeholder.to_owned());
    let layout = Arc::make_mut(&mut input.style.layout);
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;

    #[test]
    fn defaults_to_collapsed_find_only_chrome() {
        let bar = FindReplaceBar::new();
        assert!(!bar.expanded);
        assert!(!bar.read_only);
        assert_eq!(bar.query_placeholder.as_ref(), "查找");
    }

    #[test]
    fn assembly_retains_children_and_read_only_gates_replacement() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let bar = context
            .create_component(document, FindReplaceBar::new().read_only(true))
            .unwrap();
        assert!(context.assemble_find_replace_bar(bar).unwrap());
        let snapshot = context.read(bar, Clone::clone).unwrap();
        assert!(snapshot.retained.panel.is_some());
        assert!(snapshot.retained.replacement_input.is_some());
        assert!(!context.assemble_find_replace_bar(bar).unwrap());
        let replacement =
            Entity::<TextInput>::from_stable_id(snapshot.retained.replacement_input.unwrap());
        assert!(context.read(replacement, |input| input.disabled).unwrap());
    }

    #[test]
    fn replacing_a_semantic_snapshot_keeps_retained_children() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let bar = context
            .create_component(document, FindReplaceBar::new())
            .unwrap();
        context.assemble_find_replace_bar(bar).unwrap();
        let before = context.read(bar, Clone::clone).unwrap().retained;

        let next = FindReplaceBar::new().query("next");
        context.set_component(bar, next).unwrap();
        let after = context.read(bar, Clone::clone).unwrap().retained;
        assert_eq!(after, before);
    }

    #[test]
    fn partial_child_recreation_rewires_only_the_new_observer() {
        use std::sync::{Arc, Mutex};

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let bar = context
            .create_component(document, FindReplaceBar::new())
            .unwrap();
        context.assemble_find_replace_bar(bar).unwrap();
        let events = Arc::new(Mutex::new(Vec::<FindReplaceEvent>::new()));
        let observed = Arc::clone(&events);
        context
            .on(bar, move |_, event: &FindReplaceEvent, _| {
                observed.lock().unwrap().push(event.clone());
            })
            .unwrap();

        let old_query = context
            .read(bar, |bar| bar.retained.query_input)
            .unwrap()
            .unwrap();
        context.despawn_node(old_query).unwrap();
        context.assemble_find_replace_bar(bar).unwrap();
        let new_query = context
            .read(bar, |bar| bar.retained.query_input)
            .unwrap()
            .unwrap();
        assert_ne!(old_query, new_query);

        let query = Entity::<TextInput>::from_stable_id(new_query);
        context
            .update_component(query, |input, cx| {
                input.state.replace_value("new");
                cx.emit(TextChanged {
                    value: input.state.value.clone(),
                    selection: input.state.selection,
                });
            })
            .unwrap();
        assert_eq!(context.read(bar, |bar| bar.query.clone()).unwrap(), "new");
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [FindReplaceEvent::QueryChanged("new".to_owned())]
        );
    }

    #[test]
    fn repeated_assembly_does_not_duplicate_observers() {
        use std::sync::{Arc, Mutex};

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let bar = context
            .create_component(document, FindReplaceBar::new())
            .unwrap();
        context.assemble_find_replace_bar(bar).unwrap();
        let events = Arc::new(Mutex::new(Vec::<FindReplaceEvent>::new()));
        let observed = Arc::clone(&events);
        context
            .on(bar, move |_, event: &FindReplaceEvent, _| {
                observed.lock().unwrap().push(event.clone());
            })
            .unwrap();
        context.assemble_find_replace_bar(bar).unwrap();

        let query = Entity::<TextInput>::from_stable_id(
            context
                .read(bar, |bar| bar.retained.query_input)
                .unwrap()
                .unwrap(),
        );
        context
            .update_component(query, |input, cx| {
                input.state.replace_value("once");
                cx.emit(TextChanged {
                    value: input.state.value.clone(),
                    selection: input.state.selection,
                });
            })
            .unwrap();
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [FindReplaceEvent::QueryChanged("once".to_owned())]
        );
    }
}
