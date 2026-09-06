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
    nodes: HashMap<StableNodeId, Vec<(usize, PrimitiveId)>>,
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
    let primitive = scene.draw_primitive(id)?;
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
        _ => return self_clip_bounds(scene, &primitive),
    }
    transform(bounds, primitive.transform)
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

fn self_clip_bounds(scene: &UiScene, primitive: &SceneDraw<'_>) -> Option<SceneRect> {
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
        .primitive
        .clips
        .iter()
        .skip(parent_clip_count)
        .any(|clip| clip.bounds == bounds && clip.transform == base_transform)
    {
        return None;
    }
    transform(bounds, primitive.transform)
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
        };
        for (offset, operation) in index.plan.operations.iter().enumerate() {
            let id = match operation {
                RenderOperation::Draw(id) | RenderOperation::InvokeCustom(id) => *id,
                RenderOperation::PrepareExternal(_) => continue,
            };
            index.bounds[leaf + offset] = primitive_bounds(scene, id);
            index.nodes.entry(id.node).or_default().push((offset, id));
            let mut parent = scene.nodes.get(&id.node).and_then(|node| node.parent);
            while let Some(id) = parent {
                let ranges = index.descendants.entry(id).or_default();
                if let Some(last) = ranges.last_mut().filter(|range| range.end == offset) {
                    last.end += 1;
                } else {
                    ranges.push(offset..offset + 1);
                }
                parent = scene.nodes.get(&id).and_then(|node| node.parent);
            }
        }
        for offset in (1..leaf).rev() {
            index.bounds[offset] = union(index.bounds[offset * 2], index.bounds[offset * 2 + 1]);
        }
        index
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
        for node in changed {
            let Some(slots) = self.nodes.get(node).cloned() else {
                continue;
            };
            for (offset, id) in slots {
                self.set_bound(1, 0, self.leaf, offset, primitive_bounds(scene, id));
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
