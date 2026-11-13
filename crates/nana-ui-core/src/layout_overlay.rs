//! One [`LayoutStyle`] written over another, field by field.
//!
//! A component's style is built from layers (a recipe's, a variant's, what
//! the application set). Laying one over another keeps the lower layer's
//! value wherever the upper one left a field at its default, down to the
//! fields of the nested groups (`paint.outline.width`, `theme.gap`).
//!
//! This used to go through `serde_json`: both layouts serialized, the trees
//! merged, the result deserialized. Every application then linked the whole
//! Style Model's `Serialize` and `Deserialize` for it (about 0.45 MiB of a
//! hosted binary). The fields are named here instead. Each struct is listed
//! exhaustively: a field added to one does not compile until it is listed,
//! so it cannot be silently left out of an overlay.

use crate::shared::{Shared, SharedDefault};
use crate::style_model::PaintColorSlots;
use crate::theme::ThemeLayout;
use crate::typography::TextTypography;
use crate::{GridPlacement, LayoutStyle, LogicalEdges, OutlineSpec, PaintStyle};

/// A struct of the Style Model whose fields overlay one by one.
trait Overlay: Sized {
    /// How many leaf fields this struct holds, its nested structs' included.
    const LEAVES: usize;

    /// Write over `self` each field `over` holds away from `default`.
    fn overlay(&mut self, default: &Self, over: &Self);

    /// The dotted path of each leaf field, under `prefix`.
    fn leaf_paths(prefix: &str, out: &mut Vec<String>);
}

/// A nested struct held in place.
fn plain<T: Overlay + PartialEq>(base: &mut T, default: &T, over: &T) {
    if over != default {
        base.overlay(default, over);
    }
}

/// A nested struct held as [`Shared`]: untouched, and so still shared,
/// unless `over` sets something in it.
fn shared<T: Overlay + SharedDefault + PartialEq>(
    base: &mut Shared<T>,
    default: &Shared<T>,
    over: &Shared<T>,
) {
    if over.ptr_eq(default) || over == default {
        return;
    }
    let base: &mut T = base;
    base.overlay(default, over);
}

macro_rules! overlay {
    (
        $ty:ident { $($leaf:ident),* $(,)? }
        nested { $($nested:ident: $kind:ident $nested_ty:ident),* $(,)? }
    ) => {
        impl Overlay for $ty {
            const LEAVES: usize = 0 $(+ { stringify!($leaf); 1 })* $(+ <$nested_ty as Overlay>::LEAVES)*;

            fn overlay(&mut self, default: &Self, over: &Self) {
                // Exhaustive on purpose: see the module documentation.
                let $ty { $($leaf: _,)* $($nested: _,)* } = over;
                $(
                    if over.$leaf != default.$leaf {
                        self.$leaf = over.$leaf.clone();
                    }
                )*
                $( $kind(&mut self.$nested, &default.$nested, &over.$nested); )*
            }

            fn leaf_paths(prefix: &str, out: &mut Vec<String>) {
                $( out.push(format!("{prefix}{}", stringify!($leaf))); )*
                $(
                    <$nested_ty as Overlay>::leaf_paths(
                        &format!("{prefix}{}.", stringify!($nested)),
                        out,
                    );
                )*
            }
        }
    };
}

overlay! {
    LayoutStyle {
        layout_isolation, direction, dir, writing_mode, unsupported_writing_mode,
        text_orientation, flex_reverse, order, flex_wrap, equal_wrap, display,
        box_sizing, position, anchor, z_index, isolation, isolate_from_ancestors, transform, transform_3d,
        unsupported_transform, transform_origin, css_perspective, preserve_3d,
        transform_box, gap, row_gap, column_gap, padding, padding_top,
        padding_right, padding_bottom, padding_left, margin, margin_top,
        margin_right, margin_bottom, margin_left, offset_top, offset_right,
        offset_bottom, offset_left, width, height, min_width, max_width, min_height,
        max_height, allow_shrink, align_items, align_self, align_content,
        justify_content, justify_items, justify_self, flex_grow, flex_shrink,
        flex_basis, adaptation, overflow_x, overflow_y, text_overflow_ellipsis,
        line_clamp, pointer_events, cursor, user_select, white_space_nowrap,
        white_space, word_break, overflow_wrap, aspect_ratio, text_align, float,
        clear, font_size, font_weight, font_italic, font_family, line_height,
        letter_spacing, color, text_decoration, text_emphasis_style,
        text_emphasis_position, text_emphasis_color, font_features,
        font_variation_settings, font_kerning, line_break,
        unsupported_font_variation, placeholder_color, placeholder_opacity,
        selection_background, selection_color, grid_columns, grid_rows,
        grid_columns_unsupported, grid_rows_unsupported, grid_auto_columns,
        grid_auto_rows, grid_auto_flow, grid_columns_repeat, grid_rows_repeat,
        grid_columns_subgrid, grid_rows_subgrid, grid_template_areas,
        grid_column_line_names, grid_row_line_names, hidden, opacity, background,
        border_radius, border_width, border_top_width, border_right_width,
        border_bottom_width, border_left_width, border_color, border_top_color,
        border_right_color, border_bottom_color, border_left_color, border_style,
        border_top_style, border_right_style, border_bottom_style,
        border_left_style, container_type, container_name,
    }
    nested {
        theme: shared ThemeLayout,
        paint_colors: shared PaintColorSlots,
        logical_padding: shared LogicalEdges,
        logical_margin: shared LogicalEdges,
        logical_inset: shared LogicalEdges,
        text_typography: plain TextTypography,
        grid_placement: shared GridPlacement,
        paint: shared PaintStyle,
    }
}

overlay! {
    ThemeLayout {
        gap, row_gap, column_gap, padding, padding_top, padding_right,
        padding_bottom, padding_left, margin, margin_top, margin_right,
        margin_bottom, margin_left, offset_top, offset_right, offset_bottom,
        offset_left, width, height, min_width, max_width, min_height, max_height,
        flex_basis, font_size, font_weight, line_height, letter_spacing,
        border_width, border_top_width, border_right_width, border_bottom_width,
        border_left_width, border_radius, scrollbar_room,
    }
    nested {
    }
}

overlay! {
    PaintColorSlots {
        color, background, border, border_top, border_right, border_bottom,
        border_left, outline, selection_background, selection_color,
    }
    nested {
    }
}

overlay! {
    LogicalEdges {
        inline_start, inline_end, block_start, block_end, phys_top, phys_right,
        phys_bottom, phys_left, generations, next_gen, derived,
    }
    nested {
    }
}

overlay! {
    TextTypography {
        spacing_trim, autospace, justify, wrap_style, justify_lines,
    }
    nested {
    }
}

overlay! {
    GridPlacement {
        column_start, column_end, row_start, row_end, area,
    }
    nested {
    }
}

overlay! {
    PaintStyle {
        background_material, border_material, text_material, visibility,
        border_radii, box_shadows, scrollbar, scroll_edge_fade, text_shadow,
        text_shadows, text_stroke, paint_order_stroke_first, mix_blend,
        unsupported_border_image, border_image, background_image, background_layers,
        background_size_list, background_size_lengths, background_position_list,
        background_repeat_list, content_image, skipped_replaced, object_fit,
        object_position, mask, clip_path, filter, backdrop_filter,
    }
    nested {
        outline: plain OutlineSpec,
    }
}

overlay! {
    OutlineSpec {
        width, color, style,
    }
    nested {
    }
}

impl LayoutStyle {
    /// How many fields [`Self::overlaid`] carries one by one. The Style
    /// Model's own count of them (the leaves of a serialized layout) is the
    /// same number; `nana-ui-css` holds the two together.
    pub const OVERLAY_LEAVES: usize = <LayoutStyle as Overlay>::LEAVES;

    /// The dotted path of every field [`Self::overlaid`] carries
    /// (`align_items`, `paint.outline.width`): the Style Model's leaves by
    /// their Rust names. A build tool that addresses fields by path (a
    /// compiled style patch does) checks its paths against these.
    pub fn overlay_leaf_paths() -> Vec<String> {
        let mut paths = Vec::with_capacity(Self::OVERLAY_LEAVES);
        <LayoutStyle as Overlay>::leaf_paths("", &mut paths);
        paths
    }

    /// `self` with each field `over` sets written over it: every field
    /// whose value differs from the default layout's. A nested group is
    /// written field by field; anything else (a value, an `Option`, an
    /// enum, a list) whole.
    pub fn overlaid(&self, over: &LayoutStyle) -> LayoutStyle {
        let mut layout = self.clone();
        layout.overlay(&LayoutStyle::default(), over);
        layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_leaf_has_a_path_of_its_own() {
        let paths = LayoutStyle::overlay_leaf_paths();
        assert_eq!(paths.len(), LayoutStyle::OVERLAY_LEAVES);
        let unique: std::collections::BTreeSet<&String> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len());
        assert!(paths.iter().any(|path| path == "paint.outline.width"));
    }

    #[test]
    fn an_overlay_writes_what_the_upper_layer_set_and_keeps_the_rest() {
        let mut base = LayoutStyle {
            order: 3,
            ..LayoutStyle::default()
        };
        base.paint.outline.width = 2.0;
        // A field its group leaves out of its serialized form when unset.
        // The JSON merge this replaced took the whole group from the upper
        // layer whenever one was set, and lost this one.
        let space = crate::ThemeExpr::token(crate::ThemeToken::Space(crate::SpacingStep::Md));
        base.theme.scrollbar_room = Some(space.clone());
        let mut over = LayoutStyle {
            flex_reverse: true,
            ..LayoutStyle::default()
        };
        over.paint.outline.width = 5.0;
        over.theme.gap = Some(space.clone());
        let mut expected = base.clone();
        expected.flex_reverse = true;
        expected.paint.outline.width = 5.0;
        expected.theme.gap = Some(space);
        assert_eq!(base.overlaid(&over), expected);
        // Nothing set above: the lower layer comes through, groups still shared.
        let through = base.overlaid(&LayoutStyle::default());
        assert_eq!(through, base);
        assert!(through.paint.ptr_eq(&base.paint));
    }
}
