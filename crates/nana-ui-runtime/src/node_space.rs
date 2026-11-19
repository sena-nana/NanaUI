//! How a node stands to the coordinate spaces around it, classified once.
//!
//! Every consumer that keeps what a moved box holds in that box's
//! coordinates -- hit entries, accessible bounds, extracted records -- has
//! to know the same few things of each box: what layout measures it from,
//! whether its own transform only moves it, scales it upright or turns it,
//! whether it opens a 3D context, whether it cuts what it holds, and whether
//! its visual places anything from the page. [`NodeSpace`] is that answer,
//! computed from the node's own record and visual by [`NodeSpace::of`] and
//! kept on [`crate::store::NodeRecord::space`]. [`crate::store::NodeStore`]
//! brings it up to date wherever a field it reads is written
//! (`note_node_space`): style resolution, a write of the layout frame, a
//! visual, an insert. Readers derive what they need from it -- hit testing,
//! accessibility, extraction -- and add what only their chain knows (the
//! transforms above, a hit parent, a scroll).
//!
//! Only what a node's own record decides is here: nothing that a change
//! above it could alter, so no write needs to visit a subtree.

use nana_ui_core::{LayoutStyle, PositionSpec};

use crate::{LayoutFrame, NodeKind};

/// The classification ([module docs](self)). A set of bits; each derived
/// question below names the consumers it answers for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct NodeSpace(u16);

impl NodeSpace {
    // Where layout measures its box from.
    /// Its layout frame is not its parent: a block above it (an absolute
    /// box escaping its parent) or the page (a root, a `fixed` box, a
    /// triggered menu's item).
    pub(crate) const OFF_PARENT: Self = Self(1 << 0);
    /// `position: fixed`.
    pub(crate) const FIXED: Self = Self(1 << 1);
    /// `position: sticky`: placed by a sticky offset from where it stands.
    pub(crate) const STICKY: Self = Self(1 << 2);

    // Its own 2D `transform`: at most one of the three.
    /// A transform whose linear part is the identity: it only moves.
    pub(crate) const MOVES: Self = Self(1 << 3);
    /// A positive scale on each axis other than one: upright, but its own
    /// layout px are not its parent's.
    pub(crate) const SCALES: Self = Self(1 << 4);
    /// A rotation, a skew, a flip (or a linear part that is not finite).
    pub(crate) const TURNS: Self = Self(1 << 5);

    // A 3D context of its own.
    /// A `transform_3d`. Kept apart from [`Self::PERSPECTIVE`] and
    /// [`Self::PRESERVE_3D`]: a hit test takes a planar `transform_3d`'s
    /// homography as it comes and hands local coordinates when it is affine
    /// ([`Self::hit_frames_itself`]), where accessibility takes any of the
    /// three as 3D and falls back to the box around projected corners
    /// ([`Self::depth`]). Unifying them would change what accessibility
    /// hands out for a planar 3D transform, so both stay as they are.
    pub(crate) const TRANSFORM_3D: Self = Self(1 << 6);
    /// `perspective`.
    pub(crate) const PERSPECTIVE: Self = Self(1 << 7);
    /// `transform-style: preserve-3d`.
    pub(crate) const PRESERVE_3D: Self = Self(1 << 8);

    /// It clips its overflow on either axis.
    pub(crate) const CLIPS: Self = Self(1 << 9);
    /// A document node.
    pub(crate) const DOCUMENT: Self = Self(1 << 10);

    // What its visual reads and places.
    /// Its visual places accessible nodes from its geometry, or makes its
    /// own accessible box something other than its layout box: an open
    /// select's list box and options, a tree's rows, a menu surface's
    /// items, a modal frame's surface. A closed select projects only its
    /// field, which is placed as any box is.
    pub(crate) const VIRTUAL: Self = Self(1 << 11);
    /// A modal frame: its accessible box is its surface, and a hit test
    /// cuts its children to it.
    pub(crate) const MODAL: Self = Self(1 << 12);
    /// Its visual's geometry reads the page
    /// ([`crate::WidgetVisual::geometry_reads_page`]: an open menu surface).
    pub(crate) const READS_PAGE: Self = Self(1 << 13);
    /// An open select: its menu is fitted to a room on the page, which a
    /// move above it changes (`UiWorld::local_select_menu_surface`).
    pub(crate) const MENU_ROOM: Self = Self(1 << 14);

    const TRANSFORMED: Self = Self(Self::MOVES.0 | Self::SCALES.0 | Self::TURNS.0);
    const DEPTH: Self = Self(Self::TRANSFORM_3D.0 | Self::PERSPECTIVE.0 | Self::PRESERVE_3D.0);
    const PLACED_ELSEWHERE: Self = Self(Self::OFF_PARENT.0 | Self::FIXED.0 | Self::STICKY.0);

    /// The node whose resolved layout is `style`, measured from `frame`, of
    /// `kind`, showing `widget_visual`.
    pub(crate) fn of(
        style: &LayoutStyle,
        frame: LayoutFrame,
        kind: &NodeKind,
        widget_visual: Option<&crate::WidgetVisualHandle>,
    ) -> Self {
        let mut space = Self::of_style(style);
        if let Some(widget_visual) = widget_visual {
            if widget_visual.geometry_reads_page() {
                space = space | Self::READS_PAGE;
            }
            if widget_visual.has_virtual_nodes() {
                space = space | Self::VIRTUAL;
            }
            if widget_visual.is_open_select_menu() {
                space = space | Self::VIRTUAL | Self::MENU_ROOM;
            }
            if widget_visual.is_menu_surface() {
                space = space | Self::VIRTUAL;
            }
            if widget_visual.is_modal_frame() {
                space = space | Self::VIRTUAL | Self::MODAL;
            }
        }
        if frame != LayoutFrame::Parent {
            space = space | Self::OFF_PARENT;
        }
        if matches!(kind, NodeKind::Document) {
            space = space | Self::DOCUMENT;
        }
        space
    }

    /// What `style` alone decides: placement by position, the own
    /// transform, the 3D context and the clip. Hit testing classifies a
    /// box's effective style this way where it is not its resolved one
    /// (a motion overlay, an overlay host's branch).
    pub(crate) fn of_style(style: &LayoutStyle) -> Self {
        let mut bits = 0;
        match style.position {
            PositionSpec::Fixed => bits |= Self::FIXED.0,
            PositionSpec::Sticky => bits |= Self::STICKY.0,
            _ => {}
        }
        let transforms = &*style.transforms;
        if let Some(turn) = transforms.transform {
            let (a, b, c, d) = (turn.a, turn.b, turn.c, turn.d);
            bits |= if a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 {
                Self::MOVES.0
            } else if b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0 {
                Self::SCALES.0
            } else {
                Self::TURNS.0
            };
        }
        if transforms.transform_3d.is_some() {
            bits |= Self::TRANSFORM_3D.0;
        }
        if transforms.css_perspective.is_some() {
            bits |= Self::PERSPECTIVE.0;
        }
        if transforms.preserve_3d {
            bits |= Self::PRESERVE_3D.0;
        }
        if style.clips_overflow() {
            bits |= Self::CLIPS.0;
        }
        Self(bits)
    }

    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub(crate) const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Its own style opens or joins a 3D context (a `transform_3d`,
    /// `perspective` or `preserve-3d`). Accessibility hands what such a box
    /// holds no offsets ([`Self::hands_offsets`]); a hit entry is not built
    /// again in place under one.
    pub(crate) const fn depth(self) -> bool {
        self.intersects(Self::DEPTH)
    }

    /// It asks for a 3D rendering context its children are drawn in
    /// (`perspective`, `preserve-3d`:
    /// [`LayoutStyle::fails_closed_3d_context`]). Hit testing and the
    /// accessible transform chain carry it down.
    pub(crate) const fn opens_3d_context(self) -> bool {
        self.intersects(Self(Self::PERSPECTIVE.0 | Self::PRESERVE_3D.0))
    }

    /// It has a 2D transform of its own, of any kind.
    pub(crate) const fn transformed(self) -> bool {
        self.intersects(Self::TRANSFORMED)
    }

    /// Its own 2D transform does more than move it -- it scales, turns,
    /// skews or flips -- so its own layout px are a space of their own
    /// (accessibility: [`Self::keeps_space`], a framed space; hit testing:
    /// [`Self::hit_frames_itself`]).
    pub(crate) const fn frames_itself(self) -> bool {
        self.intersects(Self(Self::SCALES.0 | Self::TURNS.0))
    }

    /// Its own 2D transform, if any, holds its box upright: none, a move or
    /// a positive scale.
    pub(crate) const fn upright(self) -> bool {
        !self.intersects(Self::TURNS)
    }

    /// Its accessible bounds can be its local offset from its parent and
    /// its own 2D transform, where its parent's space holds its box
    /// upright: measured from its parent, and placed nowhere else by its
    /// own style (no 3D context of its own, not `fixed` or `sticky`).
    pub(crate) const fn places_by_offset(self) -> bool {
        !self.intersects(Self::PLACED_ELSEWHERE) && !self.depth()
    }

    /// The accessible boxes under it can be placed by their offsets in its
    /// space: no 3D context. Any 2D transform will do: a translation and a
    /// positive scale multiply the offsets under it by factors that do not
    /// depend on where any box stands, and a rotation, a skew or a flip is
    /// handed to the platform as the box's own transform
    /// ([`crate::AccessibilityNode::transform`]), its children placed by
    /// their offsets inside it. Scrolling, sticky offsets and `absolute` /
    /// `fixed` placement only translate, and what they translate by is not
    /// part of an offset. Under a 3D context the bounds are the box around
    /// projected corners, which no offset gives.
    pub(crate) const fn hands_offsets(self) -> bool {
        !self.depth()
    }

    /// It, or what it holds, may not keep its accessible bounds when a box
    /// above it moves: not placed by its offset, cutting what it holds (the
    /// accessible `clip` is in viewport coordinates), handing its children
    /// no offsets, or a visual whose accessible nodes come from its
    /// geometry. Every other box keeps its bounds and transform when its
    /// parent moves, to the bit, and so does all it holds.
    ///
    /// Unlike a pinned hit entry (`HitEntry::pinned`), a `fixed` box
    /// and a clip pin here: a hit entry hangs a `fixed` box from no parent
    /// and keeps its clips in its own box's coordinates, where accessible
    /// bounds are relative to the tree parent and clips are on the viewport.
    pub(crate) const fn pins_accessibility(self) -> bool {
        !self.places_by_offset()
            || self.intersects(Self::CLIPS)
            || !self.hands_offsets()
            || self.intersects(Self::VIRTUAL)
    }

    /// Whether a box whose style went from `self` to `after` hands the
    /// accessible bounds of what it holds the same space: no 3D context on
    /// either side, and its own 2D transform frames a space of its own both
    /// times or only moves (or is none) both times. Then what it holds
    /// placed by its offset keeps its bounds to the bit
    /// ([`crate::SystemWork::transform_kept`]).
    pub(crate) const fn keeps_space(self, after: Self) -> bool {
        !self.depth() && !after.depth() && self.frames_itself() == after.frames_itself()
    }

    /// Its style leaves its accessible box where its parent's coordinates
    /// put it: no transform, 3D context, `fixed` or `sticky` (the scroll
    /// and a triggered menu's placement are the chain's to add).
    pub(crate) const fn aligned_by_style(self) -> bool {
        !self.intersects(Self(
            Self::TRANSFORMED.0 | Self::DEPTH.0 | Self::FIXED.0 | Self::STICKY.0,
        ))
    }

    /// Whether a hit entry whose own local transform is `local` -- its
    /// transform about its origin, the sticky offset folded in, or the
    /// identity where a `transform_3d` meets a 3D context above -- hands its
    /// children its own layout coordinates as far as the box itself goes:
    /// its 2D transform frames itself, or a planar `transform_3d` came out
    /// affine and not a move. The chain adds that no perspective reaches it
    /// and no 3D context holds its children.
    pub(crate) fn hit_frames_itself(self, local: &([f32; 6], [f32; 2])) -> bool {
        if self.contains(Self::TRANSFORM_3D) {
            local.0[..4] != [1.0, 0.0, 0.0, 1.0] && local.1 == [0.0, 0.0]
        } else {
            self.frames_itself()
        }
    }

    /// A select whose menu needs its room on the page, or a visual whose
    /// geometry reads the page: what extraction looks at under a move
    /// (`UiWorld::settle_layout_moves`).
    pub(crate) const fn page_bound(self) -> bool {
        self.intersects(Self(Self::READS_PAGE.0 | Self::MENU_ROOM.0))
    }
}

impl std::ops::BitOr for NodeSpace {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::PaintTransform;

    fn transformed(a: f32, b: f32, c: f32, d: f32) -> NodeSpace {
        let mut style = LayoutStyle::default();
        style.transforms.transform = Some(PaintTransform {
            a,
            b,
            c,
            d,
            e: 3.0,
            f: -2.0,
        });
        NodeSpace::of_style(&style)
    }

    #[test]
    fn a_transform_is_one_of_three_kinds() {
        assert_eq!(transformed(1.0, 0.0, 0.0, 1.0), NodeSpace::MOVES);
        assert_eq!(transformed(1.5, 0.0, 0.0, 0.5), NodeSpace::SCALES);
        assert_eq!(transformed(1.0, 0.0, 0.0, 2.0), NodeSpace::SCALES);
        for turned in [
            transformed(-1.0, 0.0, 0.0, 1.0),
            transformed(1.0, 0.2, 0.0, 1.0),
            transformed(1.0, 0.0, 0.3, 1.0),
            transformed(0.0, 0.0, 0.0, 1.0),
            transformed(f32::NAN, 0.0, 0.0, 1.0),
        ] {
            assert_eq!(turned, NodeSpace::TURNS);
            assert!(turned.frames_itself() && !turned.upright());
        }
        assert!(NodeSpace::default().upright() && !NodeSpace::default().frames_itself());
    }

    #[test]
    fn a_space_is_kept_between_two_frames_or_two_moves() {
        let turn = transformed(0.0, 1.0, -1.0, 0.0);
        let scale = transformed(2.0, 0.0, 0.0, 2.0);
        let moves = transformed(1.0, 0.0, 0.0, 1.0);
        let none = NodeSpace::default();
        assert!(turn.keeps_space(scale) && scale.keeps_space(turn));
        assert!(moves.keeps_space(none) && none.keeps_space(moves));
        assert!(!turn.keeps_space(moves) && !none.keeps_space(scale));
        let deep = turn | NodeSpace::PRESERVE_3D;
        assert!(!deep.keeps_space(turn) && !turn.keeps_space(deep));
    }
}
