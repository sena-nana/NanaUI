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
    /// Of [`Self::SIZING`], a field that sizes the box horizontally.
    pub const SIZING_WIDTH: Self = Self(1 << 18);
    /// Of [`Self::SIZING`], a field that sizes the box vertically.
    pub const SIZING_HEIGHT: Self = Self(1 << 19);
    /// Width and its limits.
    pub const WIDTH: Self = Self(Self::SIZING.0 | Self::SIZING_WIDTH.0);
    /// Height and its limits.
    pub const HEIGHT: Self = Self(Self::SIZING.0 | Self::SIZING_HEIGHT.0);
    /// A sizing field of neither axis alone -- box sizing, aspect ratio --
    /// sizes the box on both.
    pub const SIZING_BOTH: Self =
        Self(Self::SIZING.0 | Self::SIZING_WIDTH.0 | Self::SIZING_HEIGHT.0);
    /// Of [`Self::FLOW`], a field that sizes a flex item along its parent's
    /// line: grow, shrink and basis.
    pub const FLEX_FACTOR: Self = Self(1 << 20);
    /// Of [`Self::FLOW`], a field that shapes the flow itself: display,
    /// order, wrapping, direction of the line, isolation.
    pub const FLOW_SHAPE: Self = Self(1 << 21);
    /// Grow, shrink and basis.
    pub const FLEX_SIZING: Self = Self(Self::FLOW.0 | Self::FLEX_FACTOR.0);
    /// Display, order, wrapping, line direction, isolation.
    pub const FLOW_STRUCTURE: Self = Self(Self::FLOW.0 | Self::FLOW_SHAPE.0);
    /// Whether and by which names the box answers container-size queries.
    /// It moves no box; the rules that query a container find it again.
    pub const CONTAINER: Self = Self(1 << 22);

    /// Kinds that move or resize boxes.
    pub const LAYOUT: Self = Self(
        Self::WRITING.0
            | Self::FLOW.0
            | Self::SIZING.0
            | Self::SIZING_WIDTH.0
            | Self::SIZING_HEIGHT.0
            | Self::FLEX_FACTOR.0
            | Self::FLOW_SHAPE.0
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
            | Self::SIZING_WIDTH.0
            | Self::SIZING_HEIGHT.0
            | Self::FLEX_FACTOR.0
            | Self::FLOW_SHAPE.0
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

/// A set of [`LayoutStyle`] fields, one bit each: what a patch writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LayoutFieldSet([u64; 2]);

impl LayoutFieldSet {
    pub const EMPTY: Self = Self([0; 2]);

    pub const fn is_empty(&self) -> bool {
        self.0[0] == 0 && self.0[1] == 0
    }

    /// How many fields are in the set.
    pub const fn len(&self) -> u32 {
        self.0[0].count_ones() + self.0[1].count_ones()
    }

    fn insert(&mut self, field: usize) {
        self.0[field / 64] |= 1 << (field % 64);
    }

    fn contains(&self, field: usize) -> bool {
        self.0[field / 64] & (1 << (field % 64)) != 0
    }
}

macro_rules! layout_style_fields {
    ($($field:ident: $kind:ident $(by $eq:path)?,)*) => {
        /// Each field's bit in a [`LayoutFieldSet`].
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy)]
        enum Field {
            $($field,)*
        }

        const _: () = assert!(
            [$(Field::$field),*].len() <= 128,
            "LayoutFieldSet holds 128 fields"
        );

        impl LayoutStyle {
            /// The fields whose values differ between `self` and `other`,
            /// one by one: what a patch from `self` to `other` writes.
            pub fn differing_fields(&self, other: &Self) -> LayoutFieldSet {
                let Self { $($field),* } = self;
                let mut fields = LayoutFieldSet::EMPTY;
                $(
                    if *$field != other.$field {
                        fields.insert(Field::$field as usize);
                    }
                )*
                fields
            }

            /// Write the fields in `fields` from `other` over `self`'s.
            pub fn copy_fields(&mut self, other: &Self, fields: &LayoutFieldSet) {
                $(
                    if fields.contains(Field::$field as usize) {
                        self.$field = other.$field.clone();
                    }
                )*
            }

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
    layout_isolation: FLOW_STRUCTURE,
    direction: WRITING by same_direction,
    dir: WRITING,
    writing_mode: WRITING,
    unsupported_writing_mode: WRITING,
    text_orientation: WRITING,
    flex_reverse: FLOW_STRUCTURE,
    order: FLOW_STRUCTURE,
    flex_wrap: FLOW_STRUCTURE,
    display: FLOW_STRUCTURE,
    box_sizing: SIZING_BOTH,
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
    width: WIDTH,
    height: HEIGHT,
    min_width: WIDTH,
    max_width: WIDTH,
    min_height: HEIGHT,
    max_height: HEIGHT,
    allow_shrink: SIZING_BOTH,
    align_items: ALIGNMENT,
    align_self: ALIGNMENT,
    align_content: ALIGNMENT,
    justify_content: ALIGNMENT,
    justify_items: ALIGNMENT,
    justify_self: ALIGNMENT,
    flex_grow: FLEX_SIZING,
    flex_shrink: FLEX_SIZING,
    flex_basis: FLEX_SIZING,
    adaptation: FLEX_SIZING,
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
    aspect_ratio: SIZING_BOTH,
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
    container_type: CONTAINER,
    container_name: CONTAINER,
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
            LayoutStyleChange::WIDTH.union(LayoutStyleChange::CURSOR)
        );
        let copied = base.with_fields_of(&wider, LayoutStyleChange::SIZING);
        assert_eq!(copied.width, Some(LengthSpec::Px(10.0)));
        assert_eq!(copied.cursor, None);
        assert!(base.changed_fields(&base).is_empty());
    }

    /// A patch is the fields that differ, value for value: copied over the
    /// base it was taken against, it gives the styled layout back, and it
    /// writes nothing else over another base.
    #[test]
    fn differing_fields_copy_back_field_by_field() {
        let base = LayoutStyle::default();
        let styled = LayoutStyle {
            width: Some(LengthSpec::Px(10.0)),
            background: Some([1.0, 0.0, 0.0, 1.0]),
            container_name: vec!["card".into()],
            ..LayoutStyle::default()
        };
        let fields = base.differing_fields(&styled);
        assert_eq!(fields.len(), 3);
        assert!(base.differing_fields(&base).is_empty());
        let mut patched = base.clone();
        patched.copy_fields(&styled, &fields);
        assert_eq!(patched, styled);
        let mut other = LayoutStyle {
            height: Some(LengthSpec::Px(4.0)),
            ..LayoutStyle::default()
        };
        other.copy_fields(&styled, &fields);
        assert_eq!(other.height, Some(LengthSpec::Px(4.0)));
        assert_eq!(other.width, Some(LengthSpec::Px(10.0)));
        assert!(
            base.changed_fields(&styled)
                .intersects(LayoutStyleChange::CONTAINER)
        );
    }

    /// A sizing field says which axis it sizes; one of neither sizes both.
    #[test]
    fn sizing_fields_report_their_axis() {
        let base = LayoutStyle::default();
        let taller = LayoutStyle {
            max_height: Some(LengthSpec::Px(10.0)),
            ..base.clone()
        };
        let changed = base.changed_fields(&taller);
        assert!(changed.intersects(LayoutStyleChange::SIZING));
        assert!(changed.intersects(LayoutStyleChange::SIZING_HEIGHT));
        assert!(!changed.intersects(LayoutStyleChange::SIZING_WIDTH));
        let ratio = LayoutStyle {
            aspect_ratio: Some(2.0),
            ..base.clone()
        };
        assert_eq!(base.changed_fields(&ratio), LayoutStyleChange::SIZING_BOTH);
        let growing = LayoutStyle {
            flex_grow: Some(1.0),
            ..base.clone()
        };
        assert_eq!(
            base.changed_fields(&growing),
            LayoutStyleChange::FLEX_SIZING
        );
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
                LayoutStyleChange::FLOW_STRUCTURE,
            ),
        ] {
            let changes = base.changed_fields(&changed);
            assert_eq!(changes, kind);
            assert!(changes.intersects(LayoutStyleChange::LAYOUT));
        }
    }
}
