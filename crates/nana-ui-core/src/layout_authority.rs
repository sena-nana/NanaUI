//! Backend-neutral metadata for layout field groups.
//!
//! The mask is intentionally independent from `nana-ui-css`: CSS, semantic
//! projection, and Rust L3 can all describe the same ownership
//! contract without importing a parser or a runtime tree.

/// `names(self)` for a set of flag constants, from a `FLAG: "name"` table:
/// the name of every flag the set holds, in table order.
macro_rules! flag_names {
    ($($flag:ident: $name:literal),* $(,)?) => {
        /// Names of the set flags, for diagnostics.
        pub fn names(self) -> impl Iterator<Item = &'static str> {
            [$((Self::$flag, $name)),*]
                .into_iter()
                .filter(move |(flag, _)| self.contains(*flag))
                .map(|(_, name)| name)
        }
    };
}
pub(crate) use flag_names;

/// Coarse field groups used for ownership and invalidation. A group is the
/// smallest stable public unit needed by the current layout engine; individual
/// CSS longhands remain parser concerns.
///
/// Every typed layout invalidation carries one of these, and a node holds
/// at most one pending invalidation, so the width is per-node metadata: 32
/// bits leave room for new groups without doubling that slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutFieldMask(u32);

impl LayoutFieldMask {
    pub const NONE: Self = Self(0);
    pub const FLOW: Self = Self(1 << 0);
    pub const SIZING: Self = Self(1 << 1);
    pub const SPACING: Self = Self(1 << 2);
    pub const POSITION: Self = Self(1 << 3);
    pub const ALIGNMENT: Self = Self(1 << 4);
    pub const GRID: Self = Self(1 << 5);
    pub const TYPOGRAPHY: Self = Self(1 << 6);
    pub const PAINT: Self = Self(1 << 7);
    pub const VISIBILITY: Self = Self(1 << 8);
    pub const INTERACTION: Self = Self(1 << 9);
    pub const TRANSFORM: Self = Self(1 << 10);
    pub const SCROLL: Self = Self(1 << 11);
    pub const INTRINSIC: Self = Self(1 << 12);
    pub const ALL: Self = Self((1 << 13) - 1);

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    flag_names! {
        FLOW: "flow",
        SIZING: "sizing",
        SPACING: "spacing",
        POSITION: "position",
        ALIGNMENT: "alignment",
        GRID: "grid",
        TYPOGRAPHY: "typography",
        PAINT: "paint",
        VISIBILITY: "visibility",
        INTERACTION: "interaction",
        TRANSFORM: "transform",
        SCROLL: "scroll",
        INTRINSIC: "intrinsic",
    }

    /// Classify a CSS longhand (or the corresponding L3 field name).
    pub fn from_property(property: &str) -> Self {
        let property = property.trim().to_ascii_lowercase();
        let property = if property.starts_with("grid.") {
            return Self::GRID;
        } else if property.starts_with("transforms.") {
            return Self::TRANSFORM;
        } else if property.starts_with("logical_edges.") {
            return if property.contains("inset") {
                Self::POSITION
            } else {
                Self::SPACING
            };
        } else if property.starts_with("border_sides.") {
            return Self::PAINT;
        } else if property.starts_with("container.") {
            return Self::INTRINSIC;
        } else if let Some(rest) = property.strip_prefix("text_ext.") {
            rest
        } else {
            property.as_str()
        };
        if property.contains("font")
            || property.contains("text")
            || property.contains("line-height")
            || property.contains("letter-spacing")
        {
            Self::TYPOGRAPHY
        } else if property.contains("background")
            || property.contains("border")
            || property.contains("shadow")
            || property.contains("filter")
            || property.contains("outline")
            || property == "color"
        {
            Self::PAINT
        } else if property.contains("transform") || property == "perspective" {
            Self::TRANSFORM
        } else if property.contains("grid") {
            Self::GRID
        } else if property.contains("align") || property.contains("justify") {
            Self::ALIGNMENT
        } else if property.contains("padding") || property.contains("margin") || property == "gap" {
            Self::SPACING
        } else if property.contains("width")
            || property.contains("height")
            || property == "aspect-ratio"
            || property == "box-sizing"
        {
            Self::SIZING
        } else if property == "position"
            || property == "top"
            || property == "right"
            || property == "bottom"
            || property == "left"
            || property == "inset"
            || property == "z-index"
        {
            Self::POSITION
        } else if property == "display"
            || property == "flex"
            || property == "flex-direction"
            || property == "flex-wrap"
            || property == "order"
        {
            Self::FLOW
        } else if property == "overflow" || property.starts_with("scroll") {
            Self::SCROLL
        } else if property == "opacity" || property == "visibility" {
            Self::VISIBILITY
        } else if property == "pointer-events"
            || property == "interactivity"
            || property == "inert"
            || property == "cursor"
            || property == "user-select"
        {
            Self::INTERACTION
        } else {
            Self::INTRINSIC
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_property_groups_are_stable() {
        assert!(LayoutFieldMask::from_property("flex-direction").contains(LayoutFieldMask::FLOW));
        assert!(
            LayoutFieldMask::from_property("padding-inline").contains(LayoutFieldMask::SPACING)
        );
        assert!(
            LayoutFieldMask::from_property("background-color").contains(LayoutFieldMask::PAINT)
        );
    }
}
