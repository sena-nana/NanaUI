//! Stable, generational text handles. Not indices, and never a third-party id.
//!
//! Every handle here is `{ index, generation }` with `generation == 0` reserved
//! for [`NULL`](FontId::NULL), matching `nana_ui_core::motion::MotionHandle`.
//! A freed slot is reissued at a higher generation, so a handle kept across the
//! free is rejected instead of silently aliasing whatever now lives there.
//!
//! The distinct named types are the point: a `ShapeRunId` must not be passable
//! where a `TextLayoutId` is expected, so this is a macro rather than a generic
//! `Handle<T>`.

use serde::{Deserialize, Serialize};

macro_rules! generational_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name {
            index: u32,
            generation: u32,
        }

        impl $name {
            /// The null handle. `generation == 0` is never issued, so this can
            /// never alias a live slot.
            pub const NULL: Self = Self {
                index: 0,
                generation: 0,
            };

            pub const fn from_parts(index: u32, generation: u32) -> Self {
                Self { index, generation }
            }

            pub const fn index(self) -> u32 {
                self.index
            }

            pub const fn generation(self) -> u32 {
                self.generation
            }

            pub const fn is_null(self) -> bool {
                self.generation == 0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::NULL
            }
        }
    };
}

generational_id!(
    /// A face in an engine's font database.
    ///
    /// Meaningful **only** together with the [`FontGeneration`] it was issued
    /// under. It is not a `fontdb::ID` and not an index into any public list.
    FontId
);

generational_id!(
    /// One registration: a file, a byte buffer, or the system scan.
    ///
    /// Unregistering or replacing a source retires every [`FontId`] it issued.
    FontSourceId
);

generational_id!(
    /// One shaped run.
    ///
    /// Invalidated by a change to its source bytes, its style, its
    /// script/direction segmentation, or the font generation.
    ShapeRunId
);

generational_id!(
    /// One laid-out paragraph.
    ///
    /// Invalidated by a change to the source revision, the style, the
    /// constraints, or the font generation.
    TextLayoutId
);

/// Monotonic edit counter for one [`TextSource`](crate::TextSource).
///
/// Comparable **only within a single source**: two unrelated sources may both
/// sit at revision 7. A cache key must pair this with the caller's own identity
/// for the text node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TextRevision(u64);

impl TextRevision {
    /// The revision a freshly constructed source starts at. Never 0, so a
    /// defaulted `TextRevision` cannot be mistaken for a real one.
    pub const INITIAL: Self = Self(1);

    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl Default for TextRevision {
    fn default() -> Self {
        Self::INITIAL
    }
}

/// Monotonic font-database generation.
///
/// Bumped by every mutation of the face set: registration, removal, family
/// alias change, fallback-chain change, default-family override. A layout
/// carries the generation it was produced under, so staleness is an O(1)
/// comparison instead of a re-fingerprint of the text.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct FontGeneration(u64);

impl FontGeneration {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn bumped(self) -> Self {
        Self(self.0 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_handles_are_distinguishable_from_the_first_live_slot() {
        assert!(FontId::NULL.is_null());
        assert!(FontId::default().is_null());
        // Slot 0 at generation 1 is a real handle and must not read as null.
        let first = FontId::from_parts(0, 1);
        assert!(!first.is_null());
        assert_ne!(first, FontId::NULL);
    }

    #[test]
    fn a_reissued_slot_does_not_equal_the_handle_it_replaced() {
        let stale = ShapeRunId::from_parts(3, 1);
        let fresh = ShapeRunId::from_parts(3, 2);
        assert_eq!(stale.index(), fresh.index());
        assert_ne!(stale, fresh);
    }

    #[test]
    fn revisions_and_generations_only_move_forward() {
        assert_eq!(TextRevision::INITIAL.get(), 1);
        assert_eq!(TextRevision::INITIAL.next().get(), 2);
        assert!(TextRevision::INITIAL < TextRevision::INITIAL.next());
        assert_eq!(FontGeneration::default().get(), 0);
        assert_eq!(FontGeneration::default().bumped().get(), 1);
    }
}
