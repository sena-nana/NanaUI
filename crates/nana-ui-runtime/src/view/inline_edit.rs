//! Zero-shift click-to-edit inline text control ([`InlineEdit`] / [`EditableText`]).
//!
//! Displays a clickable text readout until activated, then swaps in a matching
//! [`TextInput`], focuses and selects all text on mount via [`AppContext`],
//! tracks whether the draft is dirty, commits on Enter or dirty blur, and
//! cancels cleanly on Escape or unmodified blur.

use std::sync::{Arc, Mutex};

use nana_ui_core::{
    AlignSpec, ControlSize, JustifySpec, LengthSpec, RadiusTier, SemanticColorRole, SpacingStep,
    TypeRole,
};

use super::node::{IntoView, NodeRef, ViewBuilder, node_ref, widget};
use super::prop::{IntoProp, PropSource};
use super::reactive::{on_mount, signal};
use super::structural::when;
use crate::{
    AccessibilityRole, AccessibilityState, Activate, ComponentView, Entity, FocusWithinChanged,
    InteractionState, MutationQueue, NodeKind, NodeStyle, StableNodeId, Text, TextChanged,
    TextInput, TextSubmitted, UiWorld,
};

/// Result of completing an inline edit session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineEditAction {
    /// Draft changed and passed validation: commit the new value.
    Commit(String),
    /// Draft was unchanged, cancelled (Escape), or rejected as empty.
    Cancel,
}

/// Pure, deterministic state transition for finishing an [`InlineEdit`] session.
pub fn resolve_inline_edit(
    initial: &str,
    draft: &str,
    dirty: bool,
    allow_empty: bool,
    cancelled: bool,
) -> InlineEditAction {
    if cancelled || !dirty {
        return InlineEditAction::Cancel;
    }
    let trimmed = draft.trim();
    if trimmed.is_empty() && !allow_empty {
        return InlineEditAction::Cancel;
    }
    if draft == initial {
        return InlineEditAction::Cancel;
    }
    InlineEditAction::Commit(draft.to_owned())
}

#[derive(Debug, Clone, PartialEq)]
struct InlineEditDisplay {
    label: Arc<str>,
    disabled: bool,
    size: ControlSize,
    style: NodeStyle,
}

impl InlineEditDisplay {
    fn new(label: Arc<str>, disabled: bool, size: ControlSize) -> Self {
        let mut style = NodeStyle::default();
        style.foreground = Some(SemanticColorRole::Text);
        style.interaction.hovered.background = Some(SemanticColorRole::Hover);
        style.interaction.pressed.background = Some(SemanticColorRole::Active);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.direction = Some(crate::FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Start;
        layout.theme.padding_left = Some(crate::theme_intent::space(SpacingStep::Sm));
        layout.theme.padding_right = Some(crate::theme_intent::space(SpacingStep::Sm));
        layout.theme.border_radius = Some(nana_ui_core::ThemeToken::Radius(RadiusTier::Sm).into());
        Self {
            label,
            disabled,
            size,
            style,
        }
    }
}

impl ComponentView for InlineEditDisplay {
    const TYPE_ID: Option<&'static str> = None;

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(|cx, entity| {
            let disabled = cx.read(entity, |d| d.disabled)?;
            if disabled {
                return Ok(false);
            }
            cx.update_component(entity, |_, view_cx| {
                view_cx.emit(Activate);
            })?;
            Ok(true)
        }),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "inline-edit-display".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut effective = self.style.clone();
        let h = self.size.height_in(world.node_theme(id).metrics());
        Arc::make_mut(&mut effective.layout).min_height = Some(LengthSpec::Px(h));
        crate::view_components::project_common(
            id,
            world,
            mutations,
            &effective,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::clone(&self.label)),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Zero-shift click-to-edit inline text control.
pub struct InlineEdit {
    value: PropSource<Arc<str>>,
    placeholder: PropSource<Arc<str>>,
    accessible_name: Option<Arc<str>>,
    disabled: PropSource<bool>,
    editing: Option<super::Signal<bool>>,
    size: ControlSize,
    commit_on_blur: bool,
    allow_empty: bool,
    on_commit: Option<Arc<Mutex<Box<dyn FnMut(String) + Send + 'static>>>>,
    on_cancel: Option<Arc<Mutex<Box<dyn FnMut() + Send + 'static>>>>,
}

/// Alias matching design-system terminology (`EditableText`).
pub type EditableText = InlineEdit;

/// Create a click-to-edit [`InlineEdit`] control displaying `value`.
pub fn inline_edit(value: impl IntoProp<Arc<str>>) -> InlineEdit {
    InlineEdit {
        value: value.into_source(),
        placeholder: PropSource::Const(Arc::from("")),
        accessible_name: None,
        disabled: PropSource::Const(false),
        editing: None,
        size: ControlSize::Small,
        commit_on_blur: true,
        allow_empty: false,
        on_commit: None,
        on_cancel: None,
    }
}

/// Alias constructor for [`inline_edit`].
pub fn editable_text(value: impl IntoProp<Arc<str>>) -> EditableText {
    inline_edit(value)
}

impl InlineEdit {
    pub fn placeholder(mut self, placeholder: impl IntoProp<Arc<str>>) -> Self {
        self.placeholder = placeholder.into_source();
        self
    }

    pub fn accessible_name(mut self, name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        self.accessible_name = (!name.is_empty()).then_some(name);
        self
    }

    pub fn disabled(mut self, disabled: impl IntoProp<bool>) -> Self {
        self.disabled = disabled.into_source();
        self
    }

    /// Share the editing state with a caller-owned signal (for example to
    /// enter edit mode from a context menu action).
    pub fn editing(mut self, editing: super::Signal<bool>) -> Self {
        self.editing = Some(editing);
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Whether blurring the field while dirty commits (`true`, default) or
    /// cancels (`false`).
    pub fn commit_on_blur(mut self, commit: bool) -> Self {
        self.commit_on_blur = commit;
        self
    }

    /// Whether empty / whitespace-only drafts are accepted on commit (default
    /// `false`, which reverts empty edits).
    pub fn allow_empty(mut self, allow: bool) -> Self {
        self.allow_empty = allow;
        self
    }

    pub fn on_commit(mut self, handler: impl FnMut(String) + Send + 'static) -> Self {
        self.on_commit = Some(Arc::new(Mutex::new(Box::new(handler))));
        self
    }

    pub fn on_cancel(mut self, handler: impl FnMut() + Send + 'static) -> Self {
        self.on_cancel = Some(Arc::new(Mutex::new(Box::new(handler))));
        self
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_edit(
    editing: super::Signal<bool>,
    initial: super::Signal<Arc<str>>,
    draft: super::Signal<String>,
    dirty: super::Signal<bool>,
    allow_empty: bool,
    cancelled: bool,
    on_commit: &Option<Arc<Mutex<Box<dyn FnMut(String) + Send + 'static>>>>,
    on_cancel: &Option<Arc<Mutex<Box<dyn FnMut() + Send + 'static>>>>,
) {
    if !editing.get_untracked() {
        return;
    }
    let init_val = initial.get_untracked();
    let draft_val = draft.get_untracked();
    let was_dirty = dirty.get_untracked();
    editing.set(false);
    dirty.set(false);
    match resolve_inline_edit(&init_val, &draft_val, was_dirty, allow_empty, cancelled) {
        InlineEditAction::Commit(committed) => {
            if let Some(cb) = on_commit
                && let Ok(mut f) = cb.lock()
            {
                f(committed);
            }
        }
        InlineEditAction::Cancel => {
            if let Some(cb) = on_cancel
                && let Ok(mut f) = cb.lock()
            {
                f();
            }
        }
    }
}

struct InlineEditTextWrite;
impl super::prop::FieldWrite<Text, String> for InlineEditTextWrite {
    const FIELD: &'static str = "Text.value";
    fn write(target: &mut Text, value: String) {
        target.value = value;
    }
    fn differs(target: &Text, value: &String) -> bool {
        target.value != *value
    }
}

impl IntoView for InlineEdit {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let value = Arc::new(Mutex::new(self.value));
        let placeholder = Arc::new(Mutex::new(self.placeholder));
        let disabled = Arc::new(Mutex::new(self.disabled));
        let editing = self.editing.unwrap_or_else(|| signal(false));
        let initial_value = super::untrack(|| value.lock().unwrap().get());
        let initial_snapshot = signal::<Arc<str>>(Arc::clone(&initial_value));
        let draft = signal(initial_value.to_string());
        let dirty = signal(false);
        let size = self.size;
        let commit_on_blur = self.commit_on_blur;
        let allow_empty = self.allow_empty;
        let accessible_name = self.accessible_name;
        let on_commit = self.on_commit;
        let on_cancel = self.on_cancel;

        let value_display = Arc::clone(&value);
        let placeholder_display = Arc::clone(&placeholder);
        let disabled_display = Arc::clone(&disabled);
        let display_branch = move || {
            let value_click = Arc::clone(&value_display);
            let value_text = Arc::clone(&value_display);
            let placeholder_text = Arc::clone(&placeholder_display);
            let disabled_click = Arc::clone(&disabled_display);
            let label = accessible_name
                .clone()
                .unwrap_or_else(|| super::untrack(|| value_display.lock().unwrap().get()));
            let is_disabled = super::untrack(|| disabled_display.lock().unwrap().get());
            let mut label_text = Text::new("").role(TypeRole::Body).decorative();
            {
                let l = Arc::make_mut(&mut label_text.style.layout);
                l.min_width = Some(LengthSpec::Px(f32::default()));
                l.text_overflow_ellipsis = true;
                l.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
            }
            widget(InlineEditDisplay::new(label, is_disabled, size))
                .on(move |_: &Activate| {
                    if super::untrack(|| disabled_click.lock().unwrap().get()) {
                        return;
                    }
                    let current = super::untrack(|| value_click.lock().unwrap().get());
                    initial_snapshot.set(Arc::clone(&current));
                    draft.set(current.to_string());
                    dirty.set(false);
                    editing.set(true);
                })
                .children(
                    widget(label_text).prop::<String, InlineEditTextWrite>(move || {
                        let current = value_text.lock().unwrap().get();
                        if current.is_empty() {
                            placeholder_text.lock().unwrap().get().to_string()
                        } else {
                            current.to_string()
                        }
                    }),
                )
        };

        let placeholder_edit = Arc::clone(&placeholder);
        let edit_branch = move || {
            let input_ref: NodeRef = node_ref();
            let esc_commit = on_commit.clone();
            let esc_cancel = on_cancel.clone();
            let placeholder_input = Arc::clone(&placeholder_edit);
            on_mount(move |cx| {
                let Some(id) = input_ref.get_untracked() else {
                    return;
                };
                if let Some(doc) = cx.world().document_of(id) {
                    let _ = cx.focus_node(doc, id);
                    let _ = cx.select_all_focused_text(doc);
                }
                let entity = Entity::<TextInput>::from_stable_id(id);
                let _ = cx.on_key(entity, move |key| {
                    if key.state == nana_ui_input::KeyState::Pressed
                        && key.logical.0.as_ref() == "Escape"
                    {
                        finish_edit(
                            editing,
                            initial_snapshot,
                            draft,
                            dirty,
                            allow_empty,
                            true,
                            &esc_commit,
                            &esc_cancel,
                        );
                        return true;
                    }
                    false
                });
            });

            let submit_commit = on_commit.clone();
            let submit_cancel = on_cancel.clone();
            let blur_commit = on_commit.clone();
            let blur_cancel = on_cancel.clone();

            widget(TextInput::new(draft.get_untracked()).size(size))
                .node_ref(input_ref)
                .value(draft)
                .placeholder(move || placeholder_input.lock().unwrap().get())
                .on(move |changed: &TextChanged| {
                    let next = changed.value.as_str().to_owned();
                    let changed_from_initial =
                        next.as_str() != initial_snapshot.get_untracked().as_ref();
                    draft.set(next);
                    dirty.set(changed_from_initial);
                })
                .on(move |submitted: &TextSubmitted| {
                    draft.set(submitted.value.clone());
                    let changed_from_initial =
                        submitted.value.as_str() != initial_snapshot.get_untracked().as_ref();
                    dirty.set(changed_from_initial);
                    finish_edit(
                        editing,
                        initial_snapshot,
                        draft,
                        dirty,
                        allow_empty,
                        false,
                        &submit_commit,
                        &submit_cancel,
                    );
                })
                .on(move |focus: &FocusWithinChanged| {
                    if !focus.focused {
                        finish_edit(
                            editing,
                            initial_snapshot,
                            draft,
                            dirty,
                            allow_empty,
                            !commit_on_blur,
                            &blur_commit,
                            &blur_cancel,
                        );
                    }
                })
        };

        when(move || editing.get(), edit_branch)
            .otherwise(display_branch)
            .build(vb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_inline_edit_handles_dirty_empty_and_cancel() {
        assert_eq!(
            resolve_inline_edit("Model", "Model", false, false, false),
            InlineEditAction::Cancel
        );
        assert_eq!(
            resolve_inline_edit("Model", "Renamed", true, false, true),
            InlineEditAction::Cancel
        );
        assert_eq!(
            resolve_inline_edit("Model", "   ", true, false, false),
            InlineEditAction::Cancel
        );
        assert_eq!(
            resolve_inline_edit("Model", "   ", true, true, false),
            InlineEditAction::Commit("   ".into())
        );
        assert_eq!(
            resolve_inline_edit("Model", "Model", true, false, false),
            InlineEditAction::Cancel
        );
        assert_eq!(
            resolve_inline_edit("Model", "Hero", true, false, false),
            InlineEditAction::Commit("Hero".into())
        );
    }
}
