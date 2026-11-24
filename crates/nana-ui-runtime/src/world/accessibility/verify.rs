//! Checks of the retained projection against a cold one (tests and
//! `layout-verify`).

use super::*;

/// What the projections handed out, kept where a delta's narrowing is
/// checked: a node a delta leaves alone must still be what it was handed out
/// as. Only small worlds are kept, as the hit index's own check.
#[cfg(any(test, feature = "layout-verify"))]
#[derive(Default)]
pub(crate) struct ShippedAccessibility(
    std::sync::Mutex<hashbrown::HashMap<StableNodeId, AccessibilityNode>>,
);

#[cfg(any(test, feature = "layout-verify"))]
const SHIPPED_ACCESSIBILITY_LIMIT: usize = 4096;

#[cfg(any(test, feature = "layout-verify"))]
impl ShippedAccessibility {
    pub(super) fn record(&self, updated: &[AccessibilityNode], removed: &[StableNodeId]) {
        let mut shipped = self.0.lock().unwrap_or_else(|error| error.into_inner());
        for id in removed {
            shipped.remove(id);
        }
        if shipped.len() + updated.len() > SHIPPED_ACCESSIBILITY_LIMIT {
            shipped.clear();
            return;
        }
        for node in updated {
            shipped.insert(node.id, node.clone());
        }
    }
}

#[cfg(any(test, feature = "layout-verify"))]
impl UiWorld {
    /// `placed`, what [`Self::offset_placed`] makes of `entry` from its
    /// offset, puts its box where its layout box's turned corners stand in
    /// the viewport, up to the rounding of the page and viewport sums the
    /// offset leaves out: its parent's children's space taken to the
    /// viewport ([`Self::children_space_to_viewport`]), then its bounds' move and
    /// its transform, take the corners of its own box to the corners its
    /// whole chain of transforms takes its page box to.
    pub(super) fn check_offset_bounds(
        &self,
        entry: &AccessibilityNode,
        parent: StableNodeId,
        (bounds, transform): (LayoutBox, Option<nana_ui_core::PaintTransform>),
        memo: &mut ProjectionMemo,
    ) {
        let above = self
            .children_space_to_viewport(parent, memo)
            .expect("a placed box's parent space reaches the viewport");
        let mut own = transform.unwrap_or_default();
        own.e += bounds.x;
        own.f += bounds.y;
        let [a, b, c, d, e, f] = above;
        let to_viewport = nana_ui_core::PaintTransform { a, b, c, d, e, f }.then(own);
        let placed = |x: f32, y: f32| {
            [
                to_viewport.a * x + to_viewport.c * y + to_viewport.e,
                to_viewport.b * x + to_viewport.d * y + to_viewport.f,
            ]
        };
        self.accessibility_ancestor_transform(entry.id, memo)
            .expect("a placed box has a transform");
        let ([ca, cb, cc, cd, ce, cf], _) = memo.transforms[&entry.id].own;
        let page = self
            .component_layout_box(entry.id)
            .expect("a placed box has a box");
        let cold = |x: f32, y: f32| [ca * x + cc * y + ce, cb * x + cd * y + cf];
        let (w, h) = (bounds.width, bounds.height);
        let got = [
            placed(0.0, 0.0),
            placed(w, 0.0),
            placed(0.0, h),
            placed(w, h),
        ];
        let want = [
            cold(page.x, page.y),
            cold(page.x + page.width, page.y),
            cold(page.x, page.y + page.height),
            cold(page.x + page.width, page.y + page.height),
        ];
        let scale = 1.0
            + want
                .iter()
                .chain(&got)
                .flatten()
                .map(|value| value.abs())
                .fold(0.0, f32::max)
                * 4.0
            + page.x.abs()
            + page.y.abs();
        assert!(
            got.iter()
                .flatten()
                .zip(want.iter().flatten())
                .all(|(got, want)| (got - want).abs() <= 2e-5 * scale),
            "{:?}: corners placed from its offset {got:?} are not where its turned box stands, {want:?}",
            entry.id
        );
    }

    /// What [`Self::viewport_frame_in_parent`] makes of `id` puts the corners
    /// of its own box, through its parent's space's map to the viewport,
    /// where its chain of transforms takes its layout box's corners, up to
    /// rounding.
    pub(super) fn check_viewport_frame(
        &self,
        id: StableNodeId,
        parent: Option<StableNodeId>,
        bounds: LayoutBox,
        transform: nana_ui_core::PaintTransform,
        memo: &mut ProjectionMemo,
    ) {
        let [a, b, c, d, e, f] = self
            .parent_space_to_viewport(parent, memo)
            .expect("a framed box's parent space reaches the viewport");
        let mut own = transform;
        own.e += bounds.x;
        own.f += bounds.y;
        let to_viewport = nana_ui_core::PaintTransform { a, b, c, d, e, f }.then(own);
        let placed = |x: f32, y: f32| {
            [
                to_viewport.a * x + to_viewport.c * y + to_viewport.e,
                to_viewport.b * x + to_viewport.d * y + to_viewport.f,
            ]
        };
        self.accessibility_ancestor_transform(id, memo)
            .expect("a framed box has a transform");
        let ([ca, cb, cc, cd, ce, cf], _) = memo.transforms[&id].own;
        let page = self
            .component_layout_box(id)
            .expect("a framed box has a box");
        let cold = |x: f32, y: f32| [ca * x + cc * y + ce, cb * x + cd * y + cf];
        let (w, h) = (bounds.width, bounds.height);
        let got = [
            placed(0.0, 0.0),
            placed(w, 0.0),
            placed(0.0, h),
            placed(w, h),
        ];
        let want = [
            cold(page.x, page.y),
            cold(page.x + page.width, page.y),
            cold(page.x, page.y + page.height),
            cold(page.x + page.width, page.y + page.height),
        ];
        let scale = 1.0
            + want
                .iter()
                .chain(&got)
                .flatten()
                .map(|value| value.abs())
                .fold(0.0, f32::max)
                * 4.0
            + page.x.abs()
            + page.y.abs();
        assert!(
            got.iter()
                .flatten()
                .zip(want.iter().flatten())
                .all(|(got, want)| (got - want).abs() <= 2e-5 * scale),
            "{id:?}: corners placed in its own frame {got:?} are not where its turned box stands, {want:?}"
        );
    }

    /// The pins found under `roots` are what walking every box under them
    /// finds, and every box that pins is noted as a pin. Small worlds only,
    /// as the other checks.
    pub(super) fn check_moved_pins(
        &self,
        roots: &[StableNodeId],
        layout_moved: &[StableNodeId],
        affected: &[StableNodeId],
        pending: &[StableNodeId],
    ) {
        if self.len() > SHIPPED_ACCESSIBILITY_LIMIT {
            return;
        }
        self.nodes.check_node_spaces();
        let mut memo = ProjectionMemo::default();
        let (mut walked, mut walked_subtrees, mut open) = (Vec::new(), Vec::new(), Vec::new());
        for &root in roots {
            self.reach_moved_subtree(
                root,
                &mut open,
                layout_moved,
                true,
                &mut walked,
                &mut walked_subtrees,
                &mut memo,
            );
        }
        let sorted = |ids: &[StableNodeId]| {
            let mut ids = ids.to_vec();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        assert_eq!(
            sorted(affected),
            sorted(&walked),
            "the pins under moved boxes found other boxes to project than a walk"
        );
        assert_eq!(
            sorted(pending),
            sorted(&walked_subtrees),
            "the pins under moved boxes found other subtrees to project than a walk"
        );
    }
}

#[cfg(any(test, feature = "layout-verify"))]
impl UiWorld {
    /// Every node under a moved box the delta does not project is what the
    /// projections last handed out: projected from scratch, it is the same.
    pub(super) fn check_kept_accessibility(
        &self,
        moved_seeds: &[StableNodeId],
        affected: &[StableNodeId],
    ) {
        if moved_seeds.is_empty() || self.len() > SHIPPED_ACCESSIBILITY_LIMIT {
            return;
        }
        let shipped = self
            .overlays_a11y
            .accessibility_shipped
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut memo = ProjectionMemo::default();
        let mut pending = moved_seeds.to_vec();
        let mut visited = hashbrown::HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            if let Some(node) = self.nodes.get(id) {
                pending.extend(node.hierarchy.children.iter().copied());
            }
            if affected.binary_search(&id).is_ok() {
                continue;
            }
            let fresh = self.project_accessibility_entries(id, false, true, &mut memo);
            match fresh.first() {
                Some(fresh) => {
                    if let Some(held) = shipped.get(&id) {
                        assert_eq!(
                            held, fresh,
                            "{id:?}: a delta kept an accessibility node that projects differently now"
                        );
                    }
                }
                None => assert!(
                    !shipped.contains_key(&id),
                    "{id:?}: a delta kept an accessibility node that no longer projects"
                ),
            }
        }
    }
}
