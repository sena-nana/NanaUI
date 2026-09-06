#![cfg(feature = "runtime-agent")]
//! Headless scroll / hover / key coverage for the Vue-free session.
//! Retained scroll and nested clipping only misbehave once something has
//! actually scrolled, so a session that cannot scroll cannot reproduce them.

use nana_ui::runtime::{
    Button, DocumentId, List, MutationQueue, NodeStyle, RuntimeDocument, ScrollAxes, ScrollView,
    SecondaryPress, StableNodeId,
};
use nana_ui_core::{LayoutStyle, LengthSpec};
use nana_ui_devtools::agent::RuntimeAgentSession;
use nana_ui_platform::InputModifiers;
use std::sync::Arc;

const ROWS: usize = 40;
const VIEWPORT_HEIGHT: f32 = 160.0;

/// A vertical scroller whose content is far taller than its viewport.
fn scrolling_document() -> (RuntimeDocument, Vec<StableNodeId>) {
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let cx = document.context_mut();
    let scroll = cx
        .create_component(
            id,
            ScrollView::new(ScrollAxes::Vertical).style(NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(320.0)),
                    height: Some(LengthSpec::Px(VIEWPORT_HEIGHT)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .unwrap();
    let list = cx.create_component(id, List::new()).unwrap();
    let mut mutations = MutationQueue::new();
    mutations.insert(scroll.stable_id(), list.stable_id(), None);
    let mut rows = Vec::with_capacity(ROWS);
    for row in 0..ROWS {
        let button = cx
            .create_component(id, Button::new(format!("row-{row}")))
            .unwrap();
        mutations.insert(list.stable_id(), button.stable_id(), None);
        rows.push(button.stable_id());
    }
    cx.commit_mutations(mutations).unwrap();
    (document, rows)
}

fn row_top(session: &RuntimeAgentSession, target: StableNodeId) -> Option<f32> {
    session
        .accessibility_dump()
        .into_iter()
        .find(|node| node.id == target.get())
        .map(|node| node.bounds.y)
}

#[test]
fn scroll_by_moves_retained_row_geometry() {
    let (document, rows) = scrolling_document();
    let mut session = RuntimeAgentSession::new(document, 360, 240).unwrap();
    let first = rows[0];
    let before = row_top(&session, first).expect("the first row projects into accessibility");

    session.scroll_by(160.0, 80.0, 0.0, -240.0).unwrap();

    let after = row_top(&session, first).expect("the row stays projected after scrolling");
    assert!(
        after < before,
        "scrolling down must move the first row up: {before} -> {after}"
    );
}

#[test]
fn hover_alone_does_not_move_or_activate() {
    let (document, rows) = scrolling_document();
    let mut session = RuntimeAgentSession::new(document, 360, 240).unwrap();
    let before = row_top(&session, rows[0]).unwrap();

    session.hover_xy(160.0, 80.0).unwrap();

    assert_eq!(
        before,
        row_top(&session, rows[0]).unwrap(),
        "a hover changes no geometry"
    );
}

#[test]
fn key_press_commits_no_text_into_labels() {
    let (document, rows) = scrolling_document();
    let mut session = RuntimeAgentSession::new(document, 360, 240).unwrap();

    session
        .key_press("ArrowDown", "ArrowDown", InputModifiers::default())
        .unwrap();

    let label = session
        .accessibility_dump()
        .into_iter()
        .find(|node| node.id == rows[0].get())
        .and_then(|node| node.label);
    assert_eq!(
        label.as_deref(),
        Some("row-0"),
        "a navigation key must not insert characters"
    );
}

/// Pixel evidence that `scroll_by` reaches the painter, not just the retained
/// tree. Writes both frames so the difference can be inspected by eye.
#[test]
#[ignore = "requires a GPU adapter; writes validation screenshots"]
fn scroll_by_changes_painted_pixels() {
    let (document, _) = scrolling_document();
    let mut session = RuntimeAgentSession::new(document, 360, 240).unwrap();

    session
        .screenshot_png("target/agent-scroll-before.png")
        .unwrap();
    let (_, before) = session.screenshot_rgba().unwrap();

    session.scroll_by(160.0, 80.0, 0.0, -240.0).unwrap();

    session
        .screenshot_png("target/agent-scroll-after.png")
        .unwrap();
    let (_, after) = session.screenshot_rgba().unwrap();

    assert_ne!(
        before, after,
        "scrolling must change the painted frame, not only the layout boxes"
    );
}

/// Context menus open on a secondary press. `click_xy` is button 0 and never
/// routes there, so without a secondary entry point every consumer that wants
/// to verify a right-click menu headlessly hand-rolls the same `InputEvent`.
#[test]
fn secondary_click_reaches_handlers_that_a_primary_click_does_not() {
    let (document, rows) = scrolling_document();
    let mut session = RuntimeAgentSession::new(document, 360, 240).unwrap();

    let presses = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = std::sync::Arc::clone(&presses);
    let first = rows[0];
    session
        .document_mut()
        .context_mut()
        .on(
            nana_ui::runtime::Entity::<Button>::from_stable_id(first),
            move |_, press: &SecondaryPress, _| {
                recorder.lock().unwrap().push(press.target);
            },
        )
        .unwrap();
    session.flush().unwrap();

    let top = row_top(&session, first).expect("the row projects into accessibility") + 4.0;
    session.click_xy(20.0, top).unwrap();
    assert!(
        presses.lock().unwrap().is_empty(),
        "a primary click must not raise SecondaryPress"
    );

    session.secondary_click_xy(20.0, top).unwrap();
    assert_eq!(
        presses.lock().unwrap().as_slice(),
        &[first],
        "a secondary click must reach the handler on the pressed row"
    );
}
