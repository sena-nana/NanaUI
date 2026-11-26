//! Roving-focus capsule bar ([`CapsuleBar`] / [`ExpandingTabBar`]) and
//! generic stage edge-slot / bottom-drawer layout helper ([`WorkbenchSlots`]).

use std::sync::{Arc, Mutex};

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, JustifySpec, LengthSpec, MotionRole,
    PaintTransform, PositionSpec, RadiusTier, SemanticColorRole, SpacingStep, TextWeight, TypeRole,
};

use super::controls::StyledComponent;
use super::node::{AnyView, IntoView, ViewBuilder, widget};
use super::prop::{FieldWrite, IntoProp, PropSource};
use super::structural::when;
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AnimatableProperty, ComponentView, IconGlyph,
    InteractionState, MutationQueue, NodeKind, NodeStyle, RovingFocusGroup, StableNodeId, Stack,
    Text, UiWorld,
};

/// Axis along which [`CapsuleBar`] lays out and roves focus across its items.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CapsuleOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// One item in a [`CapsuleBar`] / [`ExpandingTabBar`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsuleItem<K> {
    pub key: K,
    pub icon: Icon,
    pub label: Arc<str>,
    pub disabled: bool,
}

impl<K> CapsuleItem<K> {
    pub fn new(key: K, icon: Icon, label: impl Into<Arc<str>>) -> Self {
        Self {
            key,
            icon,
            label: label.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Pure selection transition for [`CapsuleBar`]: returns the next active key.
pub fn next_capsule_selection<K: PartialEq + Clone>(
    current: Option<&K>,
    clicked: &K,
    toggleable: bool,
) -> Option<K> {
    if toggleable && current == Some(clicked) {
        None
    } else {
        Some(clicked.clone())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CapsuleTabList {
    label: Arc<str>,
    orientation: CapsuleOrientation,
    style: NodeStyle,
}

impl CapsuleTabList {
    fn new(label: Arc<str>, orientation: CapsuleOrientation) -> Self {
        let mut style = NodeStyle::default();
        style.background = Some(SemanticColorRole::Surface);
        style.border = Some(SemanticColorRole::BorderSoft);
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(match orientation {
            CapsuleOrientation::Horizontal => FlexDirection::Row,
            CapsuleOrientation::Vertical => FlexDirection::Column,
        });
        layout.align_items = AlignSpec::Center;
        layout.theme.gap = Some(crate::theme_intent::space(SpacingStep::Xxs));
        layout.theme.padding_left = Some(crate::theme_intent::space(SpacingStep::Xxs));
        layout.theme.padding_right = Some(crate::theme_intent::space(SpacingStep::Xxs));
        layout.theme.padding_top = Some(crate::theme_intent::space(SpacingStep::Xxs));
        layout.theme.padding_bottom = Some(crate::theme_intent::space(SpacingStep::Xxs));
        layout.theme.border_radius =
            Some(nana_ui_core::ThemeToken::Radius(RadiusTier::Full).into());
        layout.border_width = Some(nana_ui_core::HAIRLINE);
        layout.theme.border_width =
            Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
        Self {
            label,
            orientation,
            style,
        }
    }
}

impl ComponentView for CapsuleTabList {
    const TYPE_ID: Option<&'static str> = None;

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "capsule-bar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let orientation = match self.orientation {
            CapsuleOrientation::Horizontal => crate::SelectionOrientation::Horizontal,
            CapsuleOrientation::Vertical => crate::SelectionOrientation::Vertical,
        };
        crate::view_components::project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::TabList,
                label: Some(Arc::clone(&self.label)),
                orientation: Some(orientation),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CapsuleTabButton {
    label: Arc<str>,
    selected: bool,
    disabled: bool,
    size: ControlSize,
    style: NodeStyle,
}

impl CapsuleTabButton {
    fn new(label: Arc<str>, selected: bool, disabled: bool, size: ControlSize) -> Self {
        let mut style = NodeStyle::default();
        style.interaction.hovered.background = Some(SemanticColorRole::Hover);
        style.interaction.pressed.background = Some(SemanticColorRole::Active);
        style.interaction.disabled.foreground = Some(SemanticColorRole::Muted);
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Center;
        layout.theme.gap = Some(crate::theme_intent::space(SpacingStep::Xs));
        layout.theme.padding_left = Some(crate::theme_intent::space(SpacingStep::Sm));
        layout.theme.padding_right = Some(crate::theme_intent::space(SpacingStep::Sm));
        layout.theme.border_radius =
            Some(nana_ui_core::ThemeToken::Radius(RadiusTier::Full).into());
        Self {
            label,
            selected,
            disabled,
            size,
            style,
        }
    }
}

impl ComponentView for CapsuleTabButton {
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
            tag: "capsule-tab".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut effective = self.style.clone();
        let h = self.size.height_in(world.node_theme(id).metrics());
        let layout = Arc::make_mut(&mut effective.layout);
        layout.min_height = Some(LengthSpec::Px(h));
        layout.min_width = Some(LengthSpec::Px(h));
        if self.selected {
            effective.background = Some(SemanticColorRole::AccentSoft);
            effective.foreground = Some(SemanticColorRole::Accent);
        } else {
            effective.background = None;
            effective.foreground = Some(SemanticColorRole::Muted);
        }
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
                role: AccessibilityRole::Tab,
                label: Some(Arc::clone(&self.label)),
                selected: Some(self.selected),
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

struct CapsuleSelected;
impl FieldWrite<CapsuleTabButton, bool> for CapsuleSelected {
    const FIELD: &'static str = "CapsuleTabButton.selected";
    fn write(target: &mut CapsuleTabButton, value: bool) {
        target.selected = value;
    }
    fn differs(target: &CapsuleTabButton, value: &bool) -> bool {
        target.selected != *value
    }
}

/// Roving-focus floating pill bar whose active tab optionally expands to show
/// its label beside its icon.
pub struct CapsuleBar<K: 'static> {
    items: Vec<CapsuleItem<K>>,
    active: PropSource<Option<K>>,
    orientation: CapsuleOrientation,
    toggleable: bool,
    expand_active_label: bool,
    size: ControlSize,
    label: Arc<str>,
    on_select: Option<Arc<Mutex<Box<dyn FnMut(Option<K>) + Send + 'static>>>>,
}

/// Alias matching tab-bar naming conventions.
pub type ExpandingTabBar<K> = CapsuleBar<K>;

/// Create a [`CapsuleBar`] over `items` bound to `active`.
pub fn capsule_bar<K: Clone + PartialEq + Send + 'static>(
    items: impl IntoIterator<Item = CapsuleItem<K>>,
    active: impl IntoProp<Option<K>>,
) -> CapsuleBar<K> {
    CapsuleBar {
        items: items.into_iter().collect(),
        active: active.into_source(),
        orientation: CapsuleOrientation::Horizontal,
        toggleable: false,
        expand_active_label: true,
        size: ControlSize::Small,
        label: Arc::from("Mode"),
        on_select: None,
    }
}

/// Alias constructor for [`capsule_bar`].
pub fn expanding_tab_bar<K: Clone + PartialEq + Send + 'static>(
    items: impl IntoIterator<Item = CapsuleItem<K>>,
    active: impl IntoProp<Option<K>>,
) -> ExpandingTabBar<K> {
    capsule_bar(items, active)
}

impl<K: 'static> CapsuleBar<K> {
    pub fn orientation(mut self, orientation: CapsuleOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Allow activating the already-active item to clear selection (`None`).
    pub fn toggleable(mut self, toggleable: bool) -> Self {
        self.toggleable = toggleable;
        self
    }

    /// Show the text label only for the active item (`true`, default) or for
    /// every item (`false`).
    pub fn expand_active_label(mut self, expand: bool) -> Self {
        self.expand_active_label = expand;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Accessible name for the enclosing `TabList`.
    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = label.into();
        self
    }

    pub fn on_select(mut self, handler: impl FnMut(Option<K>) + Send + 'static) -> Self {
        self.on_select = Some(Arc::new(Mutex::new(Box::new(handler))));
        self
    }
}

impl<K: Clone + PartialEq + Send + 'static> IntoView for CapsuleBar<K> {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let active = Arc::new(Mutex::new(self.active));
        let toggleable = self.toggleable;
        let expand_active_label = self.expand_active_label;
        let on_select = self.on_select;
        let size = self.size;
        let initial_active = super::untrack(|| active.lock().unwrap().get());

        let roving = match self.orientation {
            CapsuleOrientation::Horizontal => RovingFocusGroup::horizontal(),
            CapsuleOrientation::Vertical => RovingFocusGroup::vertical(),
        };

        let tabs: Vec<AnyView> = self
            .items
            .into_iter()
            .map(|item| {
                let key = item.key;
                let key_sel = key.clone();
                let key_label = key.clone();
                let key_click = key.clone();
                let active_label = Arc::clone(&active);
                let active_sel = Arc::clone(&active);
                let active_click = Arc::clone(&active);
                let initial_selected = initial_active.as_ref() == Some(&key);
                let label_for_text = Arc::clone(&item.label);
                let on_select = on_select.clone();

                let glyph = widget(
                    IconGlyph::new(item.icon)
                        .size_role(TypeRole::Meta)
                        .decorative(),
                );

                let label_view = when(
                    move || {
                        !expand_active_label
                            || active_label.lock().unwrap().get().as_ref() == Some(&key_label)
                    },
                    move || {
                        let mut text_view = Text::new(label_for_text.as_ref())
                            .role(TypeRole::Meta)
                            .weight(TextWeight::Medium)
                            .decorative();
                        Arc::make_mut(&mut text_view.style.layout).white_space =
                            nana_ui_core::WhiteSpaceSpec::Nowrap;
                        widget(text_view)
                    },
                );

                let mut tab = widget(CapsuleTabButton::new(
                    item.label,
                    initial_selected,
                    item.disabled,
                    size,
                ))
                .prop::<bool, CapsuleSelected>(move || {
                    active_sel.lock().unwrap().get().as_ref() == Some(&key_sel)
                });

                if let Some(cb) = on_select {
                    tab = tab.on(move |_: &Activate| {
                        let current = super::untrack(|| active_click.lock().unwrap().get());
                        let next = next_capsule_selection(current.as_ref(), &key_click, toggleable);
                        if let Ok(mut f) = cb.lock() {
                            f(next);
                        }
                    });
                }

                tab.children((glyph, label_view)).into_any()
            })
            .collect();

        widget(CapsuleTabList::new(self.label, self.orientation))
            .roving_focus(roving)
            .children(tabs)
            .build(vb);
    }
}

/// Bindable slide/fade presentation writer for [`WorkbenchSlots`]'s bottom
/// drawer.
struct DrawerPresence;

impl FieldWrite<Stack, bool> for DrawerPresence {
    const FIELD: &'static str = "WorkbenchSlots.bottom_drawer_open";

    fn write(target: &mut Stack, open: bool) {
        let layout = Arc::make_mut(&mut target.node_style_mut().layout);
        if open {
            layout.hidden = false;
            layout.opacity = None;
            layout.transforms.transform = None;
        } else {
            layout.hidden = true;
            layout.opacity = Some(f32::default());
            layout.transforms.transform = Some(PaintTransform {
                a: 1.0,
                b: f32::default(),
                c: f32::default(),
                d: 1.0,
                e: f32::default(),
                f: nana_ui_core::SpacingTokens::DEFAULT.resolve(SpacingStep::Lg),
            });
        }
    }

    fn differs(target: &Stack, open: &bool) -> bool {
        target.node_style().layout.hidden != !*open
    }
}

/// Generic edge-slot & animated bottom-drawer workbench overlay layout over a
/// full-bleed stage canvas.
///
/// Empty rail containers leave the stage clear while side slots and bottom
/// drawers stay cleanly separated without overlapping.
pub struct WorkbenchSlots {
    stage: Option<AnyView>,
    top_start: Option<AnyView>,
    top_center: Option<AnyView>,
    top_end: Option<AnyView>,
    left: Option<AnyView>,
    right: Option<AnyView>,
    bottom_center: Option<AnyView>,
    bottom_drawer: Option<(PropSource<bool>, AnyView)>,
}

/// Create an empty [`WorkbenchSlots`] layout builder.
pub fn workbench_slots() -> WorkbenchSlots {
    WorkbenchSlots {
        stage: None,
        top_start: None,
        top_center: None,
        top_end: None,
        left: None,
        right: None,
        bottom_center: None,
        bottom_drawer: None,
    }
}

impl WorkbenchSlots {
    pub fn stage<V: IntoView>(mut self, view: V) -> Self {
        self.stage = Some(view.into_any());
        self
    }

    pub fn top_start<V: IntoView>(mut self, view: V) -> Self {
        self.top_start = Some(view.into_any());
        self
    }

    pub fn top_center<V: IntoView>(mut self, view: V) -> Self {
        self.top_center = Some(view.into_any());
        self
    }

    pub fn top_end<V: IntoView>(mut self, view: V) -> Self {
        self.top_end = Some(view.into_any());
        self
    }

    pub fn left<V: IntoView>(mut self, view: V) -> Self {
        self.left = Some(view.into_any());
        self
    }

    pub fn right<V: IntoView>(mut self, view: V) -> Self {
        self.right = Some(view.into_any());
        self
    }

    pub fn bottom_center<V: IntoView>(mut self, view: V) -> Self {
        self.bottom_center = Some(view.into_any());
        self
    }

    /// Bottom edge drawer whose open state animates smoothly via presentation
    /// opacity and translation without shifting side rails.
    pub fn bottom_drawer<V: IntoView>(mut self, open: impl IntoProp<bool>, view: V) -> Self {
        self.bottom_drawer = Some((open.into_source(), view.into_any()));
        self
    }
}

impl IntoView for WorkbenchSlots {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let mut root_children: Vec<AnyView> = Vec::new();

        if let Some(stage) = self.stage {
            root_children.push(
                widget(Stack::column(f32::default()).with_layout(|l| {
                    l.position = PositionSpec::Absolute;
                    l.offset_left = Some(LengthSpec::Px(f32::default()));
                    l.offset_right = Some(LengthSpec::Px(f32::default()));
                    l.offset_top = Some(LengthSpec::Px(f32::default()));
                    l.offset_bottom = Some(LengthSpec::Px(f32::default()));
                }))
                .children(stage)
                .into_any(),
            );
        }

        let mut overlay_rows: Vec<AnyView> = Vec::new();

        // Top rail (start / center / end).
        if self.top_start.is_some() || self.top_center.is_some() || self.top_end.is_some() {
            let mut top_cells: Vec<AnyView> = Vec::new();
            let slot_wrap = |align: JustifySpec, content: Option<AnyView>| {
                let stack = Stack::row(f32::default())
                    .justify(align)
                    .align(AlignSpec::Center)
                    .with_layout(|l| {
                        l.flex_grow = Some(1.0);
                        l.flex_basis = Some(LengthSpec::Px(f32::default()));
                        l.min_width = Some(LengthSpec::Px(f32::default()));
                    });
                match content {
                    Some(view) => widget(stack).children(view).into_any(),
                    None => widget(stack).into_any(),
                }
            };
            top_cells.push(slot_wrap(JustifySpec::Start, self.top_start));
            top_cells.push(slot_wrap(JustifySpec::Center, self.top_center));
            top_cells.push(slot_wrap(JustifySpec::End, self.top_end));
            overlay_rows.push(
                widget(
                    Stack::row(f32::default())
                        .width(LengthSpec::Fill)
                        .gap_step(SpacingStep::Sm)
                        .align(AlignSpec::Center),
                )
                .children(top_cells)
                .into_any(),
            );
        }

        // Middle non-overlapping side slots (`left` and `right` separated by a flexible spacer).
        let mut middle_cells: Vec<AnyView> = Vec::new();
        if let Some(left) = self.left {
            middle_cells.push(
                widget(Stack::column(f32::default()).align(AlignSpec::Start))
                    .children(left)
                    .into_any(),
            );
        }
        middle_cells.push(
            widget(Stack::column(f32::default()).with_layout(|l| {
                l.flex_grow = Some(1.0);
                l.min_width = Some(LengthSpec::Px(f32::default()));
            }))
            .into_any(),
        );
        if let Some(right) = self.right {
            middle_cells.push(
                widget(Stack::column(f32::default()).align(AlignSpec::End))
                    .children(right)
                    .into_any(),
            );
        }
        overlay_rows.push(
            widget(
                Stack::row(f32::default())
                    .width(LengthSpec::Fill)
                    .gap_step(SpacingStep::Md)
                    .align(AlignSpec::Stretch)
                    .with_layout(|l| {
                        l.flex_grow = Some(1.0);
                        l.min_height = Some(LengthSpec::Px(f32::default()));
                    }),
            )
            .children(middle_cells)
            .into_any(),
        );

        // Animated bottom edge drawer.
        if let Some((open, drawer_content)) = self.bottom_drawer {
            let duration =
                nana_ui_core::MotionTokens::DEFAULT.duration(MotionRole::SidebarCollapse);
            let drawer = widget(
                Stack::column(f32::default())
                    .width(LengthSpec::Fill)
                    .align(AlignSpec::Center),
            )
            .prop::<bool, DrawerPresence>(move || open.get())
            .animate([
                super::Implicit::new(AnimatableProperty::Opacity, duration),
                super::Implicit::new(AnimatableProperty::Transform, duration),
            ])
            .children(drawer_content)
            .into_any();
            overlay_rows.push(drawer);
        }

        // Bottom center capsule bar slot.
        if let Some(bottom_center) = self.bottom_center {
            overlay_rows.push(
                widget(
                    Stack::row(f32::default())
                        .width(LengthSpec::Fill)
                        .justify(JustifySpec::Center)
                        .align(AlignSpec::Center),
                )
                .children(bottom_center)
                .into_any(),
            );
        }

        root_children.push(
            widget(
                Stack::column(f32::default())
                    .gap_step(SpacingStep::Sm)
                    .padding_step(SpacingStep::Md, SpacingStep::Md)
                    .with_layout(|l| {
                        l.position = PositionSpec::Absolute;
                        l.offset_left = Some(LengthSpec::Px(f32::default()));
                        l.offset_right = Some(LengthSpec::Px(f32::default()));
                        l.offset_top = Some(LengthSpec::Px(f32::default()));
                        l.offset_bottom = Some(LengthSpec::Px(f32::default()));
                    }),
            )
            .children(overlay_rows)
            .into_any(),
        );

        widget(
            Stack::column(f32::default())
                .width(LengthSpec::Fill)
                .height(LengthSpec::Fill)
                .with_layout(|l| {
                    l.position = PositionSpec::Relative;
                }),
        )
        .children(root_children)
        .build(vb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{signal, text};
    use crate::{AppContext, DocumentId, LayoutViewport};

    #[test]
    fn capsule_bar_supports_toggleable_selection_and_expands_active_label() {
        assert_eq!(next_capsule_selection(Some(&1u8), &1u8, true), None);
        assert_eq!(next_capsule_selection(Some(&1u8), &1u8, false), Some(1));
        assert_eq!(next_capsule_selection(Some(&1u8), &2u8, true), Some(2));

        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let active = signal(Some("scene"));
        let view = cx
            .mount_view_root(document, move || {
                capsule_bar(
                    [
                        CapsuleItem::new("scene", Icon::Search, "Scene"),
                        CapsuleItem::new("model", Icon::Add, "Model"),
                    ],
                    active,
                )
                .toggleable(true)
                .on_select(move |next| active.set(next))
                .into_any()
            })
            .unwrap();
        cx.flush_reactive().unwrap();
        cx.layout_document(document, LayoutViewport::new(400.0, 100.0))
            .unwrap();

        let tabs = cx.world().node(view.roots()[0]).unwrap().children.clone();
        assert_eq!(tabs.len(), 2);
        assert_eq!(
            cx.world().accessibility(tabs[0]).unwrap().selected,
            Some(true)
        );
        assert_eq!(
            cx.world().accessibility(tabs[1]).unwrap().selected,
            Some(false)
        );

        // Activating the second tab switches selection.
        cx.activate_node(tabs[1]).unwrap();
        cx.flush_reactive().unwrap();
        assert_eq!(active.get_untracked(), Some("model"));
        assert_eq!(
            cx.world().accessibility(tabs[1]).unwrap().selected,
            Some(true)
        );

        // Activating again toggles off when `toggleable(true)`.
        cx.activate_node(tabs[1]).unwrap();
        cx.flush_reactive().unwrap();
        assert_eq!(active.get_untracked(), None);
    }

    #[test]
    fn workbench_slots_mounts_stage_rails_and_toggles_bottom_drawer() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let drawer_open = signal(false);
        let view = cx
            .mount_view_root(document, move || {
                workbench_slots()
                    .stage(text("Stage"))
                    .top_start(text("Title"))
                    .left(text("Left Rail"))
                    .right(text("Right Rail"))
                    .bottom_drawer(drawer_open, text("Drawer Body"))
                    .bottom_center(text("Dock"))
                    .into_any()
            })
            .unwrap();
        cx.flush_reactive().unwrap();
        cx.layout_document(document, LayoutViewport::new(800.0, 600.0))
            .unwrap();
        let _ = view;

        drawer_open.set(true);
        cx.flush_reactive().unwrap();
    }
}
