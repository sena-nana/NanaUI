//! Issue #381: what the scroll path's work counters count, on a scroll-only
//! frame, a viewport resize and a virtual list scroll.
//!
//! A page as tall as the viewport scrolls two lists, `list` and `other`.
//! Each list holds a sticky header with two children, then [`ROWS`] rows.
//!
//! - `scroll_viewport_updates`: viewport size changes of a scroll container.
//!   A store of equal metrics, or one that moved only the content, is not one.
//! - `scroll_content_extent_recomputes`: content index entries recomputed.
//!   Pure scrolling recomputes none.
//! - `scroll_clip_updates`: hit entries, clips with them, the scroll path
//!   rewrote. A list's sticky branch is three; its rows share the list's
//!   shift and the other list is not reached.
//! - Pure scrolling lays nothing out: measure, placement and shaping stay 0.
//! - A virtual list maps onto `virtual_rows_materialized_from_layout`,
//!   `entities_spawned` and `entities_despawned`.

#![cfg(test)]

use nana_ui_core::WorkCounters;
use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, OverflowSpec, PositionSpec};

use super::issue262::{Feed, Pass, ROW};
use super::reflow_oracle::{Builder, bundled_face_shaper, fixed, product_frame, resize_frame};
use crate::{
    AppContext, DocumentId, LayoutViewport, MutationQueue, NanaTextEngineShaper, ScrollOffset,
    StableNodeId,
};

/// Rows under each list's header.
const ROWS: usize = 50;
/// A sticky header and its two children: the branch a list's scroll moves.
const STICKY_BRANCH: usize = 3;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn scroller(width: LengthSpec, height: LengthSpec) -> LayoutStyle {
    LayoutStyle {
        overflow_x: OverflowSpec::Auto,
        overflow_y: OverflowSpec::Auto,
        direction: Some(FlexDirection::Column),
        flex_shrink: Some(0.0),
        width: Some(width),
        height: Some(height),
        ..LayoutStyle::default()
    }
}

struct Page {
    cx: AppContext,
    shaper: NanaTextEngineShaper,
    viewport: LayoutViewport,
    page: StableNodeId,
    list: StableNodeId,
    other: StableNodeId,
}

impl Page {
    fn new() -> Self {
        let (mut b, root) = Builder::new(document(), 1);
        let page = b.element(
            root,
            scroller(LengthSpec::Percent(100.0), LengthSpec::Percent(100.0)),
        );
        let mut list = || {
            let list = b.element(page, scroller(LengthSpec::Px(300.0), LengthSpec::Px(200.0)));
            let header = b.element(
                list,
                LayoutStyle {
                    position: PositionSpec::Sticky,
                    offset_top: Some(LengthSpec::Px(0.0)),
                    flex_shrink: Some(0.0),
                    ..fixed(300.0, 20.0)
                },
            );
            b.element(header, fixed(100.0, 10.0));
            b.element(header, fixed(100.0, 10.0));
            for _ in 0..ROWS {
                b.element(
                    list,
                    LayoutStyle {
                        flex_shrink: Some(0.0),
                        ..fixed(300.0, 24.0)
                    },
                );
            }
            list
        };
        let (list, other) = (list(), list());
        // Taller than the window, so the page scrolls too.
        b.element(
            page,
            LayoutStyle {
                flex_shrink: Some(0.0),
                ..fixed(300.0, 800.0)
            },
        );
        let mut cx = AppContext::new();
        cx.commit_mutations(b.queue).unwrap();
        let mut page = Self {
            cx,
            shaper: bundled_face_shaper(),
            viewport: LayoutViewport::new(800.0, 600.0),
            page,
            list,
            other,
        };
        page.frame();
        page.cx.rebuild_hit_test(document());
        page
    }

    /// A frame as `RuntimeDocument` runs a scroll-only one: lay out, then
    /// patch the hit index in place.
    fn frame(&mut self) -> WorkCounters {
        product_frame(&mut self.cx, document(), self.viewport, &mut self.shaper);
        for (scroller, delta) in self.cx.take_scroll_hit_updates() {
            self.cx.update_hit_test_scroll(document(), scroller, delta);
        }
        self.cx.last_work_counters()
    }

    fn scroll(&mut self, to: &[(StableNodeId, f32)]) -> WorkCounters {
        let mut q = MutationQueue::new();
        for &(id, y) in to {
            q.set_scroll_offset(id, ScrollOffset { x: 0.0, y });
        }
        self.cx.commit_mutations(q).unwrap();
        self.frame()
    }

    fn resize(&mut self, width: f32, height: f32) -> WorkCounters {
        self.viewport = LayoutViewport::new(width, height);
        resize_frame(&mut self.cx, document(), self.viewport, &mut self.shaper)
    }

    fn viewport_height(&self, id: StableNodeId) -> f32 {
        self.cx.world().scroll_metrics(id).unwrap().viewport_height
    }
}

/// `(scroll_viewport_updates, scroll_content_extent_recomputes,
/// scroll_clip_updates)`.
fn scroll_counters(work: &WorkCounters) -> (usize, usize, usize) {
    (
        work.scroll_viewport_updates,
        work.scroll_content_extent_recomputes,
        work.scroll_clip_updates,
    )
}

fn lays_nothing_out(work: &WorkCounters) -> bool {
    (
        work.layout_measure_nodes,
        work.layout_placement_nodes,
        work.text_shaped,
    ) == (0, 0, 0)
}

/// A scroll-only frame stores no viewport, recomputes no extent, and
/// rewrites the clips of the sticky branch it moves: three entries per list
/// scrolled, not the list's rows and not the other list's header.
#[test]
fn issue381_a_scroll_only_frame_rewrites_the_clips_of_the_branch_it_moves() {
    let mut page = Page::new();
    let (list, other, outer) = (page.list, page.other, page.page);
    let generation = page.cx.world().layout_generation();
    // One list: its sticky branch.
    let work = page.scroll(&[(list, 100.0)]);
    assert!(lays_nothing_out(&work), "{work:?}");
    assert_eq!(scroll_counters(&work), (0, 0, STICKY_BRANCH));
    // The other list: its own branch only.
    let work = page.scroll(&[(other, 40.0)]);
    assert!(lays_nothing_out(&work), "{work:?}");
    assert_eq!(scroll_counters(&work), (0, 0, STICKY_BRANCH));
    // Both: a branch each.
    let work = page.scroll(&[(list, 20.0), (other, 300.0)]);
    assert!(lays_nothing_out(&work), "{work:?}");
    assert_eq!(scroll_counters(&work), (0, 0, 2 * STICKY_BRANCH));
    // The page: no sticky box hangs from it, so it moves every entry under
    // it by one shift and rewrites none.
    let work = page.scroll(&[(outer, 120.0)]);
    assert_eq!(page.cx.world().scroll_offset(outer).unwrap().y, 120.0);
    assert!(lays_nothing_out(&work), "{work:?}");
    assert_eq!(scroll_counters(&work), (0, 0, 0));
    assert_eq!(page.cx.world().layout_generation(), generation);
}

/// A viewport resize counts the one scroll container whose viewport moved.
/// A store of equal metrics, and content that grows inside a viewport that
/// holds, store no viewport.
#[test]
fn issue381_a_viewport_resize_counts_the_viewports_it_moved() {
    let mut page = Page::new();
    let (list, other, outer) = (page.list, page.other, page.page);
    assert_eq!(page.viewport_height(outer), 600.0);
    // The page follows the window; the lists keep their fixed size.
    let work = page.resize(800.0, 500.0);
    assert_eq!(page.viewport_height(outer), 500.0);
    assert_eq!(page.viewport_height(list), 200.0);
    assert_eq!(page.viewport_height(other), 200.0);
    assert_eq!(work.scroll_viewport_updates, 1, "{work:?}");
    // Only the page's own entry: no box under it moved.
    assert_eq!(work.scroll_content_extent_recomputes, 1, "{work:?}");
    assert_eq!(work.scroll_clip_updates, 0, "{work:?}");
    // The same metrics stored again: no work, so it is read as the
    // counters it leaves pending.
    let before = page.cx.last_work_counters();
    let metrics = page.cx.world().scroll_metrics(list);
    let mut q = MutationQueue::new();
    q.set_scroll_metrics(list, metrics);
    page.cx.commit_mutations(q).unwrap();
    assert_eq!(
        scroll_counters(&page.cx.last_work_counters()),
        scroll_counters(&before)
    );
    // A row grows inside `list`: its extent moves, its viewport holds.
    let row = page.cx.world().node(list).unwrap().children[1];
    let mut q = MutationQueue::new();
    q.set_style(
        row,
        super::reflow_oracle::styled(LayoutStyle {
            flex_shrink: Some(0.0),
            ..fixed(300.0, 64.0)
        }),
    );
    page.cx.commit_mutations(q).unwrap();
    let before = page.cx.world().scroll_metrics(list).unwrap();
    let work = page.frame();
    let after = page.cx.world().scroll_metrics(list).unwrap();
    assert_eq!(after.content_height, before.content_height + 40.0);
    assert_eq!(work.scroll_viewport_updates, 0, "{work:?}");
    assert!(work.scroll_content_extent_recomputes > 0, "{work:?}");
}

/// A far scroll of a virtual list over `rows` rows, from a window at the
/// top: the scroll-only frame, the list's sync, the frame that lays the new
/// window out, and the rows the old and the new window held.
fn far_scroll(rows: usize) -> (WorkCounters, Pass, WorkCounters, usize, usize) {
    let mut feed = Feed::new(rows);
    feed.cx.rebuild_hit_test(document());
    let unmounted = feed.mounted().len();
    // Far past the window: none of its rows stays mounted.
    feed.cx
        .scroll_to(
            feed.scroll,
            ScrollOffset {
                x: 0.0,
                y: 1_000.0 * ROW,
            },
        )
        .unwrap();
    feed.frame();
    for (scroller, delta) in feed.cx.take_scroll_hit_updates() {
        feed.cx.update_hit_test_scroll(document(), scroller, delta);
    }
    let scrolled = feed.cx.last_work_counters();
    let pass = feed.sync();
    let placed = feed.frame();
    (scrolled, pass, placed, unmounted, feed.mounted().len())
}

/// A virtual list scroll: the scroll-only frame stores and recomputes
/// nothing and rewrites no clip; the list's sync materializes the new
/// window and the frame after it spawns it and despawns the old one; that
/// frame recomputes extents for the window it lays out, the same over a
/// collection a hundred times larger.
#[test]
fn issue381_a_virtual_list_scroll_counts_the_window_it_mounts() {
    let (scrolled, pass, placed, unmounted, mounted) = far_scroll(10_000);
    assert!(lays_nothing_out(&scrolled), "{scrolled:?}");
    assert_eq!(scroll_counters(&scrolled), (0, 0, 0));
    // Materialized rows are the new window's; each row is the same number
    // of entities, spawned for the new window and despawned for the old.
    assert_eq!(pass.materialized, mounted, "{pass:?}");
    assert_eq!(placed.virtual_rows_materialized_from_layout, mounted);
    let per_row = placed.entities_spawned / mounted;
    assert!(per_row > 0, "{placed:?}");
    assert_eq!(placed.entities_spawned, per_row * mounted, "{placed:?}");
    assert_eq!(placed.entities_despawned, per_row * unmounted, "{placed:?}");
    assert_eq!(placed.scroll_viewport_updates, 0, "{placed:?}");
    // Each publish batch recomputes an entry at most once, and the world
    // holds only the window.
    let recomputes = placed.scroll_content_extent_recomputes;
    assert!(recomputes > 0, "{placed:?}");
    assert!(
        recomputes <= placed.layout_delta_commits * placed.entities_total,
        "{placed:?}"
    );
    let (_, _, larger, ..) = far_scroll(1_000_000);
    assert_eq!(scroll_counters(&larger), scroll_counters(&placed));
}
