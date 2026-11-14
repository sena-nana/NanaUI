use nana_ui::runtime::{
    ActionMenuItem, Button, Chip, DocumentId, FormField, Icon, InlineContent, LengthSpec,
    RuntimeDocument, Select, SelectOption, Stack, TextInput,
};
use nana_ui_core::{PaintColor, RichSpanStyle};
use nana_ui_devtools::{agent::RuntimeAgentSession, offscreen};
use nana_ui_scene::{PrimitiveId, ScenePrimitiveKind};

#[test]
fn issue205_migrated_components_paint_from_retained_parts() {
    if !offscreen::pixels_available() {
        assert!(
            std::env::var_os("NANA_COMPONENT_REQUIRE_GPU").is_none(),
            "this run requires a GPU snapshot backend"
        );
        return;
    }
    let id = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(id);
    let cx = doc.context_mut();
    cx.set_reduced_motion(true);
    let root = cx
        .create_component(
            id,
            Stack::column(8.0)
                .padding(12.0)
                .width(LengthSpec::Px(300.0)),
        )
        .unwrap();
    let button = cx
        .create_detached_component(
            id,
            Button::new("Save document")
                .icon(Icon::Add)
                .trailing_icon(Icon::ChevronDown),
        )
        .unwrap();
    let chip = cx.create_detached_component(id, Chip::new("Beta")).unwrap();
    let menu = cx
        .create_detached_component(
            id,
            ActionMenuItem::new("Open document")
                .leading(Icon::Add)
                .hint("Ctrl+O"),
        )
        .unwrap();
    cx.append_child(root, button).unwrap();
    cx.append_child(root, chip).unwrap();
    cx.append_child(root, menu).unwrap();
    let mut session = RuntimeAgentSession::new(doc, 320, 200).unwrap();
    let (size, initial) = session.screenshot_rgba().unwrap();
    let generation = session.document().context().world().layout_generation();
    let (_, idle) = session.screenshot_rgba().unwrap();
    assert!(
        initial == idle,
        "idle frame retains identical content pixels"
    );
    assert_eq!(
        session
            .document()
            .context()
            .world()
            .hit_test(id, 150.0, 28.0),
        Some(button.stable_id())
    );
    session.hover_xy(150.0, 28.0).unwrap();
    // Reduced motion still permits a colour crossfade. Sample its settled
    // presentation time rather than asserting a change at the fade's start.
    session
        .document_mut()
        .context_mut()
        .advance_animations(std::time::Duration::from_secs(1));
    let (_, hover) = session.screenshot_rgba().unwrap();
    if let Some(dir) = std::env::var_os("NANA_COMPONENT_SNAPSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        offscreen::write_png(&dir.join("components.png"), size, &initial).unwrap();
        offscreen::write_png(&dir.join("components-hover.png"), size, &hover).unwrap();
    }
    assert!(initial != hover, "hover must paint the live control");
    assert_eq!(
        generation,
        session.document().context().world().layout_generation()
    );
}

#[test]
fn issue205_form_field_and_select_parts_reach_the_real_gpu_scene() {
    if !offscreen::pixels_available() {
        assert!(
            std::env::var_os("NANA_COMPONENT_REQUIRE_GPU").is_none(),
            "this run requires a GPU snapshot backend"
        );
        return;
    }
    let id = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(id);
    let cx = doc.context_mut();
    let root = cx
        .create_component(
            id,
            Stack::column(8.0)
                .padding(12.0)
                .width(LengthSpec::Px(300.0)),
        )
        .unwrap();
    let field = cx
        .create_detached_component(
            id,
            FormField::new("Email")
                .hint("Work email")
                .error("Invalid email"),
        )
        .unwrap();
    let input = cx
        .create_detached_component(id, TextInput::new("value"))
        .unwrap();
    cx.append_child(root, field).unwrap();
    cx.set_form_field_control(field, Some(input.stable_id()))
        .unwrap();
    let select = cx
        .create_detached_component(
            id,
            Select::new(Some("a")).options([
                SelectOption::new("a", "Alpha").hint("A"),
                SelectOption::new("b", "Beta").hint("B"),
            ]),
        )
        .unwrap();
    cx.append_child(root, select).unwrap();
    let rich_menu = cx
        .create_detached_component(
            id,
            ActionMenuItem::new("Open")
                .content(
                    InlineContent::builder()
                        .styled(
                            "Open",
                            RichSpanStyle::new().color(PaintColor::srgb([0.95, 0.35, 0.25, 1.0])),
                        )
                        .text(" document")
                        .build(),
                )
                .hint("⌘O"),
        )
        .unwrap();
    cx.append_child(root, rich_menu).unwrap();
    let mut session = RuntimeAgentSession::new(doc, 340, 260).unwrap();
    let (_, pixels) = session.screenshot_rgba().unwrap();
    let field_parts = session
        .document()
        .context()
        .world()
        .layout_result(field.stable_id())
        .unwrap()
        .parts
        .iter()
        .filter(|part| {
            matches!(
                part.kind,
                nana_ui::runtime::LayoutPartKind::Label
                    | nana_ui::runtime::LayoutPartKind::Hint
                    | nana_ui::runtime::LayoutPartKind::LeadingContent
            )
        })
        .count();
    let select_parts = session
        .document()
        .context()
        .world()
        .layout_result(select.stable_id())
        .unwrap()
        .parts
        .iter()
        .filter(|part| {
            matches!(
                part.kind,
                nana_ui::runtime::LayoutPartKind::TextContent
                    | nana_ui::runtime::LayoutPartKind::Hint
                    | nana_ui::runtime::LayoutPartKind::TrailingContent
            )
        })
        .count();
    assert_eq!(field_parts, 3);
    assert_eq!(select_parts, 3);
    assert!(pixels.iter().skip(3).step_by(4).any(|alpha| *alpha != 0));
    let rich_primitive = session
        .document()
        .scene()
        .primitive(PrimitiveId {
            node: rich_menu.stable_id(),
            slot: 2,
        })
        .expect("rich option label primitive");
    let ScenePrimitiveKind::Text { spans, .. } = &rich_primitive.kind else {
        panic!("rich option label must remain one text primitive")
    };
    assert!(
        !spans.is_empty(),
        "styled option content must reach Scene paint"
    );
    let generation = session
        .document()
        .context()
        .world()
        .layout_result(rich_menu.stable_id())
        .unwrap()
        .generation;
    session
        .document_mut()
        .context_mut()
        .update_component(rich_menu, |menu, _| {
            menu.content = Some(
                InlineContent::builder()
                    .styled(
                        "Open",
                        RichSpanStyle::new().color(PaintColor::srgb([0.20, 0.85, 0.45, 1.0])),
                    )
                    .text(" document")
                    .build()
                    .into(),
            );
        })
        .unwrap();
    session.screenshot_rgba().unwrap();
    assert_eq!(
        session
            .document()
            .context()
            .world()
            .layout_result(rich_menu.stable_id())
            .unwrap()
            .generation,
        generation,
        "rich paint-only option changes must retain chrome layout"
    );
}
