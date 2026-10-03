//! Generic request card composition.
//!
//! [`InteractionRequestCard`] is a neutral surface for a title, an optional
//! prompt/body, keyed form fields, and actions. It owns only the presentation
//! tree. Hosts own field values, validation policy, and action handling.

use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::{AlignSpec, CardKind, FlexDirection, JustifySpec, LengthSpec, space};

use crate::AccessibilityState;
use crate::{
    AppContext, Card, ComponentView, Entity, FormField, FrameworkError, MutationQueue, NodeKind,
    NodeStyle, StableNodeId, UiWorld,
};

/// A keyed field supplied to an [`InteractionRequestCard`].
///
/// The control is an application-owned component. The request card creates a
/// [`FormField`] wrapper and attaches the control to it while retaining the
/// wrapper by `key`. Values and change events remain in the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionRequestField {
    pub key: Arc<str>,
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
    pub error: Option<Arc<str>>,
    pub control: StableNodeId,
}

impl InteractionRequestField {
    pub fn new(
        key: impl Into<Arc<str>>,
        label: impl Into<Arc<str>>,
        control: StableNodeId,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            hint: None,
            error: None,
            control,
        }
    }

    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn error(mut self, error: impl Into<Arc<str>>) -> Self {
        self.error = Some(error.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldSlot {
    key: Arc<str>,
    wrapper: StableNodeId,
}

/// Neutral request/approval card.
///
/// `prompt`, `body`, and `actions` are heterogeneous child slots. `fields`
/// describes controls that should be wrapped in framework [`FormField`]s;
/// each field is retained by its stable `key` as the request changes. The
/// component emits no product or workflow events. Applications listen to the
/// controls and actions they provide.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionRequestCard {
    pub title: Arc<str>,
    pub kind: CardKind,
    pub loading: bool,
    pub prompt: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub actions: Vec<StableNodeId>,
    pub fields: Vec<InteractionRequestField>,
    pub style: NodeStyle,
    field_slots: Vec<FieldSlot>,
}

impl InteractionRequestCard {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        let mut style = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.direction = Some(FlexDirection::Column);
            layout.align_items = AlignSpec::Stretch;
            layout.justify_content = JustifySpec::Start;
            layout.gap = Some(LengthSpec::Px(space::MD));
            layout.width = Some(LengthSpec::Fill);
            layout.min_width = Some(LengthSpec::Px(0.0));
            layout.flex_grow = Some(0.0);
            layout.flex_shrink = Some(0.0);
        }
        Self {
            title: title.into(),
            kind: CardKind::Surface,
            loading: false,
            prompt: None,
            body: None,
            actions: Vec::new(),
            fields: Vec::new(),
            style,
            field_slots: Vec::new(),
        }
    }

    pub fn kind(mut self, kind: CardKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn prompt(mut self, prompt: StableNodeId) -> Self {
        self.prompt = Some(prompt);
        self
    }

    pub fn body(mut self, body: StableNodeId) -> Self {
        self.body = Some(body);
        self
    }

    pub fn actions(mut self, actions: impl IntoIterator<Item = StableNodeId>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    pub fn field(mut self, field: InteractionRequestField) -> Self {
        self.fields.push(field);
        self
    }

    pub fn fields(mut self, fields: impl IntoIterator<Item = InteractionRequestField>) -> Self {
        self.fields = fields.into_iter().collect();
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn field_wrappers(&self) -> impl Iterator<Item = (&str, StableNodeId)> {
        self.field_slots
            .iter()
            .map(|slot| (slot.key.as_ref(), slot.wrapper))
    }

    fn card(&self) -> Card {
        Card::new()
            .title(Arc::clone(&self.title))
            .kind(self.kind)
            .loading(self.loading)
            .style(self.style.clone())
    }
}

impl Default for InteractionRequestCard {
    fn default() -> Self {
        Self::new("")
    }
}

impl ComponentView for InteractionRequestCard {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(crate::AppContext::assemble_interaction_request_card),
        slot_assembler: Some(crate::AppContext::assemble_interaction_request_card),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        next.field_slots = std::mem::take(&mut self.field_slots);
        *self = next;
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "interaction-request-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.card().project(id, world, mutations);
        // Card already supplies the accessible name; the explicit role keeps
        // this composite a passive region even when it contains controls.
        mutations.set_accessibility(
            id,
            AccessibilityState {
                role: crate::AccessibilityRole::Region,
                label: Some(Arc::clone(&self.title)),
                busy: self.loading,
                ..AccessibilityState::default()
            },
        );
    }
}

impl AppContext {
    /// Assemble the heterogeneous slots and keyed [`FormField`] wrappers.
    pub fn assemble_interaction_request_card(
        &mut self,
        card: Entity<InteractionRequestCard>,
    ) -> Result<bool, FrameworkError> {
        let id = card.stable_id();
        let node = self
            .world()
            .node(id)
            .ok_or(FrameworkError::MissingView(id))?;
        let document = node.document;
        let current_children = node.children.clone();
        let (prompt, body, actions, fields, previous_slots) = self.read(card, |card| {
            (
                card.prompt,
                card.body,
                card.actions.clone(),
                card.fields.clone(),
                card.field_slots.clone(),
            )
        })?;

        let mut seen = HashSet::with_capacity(fields.len());
        let mut controls = HashSet::with_capacity(fields.len());
        for field in &fields {
            if field.key.is_empty()
                || field.key.contains(crate::ASSEMBLY_PATH_SEPARATOR)
                || !seen.insert(field.key.as_ref())
                || !controls.insert(field.control)
            {
                return Err(FrameworkError::InvalidInput);
            }
        }

        let mut slots = Vec::with_capacity(fields.len());
        let mut created = false;
        for field in &fields {
            let wrapper = previous_slots
                .iter()
                .find(|slot| slot.key == field.key)
                .filter(|slot| self.world().contains(slot.wrapper))
                .map(|slot| slot.wrapper);
            let wrapper = match wrapper {
                Some(wrapper) => wrapper,
                None => {
                    created = true;
                    self.create_detached_component(
                        document,
                        FormField::new(Arc::clone(&field.label)),
                    )?
                    .stable_id()
                }
            };
            let field_entity = Entity::<FormField>::from_stable_id(wrapper);
            self.set_form_field_control(field_entity, Some(field.control))?;
            self.update_component(field_entity, |field_view, _| {
                field_view.label = Arc::clone(&field.label);
                field_view.hint = field.hint.clone();
                field_view.error = field.error.clone();
            })?;
            slots.push(FieldSlot {
                key: Arc::clone(&field.key),
                wrapper,
            });
        }

        let desired: Vec<StableNodeId> = slots
            .iter()
            .map(|slot| slot.wrapper)
            .chain(prompt)
            .chain(body)
            .chain(actions.iter().copied())
            .collect();
        let desired_set: HashSet<_> = desired.iter().copied().collect();
        if desired_set.len() != desired.len() {
            return Err(FrameworkError::InvalidInput);
        }

        let stale: Vec<_> = current_children
            .iter()
            .copied()
            .filter(|child| !desired_set.contains(child))
            .collect();
        if !stale.is_empty() {
            let mut mutations = MutationQueue::new();
            for child in stale {
                mutations.park_subtree(child);
            }
            self.commit_mutations(mutations)?;
        }

        let slots_changed = previous_slots != slots;
        self.update_component(card, |card, _| {
            card.field_slots = slots;
        })?;
        let structure_changed = current_children.as_slice() != desired.as_slice();
        self.reconcile_children(id, &desired)?;
        Ok(created || structure_changed || slots_changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentId, Text, TextInput};

    #[test]
    fn request_card_retains_keyed_fields_and_orders_slots() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_detached_component(document, TextInput::new("value"))
            .unwrap();
        let prompt = context
            .create_detached_component(document, Text::new("Prompt"))
            .unwrap();
        let body = context
            .create_detached_component(document, Text::new("Body"))
            .unwrap();
        let action = context
            .create_detached_component(document, crate::Button::new("Continue"))
            .unwrap();
        let card = context
            .create_component(
                document,
                InteractionRequestCard::new("Request")
                    .prompt(prompt.stable_id())
                    .body(body.stable_id())
                    .actions([action.stable_id()])
                    .field(InteractionRequestField::new(
                        "message",
                        "Message",
                        control.stable_id(),
                    )),
            )
            .unwrap();
        context.assemble_interaction_request_card(card).unwrap();

        let wrappers = context
            .read(card, |card| {
                card.field_wrappers()
                    .map(|(key, id)| (key.to_owned(), id))
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(wrappers.len(), 1);
        let children = context
            .world()
            .node(card.stable_id())
            .unwrap()
            .children
            .clone();
        assert_eq!(children.len(), 4);
        assert_eq!(children[0], wrappers[0].1);
        assert_eq!(children[1], prompt.stable_id());
        assert_eq!(children[2], body.stable_id());
        assert_eq!(children[3], action.stable_id());
        assert_eq!(
            context
                .world()
                .node(wrappers[0].1)
                .unwrap()
                .children
                .as_slice(),
            &[control.stable_id()]
        );

        context
            .set_component(
                card,
                InteractionRequestCard::new("Updated request")
                    .prompt(prompt.stable_id())
                    .body(body.stable_id())
                    .actions([action.stable_id()])
                    .field(InteractionRequestField::new(
                        "message",
                        "Updated message",
                        control.stable_id(),
                    )),
            )
            .unwrap();
        let wrappers_after_set = context
            .read(card, |card| {
                card.field_wrappers()
                    .map(|(key, id)| (key.to_owned(), id))
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(wrappers_after_set, wrappers);

        let next_control = context
            .create_detached_component(document, TextInput::new("next"))
            .unwrap();
        context
            .update_component(card, |card, _| {
                card.fields[0].control = next_control.stable_id();
            })
            .unwrap();
        let wrappers_after = context
            .read(card, |card| {
                card.field_wrappers()
                    .map(|(key, id)| (key.to_owned(), id))
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(wrappers_after[0].1, wrappers[0].1);
        assert_eq!(
            context
                .world()
                .node(wrappers[0].1)
                .unwrap()
                .children
                .as_slice(),
            &[next_control.stable_id()]
        );
    }
}
