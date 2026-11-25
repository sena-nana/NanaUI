//! Incremental window [`InputRegionSnapshot`] builder (#159).
//!
//! Derives coalesced `Client` / `Drag` / `Resize` / `PassThrough` rectangles
//! directly from [`UiWorld`] and its [`HitIndex`] — never from GPU framebuffer
//! alpha readback or [`WindowVisualShape`] — and unifies `#49`
//! `MousePassthroughMode::Forward` with native `WM_NCHITTEST` / X11 `ShapeInput`
//! / Wayland `wl_region` through a single authoritative snapshot.

use std::collections::HashSet;

use nana_diagnostics::metric;
use nana_ui_core::{
    HitTestBehavior, InputRegionRect, InputRegionSnapshot, NativeHitTestClass, WindowResizeEdge,
};

use super::*;

/// Maximum coalesced rectangles emitted into [`InputRegionSnapshot`] so huge
/// UI trees never flood OS region APIs (`SetWindowRgn`/`XShapeCombineRectangles`).
pub(crate) const MAX_INPUT_REGION_RECTS: usize = 128;

const MAX_OCCLUDERS_FOR_EXACT_CARVE: usize = 256;
const MIN_REGION_EXTENT: f32 = 0.25;

/// Background treatment outside explicit interactive/chrome UI regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputRegionBackground {
    /// Normal interactive desktop window (`MousePassthroughMode::Off`):
    /// areas not covered by `Drag`, `Resize`, or explicit `PassThrough` classify as `Client`.
    OpaqueClient,
    /// Click-through outside interactive content (`MousePassthroughMode::Forward`):
    /// uncovered/transparent/non-interactive areas classify as `PassThrough`.
    TransparentForward,
    /// Whole-window passthrough (`MousePassthroughMode::Passthrough`):
    /// everything inside the window classifies as `PassThrough`.
    FullPassThrough,
}

impl InputRegionBackground {
    fn tag(self) -> u8 {
        match self {
            Self::OpaqueClient => 0,
            Self::TransparentForward => 1,
            Self::FullPassThrough => 2,
        }
    }
}

/// Window parameters that affect [`InputRegionSnapshot`] beyond `UiWorld`'s hit index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowInteractionRegionConfig {
    pub logical_size: (f32, f32),
    pub scale_factor: f32,
    pub background: InputRegionBackground,
    pub resize_border_thickness: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegionCacheKey {
    document: DocumentId,
    region_epoch: u64,
    logical_size_bits: [u32; 2],
    scale_milli: u32,
    background: u8,
    resize_thickness_bits: u32,
}

impl RegionCacheKey {
    fn new(document: DocumentId, region_epoch: u64, config: WindowInteractionRegionConfig) -> Self {
        let scale_milli = if config.scale_factor.is_finite() && config.scale_factor > 0.0 {
            (config.scale_factor * 1000.0).round() as u32
        } else {
            1000
        };
        Self {
            document,
            region_epoch,
            logical_size_bits: [
                config.logical_size.0.to_bits(),
                config.logical_size.1.to_bits(),
            ],
            scale_milli,
            background: config.background.tag(),
            resize_thickness_bits: config.resize_border_thickness.to_bits(),
        }
    }
}

/// Incremental cached builder for one window's [`InputRegionSnapshot`].
#[derive(Debug, Default)]
pub struct WindowInteractionRegionBuilder {
    revision: u64,
    last_key: Option<RegionCacheKey>,
    last_snapshot: InputRegionSnapshot,
}

impl WindowInteractionRegionBuilder {
    pub fn snapshot(&self) -> &InputRegionSnapshot {
        &self.last_snapshot
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Update the cached [`InputRegionSnapshot`] if `world` hit-test state or
    /// `config` changed since the last build.
    ///
    /// Returns `None` on idle frames and pointer moves (zero allocations).
    pub fn update(
        &mut self,
        world: &UiWorld,
        document: DocumentId,
        config: WindowInteractionRegionConfig,
    ) -> Option<&InputRegionSnapshot> {
        let motion_active = world.has_compositor_transform_overlay_for(document);
        let key = RegionCacheKey::new(document, world.input_region_epoch(), config);
        if !motion_active && self.last_key == Some(key) {
            return None;
        }
        self.revision = self.revision.wrapping_add(1);
        let snapshot = build_snapshot(self.revision, world, document, config, motion_active);
        metric!(
            nana_diagnostics::framework::window::INTERACTION_INPUT_REGION_REVISION,
            self.revision
        );
        metric!(
            nana_diagnostics::framework::window::INTERACTION_INPUT_REGION_REBUILDS,
            1u64
        );
        metric!(
            nana_diagnostics::framework::window::INTERACTION_INPUT_REGION_SOURCE_RECTS,
            snapshot.source_rect_count as u64
        );
        metric!(
            nana_diagnostics::framework::window::INTERACTION_INPUT_REGION_MERGED_RECTS,
            snapshot.rects.len() as u64
        );
        self.last_key = if motion_active { None } else { Some(key) };
        self.last_snapshot = snapshot;
        Some(&self.last_snapshot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RegionBox {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl RegionBox {
    fn from_size(w: f32, h: f32) -> Option<Self> {
        (w.is_finite() && h.is_finite() && w > MIN_REGION_EXTENT && h > MIN_REGION_EXTENT)
            .then_some(Self {
                x0: 0.0,
                y0: 0.0,
                x1: w,
                y1: h,
            })
    }

    fn from_layout_box(b: LayoutBox) -> Option<Self> {
        let x1 = b.x + b.width;
        let y1 = b.y + b.height;
        (b.x.is_finite()
            && b.y.is_finite()
            && x1.is_finite()
            && y1.is_finite()
            && b.width > MIN_REGION_EXTENT
            && b.height > MIN_REGION_EXTENT)
            .then_some(Self {
                x0: b.x,
                y0: b.y,
                x1,
                y1,
            })
    }

    fn intersect(self, other: Self) -> Option<Self> {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        let x1 = self.x1.min(other.x1);
        let y1 = self.y1.min(other.y1);
        ((x1 - x0) > MIN_REGION_EXTENT && (y1 - y0) > MIN_REGION_EXTENT).then_some(Self {
            x0,
            y0,
            x1,
            y1,
        })
    }

    fn intersects(self, other: Self) -> bool {
        self.x0 < other.x1 && self.x1 > other.x0 && self.y0 < other.y1 && self.y1 > other.y0
    }

    fn contains_box(self, other: Self) -> bool {
        self.x0 <= other.x0 && self.y0 <= other.y0 && self.x1 >= other.x1 && self.y1 >= other.y1
    }

    fn width(self) -> f32 {
        self.x1 - self.x0
    }

    fn height(self) -> f32 {
        self.y1 - self.y0
    }

    fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    fn to_region_rect(self, class: NativeHitTestClass) -> InputRegionRect {
        InputRegionRect {
            x: self.x0,
            y: self.y0,
            width: self.width(),
            height: self.height(),
            class,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    rect: RegionBox,
    class: NativeHitTestClass,
    is_full_root: bool,
}

fn build_snapshot(
    revision: u64,
    world: &UiWorld,
    document: DocumentId,
    config: WindowInteractionRegionConfig,
    compositor_motion: bool,
) -> InputRegionSnapshot {
    let scale_factor = if config.scale_factor.is_finite() && config.scale_factor > 0.0 {
        config.scale_factor
    } else {
        1.0
    };
    let Some(viewport) = RegionBox::from_size(config.logical_size.0, config.logical_size.1) else {
        return InputRegionSnapshot {
            revision,
            rects: Vec::new(),
            logical_size: config.logical_size,
            scale_factor,
            capped: false,
            source_rect_count: 0,
        };
    };
    if config.background == InputRegionBackground::FullPassThrough {
        return InputRegionSnapshot {
            revision,
            rects: Vec::new(),
            logical_size: config.logical_size,
            scale_factor,
            capped: false,
            source_rect_count: 0,
        };
    }

    let mut ordered_nodes = Vec::new();
    let index = world.hit_index.hit_test_index.get(&document);
    if let Some(index) = index {
        collect_front_to_back_nodes(index, &mut ordered_nodes);
    }

    let mut front_controls = Vec::new();
    let mut drag_and_back = Vec::new();
    let mut source_rect_count = 0usize;

    // Open triggered menus (Popover / ActionMenu / HoverCard) sit above the page.
    for &id in &world.overlays_a11y.triggered_overlays {
        if world.document_of(id) != Some(document) || world.motion_blocks_input(id) {
            continue;
        }
        let record = world.record(id);
        let computed = record.resolved.0.as_ref();
        if !computed.box_visible || !computed.visible || !computed.pointer_events.hittable() {
            continue;
        }
        if let Some(surface) = crate::popover::hanging_surface(world, id) {
            let projected = match world.hanging_surface_transform(id) {
                Some((xf, persp)) => project_transformed_box(surface, xf, persp),
                None => Some(surface),
            };
            if let Some(bounds) = projected.and_then(RegionBox::from_layout_box)
                && let Some(clipped) = bounds.intersect(viewport)
            {
                source_rect_count += 1;
                front_controls.push((clipped, NativeHitTestClass::Client));
            }
        }
    }

    if let Some(index) = index {
        for id in ordered_nodes {
            if world.motion_blocks_input(id) {
                continue;
            }
            let Some(candidate) =
                classify_node(world, document, index, id, compositor_motion, viewport)
            else {
                continue;
            };
            source_rect_count += 1;
            if candidate.is_full_root || candidate.class == NativeHitTestClass::Drag {
                drag_and_back.push((candidate.rect, candidate.class));
            } else {
                front_controls.push((candidate.rect, candidate.class));
            }
        }
    }

    let mut pipeline: Vec<(RegionBox, NativeHitTestClass)> =
        Vec::with_capacity(front_controls.len() + drag_and_back.len() + 10);
    pipeline.extend(front_controls);
    if config.resize_border_thickness.is_finite() && config.resize_border_thickness > 0.0 {
        append_resize_border_strips(viewport, config.resize_border_thickness, &mut pipeline);
        source_rect_count += 8;
    }
    pipeline.extend(drag_and_back);
    if config.background == InputRegionBackground::OpaqueClient {
        pipeline.push((viewport, NativeHitTestClass::Client));
    }

    let mut rects = resolve_front_to_back(&pipeline);
    coalesce_same_class(&mut rects);
    let capped = enforce_rect_cap(&mut rects, MAX_INPUT_REGION_RECTS);

    InputRegionSnapshot {
        revision,
        rects,
        logical_size: config.logical_size,
        scale_factor,
        capped,
        source_rect_count,
    }
}

fn collect_front_to_back_nodes(index: &HitIndex, out: &mut Vec<StableNodeId>) {
    enum Step {
        Enter(StableNodeId),
        Emit(StableNodeId),
    }

    let mut seen = HashSet::new();
    let mut stack: Vec<Step> = index
        .roots
        .iter()
        .flatten()
        .copied()
        .map(Step::Enter)
        .collect();

    while let Some(step) = stack.pop() {
        match step {
            Step::Emit(id) => out.push(id),
            Step::Enter(id) => {
                if !seen.insert(id) {
                    continue;
                }
                let Some(node) = index.entries.get(&id) else {
                    continue;
                };
                // LIFO stack: push negative-z children ascending (popped last),
                // then `Emit(id)`, then non-negative-z children ascending (popped first).
                for child_id in node.children.iter().flatten().copied() {
                    if index
                        .entries
                        .get(&child_id)
                        .is_some_and(|c| c.entry.z_index < 0)
                    {
                        stack.push(Step::Enter(child_id));
                    }
                }
                stack.push(Step::Emit(id));
                for child_id in node.children.iter().flatten().copied() {
                    if index
                        .entries
                        .get(&child_id)
                        .is_some_and(|c| c.entry.z_index >= 0)
                    {
                        stack.push(Step::Enter(child_id));
                    }
                }
            }
        }
    }
}

fn classify_node(
    world: &UiWorld,
    document: DocumentId,
    index: &HitIndex,
    id: StableNodeId,
    compositor_motion: bool,
    viewport: RegionBox,
) -> Option<Candidate> {
    let record = world.record(id);
    let computed = record.resolved.0.as_ref();
    if !computed.box_visible || !computed.visible {
        return None;
    }
    let indexed = index.entries.get(&id)?;
    let behavior = world.hit_test_behavior(id);
    let hittable_pe = computed.pointer_events.hittable();
    let class = match behavior {
        HitTestBehavior::PassThrough => NativeHitTestClass::PassThrough,
        HitTestBehavior::Client => hittable_pe.then_some(NativeHitTestClass::Client)?,
        HitTestBehavior::Drag => hittable_pe.then_some(NativeHitTestClass::Drag)?,
        HitTestBehavior::Resize(edge) => hittable_pe.then_some(NativeHitTestClass::Resize(edge))?,
        HitTestBehavior::Auto => {
            if !hittable_pe {
                return None;
            }
            let has_listeners = world
                .nodes
                .event_listeners(id)
                .is_some_and(|listeners| !listeners.is_empty());
            if indexed.entry.hittable || has_listeners {
                NativeHitTestClass::Client
            } else {
                return None;
            }
        }
    };

    let rect =
        projected_entry_window_rect(world, document, index, id, compositor_motion, viewport)?;
    let is_full_root = behavior == HitTestBehavior::Auto
        && class == NativeHitTestClass::Client
        && indexed.parent.is_none()
        && rect.x0 <= 0.5
        && rect.y0 <= 0.5
        && rect.x1 >= viewport.x1 - 0.5
        && rect.y1 >= viewport.y1 - 0.5;
    Some(Candidate {
        rect,
        class,
        is_full_root,
    })
}

fn projected_entry_window_rect(
    world: &UiWorld,
    _document: DocumentId,
    index: &HitIndex,
    id: StableNodeId,
    compositor_motion: bool,
    viewport: RegionBox,
) -> Option<RegionBox> {
    let indexed = index.entries.get(&id)?;
    let entry = &indexed.entry;
    if entry.layout.width <= MIN_REGION_EXTENT || entry.layout.height <= MIN_REGION_EXTENT {
        return None;
    }

    let mut rect = if compositor_motion {
        let (xf, persp) = world.layout_projection_transform(id)?;
        let origin = world.layout_origin(id).unwrap_or_default();
        let own = project_transformed_box(
            LayoutBox {
                x: entry.layout.x - origin[0],
                y: entry.layout.y - origin[1],
                ..entry.layout
            },
            xf,
            persp,
        )?;
        let mut r = RegionBox::from_layout_box(own)?;
        for (clip_box, clip_xf) in &entry.self_clips {
            if let Some(clip) = project_transformed_box(*clip_box, *clip_xf, [0.0, 0.0])
                && let Some(clip_r) = RegionBox::from_layout_box(clip)
            {
                let moved = project_box_through_window(index, id, clip_r)?;
                r = r.intersect(moved)?;
            }
        }
        r
    } else {
        let own = project_transformed_box(entry.layout, entry.transform, entry.persp)?;
        let mut r = RegionBox::from_layout_box(own)?;
        for (clip_box, clip_xf) in &entry.self_clips {
            let clip = project_transformed_box(*clip_box, *clip_xf, [0.0, 0.0])?;
            let clip_r = RegionBox::from_layout_box(clip)?;
            r = r.intersect(clip_r)?;
        }
        project_box_through_window(index, id, r)?
    };

    // Intersect ancestor `child_clips` (for clips separated by `local_children`).
    let mut ancestor = indexed.parent;
    while let Some(anc_id) = ancestor {
        let Some(anc) = index.entries.get(&anc_id) else {
            break;
        };
        if anc.entry.escaped {
            break;
        }
        for (clip_box, clip_xf) in &anc.entry.child_clips {
            let clip = project_transformed_box(*clip_box, *clip_xf, [0.0, 0.0])?;
            let clip_r = RegionBox::from_layout_box(clip)?;
            let clip_win = project_box_through_window(index, anc_id, clip_r)?;
            rect = rect.intersect(clip_win)?;
        }
        ancestor = anc.parent;
    }

    rect.intersect(viewport)
}

fn project_box_through_window(
    index: &HitIndex,
    id: StableNodeId,
    local: RegionBox,
) -> Option<RegionBox> {
    let corners = [
        index.own_to_window(id, local.x0, local.y0),
        index.own_to_window(id, local.x1, local.y0),
        index.own_to_window(id, local.x0, local.y1),
        index.own_to_window(id, local.x1, local.y1),
    ];
    let mut x0 = f32::INFINITY;
    let mut y0 = f32::INFINITY;
    let mut x1 = f32::NEG_INFINITY;
    let mut y1 = f32::NEG_INFINITY;
    for (cx, cy) in corners {
        if !cx.is_finite() || !cy.is_finite() {
            return None;
        }
        x0 = x0.min(cx);
        y0 = y0.min(cy);
        x1 = x1.max(cx);
        y1 = y1.max(cy);
    }
    ((x1 - x0) > MIN_REGION_EXTENT && (y1 - y0) > MIN_REGION_EXTENT).then_some(RegionBox {
        x0,
        y0,
        x1,
        y1,
    })
}

fn append_resize_border_strips(
    viewport: RegionBox,
    thickness: f32,
    out: &mut Vec<(RegionBox, NativeHitTestClass)>,
) {
    let t_x = thickness.min(viewport.width() * 0.5);
    let t_y = thickness.min(viewport.height() * 0.5);
    if t_x <= MIN_REGION_EXTENT || t_y <= MIN_REGION_EXTENT {
        return;
    }
    let w = viewport.x1;
    let h = viewport.y1;
    let strips = [
        (
            RegionBox {
                x0: 0.0,
                y0: 0.0,
                x1: t_x,
                y1: t_y,
            },
            WindowResizeEdge::NorthWest,
        ),
        (
            RegionBox {
                x0: w - t_x,
                y0: 0.0,
                x1: w,
                y1: t_y,
            },
            WindowResizeEdge::NorthEast,
        ),
        (
            RegionBox {
                x0: 0.0,
                y0: h - t_y,
                x1: t_x,
                y1: h,
            },
            WindowResizeEdge::SouthWest,
        ),
        (
            RegionBox {
                x0: w - t_x,
                y0: h - t_y,
                x1: w,
                y1: h,
            },
            WindowResizeEdge::SouthEast,
        ),
        (
            RegionBox {
                x0: t_x,
                y0: 0.0,
                x1: w - t_x,
                y1: t_y,
            },
            WindowResizeEdge::North,
        ),
        (
            RegionBox {
                x0: t_x,
                y0: h - t_y,
                x1: w - t_x,
                y1: h,
            },
            WindowResizeEdge::South,
        ),
        (
            RegionBox {
                x0: 0.0,
                y0: t_y,
                x1: t_x,
                y1: h - t_y,
            },
            WindowResizeEdge::West,
        ),
        (
            RegionBox {
                x0: w - t_x,
                y0: t_y,
                x1: w,
                y1: h - t_y,
            },
            WindowResizeEdge::East,
        ),
    ];
    for (rect, edge) in strips {
        if rect.width() > MIN_REGION_EXTENT && rect.height() > MIN_REGION_EXTENT {
            out.push((rect, NativeHitTestClass::Resize(edge)));
        }
    }
}

fn subtract_rect(piece: RegionBox, hole: RegionBox, out: &mut Vec<RegionBox>) {
    if !piece.intersects(hole) {
        out.push(piece);
        return;
    }
    if hole.contains_box(piece) {
        return;
    }
    let ix0 = piece.x0.max(hole.x0);
    let iy0 = piece.y0.max(hole.y0);
    let ix1 = piece.x1.min(hole.x1);
    let iy1 = piece.y1.min(hole.y1);

    // Top strip
    if iy0 - piece.y0 > MIN_REGION_EXTENT {
        out.push(RegionBox {
            x0: piece.x0,
            y0: piece.y0,
            x1: piece.x1,
            y1: iy0,
        });
    }
    // Bottom strip
    if piece.y1 - iy1 > MIN_REGION_EXTENT {
        out.push(RegionBox {
            x0: piece.x0,
            y0: iy1,
            x1: piece.x1,
            y1: piece.y1,
        });
    }
    // Left strip between iy0..iy1
    if ix0 - piece.x0 > MIN_REGION_EXTENT && iy1 - iy0 > MIN_REGION_EXTENT {
        out.push(RegionBox {
            x0: piece.x0,
            y0: iy0,
            x1: ix0,
            y1: iy1,
        });
    }
    // Right strip between iy0..iy1
    if piece.x1 - ix1 > MIN_REGION_EXTENT && iy1 - iy0 > MIN_REGION_EXTENT {
        out.push(RegionBox {
            x0: ix1,
            y0: iy0,
            x1: piece.x1,
            y1: iy1,
        });
    }
}

fn resolve_front_to_back(pipeline: &[(RegionBox, NativeHitTestClass)]) -> Vec<InputRegionRect> {
    let mut result = Vec::new();
    let mut occluded: Vec<RegionBox> = Vec::new();
    let mut fragments = Vec::new();
    let mut next_fragments = Vec::new();

    for &(rect, class) in pipeline {
        if class == NativeHitTestClass::PassThrough {
            if occluded.len() < MAX_OCCLUDERS_FOR_EXACT_CARVE {
                occluded.push(rect);
            }
            continue;
        }
        // Fast path: if an existing occluder completely covers `rect`, skip it cleanly.
        if occluded.iter().any(|occ| occ.contains_box(rect)) {
            continue;
        }
        fragments.clear();
        fragments.push(rect);
        for &hole in &occluded {
            if fragments.is_empty() {
                break;
            }
            next_fragments.clear();
            for &piece in &fragments {
                subtract_rect(piece, hole, &mut next_fragments);
            }
            std::mem::swap(&mut fragments, &mut next_fragments);
            // Bound pathological checkerboard fragmentation from a single layer.
            if fragments.len() > MAX_INPUT_REGION_RECTS {
                let merged = fragments
                    .iter()
                    .copied()
                    .reduce(RegionBox::union)
                    .expect("non-empty");
                fragments.clear();
                fragments.push(merged);
                break;
            }
        }
        for &frag in &fragments {
            result.push(frag.to_region_rect(class));
        }
        if occluded.len() < MAX_OCCLUDERS_FOR_EXACT_CARVE {
            // Skip inserting tiny fragments or already-covered occluders.
            if !fragments.is_empty() {
                occluded.push(rect);
            }
        }
    }
    result
}

fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.05
}

fn can_merge_same_class(a: InputRegionRect, b: InputRegionRect) -> Option<InputRegionRect> {
    if a.class != b.class {
        return None;
    }
    // Same horizontal span, vertically touching/overlapping.
    if approx_eq(a.x, b.x)
        && approx_eq(a.width, b.width)
        && a.y <= b.bottom() + 0.05
        && b.y <= a.bottom() + 0.05
    {
        let y0 = a.y.min(b.y);
        let y1 = a.bottom().max(b.bottom());
        return Some(InputRegionRect {
            x: a.x,
            y: y0,
            width: a.width,
            height: y1 - y0,
            class: a.class,
        });
    }
    // Same vertical span, horizontally touching/overlapping.
    if approx_eq(a.y, b.y)
        && approx_eq(a.height, b.height)
        && a.x <= b.right() + 0.05
        && b.x <= a.right() + 0.05
    {
        let x0 = a.x.min(b.x);
        let x1 = a.right().max(b.right());
        return Some(InputRegionRect {
            x: x0,
            y: a.y,
            width: x1 - x0,
            height: a.height,
            class: a.class,
        });
    }
    None
}

fn coalesce_same_class(rects: &mut Vec<InputRegionRect>) {
    loop {
        let mut changed = false;
        let mut i = 0;
        while i < rects.len() {
            let mut j = i + 1;
            while j < rects.len() {
                if let Some(merged) = can_merge_same_class(rects[i], rects[j]) {
                    rects[i] = merged;
                    rects.swap_remove(j);
                    changed = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !changed {
            break;
        }
    }
}

fn enforce_rect_cap(rects: &mut Vec<InputRegionRect>, cap: usize) -> bool {
    if rects.len() <= cap {
        return false;
    }
    // Collapse excess same-class `Client` rects into a single bounding hull first,
    // keeping explicit non-client `Drag`/`Resize` slices exact when possible.
    let mut non_client = Vec::new();
    let mut client_union: Option<RegionBox> = None;
    for &rect in rects.iter() {
        if rect.class == NativeHitTestClass::Client {
            let r = RegionBox {
                x0: rect.x,
                y0: rect.y,
                x1: rect.right(),
                y1: rect.bottom(),
            };
            client_union = Some(match client_union {
                Some(acc) => acc.union(r),
                None => r,
            });
        } else {
            non_client.push(rect);
        }
    }
    rects.clear();
    rects.extend(non_client.into_iter().take(cap.saturating_sub(1)));
    if let Some(u) = client_union {
        rects.push(u.to_region_rect(NativeHitTestClass::Client));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppContext, AppTitleBar, Entity, LayoutViewport, MutationQueue, NodeStyle, OverflowSpec,
        PaintTransform, PointerEventsSpec, VisibilitySpec,
    };

    fn doc() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn nid(n: u64) -> StableNodeId {
        StableNodeId::new(n).unwrap()
    }

    fn rect_box(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
        LayoutBox {
            x,
            y,
            width,
            height,
        }
    }

    fn add_box(
        q: &mut MutationQueue,
        doc: DocumentId,
        id: StableNodeId,
        parent: Option<StableNodeId>,
        bounds: LayoutBox,
        hittable: bool,
        style: NodeStyle,
    ) {
        q.create(id, doc, NodeKind::Element { tag: "div".into() });
        if let Some(parent) = parent {
            q.insert(parent, id, None);
        }
        q.write_layout(id, bounds);
        q.set_interaction(
            id,
            InteractionState {
                pointer_events: hittable,
                focusable: false,
            },
        );
        q.set_style(id, style);
    }

    fn sync_world(world: &mut UiWorld, document: DocumentId) {
        world
            .resolve_styles(&world.document_order(document))
            .unwrap();
        world.rebuild_hit_test(document);
    }

    fn forward_cfg(w: f32, h: f32, resize_thickness: f32) -> WindowInteractionRegionConfig {
        WindowInteractionRegionConfig {
            logical_size: (w, h),
            scale_factor: 1.0,
            background: InputRegionBackground::TransparentForward,
            resize_border_thickness: resize_thickness,
        }
    }

    #[test]
    fn priority_hole_carving_and_nested_client_pill() {
        let mut world = UiWorld::new();
        let d = doc();
        let (bar, hole, btn, pill) = (nid(1), nid(2), nid(3), nid(4));
        let mut q = MutationQueue::new();

        let mut bar_style = NodeStyle::default();
        Arc::make_mut(&mut bar_style.layout).hit_test = Some(HitTestBehavior::Drag);
        add_box(
            &mut q,
            d,
            bar,
            None,
            rect_box(0.0, 0.0, 400.0, 40.0),
            false,
            bar_style,
        );

        let mut hole_style = NodeStyle::default();
        Arc::make_mut(&mut hole_style.layout).hit_test = Some(HitTestBehavior::PassThrough);
        add_box(
            &mut q,
            d,
            hole,
            Some(bar),
            rect_box(100.0, 4.0, 100.0, 32.0),
            false,
            hole_style,
        );

        add_box(
            &mut q,
            d,
            btn,
            Some(bar),
            rect_box(350.0, 4.0, 40.0, 32.0),
            true,
            NodeStyle::default(),
        );

        let mut pill_style = NodeStyle::default();
        Arc::make_mut(&mut pill_style.layout).hit_test = Some(HitTestBehavior::Client);
        add_box(
            &mut q,
            d,
            pill,
            Some(hole),
            rect_box(130.0, 8.0, 40.0, 24.0),
            false,
            pill_style,
        );

        world.commit(q).unwrap();
        sync_world(&mut world, d);

        let mut builder = WindowInteractionRegionBuilder::default();
        let snap = builder
            .update(&world, d, forward_cfg(400.0, 300.0, 0.0))
            .expect("initial snapshot");

        assert_eq!(snap.classify(20.0, 20.0), NativeHitTestClass::Drag);
        assert_eq!(snap.classify(110.0, 20.0), NativeHitTestClass::PassThrough);
        assert_eq!(snap.classify(150.0, 20.0), NativeHitTestClass::Client);
        assert_eq!(snap.classify(370.0, 20.0), NativeHitTestClass::Client);
        assert_eq!(snap.classify(200.0, 200.0), NativeHitTestClass::PassThrough);
    }

    #[test]
    fn resize_edges_yield_to_client_controls_and_beat_drag() {
        let mut world = UiWorld::new();
        let d = doc();
        let (bar, close_btn) = (nid(1), nid(2));
        let mut q = MutationQueue::new();

        let mut bar_style = NodeStyle::default();
        Arc::make_mut(&mut bar_style.layout).hit_test = Some(HitTestBehavior::Drag);
        add_box(
            &mut q,
            d,
            bar,
            None,
            rect_box(0.0, 0.0, 400.0, 38.0),
            false,
            bar_style,
        );

        add_box(
            &mut q,
            d,
            close_btn,
            Some(bar),
            rect_box(376.0, 0.0, 24.0, 24.0),
            true,
            NodeStyle::default(),
        );

        world.commit(q).unwrap();
        sync_world(&mut world, d);

        let mut builder = WindowInteractionRegionBuilder::default();
        let snap = builder
            .update(&world, d, forward_cfg(400.0, 300.0, 6.0))
            .unwrap();

        // Top-right close button covers top-right corner: Client wins over Resize(NorthEast).
        assert_eq!(snap.classify(398.0, 2.0), NativeHitTestClass::Client);
        // Top-left corner is inside resize ring over Drag bar: Resize(NorthWest) wins over Drag.
        assert_eq!(
            snap.classify(2.0, 2.0),
            NativeHitTestClass::Resize(WindowResizeEdge::NorthWest)
        );
        // Top middle edge (outside close button): Resize(North).
        assert_eq!(
            snap.classify(200.0, 2.0),
            NativeHitTestClass::Resize(WindowResizeEdge::North)
        );
        // Inside Drag bar below resize border: Drag.
        assert_eq!(snap.classify(200.0, 20.0), NativeHitTestClass::Drag);
    }

    #[test]
    fn static_frames_and_pointer_moves_do_not_rebuild_snapshot() {
        let mut world = UiWorld::new();
        let d = doc();
        let btn = nid(1);
        let mut q = MutationQueue::new();
        add_box(
            &mut q,
            d,
            btn,
            None,
            rect_box(10.0, 10.0, 80.0, 30.0),
            true,
            NodeStyle::default(),
        );
        world.commit(q).unwrap();
        sync_world(&mut world, d);

        let mut builder = WindowInteractionRegionBuilder::default();
        let cfg = forward_cfg(300.0, 200.0, 0.0);
        assert!(builder.update(&world, d, cfg).is_some());
        let rev1 = builder.revision();

        // Pointer hit test and idle update must NOT rebuild.
        let _ = world.hit_test(d, 20.0, 20.0);
        assert!(builder.update(&world, d, cfg).is_none());
        assert_eq!(builder.revision(), rev1);

        // Mutating HitTestBehavior bumps epoch and rebuilds cleanly.
        let mut q2 = MutationQueue::new();
        q2.set_hit_test_behavior(btn, Some(HitTestBehavior::Drag));
        world.commit(q2).unwrap();
        let snap2 = builder
            .update(&world, d, cfg)
            .expect("intent change rebuilds");
        assert_eq!(snap2.classify(20.0, 20.0), NativeHitTestClass::Drag);
        assert!(snap2.revision > rev1);
    }

    #[test]
    fn rect_merging_and_bounded_cap() {
        let mut world = UiWorld::new();
        let d = doc();
        let mut q = MutationQueue::new();

        // Two adjacent same-row Client boxes coalesce into one.
        add_box(
            &mut q,
            d,
            nid(1),
            None,
            rect_box(0.0, 0.0, 50.0, 20.0),
            true,
            NodeStyle::default(),
        );
        add_box(
            &mut q,
            d,
            nid(2),
            None,
            rect_box(50.0, 0.0, 50.0, 20.0),
            true,
            NodeStyle::default(),
        );
        world.commit(q).unwrap();
        sync_world(&mut world, d);

        let mut builder = WindowInteractionRegionBuilder::default();
        let snap = builder
            .update(&world, d, forward_cfg(500.0, 500.0, 0.0))
            .unwrap();
        assert_eq!(snap.rects.len(), 1);
        assert!(!snap.capped);

        // Add a non-adjacent checkerboard of > MAX_INPUT_REGION_RECTS boxes -> capped cleanly.
        let mut q2 = MutationQueue::new();
        let mut next_id = 10u64;
        for row in 0..20 {
            for col in 0..16 {
                add_box(
                    &mut q2,
                    d,
                    nid(next_id),
                    None,
                    rect_box(
                        col as f32 * 20.0 + 1.0,
                        30.0 + row as f32 * 20.0 + 1.0,
                        10.0,
                        10.0,
                    ),
                    true,
                    NodeStyle::default(),
                );
                next_id += 1;
            }
        }
        world.commit(q2).unwrap();
        sync_world(&mut world, d);

        let capped_snap = builder
            .update(&world, d, forward_cfg(500.0, 500.0, 0.0))
            .unwrap();
        assert!(capped_snap.capped);
        assert!(capped_snap.rects.len() <= MAX_INPUT_REGION_RECTS);
    }

    #[test]
    fn overflow_clip_transform_pointer_events_none_and_invisible_handle() {
        let mut world = UiWorld::new();
        let d = doc();
        let mut q = MutationQueue::new();

        let (clipper, overflow_child, transformed, pe_none, hidden_drag) =
            (nid(1), nid(2), nid(3), nid(4), nid(5));

        let mut clip_style = NodeStyle::default();
        Arc::make_mut(&mut clip_style.layout).overflow_x = OverflowSpec::Hidden;
        Arc::make_mut(&mut clip_style.layout).overflow_y = OverflowSpec::Hidden;
        add_box(
            &mut q,
            d,
            clipper,
            None,
            rect_box(10.0, 10.0, 100.0, 100.0),
            false,
            clip_style,
        );
        add_box(
            &mut q,
            d,
            overflow_child,
            Some(clipper),
            rect_box(50.0, 10.0, 200.0, 40.0),
            true,
            NodeStyle::default(),
        );

        let mut xf_style = NodeStyle::default();
        Arc::make_mut(&mut xf_style.layout).transforms.transform = Some(PaintTransform {
            e: 80.0,
            ..PaintTransform::IDENTITY
        });
        Arc::make_mut(&mut xf_style.layout).hit_test = Some(HitTestBehavior::Drag);
        add_box(
            &mut q,
            d,
            transformed,
            None,
            rect_box(0.0, 150.0, 60.0, 30.0),
            false,
            xf_style,
        );

        let mut pe_style = NodeStyle::default();
        Arc::make_mut(&mut pe_style.layout).pointer_events = Some(PointerEventsSpec::None);
        Arc::make_mut(&mut pe_style.layout).hit_test = Some(HitTestBehavior::Drag);
        add_box(
            &mut q,
            d,
            pe_none,
            None,
            rect_box(200.0, 150.0, 60.0, 30.0),
            false,
            pe_style,
        );

        let mut hid_style = NodeStyle::default();
        Arc::make_mut(&mut hid_style.layout).paint.visibility = Some(VisibilitySpec::Hidden);
        Arc::make_mut(&mut hid_style.layout).hit_test = Some(HitTestBehavior::Drag);
        add_box(
            &mut q,
            d,
            hidden_drag,
            None,
            rect_box(280.0, 150.0, 60.0, 30.0),
            false,
            hid_style,
        );

        world.commit(q).unwrap();
        sync_world(&mut world, d);

        let mut builder = WindowInteractionRegionBuilder::default();
        let snap = builder
            .update(&world, d, forward_cfg(400.0, 300.0, 0.0))
            .unwrap();

        // Inside clipped region -> Client; beyond parent x=110 -> PassThrough.
        assert_eq!(snap.classify(80.0, 30.0), NativeHitTestClass::Client);
        assert_eq!(snap.classify(150.0, 30.0), NativeHitTestClass::PassThrough);
        // Translated drag handle: logical (20, 165) is PassThrough, translated (100, 165) is Drag.
        assert_eq!(snap.classify(20.0, 165.0), NativeHitTestClass::PassThrough);
        assert_eq!(snap.classify(100.0, 165.0), NativeHitTestClass::Drag);
        // pointer-events: none drag handle -> PassThrough.
        assert_eq!(snap.classify(220.0, 165.0), NativeHitTestClass::PassThrough);
        // visibility: hidden drag handle -> PassThrough.
        assert_eq!(snap.classify(300.0, 165.0), NativeHitTestClass::PassThrough);
    }

    #[test]
    fn app_title_bar_projects_drag_background_and_client_controls() {
        let mut cx = AppContext::new();
        let d = doc();
        let bar: Entity<AppTitleBar> = cx
            .create_component(d, AppTitleBar::new("Title").native_controls(false))
            .unwrap();
        cx.assemble_app_title_bar(bar).unwrap();
        cx.layout_document(d, LayoutViewport::new(400.0, 200.0))
            .unwrap();
        cx.world_mut().rebuild_hit_test(d);

        let mut builder = WindowInteractionRegionBuilder::default();
        let snap = builder
            .update(cx.world(), d, forward_cfg(400.0, 200.0, 0.0))
            .unwrap();
        // Title bar empty background on the left -> Drag.
        assert_eq!(snap.classify(100.0, 16.0), NativeHitTestClass::Drag);
        // Title bar window control buttons on the right -> Client.
        assert_eq!(snap.classify(385.0, 16.0), NativeHitTestClass::Client);

        // Disabling drag switches background to Client.
        cx.update_component(bar, |b, _| b.drag_enabled = false)
            .unwrap();
        cx.layout_document(d, LayoutViewport::new(400.0, 200.0))
            .unwrap();
        cx.world_mut().rebuild_hit_test(d);
        let snap2 = builder
            .update(cx.world(), d, forward_cfg(400.0, 200.0, 0.0))
            .unwrap();
        assert_eq!(snap2.classify(100.0, 16.0), NativeHitTestClass::Client);
    }
}
