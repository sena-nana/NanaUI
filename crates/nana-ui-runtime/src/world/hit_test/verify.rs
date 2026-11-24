//! Checks of the retained index: moved entries against a cold build (tests
//! and `layout-verify`) and pointer eligibility (`debug_assertions`).

use super::*;

impl UiWorld {
    /// A moved entry, and every entry below it, is what a rebuild of the
    /// whole index makes of it, wherever that rebuild stays cheap. Bit for
    /// bit where no scroll since its build has shifted it or what is above
    /// it; under a shift the index holds it in the space of the scroll it
    /// was built at, so its transforms and bounds are brought into the live
    /// space and compared to rounding there.
    #[cfg(any(test, feature = "layout-verify"))]
    pub(super) fn check_moved_hit_entries(&self, document: DocumentId, moved: &[StableNodeId]) {
        if self.len() > 4096 {
            return;
        }
        let index = &self.hit_index.hit_test_index[&document];
        let mut below: Vec<StableNodeId> = Vec::new();
        let mut open: Vec<StableNodeId> = moved.to_vec();
        while let Some(id) = open.pop() {
            let Some(hit) = index.entries.get(&id) else {
                continue;
            };
            below.push(id);
            open.extend(hit.children.iter().flatten().copied());
        }
        let seeds = self
            .document_roots(document)
            .iter()
            .map(|&id| (id, IDENTITY_AFFINE, None))
            .collect::<Vec<_>>();
        let fresh = HitIndex::from_entries(self.build_hit_entries(seeds));
        let near =
            |a: f32, b: f32| a == b || (a - b).abs() <= (a.abs().max(b.abs()) * 1e-6).max(1e-4);
        let near_affine = |a: [f32; 6], b: [f32; 6], shift: [f32; 2]| {
            (0..4).all(|i| a[i] == b[i])
                && near(a[4] + shift[0], b[4])
                && near(a[5] + shift[1], b[5])
        };
        let near_bounds = |a: Bounds, b: Bounds, shift: [f32; 2]| match (a.translated(shift), b) {
            (Bounds::Known(a), Bounds::Known(b)) => {
                near(a.x, b.x)
                    && near(a.y, b.y)
                    && near(a.width, b.width)
                    && near(a.height, b.height)
            }
            (a, b) => a == b,
        };
        let near_clips = |a: &[(LayoutBox, [f32; 6])], b: &[(LayoutBox, [f32; 6])], shift| {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((a, at), (b, bt))| a == b && near_affine(*at, *bt, shift))
        };
        // Whether an entry holds a scroll shift at or below it: its bounds
        // merge what that shift holds in the space of another scroll, so
        // they come to rounding only, though its own transform is exact.
        // `below` lists each entry before what it holds.
        let mut shifted_below: crate::NodeSet = crate::NodeSet::default();
        for &id in below.iter().rev() {
            let hit = &index.entries[&id];
            if hit.shift != [0.0, 0.0]
                || hit
                    .children
                    .iter()
                    .flatten()
                    .any(|child| shifted_below.contains(child))
            {
                shifted_below.insert(id);
            }
        }
        for &id in &below {
            let (Some(ours), Some(theirs)) = (index.entries.get(&id), fresh.entries.get(&id))
            else {
                panic!("{id:?}: a moved hit entry has no counterpart in a rebuild");
            };
            let (a, b) = (&ours.entry, &theirs.entry);
            let shift = index.inherited_shift(id);
            let placed = if shift == [0.0, 0.0] && ours.shift == [0.0, 0.0] {
                (if shifted_below.contains(&id) {
                    near_bounds(ours.bounds, theirs.bounds, shift)
                } else {
                    ours.bounds == theirs.bounds
                }) && a.transform == b.transform
                    && a.scroll == b.scroll
                    && a.self_clips == b.self_clips
                    && a.child_clips == b.child_clips
                    && hit_bounds(a) == hit_bounds(b)
            } else {
                near_bounds(ours.bounds, theirs.bounds, shift)
                    && near_affine(a.transform, b.transform, shift)
                    && near(a.scroll[0], b.scroll[0])
                    && near(a.scroll[1], b.scroll[1])
                    && near_clips(&a.self_clips, &b.self_clips, shift)
                    && near_clips(&a.child_clips, &b.child_clips, shift)
                    && near_bounds(hit_bounds(a), hit_bounds(b), shift)
            };
            assert!(
                placed
                    && ours.parent == theirs.parent
                    && a.layout == b.layout
                    && a.origin == b.origin
                    && a.pinned == b.pinned
                    && a.persp == b.persp
                    && a.local_children == b.local_children
                    && a.hittable == b.hittable
                    && a.z_index == b.z_index
                    && a.menu == b.menu
                    && a.escaped == b.escaped
                    && a.path_clips.is_empty() == b.path_clips.is_empty(),
                "{id:?}: a moved hit entry differs from a rebuilt one: {a:?} != {b:?} \
                 (bounds {:?} != {:?}, parent {:?} != {:?}, scroll shift {shift:?} / {:?})",
                ours.bounds,
                theirs.bounds,
                ours.parent,
                theirs.parent,
                ours.shift,
            );
        }
    }
}

/// World size up to which commits and hit-index builds re-check the index
/// against the pointer authority under `debug_assertions`. Unit-test worlds
/// are a handful of nodes; product worlds are thousands, and auditing those
/// on every commit would slow debug builds without testing anything the small
/// worlds do not.
#[cfg(debug_assertions)]
const HIT_AUDIT_LIMIT: usize = 512;

impl UiWorld {
    /// Assert that the retained index offers no pointer target the committed
    /// pointer authority ([`Self::validate_pointer_target`]) rejects.
    ///
    /// The index keeps its geometry until the frame rebuilds it, so a commit
    /// that takes a node out of pointer dispatch must withdraw the entry at
    /// once, or a hit between that commit and the next frame lands on it.
    /// Which mutations do that is a judgement no still-frame test checks;
    /// running this after every commit and every build makes the whole suite
    /// a test of it, at O(index × depth), so it is bounded to small worlds.
    #[cfg(debug_assertions)]
    pub(in crate::world) fn audit_hit_eligibility(&self, site: &str) {
        if self.nodes.len() > HIT_AUDIT_LIMIT {
            return;
        }
        for (&document, index) in &self.hit_index.hit_test_index {
            for (&id, indexed) in &index.entries {
                let entry = &indexed.entry;
                if !entry.hittable && entry.menu.is_none() {
                    continue;
                }
                if let Err(error) = self.validate_pointer_target(document, id) {
                    let why = self.nodes.get(id).map(|node| {
                        (
                            self.is_mounted(id),
                            node.interaction.pointer_events,
                            self.used_pointer_events(id),
                            node.resolved.0.pointer_events,
                        )
                    });
                    panic!(
                        "{site}: retained hit index offers {id:?}, which the pointer authority \
                         rejects: {error:?} (mounted, interaction, used, computed) = {why:?}"
                    );
                }
            }
        }
    }
}
