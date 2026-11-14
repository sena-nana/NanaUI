use nana_ui::runtime::{
    DocumentId, LengthSpec, MutationQueue, OverflowSpec, PositionSpec, RuntimeDocument,
    ScrollOffset, Stack,
};
use nana_ui_devtools::{agent::RuntimeAgentSession, offscreen};

#[test]
fn issue200_sticky_real_document_pixels_follow_scroll_without_reflow() {
    if !offscreen::pixels_available() {
        assert!(
            std::env::var_os("NANA_STICKY_REQUIRE_GPU").is_none(),
            "this run requires a GPU snapshot backend"
        );
        return;
    }
    let id = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(id);
    let cx = doc.context_mut();
    let scroller = cx
        .create_component(
            id,
            Stack::column(0.0)
                .width(LengthSpec::Px(160.0))
                .height(LengthSpec::Px(100.0))
                .with_layout(|s| s.overflow_y = OverflowSpec::Scroll),
        )
        .unwrap();
    let group = cx
        .create_detached_component(
            id,
            Stack::column(0.0)
                .width(LengthSpec::Px(160.0))
                .height(LengthSpec::Px(200.0)),
        )
        .unwrap();
    cx.append_child(scroller, group).unwrap();
    for (height, sticky) in [(20.0, false), (20.0, true), (160.0, false)] {
        let row = cx
            .create_detached_component(
                id,
                Stack::column(0.0)
                    .width(LengthSpec::Px(160.0))
                    .height(LengthSpec::Px(height))
                    .with_layout(|s| {
                        s.flex_shrink = Some(0.0);
                        if sticky {
                            s.position = PositionSpec::Sticky;
                            s.offset_top = Some(LengthSpec::Px(0.0));
                            s.background = Some([1.0, 0.0, 0.0, 1.0]);
                            s.z_index = Some(1);
                        }
                    }),
            )
            .unwrap();
        cx.append_child(group, row).unwrap();
    }
    let tail = cx
        .create_detached_component(
            id,
            Stack::column(0.0)
                .width(LengthSpec::Px(160.0))
                .height(LengthSpec::Px(200.0)),
        )
        .unwrap();
    cx.append_child(scroller, tail).unwrap();
    let mut session = RuntimeAgentSession::new(doc, 160, 100).unwrap();
    let generation = session.document().context().world().layout_generation();
    let mut snapshots = Vec::new();
    for (offset, top_red) in [(0.0, false), (70.0, true), (190.0, true), (0.0, false)] {
        let mut q = MutationQueue::new();
        q.set_scroll_offset(scroller.stable_id(), ScrollOffset { x: 0.0, y: offset });
        session
            .document_mut()
            .context_mut()
            .commit_mutations(q)
            .unwrap();
        let (size, pixels) = session.screenshot_rgba().unwrap();
        let at = (5 * size.width as usize + 5) * 4;
        let red = pixels[at] > 240 && pixels[at + 1] < 15 && pixels[at + 2] < 15;
        assert_eq!(red, top_red, "sticky pixel after scrolling {offset}");
        assert_eq!(
            session.document().context().world().layout_generation(),
            generation
        );
        if let Some(dir) = std::env::var_os("NANA_STICKY_SNAPSHOT_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            offscreen::write_png(&dir.join(format!("sticky-{offset:.0}.png")), size, &pixels)
                .unwrap();
        }
        snapshots.push(pixels);
    }
    assert_eq!(
        snapshots[0], snapshots[3],
        "scrolling back must restore identical pixels"
    );
}
