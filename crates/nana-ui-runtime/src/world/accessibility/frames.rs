//! Bounds relative to the parent's space: offsets in a parent space, a
//! turned space's inverse and frames handed from the viewport.

use super::*;

impl UiWorld {
    /// Where `id`'s accessible box stands in the viewport, as its projection
    /// computes it.
    pub(super) fn accessible_viewport_bounds(
        &self,
        id: StableNodeId,
        plain: bool,
        memo: &mut ProjectionMemo,
    ) -> Option<LayoutBox> {
        if plain || self.axis_aligned(id, memo) {
            self.component_layout_box(id)
        } else {
            self.visible_accessibility_bounds(id, memo)
        }
    }

    /// Move `entries`' bounds from the viewport to their parents' children's
    /// spaces (each parent's space moved by its scroll,
    /// [`AccessibilityNode::scroll`]), and give the first its own scroll.
    /// The first entry's parent is a retained node; the virtual ones after
    /// it hang from entries before them, and `pages` holds where each of
    /// those stands on the page (`pages[i]` for `entries[i + 1]`).
    pub(super) fn make_bounds_relative(
        &self,
        entries: &mut [AccessibilityNode],
        pages: &[LayoutBox],
        plain: bool,
        memo: &mut ProjectionMemo,
    ) {
        let owner = entries[0].id;
        // What the owner scrolls is placed in its space moved by that much;
        // virtual nodes scroll nothing.
        let owner_scroll = if plain {
            [0.0, 0.0]
        } else {
            self.accessible_scroll(owner, memo)
        };
        entries[0].scroll = owner_scroll;
        // Virtual nodes stand in their owner's space. Where that is framed
        // (turned or scaled from the viewport), they are placed in it from
        // their page boxes, upright there as on the page; elsewhere from
        // their viewport boxes.
        let turned = (entries.len() > 1 && !plain)
            .then(|| self.accessible_space(owner, memo))
            .flatten()
            .filter(|space| space.framed)
            .and_then(|space| Some((space.unit?, self.layout_origin(owner)?)));
        let in_owner = |page: LayoutBox| {
            let ([ua, ud], [ox, oy]) = turned.expect("placed in a turned owner");
            LayoutBox {
                x: ua * (page.x - ox),
                y: ud * (page.y - oy),
                width: ua * page.width,
                height: ud * page.height,
            }
        };
        let origins = if entries.len() > 1 {
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| match (turned, index) {
                    (Some(_), 0) => (entry.id, 0.0, 0.0),
                    (Some(_), _) => {
                        let placed = in_owner(pages[index - 1]);
                        (entry.id, placed.x, placed.y)
                    }
                    (None, _) => (entry.id, entry.bounds.x, entry.bounds.y),
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for (index, entry) in entries.iter_mut().enumerate() {
            let Some(parent) = entry.parent else {
                // A root stands in the viewport, framed or not.
                if index == 0
                    && !plain
                    && let Some((bounds, transform)) =
                        self.viewport_frame_in_parent(entry.id, None, memo)
                {
                    #[cfg(any(test, feature = "layout-verify"))]
                    self.check_viewport_frame(entry.id, None, bounds, transform, memo);
                    entry.bounds = bounds;
                    entry.transform = Some(transform);
                }
                continue;
            };
            // Placed from a box in the parent's own space: into the space
            // its scroll moves.
            let mut scroll = if index != 0 && parent == owner {
                owner_scroll
            } else {
                [0.0, 0.0]
            };
            if index == 0 {
                if let Some((bounds, transform)) =
                    self.offset_bounds_in_parent(entry, parent, plain, memo)
                {
                    entry.bounds = bounds;
                    entry.transform = transform;
                    continue;
                }
                if !plain {
                    scroll = self.accessible_scroll(parent, memo);
                }
                if !plain
                    && let Some((bounds, transform)) =
                        self.viewport_frame_in_parent(entry.id, Some(parent), memo)
                {
                    #[cfg(any(test, feature = "layout-verify"))]
                    self.check_viewport_frame(entry.id, Some(parent), bounds, transform, memo);
                    entry.bounds = bounds;
                    entry.transform = Some(transform);
                    continue;
                }
                if !plain && let Some(inverse) = self.turned_space_inverse(parent, memo) {
                    // Placed from its viewport box under a framed space: the
                    // inverse map (of its children's space) brings that box
                    // back upright.
                    let [a, b, c, d, e, f] = inverse;
                    let (x, y) = (entry.bounds.x, entry.bounds.y);
                    entry.bounds.x = a * x + c * y + e;
                    entry.bounds.y = b * x + d * y + f;
                    entry.transform = Some(nana_ui_core::PaintTransform {
                        a,
                        b,
                        c,
                        d,
                        e: 0.0,
                        f: 0.0,
                    });
                    continue;
                }
            } else if turned.is_some() {
                let placed = in_owner(pages[index - 1]);
                let (x, y) = origins
                    .iter()
                    .find(|(id, ..)| *id == parent)
                    .map_or((0.0, 0.0), |&(_, x, y)| (x, y));
                entry.bounds = LayoutBox {
                    x: placed.x - x + scroll[0],
                    y: placed.y - y + scroll[1],
                    ..placed
                };
                continue;
            }
            let origin = if index == 0 {
                if plain && !self.nodes.has_widget_visuals() {
                    self.layout_origin(parent).map(|[x, y]| (x, y))
                } else {
                    self.accessible_viewport_bounds(parent, plain, memo)
                        .map(|bounds| (bounds.x, bounds.y))
                }
            } else {
                origins
                    .iter()
                    .find(|(id, ..)| *id == parent)
                    .map(|&(_, x, y)| (x, y))
            };
            if let Some((x, y)) = origin {
                entry.bounds.x -= x;
                entry.bounds.y -= y;
            }
            entry.bounds.x += scroll[0];
            entry.bounds.y += scroll[1];
        }
    }

    /// The map from the viewport into the space `id`'s children are placed
    /// in, when `id`'s space is framed: turned or scaled from the viewport's
    /// ([`AccessibleSpace::framed`]).
    fn turned_space_inverse(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<[f32; 6]> {
        if !self.accessible_space(id, memo)?.framed {
            return None;
        }
        invert_affine(self.children_space_to_viewport(id, memo)?)
    }

    /// The map from the space `parent`'s children are placed in to the
    /// viewport, for a box placed in it from the viewport: the inverse-free
    /// move to its accessible box, less its scroll, where its space is the
    /// viewport's moved, its own map after its scroll where it is framed.
    pub(super) fn parent_space_to_viewport(
        &self,
        parent: Option<StableNodeId>,
        memo: &mut ProjectionMemo,
    ) -> Option<[f32; 6]> {
        let Some(parent) = parent else {
            return Some(IDENTITY_AFFINE);
        };
        if self.accessible_space(parent, memo)?.framed {
            return self.children_space_to_viewport(parent, memo);
        }
        let origin = self
            .accessible_viewport_bounds(parent, false, memo)
            .map_or([0.0, 0.0], |bounds| [bounds.x, bounds.y]);
        Some(scrolled_map(
            [1.0, 0.0, 0.0, 1.0, origin[0], origin[1]],
            self.accessible_scroll(parent, memo),
        ))
    }

    /// `id`'s bounds and transform in `parent`'s space where it is placed
    /// from its viewport box and hands its own frame
    /// ([`Self::frames_from_viewport`]): its layout box, at the origin of its
    /// own space, which its chain's map to the viewport takes, after the
    /// parent's space's inverse, to the parent's. The move of that map is its
    /// offset, the rest its transform.
    pub(super) fn viewport_frame_in_parent(
        &self,
        id: StableNodeId,
        parent: Option<StableNodeId>,
        memo: &mut ProjectionMemo,
    ) -> Option<(LayoutBox, nana_ui_core::PaintTransform)> {
        let space = self.accessible_space(id, memo)?;
        if space.local || !space.framed {
            return None;
        }
        let record = self.nodes.get(id)?;
        if record.hierarchy.parent != parent {
            return None;
        }
        let [a, b, c, d, e, f] = self.accessible_space_to_viewport(id, memo)?;
        let [ia, ib, ic, id_, ie, if_] =
            invert_affine(self.parent_space_to_viewport(parent, memo)?)?;
        let local = self.component_layout_box(id)?;
        let placed = [
            ia * a + ic * b,
            ib * a + id_ * b,
            ia * c + ic * d,
            ib * c + id_ * d,
            ia * e + ic * f + ie,
            ib * e + id_ * f + if_,
        ];
        Some((
            LayoutBox {
                x: placed[4],
                y: placed[5],
                width: local.width,
                height: local.height,
            },
            nana_ui_core::PaintTransform {
                a: placed[0],
                b: placed[1],
                c: placed[2],
                d: placed[3],
                e: 0.0,
                f: 0.0,
            },
        ))
    }

    /// `entry`'s bounds and transform in `parent`'s space, from its local
    /// layout box read straight off the record, when that is what they are:
    /// both accessible boxes are their layout boxes (its offset), or it is
    /// placed by its offset ([`Self::offset_placed`]: the offset less the
    /// parent's scroll, scaled, with its own transform folded in or handed
    /// on). The viewport boxes are chain sums through the transforms, so
    /// their difference would equal this only up to the last bit, and that
    /// bit moves with every ancestor a fractional move shifts; the offset
    /// itself does not, which is what lets a box that follows its parent
    /// keep the bounds it was handed ([`Self::keeps_relative_bounds`]).
    fn offset_bounds_in_parent(
        &self,
        entry: &AccessibilityNode,
        parent: StableNodeId,
        plain: bool,
        memo: &mut ProjectionMemo,
    ) -> Option<(LayoutBox, Option<nana_ui_core::PaintTransform>)> {
        let record = self.nodes.get(entry.id)?;
        if self.nodes.layout_frame(entry.id) != crate::LayoutFrame::Parent
            || record.hierarchy.parent != Some(parent)
            || matches!(record.kind.as_ref(), NodeKind::Document)
            || self.invalidation.geometry_frame.get().is_some()
        {
            return None;
        }
        let local = self.nodes.local_layout(entry.id);
        let offset = LayoutBox {
            x: local.x,
            y: local.y,
            width: entry.bounds.width,
            height: entry.bounds.height,
        };
        // An aligned box's chain runs through its parent, so the parent's
        // accessible box is its layout box too.
        if plain || self.axis_aligned(entry.id, memo) {
            return Some((offset, None));
        }
        // [`Self::offset_placed`], with the parent's space looked up once.
        let above = self.accessible_space(parent, memo);
        if let Some(unit) = above.and_then(|space| space.unit)
            && self.motion_free_unclipped()
            && self.offset_placed_under(entry.id, record, true)
        {
            #[cfg(any(test, feature = "layout-verify"))]
            assert!(self.offset_placed(entry.id, record, memo));
            let placed = self.local_placement(entry.id, record, unit);
            #[cfg(any(test, feature = "layout-verify"))]
            self.check_offset_bounds(entry, parent, placed, memo);
            return Some(placed);
        }
        // Off those paths, only where what was projected for both is their
        // layout boxes, as a cold projection finds them, the parent's space
        // is the viewport's, moved, and the box hands no frame of its own.
        if above.is_some_and(|space| space.framed)
            || self
                .accessible_space(entry.id, memo)
                .is_some_and(|space| space.framed)
        {
            return None;
        }
        // A box under a scroll stands off its layout box, and its offset is
        // not in the space the scroll moves.
        if !self.nodes.contains(parent)
            || self.nodes.scroll_offset(parent) != ScrollOffset::default()
        {
            return None;
        }
        let own = self.record_layout_origin(entry.id, record);
        let above = self.layout_origin(parent)?;
        let parent_bounds = self.accessible_viewport_bounds(parent, plain, memo)?;
        ([entry.bounds.x, entry.bounds.y] == own && [parent_bounds.x, parent_bounds.y] == above)
            .then_some((offset, None))
    }
}
