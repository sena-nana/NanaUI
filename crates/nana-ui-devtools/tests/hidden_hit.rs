#![cfg(feature = "agent")]

use nana_ui::runtime::{DocumentId, MutationQueue, NodeStyle, RuntimeDocument, Stack, TextInput};
use nana_ui_core::{LayoutStyle, LengthSpec, PaintTransform, VisibilitySpec};
use nana_ui_devtools::agent::RuntimeAgentSession;
use std::sync::Arc;

fn assert_accessible_path(
    session: &RuntimeAgentSession,
    document: DocumentId,
    target: nana_ui::runtime::StableNodeId,
) {
    let nodes = session
        .document()
        .context()
        .world()
        .project_accessibility(document)
        .into_iter()
        .map(|node| (node.id, node))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut cursor = Some(target);
    while let Some(id) = cursor {
        let entry = nodes
            .get(&id)
            .expect("visible editor must remain connected in accessibility");
        if let Some(parent) = entry.parent {
            assert!(
                nodes
                    .get(&parent)
                    .expect("missing accessible ancestor")
                    .children
                    .contains(&id)
            );
        }
        cursor = entry.parent;
    }
    assert!(nodes[&target].focused);
}

#[test]
#[ignore = "requires a GPU adapter; writes validation screenshots"]
fn hidden_scroller_clips_visible_editors_and_scrolls_their_hit_targets() {
    use nana_ui::runtime::{ScrollAxes, ScrollOffset, ScrollView};
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let cx = document.context_mut();
    let mut layout = LayoutStyle {
        width: Some(LengthSpec::Px(180.0)),
        height: Some(LengthSpec::Px(64.0)),
        ..Default::default()
    };
    layout.paint.visibility = Some(VisibilitySpec::Hidden);
    let scroll = cx
        .create_component(
            id,
            ScrollView::new(ScrollAxes::Vertical).style(NodeStyle {
                layout: Arc::new(layout),
                ..Default::default()
            }),
        )
        .unwrap();
    let column = cx.create_component(id, Stack::column(0.0)).unwrap();
    let mut mutations = MutationQueue::new();
    mutations.insert(scroll.stable_id(), column.stable_id(), None);
    let mut editors = Vec::new();
    for row in 0..4 {
        let mut layout = LayoutStyle {
            width: Some(LengthSpec::Px(160.0)),
            height: Some(LengthSpec::Px(32.0)),
            flex_shrink: Some(0.0),
            color: Some([1.0, 1.0, 1.0, 1.0]),
            background: Some([0.1, 0.1, 0.1, 1.0]),
            ..Default::default()
        };
        layout.paint.visibility = Some(VisibilitySpec::Visible);
        let editor = cx
            .create_component(
                id,
                TextInput::new(format!("Row {row}")).style(NodeStyle {
                    layout: Arc::new(layout),
                    ..Default::default()
                }),
            )
            .unwrap();
        mutations.insert(column.stable_id(), editor.stable_id(), None);
        editors.push(editor);
    }
    cx.commit_mutations(mutations).unwrap();
    let mut session = RuntimeAgentSession::new(document, 240, 160).unwrap();
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/hidden-hit");
    std::fs::create_dir_all(&output).unwrap();
    session
        .screenshot_png(output.join("scroll-before.png"))
        .unwrap();
    assert!(
        !session
            .document()
            .context()
            .world()
            .hit_test_candidates(id, 12.0, 80.0)
            .contains(&editors[2].stable_id())
    );
    session
        .document_mut()
        .context_mut()
        .scroll_to(scroll, ScrollOffset { x: 0.0, y: 32.0 })
        .unwrap();
    session.flush().unwrap();
    let bounds = session
        .document()
        .scene()
        .draw_node_bounds(editors[1].stable_id())
        .unwrap();
    assert_eq!(bounds.y, 0.0);
    assert!(bounds.width > 8.0 && bounds.height > 8.0);
    let hits = session
        .document()
        .context()
        .world()
        .hit_test_candidates(id, 12.0, 12.0);
    assert!(hits.contains(&editors[1].stable_id()));
    assert!(!hits.contains(&editors[0].stable_id()));
    session
        .screenshot_png(output.join("scroll-after.png"))
        .unwrap();
    session.click_xy(12.0, 12.0).unwrap();
    assert_accessible_path(&session, id, editors[1].stable_id());
    assert_eq!(
        session.document().context().world().focused(id),
        Some(editors[1].stable_id())
    );
}

#[test]
#[ignore = "requires a GPU adapter; writes validation screenshots"]
fn visible_editor_in_hidden_container_moves_with_its_hit_target() {
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let cx = document.context_mut();
    let root = cx.create_component(id, Stack::column(0.0)).unwrap();
    let mut container_style = NodeStyle::default();
    Arc::make_mut(&mut container_style.layout).paint.visibility = Some(VisibilitySpec::Hidden);
    let container = cx
        .create_component(id, Stack::column(0.0).style(container_style.clone()))
        .unwrap();
    let mut editor_layout = LayoutStyle {
        width: Some(LengthSpec::Px(140.0)),
        height: Some(LengthSpec::Px(32.0)),
        color: Some([0.1, 0.1, 0.1, 1.0]),
        ..Default::default()
    };
    editor_layout.paint.visibility = Some(VisibilitySpec::Visible);
    let editor = cx
        .create_component(
            id,
            TextInput::new("Visible editor").style(NodeStyle {
                layout: Arc::new(editor_layout),
                ..Default::default()
            }),
        )
        .unwrap();
    let mut mutations = MutationQueue::new();
    mutations.insert(root.stable_id(), container.stable_id(), None);
    mutations.insert(container.stable_id(), editor.stable_id(), None);
    cx.commit_mutations(mutations).unwrap();
    let mut session = RuntimeAgentSession::new(document, 360, 120).unwrap();
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/hidden-hit");
    std::fs::create_dir_all(&output).unwrap();
    session.screenshot_png(output.join("before.png")).unwrap();
    let before = session
        .document()
        .scene()
        .draw_node_bounds(editor.stable_id())
        .unwrap();

    Arc::make_mut(&mut container_style.layout).transform = Some(PaintTransform {
        e: 160.0,
        ..Default::default()
    });
    let mut mutations = MutationQueue::new();
    mutations.set_style(container.stable_id(), container_style);
    session
        .document_mut()
        .context_mut()
        .commit_mutations(mutations)
        .unwrap();
    session.flush().unwrap();
    let after = session
        .document()
        .scene()
        .draw_node_bounds(editor.stable_id())
        .unwrap();
    assert_eq!(after.x, before.x + 160.0);
    assert!(after.width > 8.0 && after.height > 8.0);
    assert!(
        !session
            .document()
            .context()
            .world()
            .hit_test_candidates(id, before.x + 12.0, before.y + 12.0)
            .contains(&editor.stable_id())
    );
    session.screenshot_png(output.join("after.png")).unwrap();
    session.click_xy(after.x + 12.0, after.y + 12.0).unwrap();
    assert_accessible_path(&session, id, editor.stable_id());
    assert_eq!(
        session.document().context().world().focused(id),
        Some(editor.stable_id())
    );
}
