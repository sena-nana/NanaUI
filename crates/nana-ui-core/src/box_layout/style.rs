//! Retained `LayoutStyle` struct, logical edges, and box/layout resolution helpers.

use crate::style_model::{PaintColor, StyleModelRef};
use crate::typography::{FontKerningSpec, FontVariationSetting, LineBreakSpec};

use super::grid::{lookup_named_line_after, lookup_named_line_nth, *};
use super::paint::{PAINT_MAT4_EPS, edge_paints_stroke, paint_border_width, used_border_width, *};
use super::units::*;

/// The one track each axis of a [`DisplaySpec::Layers`] container has: as
/// large as its largest item, so the container sizes to it. Placement then
/// widens the cell to the container's content box.
const LAYER_TRACKS: &[GridTrack] = &[GridTrack::Auto];

/// CSS `container-type`: whether a box answers container-size queries
/// (Issue #265), and on which axes. A query container is not
/// size-contained here: its size still follows its content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ContainerType {
    /// Not a query container.
    #[default]
    Normal,
    /// Answers queries on its inline size.
    InlineSize,
    /// Answers queries on its inline and its block size.
    Size,
}

/// The four logical edges of one box property (`padding`, `margin`, `inset`)
/// and the four physical edges they compete with.
///
/// Which physical edge a logical one lands on is only known once the final
/// `writing-mode` and `direction` are — `padding-inline-start` is the left
/// edge in `horizontal-tb` LTR, the right one in RTL, the top in a vertical
/// mode and the bottom in a vertical RTL one. So both are kept as specified and
/// resolved through [`crate::WritingContext`] when the box's context is final
/// ([`LayoutStyle::resolve_logical_box_edges`]).
///
/// Cascade order between a logical and a physical declaration of the same edge
/// is kept with generation stamps: the later one wins, whichever kind it is.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LogicalEdges {
    pub inline_start: Option<LengthSpec>,
    pub inline_end: Option<LengthSpec>,
    pub block_start: Option<LengthSpec>,
    pub block_end: Option<LengthSpec>,
    pub phys_top: Option<LengthSpec>,
    pub phys_right: Option<LengthSpec>,
    pub phys_bottom: Option<LengthSpec>,
    pub phys_left: Option<LengthSpec>,
    /// Generation of each field above, in the same order.
    pub generations: [u32; 8],
    pub next_gen: u32,
    /// Physical edges (top, right, bottom, left as bits 0–3) the last
    /// [`Self::resolve_into`] wrote from a logical edge, so a later context in
    /// which no logical edge lands there clears them instead of leaving the old
    /// value behind.
    pub derived: u8,
}

/// Index of a logical edge in [`LogicalEdges::generations`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalEdge {
    InlineStart = 0,
    InlineEnd = 1,
    BlockStart = 2,
    BlockEnd = 3,
}

impl LogicalEdges {
    pub fn has_logical(&self) -> bool {
        self.inline_start.is_some()
            || self.inline_end.is_some()
            || self.block_start.is_some()
            || self.block_end.is_some()
    }

    /// Physical records and no logical edge: nothing for them to order
    /// against (see [`LayoutStyle::has_inert_physical_edges`]).
    pub fn has_only_physical(&self) -> bool {
        !self.has_logical()
            && (self.phys_top.is_some()
                || self.phys_right.is_some()
                || self.phys_bottom.is_some()
                || self.phys_left.is_some())
    }

    fn stamp(&mut self, slot: usize) {
        self.next_gen = self.next_gen.saturating_add(1);
        self.generations[slot] = self.next_gen;
    }

    pub fn set(&mut self, edge: LogicalEdge, spec: Option<LengthSpec>) {
        *match edge {
            LogicalEdge::InlineStart => &mut self.inline_start,
            LogicalEdge::InlineEnd => &mut self.inline_end,
            LogicalEdge::BlockStart => &mut self.block_start,
            LogicalEdge::BlockEnd => &mut self.block_end,
        } = spec;
        self.stamp(edge as usize);
    }

    pub fn set_start(&mut self, spec: Option<LengthSpec>) {
        self.set(LogicalEdge::InlineStart, spec);
    }

    pub fn set_end(&mut self, spec: Option<LengthSpec>) {
        self.set(LogicalEdge::InlineEnd, spec);
    }

    /// Records a physical declaration of `edge`, so a logical one declared
    /// before it loses and one declared after wins.
    pub fn set_phys(&mut self, edge: crate::PhysicalEdge, spec: Option<LengthSpec>) {
        let slot = 4 + Self::phys_slot(edge);
        *self.phys_mut(edge) = spec;
        self.stamp(slot);
    }

    pub fn set_phys_left(&mut self, spec: Option<LengthSpec>) {
        self.set_phys(crate::PhysicalEdge::Left, spec);
    }

    pub fn set_phys_right(&mut self, spec: Option<LengthSpec>) {
        self.set_phys(crate::PhysicalEdge::Right, spec);
    }

    const fn phys_slot(edge: crate::PhysicalEdge) -> usize {
        match edge {
            crate::PhysicalEdge::Top => 0,
            crate::PhysicalEdge::Right => 1,
            crate::PhysicalEdge::Bottom => 2,
            crate::PhysicalEdge::Left => 3,
        }
    }

    fn phys_mut(&mut self, edge: crate::PhysicalEdge) -> &mut Option<LengthSpec> {
        match edge {
            crate::PhysicalEdge::Top => &mut self.phys_top,
            crate::PhysicalEdge::Right => &mut self.phys_right,
            crate::PhysicalEdge::Bottom => &mut self.phys_bottom,
            crate::PhysicalEdge::Left => &mut self.phys_left,
        }
    }

    fn phys(&self, edge: crate::PhysicalEdge) -> Option<LengthSpec> {
        match edge {
            crate::PhysicalEdge::Top => self.phys_top,
            crate::PhysicalEdge::Right => self.phys_right,
            crate::PhysicalEdge::Bottom => self.phys_bottom,
            crate::PhysicalEdge::Left => self.phys_left,
        }
    }

    /// The used value of the physical `edge` in `context`: the later of its
    /// own declaration and the logical edge that lands on it.
    pub fn used(
        &self,
        edge: crate::PhysicalEdge,
        context: crate::WritingContext,
    ) -> Option<LengthSpec> {
        let logical = [
            (
                context.inline_start(),
                self.inline_start,
                LogicalEdge::InlineStart,
            ),
            (
                context.inline_end(),
                self.inline_end,
                LogicalEdge::InlineEnd,
            ),
            (
                context.block_start(),
                self.block_start,
                LogicalEdge::BlockStart,
            ),
            (context.block_end(), self.block_end, LogicalEdge::BlockEnd),
        ]
        .into_iter()
        .find(|(landing, _, _)| *landing == edge)
        .map(|(_, value, which)| (value, self.generations[which as usize]));
        let physical = (self.phys(edge), self.generations[4 + Self::phys_slot(edge)]);
        match (logical, physical) {
            (Some((Some(value), logical_gen)), (Some(phys), phys_gen)) => {
                Some(if phys_gen >= logical_gen { phys } else { value })
            }
            (Some((Some(value), _)), (None, _)) => Some(value),
            (_, (phys, _)) => phys,
        }
    }

    pub fn used_left(&self, context: crate::WritingContext) -> Option<LengthSpec> {
        self.used(crate::PhysicalEdge::Left, context)
    }

    pub fn used_right(&self, context: crate::WritingContext) -> Option<LengthSpec> {
        self.used(crate::PhysicalEdge::Right, context)
    }

    /// Writes the used value of every physical edge a declaration here
    /// decides, in `context`, and leaves the rest alone.
    ///
    /// An edge a logical edge lands on takes the later of that and its own
    /// physical declaration. An edge with only a recorded physical declaration
    /// takes it. An edge this wrote from a logical edge last time, and which
    /// nothing lands on now, is cleared — the logical edge moved away. Any
    /// other edge keeps what the style already had, so a physical edge set
    /// directly on a hand-built style survives.
    pub fn resolve_into(
        &mut self,
        context: crate::WritingContext,
        top: &mut Option<LengthSpec>,
        right: &mut Option<LengthSpec>,
        bottom: &mut Option<LengthSpec>,
        left: &mut Option<LengthSpec>,
    ) {
        let landings = [
            (context.inline_start(), self.inline_start),
            (context.inline_end(), self.inline_end),
            (context.block_start(), self.block_start),
            (context.block_end(), self.block_end),
        ];
        let edges = [
            (crate::PhysicalEdge::Top, top),
            (crate::PhysicalEdge::Right, right),
            (crate::PhysicalEdge::Bottom, bottom),
            (crate::PhysicalEdge::Left, left),
        ];
        let mut derived = 0u8;
        for (edge, field) in edges {
            let bit = 1u8 << Self::phys_slot(edge);
            let lands = landings
                .iter()
                .any(|(landing, value)| *landing == edge && value.is_some());
            let recorded = self.generations[4 + Self::phys_slot(edge)] > 0;
            if lands {
                *field = self.used(edge, context);
                derived |= bit;
            } else if recorded {
                *field = self.phys(edge);
            } else if self.derived & bit != 0 {
                *field = None;
            }
        }
        self.derived = derived;
    }

    /// Pure read-only version of [`Self::resolve_into`] that returns physical
    /// `[top, right, bottom, left]` for `context` without mutating `self`.
    pub fn resolved_edges(
        &self,
        context: crate::WritingContext,
        base: [Option<LengthSpec>; 4],
    ) -> [Option<LengthSpec>; 4] {
        let landings = [
            (context.inline_start(), self.inline_start),
            (context.inline_end(), self.inline_end),
            (context.block_start(), self.block_start),
            (context.block_end(), self.block_end),
        ];
        let edges = [
            crate::PhysicalEdge::Top,
            crate::PhysicalEdge::Right,
            crate::PhysicalEdge::Bottom,
            crate::PhysicalEdge::Left,
        ];
        let mut out = base;
        for (idx, edge) in edges.into_iter().enumerate() {
            let bit = 1u8 << Self::phys_slot(edge);
            let lands = landings
                .iter()
                .any(|(landing, value)| *landing == edge && value.is_some());
            let recorded = self.generations[4 + Self::phys_slot(edge)] > 0;
            if lands {
                out[idx] = self.used(edge, context);
            } else if recorded {
                out[idx] = self.phys(edge);
            } else if self.derived & bit != 0 {
                out[idx] = None;
            }
        }
        out
    }
}

/// Cold 2D/3D paint transform properties shared out of [`LayoutStyle`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransformStyle {
    /// Paint-only 2D `transform` subset. It never participates in flex/grid
    /// measurement. Planar CSS 3D is [`Self::transform_3d`]; parent
    /// `perspective` is [`Self::css_perspective`] + [`Self::perspective_origin`],
    /// and `preserve-3d` is [`Self::preserve_3d`] (which fail-closes child 3D).
    pub transform: Option<PaintTransform>,
    /// Planar 4×4 (`perspective()` + `rotateY`, true `matrix3d`). Additive to
    /// the 2×3: 2D lists stay on [`Self::transform`].
    pub transform_3d: Option<PaintMat4>,
    pub unsupported_transform: Option<String>,
    /// CSS `transform-origin`. `None` = `50% 50%` (box center).
    /// Applied to both the 2×3 and the 4×4 planar path.
    pub origin: Option<TransformOrigin>,
    /// CSS `perspective` property (not the transform function). Applies to
    /// direct children's 3D transforms around [`Self::perspective_origin`].
    pub css_perspective: Option<f32>,
    /// CSS `perspective-origin`. `None` = `50% 50%` of this box's border box.
    pub perspective_origin: Option<TransformOrigin>,
    /// CSS `transform-style: preserve-3d`. Flattening is not implemented.
    pub preserve_3d: bool,
    /// CSS `backface-visibility`. Default `visible`.
    pub backface_visibility: BackfaceVisibility,
    /// CSS `transform-box`. Default `view-box` (used as border-box for HTML).
    pub transform_box: TransformBox,
}

/// Cold typography, text emphasis, writing-orientation and pseudo-element
/// colors shared out of [`LayoutStyle`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextStyleExt {
    /// Preferred named family from `font-family` (generics stripped). `None` = UI default.
    pub font_family: Option<String>,
    /// CSS `letter-spacing` in px. `None` = inherit / normal (0).
    pub letter_spacing: Option<f32>,
    /// `line-clamp` / `-webkit-line-clamp` (wrap + ellipsis, max N lines).
    pub line_clamp: Option<u16>,
    /// CSS `word-break`. `None` = inherit / `normal`.
    pub word_break: Option<WordBreakSpec>,
    /// CSS `overflow-wrap` / `word-wrap`. `None` = inherit / `normal`.
    pub overflow_wrap: Option<OverflowWrapSpec>,
    /// CSS `text-decoration` / `text-decoration-line`. `None` = inherit / none.
    pub text_decoration: Option<TextDecorationLine>,
    /// CSS `text-emphasis-style`. `None` = inherit / `none`.
    pub text_emphasis_style: Option<crate::typography::TextEmphasisStyle>,
    /// CSS `text-emphasis-position`. `None` = inherit / `over right`.
    pub text_emphasis_position: Option<crate::typography::TextEmphasisPosition>,
    /// CSS `text-emphasis-color`. `None` = inherit / `currentColor`.
    pub text_emphasis_color: Option<crate::typography::TextEmphasisColor>,
    /// CSS `font-feature-settings`. `None` = inherit; `Some([])` = `normal`.
    pub font_features: Option<Vec<FontFeatureSetting>>,
    /// CSS `font-variation-settings`. `None` = inherit; `Some([])` = `normal`.
    /// Declared axes are passed to shaping; axes missing from the face are
    /// ignored. `wght` also merges into [`LayoutStyle::font_weight`].
    pub font_variation_settings: Option<Vec<FontVariationSetting>>,
    /// CSS `font-kerning`. `None` = inherit.
    pub font_kerning: Option<FontKerningSpec>,
    /// CSS `line-break`. `None` = inherit.
    pub line_break: Option<LineBreakSpec>,
    /// CSS `text-spacing-trim`, `text-autospace`, `text-justify` and
    /// `text-wrap-style`, each inherited when unset (Issue #211).
    pub text_typography: crate::typography::TextTypography,
    /// Malformed `font-variation-settings` (unparseable). Declared axes that
    /// the loaded face does not provide are skipped at shape time and never
    /// remapped onto `wght`.
    pub unsupported_font_variation: bool,
    /// Vertical / sideways `writing-mode` is fail-closed (no axis remap).
    pub unsupported_writing_mode: bool,
    /// CSS `text-orientation`. `None` = inherit, then `mixed`.
    pub text_orientation: Option<TextOrientationSpec>,
    /// CSS `vertical-align` (non-inherited; applies to inline-level boxes).
    pub vertical_align: VerticalAlignSpec,
    /// `::placeholder` color on a text input (RGBA 0..=1). Not a generated box.
    pub placeholder_color: Option<[f32; 4]>,
    /// `::placeholder` opacity multiplier. Combined with
    /// [`Self::placeholder_color`] (or theme faint) at TextInput paint.
    pub placeholder_opacity: Option<f32>,
}

impl TextStyleExt {
    /// Whether any cold text field in this group overrides the node's resolved `ComputedStyle`.
    pub fn overrides_computed_style(&self) -> bool {
        self.font_family.is_some()
            || self.letter_spacing.is_some()
            || self.font_features.is_some()
            || self.font_variation_settings.is_some()
            || self.font_kerning.is_some()
            || self.word_break.is_some()
            || self.line_break.is_some()
            || self.text_typography != crate::typography::TextTypography::default()
            || self.text_emphasis_style.is_some()
            || self.text_emphasis_position.is_some()
            || self.text_emphasis_color.is_some()
            || self.text_orientation.is_some()
    }
}

/// Cold per-side border longhands shared out of [`LayoutStyle`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BorderSides {
    pub top_width: Option<f32>,
    pub right_width: Option<f32>,
    pub bottom_width: Option<f32>,
    pub left_width: Option<f32>,
    pub top_style: Option<BorderStyle>,
    pub right_style: Option<BorderStyle>,
    pub bottom_style: Option<BorderStyle>,
    pub left_style: Option<BorderStyle>,
}

/// Cold CSS logical box edges shared out of [`LayoutStyle`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LogicalBoxEdges {
    /// Specified `padding-inline-*` plus physical left/right they compete with.
    pub padding: LogicalEdges,
    pub margin: LogicalEdges,
    pub inset: LogicalEdges,
}

/// Cold `@container` query properties shared out of [`LayoutStyle`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContainerStyle {
    /// CSS `container-type`: whether container-size rules below this box
    /// can query it. It moves no box.
    pub container_type: ContainerType,
    /// CSS `container-name`: the names a named `@container` query finds
    /// this box by. Empty, it answers unnamed queries only.
    pub container_name: Vec<String>,
}

/// 可测布局意图（Style Model Layout 盒切片）。
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutStyle {
    /// Typed theme expressions, resolved by Runtime after cascade and responsive rules.
    pub theme: crate::shared::Shared<crate::theme::ThemeLayout>,
    /// Explicit authoring-space colors retained alongside compatibility RGBA fields.
    pub paint_colors: crate::shared::Shared<crate::style_model::PaintColorSlots>,
    /// Isolate descendant layout invalidation when both authored dimensions
    /// are fixed pixels and the box is in normal flow. Other boxes retain
    /// normal dependency propagation.
    pub layout_isolation: bool,
    pub direction: Option<FlexDirection>,
    /// CSS `direction` (`ltr` / `rtl`). `None` = inherit / initial `ltr`.
    /// Not [`Self::direction`] (`flex-direction`). Remaps logical box edges
    /// and `text-align: start | end` only — not flex/grid start or item order.
    pub dir: Option<DirSpec>,
    /// CSS `writing-mode`. `None` = inherit, then `horizontal-tb`.
    /// `sideways-*` stay fail-closed via [`TextStyleExt::unsupported_writing_mode`].
    pub writing_mode: Option<WritingModeSpec>,
    /// `row-reverse` / `column-reverse`：主轴起点对调（measure 反转子项序）。
    pub flex_reverse: bool,
    /// CSS `order`（flex/grid 项）。默认 `0`；可为负。布局按升序，同值保留源序；
    /// 再应用 [`Self::flex_reverse`]。
    pub order: i32,
    pub flex_wrap: FlexWrap,
    /// Pack each wrap line by its largest item's intrinsic main size, then
    /// divide the line equally. Opt-in retained layout intent for equal pickers.
    pub equal_wrap: bool,
    pub display: Option<DisplaySpec>,
    pub box_sizing: BoxSizing,
    pub position: PositionSpec,
    /// Anchor positioning for a `position: fixed` box; see [`AnchorSpec`].
    pub anchor: Option<std::sync::Arc<AnchorSpec>>,
    /// CSS `z-index`（整数）。`None` = auto；fixed 层默认按树序叠在内容之上。
    pub z_index: Option<i32>,
    /// CSS `isolation: isolate`. Paint-order stacking context; not a layout box.
    pub isolation: bool,
    /// Cold 2D/3D paint transform properties.
    pub transforms: crate::shared::Shared<TransformStyle>,
    /// Uniform `gap` shorthand residue / class defaults. Axis overrides win.
    /// `%` / calc 保留为 [`LengthSpec`]（同 margin/padding），布局时相对 CB 解析；
    /// 解析期无 CB 时不得收成 px 或静默丢弃。
    pub gap: Option<LengthSpec>,
    /// `row-gap`（或 `gap: row column` 的第一值）。
    pub row_gap: Option<LengthSpec>,
    /// `column-gap`（或 `gap: row column` 的第二值）。
    pub column_gap: Option<LengthSpec>,
    /// Uniform padding shorthand residue. `%` / calc 保留为 [`LengthSpec`]，
    /// 布局时相对包含块**宽度**解析（含上下边）。
    pub padding: Option<LengthSpec>,
    pub padding_top: Option<LengthSpec>,
    pub padding_right: Option<LengthSpec>,
    pub padding_bottom: Option<LengthSpec>,
    pub padding_left: Option<LengthSpec>,
    /// Uniform margin shorthand residue（`%` 合同同 padding）。
    pub margin: Option<LengthSpec>,
    pub margin_top: Option<LengthSpec>,
    pub margin_right: Option<LengthSpec>,
    pub margin_bottom: Option<LengthSpec>,
    pub margin_left: Option<LengthSpec>,
    /// Inset：`relative` / `absolute` / `fixed` 用（`Px` 或 `%`；measure 时相对 CB 解析）。
    /// `Static` 忽略；`sticky` defer。
    pub offset_top: Option<LengthSpec>,
    pub offset_right: Option<LengthSpec>,
    pub offset_bottom: Option<LengthSpec>,
    pub offset_left: Option<LengthSpec>,
    /// Cold CSS logical padding/margin/inset edges.
    pub logical_edges: crate::shared::Shared<LogicalBoxEdges>,
    pub width: Option<LengthSpec>,
    pub height: Option<LengthSpec>,
    /// `min-width`：保留 [`LengthSpec`]（px / `%` / calc / em / viewport），布局时解析。
    pub min_width: Option<LengthSpec>,
    /// `max-width`：同上。`Fill` 表示无有限上限。
    pub max_width: Option<LengthSpec>,
    /// `min-height`：同上。
    pub min_height: Option<LengthSpec>,
    /// `max-height`：同上。
    pub max_height: Option<LengthSpec>,
    /// `min-width: 0` → 允许 flex 子项收缩。
    pub allow_shrink: bool,
    pub align_items: AlignSpec,
    /// `align-self`；`None` = `auto`（继承容器 `align-items`）。
    pub align_self: Option<AlignSpec>,
    /// `align-content`（多行 flex 线间分布；`stretch`/`normal` 均分剩余交叉空间）。
    pub align_content: JustifySpec,
    pub justify_content: JustifySpec,
    /// `justify-items`（Box Alignment；flex 项忽略，供 grid/place-items 第二值）。
    pub justify_items: Option<AlignSpec>,
    /// `justify-self`（Box Alignment；flex 项忽略，供 place-self 第二值）。
    pub justify_self: Option<AlignSpec>,
    /// `flex-grow`；>0 时主轴随父方向 Fill。
    pub flex_grow: Option<f32>,
    /// `flex-shrink`。未写长手时 `None` 按 **0**（不是 CSS initial 1），避免溢出的
    /// 定宽行被压扁。`flex` 简写省略 shrink 时仍按 CSS 写成 `Some(1.0)`
    ///（`flex: initial` / `flex: N` / `flex: N <basis>`）。
    pub flex_shrink: Option<f32>,
    pub flex_basis: Option<LengthSpec>,
    /// What this box declares about its elasticity, and whether its line
    /// solves overflow by compressing its children's (Issue #207). `None`
    /// declares nothing and solves nothing.
    pub adaptation: Option<std::sync::Arc<crate::dynamic_layout::AdaptationProfile>>,
    pub overflow_x: OverflowSpec,
    pub overflow_y: OverflowSpec,
    /// `text-overflow: ellipsis`（需配合 nowrap / 定宽；Scene text 路径兑现）。
    pub text_overflow_ellipsis: bool,
    /// Specified CSS `pointer-events`. `None` means inherit (not `auto`).
    pub pointer_events: Option<PointerEventsSpec>,
    /// HTML `inert`: this box and every box under it still lay out, paint and
    /// animate, but take no pointer, cannot be focused (sequentially or
    /// programmatically) and offer no accessibility actions. Unlike
    /// [`Self::pointer_events`], a descendant cannot turn it back off.
    /// Setting it moves no box.
    pub inert: bool,
    /// CSS `cursor`; unspecified values inherit from the parent.
    pub cursor: Option<CursorSpec>,
    /// CSS `user-select`; unspecified values inherit from the parent.
    pub user_select: Option<UserSelectSpec>,
    /// `white-space` 子集。
    pub white_space: WhiteSpaceSpec,
    /// CSS `aspect-ratio` as width/height. `None` = `auto`.
    pub aspect_ratio: Option<f32>,
    /// `text-align`（IFC 行对齐）。
    pub text_align: TextAlignSpec,
    /// Computed `font-size` in CSS px. `None` = inherit (then initial / ControlSize).
    pub font_size: Option<f32>,
    /// CSS `font-weight` as 100..=900. `None` = inherit / normal.
    pub font_weight: Option<u16>,
    /// CSS `font-style: italic` / `oblique`. `None` = inherit / `normal`.
    pub font_italic: Option<bool>,
    /// CSS `line-height` subset. `None` = inherit / engine default.
    pub line_height: Option<LineHeightSpec>,
    /// Cold text styling, typography, emphasis, and pseudo-element properties.
    pub text_ext: crate::shared::Shared<TextStyleExt>,
    /// Cold CSS Grid template and placement properties.
    pub grid: crate::shared::Shared<GridStyle>,
    pub hidden: bool,
    /// Paint-only surface: corner radii, box-shadow, CSS visibility.
    pub paint: crate::shared::Shared<PaintStyle>,
    /// CSS `opacity` (0..=1). `None` = unset / inherit (treated as 1.0 at paint).
    /// Parsed with other declarations so CSS adapters need not re-scan the style
    /// string.
    pub opacity: Option<f32>,
    pub border_radius: Option<f32>,
    pub border_width: Option<f32>,
    /// Uniform `border-style` shorthand residue. Longhands override.
    pub border_style: Option<BorderStyle>,
    /// Cold per-side border widths and styles.
    pub border_sides: crate::shared::Shared<BorderSides>,
    /// Cold `@container` query properties.
    pub container: crate::shared::Shared<ContainerStyle>,
}

impl Default for LayoutStyle {
    fn default() -> Self {
        Self {
            theme: Default::default(),
            paint_colors: Default::default(),
            layout_isolation: false,
            direction: None,
            dir: None,
            writing_mode: None,
            flex_reverse: false,
            order: 0,
            flex_wrap: FlexWrap::NoWrap,
            equal_wrap: false,
            display: None,
            box_sizing: BoxSizing::BorderBox,
            position: PositionSpec::Static,
            anchor: None,
            z_index: None,
            isolation: false,
            transforms: Default::default(),
            gap: None,
            row_gap: None,
            column_gap: None,
            padding: None,
            padding_top: None,
            padding_right: None,
            padding_bottom: None,
            padding_left: None,
            margin: None,
            margin_top: None,
            margin_right: None,
            margin_bottom: None,
            margin_left: None,
            offset_top: None,
            offset_right: None,
            offset_bottom: None,
            offset_left: None,
            logical_edges: Default::default(),
            width: None,
            height: None,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            allow_shrink: false,
            align_items: AlignSpec::Start,
            align_self: None,
            align_content: JustifySpec::Start,
            justify_content: JustifySpec::Start,
            justify_items: None,
            justify_self: None,
            flex_grow: None,
            flex_shrink: None,
            flex_basis: None,
            adaptation: None,
            overflow_x: OverflowSpec::Visible,
            overflow_y: OverflowSpec::Visible,
            text_overflow_ellipsis: false,
            pointer_events: None,
            inert: false,
            cursor: None,
            user_select: None,
            white_space: WhiteSpaceSpec::Normal,
            aspect_ratio: None,
            text_align: TextAlignSpec::Start,
            font_size: None,
            font_weight: None,
            font_italic: None,
            line_height: None,
            text_ext: Default::default(),
            grid: Default::default(),
            hidden: false,
            paint: Default::default(),
            opacity: None,
            border_radius: None,
            border_width: None,
            border_style: None,
            border_sides: Default::default(),
            container: Default::default(),
        }
    }
}

impl LayoutStyle {
    /// Construct a `LayoutStyle` from `Default` and configure it in place.
    #[inline]
    pub fn with(configure: impl FnOnce(&mut Self)) -> Self {
        let mut style = Self::default();
        configure(&mut style);
        style
    }
    /// Box-local pivot in px, relative to the layout (border) box origin.
    /// Missing origin is CSS `50% 50%`. `content-box` / `fill-box` subtract
    /// border + padding (`resolved_padding`: `%` without a CB is 0).
    pub fn resolved_transform_origin(&self, width: f32, height: f32) -> [f32; 2] {
        let (origin, uses_content) = if self.transforms.is_shared_default() {
            (TransformOrigin::default(), false)
        } else {
            (
                self.transforms.origin.unwrap_or_default(),
                self.transforms.transform_box.uses_content_box(),
            )
        };
        if !uses_content {
            return origin.resolve(width, height);
        }
        let border = self.resolved_border_edges();
        let pad = self.resolved_padding();
        let inset_x = border.left + pad.left;
        let inset_y = border.top + pad.top;
        let content_w = (width - inset_x - border.right - pad.right).max(0.0);
        let content_h = (height - inset_y - border.bottom - pad.bottom).max(0.0);
        let [ox, oy] = origin.resolve(content_w, content_h);
        [inset_x + ox, inset_y + oy]
    }

    /// World-space 2×3 for this node's paint transform, pivoted at origin.
    /// Planar 3D uses [`Self::world_scene_transform`] instead.
    pub fn world_paint_transform(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<[f32; 6]> {
        let (matrix, persp) = self.world_scene_transform(x, y, width, height)?;
        (persp[0].abs() <= PAINT_MAT4_EPS && persp[1].abs() <= PAINT_MAT4_EPS).then_some(matrix)
    }

    /// World-space homography of the z=0 plane, pivoted at transform-origin.
    ///
    /// `persp` is `(g, h)` in `w = g x + h y + 1`. Zeroes keep a 2×3 affine.
    pub fn world_scene_transform(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<([f32; 6], [f32; 2])> {
        self.world_scene_transform_with_perspective(x, y, width, height, None)
    }

    /// World-space homography of the z=0 plane with an optional direct-parent
    /// CSS `perspective` + `perspective-origin` applied in 4×4 space before
    /// extracting the planar homography.
    pub fn world_scene_transform_with_perspective(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        parent_perspective: Option<ParentPerspective>,
    ) -> Option<([f32; 6], [f32; 2])> {
        if self.transforms.is_shared_default() {
            return None;
        }
        let t = &*self.transforms;
        if t.transform.is_none() && t.transform_3d.is_none() {
            return None;
        }
        let [ox, oy] = self.resolved_transform_origin(width, height);
        if let Some(mat4) = t.transform_3d {
            let local_4x4 = mat4.around_origin(x, y, ox, oy);
            let composed = if let Some(pp) = parent_perspective
                && pp.distance.is_finite()
                && pp.distance > 1e-5
                && let Some(p_mat) = PaintMat4::perspective(pp.distance)
            {
                p_mat
                    .around_origin(0.0, 0.0, pp.origin[0], pp.origin[1])
                    .then(local_4x4)
            } else {
                local_4x4
            };
            return composed.planar_homography();
        }
        let transform = t.transform?;
        Some((transform.around_origin(x, y, ox, oy), [0.0, 0.0]))
    }

    /// Resolved CSS `perspective-origin` in box-local px relative to the border box.
    pub fn resolved_perspective_origin(&self, width: f32, height: f32) -> [f32; 2] {
        if self.transforms.is_shared_default() {
            return TransformOrigin::default().resolve(width, height);
        }
        self.transforms
            .perspective_origin
            .unwrap_or_default()
            .resolve(width, height)
    }

    /// True when this node sets a positive finite CSS `perspective` length.
    pub fn has_css_perspective(&self) -> bool {
        !self.transforms.is_shared_default()
            && self
                .transforms
                .css_perspective
                .is_some_and(|d| d.is_finite() && d > 1e-5)
    }

    /// True when this node asked for a `preserve-3d` rendering context we do not flatten.
    pub fn fails_closed_3d_context(&self) -> bool {
        if self.transforms.is_shared_default() {
            return false;
        }
        self.transforms.preserve_3d
    }

    pub fn has_surface_paint(&self) -> bool {
        self.paint_colors.background.is_some()
            || self
                .resolved_border_radii(0.0, 0.0)
                .iter()
                .any(|r| *r > 0.0)
            || self.resolved_border_width() > 0.0
            || !self.paint.box_shadows.is_empty()
            || self.paint.has_advanced_paint()
    }

    /// True when any paint color slot, gradient stop, shadow, or text stroke
    /// carries a theme-dynamic [`PaintColor`] that must be resolved against
    /// the live [`StyleModelRef`] rather than baked as static sRGB.
    pub fn has_dynamic_paint_colors(&self) -> bool {
        (!self.paint_colors.is_shared_default() && self.paint_colors.has_dynamic_theme())
            || self.has_dynamic_paint()
    }

    /// True when advanced `paint` slots (gradients, shadows, strokes) carry dynamic theme colors.
    pub fn has_dynamic_paint(&self) -> bool {
        !self.paint.is_shared_default() && self.paint.has_dynamic_theme()
    }

    /// Resolve only advanced `paint` slots against `model`.
    pub fn resolve_theme_paint(&mut self, model: StyleModelRef) {
        if self.has_dynamic_paint() {
            self.paint.resolve_theme(model);
        }
    }

    /// Apply pre-resolved [`PaintColorSlots`] when this style holds dynamic theme colors.
    pub fn apply_pre_resolved_paint_colors(
        &mut self,
        resolved: &crate::shared::Shared<crate::style_model::PaintColorSlots>,
    ) {
        if !self.paint_colors.is_shared_default() && self.paint_colors.has_dynamic_theme() {
            self.paint_colors = resolved.clone();
        }
    }

    /// Resolve every theme-dynamic [`PaintColor`] against `model`.
    pub fn resolve_theme_colors(&mut self, model: StyleModelRef) {
        if !self.paint_colors.is_shared_default() && self.paint_colors.has_dynamic_theme() {
            self.paint_colors.resolve_theme(model);
        }

        if !self.paint.is_shared_default() && self.paint.has_dynamic_theme() {
            self.paint.resolve_theme(model);
        }
    }

    /// CSS `visibility: hidden` keeps layout but suppresses paint / hit-test.
    pub fn is_paint_visible(&self) -> bool {
        self.paint.is_visible()
    }

    /// Resolve corner radii (TL, TR, BR, BL) to physical px for a border box.
    pub fn resolved_border_radii(&self, width: f32, height: f32) -> [f32; 4] {
        if let Some(specs) = self.paint.border_radii {
            [
                resolve_corner_radius(specs[0], width, height),
                resolve_corner_radius(specs[1], width, height),
                resolve_corner_radius(specs[2], width, height),
                resolve_corner_radius(specs[3], width, height),
            ]
        } else {
            let uniform = self.border_radius.unwrap_or(0.0).max(0.0);
            [uniform; 4]
        }
    }

    /// `align-self: auto` → 容器 `align-items`；否则用自身值。
    pub fn resolved_align_self(&self, parent_align_items: AlignSpec) -> AlignSpec {
        self.align_self.unwrap_or(parent_align_items)
    }

    /// The alignment this container's children take when they name none:
    /// what [`Self::resolved_align_self`] falls back to.
    ///
    /// A block container stacking its children down the block axis lays
    /// them out as CSS block layout does, which has no `align-items`: a
    /// block-level child whose width is `auto` fills the containing block,
    /// less its margins (CSS 2.1 §10.3.3). That is `display: block`,
    /// `flow-root`, and an unset `display`, which is the same flow. A
    /// container laid along a row, and every flex, grid and layers
    /// container, aligns by its own `align_items`.
    pub fn child_align_items(&self) -> AlignSpec {
        if self.stacks_as_block() {
            AlignSpec::Stretch
        } else {
            self.align_items
        }
    }

    /// Whether this container uses default column stretch semantics:
    /// `display` unset and not laid along a row.
    pub fn stacks_as_block(&self) -> bool {
        self.display.is_none() && self.direction != Some(FlexDirection::Row)
    }

    pub fn gap_or(&self, default: f32) -> f32 {
        self.gap
            .or(self.row_gap)
            .or(self.column_gap)
            .and_then(|s| s.resolve_px(None))
            .unwrap_or(default)
            .max(0.0)
    }

    /// CSS `row-gap`（回退到 uniform `gap`）。无 `%` 基时仅兑现 `Px`。
    pub fn resolved_row_gap(&self) -> f32 {
        self.resolved_row_gap_against(None)
    }

    /// `row-gap` / uniform `gap`，`%` 相对 `percent_base`（通常为 CB 高度，缺省回退宽度）。
    pub fn resolved_row_gap_against(&self, percent_base: Option<f32>) -> f32 {
        self.resolved_row_gap_against_fonts(percent_base, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_row_gap_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_row_gap_against_fonts(
        &self,
        percent_base: Option<f32>,
        fonts: FontSizeContext,
    ) -> f32 {
        self.row_gap
            .or(self.gap)
            .and_then(|s| s.resolve_with_fonts(percent_base, None, fonts))
            .unwrap_or(0.0)
            .max(0.0)
    }

    /// CSS `column-gap`（回退到 uniform `gap`）。无 `%` 基时仅兑现 `Px`。
    pub fn resolved_column_gap(&self) -> f32 {
        self.resolved_column_gap_against(None)
    }

    /// `column-gap` / uniform `gap`，`%` 相对包含块宽度。
    pub fn resolved_column_gap_against(&self, percent_base: Option<f32>) -> f32 {
        self.resolved_column_gap_against_fonts(percent_base, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_column_gap_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_column_gap_against_fonts(
        &self,
        percent_base: Option<f32>,
        fonts: FontSizeContext,
    ) -> f32 {
        self.column_gap
            .or(self.gap)
            .and_then(|s| s.resolve_with_fonts(percent_base, None, fonts))
            .unwrap_or(0.0)
            .max(0.0)
    }

    /// Flex 主轴 gap：Row→column-gap；Column→row-gap。无 `%` 基时仅兑现 `Px`。
    pub fn main_gap(&self, direction: FlexDirection) -> f32 {
        self.main_gap_against(direction, ParentBox::default())
    }

    /// 主轴 gap，携带 CB 供 `%` 解析（column-gap→宽；row-gap→高，缺省回退宽）。
    pub fn main_gap_against(&self, direction: FlexDirection, cb: ParentBox) -> f32 {
        self.main_gap_against_fonts(direction, cb, FontSizeContext::default())
    }

    /// 同 [`Self::main_gap_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn main_gap_against_fonts(
        &self,
        direction: FlexDirection,
        cb: ParentBox,
        fonts: FontSizeContext,
    ) -> f32 {
        match direction {
            FlexDirection::Row => self.resolved_column_gap_against_fonts(cb.width, fonts),
            FlexDirection::Column => {
                self.resolved_row_gap_against_fonts(definite_length(cb.height).or(cb.width), fonts)
            }
        }
    }

    /// Flex 交叉轴 / wrap 行间 gap：Row→row-gap；Column→column-gap。
    pub fn cross_gap(&self, direction: FlexDirection) -> f32 {
        self.cross_gap_against(direction, ParentBox::default())
    }

    /// 交叉轴 gap，携带 CB 供 `%` 解析。
    /// Row 的 row-gap `%`：定高优先，否则回退宽度（wrap 自动高常见路径）。
    pub fn cross_gap_against(&self, direction: FlexDirection, cb: ParentBox) -> f32 {
        self.cross_gap_against_fonts(direction, cb, FontSizeContext::default())
    }

    /// 同 [`Self::cross_gap_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn cross_gap_against_fonts(
        &self,
        direction: FlexDirection,
        cb: ParentBox,
        fonts: FontSizeContext,
    ) -> f32 {
        match direction {
            FlexDirection::Row => {
                self.resolved_row_gap_against_fonts(definite_length(cb.height).or(cb.width), fonts)
            }
            FlexDirection::Column => self.resolved_column_gap_against_fonts(cb.width, fonts),
        }
    }

    /// Per-side border widths (physical px). Longhands override the uniform
    /// [`Self::border_width`] shorthand. `border-style: none` zeros that side.
    pub fn resolved_border_edges(&self) -> PaddingSpec {
        let uniform = self.border_width.unwrap_or(0.0).max(0.0);
        let styles = self.resolved_border_styles();
        if self.border_sides.is_shared_default() {
            return PaddingSpec {
                top: used_border_width(uniform, styles[0]),
                right: used_border_width(uniform, styles[1]),
                bottom: used_border_width(uniform, styles[2]),
                left: used_border_width(uniform, styles[3]),
            };
        }
        let sides = &*self.border_sides;
        PaddingSpec {
            top: used_border_width(sides.top_width.unwrap_or(uniform), styles[0]),
            right: used_border_width(sides.right_width.unwrap_or(uniform), styles[1]),
            bottom: used_border_width(sides.bottom_width.unwrap_or(uniform), styles[2]),
            left: used_border_width(sides.left_width.unwrap_or(uniform), styles[3]),
        }
    }

    /// Per-side styles. Longhands override [`Self::border_style`].
    pub fn resolved_border_styles(&self) -> [Option<BorderStyle>; 4] {
        let uniform = self.border_style;
        if self.border_sides.is_shared_default() {
            return [uniform; 4];
        }
        let sides = &*self.border_sides;
        [
            sides.top_style.or(uniform),
            sides.right_style.or(uniform),
            sides.bottom_style.or(uniform),
            sides.left_style.or(uniform),
        ]
    }

    /// Foreground `color` from [`Self::paint_colors`].
    #[inline]
    pub fn resolved_color(&self) -> Option<PaintColor> {
        self.paint_colors.color
    }

    /// Foreground `color` resolved to sRGB `[f32; 4]`.
    #[inline]
    pub fn color_srgb(&self) -> Option<[f32; 4]> {
        self.paint_colors.color.map(PaintColor::to_srgb)
    }

    /// Surface `background` from [`Self::paint_colors`].
    #[inline]
    pub fn resolved_background(&self) -> Option<PaintColor> {
        self.paint_colors.background
    }

    /// Surface `background` resolved to sRGB `[f32; 4]`.
    #[inline]
    pub fn background_srgb(&self) -> Option<[f32; 4]> {
        self.paint_colors.background.map(PaintColor::to_srgb)
    }

    /// CSS `outline` color from [`Self::paint_colors`].
    #[inline]
    pub fn resolved_outline_color(&self) -> Option<PaintColor> {
        self.paint_colors.outline
    }

    /// Author `::selection` background from [`Self::paint_colors`].
    #[inline]
    pub fn resolved_selection_background(&self) -> Option<PaintColor> {
        self.paint_colors.selection_background
    }

    /// Author `::selection` foreground from [`Self::paint_colors`].
    #[inline]
    pub fn resolved_selection_color(&self) -> Option<PaintColor> {
        self.paint_colors.selection_color
    }

    /// Per-side border colors as [`PaintColor`]. Longhands override `paint_colors.border`.
    pub fn resolved_border_side_colors(&self) -> [Option<PaintColor>; 4] {
        let slots = &*self.paint_colors;
        let uniform = slots.border;
        [
            slots.border_top.or(uniform),
            slots.border_right.or(uniform),
            slots.border_bottom.or(uniform),
            slots.border_left.or(uniform),
        ]
    }

    /// Per-side sRGB border colors. Longhands override `paint_colors.border`.
    pub fn resolved_border_edge_colors(&self) -> [Option<[f32; 4]>; 4] {
        self.resolved_border_side_colors()
            .map(|color| color.map(PaintColor::to_srgb))
    }

    /// First available border color (uniform or any side) resolved to sRGB.
    pub fn resolved_border_color(&self) -> Option<[f32; 4]> {
        if let Some(border) = self.paint_colors.border {
            return Some(border.to_srgb());
        }
        self.resolved_border_edge_colors()
            .into_iter()
            .flatten()
            .next()
    }

    /// Stroke widths for the existing rounded-box path (`none` / unsupported → 0).
    ///
    /// A side with no colour paints nothing, which is CSS's rule and the right
    /// one *for a colour authored in CSS*. An L3 component names its border as
    /// a [`SemanticColorRole`](crate::SemanticColorRole) instead, so the colour
    /// is resolved onto the computed style and never reaches this struct — see
    /// [`Self::paint_border_edges_with`].
    pub fn paint_border_edges(&self) -> PaddingSpec {
        self.paint_border_edges_with(None)
    }

    /// Stroke widths, with `fallback` standing in for a side that has no colour
    /// of its own on this struct.
    ///
    /// The caller passes the colour it is actually going to stroke with. Width
    /// and colour have to come from one decision: taking the width from the
    /// CSS-only view while taking the colour from the resolved semantic style
    /// is how an `Outlined` card asked for a 1px border, supplied a colour, and
    /// painted neither — `border_width: Some(1.0)` on the layout,
    /// `paint_colors.border: None` on the layout, and the role's colour sitting on the
    /// computed style where this function could not see it.
    pub fn paint_border_edges_with(&self, fallback: Option<[f32; 4]>) -> PaddingSpec {
        let layout = self.resolved_border_edges();
        let styles = self.resolved_border_styles();
        let colors = self.resolved_border_side_colors();
        let fb = fallback.map(PaintColor::srgb);
        PaddingSpec {
            top: paint_border_width(layout.top, styles[0], colors[0].or(fb)),
            right: paint_border_width(layout.right, styles[1], colors[1].or(fb)),
            bottom: paint_border_width(layout.bottom, styles[2], colors[2].or(fb)),
            left: paint_border_width(layout.left, styles[3], colors[3].or(fb)),
        }
    }

    /// Per-side colors with zero-alpha on sides that do not stroke.
    pub fn paint_border_edge_colors(&self) -> [[f32; 4]; 4] {
        let colors = self.resolved_border_edge_colors();
        let styles = self.resolved_border_styles();
        let mut out = [[0.0; 4]; 4];
        for (i, color) in colors.iter().enumerate() {
            if edge_paints_stroke(styles[i])
                && let Some(c) = *color
            {
                out[i] = c;
            }
        }
        out
    }

    /// Per-side shader codes (T,R,B,L): 0 solid, 1 dashed, 2 dotted.
    pub fn paint_border_style_codes(&self) -> [u8; 4] {
        let styles = self.resolved_border_styles();
        [
            styles[0].map(BorderStyle::shader_code).unwrap_or(0),
            styles[1].map(BorderStyle::shader_code).unwrap_or(0),
            styles[2].map(BorderStyle::shader_code).unwrap_or(0),
            styles[3].map(BorderStyle::shader_code).unwrap_or(0),
        ]
    }

    pub fn paints_any_border(&self) -> bool {
        let edges = self.paint_border_edges();
        !edges.is_zero()
    }

    /// Uniform stroke used by the current rounded-box shader (max of four sides).
    pub fn resolved_border_width(&self) -> f32 {
        let edges = self.resolved_border_edges();
        edges.top.max(edges.right).max(edges.bottom).max(edges.left)
    }

    /// Clip rect for descendants, expanded on axes that stay `overflow: visible`.
    pub fn overflow_clip_box(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<(f32, f32, f32, f32)> {
        overflow_clip_rect(
            x,
            y,
            width,
            height,
            self.overflow_x.clips(),
            self.overflow_y.clips(),
        )
    }

    /// Resolve padding to px without a containing-block width (`%` → 0).
    pub fn resolved_padding(&self) -> PaddingSpec {
        self.resolved_padding_against(None)
    }

    /// Resolve padding; `%` / calc 相对包含块宽度 `percent_base`。
    /// `em`/`rem` 用本节点 [`Self::font_size_context`]（缺省 16px）。
    pub fn resolved_padding_against(&self, percent_base: Option<f32>) -> PaddingSpec {
        self.resolved_padding_against_fonts(percent_base, self.font_size_context(16.0))
    }

    /// 同 [`Self::resolved_padding_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_padding_against_fonts(
        &self,
        percent_base: Option<f32>,
        fonts: FontSizeContext,
    ) -> PaddingSpec {
        resolve_box_edge_specs(
            self.padding,
            self.padding_top,
            self.padding_right,
            self.padding_bottom,
            self.padding_left,
            percent_base,
            fonts,
        )
    }

    /// Resolve margin to px without a containing-block width (`%` → 0).
    pub fn resolved_margin(&self) -> PaddingSpec {
        self.resolved_margin_against(None)
    }

    /// Resolve margin; `%` / calc 相对包含块宽度 `percent_base`（含上下边）。
    /// CSS 允许负 margin；不钳制到 0。
    pub fn resolved_margin_against(&self, percent_base: Option<f32>) -> PaddingSpec {
        self.resolved_margin_against_fonts(percent_base, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_margin_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_margin_against_fonts(
        &self,
        percent_base: Option<f32>,
        fonts: FontSizeContext,
    ) -> PaddingSpec {
        resolve_box_edge_specs_signed(
            self.margin,
            self.margin_top,
            self.margin_right,
            self.margin_bottom,
            self.margin_left,
            percent_base,
            fonts,
        )
    }

    /// Zero-clone margin + inset resolution for sticky positioning against
    /// `node_writing` without cloning or mutating `self`.
    pub fn resolved_sticky_edges_in(
        &self,
        node_writing: crate::WritingContext,
        margin_percent_base: Option<f32>,
        fonts: FontSizeContext,
    ) -> (PaddingSpec, [Option<LengthSpec>; 4]) {
        let mut m_sides = [
            self.margin_top,
            self.margin_right,
            self.margin_bottom,
            self.margin_left,
        ];
        let mut i_sides = [
            self.offset_top,
            self.offset_right,
            self.offset_bottom,
            self.offset_left,
        ];
        if !self.logical_edges.is_shared_default() {
            let le = &*self.logical_edges;
            if le.margin != LogicalEdges::default() {
                m_sides = le.margin.resolved_edges(node_writing, m_sides);
            }
            if le.inset != LogicalEdges::default() {
                i_sides = le.inset.resolved_edges(node_writing, i_sides);
            }
        }
        let margin = resolve_box_edge_specs_signed(
            self.margin,
            m_sides[0],
            m_sides[1],
            m_sides[2],
            m_sides[3],
            margin_percent_base,
            fonts,
        );
        (margin, i_sides)
    }

    /// `min-width` → 非负 px；无法解析时 0。
    pub fn resolved_min_width(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> f32 {
        self.resolved_min_width_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_min_width`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_min_width_fonts(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> f32 {
        resolve_min_size(self.min_width, percent_base, viewport, fonts)
    }

    /// `max-width` → 有限非负 px；`Fill`/`Auto`/`Shrink` 或无法解析 → `None`（不钳制）。
    pub fn resolved_max_width(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        self.resolved_max_width_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_max_width`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_max_width_fonts(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        resolve_max_size(self.max_width, percent_base, viewport, fonts)
    }

    /// `min-height` → 非负 px；无法解析时 0。
    pub fn resolved_min_height(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> f32 {
        self.resolved_min_height_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_min_height`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_min_height_fonts(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> f32 {
        resolve_min_size(self.min_height, percent_base, viewport, fonts)
    }

    /// `max-height` → 有限非负 px；无有限上限 → `None`。
    pub fn resolved_max_height(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
    ) -> Option<f32> {
        self.resolved_max_height_fonts(percent_base, viewport, FontSizeContext::default())
    }

    /// 同 [`Self::resolved_max_height`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolved_max_height_fonts(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        resolve_max_size(self.max_height, percent_base, viewport, fonts)
    }

    /// `min-width: 0` / `0px` / `0%` 等零值。
    pub fn has_zero_min_width(&self) -> bool {
        matches!(self.min_width, Some(LengthSpec::Px(v)) if v == 0.0)
            || matches!(self.min_width, Some(LengthSpec::Percent(p)) if p == 0.0)
            || matches!(self.min_width, Some(LengthSpec::Em(v) | LengthSpec::Rem(v)) if v == 0.0)
    }

    /// `position: relative` 画盒偏移（逻辑像素）。无 `%` 基时仅兑现 `Px`。
    pub fn relative_offset(&self) -> (f32, f32) {
        self.relative_offset_against(None, None)
    }

    /// `relative` inset；`%` 相对 containing block 宽/高。优先 `left`/`top`。
    pub fn relative_offset_against(&self, base_w: Option<f32>, base_h: Option<f32>) -> (f32, f32) {
        self.relative_offset_against_fonts(base_w, base_h, FontSizeContext::default())
    }

    /// 同 [`Self::relative_offset_against`]，携带显式 `em`/`rem` 字号上下文。
    pub fn relative_offset_against_fonts(
        &self,
        base_w: Option<f32>,
        base_h: Option<f32>,
        fonts: FontSizeContext,
    ) -> (f32, f32) {
        if !self.position.applies_relative_offset() {
            return (0.0, 0.0);
        }
        let dx = self
            .offset_left
            .and_then(|l| l.resolve_with_fonts(base_w, None, fonts))
            .or_else(|| {
                self.offset_right
                    .and_then(|r| r.resolve_with_fonts(base_w, None, fonts))
                    .map(|v| -v)
            })
            .unwrap_or(0.0);
        let dy = self
            .offset_top
            .and_then(|t| t.resolve_with_fonts(base_h, None, fonts))
            .or_else(|| {
                self.offset_bottom
                    .and_then(|b| b.resolve_with_fonts(base_h, None, fonts))
                    .map(|v| -v)
            })
            .unwrap_or(0.0);
        (dx, dy)
    }

    /// Resolve a single inset against a containing-block edge length.
    /// `em`/`rem` 使用 [`FontSizeContext::default`]（16px）。
    pub fn resolve_inset(spec: Option<LengthSpec>, base: f32) -> Option<f32> {
        Self::resolve_inset_fonts(spec, base, FontSizeContext::default())
    }

    /// 同 [`Self::resolve_inset`]，携带显式 `em`/`rem` 字号上下文。
    pub fn resolve_inset_fonts(
        spec: Option<LengthSpec>,
        base: f32,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        spec.and_then(|s| s.resolve_with_fonts(Some(base), None, fonts))
    }

    pub fn is_absolute(&self) -> bool {
        self.position.is_out_of_flow_absolute()
    }

    pub fn is_fixed(&self) -> bool {
        self.position.is_out_of_flow_fixed()
    }

    /// `absolute` 或 `fixed`：脱正常文档流。
    pub fn is_out_of_flow(&self) -> bool {
        self.position.is_out_of_flow()
    }

    /// Ellipsis paint intent (`text-overflow` or `line-clamp`).
    pub fn uses_text_ellipsis(&self) -> bool {
        self.text_overflow_ellipsis || self.resolved_line_clamp().is_some()
    }

    /// `line-clamp` / `-webkit-line-clamp` line cap.
    pub fn resolved_line_clamp(&self) -> Option<u16> {
        if self.text_ext.is_shared_default() {
            return None;
        }
        self.text_ext.line_clamp.filter(|n| *n > 0)
    }

    /// Soft wrap for text layout / Scene. `line-clamp` forces wrap.
    pub fn text_wraps(&self) -> bool {
        self.resolved_line_clamp().is_some() || self.white_space.wraps()
    }

    /// Wrap break once [`Self::text_wraps`] is true.
    pub fn text_wrap_break(&self) -> TextWrapBreak {
        if self.text_ext.is_shared_default() {
            return TextWrapBreak::Word;
        }
        let ext = &*self.text_ext;
        if matches!(ext.word_break, Some(WordBreakSpec::BreakAll)) {
            TextWrapBreak::Glyph
        } else if matches!(ext.overflow_wrap, Some(OverflowWrapSpec::Anywhere)) {
            TextWrapBreak::WordOrGlyph
        } else {
            TextWrapBreak::Word
        }
    }

    /// In-flow container auto width is stretch-fit unless shrink-to-fit.
    /// Out-of-flow and inline-level boxes shrink.
    pub fn stretch_fit_inline(&self) -> bool {
        !self.is_inline_level() && !self.is_out_of_flow()
    }

    /// Apply `aspect-ratio` to used **border-box** extents.
    ///
    /// CSS: a definite used width fills an automatic height. Block `width:auto`
    /// is stretch-fit (already the CB) so both axes then definite — the ratio
    /// does **not** shrink the inline size from a definite height.
    ///
    /// The ratio relates the boxes [`Self::box_sizing`] names (CSS Sizing 4
    /// §5.1): border boxes, or the content boxes inside `chrome` (padding plus
    /// border, horizontal then vertical). A border box is never shorter than
    /// its own chrome.
    pub fn apply_aspect_ratio_used(
        &self,
        width: &mut Option<f32>,
        height: &mut Option<f32>,
        chrome: (f32, f32),
    ) {
        let Some(ratio) = self.aspect_ratio.filter(|r| r.is_finite() && *r > 0.0) else {
            return;
        };
        let (chrome_w, chrome_h) = chrome;
        if let (Some(w), None) = (*width, *height)
            && w > 0.0
        {
            *height = Some(match self.box_sizing {
                BoxSizing::BorderBox => (w / ratio).max(chrome_h),
                BoxSizing::ContentBox => (w - chrome_w).max(0.0) / ratio + chrome_h,
            });
        }
    }

    /// CSS `direction: rtl` used value (`None` / `ltr` → false).
    pub fn is_rtl(&self) -> bool {
        matches!(self.dir, Some(DirSpec::Rtl))
    }

    /// Map stored logical edges onto used `padding_*` / `margin_*` /
    /// `offset_*` for the box's current [`Self::writing_context`].
    ///
    /// Only the edges a declaration decides are written (see
    /// [`LogicalEdges::resolve_into`]), so hand-built physical edges survive.
    /// A style that wants cascade order between a logical and a physical
    /// declaration of one edge records the physical one with
    /// [`LogicalEdges::set_phys`].
    pub fn resolve_logical_box_edges(&mut self) {
        self.resolve_logical_box_edges_in(self.writing_context());
    }

    /// True when any padding / margin / inset edge was declared logically,
    /// so its physical edge depends on the writing context.
    pub fn has_logical_box_edges(&self) -> bool {
        if self.logical_edges.is_shared_default() {
            return false;
        }
        let le = &*self.logical_edges;
        le.padding.has_logical() || le.margin.has_logical() || le.inset.has_logical()
    }

    /// Whether a logical edge group holds physical records and no logical
    /// edge. CSS `top` or `padding-left` keeps such a record so that a
    /// logical declaration can be ordered against it. With no logical edge in
    /// the group, the record decides nothing: the physical field holds the
    /// value.
    pub fn has_inert_physical_edges(&self) -> bool {
        if self.logical_edges.is_shared_default() {
            return false;
        }
        let le = &*self.logical_edges;
        le.padding.has_only_physical()
            || le.margin.has_only_physical()
            || le.inset.has_only_physical()
    }

    /// Drop the records [`Self::has_inert_physical_edges`] finds, so the
    /// same box states one intent whether CSS or Rust wrote its edges.
    pub fn drop_inert_physical_edges(&mut self) {
        if self.logical_edges.is_shared_default() {
            return;
        }
        let le = &*self.logical_edges;
        if !le.padding.has_only_physical()
            && !le.margin.has_only_physical()
            && !le.inset.has_only_physical()
        {
            return;
        }
        let le_mut = &mut *self.logical_edges;
        for group in [&mut le_mut.padding, &mut le_mut.margin, &mut le_mut.inset] {
            if group.has_only_physical() {
                *group = Default::default();
            }
        }
        if *self.logical_edges == LogicalBoxEdges::default() {
            self.logical_edges = crate::shared::Shared::empty();
        }
    }

    /// [`Self::resolve_logical_box_edges`] against an explicit context: the
    /// one the box inherits when it declares no `writing-mode` / `direction`
    /// of its own.
    pub fn resolve_logical_box_edges_in(&mut self, context: crate::WritingContext) {
        if self.logical_edges.is_shared_default() {
            return;
        }
        let default_edges = LogicalEdges::default();
        let (do_pad, do_mar, do_ins) = {
            let le = &*self.logical_edges;
            (
                le.padding != default_edges,
                le.margin != default_edges,
                le.inset != default_edges,
            )
        };
        if !do_pad && !do_mar && !do_ins {
            return;
        }
        let le = &mut *self.logical_edges;
        if do_pad {
            le.padding.resolve_into(
                context,
                &mut self.padding_top,
                &mut self.padding_right,
                &mut self.padding_bottom,
                &mut self.padding_left,
            );
        }
        if do_mar {
            le.margin.resolve_into(
                context,
                &mut self.margin_top,
                &mut self.margin_right,
                &mut self.margin_bottom,
                &mut self.margin_left,
            );
        }
        if do_ins {
            le.inset.resolve_into(
                context,
                &mut self.offset_top,
                &mut self.offset_right,
                &mut self.offset_bottom,
                &mut self.offset_left,
            );
        }
    }

    /// Fill unset inherited typography from `parent` (CSS inheritance).
    pub fn inherit_typography_from(&mut self, parent: &Self) {
        if self.dir.is_none() {
            self.dir = parent.dir;
        }
        if self.writing_mode.is_none() {
            self.writing_mode = parent.writing_mode;
        }
        self.resolve_logical_box_edges();
        if self.font_size.is_none() {
            self.font_size = parent.font_size;
        }
        if self.font_weight.is_none() {
            self.font_weight = parent.font_weight;
        }
        if self.font_italic.is_none() {
            self.font_italic = parent.font_italic;
        }
        if self.line_height.is_none() {
            self.line_height = parent.line_height;
        }
        if self.paint_colors.color.is_none() && parent.paint_colors.color.is_some() {
            self.paint_colors.color = parent.paint_colors.color;
        }
        if parent.text_ext.is_shared_default() {
            return;
        }
        let p = &*parent.text_ext;
        let cur = &*self.text_ext;
        let needs_copy = (p.word_break.is_some() && cur.word_break.is_none())
            || (p.overflow_wrap.is_some() && cur.overflow_wrap.is_none())
            || (p.font_family.is_some() && cur.font_family.is_none())
            || (p.letter_spacing.is_some() && cur.letter_spacing.is_none())
            || (p.text_decoration.is_some() && cur.text_decoration.is_none())
            || (p.text_emphasis_style.is_some() && cur.text_emphasis_style.is_none())
            || (p.text_emphasis_position.is_some() && cur.text_emphasis_position.is_none())
            || (p.text_emphasis_color.is_some() && cur.text_emphasis_color.is_none())
            || (p.font_features.is_some() && cur.font_features.is_none())
            || (p.font_variation_settings.is_some() && cur.font_variation_settings.is_none())
            || (p.font_kerning.is_some() && cur.font_kerning.is_none())
            || (p.line_break.is_some() && cur.line_break.is_none());
        if !needs_copy {
            return;
        }
        let ext = &mut *self.text_ext;
        if ext.word_break.is_none() {
            ext.word_break = p.word_break;
        }
        if ext.overflow_wrap.is_none() {
            ext.overflow_wrap = p.overflow_wrap;
        }
        if ext.font_family.is_none() {
            ext.font_family = p.font_family.clone();
        }
        if ext.letter_spacing.is_none() {
            ext.letter_spacing = p.letter_spacing;
        }
        if ext.text_decoration.is_none() {
            ext.text_decoration = p.text_decoration;
        }
        if ext.text_emphasis_style.is_none() {
            ext.text_emphasis_style = p.text_emphasis_style.clone();
        }
        if ext.text_emphasis_position.is_none() {
            ext.text_emphasis_position = p.text_emphasis_position;
        }
        if ext.text_emphasis_color.is_none() {
            ext.text_emphasis_color = p.text_emphasis_color;
        }
        if ext.font_features.is_none() {
            ext.font_features = p.font_features.clone();
        }
        if ext.font_variation_settings.is_none() {
            ext.font_variation_settings = p.font_variation_settings.clone();
        }
        if ext.font_kerning.is_none() {
            ext.font_kerning = p.font_kerning;
        }
        if ext.line_break.is_none() {
            ext.line_break = p.line_break;
        }
    }

    /// `em`/`rem` context for this element's lengths (`em` uses computed font-size).
    pub fn font_size_context(&self, root_px: f32) -> FontSizeContext {
        let root = root_px.max(0.0);
        FontSizeContext::new(root, self.font_size.unwrap_or(root))
    }

    pub fn grows(&self) -> bool {
        self.flex_grow.map(|g| g > 0.0).unwrap_or(false)
    }

    pub fn scrolls_y(&self) -> bool {
        self.overflow_y.scrolls()
    }

    /// Either axis is `overflow: hidden` / `clip` — paint must clip children.
    pub fn clips_overflow(&self) -> bool {
        self.overflow_x.clips() || self.overflow_y.clips()
    }

    /// This box resolves against the viewport, not only against its containing
    /// block: `position: fixed`, or any `vw` / `vh` / `vmin` / `vmax` length.
    ///
    /// A viewport change moves such a box even when its containing block keeps
    /// the exact same size, so incremental relayout cannot reuse it.
    pub fn depends_on_viewport(&self) -> bool {
        self.viewport_axes().any()
    }

    /// The sides of the viewport this box's own layout reads: those its
    /// lengths name, and both for a `position: fixed` box, whose containing
    /// block is the viewport.
    pub fn viewport_axes(&self) -> ViewportAxes {
        if self.position == PositionSpec::Fixed {
            return ViewportAxes::BOTH;
        }
        let viewport = |length: &Option<LengthSpec>| {
            length.map_or(ViewportAxes::NONE, LengthSpec::viewport_axes)
        };
        let logical = |edges: &LogicalEdges| {
            [
                &edges.inline_start,
                &edges.inline_end,
                &edges.block_start,
                &edges.block_end,
                &edges.phys_top,
                &edges.phys_right,
                &edges.phys_bottom,
                &edges.phys_left,
            ]
            .into_iter()
            .map(viewport)
            .fold(ViewportAxes::NONE, ViewportAxes::union)
        };
        let axes = [
            &self.gap,
            &self.row_gap,
            &self.column_gap,
            &self.padding,
            &self.padding_top,
            &self.padding_right,
            &self.padding_bottom,
            &self.padding_left,
            &self.margin,
            &self.margin_top,
            &self.margin_right,
            &self.margin_bottom,
            &self.margin_left,
            &self.offset_top,
            &self.offset_right,
            &self.offset_bottom,
            &self.offset_left,
            &self.width,
            &self.height,
            &self.min_width,
            &self.max_width,
            &self.min_height,
            &self.max_height,
            &self.flex_basis,
        ]
        .into_iter()
        .map(viewport)
        .fold(ViewportAxes::NONE, ViewportAxes::union);
        let axes = if let VerticalAlignSpec::Offset(len) = self.resolved_vertical_align() {
            axes.union(len.viewport_axes())
        } else {
            axes
        };
        // The logical group is nearly always the shared default: skip it
        // without reading its 24 lengths.
        if self.logical_edges.is_shared_default() {
            axes
        } else {
            let le = &*self.logical_edges;
            axes.union(logical(&le.padding))
                .union(logical(&le.margin))
                .union(logical(&le.inset))
        }
    }

    /// Internal `hidden` or `display: none` — skip layout flow.
    ///
    /// CSS `visibility: hidden` does **not** omit the box; see
    /// [`Self::is_paint_visible`].
    pub fn omits_box(&self) -> bool {
        self.hidden || matches!(self.display, Some(DisplaySpec::None))
    }

    /// Whether any field on this `LayoutStyle` can change a node's resolved
    /// `ComputedStyle`. Layout-only properties (`display: flex`, `gap`,
    /// `padding`, sizing, radii, borders width, shadows, etc.) leave this
    /// `false` so children can share their parent's resolved computed style.
    pub fn overrides_computed_style(&self) -> bool {
        self.opacity.is_some()
            || self.paint.visibility.is_some()
            || self.omits_box()
            || self.pointer_events.is_some()
            || self.inert
            || self.cursor.is_some()
            || self.user_select.is_some()
            || self.font_size.is_some()
            || self.font_weight.is_some()
            || self.font_italic.is_some()
            || self.line_height.is_some()
            || self.dir.is_some()
            || self.writing_mode.is_some()
            || !self.paint_colors.is_shared_default()
            || !self.theme.is_shared_default()
            || (!self.text_ext.is_shared_default() && self.text_ext.overrides_computed_style())
    }

    /// Generates a layout / paint box.
    pub fn generates_box(&self) -> bool {
        !self.omits_box()
    }

    /// 行内级盒子（`inline` / `inline-flex` / `inline-grid`；flex/grid 项会被块化）。
    pub fn is_inline_level(&self) -> bool {
        self.display.is_some_and(DisplaySpec::is_inline_level)
    }

    /// 某边 margin 是否为 `auto`（含 `margin: 0 auto` 简写）。
    pub fn margin_auto_left(&self) -> bool {
        edge_is_auto(self.margin_left, self.margin)
    }

    pub fn margin_auto_right(&self) -> bool {
        edge_is_auto(self.margin_right, self.margin)
    }

    pub fn margin_auto_top(&self) -> bool {
        edge_is_auto(self.margin_top, self.margin)
    }

    pub fn margin_auto_bottom(&self) -> bool {
        edge_is_auto(self.margin_bottom, self.margin)
    }

    /// 第一行基线相对 border-box 顶边的近似偏移。
    pub fn approximate_baseline(&self, fallback_font_px: f32) -> f32 {
        let font = self.font_size.unwrap_or(fallback_font_px).max(0.0);
        let pad = self.resolved_padding_against(None);
        let border = self.resolved_border_edges();
        pad.top + border.top + font * 0.8
    }

    /// 第一行基线相对 border-box 顶边。
    ///
    /// `shaped_ascent` 来自 `nana-text` 排出的 `TextMetrics`（第一行 `line_y - line_top`）。
    /// 缺省仍用 [`TEXT_APPROX_ASCENT_EM`]（0.8em）。
    pub fn baseline_from_ascent(&self, fallback_font_px: f32, shaped_ascent: Option<f32>) -> f32 {
        let font = self.font_size.unwrap_or(fallback_font_px).max(0.0);
        let pad = self.resolved_padding_against(None);
        let border = self.resolved_border_width();
        let ascent = shaped_ascent
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(font * TEXT_APPROX_ASCENT_EM);
        pad.top + border + ascent
    }

    /// CSS `direction` after inherit; initial is LTR.
    pub fn resolved_direction(&self) -> DirSpec {
        self.dir.unwrap_or(DirSpec::Ltr)
    }

    /// CSS `writing-mode` after inherit; initial is `horizontal-tb`.
    pub fn resolved_writing_mode(&self) -> WritingModeSpec {
        self.writing_mode.unwrap_or(WritingModeSpec::HorizontalTb)
    }

    /// The writing mode and direction this box lays out in: where its
    /// inline and block axes run and start. See [`crate::WritingContext`].
    pub fn writing_context(&self) -> crate::WritingContext {
        let orientation = if self.text_ext.is_shared_default() {
            TextOrientationSpec::default()
        } else {
            self.text_ext.text_orientation.unwrap_or_default()
        };
        crate::WritingContext::used(
            self.resolved_writing_mode(),
            self.resolved_direction(),
            orientation,
        )
    }

    /// Resolve the logical padding / margin / inset onto physical edges for
    /// the current writing mode and direction. The same as
    /// [`Self::resolve_logical_box_edges`], kept for the builder API's name.
    pub fn bake_logical_edges(&mut self) {
        self.resolve_logical_box_edges();
    }

    /// Update CSS `direction` and re-resolve logical edges onto physical ones.
    pub fn set_writing_direction(&mut self, next: DirSpec) {
        if self.dir == Some(next) {
            return;
        }
        self.dir = Some(next);
        self.resolve_logical_box_edges();
    }

    /// Update CSS `writing-mode` and re-resolve logical edges onto physical
    /// ones.
    pub fn set_writing_mode(&mut self, next: WritingModeSpec) {
        if self.writing_mode == Some(next) {
            return;
        }
        self.writing_mode = Some(next);
        self.resolve_logical_box_edges();
    }

    /// Update CSS `text-orientation` (`None` inherits) and re-resolve logical
    /// edges: `upright` changes the used direction of a vertical box.
    pub fn set_text_orientation(&mut self, next: Option<TextOrientationSpec>) {
        if self.text_ext.text_orientation == next {
            return;
        }
        self.text_ext.text_orientation = next;
        self.resolve_logical_box_edges();
    }

    /// CSS `vertical-align` for inline-level boxes (default [`VerticalAlignSpec::Baseline`]).
    #[inline]
    pub fn resolved_vertical_align(&self) -> VerticalAlignSpec {
        if self.text_ext.is_shared_default() {
            VerticalAlignSpec::Baseline
        } else {
            self.text_ext.vertical_align
        }
    }

    /// Set CSS `vertical-align` without mutating the shared default when already equal.
    pub fn set_vertical_align(&mut self, next: VerticalAlignSpec) {
        if self.resolved_vertical_align() == next {
            return;
        }
        self.text_ext.vertical_align = next;
    }

    /// CSS explicit width/height/min-*/border-radius overlay ControlSize tokens.
    pub fn overlay_css_size_overrides(&mut self, css: &Self) {
        if css.width.is_some() {
            self.width = css.width;
        }
        if css.height.is_some() {
            self.height = css.height;
            if css.min_height.is_none() {
                self.min_height = None;
            }
        }
        if css.min_width.is_some() {
            self.min_width = css.min_width;
        }
        if css.min_height.is_some() {
            self.min_height = css.min_height;
        }
        if css.max_width.is_some() {
            self.max_width = css.max_width;
        }
        if css.max_height.is_some() {
            self.max_height = css.max_height;
        }
        if css.border_radius.is_some() {
            self.border_radius = css.border_radius;
        }
    }

    /// 查找命名网格线（1-based）。含 `name-start` / `name-end` 由 areas 推导。
    pub fn named_column_line(&self, name: &str) -> Option<i32> {
        self.named_column_line_nth(name, 1)
    }

    /// 第 `occurrence` 根同名列线（1-based occurrence，1 = 第一根）。
    pub fn named_column_line_nth(&self, name: &str, occurrence: u32) -> Option<i32> {
        self.named_column_line_nth_from(name, occurrence, None)
    }

    /// Like [`Self::named_column_line_nth`], but resolve against expanded names
    /// (auto-fit / repeat copies) when `names` is `Some`.
    pub fn named_column_line_nth_from(
        &self,
        name: &str,
        occurrence: u32,
        names: Option<&[Vec<String>]>,
    ) -> Option<i32> {
        let grid = &*self.grid;
        lookup_named_line_nth(
            name,
            names.or(grid.column_line_names.as_deref()),
            grid.template_areas.as_ref(),
            true,
            occurrence,
        )
    }

    /// 列线上 `after_line`（1-based）之后的下一根同名线。
    pub fn named_column_line_after(&self, name: &str, after_line: i32) -> Option<i32> {
        self.named_column_line_after_from(name, after_line, None)
    }

    pub fn named_column_line_after_from(
        &self,
        name: &str,
        after_line: i32,
        names: Option<&[Vec<String>]>,
    ) -> Option<i32> {
        lookup_named_line_after(
            name,
            names.or(self.grid.column_line_names.as_deref()),
            after_line,
        )
    }

    pub fn named_row_line(&self, name: &str) -> Option<i32> {
        self.named_row_line_nth(name, 1)
    }

    pub fn named_row_line_nth(&self, name: &str, occurrence: u32) -> Option<i32> {
        self.named_row_line_nth_from(name, occurrence, None)
    }

    pub fn named_row_line_nth_from(
        &self,
        name: &str,
        occurrence: u32,
        names: Option<&[Vec<String>]>,
    ) -> Option<i32> {
        let grid = &*self.grid;
        lookup_named_line_nth(
            name,
            names.or(grid.row_line_names.as_deref()),
            grid.template_areas.as_ref(),
            false,
            occurrence,
        )
    }

    pub fn named_row_line_after(&self, name: &str, after_line: i32) -> Option<i32> {
        self.named_row_line_after_from(name, after_line, None)
    }

    pub fn named_row_line_after_from(
        &self,
        name: &str,
        after_line: i32,
        names: Option<&[Vec<String>]>,
    ) -> Option<i32> {
        lookup_named_line_after(
            name,
            names.or(self.grid.row_line_names.as_deref()),
            after_line,
        )
    }

    /// `justify-self: auto` → 容器 `justify-items`（缺省 Stretch，对齐 CSS Grid）。
    pub fn resolved_justify_self(&self, parent_justify_items: Option<AlignSpec>) -> AlignSpec {
        self.justify_self
            .or(parent_justify_items)
            .unwrap_or(AlignSpec::Stretch)
    }

    /// Explicit `grid-template-columns` tracks that participate in layout.
    ///
    /// CSS: `grid-template-columns` is inert on flex containers. Tracks remain
    /// active when `display` is `grid` / `inline-grid` / unset (compat) or other
    /// non-flex values.
    ///
    /// Explicit columns from `grid-template-columns`, ignoring `repeat(auto-*)` and `flex` tracks.
    /// Implicit columns come from [`GridStyle::auto_columns`] / auto-placement.
    /// [`GridStyle::columns_repeat`] expands `repeat(auto-fit|auto-fill)` at layout.
    /// If [`GridStyle::columns_unsupported`] is set, the author wrote
    /// mixed/unexpandable syntax (not a successful `repeat(auto-fit|fill)`).
    pub fn active_grid_columns(&self) -> Option<&[GridTrack]> {
        if self.display.is_some_and(DisplaySpec::is_flex_container) {
            return None;
        }
        if self.display.is_some_and(DisplaySpec::is_layers) {
            return Some(LAYER_TRACKS);
        }
        if self.grid.is_shared_default() {
            return None;
        }
        self.grid.columns.as_deref().filter(|t| !t.is_empty())
    }

    /// Row tracks that participate in layout (see [`Self::active_grid_columns`]).
    ///
    /// Implicit rows come from [`GridStyle::auto_rows`] / auto-placement.
    pub fn active_grid_rows(&self) -> Option<&[GridTrack]> {
        if self.display.is_some_and(DisplaySpec::is_flex_container) {
            return None;
        }
        if self.display.is_some_and(DisplaySpec::is_layers) {
            return Some(LAYER_TRACKS);
        }
        if self.grid.is_shared_default() {
            return None;
        }
        self.grid.rows.as_deref().filter(|t| !t.is_empty())
    }

    /// Author wrote `grid-auto-*` / `grid-auto-flow` (consumed by 2D placement).
    pub fn has_deferred_grid_auto(&self) -> bool {
        if self.grid.is_shared_default() {
            return false;
        }
        let g = &*self.grid;
        g.auto_columns.as_ref().is_some_and(|t| !t.is_empty())
            || g.auto_rows.as_ref().is_some_and(|t| !t.is_empty())
            || g.auto_flow.is_some()
    }

    /// `grid-template-columns: subgrid` on a grid container (`grid` / `inline-grid`).
    pub fn is_subgrid_columns(&self) -> bool {
        self.display.is_some_and(DisplaySpec::is_grid_container)
            && !self.grid.is_shared_default()
            && self.grid.columns_subgrid
    }

    /// `grid-template-rows: subgrid` on a grid container (`grid` / `inline-grid`).
    pub fn is_subgrid_rows(&self) -> bool {
        self.display.is_some_and(DisplaySpec::is_grid_container)
            && !self.grid.is_shared_default()
            && self.grid.rows_subgrid
    }

    /// 作者写了无法展开的模板（track-list token `subgrid`、嵌套 auto-fit / auto-fill、混写语法等）。
    ///
    /// 成功的整值 `subgrid` 写在 [`GridStyle::columns_subgrid`] / [`GridStyle::rows_subgrid`]，
    /// 成功的 `repeat(auto-fit|auto-fill)` 写在 [`GridStyle::columns_repeat`] /
    /// [`GridStyle::rows_repeat`]，**均不**置本旗标。
    pub fn has_unsupported_grid_template(&self) -> bool {
        if self.grid.is_shared_default() {
            return false;
        }
        self.grid.columns_unsupported.is_some() || self.grid.rows_unsupported.is_some()
    }

    /// Paint-order stacking context (not compositing). Opacity groups are
    /// separate in Scene; this covers `isolation` and positioned + `z-index`.
    pub fn creates_paint_stacking_context(&self) -> bool {
        self.isolation || (self.z_index.is_some() && self.position.is_positioned())
    }

    /// 子项主轴 Length：Row→width，Column→height。
    pub fn child_main_length(&self, parent_direction: FlexDirection) -> Option<LengthSpec> {
        if self.grows() {
            return Some(LengthSpec::Fill);
        }
        if let Some(basis) = self.flex_basis
            && !matches!(basis, LengthSpec::Auto)
        {
            return Some(basis);
        }
        match parent_direction {
            FlexDirection::Row => self.width,
            FlexDirection::Column => self.height,
        }
    }

    /// 根据自身 LengthSpec + 父盒推算子级可用 ParentBox（定高链 / %）。
    ///
    /// `ParentBox` 始终是**内容区**（供后代 `%` / 定高链）：
    /// - `content-box` + 声明 `Px`/`%`/`calc`：声明值即内容宽/高，不减 chrome
    /// - `Fill` / `100%`→Fill / grow 分配：分配量是 **border-box**，内容区 =
    ///   分配 − padding − border（与 CSS 在 content-box 下对 flexed/filled
    ///   border-box 的内容区一致；勿把分配量直接当内容区）
    /// - `border-box`：声明/分配含 padding+border，内容区同样减 chrome
    pub fn resolve_content_box(&self, parent: ParentBox) -> ParentBox {
        self.resolve_content_box_with_viewport(parent, None)
    }

    /// Same as [`Self::resolve_content_box`], with viewport for `vw`/`vh`/`min()`…
    pub fn resolve_content_box_with_viewport(
        &self,
        parent: ParentBox,
        viewport: Option<(f32, f32)>,
    ) -> ParentBox {
        let pad = self.resolved_padding_against(parent.width);
        let border = self.resolved_border_edges();
        let content_box = matches!(self.box_sizing, BoxSizing::ContentBox);
        let chrome_w = pad.left + pad.right + border.left + border.right;
        let chrome_h = pad.top + pad.bottom + border.top + border.bottom;
        let mut width = self
            .width
            .and_then(|w| w.resolve_with(parent.width, viewport))
            .or_else(|| {
                if matches!(self.width, Some(LengthSpec::Fill))
                    || (self.width.is_none() && self.grows())
                {
                    parent.width
                } else {
                    None
                }
            })
            .map(|w| Self::axis_content_extent(w, chrome_w, content_box, self.width));
        let mut height = self
            .height
            .and_then(|h| h.resolve_with(parent.height, viewport))
            .or_else(|| {
                if matches!(self.height, Some(LengthSpec::Fill)) || self.grows() {
                    parent.height
                } else {
                    let mh = self.resolved_min_height(parent.height, viewport);
                    if mh > 0.0 { Some(mh) } else { None }
                }
            })
            .or(parent
                .height
                .filter(|_| matches!(self.height, Some(LengthSpec::Fill))))
            .map(|h| Self::axis_content_extent(h, chrome_h, content_box, self.height));
        if self.aspect_ratio.is_some_and(|r| r.is_finite() && r > 0.0) {
            if width.is_none() && self.stretch_fit_inline() {
                width = parent
                    .width
                    .map(|w| Self::axis_content_extent(w, chrome_w, content_box, self.width));
            }
            let mut border_w = width.map(|w| w + chrome_w);
            let mut border_h = height.map(|h| h + chrome_h);
            self.apply_aspect_ratio_used(&mut border_w, &mut border_h, (chrome_w, chrome_h));
            height = border_h.map(|h| (h - chrome_h).max(0.0));
        }
        ParentBox { width, height }
    }

    /// One axis of [`Self::resolve_content_box`]: map resolved/allocated size → content extent.
    fn axis_content_extent(
        resolved: f32,
        chrome: f32,
        content_box: bool,
        axis_spec: Option<LengthSpec>,
    ) -> f32 {
        if content_box && axis_spec.is_some_and(LengthSpec::is_definite_declared) {
            // Declared length is already the content size.
            resolved.max(0.0)
        } else {
            // Border-box declaration, or Fill/grow allocation (border-box share).
            (resolved - chrome).max(0.0)
        }
    }

    pub fn ensure_direction(&mut self, dir: FlexDirection) {
        if self.direction.is_none() {
            self.direction = Some(dir);
        }
    }
}

/// Positive definite length for `%` bases; `None` / `≤0` treated as indefinite.
fn definite_length(v: Option<f32>) -> Option<f32> {
    v.filter(|n| *n > 0.0)
}

fn resolve_corner_radius(spec: LengthSpec, width: f32, height: f32) -> f32 {
    match spec {
        LengthSpec::Px(v) => v.max(0.0),
        LengthSpec::Percent(p) => {
            let horizontal = width.max(0.0) * p / 100.0;
            let vertical = height.max(0.0) * p / 100.0;
            let raw = horizontal.min(vertical).max(0.0);
            let max_corner = width.min(height).max(0.0) / 2.0;
            raw.min(max_corner)
        }
        other => other
            .resolve_px(Some(width.max(0.0)))
            .unwrap_or(0.0)
            .max(0.0),
    }
}

fn edge_is_auto(longhand: Option<LengthSpec>, uniform: Option<LengthSpec>) -> bool {
    matches!(longhand.or(uniform), Some(LengthSpec::Auto))
}

fn resolve_edge_length(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) -> Option<f32> {
    spec.and_then(|s| match s {
        LengthSpec::Fill
        | LengthSpec::Shrink
        | LengthSpec::Auto
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent => None,
        other => other.resolve_non_negative_fonts(percent_base, None, fonts),
    })
}

/// Margin 边长：允许负值（CSS Values / Box Model）。
fn resolve_edge_length_signed(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) -> Option<f32> {
    spec.and_then(|s| match s {
        LengthSpec::Fill
        | LengthSpec::Shrink
        | LengthSpec::Auto
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent => None,
        other => other.resolve_with_fonts(percent_base, None, fonts),
    })
}

fn resolve_min_size(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    viewport: Option<(f32, f32)>,
    fonts: FontSizeContext,
) -> f32 {
    match spec {
        None
        | Some(LengthSpec::Auto)
        | Some(LengthSpec::Shrink)
        | Some(LengthSpec::Fill)
        | Some(LengthSpec::MinContent)
        | Some(LengthSpec::MaxContent)
        | Some(LengthSpec::FitContent) => 0.0,
        Some(other) => other
            .resolve_non_negative_fonts(percent_base, viewport, fonts)
            .unwrap_or(0.0),
    }
}

fn resolve_max_size(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    viewport: Option<(f32, f32)>,
    fonts: FontSizeContext,
) -> Option<f32> {
    match spec? {
        LengthSpec::Fill
        | LengthSpec::Auto
        | LengthSpec::Shrink
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent => None,
        other => other
            .resolve_non_negative_fonts(percent_base, viewport, fonts)
            .filter(|v| v.is_finite()),
    }
}

fn resolve_box_edge_specs(
    uniform: Option<LengthSpec>,
    top: Option<LengthSpec>,
    right: Option<LengthSpec>,
    bottom: Option<LengthSpec>,
    left: Option<LengthSpec>,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) -> PaddingSpec {
    let u = resolve_edge_length(uniform, percent_base, fonts).unwrap_or(0.0);
    PaddingSpec {
        top: resolve_edge_length(top, percent_base, fonts)
            .unwrap_or(u)
            .max(0.0),
        right: resolve_edge_length(right, percent_base, fonts)
            .unwrap_or(u)
            .max(0.0),
        bottom: resolve_edge_length(bottom, percent_base, fonts)
            .unwrap_or(u)
            .max(0.0),
        left: resolve_edge_length(left, percent_base, fonts)
            .unwrap_or(u)
            .max(0.0),
    }
}

fn resolve_box_edge_specs_signed(
    uniform: Option<LengthSpec>,
    top: Option<LengthSpec>,
    right: Option<LengthSpec>,
    bottom: Option<LengthSpec>,
    left: Option<LengthSpec>,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) -> PaddingSpec {
    let u = resolve_edge_length_signed(uniform, percent_base, fonts).unwrap_or(0.0);
    PaddingSpec {
        top: resolve_edge_length_signed(top, percent_base, fonts).unwrap_or(u),
        right: resolve_edge_length_signed(right, percent_base, fonts).unwrap_or(u),
        bottom: resolve_edge_length_signed(bottom, percent_base, fonts).unwrap_or(u),
        left: resolve_edge_length_signed(left, percent_base, fonts).unwrap_or(u),
    }
}
