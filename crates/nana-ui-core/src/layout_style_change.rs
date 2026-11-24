//! What changed between two [`LayoutStyle`]s, by kind: the one place every
//! field is classified.
//!
//! Invalidation (does this write move boxes?), geometry-only detection (does
//! it move boxes and nothing else?) and containing-block classification all
//! read these kinds instead of keeping their own field lists. Both functions
//! below destructure the whole struct: a field added to [`LayoutStyle`]
//! without a kind here does not compile.

use crate::box_layout::{
    BorderSides, ContainerStyle, FlexDirection, GridStyle, LayoutStyle, LogicalBoxEdges,
    TextStyleExt, TransformStyle,
};
use crate::shared::Shared;

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
    /// Positioning scheme and insets.
    pub const POSITION: Self = Self(1 << 4);
    /// Alignment and justification.
    pub const ALIGNMENT: Self = Self(1 << 5);
    /// Every grid template, track and placement field.
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
    /// Whether the subtree is inert: no pointer, focus or accessibility
    /// actions. It moves no box and changes no pixel.
    pub const INERT: Self = Self(1 << 23);
    /// Non-style intrinsic metrics or content changes (text, replaced media,
    /// component parts) represented in the same invalidation mask.
    pub const INTRINSIC: Self = Self(1 << 24);
    /// Font face/metrics plus text layout properties.
    pub const TYPOGRAPHY: Self = Self(Self::FONT.0 | Self::TEXT_LAYOUT.0);
    /// Every field and intrinsic change group.
    pub const ALL: Self = Self((1 << 25) - 1);

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
            | Self::BORDER_STYLE.0
            | Self::INTRINSIC.0,
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

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
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

    pub fn names(self) -> impl Iterator<Item = &'static str> {
        const FLAGS: &[(LayoutStyleChange, &str)] = &[
            (LayoutStyleChange::WRITING, "writing"),
            (LayoutStyleChange::FLOW, "flow"),
            (LayoutStyleChange::SIZING, "sizing"),
            (LayoutStyleChange::SPACING, "spacing"),
            (LayoutStyleChange::POSITION, "position"),
            (LayoutStyleChange::ALIGNMENT, "alignment"),
            (LayoutStyleChange::GRID, "grid"),
            (LayoutStyleChange::TEXT_LAYOUT, "text_layout"),
            (LayoutStyleChange::SCROLL, "scroll"),
            (LayoutStyleChange::BORDER_GEOMETRY, "border_geometry"),
            (LayoutStyleChange::BORDER_STYLE, "border_style"),
            (LayoutStyleChange::FONT, "font"),
            (LayoutStyleChange::TRANSFORM, "transform"),
            (LayoutStyleChange::CURSOR, "cursor"),
            (LayoutStyleChange::INTERACTION, "interaction"),
            (LayoutStyleChange::STACKING, "stacking"),
            (LayoutStyleChange::VISIBILITY, "visibility"),
            (LayoutStyleChange::PAINT, "paint"),
            (LayoutStyleChange::SIZING_WIDTH, "sizing_width"),
            (LayoutStyleChange::SIZING_HEIGHT, "sizing_height"),
            (LayoutStyleChange::FLEX_FACTOR, "flex_factor"),
            (LayoutStyleChange::FLOW_SHAPE, "flow_shape"),
            (LayoutStyleChange::CONTAINER, "container"),
            (LayoutStyleChange::INERT, "inert"),
            (LayoutStyleChange::INTRINSIC, "intrinsic"),
        ];
        FLAGS
            .iter()
            .filter(move |(flag, _)| self.intersects(*flag))
            .map(|(_, name)| *name)
    }
}

/// `None` lays out as the default column axis: spelling it either way is no
/// change.
fn same_direction(left: &Option<FlexDirection>, right: &Option<FlexDirection>) -> bool {
    left.unwrap_or(FlexDirection::Column) == right.unwrap_or(FlexDirection::Column)
}

/// A set of [`LayoutStyle`] fields, one bit each: what a patch writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LayoutFieldSet([u64; 3]);

impl LayoutFieldSet {
    pub const EMPTY: Self = Self([0; 3]);

    pub const fn is_empty(&self) -> bool {
        self.0[0] == 0 && self.0[1] == 0 && self.0[2] == 0
    }

    /// How many fields are in the set.
    pub const fn len(&self) -> u32 {
        self.0[0].count_ones() + self.0[1].count_ones() + self.0[2].count_ones()
    }

    fn insert(&mut self, field: usize) {
        self.0[field / 64] |= 1 << (field % 64);
    }

    fn contains(&self, field: usize) -> bool {
        self.0[field / 64] & (1 << (field % 64)) != 0
    }
}

macro_rules! layout_style_fields {
    (
        top {
            $($field:ident: $kind:ident $(by $eq:path)?,)*
        }
        $(
            shared $group:ident : $group_ty:ident {
                $($gfield:ident => $variant:ident : $gkind:ident $(by $geq:path)?,)*
            }
        )*
    ) => {
        /// Each field's bit in a [`LayoutFieldSet`].
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy)]
        enum Field {
            $($field,)*
            $($($variant,)*)*
        }

        const _: () = assert!(
            [$(Field::$field,)* $($(Field::$variant,)*)*].len() <= 192,
            "LayoutFieldSet holds 192 fields"
        );

        impl LayoutStyle {
            /// The fields whose values differ between `self` and `other`,
            /// one by one: what a patch from `self` to `other` writes.
            pub fn differing_fields(&self, other: &Self) -> LayoutFieldSet {
                let Self { $($field,)* $($group,)* } = self;
                let mut fields = LayoutFieldSet::EMPTY;
                $(
                    if *$field != other.$field {
                        fields.insert(Field::$field as usize);
                    }
                )*
                $(
                    if !$group.ptr_eq(&other.$group) {
                        let $group_ty { $($gfield,)* } = &**$group;
                        $(
                            if *$gfield != other.$group.$gfield {
                                fields.insert(Field::$variant as usize);
                            }
                        )*
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
                $(
                    if !self.$group.ptr_eq(&other.$group) {
                        let any_diff = $(
                            fields.contains(Field::$variant as usize)
                                && self.$group.$gfield != other.$group.$gfield
                        )||*;
                        if any_diff {
                            {
                                let target = &mut *self.$group;
                                let source = &*other.$group;
                                $(
                                    if fields.contains(Field::$variant as usize) {
                                        target.$gfield = source.$gfield.clone();
                                    }
                                )*
                            }
                            if &*self.$group == &<$group_ty>::default() {
                                self.$group = Shared::empty();
                            }
                        }
                    }
                )*
            }

            /// The kinds of field that differ between `self` and `other`.
            pub fn changed_fields(&self, other: &Self) -> LayoutStyleChange {
                let Self { $($field,)* $($group,)* } = self;
                let mut changed = LayoutStyleChange::NONE;
                $(
                    if !layout_style_fields!(@eq $field, &other.$field $(, $eq)?) {
                        changed = changed.union(LayoutStyleChange::$kind);
                    }
                )*
                $(
                    if !$group.ptr_eq(&other.$group) {
                        let $group_ty { $($gfield,)* } = &**$group;
                        $(
                            if !layout_style_fields!(@eq $gfield, &other.$group.$gfield $(, $geq)?) {
                                changed = changed.union(LayoutStyleChange::$gkind);
                            }
                        )*
                    }
                )*
                // A box that leaves its parent's flow or joins it changes
                // what the parent measures, as `display` does; insets, and a
                // scheme that stays on one side, only place it.
                if self.position.is_out_of_flow() != other.position.is_out_of_flow() {
                    changed = changed.union(LayoutStyleChange::FLOW_STRUCTURE);
                }
                changed
            }

            /// `self` with every field of a kind in `kinds` taken from
            /// `other`.
            pub fn with_fields_of(&self, other: &Self, kinds: LayoutStyleChange) -> Self {
                let Self { $($field,)* $($group,)* } = self;
                Self {
                    $(
                        $field: if kinds.intersects(LayoutStyleChange::$kind) {
                            other.$field.clone()
                        } else {
                            $field.clone()
                        },
                    )*
                    $(
                        $group: if $group.ptr_eq(&other.$group) {
                            $group.clone()
                        } else {
                            let group_kinds = LayoutStyleChange::NONE
                                $(.union(LayoutStyleChange::$gkind))*;
                            if !kinds.intersects(group_kinds) {
                                $group.clone()
                            } else {
                                let merged = $group_ty {
                                    $(
                                        $gfield: if kinds.intersects(LayoutStyleChange::$gkind) {
                                            other.$group.$gfield.clone()
                                        } else {
                                            $group.$gfield.clone()
                                        },
                                    )*
                                };
                                if merged == <$group_ty>::default() {
                                    Shared::empty()
                                } else {
                                    Shared::new(merged)
                                }
                            }
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
    top {
        theme: NONE,
        paint_colors: PAINT,
        layout_isolation: FLOW_STRUCTURE,
        direction: WRITING by same_direction,
        dir: WRITING,
        writing_mode: WRITING,
        flex_reverse: FLOW_STRUCTURE,
        order: FLOW_STRUCTURE,
        flex_wrap: FLOW_STRUCTURE,
        equal_wrap: FLOW_STRUCTURE,
        display: FLOW_STRUCTURE,
        box_sizing: SIZING_BOTH,
        position: POSITION,
        anchor: POSITION,
        z_index: STACKING,
        isolation: STACKING,
        gap: SPACING,
        row_gap: SPACING,
        column_gap: SPACING,
        padding: SPACING,
        padding_top: SPACING,
        padding_right: SPACING,
        padding_bottom: SPACING,
        padding_left: SPACING,
        margin: SPACING,
        margin_top: SPACING,
        margin_right: SPACING,
        margin_bottom: SPACING,
        margin_left: SPACING,
        offset_top: POSITION,
        offset_right: POSITION,
        offset_bottom: POSITION,
        offset_left: POSITION,
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
        pointer_events: INTERACTION,
        inert: INERT,
        cursor: CURSOR,
        user_select: CURSOR,
        white_space: TEXT_LAYOUT,
        aspect_ratio: SIZING_BOTH,
        text_align: TEXT_LAYOUT,
        font_size: FONT,
        font_weight: FONT,
        font_italic: TEXT_LAYOUT,
        line_height: FONT,
        hidden: VISIBILITY,
        paint: PAINT,
        opacity: PAINT,
        border_radius: PAINT,
        border_width: BORDER_GEOMETRY,
        border_style: BORDER_STYLE,
    }
    shared transforms: TransformStyle {
        transform => transforms_transform: TRANSFORM,
        transform_3d => transforms_transform_3d: TRANSFORM,
        unsupported_transform => transforms_unsupported_transform: TRANSFORM,
        origin => transforms_origin: TRANSFORM,
        css_perspective => transforms_css_perspective: TRANSFORM,
        perspective_origin => transforms_perspective_origin: TRANSFORM,
        preserve_3d => transforms_preserve_3d: TRANSFORM,
        backface_visibility => transforms_backface_visibility: TRANSFORM,
        transform_box => transforms_transform_box: TRANSFORM,
    }
    shared logical_edges: LogicalBoxEdges {
        padding => logical_padding: SPACING,
        margin => logical_margin: SPACING,
        inset => logical_inset: POSITION,
    }
    shared text_ext: TextStyleExt {
        font_family => text_ext_font_family: FONT,
        letter_spacing => text_ext_letter_spacing: FONT,
        line_clamp => text_ext_line_clamp: TEXT_LAYOUT,
        word_break => text_ext_word_break: TEXT_LAYOUT,
        overflow_wrap => text_ext_overflow_wrap: TEXT_LAYOUT,
        text_decoration => text_ext_text_decoration: PAINT,
        text_emphasis_style => text_ext_text_emphasis_style: TEXT_LAYOUT,
        text_emphasis_position => text_ext_text_emphasis_position: TEXT_LAYOUT,
        text_emphasis_color => text_ext_text_emphasis_color: PAINT,
        font_features => text_ext_font_features: FONT,
        font_variation_settings => text_ext_font_variation_settings: FONT,
        font_kerning => text_ext_font_kerning: FONT,
        line_break => text_ext_line_break: TEXT_LAYOUT,
        text_typography => text_ext_text_typography: TEXT_LAYOUT,
        unsupported_font_variation => text_ext_unsupported_font_variation: FONT,
        unsupported_writing_mode => text_ext_unsupported_writing_mode: WRITING,
        text_orientation => text_ext_text_orientation: WRITING,
        placeholder_color => text_ext_placeholder_color: PAINT,
        placeholder_opacity => text_ext_placeholder_opacity: PAINT,
    }
    shared grid: GridStyle {
        columns => grid_columns: GRID,
        rows => grid_rows: GRID,
        columns_unsupported => grid_columns_unsupported: GRID,
        rows_unsupported => grid_rows_unsupported: GRID,
        auto_columns => grid_auto_columns: GRID,
        auto_rows => grid_auto_rows: GRID,
        auto_flow => grid_auto_flow: GRID,
        columns_repeat => grid_columns_repeat: GRID,
        rows_repeat => grid_rows_repeat: GRID,
        columns_subgrid => grid_columns_subgrid: GRID,
        rows_subgrid => grid_rows_subgrid: GRID,
        placement => grid_placement: GRID,
        template_areas => grid_template_areas: GRID,
        column_line_names => grid_column_line_names: GRID,
        row_line_names => grid_row_line_names: GRID,
    }
    shared border_sides: BorderSides {
        top_width => border_top_width: BORDER_GEOMETRY,
        right_width => border_right_width: BORDER_GEOMETRY,
        bottom_width => border_bottom_width: BORDER_GEOMETRY,
        left_width => border_left_width: BORDER_GEOMETRY,
        top_style => border_top_style: BORDER_STYLE,
        right_style => border_right_style: BORDER_STYLE,
        bottom_style => border_bottom_style: BORDER_STYLE,
        left_style => border_left_style: BORDER_STYLE,
    }
    shared container: ContainerStyle {
        container_type => container_type: CONTAINER,
        container_name => container_name: CONTAINER,
    }
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
        let styled = LayoutStyle::with(|s| {
            s.width = Some(LengthSpec::Px(10.0));
            s.paint_colors.background = Some(crate::PaintColor::srgb([1.0, 0.0, 0.0, 1.0]));
            s.container.container_name = vec!["card".into()];
        });
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

    /// Leaving the flow or joining it is a change of the flow, which the
    /// parent measures; moving between schemes on one side of it, or only
    /// the insets, places the box.
    #[test]
    fn leaving_or_joining_the_flow_is_a_flow_change() {
        use crate::PositionSpec;
        let at = |position| LayoutStyle {
            position,
            ..LayoutStyle::default()
        };
        for (from, to) in [
            (PositionSpec::Static, PositionSpec::Absolute),
            (PositionSpec::Absolute, PositionSpec::Static),
            (PositionSpec::Relative, PositionSpec::Fixed),
        ] {
            let changed = at(from).changed_fields(&at(to));
            assert!(
                changed.contains(LayoutStyleChange::FLOW_STRUCTURE),
                "{from:?} -> {to:?}"
            );
            assert!(changed.intersects(LayoutStyleChange::POSITION));
        }
        for (from, to) in [
            (PositionSpec::Static, PositionSpec::Relative),
            (PositionSpec::Absolute, PositionSpec::Fixed),
        ] {
            assert_eq!(
                at(from).changed_fields(&at(to)),
                LayoutStyleChange::POSITION,
                "{from:?} -> {to:?}"
            );
        }
        let placed = LayoutStyle {
            offset_top: Some(LengthSpec::Px(4.0)),
            ..at(PositionSpec::Absolute)
        };
        assert_eq!(
            at(PositionSpec::Absolute).changed_fields(&placed),
            LayoutStyleChange::POSITION
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
    fn logical_edges_and_isolation_are_layout() {
        let base = LayoutStyle::default();
        let changed = LayoutStyle {
            layout_isolation: true,
            ..LayoutStyle::default()
        };
        let changes = base.changed_fields(&changed);
        assert_eq!(changes, LayoutStyleChange::FLOW_STRUCTURE);
        assert!(changes.intersects(LayoutStyleChange::LAYOUT));
    }
}
