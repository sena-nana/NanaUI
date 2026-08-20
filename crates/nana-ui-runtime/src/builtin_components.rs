//! Built-in components install through the same [`UiExtension`] ABI as plugins.

use std::sync::Arc;

use nana_ui_core::{Icon, StatusTone, ValidationIntent};

use crate::component_registry::{RegisterableComponent, SemanticSpec};
use crate::{
    Button, Card, Checkbox, Dialog, EmptyState, ExtensionRegistrar, FrameworkError, IconButton,
    LabeledValue, ListItem, Progress, RangeField, Spinner, StatusBadge, Switch, Text, TextArea,
    TextInput, TextInputState, UiExtension, ValidationMessage,
};

pub struct NanaBuiltinComponents;

impl UiExtension for NanaBuiltinComponents {
    fn name(&self) -> &'static str {
        "nana.builtin"
    }

    fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
        registrar.register_tags(
            "nana.column",
            &[
                "column", "col", "vstack", "div", "section", "article", "main", "nav", "header",
                "footer", "ul", "ol",
            ],
        )?;
        registrar.register_tags("nana.row", &["row", "hstack"])?;
        registrar.register_tags("nana.box", &["box", "container", "layout"])?;
        registrar.register_component::<Text>()?;
        registrar.register_component::<Button>()?;
        registrar.register_component::<IconButton>()?;
        registrar.register_component::<Checkbox>()?;
        registrar.register_component::<Switch>()?;
        registrar.register_component::<Card>()?;
        registrar.register_component::<ListItem>()?;
        registrar.register_component::<TextInput>()?;
        registrar.register_component::<TextArea>()?;
        registrar.register_component::<RangeField>()?;
        registrar.register_component::<Progress>()?;
        registrar.register_component::<Spinner>()?;
        registrar.register_component::<StatusBadge>()?;
        registrar.register_component::<ValidationMessage>()?;
        registrar.register_component::<EmptyState>()?;
        registrar.register_component::<LabeledValue>()?;
        registrar.register_component::<Dialog>()?;
        registrar.register_tags("nana.chip", &["chip"])?;
        registrar.register_tags("nana.icon", &["icon", "i"])?;
        Ok(())
    }
}

impl RegisterableComponent for Text {
    const TYPE_ID: &'static str = "nana.text";
    const TAGS: &'static [&'static str] = &["text", "label", "p", "span", "h1", "h2", "h3"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Text::new(spec.display_label()).style(crate::NodeStyle {
            layout: Arc::clone(spec.layout),
            ..crate::NodeStyle::default()
        })
    }
}

impl RegisterableComponent for Button {
    const TYPE_ID: &'static str = "nana.button";
    const TAGS: &'static [&'static str] = &["button", "btn"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Button::new(spec.display_label())
            .layout(Arc::clone(spec.layout))
            .kind(spec.button_kind)
            .size(spec.size)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .invalid(spec.invalid)
    }
}

impl RegisterableComponent for IconButton {
    const TYPE_ID: &'static str = "nana.icon-button";
    const TAGS: &'static [&'static str] = &["icon-button", "iconbutton"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let icon = spec.icon.unwrap_or(Icon::Add);
        let mut component = IconButton::new(icon, Arc::<str>::from(spec.display_label()))
            .kind(spec.button_kind)
            .size(spec.size)
            .selected(spec.active)
            .disabled(spec.disabled);
        if !spec.hint.is_empty() {
            component = component.tooltip(
                Arc::<str>::from(spec.hint),
                nana_ui_core::TooltipConfig::default(),
            );
        }
        component
    }
}

impl RegisterableComponent for Checkbox {
    const TYPE_ID: &'static str = "nana.checkbox";
    const TAGS: &'static [&'static str] = &["checkbox", "check"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Checkbox::new(spec.display_label(), spec.toggled)
            .disabled(spec.disabled)
            .invalid(spec.invalid)
    }
}

impl RegisterableComponent for Switch {
    const TYPE_ID: &'static str = "nana.switch";
    const TAGS: &'static [&'static str] = &["switch", "toggle"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Switch::new(spec.display_label(), spec.toggled)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .invalid(spec.invalid)
            .size(spec.size)
    }
}

impl RegisterableComponent for Card {
    const TYPE_ID: &'static str = "nana.card";
    const TAGS: &'static [&'static str] = &["card"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut card = Card::new().kind(nana_ui_core::CardKind::Surface);
        if !spec.display_label().is_empty() {
            card = card.title(Arc::<str>::from(spec.display_label()));
        }
        card.loading(spec.loading)
    }
}

impl RegisterableComponent for ListItem {
    const TYPE_ID: &'static str = "nana.list-item";
    const TAGS: &'static [&'static str] = &["list-item", "listitem", "li"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        ListItem::new(spec.display_label())
            .selected(spec.active)
            .disabled(spec.disabled)
            .size(spec.size)
    }
}

impl RegisterableComponent for TextInput {
    const TYPE_ID: &'static str = "nana.text-input";
    const TAGS: &'static [&'static str] = &["input", "text-field", "textfield"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let placeholder = if spec.placeholder.is_empty() {
            spec.hint
        } else {
            spec.placeholder
        };
        let mut component = TextInput::new("")
            .placeholder(Arc::<str>::from(placeholder))
            .layout(Arc::clone(spec.layout))
            .size(spec.size)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .read_only(spec.read_only)
            .secure(spec.secure)
            .invalid(spec.invalid);
        component.state = TextInputState::new(spec.value);
        if !spec.label.is_empty() {
            component = component.label(Arc::<str>::from(spec.label));
        }
        component
    }
}

impl RegisterableComponent for TextArea {
    const TYPE_ID: &'static str = "nana.textarea";
    const TAGS: &'static [&'static str] = &["textarea"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let placeholder = if spec.placeholder.is_empty() {
            spec.hint
        } else {
            spec.placeholder
        };
        let mut component = TextArea::new("")
            .placeholder(Arc::<str>::from(placeholder))
            .disabled(spec.disabled)
            .invalid(spec.invalid);
        if let Some(nana_ui_core::LengthSpec::Px(height)) = spec.layout.height {
            component = component.height(height);
        }
        if !spec.label.is_empty() {
            component = component.label(Arc::<str>::from(spec.label));
        }
        component.state = TextInputState::new(spec.value);
        component
    }
}

impl RegisterableComponent for RangeField {
    const TYPE_ID: &'static str = "nana.range-field";
    const TAGS: &'static [&'static str] = &["range", "range-field", "slider"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let min = spec.min as f64;
        let max = if spec.max > spec.min {
            spec.max as f64
        } else {
            min + 1.0
        };
        let step = if spec.step > 0.0 {
            spec.step as f64
        } else {
            0.1
        };
        let value = (spec.number as f64).clamp(min, max);
        RangeField::new(value, min, max, step)
            .unwrap_or_else(|_| RangeField::new(0.0, 0.0, 1.0, 0.1).expect("default range"))
            .disabled(spec.disabled)
            .invalid(spec.invalid)
            .size(spec.size)
    }
}

impl RegisterableComponent for Progress {
    const TYPE_ID: &'static str = "nana.progress";
    const TAGS: &'static [&'static str] = &["progress"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Progress::new(spec.number as f64, spec.max.max(1.0) as f64)
    }
}

impl RegisterableComponent for Spinner {
    const TYPE_ID: &'static str = "nana.spinner";
    const TAGS: &'static [&'static str] = &["spinner", "loading"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Spinner::new(spec.display_label())
    }
}

impl RegisterableComponent for StatusBadge {
    const TYPE_ID: &'static str = "nana.status-badge";
    const TAGS: &'static [&'static str] = &["status", "status-badge", "statusbadge"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        StatusBadge::new(spec.display_label(), StatusTone::Neutral)
    }
}

impl RegisterableComponent for ValidationMessage {
    const TYPE_ID: &'static str = "nana.validation-message";
    const TAGS: &'static [&'static str] =
        &["validation", "validation-message", "validationmessage"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        ValidationMessage::new(spec.display_label(), ValidationIntent::Danger)
    }
}

impl RegisterableComponent for EmptyState {
    const TYPE_ID: &'static str = "nana.empty-state";
    const TAGS: &'static [&'static str] = &["empty", "empty-state", "emptystate"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        EmptyState::new(spec.display_label())
    }
}

impl RegisterableComponent for LabeledValue {
    const TYPE_ID: &'static str = "nana.labeled-value";
    const TAGS: &'static [&'static str] = &["labeled-value", "labeledvalue"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        LabeledValue::new(spec.label, spec.value)
    }
}

impl RegisterableComponent for Dialog {
    const TYPE_ID: &'static str = "nana.dialog";
    const TAGS: &'static [&'static str] = &["dialog", "modal"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Dialog::new(spec.display_label())
    }
}
