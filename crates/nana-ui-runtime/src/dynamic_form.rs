//! Keyed, data-driven form surface.
//!
//! [`DynamicForm`] is the small amount of form assembly that tends to be
//! repeated by settings, extension and approval surfaces.  The descriptors
//! carry values and presentation metadata only; application state, validation
//! and actions stay in the consumer.  Fields are assembled by their stable
//! `id`, so changing a value keeps the editor (and its focus/undo history)
//! alive while inserting or removing a neighbouring field.

use std::collections::HashMap;
use std::sync::Arc;

use nana_ui_core::{ButtonKind, DropdownEvent, FlexDirection, LengthSpec};

use crate::framework::AssemblyScope;
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AppContext, Button, ComponentView, Dropdown,
    DropdownOption, Entity, FrameworkError, InteractionState, MutationQueue, NodeKind, NodeStyle,
    SettingsCard, SettingsRow, StableNodeId, Stack, Switch, Text, TextArea, TextChanged, TextInput,
    ToggleChanged, UiWorld, ValidationIntent, ValidationMessage,
};

/// A value-independent event emitted by a [`DynamicForm`] child.
///
/// The field id is the descriptor's stable key.  The form never interprets
/// the value or performs an application action; an owner listens to this event
/// and updates its own state before writing a new descriptor snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynamicFormEvent {
    Activate { field: Arc<str> },
    Toggle { field: Arc<str>, checked: bool },
    Select { field: Arc<str>, value: Arc<str> },
    Input { field: Arc<str>, value: String },
}

/// Descriptor for one row of a [`DynamicForm`].
///
/// `Section` starts a titled group; following descriptors belong to that group
/// until the next section.  IDs are application-owned, but must be non-empty
/// and must not contain `/` because they are used as retained assembly keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynamicFormField {
    Section {
        id: Arc<str>,
        label: Arc<str>,
    },
    Toggle {
        id: Arc<str>,
        label: Arc<str>,
        checked: bool,
        disabled: bool,
    },
    Choice {
        id: Arc<str>,
        label: Arc<str>,
        selected: Arc<str>,
        options: Vec<DynamicFormOption>,
        disabled: bool,
        hint: Option<Arc<str>>,
    },
    Field {
        id: Arc<str>,
        label: Arc<str>,
        value: String,
        multiline: bool,
        secure: bool,
        disabled: bool,
        identity: Option<Arc<str>>,
        hint: Option<Arc<str>>,
    },
    Text {
        id: Arc<str>,
        value: Arc<str>,
    },
    Error {
        id: Arc<str>,
        value: Arc<str>,
    },
    Action {
        id: Arc<str>,
        label: Arc<str>,
        enabled: bool,
        danger: bool,
    },
}

/// One option for [`DynamicFormField::Choice`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicFormOption {
    pub value: Arc<str>,
    pub label: Arc<str>,
    pub disabled: bool,
}

impl DynamicFormOption {
    pub fn new(value: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl DynamicFormField {
    pub fn section(id: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self::Section {
            id: id.into(),
            label: label.into(),
        }
    }

    pub fn toggle(id: impl Into<Arc<str>>, label: impl Into<Arc<str>>, checked: bool) -> Self {
        Self::Toggle {
            id: id.into(),
            label: label.into(),
            checked,
            disabled: false,
        }
    }

    pub fn choice(
        id: impl Into<Arc<str>>,
        label: impl Into<Arc<str>>,
        selected: impl Into<Arc<str>>,
        options: impl IntoIterator<Item = DynamicFormOption>,
    ) -> Self {
        Self::Choice {
            id: id.into(),
            label: label.into(),
            selected: selected.into(),
            options: options.into_iter().collect(),
            disabled: false,
            hint: None,
        }
    }

    pub fn field(
        id: impl Into<Arc<str>>,
        label: impl Into<Arc<str>>,
        value: impl Into<String>,
    ) -> Self {
        Self::Field {
            id: id.into(),
            label: label.into(),
            value: value.into(),
            multiline: false,
            secure: false,
            disabled: false,
            identity: None,
            hint: None,
        }
    }

    pub fn text(id: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        Self::Text {
            id: id.into(),
            value: value.into(),
        }
    }

    pub fn error(id: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        Self::Error {
            id: id.into(),
            value: value.into(),
        }
    }

    pub fn action(id: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self::Action {
            id: id.into(),
            label: label.into(),
            enabled: true,
            danger: false,
        }
    }

    pub fn id(&self) -> &Arc<str> {
        match self {
            Self::Section { id, .. }
            | Self::Toggle { id, .. }
            | Self::Choice { id, .. }
            | Self::Field { id, .. }
            | Self::Text { id, .. }
            | Self::Error { id, .. }
            | Self::Action { id, .. } => id,
        }
    }

    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        let hint = Some(hint.into());
        match &mut self {
            Self::Choice { hint: current, .. } | Self::Field { hint: current, .. } => {
                *current = hint
            }
            _ => {}
        }
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        match &mut self {
            Self::Toggle {
                disabled: current, ..
            }
            | Self::Choice {
                disabled: current, ..
            }
            | Self::Field {
                disabled: current, ..
            } => *current = disabled,
            _ => {}
        }
        self
    }

    pub fn multiline(mut self, multiline: bool) -> Self {
        if let Self::Field {
            multiline: current, ..
        } = &mut self
        {
            *current = multiline;
        }
        self
    }

    pub fn secure(mut self, secure: bool) -> Self {
        if let Self::Field {
            secure: current, ..
        } = &mut self
        {
            *current = secure;
        }
        self
    }

    pub fn binding_identity(mut self, identity: impl Into<Arc<str>>) -> Self {
        if let Self::Field {
            identity: current, ..
        } = &mut self
        {
            *current = Some(identity.into());
        }
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        if let Self::Action {
            enabled: current, ..
        } = &mut self
        {
            *current = enabled;
        }
        self
    }

    pub fn danger(mut self, danger: bool) -> Self {
        if let Self::Action {
            danger: current, ..
        } = &mut self
        {
            *current = danger;
        }
        self
    }
}

/// A retained form assembled from stable field descriptors.
#[derive(Debug, Clone, PartialEq)]
pub struct DynamicForm {
    pub fields: Vec<DynamicFormField>,
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
    /// Root nodes in descriptor order. This is public so a host can inspect or
    /// attach additional chrome without reaching into Runtime storage.
    pub(crate) field_nodes: Vec<(Arc<str>, StableNodeId)>,
    #[doc(hidden)]
    pub(crate) wired_nodes: Vec<StableNodeId>,
    #[doc(hidden)]
    pub(crate) editor_identities: HashMap<Arc<str>, Option<Arc<str>>>,
}

impl DynamicForm {
    pub fn new(fields: impl IntoIterator<Item = DynamicFormField>) -> Self {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.direction = Some(FlexDirection::Column);
        layout.gap = Some(LengthSpec::Px(nana_ui_core::space::SM));
        Self {
            fields: fields.into_iter().collect(),
            label: None,
            style,
            field_nodes: Vec::new(),
            wired_nodes: Vec::new(),
            editor_identities: HashMap::new(),
        }
    }

    pub fn fields(mut self, fields: impl IntoIterator<Item = DynamicFormField>) -> Self {
        self.fields = fields.into_iter().collect();
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Current descriptor-root mapping. IDs remain stable while a descriptor
    /// of the same kind is updated.
    pub fn field_nodes(&self) -> &[(Arc<str>, StableNodeId)] {
        &self.field_nodes
    }
}

impl Default for DynamicForm {
    fn default() -> Self {
        Self::new([])
    }
}

impl ComponentView for DynamicForm {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(AppContext::assemble_dynamic_form),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, next: Self) {
        // The descriptor snapshot is declarative, while these three fields
        // are retained assembly state. Replacing them on every host update
        // would duplicate observers and discard the editor identity journal.
        self.fields = next.fields;
        self.label = next.label;
        self.style = next.style;
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "dynamic-form".into(),
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
                role: AccessibilityRole::Form,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Clone, Copy)]
enum FormControl {
    Toggle(Entity<Switch>),
    Choice(Entity<Dropdown>),
    Input(Entity<TextInput>),
    Multiline(Entity<TextArea>),
    Action(Entity<Button>),
}

impl FormControl {
    fn node(self) -> StableNodeId {
        match self {
            Self::Toggle(entity) => entity.stable_id(),
            Self::Choice(entity) => entity.stable_id(),
            Self::Input(entity) => entity.stable_id(),
            Self::Multiline(entity) => entity.stable_id(),
            Self::Action(entity) => entity.stable_id(),
        }
    }

    fn is_editor(self) -> bool {
        matches!(self, Self::Input(_) | Self::Multiline(_))
    }
}

struct MountedField {
    id: Arc<str>,
    root: StableNodeId,
    control: Option<FormControl>,
    identity: Option<Arc<str>>,
}

impl MountedField {
    fn new(id: Arc<str>, root: StableNodeId) -> Self {
        Self {
            id,
            root,
            control: None,
            identity: None,
        }
    }

    fn interactive(id: Arc<str>, control: FormControl) -> Self {
        Self {
            control: Some(control),
            ..Self::new(id, control.node())
        }
    }
}

impl AppContext {
    /// Reconcile a form's keyed controls and relay their generic events to the
    /// form node. The method is public for hosts that create components by hand;
    /// normal view mounting invokes it through [`DynamicForm::BEHAVIOR`].
    pub fn assemble_dynamic_form(
        &mut self,
        form: Entity<DynamicForm>,
    ) -> Result<bool, FrameworkError> {
        let snapshot = self.read(form, Clone::clone)?;
        let previous_nodes = snapshot.field_nodes.clone();
        let previous_wired = snapshot.wired_nodes.clone();
        let previous_identities = snapshot.editor_identities.clone();
        let mut nodes = Vec::new();
        let mut controls = Vec::new();
        let mut identities = HashMap::new();
        let mut rows_to_assemble = Vec::new();

        let mut index = 0;
        self.mount(form, |scope| {
            while index < snapshot.fields.len() {
                if let DynamicFormField::Section { id, label } = &snapshot.fields[index] {
                    validate_key(id)?;
                    let section_id = id.to_string();
                    let title = Arc::clone(label);
                    let start = index + 1;
                    let end = snapshot.fields[start..]
                        .iter()
                        .position(|field| matches!(field, DynamicFormField::Section { .. }))
                        .map(|offset| start + offset)
                        .unwrap_or(snapshot.fields.len());
                    let mut mounted = Vec::new();
                    let card =
                        scope.with_child(section_id, SettingsCard::new(title), |section| {
                            section.with_child(
                                "body",
                                Stack::column(nana_ui_core::space::SM),
                                |body| {
                                    for field in &snapshot.fields[start..end] {
                                        let mounted_field = mount_dynamic_field(body, field)?;
                                        mounted.push(mounted_field);
                                    }
                                    Ok(())
                                },
                            )?;
                            Ok(())
                        })?;
                    nodes.push((Arc::clone(id), card.stable_id()));
                    for mounted_field in mounted {
                        nodes.push((mounted_field.id.clone(), mounted_field.root));
                        collect_mounted(
                            &mut controls,
                            &mut identities,
                            &mut rows_to_assemble,
                            mounted_field,
                        );
                    }
                    index = end;
                } else {
                    let mounted_field = mount_dynamic_field(scope, &snapshot.fields[index])?;
                    nodes.push((Arc::clone(snapshot.fields[index].id()), mounted_field.root));
                    collect_mounted(
                        &mut controls,
                        &mut identities,
                        &mut rows_to_assemble,
                        mounted_field,
                    );
                    index += 1;
                }
            }
            Ok(())
        })?;

        for (row, control) in rows_to_assemble {
            self.update_component(Entity::<SettingsRow>::from_stable_id(row), |row, _| {
                row.control = Some(control);
            })?;
            self.assemble_settings_row(Entity::<SettingsRow>::from_stable_id(row))?;
        }

        // `AssemblyScope` nested under a section returns only the card as a
        // root; controls are already retained under its body. Wire each newly
        // created control once. Despawning an old keyed subtree drops observers.
        for (field, control, _) in &controls {
            if previous_wired.contains(&control.node()) {
                continue;
            }
            let field = Arc::clone(field);
            let observer = form;
            install_dynamic_observer(self, *control, observer, field)?;
        }

        for (field, control, identity) in &controls {
            if control.is_editor()
                && previous_identities
                    .get(field)
                    .is_some_and(|previous| previous != identity)
            {
                self.clear_text_history(control.node())?;
            }
        }

        let next_wired = controls
            .iter()
            .map(|(_, control, _)| control.node())
            .collect();
        let changed = previous_nodes != nodes || previous_wired != next_wired;
        self.update_component(form, |form, _| {
            form.field_nodes = nodes;
            form.wired_nodes = next_wired;
            form.editor_identities = identities;
        })?;
        Ok(changed)
    }
}

fn validate_key(id: &Arc<str>) -> Result<(), FrameworkError> {
    (!id.is_empty() && !id.contains('/'))
        .then_some(())
        .ok_or(FrameworkError::InvalidInput)
}

fn collect_mounted(
    controls: &mut Vec<(Arc<str>, FormControl, Option<Arc<str>>)>,
    identities: &mut HashMap<Arc<str>, Option<Arc<str>>>,
    rows_to_assemble: &mut Vec<(StableNodeId, StableNodeId)>,
    mounted: MountedField,
) {
    if let Some(control) = mounted.control {
        if mounted.root != control.node() {
            rows_to_assemble.push((mounted.root, control.node()));
        }
        identities.insert(mounted.id.clone(), mounted.identity.clone());
        controls.push((mounted.id, control, mounted.identity));
    }
}

fn mount_dynamic_field(
    scope: &mut AssemblyScope<'_>,
    field: &DynamicFormField,
) -> Result<MountedField, FrameworkError> {
    validate_key(field.id())?;
    let id = field.id().clone();
    match field {
        DynamicFormField::Toggle {
            label,
            checked,
            disabled,
            ..
        } => {
            let view = scope.child(
                id.to_string(),
                Switch::new(label.to_string(), *checked).disabled(*disabled),
            )?;
            Ok(MountedField::interactive(id, FormControl::Toggle(view)))
        }
        DynamicFormField::Text { value, .. } => {
            let view = scope.child(id.to_string(), Text::new(value.to_string()))?;
            Ok(MountedField::new(id, view.stable_id()))
        }
        DynamicFormField::Error { value, .. } => {
            let view = scope.child(
                id.to_string(),
                ValidationMessage::new(value.to_string(), ValidationIntent::Danger),
            )?;
            Ok(MountedField::new(id, view.stable_id()))
        }
        DynamicFormField::Action {
            label,
            enabled,
            danger,
            ..
        } => {
            let kind = if *danger {
                ButtonKind::Danger
            } else {
                ButtonKind::Subtle
            };
            let view = scope.child(
                id.to_string(),
                Button::new(label.to_string())
                    .kind(kind)
                    .disabled(!*enabled),
            )?;
            Ok(MountedField::interactive(id, FormControl::Action(view)))
        }
        DynamicFormField::Choice {
            label,
            selected,
            options,
            disabled,
            hint,
            ..
        } => {
            let dropdown = Dropdown::single(Some(selected.clone()))
                .options(options.iter().map(|option| {
                    DropdownOption::new(option.value.clone(), option.label.clone())
                        .disabled(option.disabled)
                }))
                .disabled(*disabled)
                .placeholder(Arc::clone(&scope.framework_strings().dynamic_form_auto));
            mount_settings_control(scope, id, label, hint, None, dropdown, FormControl::Choice)
        }
        DynamicFormField::Field {
            label,
            value,
            multiline,
            secure,
            disabled,
            identity,
            hint,
            ..
        } => {
            if *multiline {
                let editor = TextArea::new(value.clone()).disabled(*disabled);
                mount_settings_control(
                    scope,
                    id,
                    label,
                    hint,
                    identity.clone(),
                    editor,
                    FormControl::Multiline,
                )
            } else {
                let editor = TextInput::new(value.clone())
                    .secure(*secure)
                    .disabled(*disabled);
                mount_settings_control(
                    scope,
                    id,
                    label,
                    hint,
                    identity.clone(),
                    editor,
                    FormControl::Input,
                )
            }
        }
        DynamicFormField::Section { .. } => Err(FrameworkError::InvalidInput),
    }
}

fn mount_settings_control<C: ComponentView>(
    scope: &mut AssemblyScope<'_>,
    id: Arc<str>,
    label: &Arc<str>,
    hint: &Option<Arc<str>>,
    identity: Option<Arc<str>>,
    editor: C,
    control: fn(Entity<C>) -> FormControl,
) -> Result<MountedField, FrameworkError> {
    let mut mounted = None;
    let row = scope.with_child(
        id.to_string(),
        SettingsRow::new(label.clone())
            .hint(hint.clone().unwrap_or_default())
            .stack_below(900.0),
        |row| {
            mounted = Some(control(row.child("control", editor)?));
            Ok(())
        },
    )?;
    Ok(MountedField {
        id,
        root: row.stable_id(),
        control: Some(mounted.ok_or(FrameworkError::InvalidInput)?),
        identity,
    })
}

fn install_dynamic_observer(
    context: &mut AppContext,
    source: FormControl,
    observer: Entity<DynamicForm>,
    field: Arc<str>,
) -> Result<(), FrameworkError> {
    match source {
        FormControl::Toggle(source) => {
            context.observe(source, observer, move |_, event: &ToggleChanged, cx| {
                cx.emit(DynamicFormEvent::Toggle {
                    field: field.clone(),
                    checked: event.checked,
                });
            })
        }
        FormControl::Choice(source) => context.observe(
            source,
            observer,
            move |_, event: &DropdownEvent<Arc<str>>, cx| {
                if let DropdownEvent::Select(value) = event {
                    cx.emit(DynamicFormEvent::Select {
                        field: field.clone(),
                        value: value.clone(),
                    });
                }
            },
        ),
        FormControl::Input(source) => {
            context.observe(source, observer, move |_, event: &TextChanged, cx| {
                cx.emit(DynamicFormEvent::Input {
                    field: field.clone(),
                    value: event.value.to_string(),
                });
            })
        }
        FormControl::Multiline(source) => {
            context.observe(source, observer, move |_, event: &TextChanged, cx| {
                cx.emit(DynamicFormEvent::Input {
                    field: field.clone(),
                    value: event.value.to_string(),
                });
            })
        }
        FormControl::Action(source) => {
            context.observe(source, observer, move |_, _: &Activate, cx| {
                cx.emit(DynamicFormEvent::Activate {
                    field: field.clone(),
                });
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;

    #[test]
    fn keyed_editors_survive_descriptor_updates() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let form = context
            .create_component(
                document,
                DynamicForm::new([DynamicFormField::field("name", "名称", "old")]),
            )
            .unwrap();
        context.assemble_dynamic_form(form).unwrap();
        let before = context.read(form, |form| form.field_nodes()[0].1).unwrap();

        context
            .set_component(
                form,
                DynamicForm::new([DynamicFormField::field("name", "名称", "new")]),
            )
            .unwrap();
        let after = context.read(form, |form| form.field_nodes()[0].1).unwrap();

        assert_eq!(before, after);
    }
}
