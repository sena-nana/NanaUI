//! Auto-growing composers budget the same themed rows they shape and paint.
use nana_ui::{NanaTextShaper, runtime::*};
use nana_ui_core::{AlignSpec, Icon, LengthSpec, LineHeightSpec, ThemeDefinition};
use std::sync::Arc;

fn frames(document: &mut RuntimeDocument, shaper: &mut NanaTextShaper) {
    for _ in 0..10 {
        document
            .flush(LayoutViewport::new(600.0, 700.0), shaper)
            .unwrap();
    }
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.5,
        "actual={actual}, expected={expected}"
    );
}

#[test]
fn auto_grow_tracks_themed_rows_and_preserves_draft_selection_and_focus() {
    for footer in [false, true] {
        for explicit_line in [
            None,
            Some(LineHeightSpec::Absolute(40.0)),
            Some(LineHeightSpec::Relative(1.45)),
            Some(LineHeightSpec::Relative(2.0)),
        ] {
            let id = DocumentId::new(1).unwrap();
            let mut document = RuntimeDocument::new(id);
            let cx = document.context_mut();
            let root = cx
                .create_component(id, Stack::fill_column(0.0).align(AlignSpec::Start))
                .unwrap();
            let mut composer = TextArea::new("draft").auto_grow(1, 6);
            let layout = Arc::make_mut(&mut composer.style.layout);
            layout.width = Some(LengthSpec::Px(280.0));
            if let Some(line) = explicit_line {
                layout.theme.line_height = None;
                layout.line_height = Some(line);
            }
            if footer {
                let action = cx
                    .create_detached_component(id, IconButton::new(Icon::Close, "Send"))
                    .unwrap();
                composer = composer
                    .character_counter(1000)
                    .trailing_action(action.stable_id());
            }
            let area = cx.create_detached_component(id, composer).unwrap();
            cx.append_child(root, area).unwrap();
            cx.assemble_text_area(area).unwrap();
            let mut shaper = NanaTextShaper::default();
            frames(&mut document, &mut shaper);
            assert!(
                document
                    .context_mut()
                    .focus_node_in_place(id, area.stable_id())
                    .unwrap()
            );
            let baseline_line = explicit_line.map_or(16.0, |line| line.resolve_px(13.0));
            close(
                document
                    .context()
                    .world()
                    .layout_result(area.stable_id())
                    .unwrap()
                    .content_box
                    .height,
                baseline_line,
            );
            let long = (0..12)
                .map(|row| format!("message {row}"))
                .collect::<Vec<_>>()
                .join("\n");
            document
                .context_mut()
                .update_component(area, |area, _| area.state.replace_value(long))
                .unwrap();
            frames(&mut document, &mut shaper);
            close(
                document
                    .context()
                    .world()
                    .layout_result(area.stable_id())
                    .unwrap()
                    .content_box
                    .height,
                6.0 * baseline_line,
            );
            let draft = document
                .context()
                .read(area, |area| area.state.clone())
                .unwrap();
            let mut theme = ThemeDefinition::NANA_DARK;
            theme.typography.body = 24.0;
            theme.typography.line = 32.0;
            theme.typography.line_tall = 36.0;
            theme.tokens.metrics.control_height = 48.0;
            theme.tokens.metrics.compact_control_height = 42.0;
            theme.tokens.metrics.selection_height = 56.0;
            theme.tokens.border.hairline = 2.0;
            document
                .context_mut()
                .set_theme_definition(&theme.bump())
                .unwrap();
            frames(&mut document, &mut shaper);
            let current_line = explicit_line.map_or(32.0, |line| line.resolve_px(24.0));
            close(
                document
                    .context()
                    .world()
                    .layout_result(area.stable_id())
                    .unwrap()
                    .content_box
                    .height,
                6.0 * current_line,
            );
            assert_eq!(
                document
                    .context()
                    .read(area, |area| area.state.clone())
                    .unwrap(),
                draft
            );
            assert_eq!(
                document.context().world().focused(id),
                Some(area.stable_id())
            );
            document
                .context_mut()
                .update_component(area, |area, _| area.state.replace_value("short draft"))
                .unwrap();
            frames(&mut document, &mut shaper);
            close(
                document
                    .context()
                    .world()
                    .layout_result(area.stable_id())
                    .unwrap()
                    .content_box
                    .height,
                current_line,
            );
            document
                .context_mut()
                .set_theme_definition(&ThemeDefinition::NANA_DARK)
                .unwrap();
            frames(&mut document, &mut shaper);
            close(
                document
                    .context()
                    .world()
                    .layout_result(area.stable_id())
                    .unwrap()
                    .content_box
                    .height,
                baseline_line,
            );
            assert_eq!(
                document
                    .context()
                    .read(area, |area| area.state.value.to_string())
                    .unwrap(),
                "short draft"
            );
            assert_eq!(
                document.context().world().focused(id),
                Some(area.stable_id())
            );
        }
    }
}
