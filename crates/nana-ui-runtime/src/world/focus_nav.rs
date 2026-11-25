//! 2D spatial focus index and deterministic geometric fallback scoring.

use std::collections::HashMap;

use super::*;

const CELL_SIZE: f32 = 128.0;
const MAX_CELLS_PER_ENTRY: i64 = 16;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SpatialFocusEntry {
    pub(crate) id: StableNodeId,
    pub(crate) rect: LayoutBox,
    pub(crate) doc_order: u32,
    pub(crate) clipped_by_scroll: Option<StableNodeId>,
}

#[derive(Debug)]
pub(crate) struct SpatialFocusIndex {
    pub(crate) epoch: u64,
    pub(crate) closing_surfaces: usize,
    pub(crate) modal_root: Option<StableNodeId>,
    entries: Vec<SpatialFocusEntry>,
    cells: HashMap<(i32, i32), Vec<u32>, BuildIdHasher>,
    large_entries: Vec<u32>,
    min_cx: i32,
    max_cx: i32,
    min_cy: i32,
    max_cy: i32,
    visit_marks: Vec<u32>,
    visit_gen: u32,
}

impl SpatialFocusIndex {
    pub(crate) fn first_matching<F>(&self, mut accept: F) -> (Option<StableNodeId>, u64)
    where
        F: FnMut(&SpatialFocusEntry) -> bool,
    {
        let mut scanned = 0u64;
        for entry in &self.entries {
            scanned += 1;
            if accept(entry) {
                return (Some(entry.id), scanned);
            }
        }
        (None, scanned)
    }

    /// Query the best candidate in `direction` from `origin_rect`.
    ///
    /// Sweeps spatial grid stripes outward from `origin_rect` and prunes
    /// stripes/bands whose admissible lower-bound score exceeds the best found
    /// candidate.
    pub(crate) fn query<F>(
        &mut self,
        origin_id: StableNodeId,
        origin_rect: LayoutBox,
        direction: crate::FocusDirection,
        mut accept: F,
    ) -> (Option<StableNodeId>, u64)
    where
        F: FnMut(&SpatialFocusEntry) -> bool,
    {
        if self.entries.is_empty() {
            return (None, 0);
        }
        self.visit_gen = self.visit_gen.wrapping_add(1);
        if self.visit_gen == 0 {
            self.visit_marks.fill(0);
            self.visit_gen = 1;
        }
        let visit_gen = self.visit_gen;
        let mut candidates_considered = 0u64;
        let mut best: Option<(DirectionalScoreKey, StableNodeId)> = None;

        let mut evaluate = |idx: u32,
                            best: &mut Option<(DirectionalScoreKey, StableNodeId)>,
                            considered: &mut u64| {
            let slot = idx as usize;
            if self.visit_marks[slot] == visit_gen {
                return;
            }
            self.visit_marks[slot] = visit_gen;
            let entry = &self.entries[slot];
            if entry.id == origin_id {
                return;
            }
            *considered += 1;
            let Some(score) = score_directional_candidate(origin_rect, entry.rect, direction)
            else {
                return;
            };
            if !accept(entry) {
                return;
            }
            let key = DirectionalScoreKey {
                score: score.score,
                primary_gap: score.primary_gap,
                cross_center_delta: score.cross_center_delta,
                doc_order: entry.doc_order,
                id: entry.id,
            };
            if best.as_ref().is_none_or(|(best_key, _)| key < *best_key) {
                *best = Some((key, entry.id));
            }
        };

        for idx in 0..self.large_entries.len() {
            let entry_idx = self.large_entries[idx];
            evaluate(entry_idx, &mut best, &mut candidates_considered);
        }

        let (ox0, oy0, ox1, oy1) = cell_bounds(origin_rect);
        let origin_cx0 = ox0.clamp(self.min_cx, self.max_cx);
        let origin_cx1 = ox1.clamp(self.min_cx, self.max_cx);
        let origin_cy0 = oy0.clamp(self.min_cy, self.max_cy);
        let origin_cy1 = oy1.clamp(self.min_cy, self.max_cy);

        let (p_start, p_end, c_start, c_end, p_step) = match direction {
            crate::FocusDirection::Right => (ox0, self.max_cx, origin_cy0, origin_cy1, 1),
            crate::FocusDirection::Left => (ox1, self.min_cx, origin_cy0, origin_cy1, -1),
            crate::FocusDirection::Down => (oy0, self.max_cy, origin_cx0, origin_cx1, 1),
            crate::FocusDirection::Up => (oy1, self.min_cy, origin_cx0, origin_cx1, -1),
        };
        let c_min = match direction {
            crate::FocusDirection::Left | crate::FocusDirection::Right => self.min_cy,
            crate::FocusDirection::Up | crate::FocusDirection::Down => self.min_cx,
        };
        let c_max = match direction {
            crate::FocusDirection::Left | crate::FocusDirection::Right => self.max_cy,
            crate::FocusDirection::Up | crate::FocusDirection::Down => self.max_cx,
        };

        let mut p = p_start;
        while (p_step > 0 && p <= p_end) || (p_step < 0 && p >= p_end) {
            let min_primary = min_primary_gap_for_stripe(origin_rect, direction, p);
            if let Some((best_key, _)) = &best
                && min_primary > best_key.score
            {
                break;
            }
            for c in c_start..=c_end {
                let key = match direction {
                    crate::FocusDirection::Left | crate::FocusDirection::Right => (p, c),
                    crate::FocusDirection::Up | crate::FocusDirection::Down => (c, p),
                };
                if let Some(bucket) = self.cells.get(&key) {
                    for i in 0..bucket.len() {
                        evaluate(bucket[i], &mut best, &mut candidates_considered);
                    }
                }
            }
            let mut offset = 1i32;
            loop {
                let low = c_start - offset;
                let high = c_end + offset;
                if low < c_min && high > c_max {
                    break;
                }
                let min_cross =
                    min_cross_gap_for_band(origin_rect, direction, c_start, c_end, offset);
                if let Some((best_key, _)) = &best
                    && min_primary + 2.5 * min_cross > best_key.score
                {
                    break;
                }
                if low >= c_min {
                    let key = match direction {
                        crate::FocusDirection::Left | crate::FocusDirection::Right => (p, low),
                        crate::FocusDirection::Up | crate::FocusDirection::Down => (low, p),
                    };
                    if let Some(bucket) = self.cells.get(&key) {
                        for i in 0..bucket.len() {
                            evaluate(bucket[i], &mut best, &mut candidates_considered);
                        }
                    }
                }
                if high <= c_max && high != low {
                    let key = match direction {
                        crate::FocusDirection::Left | crate::FocusDirection::Right => (p, high),
                        crate::FocusDirection::Up | crate::FocusDirection::Down => (high, p),
                    };
                    if let Some(bucket) = self.cells.get(&key) {
                        for i in 0..bucket.len() {
                            evaluate(bucket[i], &mut best, &mut candidates_considered);
                        }
                    }
                }
                offset += 1;
            }
            p += p_step;
        }

        (best.map(|(_, id)| id), candidates_considered)
    }
}

#[derive(Debug, Clone, Copy)]
struct DirectionalScore {
    score: f32,
    primary_gap: f32,
    cross_center_delta: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DirectionalScoreKey {
    score: f32,
    primary_gap: f32,
    cross_center_delta: f32,
    doc_order: u32,
    id: StableNodeId,
}

impl Eq for DirectionalScoreKey {}

impl Ord for DirectionalScoreKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| self.primary_gap.total_cmp(&other.primary_gap))
            .then_with(|| self.cross_center_delta.total_cmp(&other.cross_center_delta))
            .then_with(|| self.doc_order.cmp(&other.doc_order))
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialOrd for DirectionalScoreKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn score_directional_candidate(
    origin: LayoutBox,
    cand: LayoutBox,
    direction: crate::FocusDirection,
) -> Option<DirectionalScore> {
    let (o_p_start, o_p_end, o_c_start, o_c_end, c_p_start, c_p_end, c_c_start, c_c_end) =
        match direction {
            crate::FocusDirection::Right => (
                origin.x,
                origin.x + origin.width,
                origin.y,
                origin.y + origin.height,
                cand.x,
                cand.x + cand.width,
                cand.y,
                cand.y + cand.height,
            ),
            crate::FocusDirection::Left => (
                -(origin.x + origin.width),
                -origin.x,
                origin.y,
                origin.y + origin.height,
                -(cand.x + cand.width),
                -cand.x,
                cand.y,
                cand.y + cand.height,
            ),
            crate::FocusDirection::Down => (
                origin.y,
                origin.y + origin.height,
                origin.x,
                origin.x + origin.width,
                cand.y,
                cand.y + cand.height,
                cand.x,
                cand.x + cand.width,
            ),
            crate::FocusDirection::Up => (
                -(origin.y + origin.height),
                -origin.y,
                origin.x,
                origin.x + origin.width,
                -(cand.y + cand.height),
                -cand.y,
                cand.x,
                cand.x + cand.width,
            ),
        };

    let o_p_center = 0.5 * (o_p_start + o_p_end);
    let c_p_center = 0.5 * (c_p_start + c_p_end);
    let o_c_center = 0.5 * (o_c_start + o_c_end);
    let c_c_center = 0.5 * (c_c_start + c_c_end);

    if c_p_center <= o_p_center + 1e-3 || c_p_end <= o_p_end + 1e-3 {
        return None;
    }

    let overlap = (o_c_end.min(c_c_end) - o_c_start.max(c_c_start)).max(0.0);
    let has_overlap = overlap > 1e-3;
    if !has_overlap && c_p_start < o_p_end - 1e-3 {
        return None;
    }

    let primary_gap = (c_p_start - o_p_end).max(0.0);
    let cross_gap = if has_overlap {
        0.0
    } else {
        (c_c_start - o_c_end).max(o_c_start - c_c_end).max(0.0)
    };
    let cross_center_delta = (c_c_center - o_c_center).abs();
    let score = if has_overlap {
        primary_gap + 0.25 * cross_center_delta
    } else {
        primary_gap + 2.5 * cross_gap + 0.25 * cross_center_delta
    };
    Some(DirectionalScore {
        score,
        primary_gap,
        cross_center_delta,
    })
}

fn cell_coord(v: f32) -> i32 {
    (v / CELL_SIZE).floor() as i32
}

fn cell_bounds(rect: LayoutBox) -> (i32, i32, i32, i32) {
    let x0 = cell_coord(rect.x);
    let y0 = cell_coord(rect.y);
    let x1 = cell_coord((rect.x + rect.width - 1e-4).max(rect.x));
    let y1 = cell_coord((rect.y + rect.height - 1e-4).max(rect.y));
    (x0, y0, x1, y1)
}

fn min_primary_gap_for_stripe(
    origin: LayoutBox,
    direction: crate::FocusDirection,
    stripe: i32,
) -> f32 {
    let s_min = stripe as f32 * CELL_SIZE;
    let s_max = (stripe + 1) as f32 * CELL_SIZE;
    match direction {
        crate::FocusDirection::Right => (s_min - (origin.x + origin.width)).max(0.0),
        crate::FocusDirection::Left => (origin.x - s_max).max(0.0),
        crate::FocusDirection::Down => (s_min - (origin.y + origin.height)).max(0.0),
        crate::FocusDirection::Up => (origin.y - s_max).max(0.0),
    }
}

fn min_cross_gap_for_band(
    origin: LayoutBox,
    direction: crate::FocusDirection,
    c_start: i32,
    c_end: i32,
    offset: i32,
) -> f32 {
    let (o_c_start, o_c_end) = match direction {
        crate::FocusDirection::Left | crate::FocusDirection::Right => {
            (origin.y, origin.y + origin.height)
        }
        crate::FocusDirection::Up | crate::FocusDirection::Down => {
            (origin.x, origin.x + origin.width)
        }
    };
    let low_max = (c_start - offset + 1) as f32 * CELL_SIZE;
    let high_min = (c_end + offset) as f32 * CELL_SIZE;
    let gap_low = (o_c_start - low_max).max(0.0);
    let gap_high = (high_min - o_c_end).max(0.0);
    gap_low.min(gap_high)
}

#[derive(Clone, Copy)]
struct ScrollClipInfo {
    id: StableNodeId,
    rect: LayoutBox,
    scrolls_x: bool,
    scrolls_y: bool,
}

fn boxes_intersect(a: LayoutBox, b: LayoutBox) -> bool {
    a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y
}

fn intersect_rect(a: LayoutBox, b: LayoutBox) -> Option<LayoutBox> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > left && bottom > top).then_some(LayoutBox {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

impl UiWorld {
    /// Ensure the cached spatial focus index for `(document, modal_root)` is
    /// up to date for the current world epoch, then take it out for querying
    /// so callers can borrow `&UiWorld` / `&AppContext` during candidate
    /// filtering without `RefCell` borrows. Return it with
    /// [`Self::put_spatial_focus_index`].
    pub(crate) fn take_spatial_focus_index(
        &mut self,
        document: DocumentId,
        modal_root: Option<StableNodeId>,
    ) -> SpatialFocusIndex {
        let epoch = self.input.focus_nav_epoch;
        let closing_surfaces = self.motion.closing_surfaces_count;
        let valid = self
            .input
            .focus_nav_cache
            .get(&document)
            .is_some_and(|cached| {
                cached.epoch == epoch
                    && cached.closing_surfaces == closing_surfaces
                    && cached.modal_root == modal_root
            });
        if valid {
            return self
                .input
                .focus_nav_cache
                .remove(&document)
                .expect("validated cache entry exists");
        }
        let built = self.build_spatial_focus_index(document, modal_root);
        self.input.focus_counters.spatial_index_builds += 1;
        built
    }

    pub(crate) fn put_spatial_focus_index(
        &mut self,
        document: DocumentId,
        index: SpatialFocusIndex,
    ) {
        self.input.focus_nav_cache.insert(document, index);
    }

    fn build_spatial_focus_index(
        &self,
        document: DocumentId,
        modal_root: Option<StableNodeId>,
    ) -> SpatialFocusIndex {
        let epoch = self.input.focus_nav_epoch;
        let closing_surfaces = self.motion.closing_surfaces_count;
        let mut entries = Vec::new();
        let has_overlays = !self.overlays_a11y.overlay_host_nodes.is_empty()
            || !self.overlays_a11y.triggered_overlays.is_empty();

        let roots: Vec<StableNodeId> = match modal_root {
            Some(root) if self.is_mounted(root) && self.document_of(root) == Some(document) => {
                vec![root]
            }
            _ => self.document_roots(document),
        };

        enum Frame {
            Enter {
                id: StableNodeId,
                inherited_visible: bool,
                scroll_dx: f32,
                scroll_dy: f32,
                non_scroll_clip: Option<LayoutBox>,
                transformed: bool,
                scroll_depth: usize,
            },
            PopScroll,
        }

        let mut stack: Vec<Frame> = roots
            .into_iter()
            .rev()
            .map(|id| Frame::Enter {
                id,
                inherited_visible: true,
                scroll_dx: 0.0,
                scroll_dy: 0.0,
                non_scroll_clip: None,
                transformed: false,
                scroll_depth: 0,
            })
            .collect();
        let mut scroll_clips: Vec<ScrollClipInfo> = Vec::new();
        let mut doc_order = 0u32;

        while let Some(frame) = stack.pop() {
            let (
                id,
                inherited_visible,
                scroll_dx,
                scroll_dy,
                mut non_scroll_clip,
                mut transformed,
                scroll_depth,
            ) = match frame {
                Frame::PopScroll => {
                    scroll_clips.pop();
                    continue;
                }
                Frame::Enter {
                    id,
                    inherited_visible,
                    scroll_dx,
                    scroll_dy,
                    non_scroll_clip,
                    transformed,
                    scroll_depth,
                } => (
                    id,
                    inherited_visible,
                    scroll_dx,
                    scroll_dy,
                    non_scroll_clip,
                    transformed,
                    scroll_depth,
                ),
            };

            let Some(record) = self.nodes.get(id) else {
                continue;
            };
            if !self.presence_live(id)
                || self.mount_state(id) == Some(MountState::Parked)
                || record.style.layout.omits_box()
                || record.style.layout.inert
                || self.nodes.accessibility(id).disabled
                || !self.menu_branch_open(id)
                || self.motion_blocks_input(id)
            {
                continue;
            }
            if let Some(parent) = record.hierarchy.parent
                && self
                    .overlay_host(parent)
                    .is_some_and(|state| state.active != Some(id))
            {
                continue;
            }
            let visible = record
                .style
                .layout
                .paint
                .visibility
                .map_or(inherited_visible, |v| {
                    v != nana_ui_core::VisibilitySpec::Hidden
                });

            let layout = &record.style.layout;
            transformed |= layout.transforms.transform.is_some()
                || layout.transforms.transform_3d.is_some()
                || self.sampled_compositor_transform(id).is_some()
                || self.clip_position(id) == PositionSpec::Fixed
                || self.clip_escape(id).is_some();

            let rect = if transformed {
                self.viewport_layout_box(id)
            } else {
                let b = self.record_layout_box(id, record);
                Some(LayoutBox {
                    x: b.x - scroll_dx,
                    y: b.y - scroll_dy,
                    width: b.width,
                    height: b.height,
                })
            };

            let current_order = doc_order;
            doc_order = doc_order.saturating_add(1);

            if visible
                && let Some(rect) = rect
                && rect.width > 0.0
                && rect.height > 0.0
                && self.focus_policy(id).allows_directional()
                && (!has_overlays || self.is_overlay_reachable(id))
                && non_scroll_clip.is_none_or(|clip| boxes_intersect(rect, clip))
            {
                let mut clipped_out_permanently = false;
                let mut clipped_by_scroll = None;
                for clip in scroll_clips.iter().rev() {
                    let inter_x =
                        rect.x < clip.rect.x + clip.rect.width && rect.x + rect.width > clip.rect.x;
                    let inter_y = rect.y < clip.rect.y + clip.rect.height
                        && rect.y + rect.height > clip.rect.y;
                    if inter_x && inter_y {
                        continue;
                    }
                    let ok_x = inter_x || clip.scrolls_x;
                    let ok_y = inter_y || clip.scrolls_y;
                    if ok_x && ok_y {
                        if clipped_by_scroll.is_none() {
                            clipped_by_scroll = Some(clip.id);
                        }
                    } else {
                        clipped_out_permanently = true;
                        break;
                    }
                }
                if !clipped_out_permanently {
                    entries.push(SpatialFocusEntry {
                        id,
                        rect,
                        doc_order: current_order,
                        clipped_by_scroll,
                    });
                }
            }

            let is_scroller = self.is_scroll_container(id);
            let clips_overflow = layout.clips_overflow()
                || self
                    .nodes
                    .widget_visual(id)
                    .is_some_and(crate::WidgetVisualHandle::clips_descendants);

            let mut pushed_scroll = false;
            if let Some(rect) = rect {
                if is_scroller {
                    if let Some(clip) = non_scroll_clip
                        && !boxes_intersect(rect, clip)
                    {
                        continue;
                    }
                    let sx = layout.overflow_x.scrolls()
                        || self
                            .nodes
                            .widget_visual(id)
                            .is_some_and(crate::WidgetVisualHandle::is_scroll_container);
                    let sy = layout.overflow_y.scrolls()
                        || self
                            .nodes
                            .widget_visual(id)
                            .is_some_and(crate::WidgetVisualHandle::is_scroll_container);
                    scroll_clips.push(ScrollClipInfo {
                        id,
                        rect,
                        scrolls_x: sx,
                        scrolls_y: sy,
                    });
                    non_scroll_clip = None;
                    pushed_scroll = true;
                } else if clips_overflow {
                    non_scroll_clip = match non_scroll_clip {
                        Some(existing) => intersect_rect(existing, rect),
                        None => Some(rect),
                    };
                    if non_scroll_clip.is_none() {
                        continue;
                    }
                }
            }

            let child_scroll = self.nodes.scroll_offset(id);
            let next_scroll_dx = scroll_dx + child_scroll.x;
            let next_scroll_dy = scroll_dy + child_scroll.y;
            let next_scroll_depth = scroll_depth + usize::from(pushed_scroll);

            if pushed_scroll {
                stack.push(Frame::PopScroll);
            }
            for &child in record.hierarchy.children.iter().rev() {
                stack.push(Frame::Enter {
                    id: child,
                    inherited_visible: visible,
                    scroll_dx: next_scroll_dx,
                    scroll_dy: next_scroll_dy,
                    non_scroll_clip,
                    transformed,
                    scroll_depth: next_scroll_depth,
                });
            }
        }

        let mut cells: HashMap<(i32, i32), Vec<u32>, BuildIdHasher> = HashMap::default();
        let mut large_entries = Vec::new();
        let mut min_cx = i32::MAX;
        let mut max_cx = i32::MIN;
        let mut min_cy = i32::MAX;
        let mut max_cy = i32::MIN;

        for (i, entry) in entries.iter().enumerate() {
            let idx = i as u32;
            let (cx0, cy0, cx1, cy1) = cell_bounds(entry.rect);
            min_cx = min_cx.min(cx0);
            max_cx = max_cx.max(cx1);
            min_cy = min_cy.min(cy0);
            max_cy = max_cy.max(cy1);
            let span = i64::from(cx1 - cx0 + 1) * i64::from(cy1 - cy0 + 1);
            if span > MAX_CELLS_PER_ENTRY {
                large_entries.push(idx);
            } else {
                for cy in cy0..=cy1 {
                    for cx in cx0..=cx1 {
                        cells.entry((cx, cy)).or_default().push(idx);
                    }
                }
            }
        }
        if entries.is_empty() {
            min_cx = 0;
            max_cx = -1;
            min_cy = 0;
            max_cy = -1;
        }

        let visit_marks = vec![0; entries.len()];
        SpatialFocusIndex {
            epoch,
            closing_surfaces,
            modal_root,
            entries,
            cells,
            large_entries,
            min_cx,
            max_cx,
            min_cy,
            max_cy,
            visit_marks,
            visit_gen: 0,
        }
    }
}
