use nana_ui::runtime::{
    Button, DocumentId, InlineContent, InlineFlow, LengthSpec, RuntimeDocument, Stack, Text,
};
use nana_ui_devtools::{agent::RuntimeAgentSession, offscreen};

#[test]
fn native_mixed_paragraph_and_control_content_paint_live_children() {
    if !offscreen::pixels_available() {
        assert!(
            std::env::var_os("NANA_COMPONENT_REQUIRE_GPU").is_none(),
            "GPU snapshot backend required"
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
            Stack::column(12.0)
                .padding(12.0)
                .width(LengthSpec::Px(300.0)),
        )
        .unwrap();
    let child = cx
        .create_detached_component(id, Button::new("Details"))
        .unwrap();
    let flow = cx
        .create_detached_component(
            id,
            InlineFlow::new(
                InlineContent::builder()
                    .text("CPU ")
                    .child(child.stable_id())
                    .text(" 42%\nMixed content wraps with normal text and CJK 中文。")
                    .build(),
            )
            .width(LengthSpec::Px(280.0)),
        )
        .unwrap();
    cx.append_child(root, flow).unwrap();
    let label = cx.create_detached_component(id, Text::new("★")).unwrap();
    let control = cx
        .create_detached_component(
            id,
            Button::new("").content(
                InlineContent::builder()
                    .text("Save ")
                    .child(label.stable_id())
                    .build(),
            ),
        )
        .unwrap();
    cx.append_child(root, control).unwrap();
    let mut session = RuntimeAgentSession::new(doc, 320, 200).unwrap();
    let (size, before) = session.screenshot_rgba().unwrap();
    let (_, idle) = session.screenshot_rgba().unwrap();
    assert_eq!(before, idle);
    let world = session.document().context().world();
    let atomic = world.layout_box(label.stable_id()).unwrap();
    assert!(
        atomic.width > 0.0 && atomic.height > 0.0,
        "inline content child must be visible: {atomic:?}; paragraph {:?}",
        world.text_layout(control.stable_id())
    );
    assert_eq!(world.extract_nodes(&[label.stable_id()]).len(), 1);
    let text_box = world
        .layout_result(control.stable_id())
        .unwrap()
        .parts
        .iter()
        .find(|part| part.kind == nana_ui::runtime::LayoutPartKind::TextContent)
        .unwrap()
        .bounds;
    assert!(
        atomic.x >= text_box.x && atomic.x + atomic.width <= text_box.x + text_box.width + 0.01,
        "atomic {atomic:?} outside label {text_box:?}"
    );
    let sample = |x: u32, y: u32| {
        &before[((y * size.width + x) * 4) as usize..((y * size.width + x) * 4 + 3) as usize]
    };
    let background = sample(0, 0);
    assert!(
        (atomic.y as u32..(atomic.y + atomic.height).ceil() as u32).any(|y| (atomic.x as u32
            ..(atomic.x + atomic.width).ceil() as u32)
            .any(|x| sample(x, y) != background)),
        "atomic glyph must paint real pixels"
    );

    let mut faded = world.node_style(label.stable_id()).unwrap().clone();
    std::sync::Arc::make_mut(&mut faded.layout).opacity = Some(0.0);
    let mut queue = nana_ui::runtime::MutationQueue::new();
    queue.set_style(label.stable_id(), faded.clone());
    session
        .document_mut()
        .context_mut()
        .commit_mutations(queue)
        .unwrap();
    let (_, without_atomic) = session.screenshot_rgba().unwrap();
    assert!(
        before != without_atomic,
        "atomic child must contribute pixels independently of the parent's label"
    );
    std::sync::Arc::make_mut(&mut faded.layout).opacity = Some(1.0);
    let mut queue = nana_ui::runtime::MutationQueue::new();
    queue.set_style(label.stable_id(), faded);
    session
        .document_mut()
        .context_mut()
        .commit_mutations(queue)
        .unwrap();
    session.screenshot_rgba().unwrap();
    let bounds = session
        .document()
        .context()
        .world()
        .layout_box(child.stable_id())
        .unwrap();
    assert_eq!(
        session.document().context().world().hit_test(
            id,
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0
        ),
        Some(child.stable_id())
    );
    let generation = session.document().context().world().layout_generation();
    session
        .hover_xy(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
        .unwrap();
    session
        .document_mut()
        .context_mut()
        .advance_animations(std::time::Duration::from_secs(1));
    let (_, hover) = session.screenshot_rgba().unwrap();
    assert_ne!(before, hover);
    assert_eq!(
        session.document().context().world().layout_generation(),
        generation
    );
    if let Some(dir) = std::env::var_os("NANA_COMPONENT_SNAPSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        offscreen::write_png(&dir.join("inline-content.png"), size, &before).unwrap();
        offscreen::write_png(&dir.join("inline-content-hover.png"), size, &hover).unwrap();
    }
}
