//! Lightweight search input composite ([`SearchInput`]): leading search
//! icon, text input, conditional clear button, and Escape-to-clear/blur.

use std::sync::{Arc, Mutex};

use nana_ui_core::{
    AlignSpec, ControlSize, Icon, LengthSpec, RadiusTier, SemanticColorRole, SpacingStep, TypeRole,
};

use super::controls::icon_button;
use super::node::{IntoView, NodeRef, ViewBuilder, node_ref, widget};
use super::prop::{IntoProp, PropSource};
use super::reactive::{Signal, on_mount};
use super::structural::when;
use crate::{Entity, IconGlyph, Stack, TextChanged, TextInput, TextSubmitted};

/// Declarative search field with leading search glyph, conditional clear
/// action, and Escape-to-clear-then-blur keyboard behavior.
pub struct SearchInput {
    query: Signal<String>,
    placeholder: PropSource<Arc<str>>,
    accessible_name: PropSource<Arc<str>>,
    clear_label: PropSource<Arc<str>>,
    disabled: PropSource<bool>,
    size: ControlSize,
    input_ref: Option<NodeRef>,
    on_change: Option<Arc<Mutex<Box<dyn FnMut(String) + Send + 'static>>>>,
    on_submit: Option<Arc<Mutex<Box<dyn FnMut(String) + Send + 'static>>>>,
}

/// Create a [`SearchInput`] bound to `query`.
pub fn search_input(query: Signal<String>) -> SearchInput {
    SearchInput {
        query,
        placeholder: PropSource::Const(Arc::from("")),
        accessible_name: PropSource::Const(Arc::from("Search")),
        clear_label: PropSource::Const(Arc::from("Clear")),
        disabled: PropSource::Const(false),
        size: ControlSize::Small,
        input_ref: None,
        on_change: None,
        on_submit: None,
    }
}

impl SearchInput {
    pub fn placeholder(mut self, placeholder: impl IntoProp<Arc<str>>) -> Self {
        self.placeholder = placeholder.into_source();
        self
    }

    pub fn accessible_name(mut self, label: impl IntoProp<Arc<str>>) -> Self {
        self.accessible_name = label.into_source();
        self
    }

    pub fn clear_label(mut self, label: impl IntoProp<Arc<str>>) -> Self {
        self.clear_label = label.into_source();
        self
    }

    pub fn disabled(mut self, disabled: impl IntoProp<bool>) -> Self {
        self.disabled = disabled.into_source();
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Bind a [`NodeRef`] to the inner [`TextInput`].
    pub fn node_ref(mut self, node_ref: NodeRef) -> Self {
        self.input_ref = Some(node_ref);
        self
    }

    pub fn on_change(mut self, handler: impl FnMut(String) + Send + 'static) -> Self {
        self.on_change = Some(Arc::new(Mutex::new(Box::new(handler))));
        self
    }

    pub fn on_submit(mut self, handler: impl FnMut(String) + Send + 'static) -> Self {
        self.on_submit = Some(Arc::new(Mutex::new(Box::new(handler))));
        self
    }
}

impl IntoView for SearchInput {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let query = self.query;
        let placeholder = self.placeholder;
        let accessible_name = self.accessible_name;
        let clear_label = Arc::new(Mutex::new(self.clear_label));
        let disabled = Arc::new(Mutex::new(self.disabled));
        let size = self.size;
        let input_ref = self.input_ref.unwrap_or_else(node_ref);
        let on_change = self.on_change;
        let on_submit = self.on_submit;

        let clear_on_esc = on_change.clone();
        on_mount(move |cx| {
            let Some(id) = input_ref.get_untracked() else {
                return;
            };
            let entity = Entity::<TextInput>::from_stable_id(id);
            let _ = cx.on_key(entity, move |key| {
                if key.state == nana_ui_input::KeyState::Pressed
                    && key.logical.0.as_ref() == "Escape"
                {
                    if !query.get_untracked().is_empty() {
                        query.set(String::new());
                        if let Some(cb) = &clear_on_esc
                            && let Ok(mut f) = cb.lock()
                        {
                            f(String::new());
                        }
                        return true;
                    }
                    return false;
                }
                false
            });
        });

        let change_cb = on_change.clone();
        let clear_cb = on_change.clone();

        let mut leading_icon = IconGlyph::new(Icon::Search)
            .size_role(TypeRole::Meta)
            .role(SemanticColorRole::Muted)
            .decorative();
        Arc::make_mut(&mut leading_icon.style.layout).flex_shrink = Some(f32::default());
        let leading = widget(leading_icon);

        let disabled_field = Arc::clone(&disabled);
        let mut input_component = TextInput::new(query.get_untracked()).size(size);
        {
            let l = Arc::make_mut(&mut input_component.style.layout);
            l.flex_grow = Some(1.0);
            l.min_width = Some(LengthSpec::Px(f32::default()));
        }
        let mut field = widget(input_component)
            .node_ref(input_ref)
            .value(query)
            .placeholder(move || placeholder.get())
            .label(move || Some(accessible_name.get()))
            .disabled(move || disabled_field.lock().unwrap().get())
            .on(move |changed: &TextChanged| {
                let next = changed.value.as_str().to_owned();
                if query.get_untracked() != next {
                    query.set(next.clone());
                }
                if let Some(cb) = &change_cb
                    && let Ok(mut f) = cb.lock()
                {
                    f(next);
                }
            });

        if let Some(submit_cb) = on_submit {
            field = field.on(move |submitted: &TextSubmitted| {
                if let Ok(mut f) = submit_cb.lock() {
                    f(submitted.value.clone());
                }
            });
        }

        let clear_button = when(
            move || !query.get().is_empty() && !disabled.lock().unwrap().get(),
            move || {
                let clear_cb = clear_cb.clone();
                let clear_lbl = Arc::clone(&clear_label);
                icon_button(Icon::Close, move || clear_lbl.lock().unwrap().get())
                    .compact(true)
                    .on_activate(move || {
                        query.set(String::new());
                        if let Some(cb) = &clear_cb
                            && let Ok(mut f) = cb.lock()
                        {
                            f(String::new());
                        }
                    })
            },
        );

        widget(
            Stack::row(f32::default())
                .width(LengthSpec::Fill)
                .gap_step(SpacingStep::Xs)
                .align(AlignSpec::Center)
                .padding_step(SpacingStep::Sm, SpacingStep::Xxs)
                .radius(RadiusTier::Sm)
                .surface(SemanticColorRole::Subtle)
                .border(SemanticColorRole::Border, nana_ui_core::HAIRLINE),
        )
        .children((leading, field, clear_button))
        .build(vb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::signal;
    use crate::{AppContext, DocumentId, LayoutViewport};

    #[test]
    fn search_input_shows_clear_button_only_when_non_empty_and_clears_query() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let query = signal(String::new());
        let input_ref = node_ref();
        let view = cx
            .mount_view_root(document, move || {
                search_input(query)
                    .placeholder("Filter...")
                    .clear_label("Clear search")
                    .node_ref(input_ref)
                    .into_any()
            })
            .unwrap();
        cx.flush_reactive().unwrap();
        cx.layout_document(document, LayoutViewport::new(320.0, 80.0))
            .unwrap();

        let root = view.roots()[0];
        let a11y_empty = cx.world().project_accessibility(document);
        assert!(
            !a11y_empty
                .iter()
                .any(|n| n.label.as_deref() == Some("Clear search")),
            "clear button hidden when query is empty"
        );

        query.set("avatar".into());
        cx.flush_reactive().unwrap();
        let a11y_filled = cx.world().project_accessibility(document);
        let clear_node = a11y_filled
            .iter()
            .find(|n| n.label.as_deref() == Some("Clear search"))
            .expect("clear button mounted when query is non-empty")
            .id;

        cx.activate_node(clear_node).unwrap();
        cx.flush_reactive().unwrap();
        assert_eq!(query.get_untracked(), "");
        let _ = root;
    }
}
