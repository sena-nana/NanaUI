use super::*;
use crate::{
    AccessibilityRole, InteractionState, MutationQueue, NodeStyle, StableNodeId,
    TextHorizontalAlignment, UiWorld,
};
use std::sync::Arc;

#[cfg(test)]
mod hosted_textarea_tests {
    use super::*;
    use crate::DocumentId;

    #[test]
    fn a_disabled_icon_button_fades_and_an_enabled_one_does_not() {
        let mut context = crate::AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let opacity = |context: &crate::AppContext, id| {
            context
                .world()
                .node_style(id)
                .and_then(|style| style.layout.opacity)
        };
        let button = context
            .create_component(
                document,
                IconButton::new(nana_ui_core::Icon::Close, "Close"),
            )
            .unwrap();
        assert_eq!(opacity(&context, button.stable_id()), None);
        context
            .update_component(button, |button, _| button.disabled = true)
            .unwrap();
        assert_eq!(
            opacity(&context, button.stable_id()),
            Some(nana_ui_core::OpacityTokens::DARK.disabled)
        );
    }

    /// A button in its own colours takes its disabled look from its style
    /// too: the ink it mixed for the disabled state is not faded again, and
    /// an opacity it was given stays as written.
    #[test]
    fn a_disabled_icon_button_in_its_own_colors_does_not_fade() {
        let mut context = crate::AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let opacity = |context: &crate::AppContext, id| {
            context
                .world()
                .node_style(id)
                .and_then(|style| style.layout.opacity)
        };
        let mut own = IconButton::new(nana_ui_core::Icon::Close, "Close").colors_from_style();
        own.style.interaction.disabled.foreground_mix = Some(nana_ui_core::SemanticColorMix::new(
            nana_ui_core::SemanticColorRole::Text,
            nana_ui_core::SemanticColorRole::Surface,
            0.45,
        ));
        let button = context.create_component(document, own).unwrap();
        context
            .update_component(button, |button, _| button.disabled = true)
            .unwrap();
        assert_eq!(opacity(&context, button.stable_id()), None);
        context
            .update_component(button, |button, _| {
                Arc::make_mut(&mut button.style.layout).opacity = Some(0.8);
            })
            .unwrap();
        assert_eq!(opacity(&context, button.stable_id()), Some(0.8));
    }

    #[test]
    fn continuous_number_input_preserves_fractions_through_commit_step_and_revert() {
        let mut context = crate::AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(
                document,
                NumberInput::continuous(0.5)
                    .range(-1.0, 1.0)
                    .step(0.25)
                    .precision(0),
            )
            .unwrap();
        assert_eq!(context.read(input, NumberInput::value).unwrap(), 0.5);
        assert!(context.set_number_value(input, 0.123456789123).unwrap());
        assert_eq!(
            context
                .read(input, |field| field.state.value.clone())
                .unwrap(),
            "0.123456789123"
        );
        assert!(!context.step_number_input(input, 0).unwrap());
        assert!(context.step_number_input(input, 1).unwrap());
        assert_eq!(
            context.read(input, NumberInput::value).unwrap(),
            0.373456789123
        );
        assert_eq!(
            context
                .world()
                .accessibility(input.stable_id())
                .unwrap()
                .numeric_step,
            Some(0.25)
        );
        context
            .update_component(input, |field, _| {
                field.state.replace_value("0.000000123456789");
            })
            .unwrap();
        assert!(context.commit_number_input(input).unwrap());
        assert_eq!(
            context.read(input, NumberInput::value).unwrap(),
            0.000000123456789
        );
        context
            .update_component(input, |field, _| {
                field.state.replace_value("-");
            })
            .unwrap();
        context.commit_number_input(input).unwrap();
        assert_eq!(
            context
                .read(input, |field| field.state.value.parse::<f64>().unwrap())
                .unwrap(),
            0.000000123456789
        );
        let mut focus = MutationQueue::new();
        focus.request_focus(document, Some(input.stable_id()));
        context.commit_mutations(focus).unwrap();
        context
            .update_component(input, |field, _| {
                field.state.replace_value("uncommitted");
            })
            .unwrap();
        assert!(context.revert_focused_number_input(document).unwrap());
        assert_eq!(
            context
                .read(input, |field| field.state.value.parse::<f64>().unwrap())
                .unwrap(),
            0.000000123456789
        );
        context.set_number_value(input, 2.0).unwrap();
        assert_eq!(context.read(input, NumberInput::value).unwrap(), 1.0);
        assert_eq!(
            NumberInput::new(0.5).value(),
            1.0,
            "discrete defaults remain compatible"
        );
    }

    #[test]
    fn a_number_input_builder_normalizes_against_its_finished_rules() {
        let tenth = NumberInput::new(0.0).range(0.1, 5.0).step(0.1).precision(1);
        assert_eq!((tenth.value(), tenth.state.value.as_ref()), (0.1, "0.1"));
        let quarter = NumberInput::new(0.0)
            .range(0.25, 1.0)
            .step(0.25)
            .precision(2);
        assert_eq!(
            (quarter.value(), quarter.state.value.as_ref()),
            (0.25, "0.25")
        );
    }

    #[test]
    fn number_input_steps_from_the_value_published_by_the_application() {
        let mut context = crate::AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(
                document,
                NumberInput::new(1.0)
                    .range(0.0, 10.0)
                    .step(0.5)
                    .precision(1),
            )
            .unwrap();
        context
            .update_component(input, |input, _| {
                input.assign(7.0);
            })
            .unwrap();
        assert!(context.step_number_input(input, 1).unwrap());
        assert_eq!(context.read(input, NumberInput::value).unwrap(), 7.5);
        let nodes = context.world().project_accessibility(document);
        assert_eq!(nodes[0].numeric_value, Some(7.5));
    }

    #[test]
    fn textarea_gutter_tracks_line_count_and_restores_plain_text_spacing() {
        let mut context = crate::AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("source").line_numbers(true))
            .unwrap();
        let padding = |context: &crate::AppContext| {
            context.world().extract_nodes(&[area.stable_id()])[0]
                .source_style
                .layout
                .resolved_padding()
                .left
        };
        let single_line = padding(&context);
        let plain_padding = nana_ui_core::ControlPadding::Field.resolve(nana_ui_core::UI_METRICS);
        assert!(single_line > plain_padding);
        context
            .update_component(area, |area, _| {
                area.state.replace_value("\n".repeat(999));
            })
            .unwrap();
        assert!(padding(&context) > single_line);
        context
            .update_component(area, |area, _| {
                area.state.replace_value("source".to_owned());
            })
            .unwrap();
        assert_eq!(padding(&context), single_line);
        context
            .update_component(area, |area, _| area.line_numbers = false)
            .unwrap();
        assert_eq!(padding(&context), plain_padding);
    }

    #[test]
    fn textarea_gutter_preserves_larger_host_spacing() {
        let mut context = crate::AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut area = TextArea::new("source").line_numbers(true);
        Arc::make_mut(&mut area.style.layout).padding_left =
            Some(nana_ui_core::LengthSpec::Px(96.0));
        let area = context.create_component(document, area).unwrap();
        assert_eq!(
            context
                .world()
                .node_style(area.stable_id())
                .unwrap()
                .layout
                .resolved_padding()
                .left,
            96.0
        );
    }

    #[test]
    fn hosted_textarea_always_requests_the_highlight_presenter() {
        let editor = HostedTextarea::new("fn main() {}", "rs")
            .placeholder("fn main")
            .disabled(false);
        assert_eq!(editor.language(), Some("rs"));
        assert_eq!(
            editor
                .highlight
                .as_ref()
                .map(|request| request.presenter.as_ref()),
            Some(crate::HIGHLIGHT_PRESENTER)
        );

        let mut world = UiWorld::new();
        let id = StableNodeId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id, DocumentId::new(1).unwrap(), editor.node_kind());
        world.commit(queue).unwrap();
        let mut queue = MutationQueue::new();
        editor.project(id, &world, &mut queue);
        world.commit(queue).unwrap();
        assert_eq!(
            world
                .highlight_request(id)
                .map(|request| (request.presenter.as_ref(), request.language.as_ref())),
            Some((crate::HIGHLIGHT_PRESENTER, "rs"))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;

    fn mount(component: &ListItem) -> (UiWorld, StableNodeId) {
        let mut world = UiWorld::new();
        let id = StableNodeId::new(1).unwrap();
        let document = DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id, document, component.node_kind());
        component.project(id, &world, &mut queue);
        world.commit(queue).unwrap();
        (world, id)
    }

    #[test]
    fn a_row_can_say_it_is_a_button_or_a_toggle_without_changing_its_look() {
        let selected_paint = |world: &UiWorld, id| {
            let state = world.accessibility(id).unwrap();
            state.checked == Some(true) || state.mixed || state.selected == Some(true)
        };
        let (world, id) = mount(&ListItem::new("模型").selected(true));
        let state = world.accessibility(id).unwrap();
        assert_eq!(state.role, AccessibilityRole::ListItem);
        assert_eq!(state.selected, Some(true));

        let (world, id) = mount(
            &ListItem::new("模型")
                .selected(true)
                .role(ListItemRole::ToggleButton),
        );
        let state = world.accessibility(id).unwrap();
        assert_eq!(state.role, AccessibilityRole::Button);
        assert_eq!(state.checked, Some(true), "pressed, not selected");
        assert_eq!(state.selected, None);
        assert!(selected_paint(&world, id), "still drawn selected");

        let (world, id) = mount(&ListItem::new("模型").role(ListItemRole::ToggleButton));
        assert_eq!(world.accessibility(id).unwrap().checked, Some(false));
        assert!(!selected_paint(&world, id));

        let (world, id) = mount(&ListItem::new("Hiyori").role(ListItemRole::Button));
        let state = world.accessibility(id).unwrap();
        assert_eq!(state.role, AccessibilityRole::Button);
        assert_eq!(
            (state.checked, state.selected),
            (None, None),
            "a plain button"
        );
    }

    #[test]
    fn detail_travels_on_the_visual_not_the_node_text() {
        let item = ListItem::new("模型").detail("3 动作");
        let (world, id) = mount(&item);
        // 行文本只承载 label，detail 由几何端独立成区。
        assert_eq!(world.text(id), Some("模型"));
        assert_eq!(
            world.widget_visual_as::<ListItemVisual>(id),
            Some(&ListItemVisual {
                leading: None,
                content: None,
                trailing: None,
                note: None,
                detail: Some(Arc::from("3 动作")),
            })
        );
        // 节点文本不再拼接 detail，也就不再依赖 muted span。
        assert!(world.extract_nodes(&[id])[0].text_spans.is_empty());
    }

    #[test]
    fn a_note_follows_the_label_on_the_visual_and_in_the_name() {
        let item = ListItem::new("角度X")
            .note("ParamAngleX")
            .detail("头部左右转");
        let (world, id) = mount(&item);
        // 行文本只承载 label；note 与 detail 各自由几何端独立成区。
        assert_eq!(world.text(id), Some("角度X"));
        assert_eq!(
            world.widget_visual_as::<ListItemVisual>(id),
            Some(&ListItemVisual {
                leading: None,
                content: None,
                trailing: None,
                note: Some(Arc::from("ParamAngleX")),
                detail: Some(Arc::from("头部左右转")),
            })
        );
        // A note too narrow to draw is still said, after the label.
        assert_eq!(
            world.accessibility(id).unwrap().label.as_deref(),
            Some("角度X ParamAngleX  头部左右转")
        );
    }

    #[test]
    fn plain_rows_stay_solid() {
        let (world, id) = mount(&ListItem::new("纯标签"));
        assert_eq!(world.text(id), Some("纯标签"));
        assert_eq!(
            world.widget_visual_as::<ListItemVisual>(id),
            Some(&ListItemVisual {
                leading: None,
                content: None,
                trailing: None,
                note: None,
                detail: None,
            })
        );
        assert!(world.extract_nodes(&[id])[0].text_spans.is_empty());
    }

    #[test]
    fn content_slot_row_keeps_label_out_of_node_text() {
        let item = ListItem::new("模型")
            .note("Model")
            .detail("3 动作")
            .slots(ListItemSlots {
                content: Some(StableNodeId::new(2).unwrap()),
                ..ListItemSlots::default()
            });
        let (world, id) = mount(&item);
        assert_eq!(world.text(id), Some(""));
        assert_eq!(
            world.widget_visual_as::<ListItemVisual>(id),
            Some(&ListItemVisual {
                leading: None,
                content: Some(StableNodeId::new(2).unwrap()),
                trailing: None,
                note: None,
                detail: None,
            })
        );
    }

    #[test]
    fn trailing_slot_aligns_row_children_to_the_end() {
        let trailing = Some(StableNodeId::new(2).unwrap());
        let leading = Some(StableNodeId::new(3).unwrap());
        let justify = |item: ListItem| item.effective_style().layout.justify_content;
        assert_eq!(
            justify(ListItem::new("行").slots(ListItemSlots {
                leading,
                content: None,
                trailing,
            })),
            nana_ui_core::JustifySpec::SpaceBetween
        );
        assert_eq!(
            justify(ListItem::new("行").slots(ListItemSlots {
                leading: None,
                content: None,
                trailing,
            })),
            nana_ui_core::JustifySpec::End
        );
        assert_eq!(
            justify(ListItem::new("行")),
            nana_ui_core::JustifySpec::Start
        );
    }

    #[test]
    fn pill_bleed_extends_the_pill_and_keeps_text_inset() {
        // 样式合同：margin 取水平 padding 负值，文本内缩不变；默认不外扩。
        let px = nana_ui_core::LengthSpec::Px;
        let inset = nana_ui_core::ControlPadding::ListItem.resolve(nana_ui_core::UI_METRICS);
        let bled = ListItem::new("行")
            .pill_bleed(true)
            .effective_style()
            .layout;
        assert_eq!(
            ListItem::new("行").style.control_padding_x,
            Some(nana_ui_core::ControlPadding::ListItem)
        );
        assert_eq!(bled.margin_left, Some(px(-inset)));
        assert_eq!(bled.margin_right, bled.margin_left);
        assert_eq!(
            ListItem::new("行").effective_style().layout.margin_left,
            None
        );

        // 布局行为：行盒越出列表容器一个内边距，pill 占满可用宽度。
        let mut context = crate::AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut list_style = Stack::column(2.0).node_style();
        std::sync::Arc::make_mut(&mut list_style.layout).width = Some(px(240.0));
        let list = context
            .create_component(document, List::new().style(list_style))
            .unwrap();
        let row = context
            .create_component(document, ListItem::new("行").pill_bleed(true))
            .unwrap();
        context.append_child(list, row).unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(240.0, 120.0))
            .unwrap();
        let list_box = context
            .world()
            .canonical_layout_box(list.stable_id())
            .unwrap();
        let row_box = context
            .world()
            .canonical_layout_box(row.stable_id())
            .unwrap();
        assert!((row_box.x - (list_box.x - inset)).abs() < 0.5);
        assert!((row_box.width - (list_box.width + inset * 2.0)).abs() < 0.5);
    }
}

#[cfg(test)]
mod spacing_tests {
    use super::*;
    use nana_ui_core::{DirSpec, LengthSpec, PaddingSpec};

    #[test]
    fn wrapping_stack_reflows_controls_when_the_viewport_narrows() {
        let mut context = crate::AppContext::new();
        let document = crate::DocumentId::new(1).unwrap();
        let root = context
            .create_component(document, Stack::bar(8.0).wrap(true))
            .unwrap();
        let control = || {
            Stack::row(0.0)
                .width(LengthSpec::Px(100.0))
                .height(LengthSpec::Px(30.0))
        };
        let first = context
            .create_detached_component(document, control())
            .unwrap();
        let second = context
            .create_detached_component(document, control())
            .unwrap();
        context.append_child(root, first).unwrap();
        context.append_child(root, second).unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(240.0, 200.0))
            .unwrap();
        let first_box = context
            .world()
            .canonical_layout_box(first.stable_id())
            .unwrap();
        let second_box = context
            .world()
            .canonical_layout_box(second.stable_id())
            .unwrap();
        assert_eq!(first_box.y, second_box.y);
        context
            .layout_document(document, crate::LayoutViewport::new(160.0, 200.0))
            .unwrap();
        let first_box = context
            .world()
            .canonical_layout_box(first.stable_id())
            .unwrap();
        let second_box = context
            .world()
            .canonical_layout_box(second.stable_id())
            .unwrap();
        assert!(second_box.y >= first_box.y + first_box.height + 7.9);
        assert!((second_box.x - first_box.x).abs() < 0.1);
    }

    #[test]
    fn spacing_stack_last_padding_setter_wins_after_direction_change() {
        let stack = Stack::column(0.0)
            .padding_xy(12.0, 10.0)
            .with_layout(|layout| {
                layout
                    .logical_edges
                    .padding
                    .set_start(Some(LengthSpec::Px(30.0)));
                layout.logical_edges.padding.set(
                    nana_ui_core::LogicalEdge::BlockEnd,
                    Some(LengthSpec::Px(40.0)),
                );
            })
            .padding(0.0);
        let mut layout = (*stack.node_style().layout).clone();
        layout.dir = Some(DirSpec::Rtl);
        layout.bake_logical_edges();
        assert_eq!(layout.resolved_padding(), PaddingSpec::uniform(0.0));
        let layout = Stack::column(0.0)
            .padding(9.0)
            .padding_xy(2.0, 3.0)
            .node_style()
            .layout;
        assert_eq!(
            layout.resolved_padding(),
            PaddingSpec {
                left: 2.0,
                right: 2.0,
                top: 3.0,
                bottom: 3.0
            }
        );
    }

    #[test]
    fn spacing_card_defaults_restore_after_partial_style_removal() {
        let mut context = crate::AppContext::new();
        let document = crate::DocumentId::new(1).unwrap();
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).padding_top = Some(LengthSpec::Px(0.0));
        let card = context
            .create_component(document, Card::new().style(style))
            .unwrap();
        let padding = context.world().extract_nodes(&[card.stable_id()])[0]
            .source_style
            .layout
            .resolved_padding();
        assert_eq!(
            padding,
            PaddingSpec {
                top: 0.0,
                right: 16.0,
                bottom: 14.0,
                left: 16.0
            }
        );
        context
            .update_component(card, |card, _| {
                card.style = NodeStyle::default();
            })
            .unwrap();
        assert_eq!(
            context.world().extract_nodes(&[card.stable_id()])[0]
                .source_style
                .layout
                .resolved_padding(),
            PaddingSpec {
                top: 14.0,
                right: 16.0,
                bottom: 14.0,
                left: 16.0
            }
        );
        assert!(
            context
                .read(card, |card| card.style.layout.padding_left.is_none())
                .unwrap()
        );
    }
}

#[cfg(test)]
mod stack_preset_tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport};

    #[test]
    fn spacer_is_zero_width_flex_grow() {
        let spacer = Stack::spacer();
        let layout = spacer.node_style().layout;
        assert_eq!(layout.width, Some(nana_ui_core::LengthSpec::Px(0.0)));
        assert_eq!(layout.flex_grow, Some(1.0));
        assert_eq!(layout.min_width, Some(nana_ui_core::LengthSpec::Px(0.0)));
    }

    #[test]
    fn overlay_layer_fills_clips_and_does_not_hit() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let layer = context
            .create_component(document, Stack::overlay_layer())
            .unwrap();
        context
            .layout_document(document, LayoutViewport::new(320.0, 180.0))
            .unwrap();
        let style = context.world().node_style(layer.stable_id()).unwrap();
        assert_eq!(style.layout.position, nana_ui_core::PositionSpec::Absolute);
        assert_eq!(style.layout.overflow_x, nana_ui_core::OverflowSpec::Hidden);
        assert_eq!(
            style.layout.pointer_events,
            Some(nana_ui_core::PointerEventsSpec::None)
        );
        assert_eq!(
            context.world().interaction(layer.stable_id()),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
    }

    fn project_one(component: &impl ComponentView) -> (UiWorld, StableNodeId) {
        let mut world = UiWorld::new();
        let id = StableNodeId::new(7).unwrap();
        let document = DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id, document, component.node_kind());
        component.project(id, &world, &mut queue);
        world.commit(queue).unwrap();
        (world, id)
    }

    /// Controls drawn without text take an explicit spoken name and
    /// description, and a placeholder never stands in for the name.
    #[test]
    fn unlabelled_controls_take_an_accessible_name_and_description() {
        let spoken = |component: &dyn Fn() -> (UiWorld, StableNodeId)| {
            let (world, id) = component();
            let a11y = world.accessibility(id).unwrap().into_owned();
            (
                a11y.label.as_deref().map(str::to_owned),
                a11y.description.as_deref().map(str::to_owned),
            )
        };
        let named = (Some("Mute".to_owned()), Some("Silences alerts".to_owned()));
        assert_eq!(
            spoken(&|| project_one(
                &Switch::new("", true)
                    .accessible_name("Mute")
                    .accessible_description("Silences alerts")
            )),
            named
        );
        // Without an override the visible label still names the switch.
        assert_eq!(
            spoken(&|| project_one(&Switch::new("Mute", true))),
            (Some("Mute".to_owned()), None)
        );
        assert_eq!(
            spoken(&|| project_one(
                &RangeField::new(0.5, 0.0, 1.0, 0.1)
                    .label("Volume")
                    .accessible_name("Mute")
                    .accessible_description("Silences alerts")
            )),
            named
        );
        assert_eq!(
            spoken(&|| project_one(
                &TextInput::new("")
                    .placeholder("Search…")
                    .accessible_name("Mute")
                    .accessible_description("Silences alerts")
            )),
            named
        );
        assert_eq!(
            spoken(&|| project_one(&TextInput::new("").placeholder("Search…"))),
            (None, None)
        );
        assert_eq!(
            spoken(&|| project_one(
                &TextArea::new("")
                    .placeholder("Notes")
                    .accessible_name("Mute")
                    .accessible_description("Silences alerts")
            )),
            named
        );
        let options = [crate::SelectOption::new("a", "A")];
        assert_eq!(
            spoken(&|| project_one(
                &crate::Select::new(Some("a"))
                    .options(options.clone())
                    .accessible_name("Mute")
                    .accessible_description("Silences alerts")
            )),
            named
        );
        assert_eq!(
            spoken(&|| project_one(
                &crate::Dropdown::single(Some("a"))
                    .options([crate::DropdownOption::new("a", "A")])
                    .accessible_name("Mute")
                    .accessible_description("Silences alerts")
            )),
            named
        );
        assert_eq!(
            spoken(&|| project_one(
                &crate::RichTextEditor::new("")
                    .accessible_name("Mute")
                    .accessible_description("Silences alerts")
            )),
            named
        );
        // An empty name is none: the editor stays named by whatever names it.
        assert_eq!(
            spoken(&|| project_one(&crate::RichTextEditor::new("").accessible_name(""))),
            (None, None)
        );
    }

    #[test]
    fn decorative_text_stays_painted_and_leaves_the_accessibility_tree() {
        let (world, id) = project_one(&Text::new("睡").decorative());
        assert_eq!(world.text(id), Some("睡"));
        assert!(world.accessibility(id).unwrap().hidden);
        let (world, id) = project_one(&Text::new("睡"));
        assert!(!world.accessibility(id).unwrap().hidden);
    }

    #[test]
    fn button_accessible_name_overrides_the_visible_label() {
        let (world, id) = project_one(&Button::new("设置").accessible_name("设置 独立捕获窗口"));
        assert_eq!(world.text(id), Some("设置"));
        assert_eq!(
            world.accessibility(id).unwrap().label.as_deref(),
            Some("设置 独立捕获窗口")
        );
        let (world, id) = project_one(&Button::new("设置"));
        assert_eq!(
            world.accessibility(id).unwrap().label.as_deref(),
            Some("设置")
        );
    }

    #[test]
    fn start_aligned_button_places_its_group_at_the_content_edge() {
        let layout_label_x = |align: TextHorizontalAlignment| {
            let mut context = crate::AppContext::new();
            let document = crate::DocumentId::new(1).unwrap();
            let mut button = Button::new("会话").icon(nana_ui_core::Icon::Add);
            Arc::make_mut(&mut button.style.layout).width =
                Some(nana_ui_core::LengthSpec::Px(300.0));
            let button = context
                .create_component(document, button.content_align(align))
                .unwrap();
            context
                .layout_document(document, crate::LayoutViewport::new(400.0, 100.0))
                .unwrap();
            let bounds = context.world().layout_box(button.stable_id()).unwrap();
            let icon = context
                .world()
                .content_part_box(button.stable_id(), crate::LayoutPartKind::LeadingContent)
                .expect("button leading icon");
            (bounds, icon.x)
        };
        let (bounds, start) = layout_label_x(TextHorizontalAlignment::Start);
        let (_, centre) = layout_label_x(TextHorizontalAlignment::Center);
        assert!(start < bounds.x + 20.0, "start group hugs the left padding");
        assert!(centre > start + 80.0, "centred group sits well inside");
    }

    #[test]
    fn button_children_sit_where_the_label_would() {
        let child_offset = |style: Option<NodeStyle>| {
            let mut context = crate::AppContext::new();
            let document = crate::DocumentId::new(1).unwrap();
            let mut button = Button::new("");
            if let Some(style) = style {
                button = button.style(style);
            }
            let layout = Arc::make_mut(&mut button.style.layout);
            layout.width = Some(nana_ui_core::LengthSpec::Px(100.0));
            layout.height = Some(nana_ui_core::LengthSpec::Px(40.0));
            let button = context.create_component(document, button).unwrap();
            let child = context
                .create_component(
                    document,
                    Stack::row(0.0)
                        .width(nana_ui_core::LengthSpec::Px(40.0))
                        .height(nana_ui_core::LengthSpec::Px(20.0)),
                )
                .unwrap();
            context.append_child(button, child).unwrap();
            context
                .layout_document(document, crate::LayoutViewport::new(400.0, 100.0))
                .unwrap();
            let outer = context.world().layout_box(button.stable_id()).unwrap();
            let inner = context.world().layout_box(child.stable_id()).unwrap();
            (
                inner.x - outer.x,
                inner.y - outer.y,
                outer.x + outer.width - inner.x - inner.width,
                outer.y + outer.height - inner.y - inner.height,
            )
        };
        let mut row = NodeStyle::default();
        Arc::make_mut(&mut row.layout).direction = Some(nana_ui_core::FlexDirection::Row);
        for style in [None, Some(NodeStyle::default()), Some(row)] {
            let (left, top, right, bottom) = child_offset(style);
            assert!(
                (top - bottom).abs() < 0.5,
                "centred vertically: {top} {bottom}"
            );
            assert!((left - right).abs() < 0.5, "centred across: {left} {right}");
        }
        let mut placed = NodeStyle::default();
        Arc::make_mut(&mut placed.layout).align_items = nana_ui_core::AlignSpec::End;
        let (left, _, right, _) = child_offset(Some(placed));
        assert!(right < left, "an author alignment wins: {left} {right}");
    }

    #[test]
    fn horizontal_divider_stretches_on_the_cross_axis() {
        let divider = Divider::horizontal();
        assert_eq!(
            divider.style.layout.width,
            Some(nana_ui_core::LengthSpec::Fill)
        );
        assert_eq!(
            divider.style.layout.align_self,
            Some(nana_ui_core::AlignSpec::Stretch)
        );
    }

    #[test]
    fn an_unset_flow_axis_is_a_column_but_not_a_row() {
        use crate::{LayoutViewport, RuntimeLayoutEngine, StyleLayoutNode};
        use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec};

        let child = |name: &str| StyleLayoutNode {
            id: name.into(),
            style: LayoutStyle {
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Px(10.0)),
                ..LayoutStyle::default()
            },
            children: Vec::new(),
            text: None,
        };
        let laid_out = |direction: Option<FlexDirection>| {
            let root = StyleLayoutNode {
                id: "root".into(),
                style: LayoutStyle {
                    width: Some(LengthSpec::Px(300.0)),
                    height: Some(LengthSpec::Px(300.0)),
                    direction,
                    ..LayoutStyle::default()
                },
                children: vec![child("a"), child("b")],
                text: None,
            };
            RuntimeLayoutEngine
                .layout_style_tree(
                    &root,
                    LayoutViewport::new(320.0, 480.0),
                    &mut crate::MeasureTextShaper,
                )
                .into_iter()
                .find(|(name, _)| name == "b")
                .expect("second child")
                .1
        };
        assert_eq!(
            laid_out(None),
            laid_out(Some(FlexDirection::Column)),
            "dropping the nana.column seed has to be inert"
        );
        assert_ne!(
            laid_out(None),
            laid_out(Some(FlexDirection::Row)),
            "the nana.row seed has to stay"
        );
    }
}
