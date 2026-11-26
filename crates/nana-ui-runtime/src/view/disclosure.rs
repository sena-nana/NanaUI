//! Auto-height animated disclosure primitives: [`Collapse`],
//! [`DisclosureToggle`], and [`InlineExpansion`].
//!
//! All spacing, radii, typography, and motion timings resolve from the
//! installed theme tokens so theme and reduced-motion preferences apply
//! automatically.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, Icon, JustifySpec, LengthSpec, MotionRole, OverflowSpec,
    PaintTransform, RadiusTier, SemanticColorRole, SpacingStep, TextWeight, TypeRole,
};

use super::controls::StyledComponent;
use super::node::{AnyView, IntoView, ViewBuilder, widget};
use super::prop::{FieldWrite, IntoProp, PropSource};
use super::reactive::signal;
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AnimatableProperty, ComponentView, IconGlyph,
    InteractionState, MutationQueue, NodeKind, NodeStyle, SizeChanged, StableNodeId, Stack, Text,
    UiWorld,
};

/// Smoothly animates its child's vertical height between collapsed (`0`) and
/// the child's measured natural height (`SizeChanged`).
///
/// While collapsed, the container clips overflow and hides descendants so
/// keyboard focus and pointer hits skip collapsed controls.
pub struct Collapse {
    open: PropSource<bool>,
    content: AnyView,
    animate: bool,
}

/// Create an auto-height [`Collapse`] container controlled by `open`.
pub fn collapse<V: IntoView>(open: impl IntoProp<bool>, content: V) -> Collapse {
    Collapse {
        open: open.into_source(),
        content: content.into_any(),
        animate: true,
    }
}

impl Collapse {
    /// Enable or disable animated transitions (defaults to `true`; respects
    /// theme reduced-motion).
    pub fn animate(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }
}

/// Bindable height writer for [`Collapse`]'s clipping shell.
struct CollapseHeight;

impl FieldWrite<Stack, (bool, Option<f32>)> for CollapseHeight {
    const FIELD: &'static str = "Collapse.height";

    fn write(target: &mut Stack, (open, measured): (bool, Option<f32>)) {
        let layout = Arc::make_mut(&mut target.node_style_mut().layout);
        layout.overflow_y = OverflowSpec::Hidden;
        layout.overflow_x = OverflowSpec::Visible;
        layout.min_height = Some(LengthSpec::Px(f32::default()));
        if open {
            layout.height = measured.map(LengthSpec::Px);
            layout.hidden = false;
            layout.opacity = None;
        } else {
            layout.height = Some(LengthSpec::Px(f32::default()));
            layout.hidden = true;
            layout.opacity = Some(f32::default());
        }
    }

    fn differs(target: &Stack, (open, measured): &(bool, Option<f32>)) -> bool {
        let layout = &target.node_style().layout;
        let expected_height = if *open {
            measured.map(LengthSpec::Px)
        } else {
            Some(LengthSpec::Px(f32::default()))
        };
        layout.height != expected_height || layout.hidden != !*open
    }
}

impl IntoView for Collapse {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let measured = signal::<Option<f32>>(None);
        let open = self.open;
        let inner = widget(Stack::column(f32::default()).width(LengthSpec::Fill))
            .on(move |size: &SizeChanged| {
                if size.height.is_finite() && size.height >= f32::default() {
                    let next = Some(size.height);
                    if measured.get_untracked() != next {
                        measured.set(next);
                    }
                }
            })
            .children(self.content);

        let outer_stack = Stack::column(f32::default())
            .width(LengthSpec::Fill)
            .with_layout(|layout| {
                layout.overflow_y = OverflowSpec::Hidden;
                layout.min_height = Some(LengthSpec::Px(f32::default()));
            });
        let mut outer = widget(outer_stack)
            .prop::<(bool, Option<f32>), CollapseHeight>(move || (open.get(), measured.get()));
        if self.animate {
            let duration =
                nana_ui_core::MotionTokens::DEFAULT.duration(MotionRole::SidebarCollapse);
            outer = outer.animate([super::Implicit::new(AnimatableProperty::Opacity, duration)]);
        }
        outer.children(inner).build(vb);
    }
}

/// Rotating chevron + accessible `aria-expanded` button driving a disclosure.
#[derive(Debug, Clone, PartialEq)]
struct DisclosureHeaderButton {
    label: Arc<str>,
    open: bool,
    disabled: bool,
    size: ControlSize,
    style: NodeStyle,
}

impl DisclosureHeaderButton {
    fn new(label: Arc<str>, open: bool, disabled: bool, size: ControlSize) -> Self {
        let mut style = NodeStyle::default();
        style.foreground = Some(SemanticColorRole::Text);
        style.interaction.hovered.background = Some(SemanticColorRole::Hover);
        style.interaction.pressed.background = Some(SemanticColorRole::Active);
        style.interaction.disabled.foreground = Some(SemanticColorRole::Muted);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.direction = Some(crate::FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Start;
        layout.theme.gap = Some(crate::theme_intent::space(SpacingStep::Xs));
        layout.theme.padding_left = Some(crate::theme_intent::space(SpacingStep::Xs));
        layout.theme.padding_right = Some(crate::theme_intent::space(SpacingStep::Xs));
        layout.theme.padding_top = Some(crate::theme_intent::space(SpacingStep::Xxs));
        layout.theme.padding_bottom = Some(crate::theme_intent::space(SpacingStep::Xxs));
        layout.theme.border_radius = Some(nana_ui_core::ThemeToken::Radius(RadiusTier::Sm).into());
        Self {
            label,
            open,
            disabled,
            size,
            style,
        }
    }
}

impl ComponentView for DisclosureHeaderButton {
    const TYPE_ID: Option<&'static str> = None;

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(|cx, entity| {
            let disabled = cx.read(entity, |b| b.disabled)?;
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
            tag: "disclosure-toggle".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut effective = self.style.clone();
        let metrics = world.node_theme(id).metrics();
        let min_h = self.size.height_in(metrics);
        Arc::make_mut(&mut effective.layout).min_height = Some(LengthSpec::Px(min_h));
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
                expanded: Some(self.open),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

struct DisclosureOpen;
impl FieldWrite<DisclosureHeaderButton, bool> for DisclosureOpen {
    const FIELD: &'static str = "DisclosureToggle.open";
    fn write(target: &mut DisclosureHeaderButton, value: bool) {
        target.open = value;
    }
    fn differs(target: &DisclosureHeaderButton, value: &bool) -> bool {
        target.open != *value
    }
}

struct DisclosureLabel;
impl FieldWrite<DisclosureHeaderButton, Arc<str>> for DisclosureLabel {
    const FIELD: &'static str = "DisclosureToggle.label";
    fn write(target: &mut DisclosureHeaderButton, value: Arc<str>) {
        target.label = value;
    }
    fn differs(target: &DisclosureHeaderButton, value: &Arc<str>) -> bool {
        target.label != *value
    }
}

struct DisclosureDisabled;
impl FieldWrite<DisclosureHeaderButton, bool> for DisclosureDisabled {
    const FIELD: &'static str = "DisclosureToggle.disabled";
    fn write(target: &mut DisclosureHeaderButton, value: bool) {
        target.disabled = value;
    }
    fn differs(target: &DisclosureHeaderButton, value: &bool) -> bool {
        target.disabled != *value
    }
}

struct ChevronRotation;
impl FieldWrite<IconGlyph, bool> for ChevronRotation {
    const FIELD: &'static str = "DisclosureToggle.chevron_rotation";
    fn write(target: &mut IconGlyph, open: bool) {
        let layout = Arc::make_mut(&mut target.style.layout);
        layout.transforms.transform = if open {
            // Rotate ChevronRight 90 degrees clockwise when expanded.
            Some(PaintTransform {
                a: f32::default(),
                b: 1.0,
                c: -1.0,
                d: f32::default(),
                e: f32::default(),
                f: f32::default(),
            })
        } else {
            None
        };
    }
    fn differs(target: &IconGlyph, open: &bool) -> bool {
        target.style.layout.transforms.transform.is_some() != *open
    }
}

struct DisclosureTextContent;
impl FieldWrite<Text, String> for DisclosureTextContent {
    const FIELD: &'static str = "Text.value";
    fn write(target: &mut Text, value: String) {
        target.value = value;
    }
    fn differs(target: &Text, value: &String) -> bool {
        target.value != *value
    }
}

/// Interactive disclosure toggle carrying `aria-expanded` and a smoothly
/// rotating chevron icon.
pub struct DisclosureToggle {
    label: PropSource<Arc<str>>,
    open: PropSource<bool>,
    disabled: PropSource<bool>,
    size: ControlSize,
    trailing: Option<AnyView>,
    on_toggle: Option<Box<dyn FnMut(bool) + Send + 'static>>,
}

/// Create a [`DisclosureToggle`] labelled `label` reflecting `open`.
pub fn disclosure_toggle(
    label: impl IntoProp<Arc<str>>,
    open: impl IntoProp<bool>,
) -> DisclosureToggle {
    DisclosureToggle {
        label: label.into_source(),
        open: open.into_source(),
        disabled: PropSource::Const(false),
        size: ControlSize::Small,
        trailing: None,
        on_toggle: None,
    }
}

impl DisclosureToggle {
    pub fn disabled(mut self, disabled: impl IntoProp<bool>) -> Self {
        self.disabled = disabled.into_source();
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Optional trailing accessory placed at the far end of the toggle row.
    pub fn trailing<V: IntoView>(mut self, view: V) -> Self {
        self.trailing = Some(view.into_any());
        self
    }

    /// Called with the next open state (`!open`) whenever the toggle is
    /// activated by pointer or keyboard.
    pub fn on_toggle(mut self, handler: impl FnMut(bool) + Send + 'static) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl IntoView for DisclosureToggle {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let open = Arc::new(std::sync::Mutex::new(self.open));
        let label = Arc::new(std::sync::Mutex::new(self.label));
        let disabled = Arc::new(std::sync::Mutex::new(self.disabled));
        let initial_open = super::untrack(|| open.lock().unwrap().get());
        let initial_label = super::untrack(|| label.lock().unwrap().get());
        let initial_disabled = super::untrack(|| disabled.lock().unwrap().get());

        let chevron_duration =
            nana_ui_core::MotionTokens::DEFAULT.duration(MotionRole::SidebarCollapse);
        let open_chevron = Arc::clone(&open);
        let chevron = widget(
            IconGlyph::new(Icon::ChevronRight)
                .size_role(TypeRole::Meta)
                .role(SemanticColorRole::Muted)
                .decorative(),
        )
        .prop::<bool, ChevronRotation>(move || open_chevron.lock().unwrap().get())
        .animate([super::Implicit::new(
            AnimatableProperty::Transform,
            chevron_duration,
        )]);

        let mut label_component = Text::new(initial_label.as_ref())
            .role(TypeRole::Body)
            .weight(TextWeight::Medium)
            .decorative();
        {
            let l = Arc::make_mut(&mut label_component.style.layout);
            l.flex_grow = Some(1.0);
            l.min_width = Some(LengthSpec::Px(f32::default()));
            l.text_overflow_ellipsis = true;
            l.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
        }
        let label_text_src = Arc::clone(&label);
        let label_text = widget(label_component).prop::<String, DisclosureTextContent>(move || {
            label_text_src.lock().unwrap().get().to_string()
        });

        let open_header = Arc::clone(&open);
        let label_header = Arc::clone(&label);
        let disabled_header = Arc::clone(&disabled);
        let mut header = widget(DisclosureHeaderButton::new(
            initial_label,
            initial_open,
            initial_disabled,
            self.size,
        ))
        .prop::<bool, DisclosureOpen>(move || open_header.lock().unwrap().get())
        .prop::<Arc<str>, DisclosureLabel>(move || label_header.lock().unwrap().get())
        .prop::<bool, DisclosureDisabled>(move || disabled_header.lock().unwrap().get());

        if let Some(mut on_toggle) = self.on_toggle {
            let open_click = Arc::clone(&open);
            let disabled_click = Arc::clone(&disabled);
            header = header.on(move |_: &Activate| {
                if !super::untrack(|| disabled_click.lock().unwrap().get()) {
                    on_toggle(!super::untrack(|| open_click.lock().unwrap().get()));
                }
            });
        }

        if let Some(trailing) = self.trailing {
            header.children((chevron, label_text, trailing)).build(vb);
        } else {
            header.children((chevron, label_text)).build(vb);
        }
    }
}

/// Complete inline disclosure pairing a [`DisclosureToggle`] header with an
/// auto-height animated [`Collapse`] body.
pub struct InlineExpansion {
    label: PropSource<Arc<str>>,
    open: PropSource<bool>,
    disabled: PropSource<bool>,
    size: ControlSize,
    indented: bool,
    animate: bool,
    trailing: Option<AnyView>,
    on_toggle: Option<Box<dyn FnMut(bool) + Send + 'static>>,
    content: AnyView,
}

/// Create an [`InlineExpansion`] combining a [`DisclosureToggle`] and
/// [`Collapse`] body.
pub fn inline_expansion<V: IntoView>(
    label: impl IntoProp<Arc<str>>,
    open: impl IntoProp<bool>,
    content: V,
) -> InlineExpansion {
    InlineExpansion {
        label: label.into_source(),
        open: open.into_source(),
        disabled: PropSource::Const(false),
        size: ControlSize::Small,
        indented: true,
        animate: true,
        trailing: None,
        on_toggle: None,
        content: content.into_any(),
    }
}

impl InlineExpansion {
    pub fn disabled(mut self, disabled: impl IntoProp<bool>) -> Self {
        self.disabled = disabled.into_source();
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Indent the expanded body by `SpacingStep::Lg` (default `true`).
    pub fn indented(mut self, indented: bool) -> Self {
        self.indented = indented;
        self
    }

    pub fn animate(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }

    pub fn trailing<V: IntoView>(mut self, view: V) -> Self {
        self.trailing = Some(view.into_any());
        self
    }

    pub fn on_toggle(mut self, handler: impl FnMut(bool) + Send + 'static) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl IntoView for InlineExpansion {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let open = Arc::new(std::sync::Mutex::new(self.open));
        let open_toggle = Arc::clone(&open);
        let open_collapse = Arc::clone(&open);
        let toggle = DisclosureToggle {
            label: self.label,
            open: PropSource::Dyn(Box::new(move || open_toggle.lock().unwrap().get())),
            disabled: self.disabled,
            size: self.size,
            trailing: self.trailing,
            on_toggle: self.on_toggle,
        };
        let body = if self.indented {
            widget(
                Stack::column(f32::default())
                    .width(LengthSpec::Fill)
                    .with_layout(|l| {
                        l.theme.padding_left = Some(crate::theme_intent::space(SpacingStep::Lg));
                        l.theme.padding_top = Some(crate::theme_intent::space(SpacingStep::Xs));
                    }),
            )
            .children(self.content)
            .into_any()
        } else {
            self.content
        };

        widget(Stack::column(f32::default()).width(LengthSpec::Fill))
            .children((
                toggle,
                collapse(move || open_collapse.lock().unwrap().get(), body).animate(self.animate),
            ))
            .build(vb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{signal, text};
    use crate::{AppContext, DocumentId, LayoutViewport};

    #[test]
    fn inline_expansion_toggles_aria_expanded_and_collapse_visibility() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let open = signal(false);
        let view = cx
            .mount_view_root(document, move || {
                inline_expansion("Advanced", open, text("Secret details"))
                    .animate(false)
                    .on_toggle(move |next| open.set(next))
                    .into_any()
            })
            .unwrap();
        cx.flush_reactive().unwrap();
        cx.layout_document(document, LayoutViewport::new(320.0, 240.0))
            .unwrap();

        let root = view.roots()[0];
        let children = cx.world().node(root).unwrap().children.clone();
        assert_eq!(children.len(), 2);
        let (toggle_id, collapse_id) = (children[0], children[1]);

        let a11y_closed = cx.world().accessibility(toggle_id).unwrap();
        assert_eq!(a11y_closed.role, AccessibilityRole::Button);
        assert_eq!(a11y_closed.expanded, Some(false));
        assert!(cx.world().node_style(collapse_id).unwrap().layout.hidden);

        // Activate toggle via framework activation.
        cx.activate_node(toggle_id).unwrap();
        cx.flush_reactive().unwrap();

        assert!(open.get_untracked());
        let a11y_open = cx.world().accessibility(toggle_id).unwrap();
        assert_eq!(a11y_open.expanded, Some(true));
        assert!(!cx.world().node_style(collapse_id).unwrap().layout.hidden);
    }
}
