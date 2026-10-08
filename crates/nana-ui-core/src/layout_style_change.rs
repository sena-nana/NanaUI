//! What changed between two [`LayoutStyle`]s, by kind: the one place every
//! field is classified.
//!
//! Invalidation (does this write move boxes?), geometry-only detection (does
//! it move boxes and nothing else?) and containing-block classification all
//! read these kinds instead of keeping their own field lists. Both functions
//! below destructure the whole struct: a field added to [`LayoutStyle`]
//! without a kind here does not compile.

use crate::box_layout::{FlexDirection, LayoutStyle};

/// A set of [`LayoutStyle`] field kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LayoutStyleChange(u32);

impl LayoutStyleChange {
    pub const NONE: Self = Self(0);
    /// Main axis, direction, writing mode and text orientation.
    pub const WRITING: Self = Self(1 << 0);
    /// Display, flex order and sizing factors, wrapping, layout isolation.
    pub const FLOW: Self = Self(1 << 1);
    /// Width, height, their limits, box sizing, aspect ratio.
    pub const SIZING: Self = Self(1 << 2);
    /// Gaps, padding and margin, physical and logical.
    pub const SPACING: Self = Self(1 << 3);
    /// Positioning scheme, insets, float and clear.
    pub const POSITION: Self = Self(1 << 4);
    /// Alignment and justification.
    pub const ALIGNMENT: Self = Self(1 << 5);
    /// Every grid template, track, placement and subgrid field.
    pub const GRID: Self = Self(1 << 6);
    /// How text wraps, breaks, aligns, ends and clamps.
    pub const TEXT_LAYOUT: Self = Self(1 << 7);
    /// Overflow on either axis.
    pub const SCROLL: Self = Self(1 << 8);
    /// Border widths: they take room.
    pub const BORDER_GEOMETRY: Self = Self(1 << 9);
    /// Border styles: `none` takes no room, and paint reads every style.
    pub const BORDER_STYLE: Self = Self(1 << 10);
    /// Font face, size, weight, line height and spacing. Text measurement
    /// tracks these on its own path.
    pub const FONT: Self = Self(1 << 11);
    /// 2D and 3D transforms and their origin, box and perspective.
    pub const TRANSFORM: Self = Self(1 << 12);
    /// Cursor and text selection policy.
    pub const CURSOR: Self = Self(1 << 13);
    /// Whether the node takes the pointer.
    pub const INTERACTION: Self = Self(1 << 14);
    /// Stacking order and isolation.
    pub const STACKING: Self = Self(1 << 15);
    /// Hidden from paint, hit testing and accessibility.
    pub const VISIBILITY: Self = Self(1 << 16);
    /// Colours, opacity, radius and the rest of paint.
    pub const PAINT: Self = Self(1 << 17);

    /// Kinds that move or resize boxes.
    pub const LAYOUT: Self = Self(
        Self::WRITING.0
            | Self::FLOW.0
            | Self::SIZING.0
            | Self::SPACING.0
            | Self::POSITION.0
            | Self::ALIGNMENT.0
            | Self::GRID.0
            | Self::TEXT_LAYOUT.0
            | Self::SCROLL.0
            | Self::BORDER_GEOMETRY.0
            | Self::BORDER_STYLE.0,
    );
    /// Layout kinds paint does not also read off the style: a change of only
    /// these moves boxes and changes nothing else about the pixels.
    pub const GEOMETRY: Self = Self(
        Self::WRITING.0
            | Self::FLOW.0
            | Self::SIZING.0
            | Self::SPACING.0
            | Self::POSITION.0
            | Self::ALIGNMENT.0
            | Self::GRID.0
            | Self::SCROLL.0,
    );
    /// Layout kinds a positioned or in-flow child's containing block reads.
    pub const CONTAINING_BLOCK: Self = Self(Self::LAYOUT.0 & !Self::TEXT_LAYOUT.0);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// `None` lays out as the default column axis: spelling it either way is no
/// change.
fn same_direction(left: &Option<FlexDirection>, right: &Option<FlexDirection>) -> bool {
    left.unwrap_or(FlexDirection::Column) == right.unwrap_or(FlexDirection::Column)
}

macro_rules! layout_style_fields {
    ($($field:ident: $kind:ident $(by $eq:path)?,)*) => {
        impl LayoutStyle {
            /// The kinds of field that differ between `self` and `other`.
            pub fn changed_fields(&self, other: &Self) -> LayoutStyleChange {
                let Self { $($field),* } = self;
                let mut changed = LayoutStyleChange::NONE;
                $(
                    if !layout_style_fields!(@eq $field, &other.$field $(, $eq)?) {
                        changed = changed.union(LayoutStyleChange::$kind);
                    }
                )*
                changed
            }

            /// `self` with every field of a kind in `kinds` taken from
            /// `other`.
            pub fn with_fields_of(&self, other: &Self, kinds: LayoutStyleChange) -> Self {
                let Self { $($field),* } = self;
                Self {
                    $(
                        $field: if kinds.intersects(LayoutStyleChange::$kind) {
                            other.$field.clone()
                        } else {
                            $field.clone()
                        },
                    )*
                }
            }
        }
    };
    (@eq $left:expr, $right:expr) => { $left == $right };
    (@eq $left:expr, $right:expr, $eq:path) => { $eq($left, $right) };
}

layout_style_fields! {
    paint_colors: PAINT,
    layout_isolation: FLOW,
    direction: WRITING by same_direction,
    dir: WRITING,
    writing_mode: WRITING,
    unsupported_writing_mode: WRITING,
    text_orientation: WRITING,
    flex_reverse: FLOW,
    order: FLOW,
    flex_wrap: FLOW,
    display: FLOW,
    box_sizing: SIZING,
    position: POSITION,
    z_index: STACKING,
    isolation: STACKING,
    transform: TRANSFORM,
    transform_3d: TRANSFORM,
    unsupported_transform: TRANSFORM,
    transform_origin: TRANSFORM,
    css_perspective: TRANSFORM,
    preserve_3d: TRANSFORM,
    transform_box: TRANSFORM,
    gap: SPACING,
    row_gap: SPACING,
    column_gap: SPACING,
    padding: SPACING,
    padding_top: SPACING,
    padding_right: SPACING,
    padding_bottom: SPACING,
    padding_left: SPACING,
    logical_padding: SPACING,
    margin: SPACING,
    margin_top: SPACING,
    margin_right: SPACING,
    margin_bottom: SPACING,
    margin_left: SPACING,
    logical_margin: SPACING,
    offset_top: POSITION,
    offset_right: POSITION,
    offset_bottom: POSITION,
    offset_left: POSITION,
    logical_inset: POSITION,
    width: SIZING,
    height: SIZING,
    min_width: SIZING,
    max_width: SIZING,
    min_height: SIZING,
    max_height: SIZING,
    allow_shrink: SIZING,
    align_items: ALIGNMENT,
    align_self: ALIGNMENT,
    align_content: ALIGNMENT,
    justify_content: ALIGNMENT,
    justify_items: ALIGNMENT,
    justify_self: ALIGNMENT,
    flex_grow: FLOW,
    flex_shrink: FLOW,
    flex_basis: FLOW,
    overflow_x: SCROLL,
    overflow_y: SCROLL,
    text_overflow_ellipsis: TEXT_LAYOUT,
    line_clamp: TEXT_LAYOUT,
    pointer_events: INTERACTION,
    cursor: CURSOR,
    user_select: CURSOR,
    white_space_nowrap: TEXT_LAYOUT,
    white_space: TEXT_LAYOUT,
    word_break: TEXT_LAYOUT,
    overflow_wrap: TEXT_LAYOUT,
    aspect_ratio: SIZING,
    text_align: TEXT_LAYOUT,
    float: POSITION,
    clear: POSITION,
    font_size: FONT,
    font_weight: FONT,
    font_italic: TEXT_LAYOUT,
    font_family: FONT,
    line_height: FONT,
    letter_spacing: FONT,
    color: PAINT,
    text_decoration: PAINT,
    font_features: FONT,
    font_variation_settings: FONT,
    font_kerning: FONT,
    line_break: TEXT_LAYOUT,
    unsupported_font_variation: FONT,
    placeholder_color: PAINT,
    placeholder_opacity: PAINT,
    selection_background: PAINT,
    selection_color: PAINT,
    grid_columns: GRID,
    grid_rows: GRID,
    grid_columns_unsupported: GRID,
    grid_rows_unsupported: GRID,
    grid_auto_columns: GRID,
    grid_auto_rows: GRID,
    grid_auto_flow: GRID,
    grid_columns_repeat: GRID,
    grid_rows_repeat: GRID,
    grid_columns_subgrid: GRID,
    grid_rows_subgrid: GRID,
    grid_placement: GRID,
    grid_template_areas: GRID,
    grid_column_line_names: GRID,
    grid_row_line_names: GRID,
    hidden: VISIBILITY,
    paint: PAINT,
    opacity: PAINT,
    background: PAINT,
    border_radius: PAINT,
    border_width: BORDER_GEOMETRY,
    border_top_width: BORDER_GEOMETRY,
    border_right_width: BORDER_GEOMETRY,
    border_bottom_width: BORDER_GEOMETRY,
    border_left_width: BORDER_GEOMETRY,
    border_color: PAINT,
    border_top_color: PAINT,
    border_right_color: PAINT,
    border_bottom_color: PAINT,
    border_left_color: PAINT,
    border_style: BORDER_STYLE,
    border_top_style: BORDER_STYLE,
    border_right_style: BORDER_STYLE,
    border_bottom_style: BORDER_STYLE,
    border_left_style: BORDER_STYLE,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LengthSpec;

    #[test]
    fn each_field_reports_its_kind_and_copies_by_kind() {
        let base = LayoutStyle::default();
        let mut wider = base.clone();
        wider.width = Some(LengthSpec::Px(10.0));
        wider.cursor = Some(crate::CursorSpec::Pointer);
        assert_eq!(
            base.changed_fields(&wider),
            LayoutStyleChange::SIZING.union(LayoutStyleChange::CURSOR)
        );
        let copied = base.with_fields_of(&wider, LayoutStyleChange::SIZING);
        assert_eq!(copied.width, Some(LengthSpec::Px(10.0)));
        assert_eq!(copied.cursor, None);
        assert!(base.changed_fields(&base).is_empty());
    }

    #[test]
    fn a_respelled_default_axis_is_no_change() {
        let base = LayoutStyle::default();
        let spelled = LayoutStyle {
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        };
        assert!(base.changed_fields(&spelled).is_empty());
    }

    /// The fields the old hand-written lists missed are layout now.
    #[test]
    fn logical_edges_subgrid_and_isolation_are_layout() {
        let base = LayoutStyle::default();
        for (changed, kind) in [
            (
                LayoutStyle {
                    grid_columns_subgrid: true,
                    ..LayoutStyle::default()
                },
                LayoutStyleChange::GRID,
            ),
            (
                LayoutStyle {
                    layout_isolation: true,
                    ..LayoutStyle::default()
                },
                LayoutStyleChange::FLOW,
            ),
        ] {
            let changes = base.changed_fields(&changed);
            assert_eq!(changes, kind);
            assert!(changes.intersects(LayoutStyleChange::LAYOUT));
        }
    }
}
