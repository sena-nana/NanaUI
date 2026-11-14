//! Selection options measure and paint the installed control typography.
use nana_ui::{NanaTextShaper, runtime::*};
use nana_ui_core::{AlignSpec, ControlSize, Icon, TextWeight, ThemeDefinition, ThemeToken};

fn frames(document: &mut RuntimeDocument, shaper: &mut NanaTextShaper) {
    for _ in 0..8 {
        document
            .flush(LayoutViewport::new(800.0, 600.0), shaper)
            .unwrap();
    }
}

fn label(context: &AppContext, id: StableNodeId) -> ComponentTextRegion {
    let Some(ComponentGeometry::SelectionOption { label, .. }) =
        context.world().component_geometry(id)
    else {
        panic!("selection option geometry");
    };
    label
}

#[test]
fn mounted_segmented_and_tabs_options_grow_with_typography_and_keep_complete_labels() {
    for size in [ControlSize::Small, ControlSize::Medium, ControlSize::Large] {
        for chrome in [SelectionChrome::Segmented, SelectionChrome::Tabs] {
            let id = DocumentId::new(1).unwrap();
            let mut document = RuntimeDocument::new(id);
            let cx = document.context_mut();
            let root = cx
                .create_component(id, Stack::fill_column(8.0).align(AlignSpec::Start))
                .unwrap();
            let control = cx
                .create_detached_component(id, SegmentedControl::new().size(size).chrome(chrome))
                .unwrap();
            let plain = cx
                .create_detached_component(id, SegmentedOption::new("暗色"))
                .unwrap();
            let icon = cx
                .create_detached_component(id, SegmentedOption::new("浅色").icon(Icon::Sun))
                .unwrap();
            let mut reference_style = NodeStyle::default();
            let layout = std::sync::Arc::make_mut(&mut reference_style.layout);
            layout.theme.control_typography(size);
            layout.theme.font_weight = Some(ThemeToken::Weight(TextWeight::Medium).into());
            layout.white_space_nowrap = true;
            let reference = cx
                .create_detached_component(id, Text::new("暗色").style(reference_style))
                .unwrap();
            cx.append_child(root, control).unwrap();
            cx.append_child(root, reference).unwrap();
            cx.set_segmented_options(control, vec![plain, icon], Some(icon))
                .unwrap();
            let mut shaper = NanaTextShaper::default();
            frames(&mut document, &mut shaper);
            let before = document
                .context()
                .world()
                .layout_box(control.stable_id())
                .unwrap()
                .width;
            assert!(
                document
                    .context_mut()
                    .focus_node_in_place(id, plain.stable_id())
                    .unwrap()
            );
            let mut theme = ThemeDefinition::NANA_DARK;
            theme.typography.meta = 18.0;
            theme.typography.body = 20.0;
            theme.typography.section = 24.0;
            theme.typography.line = 26.0;
            theme.typography.line_tall = 30.0;
            theme.typography.medium = 550;
            theme.tokens.spacing.xxs = 3.0;
            theme.tokens.spacing.xs = 6.0;
            theme.tokens.spacing.sm = 9.0;
            theme.tokens.metrics.compact_control_height = 42.0;
            theme.tokens.metrics.control_height = 48.0;
            theme.tokens.metrics.selection_height = 54.0;
            document
                .context_mut()
                .set_theme_definition(&theme.bump())
                .unwrap();
            frames(&mut document, &mut shaper);
            let cx = document.context();
            let world = cx.world();
            let expected_font = size.text_size_in(world.theme().typography());
            for option in [plain, icon] {
                let label = label(cx, option.stable_id());
                assert_eq!(
                    world.computed_style(option.stable_id()).unwrap().font_size,
                    expected_font
                );
                assert_eq!(label.font_size, expected_font);
                assert_eq!(label.font_weight, Some(550));
                let natural_width = world.text_metrics(reference.stable_id()).unwrap().width;
                assert!(
                    label.bounds.width >= natural_width - 0.5,
                    "size={size:?}, chrome={chrome:?}, label={label:?}, natural={natural_width}"
                );
                assert_ne!(cx.text_truncated(option), Some(true));
            }
            assert!(world.layout_box(control.stable_id()).unwrap().width > before + 4.0);
            assert_eq!(world.focused(id), Some(plain.stable_id()));
            assert_eq!(
                cx.read(control, |control| control.selected()).unwrap(),
                Some(icon.stable_id())
            );
        }
    }
}
