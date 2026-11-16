//! Builder edits of a shared layout, remembered so a chain repeated across
//! nodes shares one result instead of copying the layout once per node.
//!
//! A builder such as `Stack::row(0.0).width(..).min_height(..)` starts from
//! the shared default layout, so its first edit copies the whole
//! `LayoutStyle` (`Arc::make_mut`), and the interner then compares the copy
//! field by field with an equal one it already holds and drops it. Building a
//! page repeats the same few chains for every row and cell. An edit here is
//! identified exactly -- its kind and its values to the bit -- and the layout
//! it starts from by pointer: the memo holds both `Arc`s, so the address is
//! not reused and the contents cannot be written in place (`Arc::make_mut`
//! copies a shared layout). Applying a remembered edit to the same input
//! therefore gives the very result remembered.
//!
//! An edit is remembered the second time it is seen from the same input, so
//! a one-off chain pays one copy, as it did, and never one per step. A layout
//! nothing else holds is edited in place and never looked up.

use std::cell::RefCell;
use std::sync::Arc;

use nana_ui_core::{AlignSpec, FlexWrap, LayoutStyle, LengthSpec};

/// A [`LengthSpec`] compared to the bit; `None` for one the memo leaves
/// alone (a `calc()` handle, a viewport unit).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LengthBits(u8, u32, u32);

impl LengthBits {
    pub(crate) fn of(spec: LengthSpec) -> Option<Self> {
        Some(match spec {
            LengthSpec::Px(value) => Self(1, value.to_bits(), 0),
            LengthSpec::Percent(value) => Self(2, value.to_bits(), 0),
            LengthSpec::Em(value) => Self(3, value.to_bits(), 0),
            LengthSpec::Rem(value) => Self(4, value.to_bits(), 0),
            LengthSpec::PercentOffset { offset_q7, percent } => {
                Self(5, percent.to_bits(), offset_q7 as i32 as u32)
            }
            LengthSpec::Calc(_) => {
                if let Some((em, offset_px)) = spec.as_em_offset() {
                    Self(6, em.to_bits(), offset_px.to_bits())
                } else if let Some((rem, offset_px)) = spec.as_rem_offset() {
                    Self(7, rem.to_bits(), offset_px.to_bits())
                } else {
                    return None;
                }
            }
            LengthSpec::Fill => Self(8, 0, 0),
            LengthSpec::Shrink => Self(9, 0, 0),
            LengthSpec::Auto => Self(10, 0, 0),
            LengthSpec::MinContent => Self(11, 0, 0),
            LengthSpec::MaxContent => Self(12, 0, 0),
            LengthSpec::FitContent => Self(13, 0, 0),
            LengthSpec::Viewport { .. } => return None,
        })
    }
}

/// `Stack::row`, `fill_row`, `column`, `fill_column` and `bar`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StackPreset {
    Row,
    FillRow,
    Column,
    FillColumn,
    Bar,
}

/// One builder edit, exact: two equal edits applied to one layout give
/// layouts equal to the bit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LayoutEdit {
    /// One of `Stack`'s presets, with the bits of its clamped gap.
    StackPreset(StackPreset, u32),
    Width(LengthBits),
    Height(LengthBits),
    MinWidth(LengthBits),
    MinHeight(LengthBits),
    /// Bits of the clamped pixel width.
    MaxWidth(u32),
    Grow(u32),
    Shrink(u32),
    /// Bits of the clamped pixel gap.
    Gap(u32),
    Align(AlignSpec),
    Wrap(FlexWrap),
}

/// How many edits a thread remembers, and how many it has seen once.
const REMEMBERED: usize = 16;

struct Remembered {
    from: Arc<LayoutStyle>,
    edit: LayoutEdit,
    to: Arc<LayoutStyle>,
}

#[derive(Default)]
struct Memo {
    remembered: [Option<Remembered>; REMEMBERED],
    next: usize,
    seen: [Option<(Arc<LayoutStyle>, LayoutEdit)>; REMEMBERED],
    next_seen: usize,
}

thread_local! {
    static MEMO: RefCell<Memo> = RefCell::default();
}

/// Apply `edit` (performed by `apply`) to `layout`: in place when nothing
/// else holds it, else the remembered result of the same edit from the same
/// layout, else on a copy.
pub(crate) fn edit_layout(
    layout: &mut Arc<LayoutStyle>,
    edit: LayoutEdit,
    apply: impl FnOnce(&mut LayoutStyle),
) {
    if let Some(unique) = Arc::get_mut(layout) {
        apply(unique);
        return;
    }
    MEMO.with(|memo| {
        let mut memo = memo.borrow_mut();
        let kept = memo
            .remembered
            .iter()
            .flatten()
            .find(|kept| kept.edit == edit && Arc::ptr_eq(&kept.from, layout));
        if let Some(kept) = kept {
            let to = Arc::clone(&kept.to);
            #[cfg(test)]
            {
                let mut fresh = LayoutStyle::clone(layout);
                apply(&mut fresh);
                assert!(
                    fresh == *to,
                    "a remembered edit gives what applying it again does"
                );
            }
            *layout = to;
            return;
        }
        let from = Arc::clone(layout);
        apply(Arc::make_mut(layout));
        let seen = memo.seen.iter_mut().find(|seen| {
            seen.as_ref()
                .is_some_and(|(seen, seen_edit)| *seen_edit == edit && Arc::ptr_eq(seen, &from))
        });
        match seen {
            Some(slot) => {
                *slot = None;
                let next = memo.next;
                memo.remembered[next] = Some(Remembered {
                    from,
                    edit,
                    to: Arc::clone(layout),
                });
                memo.next = (next + 1) % REMEMBERED;
            }
            None => {
                let next = memo.next_seen;
                memo.seen[next] = Some((from, edit));
                memo.next_seen = (next + 1) % REMEMBERED;
            }
        }
    });
}

/// The interner replaced `before`, a builder's result, with `after`, an
/// equal layout it already held: remember `after` as the result, so the
/// next chain that ends here hands the interner the layout it keeps.
pub(crate) fn adopt_shared(before: &Arc<LayoutStyle>, after: &Arc<LayoutStyle>) {
    if Arc::ptr_eq(before, after) {
        return;
    }
    MEMO.with(|memo| {
        for kept in memo.borrow_mut().remembered.iter_mut().flatten() {
            if Arc::ptr_eq(&kept.to, before) {
                kept.to = Arc::clone(after);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn width(layout: &mut Arc<LayoutStyle>, px: f32) {
        let spec = LengthSpec::Px(px);
        edit_layout(
            layout,
            LayoutEdit::Width(LengthBits::of(spec).unwrap()),
            |layout| layout.width = Some(spec),
        );
    }

    #[test]
    fn an_edit_seen_twice_from_one_layout_is_shared_the_third_time() {
        let shared = Arc::new(LayoutStyle::default());
        let built = |px| {
            let mut layout = Arc::clone(&shared);
            width(&mut layout, px);
            layout
        };
        let (first, second, third) = (built(24.0), built(24.0), built(24.0));
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&second, &third));
        assert_eq!(third.width, Some(LengthSpec::Px(24.0)));
        // The other zero is another edit, even once one is remembered.
        let zero = (built(0.0), built(0.0), built(0.0)).2;
        let negative = built(-0.0);
        let bits = |layout: &LayoutStyle| match layout.width {
            Some(LengthSpec::Px(px)) => px.to_bits(),
            _ => unreachable!(),
        };
        assert_eq!(bits(&zero), 0.0f32.to_bits());
        assert_eq!(bits(&negative), (-0.0f32).to_bits());
    }

    #[test]
    fn a_layout_nothing_else_holds_is_edited_in_place() {
        let mut layout = Arc::new(LayoutStyle::default());
        let before = Arc::as_ptr(&layout);
        width(&mut layout, 8.0);
        assert_eq!(Arc::as_ptr(&layout), before);
    }

    #[test]
    fn the_interned_result_is_what_the_next_chain_gets() {
        let shared = Arc::new(LayoutStyle::default());
        let built = || {
            let mut layout = Arc::clone(&shared);
            width(&mut layout, 12.0);
            layout
        };
        let _ = (built(), built());
        let remembered = built();
        let interned = Arc::new(LayoutStyle::clone(&remembered));
        adopt_shared(&remembered, &interned);
        assert!(Arc::ptr_eq(&built(), &interned));
    }
}
