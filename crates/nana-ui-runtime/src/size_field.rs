//! Width × height and a preset menu inside one text-field frame.
use std::sync::Arc;

use crate::view_components::{NumberChanged, NumberInput, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, ActionMenu, ActionMenuItem, Activate, AppContext,
    ComponentView, Entity, FocusWithinChanged, FrameworkError, InteractionState, MutationQueue,
    NodeKind, NodeStyle, StableNodeId, Text, UiWorld,
};
use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, LengthSpec, NumberFieldSpec, SemanticColorRole,
};

/// One named size. Identity is the width/height pair; labels may be localized.
#[derive(Debug, Clone, PartialEq)]
pub struct SizePreset {
    pub label: Arc<str>,
    pub value: [f64; 2],
    pub disabled: bool,
}

impl SizePreset {
    pub fn new(label: impl Into<Arc<str>>, value: [f64; 2]) -> Self {
        Self {
            label: label.into(),
            value,
            disabled: false,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// A committed pair, after either side is edited or a preset is chosen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeFieldChanged {
    pub value: [f64; 2],
}

/// Emitted when Enter confirms either side of a [`SizeField`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeFieldSubmitted {
    pub value: [f64; 2],
}

/// Two independently editable numbers and a square preset trigger, with one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct SizeField {
    pub value: [f64; 2],
    pub spec: NumberFieldSpec,
    pub presets: Vec<SizePreset>,
    pub size: ControlSize,
    pub disabled: bool,
    pub invalid: bool,
    pub label: Option<Arc<str>>,
    pub width_label: Option<Arc<str>>,
    pub height_label: Option<Arc<str>>,
    pub presets_label: Option<Arc<str>>,
    pub style: NodeStyle,
    pub width: Option<StableNodeId>,
    pub height: Option<StableNodeId>,
    pub menu: Option<StableNodeId>,
    separator: Option<StableNodeId>,
    items: Vec<([f64; 2], StableNodeId)>,
    focus_within: bool,
}

impl SizeField {
    pub fn new(value: [f64; 2]) -> Self {
        let spec = NumberFieldSpec::default();
        Self {
            value: value.map(|v| spec.snap(v)),
            spec,
            presets: Vec::new(),
            size: ControlSize::Medium,
            disabled: false,
            invalid: false,
            label: None,
            width_label: None,
            height_label: None,
            presets_label: None,
            style: field_style(ControlSize::Medium),
            width: None,
            height: None,
            menu: None,
            separator: None,
            items: Vec::new(),
            focus_within: false,
        }
    }
    pub fn value(mut self, value: [f64; 2]) -> Self {
        self.value = value.map(|v| self.spec.snap(v));
        self
    }
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.spec.minimum = Some(min);
        self.spec.maximum = Some(max);
        {
            let value = self.value;
            self.value(value)
        }
    }
    pub fn step(mut self, step: f64) -> Self {
        self.spec.step = step;
        {
            let value = self.value;
            self.value(value)
        }
    }
    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self.style = field_style(size);
        self
    }
    pub fn presets(mut self, presets: Vec<SizePreset>) -> Self {
        self.presets = presets;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }
    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn width_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.width_label = Some(label.into());
        self
    }
    pub fn height_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.height_label = Some(label.into());
        self
    }
    pub fn presets_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.presets_label = Some(label.into());
        self
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        if self.disabled {
            style.background = Some(SemanticColorRole::Subtle);
        } else if self.focus_within {
            style.border = Some(SemanticColorRole::FocusBorder);
        }
        if self.invalid {
            style.border = Some(SemanticColorRole::Danger);
        }
        style
    }
}

impl ComponentView for SizeField {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::SIZE_FIELD.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(AppContext::assemble_size_field),
        ..crate::TypeBehavior::NONE
    };
    fn wants_metrics_reproject() -> bool {
        true
    }
    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }
    fn reconcile(&mut self, mut next: Self) {
        next.width = self.width;
        next.height = self.height;
        next.menu = self.menu;
        next.separator = self.separator;
        next.items.clone_from(&self.items);
        next.focus_within = self.focus_within;
        *self = next;
    }
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "size-field".into(),
        }
    }
    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(
                    self.label
                        .clone()
                        .unwrap_or_else(|| world.framework_strings().size_field_label.clone()),
                ),
                value: Some(
                    format!(
                        "{} × {}",
                        self.spec.format(self.value[0]),
                        self.spec.format(self.value[1])
                    )
                    .into(),
                ),
                disabled: self.disabled,
                invalid: self.invalid,
                ..AccessibilityState::default()
            },
        );
    }
}

impl AppContext {
    /// Refresh the retained parts without replacing number drafts or menu focus.
    pub fn assemble_size_field(
        &mut self,
        field: Entity<SizeField>,
    ) -> Result<bool, FrameworkError> {
        let document = self
            .world()
            .node(field.stable_id())
            .ok_or(FrameworkError::MissingView(field.stable_id()))?
            .document;
        let snapshot = self.read(field, Clone::clone)?;
        let created = snapshot.width.is_none();
        let mut inputs = Vec::with_capacity(2);
        for (axis, id) in [snapshot.width, snapshot.height].into_iter().enumerate() {
            let input = match id.filter(|id| self.world().contains(*id)) {
                Some(id) => Entity::<NumberInput>::from_stable_id(id),
                None => {
                    let input =
                        self.create_detached_component(document, number(&snapshot, axis))?;
                    self.observe(input, field, move |field, event: &NumberChanged, cx| {
                        if !field.disabled {
                            field.value[axis] = field.spec.snap(event.value);
                            cx.emit(SizeFieldChanged { value: field.value });
                            cx.reassemble();
                        }
                    })?;
                    self.observe(
                        input,
                        field,
                        move |field, _event: &crate::NumberSubmitted, cx| {
                            if !field.disabled {
                                cx.emit(SizeFieldSubmitted { value: field.value });
                            }
                        },
                    )?;
                    input
                }
            };
            let label =
                if axis == 0 {
                    snapshot.width_label.clone().unwrap_or_else(|| {
                        self.world().framework_strings().size_field_width.clone()
                    })
                } else {
                    snapshot.height_label.clone().unwrap_or_else(|| {
                        self.world().framework_strings().size_field_height.clone()
                    })
                };
            self.update_component(input, |input, _| {
                let styled = number(&snapshot, axis);
                input.spec = snapshot.spec;
                input.publish(snapshot.value[axis]);
                input.label = Some(label);
                input.disabled = snapshot.disabled;
                input.invalid = snapshot.invalid;
                input.size = snapshot.size;
                input.steppers = false;
                input.style = styled.style;
                input.style_override = true;
            })?;
            inputs.push(input.stable_id());
        }
        let separator = match snapshot.separator.filter(|id| self.world().contains(*id)) {
            Some(id) => id,
            None => self
                .create_detached_component(document, Text::new("×").decorative())?
                .stable_id(),
        };
        let name = snapshot
            .presets_label
            .clone()
            .unwrap_or_else(|| self.world().framework_strings().size_field_presets.clone());
        let menu = match snapshot.menu.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<ActionMenu>::from_stable_id(id),
            None => self.create_detached_component(
                document,
                ActionMenu::new()
                    .trigger_icon(Icon::ChevronDown, name.clone())
                    .bare_trigger(true),
            )?,
        };
        let side =
            snapshot.size.height_in(self.world().theme_metrics()) - 2.0 * nana_ui_core::HAIRLINE;
        self.update_component(menu, |menu, _| {
            menu.disabled = snapshot.disabled || snapshot.presets.is_empty();
            menu.popover.open &= !menu.disabled;
            menu.popover.trigger = name;
            let mut style = NodeStyle::default();
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Px(side));
            layout.height = Some(LengthSpec::Px(side));
            layout.min_width = Some(LengthSpec::Px(side));
            layout.min_height = Some(LengthSpec::Px(side));
            layout.flex_shrink = Some(0.0);
            menu.popover.trigger_style = Some(style);
        })?;
        let mut items = Vec::new();
        for preset in &snapshot.presets {
            // Values are keys. Ignore duplicate keys rather than installing two handlers.
            if items.iter().any(|(value, _)| *value == preset.value) {
                continue;
            }
            let item = match snapshot
                .items
                .iter()
                .find(|(value, id)| *value == preset.value && self.world().contains(*id))
            {
                Some((_, id)) => Entity::<ActionMenuItem>::from_stable_id(*id),
                None => {
                    let item = self.create_detached_component(
                        document,
                        ActionMenuItem::new(preset.label.clone()),
                    )?;
                    let value = preset.value;
                    self.observe(item, field, move |field, _: &Activate, cx| {
                        if !field.disabled
                            && field
                                .presets
                                .iter()
                                .any(|p| p.value == value && !p.disabled)
                        {
                            field.value = value.map(|v| field.spec.snap(v));
                            cx.emit(SizeFieldChanged { value: field.value });
                            cx.reassemble();
                        }
                    })?;
                    item
                }
            };
            self.update_component(item, |item, _| {
                item.label = preset.label.clone();
                item.disabled = snapshot.disabled
                    || preset.disabled
                    || preset.value.iter().any(|v| !v.is_finite());
                item.active = preset.value == snapshot.value;
            })?;
            items.push((preset.value, item.stable_id()));
        }
        for (_, id) in &snapshot.items {
            if !items.iter().any(|(_, kept)| kept == id) && self.world().contains(*id) {
                self.despawn_node(*id)?;
            }
        }
        self.append_children(
            menu.stable_id(),
            &items.iter().map(|(_, id)| *id).collect::<Vec<_>>(),
        )?;
        if created {
            self.on(field, |field, event: &FocusWithinChanged, _| {
                field.focus_within = event.focused;
            })?;
        }
        self.update_component(field, |field, _| {
            field.width = Some(inputs[0]);
            field.height = Some(inputs[1]);
            field.separator = Some(separator);
            field.menu = Some(menu.stable_id());
            field.items = items;
        })?;
        self.append_children(
            field.stable_id(),
            &[inputs[0], separator, inputs[1], menu.stable_id()],
        )?;
        Ok(created)
    }
}

fn field_style(size: ControlSize) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::Background);
    style.border = Some(SemanticColorRole::Border);
    style.shape = Some(nana_ui_core::ShapeRole::Field);
    style.control_height = Some(nana_ui_core::ControlHeight::Min(size));
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.direction = Some(FlexDirection::Row);
    layout.align_items = AlignSpec::Center;
    layout.gap = Some(LengthSpec::Px(0.0));
    layout.border_width = Some(nana_ui_core::HAIRLINE);
    style
}

fn number(field: &SizeField, axis: usize) -> NumberInput {
    let mut input = NumberInput::new(field.value[axis])
        .size(field.size)
        .steppers(false);
    input.spec = field.spec;
    input.assign(field.value[axis]);
    input.style.background = None;
    input.style.border = None;
    input.style.clear_corners();
    input.style.control_height = None;
    input.style.interaction = crate::InteractionStyle {
        disabled: crate::SemanticPaint {
            foreground: Some(SemanticColorRole::Faint),
            ..Default::default()
        },
        ..Default::default()
    }
    .into();
    input.style_override = true;
    let layout = Arc::make_mut(&mut input.style.layout);
    layout.border_width = Some(0.0);
    layout.theme.border_width = None;
    layout.align_self = Some(AlignSpec::Stretch);
    layout.min_height = None;
    layout.width = Some(LengthSpec::Px(0.0));
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentId, LayoutViewport};
    use std::sync::Mutex;

    fn fixture() -> (AppContext, Entity<SizeField>, DocumentId) {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let field = cx
            .create_component(
                doc,
                SizeField::new([1920.0, 1080.0])
                    .range(2.0, 8192.0)
                    .step(2.0)
                    .size(ControlSize::Small)
                    .presets(vec![
                        SizePreset::new("720p", [1280.0, 720.0]),
                        SizePreset::new("Unavailable", [3840.0, 2160.0]).disabled(true),
                    ]),
            )
            .unwrap();
        cx.assemble_size_field(field).unwrap();
        (cx, field, doc)
    }

    #[test]
    fn editing_one_side_commits_a_pair_using_the_number_rules() {
        let (mut cx, field, doc) = fixture();
        let snapshot = cx.read(field, Clone::clone).unwrap();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let events = heard.clone();
        cx.on(field, move |_, event: &SizeFieldChanged, _| {
            events.lock().unwrap().push(event.value)
        })
        .unwrap();
        for (id, text, expected) in [
            (snapshot.width.unwrap(), "1279", [1280.0, 1080.0]),
            (snapshot.height.unwrap(), "9000", [1280.0, 8192.0]),
            (snapshot.height.unwrap(), "1", [1280.0, 2.0]),
        ] {
            cx.focus_node(doc, id).unwrap();
            cx.select_all_focused_text(doc).unwrap();
            cx.replace_focused_text(doc, text).unwrap();
            cx.commit_number_input(Entity::<NumberInput>::from_stable_id(id))
                .unwrap();
            assert_eq!(cx.read(field, |f| f.value).unwrap(), expected);
        }
        assert_eq!(
            *heard.lock().unwrap(),
            vec![[1280.0, 1080.0], [1280.0, 8192.0], [1280.0, 2.0]]
        );
        let mut input = crate::HeadlessInput::bind(&mut cx, doc);
        input
            .press(
                &mut cx,
                nana_ui_input::KeyInput::named(
                    "ArrowUp",
                    "ArrowUp",
                    nana_ui_input::KeyState::Pressed,
                    Default::default(),
                ),
                None,
                None,
            )
            .unwrap();
        assert_eq!(cx.read(field, |f| f.value).unwrap(), [1280.0, 4.0]);
    }

    #[test]
    fn a_preset_commits_once_and_closes_and_disabled_items_do_nothing() {
        let (mut cx, field, doc) = fixture();
        let snapshot = cx.read(field, Clone::clone).unwrap();
        let menu = snapshot.menu.unwrap();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let events = heard.clone();
        cx.on(field, move |_, event: &SizeFieldChanged, _| {
            events.lock().unwrap().push(event.value)
        })
        .unwrap();
        cx.activate_node(menu).unwrap();
        let disabled = Entity::<ActionMenuItem>::from_stable_id(snapshot.items[1].1);
        assert!(!cx.activate_action_menu_item(disabled).unwrap());
        let first = snapshot.items[0].1;
        cx.focus_node(doc, first).unwrap();
        assert!(cx.activate_node(first).unwrap());
        assert_eq!(*heard.lock().unwrap(), vec![[1280.0, 720.0]]);
        assert!(
            !cx.read(Entity::<ActionMenu>::from_stable_id(menu), |m| m
                .popover
                .open)
                .unwrap()
        );
        assert_eq!(cx.world().focused(doc), Some(menu));
        for (id, text) in [
            (snapshot.width.unwrap(), "1280"),
            (snapshot.height.unwrap(), "720"),
        ] {
            assert_eq!(
                cx.read(Entity::<NumberInput>::from_stable_id(id), |i| i
                    .state
                    .value
                    .to_string())
                    .unwrap(),
                text
            );
        }
    }

    #[test]
    fn refresh_preserves_an_unchanged_draft_and_updates_the_other_side() {
        let (mut cx, field, doc) = fixture();
        let snapshot = cx.read(field, Clone::clone).unwrap();
        let width = snapshot.width.unwrap();
        cx.focus_node(doc, width).unwrap();
        cx.select_all_focused_text(doc).unwrap();
        cx.replace_focused_text(doc, "12").unwrap();
        let before = cx
            .read(Entity::<NumberInput>::from_stable_id(width), |i| {
                i.state.clone()
            })
            .unwrap();
        cx.update_component(field, |f, _| {
            f.value[1] = 720.0;
        })
        .unwrap();
        let after = cx.read(field, Clone::clone).unwrap();
        assert_eq!(
            (after.width, after.height, after.menu),
            (snapshot.width, snapshot.height, snapshot.menu)
        );
        assert_eq!(
            cx.read(Entity::<NumberInput>::from_stable_id(width), |i| i
                .state
                .clone())
                .unwrap(),
            before
        );
        assert_eq!(cx.world().focused(doc), Some(width));
        assert!(cx.can_undo_text(width));
        assert_eq!(
            cx.read(
                Entity::<NumberInput>::from_stable_id(after.height.unwrap()),
                |i| i.value()
            )
            .unwrap(),
            720.0
        );
    }

    #[test]
    fn menu_refresh_keeps_identity_and_uses_current_presets() {
        let (mut cx, field, doc) = fixture();
        let snapshot = cx.read(field, Clone::clone).unwrap();
        let first = snapshot.items[0].1;
        cx.activate_node(snapshot.menu.unwrap()).unwrap();
        cx.focus_node(doc, first).unwrap();
        cx.update_component(field, |f, _| {
            f.presets[0].label = "HD".into();
            f.presets.swap(0, 1);
        })
        .unwrap();
        let after = cx.read(field, Clone::clone).unwrap();
        assert_eq!(after.items[1].1, first);
        assert_eq!(cx.world().focused(doc), Some(first));
        assert_eq!(
            cx.read(Entity::<ActionMenuItem>::from_stable_id(first), |i| i
                .label
                .clone())
                .unwrap()
                .as_ref(),
            "HD"
        );
        cx.activate_node(first).unwrap();
        assert_eq!(cx.read(field, |f| f.value).unwrap(), [1280.0, 720.0]);
        cx.update_component(field, |f, _| f.presets.clear())
            .unwrap();
        assert!(!cx.world().contains(first));
    }

    #[test]
    fn disabled_field_closes_its_menu_and_cannot_reopen_it() {
        let (mut cx, field, _) = fixture();
        let menu = cx.read(field, |f| f.menu.unwrap()).unwrap();
        cx.activate_node(menu).unwrap();
        cx.update_component(field, |f, _| f.disabled = true)
            .unwrap();
        assert!(!cx.activate_node(menu).unwrap());
        assert!(
            !cx.read(Entity::<ActionMenu>::from_stable_id(menu), |m| m
                .popover
                .open)
                .unwrap()
        );
        let snapshot = cx.read(field, Clone::clone).unwrap();
        for id in [snapshot.width.unwrap(), snapshot.height.unwrap(), menu] {
            assert!(cx.world().accessibility(id).unwrap().disabled);
        }
    }

    #[test]
    fn tab_order_names_focus_frame_and_escape_use_standard_controls() {
        let (mut cx, field, doc) = fixture();
        cx.update_component(field, |f, _| {
            f.width_label = Some("Width".into());
            f.height_label = Some("Height".into());
            f.presets_label = Some("Presets".into());
        })
        .unwrap();
        let snapshot = cx.read(field, Clone::clone).unwrap();
        let ids = [
            snapshot.width.unwrap(),
            snapshot.height.unwrap(),
            snapshot.menu.unwrap(),
        ];
        let mut input = crate::HeadlessInput::bind(&mut cx, doc);
        cx.layout_document(doc, LayoutViewport::new(400.0, 400.0))
            .unwrap();
        for (id, name) in ids.into_iter().zip(["Width", "Height", "Presets"]) {
            input
                .press(
                    &mut cx,
                    nana_ui_input::KeyInput::named(
                        "Tab",
                        "Tab",
                        nana_ui_input::KeyState::Pressed,
                        Default::default(),
                    ),
                    None,
                    None,
                )
                .unwrap();
            assert_eq!(cx.world().focused(doc), Some(id));
            assert_eq!(
                cx.world().accessibility(id).unwrap().label.as_deref(),
                Some(name)
            );
            assert_eq!(
                cx.world().node_style(field.stable_id()).unwrap().border,
                Some(SemanticColorRole::FocusBorder)
            );
        }
        for (logical, physical) in [("Enter", "Enter"), (" ", "Space")] {
            input
                .press(
                    &mut cx,
                    nana_ui_input::KeyInput::named(
                        logical,
                        physical,
                        nana_ui_input::KeyState::Pressed,
                        Default::default(),
                    ),
                    None,
                    None,
                )
                .unwrap();
            assert!(
                cx.read(Entity::<ActionMenu>::from_stable_id(ids[2]), |m| m
                    .popover
                    .open)
                    .unwrap()
            );
            cx.focus_node(doc, snapshot.items[0].1).unwrap();
            input
                .press(
                    &mut cx,
                    nana_ui_input::KeyInput::named(
                        "Escape",
                        "Escape",
                        nana_ui_input::KeyState::Pressed,
                        Default::default(),
                    ),
                    None,
                    None,
                )
                .unwrap();
            assert_eq!(cx.world().focused(doc), Some(ids[2]));
            assert!(
                !cx.read(Entity::<ActionMenu>::from_stable_id(ids[2]), |m| m
                    .popover
                    .open)
                    .unwrap()
            );
        }
    }

    #[test]
    fn narrow_layout_has_equal_inputs_one_frame_and_a_square_trigger() {
        for width in [180.0, 240.0, 360.0] {
            let (mut cx, field, doc) = fixture();
            cx.update_component(field, |f, _| {
                Arc::make_mut(&mut f.style.layout).width = Some(LengthSpec::Px(width))
            })
            .unwrap();
            cx.layout_document(doc, LayoutViewport::new(400.0, 400.0))
                .unwrap();
            let nodes = cx.world().document_order(doc);
            cx.resolve_styles(&nodes).unwrap();
            cx.shape_text(&nodes, &mut crate::MeasureTextShaper)
                .unwrap();
            let snapshot = cx.read(field, Clone::clone).unwrap();
            let frame = cx.world().layout_box(field.stable_id()).unwrap();
            let a = cx.world().layout_box(snapshot.width.unwrap()).unwrap();
            let b = cx.world().layout_box(snapshot.height.unwrap()).unwrap();
            let trigger = cx.world().layout_box(snapshot.menu.unwrap()).unwrap();
            assert!((a.width - b.width).abs() < 0.5, "{a:?} {b:?}");
            assert!((trigger.width - trigger.height).abs() < 0.5, "{trigger:?}");
            assert!(trigger.x + trigger.width <= frame.x + frame.width + 0.5);
            assert!(
                (frame.height - ControlSize::Small.height_in(cx.world().theme_metrics())).abs()
                    < 0.5,
                "{frame:?}"
            );
            for id in [snapshot.width.unwrap(), snapshot.height.unwrap()] {
                let style = cx.world().node_style(id).unwrap();
                assert_eq!((style.background, style.border), (None, None));
                assert!(
                    matches!(
                        cx.world().text_input_geometry(id),
                        Some(crate::TextInputGeometry { steppers: None, .. })
                    ),
                    "compound inputs have no spinner paint, hit regions or reserved width"
                );
            }
            cx.activate_node(snapshot.menu.unwrap()).unwrap();
            cx.layout_document(doc, LayoutViewport::new(400.0, 400.0))
                .unwrap();
            assert_eq!(cx.world().layout_box(field.stable_id()).unwrap(), frame);
        }
    }

    #[test]
    fn clicking_or_tabbing_into_side_selects_all_and_enter_emits_size_field_submitted() {
        let (mut cx, field, doc) = fixture();
        cx.layout_document(doc, LayoutViewport::new(400.0, 400.0))
            .unwrap();
        let order = cx.world().document_order(doc);
        cx.resolve_styles(&order).unwrap();
        let mut shaper = crate::MeasureTextShaper;
        cx.shape_text(&order, &mut shaper).unwrap();
        cx.rebuild_hit_test(doc);

        let changed = Arc::new(Mutex::new(Vec::new()));
        let submitted = Arc::new(Mutex::new(Vec::new()));
        let c_log = changed.clone();
        let s_log = submitted.clone();
        cx.on(field, move |_, ev: &SizeFieldChanged, _| {
            c_log.lock().unwrap().push(ev.value);
        })
        .unwrap();
        cx.on(field, move |_, ev: &SizeFieldSubmitted, _| {
            s_log.lock().unwrap().push(ev.value);
        })
        .unwrap();

        let snap = cx.read(field, Clone::clone).unwrap();
        let width_id = snap.width.unwrap();
        let height_id = snap.height.unwrap();
        let width_box = cx.world().layout_box(width_id).unwrap();
        let click_x = width_box.x + width_box.width / 2.0;
        let click_y = width_box.y + width_box.height / 2.0;

        let mut input = crate::HeadlessInput::bind(&mut cx, doc);
        input
            .pointer(&mut cx, nana_ui_input::PointerPhase::Down, click_x, click_y)
            .unwrap();
        input
            .pointer(&mut cx, nana_ui_input::PointerPhase::Up, click_x, click_y)
            .unwrap();
        assert_eq!(cx.world().focused(doc), Some(width_id));
        let sel = cx.world().text_input(width_id).unwrap().selection.ordered();
        assert_eq!(sel, 0..4, "unfocused click selects width draft");

        input.text(&mut cx, "640").unwrap();
        input
            .press(
                &mut cx,
                nana_ui_input::KeyInput::named(
                    "Enter",
                    "Enter",
                    nana_ui_input::KeyState::Pressed,
                    Default::default(),
                ),
                None,
                None,
            )
            .unwrap();
        assert_eq!(*changed.lock().unwrap(), vec![[640.0, 1080.0]]);
        assert_eq!(*submitted.lock().unwrap(), vec![[640.0, 1080.0]]);

        // Pressing Enter again when unchanged emits SizeFieldSubmitted without SizeFieldChanged.
        input
            .press(
                &mut cx,
                nana_ui_input::KeyInput::named(
                    "Enter",
                    "Enter",
                    nana_ui_input::KeyState::Pressed,
                    Default::default(),
                ),
                None,
                None,
            )
            .unwrap();
        assert_eq!(*changed.lock().unwrap(), vec![[640.0, 1080.0]]);
        assert_eq!(
            *submitted.lock().unwrap(),
            vec![[640.0, 1080.0], [640.0, 1080.0]]
        );

        // Tabbing to height selects all and Enter emits updated SizeFieldSubmitted.
        input
            .press(
                &mut cx,
                nana_ui_input::KeyInput::named(
                    "Tab",
                    "Tab",
                    nana_ui_input::KeyState::Pressed,
                    Default::default(),
                ),
                None,
                None,
            )
            .unwrap();
        assert_eq!(cx.world().focused(doc), Some(height_id));
        let sel = cx
            .world()
            .text_input(height_id)
            .unwrap()
            .selection
            .ordered();
        assert_eq!(sel, 0..4, "Tab selects height draft");

        input.text(&mut cx, "480").unwrap();
        input
            .press(
                &mut cx,
                nana_ui_input::KeyInput::named(
                    "Enter",
                    "Enter",
                    nana_ui_input::KeyState::Pressed,
                    Default::default(),
                ),
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            *changed.lock().unwrap(),
            vec![[640.0, 1080.0], [640.0, 480.0]]
        );
        assert_eq!(
            *submitted.lock().unwrap(),
            vec![[640.0, 1080.0], [640.0, 1080.0], [640.0, 480.0]]
        );
    }

    #[test]
    fn size_field_submit_handler_can_clear_focus_without_missing_child_view() {
        let (mut cx, field, doc) = fixture();
        cx.layout_document(doc, LayoutViewport::new(400.0, 400.0))
            .unwrap();
        let snap = cx.read(field, Clone::clone).unwrap();
        let width_id = snap.width.unwrap();
        let focus_within = Arc::new(Mutex::new(Vec::new()));
        let fw_log = focus_within.clone();
        cx.on(field, move |_, ev: &FocusWithinChanged, _| {
            fw_log.lock().unwrap().push(ev.focused);
        })
        .unwrap();
        cx.on(field, move |_, _: &SizeFieldSubmitted, cx| {
            cx.mutations().request_focus(doc, None);
        })
        .unwrap();

        assert!(cx.focus_node(doc, width_id).unwrap());
        assert!(cx.select_all_focused_text(doc).unwrap());
        let mut input = crate::HeadlessInput::bind(&mut cx, doc);
        input.text(&mut cx, "1280").unwrap();
        input
            .press(
                &mut cx,
                nana_ui_input::KeyInput::named(
                    "Enter",
                    "Enter",
                    nana_ui_input::KeyState::Pressed,
                    Default::default(),
                ),
                None,
                None,
            )
            .unwrap();

        assert_eq!(cx.world().focused(doc), None);
        assert!(!cx.read(field, |f| f.focus_within).unwrap());
        assert_eq!(*focus_within.lock().unwrap(), vec![true, false]);
        assert_eq!(cx.read(field, |f| f.value).unwrap(), [1280.0, 1080.0]);
        assert_eq!(
            cx.read(Entity::<NumberInput>::from_stable_id(width_id), |i| i
                .value())
                .unwrap(),
            1280.0
        );
    }
}
