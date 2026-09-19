//! Ordered bounding hierarchy. A missing bound means conservative inclusion;
//! it never means an invisible primitive or permission to reorder operations.
use super::*;

#[derive(Debug, Clone)]
pub(super) struct VisibilityIndex {
    plan: Arc<FramePlan>,
    bounds: Vec<Option<SceneRect>>,
    leaf: usize,
    shifts: Vec<[f32; 2]>,
    descendants: HashMap<StableNodeId, Vec<std::ops::Range<usize>>>,
    nodes: HashMap<StableNodeId, NodeSlots>,
    /// Set once the scroll fast path has moved bounds by an offset instead of
    /// re-deriving them, for [`UiScene::audit_retained_projection`].
    #[cfg(debug_assertions)]
    translated: bool,
}

/// Where one node's primitives sit in the operation list.
///
/// Almost every node owns exactly one, and a `Vec` for each cost an allocation
/// per node when the index is built and another per node when it is updated.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NodeSlots {
    One(usize, PrimitiveId),
    Many(Vec<(usize, PrimitiveId)>),
}

impl NodeSlots {
    fn push(&mut self, offset: usize, id: PrimitiveId) {
        match self {
            Self::One(first_offset, first_id) => {
                *self = Self::Many(vec![(*first_offset, *first_id), (offset, id)]);
            }
            Self::Many(slots) => slots.push((offset, id)),
        }
    }

    fn extend_into(&self, out: &mut Vec<(usize, PrimitiveId)>) {
        match self {
            Self::One(offset, id) => out.push((*offset, *id)),
            Self::Many(slots) => out.extend_from_slice(slots),
        }
    }
}

fn union(a: Option<SceneRect>, b: Option<SceneRect>) -> Option<SceneRect> {
    a.zip(b).map(|(a, b)| {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        SceneRect {
            x,
            y,
            width: (a.x + a.width).max(b.x + b.width) - x,
            height: (a.y + a.height).max(b.y + b.height) - y,
        }
    })
}
fn intersects(a: SceneRect, b: SceneRect) -> bool {
    a.x <= b.x + b.width && a.x + a.width >= b.x && a.y <= b.y + b.height && a.y + a.height >= b.y
}
pub(super) fn transform(bounds: SceneRect, affine: AffineTransform) -> Option<SceneRect> {
    if affine.is_projective() {
        return None;
    }
    let [a, b, c, d, e, f] = affine.0;
    let mut x = f32::INFINITY;
    let mut y = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for (px, py) in [
        (bounds.x, bounds.y),
        (bounds.x + bounds.width, bounds.y),
        (bounds.x, bounds.y + bounds.height),
        (bounds.x + bounds.width, bounds.y + bounds.height),
    ] {
        let tx = a * px + c * py + e;
        let ty = b * px + d * py + f;
        x = x.min(tx);
        y = y.min(ty);
        right = right.max(tx);
        bottom = bottom.max(ty);
    }
    [x, y, right, bottom]
        .iter()
        .all(|v| v.is_finite())
        .then_some(SceneRect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
}
fn primitive_bounds(scene: &UiScene, id: PrimitiveId) -> Option<SceneRect> {
    let primitive = scene.primitive(id)?;
    let draw_transform = scene.draw_transform(primitive)?;
    // Destination groups need their complete source, including pixels that a
    // blur or blend can move into the viewport. Do not cull their source leaves.
    if !scene.opacity_groups(id.node).is_empty() {
        return None;
    }
    let mut bounds = primitive.bounds;
    if let ScenePrimitiveKind::QuadBatch {
        bounds: rectangles, ..
    }
    | ScenePrimitiveKind::QuadColorBatch {
        bounds: rectangles, ..
    }
    | ScenePrimitiveKind::IconBatch {
        bounds: rectangles, ..
    } = &primitive.kind
    {
        bounds = batch_bounds(rectangles)?;
    }
    match &primitive.kind {
        ScenePrimitiveKind::Quad {
            shadow, surface, ..
        }
        | ScenePrimitiveKind::QuadBatch {
            shadow, surface, ..
        } => {
            if surface.filter.is_some() || surface.backdrop_filter.is_some() {
                return None;
            }
            let outset = shadow
                .iter()
                .chain(surface.extra_shadows.iter())
                .filter(|shadow| !shadow.inset)
                .map(|shadow| {
                    shadow.offset_x.abs().max(shadow.offset_y.abs())
                        + shadow.blur_radius * 3.0
                        + shadow.spread_radius.max(0.0)
                })
                .fold(surface.outline_width.max(0.0) + 2.0, f32::max);
            bounds.x -= outset;
            bounds.y -= outset;
            bounds.width += 2.0 * outset;
            bounds.height += 2.0 * outset;
        }
        ScenePrimitiveKind::Custom { .. }
        | ScenePrimitiveKind::Icon { .. }
        | ScenePrimitiveKind::QuadColorBatch { .. }
        | ScenePrimitiveKind::IconBatch { .. } => {}
        ScenePrimitiveKind::Stroke {
            points,
            width,
            widths,
            ..
        } => {
            bounds = stroke_bounds(points, *width, widths)?;
        }
        // Ink may exceed a nominal text/stroke box. An explicitly applied
        // self clip still gives a sound bound without shaping offscreen text.
        // Ancestor clips are deliberately excluded: scrolling their contents
        // must not translate a stationary ancestor clip in this index.
        _ => return self_clip_bounds(scene, primitive, draw_transform),
    }
    transform(bounds, draw_transform)
}

fn stroke_bounds(points: &[[f32; 2]], width: f32, widths: &[f32]) -> Option<SceneRect> {
    let first = points.first()?;
    let mut bounds = SceneRect {
        x: first[0],
        y: first[1],
        width: 0.0,
        height: 0.0,
    };
    for &[x, y] in points {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        bounds = union(
            Some(bounds),
            Some(SceneRect {
                x,
                y,
                width: 0.0,
                height: 0.0,
            }),
        )?;
    }
    // A full width contains endpoint discs and diagonal Square cap corners.
    // Also cover the shader's minimum local AA width; the painter expands
    // its viewport query for the remaining physical-pixel coverage.
    let mut outset = width.max(1e-5);
    for &width in std::iter::once(&width).chain(widths.iter()) {
        if !width.is_finite() {
            return None;
        }
        outset = outset.max(width);
    }
    bounds.x -= outset;
    bounds.y -= outset;
    bounds.width += outset * 2.0;
    bounds.height += outset * 2.0;
    Some(bounds)
}

fn self_clip_bounds(
    scene: &UiScene,
    primitive: &ScenePrimitive,
    draw_transform: AffineTransform,
) -> Option<SceneRect> {
    let node = scene.nodes.get(&primitive.node)?;
    let layout = node.layout;
    let (x, y, width, height) = node.source_style.layout.overflow_clip_box(
        layout.x,
        layout.y,
        layout.width,
        layout.height,
    )?;
    let bounds = SceneRect {
        x,
        y,
        width,
        height,
    };
    let &(_, base_transform, parent_clip_count) = scene.projections.get(&primitive.node)?;
    // Some editor popups deliberately use only parent_clips. Check the
    // retained primitive's self-clip suffix rather than inferring from style.
    if !primitive
        .clips
        .iter()
        .skip(parent_clip_count)
        .any(|clip| clip.bounds == bounds && clip.transform == base_transform)
    {
        return None;
    }
    transform(bounds, draw_transform)
}

fn batch_bounds(rectangles: &[SceneRect]) -> Option<SceneRect> {
    let mut result = *rectangles.first()?;
    for &rect in rectangles {
        if ![rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|value| value.is_finite())
            || rect.width < 0.0
            || rect.height < 0.0
        {
            return None;
        }
        result = union(Some(result), Some(rect))?;
    }
    Some(result)
}

impl VisibilityIndex {
    pub(super) fn new(scene: &UiScene, plan: Arc<FramePlan>) -> Self {
        let leaf = plan.operations.len().max(1).next_power_of_two();
        let mut index = Self {
            plan,
            bounds: vec![None; leaf * 2],
            leaf,
            shifts: vec![[0.0, 0.0]; leaf * 2],
            descendants: HashMap::new(),
            nodes: HashMap::new(),
            #[cfg(debug_assertions)]
            translated: false,
        };
        for (offset, operation) in index.plan.operations.iter().enumerate() {
            let id = match operation {
                RenderOperation::Draw(id) | RenderOperation::InvokeCustom(id) => *id,
                RenderOperation::PrepareExternal(_) => continue,
            };
            index.bounds[leaf + offset] = primitive_bounds(scene, id);
            index
                .nodes
                .entry(id.node)
                .and_modify(|slots| slots.push(offset, id))
                .or_insert(NodeSlots::One(offset, id));
            let scroll_parent = |id| {
                scene.nodes.get(&id).and_then(|node| {
                    (node.source_style.layout.position != nana_ui_core::PositionSpec::Fixed)
                        .then_some(node.parent)
                        .flatten()
                })
            };
            let mut parent = scroll_parent(id.node);
            while let Some(id) = parent {
                let ranges = index.descendants.entry(id).or_default();
                if let Some(last) = ranges.last_mut().filter(|range| range.end == offset) {
                    last.end += 1;
                } else {
                    ranges.push(offset..offset + 1);
                }
                parent = scroll_parent(id);
            }
        }
        for offset in (1..leaf).rev() {
            index.bounds[offset] = union(index.bounds[offset * 2], index.bounds[offset * 2 + 1]);
        }
        index
    }
    /// Re-derive every bound from the current scene, keeping the structure the
    /// frame plan gave this index.
    ///
    /// The leaves this writes are exactly the ones [`Self::new`] would, so the
    /// result is a rebuild minus rebuilding `nodes` and `descendants` — which
    /// depend only on `plan.operations` and on each node's parent and
    /// `position`, and a delta that moved any of those has already dropped the
    /// frame plan and this index with it. That is worth skipping: those two
    /// maps cost an ancestor walk and a `Vec` per operation.
    pub(super) fn refresh_bounds(&mut self, scene: &UiScene) {
        // Every internal bound below is re-derived from the leaves, so nothing
        // is left owing a scroll offset.
        self.shifts.fill([0.0, 0.0]);
        // Re-derived, so no longer a translation of an older build.
        #[cfg(debug_assertions)]
        {
            self.translated = false;
        }
        let plan = Arc::clone(&self.plan);
        for (offset, operation) in plan.operations.iter().enumerate() {
            let id = match operation {
                RenderOperation::Draw(id) | RenderOperation::InvokeCustom(id) => *id,
                RenderOperation::PrepareExternal(_) => continue,
            };
            self.bounds[self.leaf + offset] = primitive_bounds(scene, id);
        }
        for offset in (1..self.leaf).rev() {
            self.bounds[offset] = union(self.bounds[offset * 2], self.bounds[offset * 2 + 1]);
        }
    }

    fn shift(&mut self, at: usize, offset: [f32; 2]) {
        if let Some(bounds) = self.bounds[at].as_mut() {
            bounds.x += offset[0];
            bounds.y += offset[1];
        }
        self.shifts[at][0] += offset[0];
        self.shifts[at][1] += offset[1];
    }
    fn push(&mut self, at: usize) {
        let offset = std::mem::replace(&mut self.shifts[at], [0.0, 0.0]);
        self.shift(at * 2, offset);
        self.shift(at * 2 + 1, offset);
    }
    fn translate_range(
        &mut self,
        at: usize,
        start: usize,
        end: usize,
        range: &std::ops::Range<usize>,
        offset: [f32; 2],
    ) {
        if start >= range.end || end <= range.start {
            return;
        }
        if start >= range.start && end <= range.end {
            self.shift(at, offset);
            return;
        }
        self.push(at);
        let mid = (start + end) / 2;
        self.translate_range(at * 2, start, mid, range, offset);
        self.translate_range(at * 2 + 1, mid, end, range, offset);
        self.bounds[at] = union(self.bounds[at * 2], self.bounds[at * 2 + 1]);
    }
    pub(super) fn translate_subtree(&mut self, root: StableNodeId, offset: [f32; 2]) {
        if let Some(ranges) = self.descendants.get(&root).cloned() {
            #[cfg(debug_assertions)]
            {
                self.translated = true;
            }
            for range in ranges {
                self.translate_range(1, 0, self.leaf, &range, offset);
            }
        }
    }
    fn set_bound(
        &mut self,
        at: usize,
        start: usize,
        end: usize,
        index: usize,
        bounds: Option<SceneRect>,
    ) {
        if end - start == 1 {
            self.bounds[at] = bounds;
            self.shifts[at] = [0.0, 0.0];
            return;
        }
        self.push(at);
        let mid = (start + end) / 2;
        if index < mid {
            self.set_bound(at * 2, start, mid, index, bounds);
        } else {
            self.set_bound(at * 2 + 1, mid, end, index, bounds);
        }
        self.bounds[at] = union(self.bounds[at * 2], self.bounds[at * 2 + 1]);
    }
    pub(super) fn update(&mut self, scene: &UiScene, changed: &[StableNodeId]) {
        // Reused across the whole update: `set_bound` needs `&mut self`, so the
        // slots have to be copied out of the map first, and doing that into a
        // fresh `Vec` per node was an allocation per changed node per frame.
        let mut slots = Vec::new();
        for node in changed {
            slots.clear();
            let Some(found) = self.nodes.get(node) else {
                continue;
            };
            found.extend_into(&mut slots);
            for &(offset, id) in &slots {
                self.set_bound(1, 0, self.leaf, offset, primitive_bounds(scene, id));
            }
        }
    }
    /// Whether the scroll fast path has shifted this index since it was last
    /// derived from the scene.
    #[cfg(debug_assertions)]
    pub(super) fn translated(&self) -> bool {
        self.translated
    }

    /// The first thing a query reads that this index and the ground truth
    /// disagree on, or `None` when they are bitwise equal.
    ///
    /// Comparison is bitwise for the retained-projection audit: bounds are
    /// produced by the same arithmetic on both sides, so an index that is
    /// still valid compares equal exactly, and anything looser would not catch
    /// a bound that drifted by an ulp and then culled a primitive a pixel
    /// early.
    ///
    /// It reports *what* differs rather than *that* something does, because
    /// the two failures this catches call for opposite fixes and read the same
    /// in decimal: a bound off by an ulp is an arithmetic path that wants
    /// sharing, a bound left over from before a mutation is a delta that
    /// skipped a refresh. So name the half, the operation and node that own
    /// the slot, and both values with their bits.
    #[cfg(debug_assertions)]
    pub(super) fn mismatch(&self, other: &Self) -> Option<String> {
        fn rect_bits(rect: &Option<SceneRect>) -> Option<[u32; 4]> {
            rect.map(|rect| {
                [
                    rect.x.to_bits(),
                    rect.y.to_bits(),
                    rect.width.to_bits(),
                    rect.height.to_bits(),
                ]
            })
        }
        if self.leaf != other.leaf {
            let (retained, fresh) = (self.leaf, other.leaf);
            return Some(format!("leaf {retained} vs fresh {fresh}"));
        }
        if self.plan.operations != other.plan.operations {
            let (retained, fresh) = (self.plan.operations.len(), other.plan.operations.len());
            return Some(format!(
                "plan.operations: {retained} retained vs {fresh} fresh"
            ));
        }
        if self.bounds.len() != other.bounds.len() {
            let (retained, fresh) = (self.bounds.len(), other.bounds.len());
            return Some(format!("bounds length {retained} vs fresh {fresh}"));
        }
        for (at, (retained, fresh)) in self.bounds.iter().zip(&other.bounds).enumerate() {
            let (retained_bits, fresh_bits) = (rect_bits(retained), rect_bits(fresh));
            if retained_bits == fresh_bits {
                continue;
            }
            let origin = self.slot_origin(at);
            return Some(format!(
                "bounds[{at}] {origin}: retained {retained:?} {retained_bits:?} vs fresh {fresh:?} {fresh_bits:?}"
            ));
        }
        if self.shifts.len() != other.shifts.len() {
            let (retained, fresh) = (self.shifts.len(), other.shifts.len());
            return Some(format!("shifts length {retained} vs fresh {fresh}"));
        }
        for (at, (retained, fresh)) in self.shifts.iter().zip(&other.shifts).enumerate() {
            if (*retained).map(f32::to_bits) == (*fresh).map(f32::to_bits) {
                continue;
            }
            let origin = self.slot_origin(at);
            return Some(format!(
                "shifts[{at}] {origin}: retained {retained:?} vs fresh {fresh:?}"
            ));
        }
        for (node, retained) in &self.nodes {
            let fresh = other.nodes.get(node);
            if fresh != Some(retained) {
                return Some(format!(
                    "nodes[{node:?}]: retained {retained:?} vs fresh {fresh:?}"
                ));
            }
        }
        if let Some(node) = other
            .nodes
            .keys()
            .find(|node| !self.nodes.contains_key(node))
        {
            let fresh = other.nodes.get(node);
            return Some(format!(
                "nodes[{node:?}]: only in the fresh build, {fresh:?}"
            ));
        }
        for (node, retained) in &self.descendants {
            let fresh = other.descendants.get(node);
            if fresh != Some(retained) {
                return Some(format!(
                    "descendants[{node:?}]: retained {retained:?} vs fresh {fresh:?}"
                ));
            }
        }
        if let Some(node) = other
            .descendants
            .keys()
            .find(|node| !self.descendants.contains_key(node))
        {
            let fresh = other.descendants.get(node);
            return Some(format!(
                "descendants[{node:?}]: only in the fresh build, {fresh:?}"
            ));
        }
        None
    }

    /// Which operation, primitive and node a bounds or shift slot belongs to.
    #[cfg(debug_assertions)]
    fn slot_origin(&self, at: usize) -> String {
        let Some(offset) = at.checked_sub(self.leaf) else {
            let (left, right) = (at * 2, at * 2 + 1);
            return format!("(internal, union of slots {left} and {right})");
        };
        let Some(operation) = self.plan.operations.get(offset) else {
            let total = self.plan.operations.len();
            return format!("(padding leaf, past all {total} operations)");
        };
        match operation {
            RenderOperation::Draw(id) | RenderOperation::InvokeCustom(id) => {
                let node = id.node;
                format!("(operation {offset} {operation:?} on node {node:?})")
            }
            RenderOperation::PrepareExternal(_) => {
                format!("(operation {offset} {operation:?})")
            }
        }
    }

    fn visit(
        &self,
        at: usize,
        start: usize,
        end: usize,
        viewport: SceneRect,
        out: &mut Vec<RenderOperation>,
    ) {
        if start >= self.plan.operations.len()
            || self.bounds[at].is_some_and(|bounds| !intersects(bounds, viewport))
        {
            return;
        }
        if end - start == 1 {
            out.push(self.plan.operations[start].clone());
            return;
        }
        let viewport = SceneRect {
            x: viewport.x - self.shifts[at][0],
            y: viewport.y - self.shifts[at][1],
            ..viewport
        };
        let mid = (start + end) / 2;
        self.visit(at * 2, start, mid, viewport, out);
        self.visit(at * 2 + 1, mid, end, viewport, out);
    }
    pub(super) fn visible(&self, viewport: SceneRect) -> Vec<RenderOperation> {
        if ![viewport.x, viewport.y, viewport.width, viewport.height]
            .iter()
            .all(|value| value.is_finite())
        {
            return self.plan.operations.to_vec();
        }
        let mut out = Vec::new();
        self.visit(1, 0, self.leaf, viewport, &mut out);
        out
    }
}

impl UiScene {
    pub fn visible_operations(
        &self,
        viewport: SceneRect,
    ) -> Result<Vec<RenderOperation>, GraphError> {
        let plan = self.frame_plan()?;
        Ok(self
            .visibility
            .get_or_init(|| VisibilityIndex::new(self, plan))
            .visible(viewport))
    }
}
