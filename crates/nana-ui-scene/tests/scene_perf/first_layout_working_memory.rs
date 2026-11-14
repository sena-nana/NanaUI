//! What a page's first layout needs beyond what it keeps.
//!
//! A first layout touches every box, and each table a pass fills only to
//! copy into a retained one and free is memory the pass needs on top of the
//! page: the allocator keeps such blocks once they are freed, so a process
//! stays at the most it ever held. Three of them went: the queue of one
//! 240-byte mutation per laid-out box (the boxes are committed as 40-byte
//! placements), the pass's own table of container plans (the retained cache
//! takes it whole), and the list of every intrinsic fact gathered before
//! they were retained (they are read where the pass holds them).

use crate::{LIVE, PEAK, serial_lock};
use std::sync::atomic::Ordering;

use nana_ui_runtime::{Button, DocumentId, LayoutViewport, Stack, Text};
use nana_ui_scene::RuntimeDocument;

#[test]
fn a_first_layout_needs_little_beyond_what_the_page_keeps() {
    let _serial = serial_lock();
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    let column = cx.create_component(document, Stack::column(2.0)).unwrap();
    // Rows of a label and a button in a cell each: containers that record a
    // plan, and leaves.
    for index in 0..1_000 {
        let row = cx
            .create_detached_component(document, Stack::row(4.0))
            .unwrap();
        cx.append_child(column, row).unwrap();
        for cell in 0..2 {
            let holder = cx
                .create_detached_component(document, Stack::column(0.0))
                .unwrap();
            cx.append_child(row, holder).unwrap();
            if cell == 0 {
                let label = cx
                    .create_detached_component(document, Text::new(format!("row {index}")))
                    .unwrap();
                cx.append_child(holder, label).unwrap();
            } else {
                let button = cx
                    .create_detached_component(document, Button::new("Open"))
                    .unwrap();
                cx.append_child(holder, button).unwrap();
            }
        }
    }
    let nodes = runtime.context().world().len() as isize;

    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    runtime
        .context_mut()
        .layout_document_for_viewport(document, LayoutViewport::new(400.0, 600.0), &[])
        .unwrap();
    let kept = LIVE.load(Ordering::Relaxed);
    let most = PEAK.load(Ordering::Relaxed);

    // What the pass held at its fullest beyond what it left: 466 bytes a
    // node with the three tables, 150 without. The mutation queue alone
    // was about 390 of them.
    let working = (most - kept.max(before)) / nodes;
    assert!(
        working <= 300,
        "the first layout of {nodes} nodes held {working} bytes a node more than it kept \
         ({} kept a node)",
        (kept - before) / nodes
    );
}
