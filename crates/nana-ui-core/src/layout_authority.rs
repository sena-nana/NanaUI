//! Backend-neutral metadata for the single layout intent/resolution authority.
//!
//! The mask is intentionally independent from `nana-ui-css`: CSS, Vue
//! semantic projection, and Rust L3 can all describe the same ownership
//! contract without importing a parser or a runtime tree.

use crate::FlexDirection;

/// Origin of a layout value before it is resolved into a retained node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutOrigin {
    ComponentRequired,
    ComponentDefault,
    Author,
    RuntimeConstraint,
    LayoutOutput,
    ScrollProjection,
}

/// Coarse field groups used for ownership and invalidation. A group is the
/// smallest stable public unit needed by the current layout engine; individual
/// CSS longhands remain parser concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutFieldMask(u128);

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

    pub const fn bits(self) -> u128 {
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

    /// Classify a CSS longhand (or the corresponding L3 field name).
    pub fn from_property(property: &str) -> Self {
        let property = property.trim().to_ascii_lowercase();
        let group = if property.contains("font")
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
        } else if property == "pointer-events" || property == "cursor" || property == "user-select"
        {
            Self::INTERACTION
        } else {
            Self::INTRINSIC
        };
        group
    }
}

/// Typed ownership for a node's layout intent. Required fields win over
/// author intent; default fields are author-overridable by the resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutOwnership {
    pub required: LayoutFieldMask,
    pub defaults: LayoutFieldMask,
}

impl LayoutOwnership {
    pub const fn component_default(mask: LayoutFieldMask) -> Self {
        Self {
            required: LayoutFieldMask::NONE,
            defaults: mask,
        }
    }

    pub const fn component_required(mask: LayoutFieldMask) -> Self {
        Self {
            required: mask,
            defaults: LayoutFieldMask::NONE,
        }
    }

    pub const fn allows_author(self, fields: LayoutFieldMask) -> LayoutFieldMask {
        LayoutFieldMask(fields.bits() & !self.required.bits())
    }
}

/// Minimal component intent used by adapters that need to seed the resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutIntent {
    pub origin: LayoutOrigin,
    pub ownership: LayoutOwnership,
    pub default_direction: Option<FlexDirection>,
}

impl LayoutIntent {
    pub const fn component_default(ownership: LayoutOwnership) -> Self {
        Self {
            origin: LayoutOrigin::ComponentDefault,
            ownership,
            default_direction: None,
        }
    }

    pub const fn row_default() -> Self {
        Self {
            origin: LayoutOrigin::ComponentDefault,
            ownership: LayoutOwnership::component_default(LayoutFieldMask::FLOW),
            default_direction: Some(FlexDirection::Row),
        }
    }
}

/// Resolve a component layout against author intent using the typed ownership
/// contract. The JSON representation is used only at mutation time; it keeps
/// this adapter independent from the large and evolving `LayoutStyle` field
/// list while preserving exact field values and defaults.
pub fn resolve_layout_intent(
    component: &crate::LayoutStyle,
    author: &crate::LayoutStyle,
    ownership: LayoutOwnership,
    author_fields: LayoutFieldMask,
) -> crate::LayoutStyle {
    let mut resolved = serde_json::to_value(component).expect("layout serializes");
    let authored = serde_json::to_value(author).expect("layout serializes");
    let groups = [
        (
            LayoutFieldMask::FLOW,
            &[
                "direction",
                "dir",
                "flex_reverse",
                "order",
                "flex_wrap",
                "display",
            ][..],
        ),
        (
            LayoutFieldMask::SIZING,
            &[
                "box_sizing",
                "width",
                "height",
                "min_width",
                "max_width",
                "min_height",
                "max_height",
                "allow_shrink",
                "aspect_ratio",
            ][..],
        ),
        (
            LayoutFieldMask::SPACING,
            &[
                "gap",
                "row_gap",
                "column_gap",
                "padding",
                "padding_top",
                "padding_right",
                "padding_bottom",
                "padding_left",
                "margin",
                "margin_top",
                "margin_right",
                "margin_bottom",
                "margin_left",
                "offset_top",
                "offset_right",
                "offset_bottom",
                "offset_left",
                "logical_padding",
                "logical_margin",
                "logical_inset",
            ][..],
        ),
        (
            LayoutFieldMask::POSITION,
            &["position", "z_index", "isolation", "float", "clear"][..],
        ),
        (
            LayoutFieldMask::ALIGNMENT,
            &[
                "align_items",
                "align_self",
                "align_content",
                "justify_content",
                "justify_items",
                "justify_self",
                "flex_grow",
                "flex_shrink",
                "flex_basis",
            ][..],
        ),
        (
            LayoutFieldMask::GRID,
            &[
                "grid_columns",
                "grid_rows",
                "grid_columns_unsupported",
                "grid_rows_unsupported",
                "grid_auto_columns",
                "grid_auto_rows",
                "grid_auto_flow",
                "grid_columns_repeat",
                "grid_rows_repeat",
                "grid_placement",
                "grid_template_areas",
                "grid_column_line_names",
                "grid_row_line_names",
                "grid_columns_subgrid",
                "grid_rows_subgrid",
            ][..],
        ),
        (
            LayoutFieldMask::TYPOGRAPHY,
            &[
                "font_size",
                "font_weight",
                "font_italic",
                "font_family",
                "line_height",
                "letter_spacing",
                "font_features",
                "font_variation_settings",
                "font_kerning",
                "line_break",
                "text_orientation",
                "text_align",
                "white_space",
                "white_space_nowrap",
                "word_break",
                "overflow_wrap",
                "text_decoration",
            ][..],
        ),
        (
            LayoutFieldMask::PAINT,
            &[
                "paint",
                "opacity",
                "background",
                "border_radius",
                "border_width",
                "border_top_width",
                "border_right_width",
                "border_bottom_width",
                "border_left_width",
                "border_color",
                "border_top_color",
                "border_right_color",
                "border_bottom_color",
                "border_left_color",
                "border_style",
                "border_top_style",
                "border_right_style",
                "border_bottom_style",
                "border_left_style",
                "color",
                "placeholder_color",
                "placeholder_opacity",
                "selection_background",
                "selection_color",
            ][..],
        ),
        (LayoutFieldMask::VISIBILITY, &["hidden"][..]),
        (
            LayoutFieldMask::INTERACTION,
            &["pointer_events", "cursor", "user_select"][..],
        ),
        (
            LayoutFieldMask::TRANSFORM,
            &[
                "transform",
                "transform_3d",
                "unsupported_transform",
                "transform_origin",
                "css_perspective",
                "preserve_3d",
                "transform_box",
            ][..],
        ),
        (
            LayoutFieldMask::SCROLL,
            &[
                "overflow_x",
                "overflow_y",
                "text_overflow_ellipsis",
                "line_clamp",
            ][..],
        ),
        (
            LayoutFieldMask::INTRINSIC,
            &[
                "layout_isolation",
                "writing_mode",
                "unsupported_writing_mode",
                "unsupported_font_variation",
            ][..],
        ),
    ];
    for (group, fields) in groups {
        if ownership.required.intersects(group) || !author_fields.intersects(group) {
            continue;
        }
        for field in fields {
            if let (Some(target), Some(source)) = (resolved.get_mut(field), authored.get(field)) {
                *target = source.clone();
            }
        }
    }
    serde_json::from_value(resolved).expect("resolved layout deserializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_can_override_component_default_but_not_required_fields() {
        let ownership = LayoutOwnership::component_default(LayoutFieldMask::FLOW);
        assert!(
            ownership
                .allows_author(LayoutFieldMask::FLOW)
                .contains(LayoutFieldMask::FLOW)
        );

        let required = LayoutOwnership::component_required(LayoutFieldMask::FLOW);
        assert!(
            !required
                .allows_author(LayoutFieldMask::FLOW)
                .intersects(LayoutFieldMask::FLOW)
        );
    }

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

    #[test]
    fn resolver_preserves_component_defaults_without_author_fields() {
        let mut component = crate::LayoutStyle::default();
        component.direction = Some(FlexDirection::Row);
        let mut author = crate::LayoutStyle::default();
        author.direction = Some(FlexDirection::Column);
        let resolved = resolve_layout_intent(
            &component,
            &author,
            LayoutOwnership::component_default(LayoutFieldMask::FLOW),
            LayoutFieldMask::NONE,
        );
        assert_eq!(resolved.direction, Some(FlexDirection::Row));
    }

    #[test]
    fn resolver_applies_author_fields_and_protects_required_fields() {
        let mut component = crate::LayoutStyle::default();
        component.direction = Some(FlexDirection::Row);
        let mut author = crate::LayoutStyle::default();
        author.direction = Some(FlexDirection::Column);
        let resolved = resolve_layout_intent(
            &component,
            &author,
            LayoutOwnership::component_default(LayoutFieldMask::FLOW),
            LayoutFieldMask::FLOW,
        );
        assert_eq!(resolved.direction, Some(FlexDirection::Column));

        let required = resolve_layout_intent(
            &component,
            &author,
            LayoutOwnership::component_required(LayoutFieldMask::FLOW),
            LayoutFieldMask::FLOW,
        );
        assert_eq!(required.direction, Some(FlexDirection::Row));
    }
}
