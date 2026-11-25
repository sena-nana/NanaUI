//! Shared Runtime layout measure algorithms.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn intrinsic_size(
    id: StableNodeId,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
) -> Result<Size, UiWorldError> {
    intrinsic_size_scoped(
        id,
        available,
        parent_direction,
        viewport,
        parent_font_px,
        nodes,
        cache,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
/// Resolve the node's own width/height from its style alone.
///
/// `Some` on an axis means the used size does not depend on the node's
/// children: content-sized keywords (`min-content`, `max-content`,
/// `fit-content`, `shrink`) and an indefinite `Fill` all resolve to `None`.
fn resolved_size_specs(
    style: &nana_ui_core::LayoutStyle,
    available: Size,
    // What the node's percentage margins resolve against: its containing
    // block's inline size. See `UiWorld::containing_writing`.
    edge_base: f32,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
) -> (Option<f32>, Option<f32>) {
    // `Fill` sizes the border box to the containing block minus the node's own
    // margins — negative margins widen it, matching the stretch path below;
    // percentages keep resolving against the raw containing block.
    let margin = style.resolved_margin_against_fonts(Some(edge_base), fonts);
    let width = resolve_axis(
        demote_fill_spec_if_indefinite(style.width, available.width),
        available.width,
        available.width - margin.left - margin.right,
        viewport,
        fonts,
    );
    let height = resolve_axis(
        demote_fill_spec_if_indefinite(style.height, available.height),
        available.height,
        available.height - margin.top - margin.bottom,
        viewport,
        fonts,
    );
    (width, height)
}

/// Whether a childless node's used block size can change when only the
/// containing block's block size changes. Percent, fill, fit-content, and
/// calc stay sensitive. Reporting `false` for one of those republishes a
/// stale size from the first measurement.
pub(super) fn block_containing_size_affects_leaf(
    style: &nana_ui_core::LayoutStyle,
    writing: nana_ui_core::WritingContext,
) -> bool {
    if writing.is_vertical()
        || style
            .aspect_ratio
            .is_some_and(|ratio| ratio.is_finite() && ratio > 0.0)
    {
        return true;
    }
    !length_ignores_block_containing_size(style.height)
        || !length_ignores_block_containing_size(style.min_height)
        || !length_ignores_block_containing_size(style.max_height)
}

/// Whether `style` sizes its box on each physical axis by a length of its
/// own, reading no containing block: a definite width (height), limits that
/// read none, and no box edge that does. Horizontal writing without an
/// aspect ratio only; anything else says no.
pub(super) fn sizes_own_axes(
    style: &nana_ui_core::LayoutStyle,
    writing: nana_ui_core::WritingContext,
) -> (bool, bool) {
    if writing.is_vertical()
        || aspect_ratio_is_usable(style)
        || style.has_logical_box_edges()
        || style
            .flex_basis
            .is_some_and(|basis| !definite_own_length(basis))
        || box_edge_specs(style)
            .into_iter()
            .any(spec_tracks_containing_block)
    {
        return (false, false);
    }
    let own = |size: Option<LengthSpec>, min: Option<LengthSpec>, max: Option<LengthSpec>| {
        size.is_some_and(definite_own_length)
            && length_ignores_block_containing_size(min)
            && length_ignores_block_containing_size(max)
    };
    (
        own(style.width, style.min_width, style.max_width),
        own(style.height, style.min_height, style.max_height),
    )
}

/// Whether `style`'s own measurement reads the block extent its box is
/// offered, on a horizontal page: its height or a height limit resolved
/// against the containing block, a flex basis that does, an aspect ratio or
/// logical edges that carry a size across the axes, a declared vertical
/// writing mode, a column that wraps (the wrap budget is the offered
/// height), or any grid (rows and their fr and percentage tracks resolve
/// against it). Everything else a measurement reads -- padding, margins,
/// text, visuals, a natural size -- resolves against the inline size.
///
/// Every read of the offered block extent in this file must be one of
/// these; `world/block_reads.rs` keys measurements by it, and the
/// layout-verify reference ignores that key so a missing case shows.
pub(crate) fn reads_offered_block_extent(style: &nana_ui_core::LayoutStyle) -> bool {
    style.writing_mode.is_some_and(|mode| mode.is_vertical())
        || aspect_ratio_is_usable(style)
        || style.has_logical_box_edges()
        || !length_ignores_block_containing_size(style.height)
        || !length_ignores_block_containing_size(style.min_height)
        || !length_ignores_block_containing_size(style.max_height)
        || spec_tracks_containing_block(style.flex_basis)
        || (style.flex_wrap != nana_ui_core::FlexWrap::NoWrap
            && style.direction != Some(FlexDirection::Row))
        || style
            .display
            .is_some_and(nana_ui_core::DisplaySpec::is_grid_container)
        || (!style.grid.is_shared_default() && {
            let grid = &*style.grid;
            grid.rows.is_some()
                || grid.columns.is_some()
                || grid.rows_subgrid
                || grid.columns_subgrid
                || grid.rows_unsupported.is_some()
                || grid.columns_unsupported.is_some()
                || grid.rows_repeat.is_some()
                || grid.columns_repeat.is_some()
                || grid
                    .auto_rows
                    .as_ref()
                    .is_some_and(|tracks| !tracks.is_empty())
                || grid
                    .auto_columns
                    .as_ref()
                    .is_some_and(|tracks| !tracks.is_empty())
                || grid.template_areas.is_some()
        })
}

/// Whether `style` sizes its box's height by a length of its own on a
/// horizontal page, so what it offers its children comes from that length
/// and not from what it is offered.
pub(crate) fn sizes_own_height(style: &nana_ui_core::LayoutStyle) -> bool {
    sizes_own_axes(style, nana_ui_core::WritingContext::default()).1
}

/// A length that is the box's own: absolute, font-relative or
/// viewport-relative, never one resolved against the box's surroundings.
fn definite_own_length(spec: LengthSpec) -> bool {
    matches!(
        spec,
        LengthSpec::Px(_) | LengthSpec::Em(_) | LengthSpec::Rem(_) | LengthSpec::Viewport { .. }
    ) || matches!(spec, LengthSpec::Calc(expr) if !expr.inner().reads_percent())
}

pub(crate) fn length_ignores_block_containing_size(spec: Option<LengthSpec>) -> bool {
    spec.is_none_or(|spec| {
        definite_own_length(spec)
            || matches!(
                spec,
                LengthSpec::Auto
                    | LengthSpec::Shrink
                    | LengthSpec::MinContent
                    | LengthSpec::MaxContent
            )
    })
}

/// A definite declaration can still resolve against a containing block. Such
/// used values are valid only for the current query and must not be published
/// as intrinsic facts.
pub(crate) fn depends_on_used_basis(spec: Option<LengthSpec>) -> bool {
    spec.is_some_and(|spec| {
        !matches!(
            spec,
            LengthSpec::Px(_) | LengthSpec::Em(_) | LengthSpec::Rem(_)
        ) && spec.as_em_offset().is_none()
            && spec.as_rem_offset().is_none()
    })
}

/// Percent, fill, fit-content, and calc that read a containing block.
pub(crate) fn spec_tracks_containing_block(spec: Option<LengthSpec>) -> bool {
    depends_on_used_basis(spec) && !length_ignores_block_containing_size(spec)
}

/// Every padding and margin spec of `style`: each shorthand, then its sides.
pub(super) fn box_edge_specs(style: &nana_ui_core::LayoutStyle) -> [Option<LengthSpec>; 10] {
    [
        style.padding,
        style.padding_top,
        style.padding_right,
        style.padding_bottom,
        style.padding_left,
        style.margin,
        style.margin_top,
        style.margin_right,
        style.margin_bottom,
        style.margin_left,
    ]
}

/// Compose the used size once the content-derived defaults are known.
///
/// `default_width` / `default_height` are consumed only through `unwrap_or`, so
/// a caller that already has both specs resolved may pass anything for them —
/// see the definite-size short circuit in [`intrinsic_size_scoped`].
#[allow(clippy::too_many_arguments)]
fn finish_intrinsic_size(
    style: &nana_ui_core::LayoutStyle,
    fonts: FontSizeContext,
    viewport: LayoutViewport,
    available: Size,
    edge_base: f32,
    chrome: Size,
    parent_direction: Option<FlexDirection>,
    default_width: f32,
    default_height: f32,
) -> Size {
    let (width_spec, height_spec) =
        resolved_size_specs(style, available, edge_base, viewport, fonts);
    let width_from_spec = width_spec.is_some();
    let height_from_spec = height_spec.is_some();
    let vp = Some((viewport.width, viewport.height));
    let min_width = style.resolved_min_width_fonts(Some(available.width), vp, fonts);
    let min_height = style.resolved_min_height_fonts(Some(available.height), vp, fonts);
    let mut width = width_spec.unwrap_or(default_width).max(min_width);
    let mut height = height_spec.unwrap_or(default_height).max(min_height);
    if matches!(style.box_sizing, BoxSizing::ContentBox) {
        if style.width.is_some_and(LengthSpec::is_definite_declared) {
            width += chrome.width;
        }
        if style.height.is_some_and(LengthSpec::is_definite_declared) {
            height += chrome.height;
        }
    }
    if style.aspect_ratio.is_some_and(|r| r.is_finite() && r > 0.0) {
        let stretch_fit_width = !width_from_spec
            && style.stretch_fit_inline()
            && !matches!(parent_direction, Some(FlexDirection::Row))
            && available.width > 0.5;
        if stretch_fit_width {
            width = available.width.max(min_width);
        }
        // The ratio reads the used width, after its limits; the height it
        // gives then meets its own (CSS Sizing 4 §5.1).
        if let Some(max) = style.resolved_max_width_fonts(Some(available.width), vp, fonts) {
            width = width.min(max);
        }
        let mut used_w =
            (width_from_spec || stretch_fit_width || (!height_from_spec && width > 0.0))
                .then_some(width);
        let mut used_h = height_from_spec.then_some(height);
        style.apply_aspect_ratio_used(&mut used_w, &mut used_h, (chrome.width, chrome.height));
        if let Some(used_h) = used_h {
            height = used_h.max(min_height);
        }
    }
    if let Some(max) = style.resolved_max_width_fonts(Some(available.width), vp, fonts) {
        width = width.min(max);
    }
    if let Some(max) = style.resolved_max_height_fonts(Some(available.height), vp, fonts) {
        height = height.min(max);
    }
    Size::new(width, height)
}

pub(super) fn intrinsic_size_scoped(
    id: StableNodeId,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Size, UiWorldError> {
    measure_node(
        id,
        None,
        available,
        parent_direction,
        viewport,
        parent_font_px,
        nodes,
        cache,
        scope,
    )
}

/// A flex item measured at the main size its line gave it (`main`, border
/// box, along `direction`): its hypothetical cross size with the used main
/// size (CSS flexbox §9.4). A `Fill` column that shrank beside a fixed one
/// is as tall as its content at its used width, not at the width it was
/// first measured with. `available` is the container's content box, what
/// the item's percentages resolve against.
#[allow(clippy::too_many_arguments)]
pub(super) fn intrinsic_size_at_main(
    id: StableNodeId,
    main: f32,
    direction: FlexDirection,
    available: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Size, UiWorldError> {
    let Some(style) = nodes.style_ref(id) else {
        return Ok(Size::default());
    };
    let mut forced = (*style).clone();
    // Border box, so the chrome is inside `main` whatever the item's
    // `box-sizing`; the cross axis is not definite (the caller checks), so
    // `box-sizing` changes nothing there.
    forced.box_sizing = BoxSizing::BorderBox;
    let main = Some(LengthSpec::Px(main.max(0.0)));
    match direction {
        FlexDirection::Row => forced.width = main,
        FlexDirection::Column => forced.height = main,
    }
    measure_node(
        id,
        Some(Arc::new(forced)),
        available,
        Some(direction),
        viewport,
        parent_font_px,
        nodes,
        cache,
        scope,
    )
}

/// An out-of-flow box with an auto height, measured for its content at
/// `available`: its containing block's width, and no definite height, so
/// what it holds has no height to fill or take a percentage of.
///
/// Its own percentage `min-height` / `max-height` still read that
/// containing block, whose height is definite for a positioned box (CSS 2.1
/// §10.7): the viewport for `fixed`. The caller applies them against it, so
/// they are left out here rather than resolved against the indefinite
/// height, where `max-height: 80%` came to 0 and held the box at 0 tall.
#[allow(clippy::too_many_arguments)]
pub(super) fn intrinsic_size_out_of_flow(
    id: StableNodeId,
    available: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Size, UiWorldError> {
    let Some(style) = nodes.style_ref(id) else {
        return Ok(Size::default());
    };
    let (min_reads_block, max_reads_block) = (
        spec_tracks_containing_block(style.min_height),
        spec_tracks_containing_block(style.max_height),
    );
    if !min_reads_block && !max_reads_block {
        return intrinsic_size_scoped(
            id,
            available,
            None,
            viewport,
            parent_font_px,
            nodes,
            cache,
            scope,
        );
    }
    let mut forced = (*style).clone();
    if min_reads_block {
        forced.min_height = None;
    }
    if max_reads_block {
        forced.max_height = None;
    }
    measure_node(
        id,
        Some(Arc::new(forced)),
        available,
        None,
        viewport,
        parent_font_px,
        nodes,
        cache,
        scope,
    )
}

/// One axis set and the other not: a replaced box's natural aspect ratio
/// carries the set one over, content box to content box (Issue #263), and
/// the other axis's min/max still bound the result. Neither set: the natural
/// size keeps its ratio under min/max as CSS 2.1 §10.4 lays out for replaced
/// elements (Issue #202). Every path that resolves a replaced box's used size
/// goes through here, so a reused measurement cannot answer with the natural
/// other axis instead.
fn transfer_natural_ratio(
    style: &nana_ui_core::LayoutStyle,
    natural: Option<crate::ReplacedMetadata>,
    chrome: Size,
    available: Size,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
    size: &mut Size,
) {
    let Some(natural) = natural.filter(|natural| natural.width > 0.0 && natural.height > 0.0)
    else {
        return;
    };
    if style.aspect_ratio.is_some() {
        return;
    }
    let auto = |spec: Option<LengthSpec>| matches!(spec, None | Some(LengthSpec::Auto));
    // The limits as `finish_intrinsic_size` applies them, on the content box.
    let vp = Some((viewport.width, viewport.height));
    let limits = |min: f32, max: Option<f32>, chrome: f32| {
        let min = (min - chrome).max(0.0);
        (
            min,
            max.map_or(f32::INFINITY, |max| (max - chrome).max(min)),
        )
    };
    let (min_w, max_w) = limits(
        style.resolved_min_width_fonts(Some(available.width), vp, fonts),
        style.resolved_max_width_fonts(Some(available.width), vp, fonts),
        chrome.width,
    );
    let (min_h, max_h) = limits(
        style.resolved_min_height_fonts(Some(available.height), vp, fonts),
        style.resolved_max_height_fonts(Some(available.height), vp, fonts),
        chrome.height,
    );
    let ratio = natural.width / natural.height;
    match (auto(style.width), auto(style.height)) {
        (false, true) => {
            let inner = (size.width - chrome.width).max(0.0);
            size.height = (inner / ratio).clamp(min_h, max_h) + chrome.height;
        }
        (true, false) => {
            let inner = (size.height - chrome.height).max(0.0);
            size.width = (inner * ratio).clamp(min_w, max_w) + chrome.width;
        }
        (true, true) => {
            let (mut width, mut height) = (natural.width, natural.height);
            if !(min_w..=max_w).contains(&width) || !(min_h..=max_h).contains(&height) {
                if width > max_w || width < min_w {
                    width = width.clamp(min_w, max_w);
                    height = width / ratio;
                }
                if height > max_h || height < min_h {
                    height = height.clamp(min_h, max_h);
                    width = height * ratio;
                }
                size.width = width.clamp(min_w, max_w) + chrome.width;
                size.height = height.clamp(min_h, max_h) + chrome.height;
            }
        }
        (false, false) => {}
    }
}

/// Whether a flex item's cross size is its content's at its main size, so
/// a line that gives it another main size than it was measured with must
/// measure it again ([`intrinsic_size_at_main`]).
pub(super) fn cross_follows_used_main(
    style: &nana_ui_core::LayoutStyle,
    direction: FlexDirection,
    measured: Size,
    used: Size,
) -> bool {
    let cross = match direction {
        FlexDirection::Row => style.height,
        FlexDirection::Column => style.width,
    };
    let content_sized =
        cross.is_none_or(|spec| spec == LengthSpec::Auto || spec.is_content_sized());
    // A row item with an aspect ratio of its own takes its height from its
    // used width after placement (`fill_auto_height_from_aspect_ratio`).
    let transferred = direction == FlexDirection::Row && aspect_ratio_is_usable(style);
    content_sized
        && !transferred
        && (main_extent(measured, direction) - main_extent(used, direction)).abs() > 0.01
}

/// The inline size a text asks of a box `limit` wide.
///
/// Text that wrapped is as wide as the lines it wrapped to. The width it
/// asks for is its lines unwrapped, as far as this box gives it room: a box
/// that shrinks to its content widens again once its limit grows, and then
/// the text rewraps to the new width. The lines' own width counts only when
/// they were wrapped no wider than this box (`wrap_limit`): a word too long
/// to break keeps such a line, and the box, wider. A widest line that cannot
/// break counts as wrapped no wider than any box (`UiWorld::text_wrap_limit`),
/// so the box stays that wide once the text is shaped again in the box the
/// line widened, where it fits. Lines wrapped against a wider box, or against
/// none before the text had a box, are not lines this box holds; asking for
/// them would keep a width the text no longer has, so the result would
/// depend on which box the text was shaped in last.
fn text_inline_size(
    text: crate::TextMetrics,
    natural: Option<f32>,
    wrap_limit: Option<f32>,
    limit: f32,
) -> f32 {
    let fits = natural.unwrap_or(text.width).min(limit);
    let wrapped_within = wrap_limit.is_none_or(|wrapped| wrapped <= limit + 0.5);
    if wrapped_within {
        text.width.max(fits)
    } else {
        fits
    }
}

/// What a node's standard visual draws in its content box beside its text:
/// a button's glyphs, a checkbox's indicator, the arrow of a select that
/// sizes to its options. Measured with the text, and compared by a measure
/// plan: a visual change moves it without touching the node's style or text.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct VisualContent {
    /// Width added beside the text.
    width: f32,
    /// The least content height.
    height: f32,
    /// The natural width of text the visual draws itself rather than as the
    /// node's text (a switch's label and hint): it asks for that much beside
    /// `width`, as far as the box gives it room, the way node text does.
    text: f32,
    /// The parts laid out before the children (#385).
    lead: Option<super::parts::LeadingParts>,
}

impl VisualContent {
    /// The default, to the bit: no width, height or text (not even `-0.0`)
    /// and no leading parts.
    pub(super) fn is_nothing(&self) -> bool {
        self.width.to_bits() == 0
            && self.height.to_bits() == 0
            && self.text.to_bits() == 0
            && self.lead.is_none()
    }
}

fn visual_content(id: StableNodeId, world: &UiWorld) -> VisualContent {
    VisualContent {
        lead: world.leading_parts(id),
        ..visual_beside_text(id, world)
    }
}

fn visual_beside_text(id: StableNodeId, world: &UiWorld) -> VisualContent {
    world
        .widget_visual_ref(id)
        .map_or_else(VisualContent::default, |visual| {
            let intrinsic = visual.intrinsic_chrome_content(id, world);
            VisualContent {
                width: intrinsic.width,
                height: intrinsic.height,
                text: intrinsic.text,
                ..VisualContent::default()
            }
        })
}

/// A childless box whose own specs cannot move its border box off its text.
///
/// Grid tracks, padding, border, min/max, and aspect ratio all can. Margin
/// does not: the border box returned below never includes it. Logical edges
/// stay on the general path because their physical padding depends on the
/// writing context.
pub(super) fn plain_childless_content(style: &nana_ui_core::LayoutStyle) -> bool {
    if style.omits_box()
        || style
            .display
            .is_some_and(|display| display.is_grid_container())
        || style.active_grid_columns().is_some()
        || style.active_grid_rows().is_some()
        || style.has_logical_box_edges()
        || style
            .aspect_ratio
            .is_some_and(|ratio| ratio.is_finite() && ratio > 0.0)
    {
        return false;
    }
    style.width.is_none()
        && style.height.is_none()
        && style.min_width.is_none()
        && style.min_height.is_none()
        && style.max_width.is_none()
        && style.max_height.is_none()
        && style.padding.is_none()
        && style.padding_top.is_none()
        && style.padding_right.is_none()
        && style.padding_bottom.is_none()
        && style.padding_left.is_none()
        && style.border_width.is_none()
        && {
            let sides = &*style.border_sides;
            sides.top_width.is_none()
                && sides.right_width.is_none()
                && sides.bottom_width.is_none()
                && sides.left_width.is_none()
        }
}

/// Used border box of a [`plain_childless_content`] node.
///
/// The text limit is the available inline size: with no chrome and no
/// width spec, that is the content box the general path would measure in.
/// Published min-content still follows the flow direction, matching the
/// empty-child fold.
fn measure_plain_childless(
    id: StableNodeId,
    cache_key: MeasurementKey,
    style: &nana_ui_core::LayoutStyle,
    text_metrics: Option<crate::TextMetrics>,
    writing: nana_ui_core::WritingContext,
    available: Size,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
) -> Result<Size, UiWorldError> {
    cache.note_measure_node();
    cache.record_full_subtree();
    #[cfg(feature = "benchmark")]
    let mut clock = super::plan_stats::PhaseClock::start();
    let text_natural_width = text_metrics.and_then(|_| nodes.world.text_natural_width(id));
    let text_wrap_limit = text_metrics.and_then(|_| nodes.world.text_wrap_limit(id));
    let text = text_metrics.unwrap_or_default();
    let limit = available.width.max(0.0);
    let text_width = text_inline_size(text, text_natural_width, text_wrap_limit, limit);
    let content_w = 0.0f32.max(text_width);
    let mut content_h = 0.0f32.max(text.height);
    if text_metrics.is_none()
        && let Some(font_px) = style.font_size.filter(|value| *value > 0.0)
    {
        content_h = content_h.max(text_line_box_height_px(font_px, style.line_height));
    }
    let content = Size::new(content_w, content_h);
    let direction = used_flow_direction(style, writing, false);
    let wrapping = match direction {
        FlexDirection::Row => matches!(style.flex_wrap, FlexWrap::Wrap | FlexWrap::WrapReverse),
        FlexDirection::Column => {
            matches!(style.flex_wrap, FlexWrap::Wrap | FlexWrap::WrapReverse)
                && available.height > 0.5
        }
    };
    let min_content_w = if wrapping || direction.is_column() {
        0.0
    } else {
        content.width
    };
    cache.insert(cache_key, content);
    #[cfg(feature = "benchmark")]
    clock.lap(14);
    let fonts = fonts_of(style, parent_font_px);
    let baseline = plain_leaf_baseline(
        id,
        style,
        text_metrics,
        fonts.element_px,
        writing.inline_size(available.width, available.height),
        content,
        nodes,
    );
    #[cfg(feature = "benchmark")]
    clock.lap(15);
    cache.remember_intrinsic_bounds(
        cache_key,
        min_content_w.min(content.width),
        content.width,
        0.0,
        Some(content.height),
        content,
        baseline.first,
        baseline.last,
        None,
    );
    #[cfg(feature = "benchmark")]
    clock.lap(16);
    Ok(content)
}

/// Baseline of a plain leaf. Chrome is zero: the predicate already rejected
/// padding and border. A shaped `nana-text` line keeps its own baseline.
/// A replaced box defers to the shared authority.
fn plain_leaf_baseline(
    id: StableNodeId,
    _style: &nana_ui_core::LayoutStyle,
    _text_metrics: Option<crate::TextMetrics>,
    font_px: f32,
    inline_base: f32,
    used: Size,
    nodes: &LayoutInputMap<'_>,
) -> BaselineMetrics {
    nodes.baseline_metrics(id, font_px, Some(inline_base), Some(used))
}

/// Whether a container whose height follows its content measures the same
/// whatever height it is offered: nothing in its subtree reads that extent
/// (`world/block_reads.rs`), and its page and containing block are
/// horizontal, so its edges resolve against width. Its key then drops the
/// offered height, and a height-only resize finds its measurement.
fn block_extent_unread(id: StableNodeId, node: &PassInput, world: &UiWorld) -> bool {
    #[cfg(any(test, feature = "layout-verify"))]
    if !super::verify::honors_block_reads() {
        return false;
    }
    !node.children.is_empty()
        && !node.writing.is_vertical()
        && !node.containing_writing.is_vertical()
        && !world.subtree_reads_block_extent(id)
}

/// [`intrinsic_size_scoped`], or with `forced` for the node's own style (a
/// flex item at its used main size): a forced measurement is not cached or
/// planned, since neither is keyed by it; its descendants are measured as
/// usual.
///
/// One level of the measure recursion, on a stack that cannot run out (see
/// `recursion`).
fn child_margin_of(
    child: StableNodeId,
    child_edge_base: f32,
    child_font_px: f32,
    nodes: &LayoutInputMap<'_>,
) -> nana_ui_core::PaddingSpec {
    nodes
        .style(child)
        .map(|style| {
            style.resolved_margin_against_fonts(
                Some(child_edge_base),
                fonts_of(&style, child_font_px),
            )
        })
        .unwrap_or_default()
}

#[derive(Clone)]
struct MeasureRequest {
    id: StableNodeId,
    forced: Option<Arc<nana_ui_core::LayoutStyle>>,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
}

fn at_main_request(
    id: StableNodeId,
    main: f32,
    direction: FlexDirection,
    available: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &LayoutInputMap<'_>,
) -> Option<MeasureRequest> {
    let style = nodes.style_ref(id)?;
    let mut forced = (*style).clone();
    forced.box_sizing = BoxSizing::BorderBox;
    let main = Some(LengthSpec::Px(main.max(0.0)));
    match direction {
        FlexDirection::Row => forced.width = main,
        FlexDirection::Column => forced.height = main,
    }
    Some(MeasureRequest {
        id,
        forced: Some(Arc::new(forced)),
        available,
        parent_direction: Some(direction),
        viewport,
        parent_font_px,
    })
}

struct PendingBatch {
    requests: Vec<MeasureRequest>,
    next: usize,
    results: Vec<Size>,
    note_child_measured: bool,
}

impl PendingBatch {
    fn new(requests: Vec<MeasureRequest>, note_child_measured: bool) -> Self {
        let len = requests.len();
        Self {
            requests,
            next: 0,
            results: Vec::with_capacity(len),
            note_child_measured,
        }
    }
}

struct ContainerCommon {
    id: StableNodeId,
    unforced: bool,
    style: Arc<LayoutStyle>,
    child_ids: Arc<Vec<StableNodeId>>,
    text_metrics: Option<crate::TextMetrics>,
    text_natural_width: Option<f32>,
    text_wrap_limit: Option<f32>,
    writing: nana_ui_core::WritingContext,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    child_font_px: f32,
    fonts: FontSizeContext,
    edge_base: f32,
    chrome: Size,
    content_available: Size,
    spec_width: Option<f32>,
    spec_height: Option<f32>,
    cache_key: MeasurementKey,
    plain_leaf: bool,
    had_plan: bool,
    visual: VisualContent,
    lead: super::parts::LeadingParts,
    lead_inset: Size,
    header_chrome: bool,
    natural: Option<crate::ReplacedMetadata>,
}

enum UnchangedStep {
    Scoped {
        cursor: usize,
        entry_child: StableNodeId,
        cached: Size,
        baseline: Option<f32>,
        at_main: Option<(f32, Size)>,
    },
    AtMain {
        cursor: usize,
        cached_main: Size,
    },
}

enum SequentialStep<'s> {
    Affected {
        cursor: usize,
        entry_child: StableNodeId,
        old: Size,
        current_style: Option<Arc<LayoutStyle>>,
    },
    Arrival {
        cursor: usize,
        child: StableNodeId,
        child_style: Option<Arc<LayoutStyle>>,
    },
    _Marker(std::marker::PhantomData<&'s ()>),
}

struct SequentialDeltaState<'s> {
    affected: &'s [StableNodeId],
    direction: FlexDirection,
    child_edge_base: f32,
    cross_own: bool,
    edit: Option<ChildListEdit>,
    main_sum: f64,
    patches: Vec<(StableNodeId, Size, Option<Arc<LayoutStyle>>)>,
    flow_len: usize,
    at_cross: usize,
    arrivals: Vec<StableNodeId>,
    arrived: Vec<MeasuredChild>,
    step: SequentialStep<'s>,
}

struct FlexLineDeltaState<'s> {
    affected: &'s [StableNodeId],
    direction: FlexDirection,
    flow: Vec<StableNodeId>,
    old_sizes: Vec<Size>,
    sizes: Vec<Size>,
    patched: bool,
    cursor: usize,
    active_index: usize,
}

enum GridDeltaStep {
    Children,
    Pass2(super::grid::Grid2dPrepared),
}

struct GridDeltaState {
    flow: Vec<StableNodeId>,
    sizes: Vec<Size>,
    queries: Vec<(usize, bool)>,
    patched: bool,
    step: GridDeltaStep,
}

enum PlanWait<'s> {
    Unchanged {
        affected: &'s [StableNodeId],
        step: UnchangedStep,
    },
    Sequential(SequentialDeltaState<'s>),
    FlexLine(FlexLineDeltaState<'s>),
    IfcLine(super::inline::IfcLineDeltaPrepared),
    Grid(GridDeltaState),
}

struct PlanState<'s> {
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    wait: PlanWait<'s>,
}

enum ContainerPhase {
    NativeInline,
    FlowChildren,
    GridPass2 {
        flow_children: Vec<StableNodeId>,
        child_sizes: Vec<Size>,
        state: super::grid::Grid2dPrepared,
    },
    GridAutoTracks {
        flow_children: Vec<StableNodeId>,
        child_sizes: Vec<Size>,
        tracks: Arc<Vec<nana_ui_core::GridTrack>>,
        state: super::grid::AutoTracksPrepared,
    },
    EqualWrap {
        flow_children: Vec<StableNodeId>,
        child_sizes: Vec<Size>,
        state: super::flex::EqualWrapPrepared,
    },
    DefiniteMain {
        flow_children: Vec<StableNodeId>,
        child_sizes: Vec<Size>,
        used_sizes: Vec<Size>,
        pending: Vec<(usize, StableNodeId, f32)>,
    },
}

struct ContainerState {
    common: Box<ContainerCommon>,
    flow_children: Vec<StableNodeId>,
    descendant_dependent_flow: bool,
    context: EstablishedContext,
    direction: FlexDirection,
    wrap: FlexWrap,
    wrapping: bool,
    grid_tracks: Option<Arc<Vec<nana_ui_core::GridTrack>>>,
    #[cfg(feature = "benchmark")]
    child_phase: Option<super::plan_stats::PhaseClock>,
    phase: ContainerPhase,
}

enum FrameBody<'s> {
    Plan(PlanState<'s>),
    Container(ContainerState),
}

struct MeasureFrame<'s> {
    batch: PendingBatch,
    body: Box<FrameBody<'s>>,
}

enum FrameOutcome<'s> {
    Done(Size),
    Push { frame: MeasureFrame<'s> },
}

/// Border box of `id` under `available` (with optional `forced` style for an
/// `intrinsic_size_at_main` query), running cleanly on an explicit heap work-stack
/// rather than unbounded OS call-stack recursion.
#[allow(clippy::too_many_arguments)]
fn measure_node<'w, 's>(
    id: StableNodeId,
    forced: Option<Arc<nana_ui_core::LayoutStyle>>,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<Size, UiWorldError> {
    let root = MeasureRequest {
        id,
        forced,
        available,
        parent_direction,
        viewport,
        parent_font_px,
    };
    drive_measure(root, nodes, cache, scope)
}

enum StartOutcome<'s> {
    Fast(Size),
    Push(MeasureFrame<'s>),
}

fn drive_measure<'w, 's>(
    root: MeasureRequest,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<Size, UiWorldError> {
    let mut stack: Vec<MeasureFrame<'s>> = match start_measure_request(root, nodes, cache, scope)? {
        StartOutcome::Fast(size) => return Ok(size),
        StartOutcome::Push(frame) => {
            let mut s = Vec::with_capacity(16);
            s.push(frame);
            s
        }
    };

    while let Some(mut frame) = stack.pop() {
        while frame.batch.next < frame.batch.requests.len() {
            let idx = frame.batch.next;
            frame.batch.next += 1;
            if frame.batch.note_child_measured {
                cache.note_child_measured();
            }
            let req = frame.batch.requests[idx].clone();
            match start_measure_request(req, nodes, cache, scope)? {
                StartOutcome::Fast(size) => {
                    frame.batch.results.push(size);
                }
                StartOutcome::Push(child_frame) => {
                    stack.push(frame);
                    frame = child_frame;
                }
            }
        }

        let results = std::mem::take(&mut frame.batch.results);
        match step_measure_frame(*frame.body, results, nodes, cache, scope)? {
            FrameOutcome::Done(size) => {
                if let Some(parent) = stack.last_mut() {
                    parent.batch.results.push(size);
                } else {
                    return Ok(size);
                }
            }
            FrameOutcome::Push { frame: next_frame } => {
                stack.push(next_frame);
            }
        }
    }

    Ok(Size::default())
}

fn parent_stretches_inline_auto_width(
    id: StableNodeId,
    style: &nana_ui_core::LayoutStyle,
    writing: nana_ui_core::WritingContext,
    parent_direction: Option<FlexDirection>,
    world: &crate::UiWorld,
) -> bool {
    if style.width.is_some_and(LengthSpec::is_definite_declared)
        || style.width.is_some_and(LengthSpec::is_content_sized)
    {
        return false;
    }
    if style.display.is_some_and(|display| {
        matches!(
            display,
            DisplaySpec::InlineFlex | DisplaySpec::InlineGrid | DisplaySpec::Inline
        )
    }) {
        return false;
    }
    if parent_direction.is_none() {
        return true;
    }
    if style.display != Some(DisplaySpec::Grid) {
        return false;
    }
    let Some(parent_id) = world.parent_id(id) else {
        return false;
    };
    let Some(parent_style) = world.layout_style(parent_id) else {
        return false;
    };
    let parent_display = parent_style.display;
    let parent_definite_inline = !parent_style.width.is_none_or(LengthSpec::is_content_sized);
    if parent_display.is_some_and(DisplaySpec::is_grid_container) {
        return (parent_display == Some(DisplaySpec::Grid) || parent_definite_inline)
            && style.resolved_justify_self(parent_style.justify_items) == AlignSpec::Stretch;
    }
    if parent_display.is_some_and(DisplaySpec::is_flex_container) {
        return !writing.is_vertical()
            && matches!(parent_direction, Some(FlexDirection::Column))
            && parent_definite_inline
            && style.resolved_align_self(parent_style.child_align_items()) == AlignSpec::Stretch;
    }
    false
}

fn start_measure_request<'w, 's>(
    req: MeasureRequest,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<StartOutcome<'s>, UiWorldError> {
    let MeasureRequest {
        id,
        forced,
        available,
        parent_direction,
        viewport,
        parent_font_px,
    } = req;
    let unforced = forced.is_none();
    let world = nodes.world;
    let parent = world.parent_id(id);
    let previous_context = parent.and_then(|id| world.established_layout_context(id));
    let context = parent
        .and_then(|id| nodes.contexts.get(&id).copied().or(previous_context))
        .map(crate::FormattingContextKind::cache_id);
    let Some((node, style_facts)) = nodes.get_with_facts(id)? else {
        return Ok(StartOutcome::Fast(Size::default()));
    };
    let stretches_inline =
        parent_stretches_inline_auto_width(id, &node.style, node.writing, parent_direction, world);
    let direction_sensitive = (parent_direction.is_none() || stretches_inline)
        && (!node.children.is_empty() || node.style.display == Some(DisplaySpec::Grid))
        && !node.style.width.is_some_and(LengthSpec::is_content_sized);
    let mut keyed_available = available;
    if node.children.is_empty() && !style_facts.block_containing_size_affects_leaf(node.writing) {
        keyed_available.height = 0.0;
    }
    let (own_width, own_height) = style_facts.sizes_own_axes(node.writing);
    if own_width {
        keyed_available.width = 0.0;
    }
    if own_height || block_extent_unread(id, node, world) {
        keyed_available.height = 0.0;
    }
    let cache_key = MeasurementKey::new(
        id,
        keyed_available,
        parent_direction,
        style_facts.viewport_basis(viewport),
        parent_font_px,
        node.writing,
        node.containing_writing,
        style_facts.constraint_class(keyed_available),
        direction_sensitive,
    );
    if unforced
        && let Some(scope) = scope
        && !scope.metrics_changed.contains(&id)
        && let Some(metrics) = scope
            .retained
            .intrinsics
            .metrics(&PassIntrinsicCache::intrinsic_key(cache_key))
    {
        cache.seed_intrinsic(
            PassIntrinsicCache::intrinsic_key(cache_key),
            metrics,
            previous_context.map(crate::FormattingContextKind::cache_id),
        );
    }
    if unforced && let Some(size) = cache.get(&cache_key) {
        cache.note_measure_cache_hit();
        return Ok(StartOutcome::Fast(size));
    }
    let style_arc = forced.map_or_else(|| node.style.shared(), StyleRef::Owned);
    let child_ids = Arc::clone(node.children);
    let text_metrics = node.text_metrics;
    let (writing, containing_writing) = (node.writing, node.containing_writing);
    let plain_leaf = child_ids.is_empty()
        && if unforced {
            style_facts.plain_childless_content()
        } else {
            plain_childless_content(style_arc.as_ref())
        }
        && nodes.world.widget_visual_ref(id).is_none()
        && nodes.world.replaced_natural_size(id).is_none();
    if unforced && scope.is_none() && plain_leaf {
        #[cfg(test)]
        super::measure_trace::record(id);
        let size = measure_plain_childless(
            id,
            cache_key,
            style_arc.as_ref(),
            text_metrics,
            writing,
            available,
            parent_font_px,
            nodes,
            cache,
        )?;
        return Ok(StartOutcome::Fast(size));
    }
    let cached_intrinsic = if unforced {
        cache.get_intrinsic(cache_key, context)
    } else {
        None
    };
    if unforced
        && let Some(scope) = scope
        && (!scope.measure.contains(&id)
            && !(scope.affected.contains(&id) && scope.retained.measure_plans.contains_key(&id)))
        && let Some(size) = scope
            .retained
            .intrinsics
            .used(id)
            .and_then(|memo| memo.get(cache_key))
    {
        cache.note_measure_cache_hit();
        cache.insert(cache_key, size);
        return Ok(StartOutcome::Fast(size));
    }
    #[cfg(test)]
    super::measure_trace::record(id);
    let text_natural_width = text_metrics.and_then(|_| nodes.world.text_natural_width(id));
    let text_wrap_limit = text_metrics.and_then(|_| nodes.world.text_wrap_limit(id));
    let style = style_arc.to_arc();
    if style.omits_box() {
        return Ok(StartOutcome::Fast(Size::default()));
    }
    let fonts = fonts_of(style.as_ref(), parent_font_px);
    let child_font_px = fonts.element_px;
    let edge_base = containing_writing.inline_size(available.width, available.height);
    let padding = style.resolved_padding_against_fonts(Some(edge_base), fonts);
    let border = style.resolved_border_edges();
    let chrome = Size::new(
        padding.left + padding.right + border.left + border.right,
        padding.top + padding.bottom + border.top + border.bottom,
    );
    let margin = style.resolved_margin_against_fonts(Some(edge_base), fonts);
    let content_axis = |spec: Option<LengthSpec>, available: f32, margins: f32, chrome: f32| {
        let resolved = resolve_axis(
            demote_fill_spec_if_indefinite(spec, available),
            available,
            available - margins,
            viewport,
            fonts,
        );
        let extent = resolved.unwrap_or(available);
        if resolved.is_some()
            && matches!(style.box_sizing, BoxSizing::ContentBox)
            && spec.is_some_and(LengthSpec::is_definite_declared)
        {
            extent.max(0.0)
        } else {
            (extent - chrome).max(0.0)
        }
    };
    let mut content_available = Size::new(
        content_axis(
            style.width,
            available.width,
            margin.left + margin.right,
            chrome.width,
        ),
        content_axis(
            style.height,
            available.height,
            margin.top + margin.bottom,
            chrome.height,
        ),
    );
    let vp = Some((viewport.width, viewport.height));
    let min_width = style.resolved_min_width_fonts(Some(available.width), vp, fonts)
        + if matches!(style.box_sizing, BoxSizing::ContentBox)
            && style.width.is_some_and(LengthSpec::is_definite_declared)
        {
            chrome.width
        } else {
            0.0
        };
    let mut measured_width = (content_available.width + chrome.width).max(min_width);
    if let Some(max) = style.resolved_max_width_fonts(Some(available.width), vp, fonts) {
        measured_width = measured_width.min(max);
    }
    content_available.width = (measured_width - chrome.width).max(0.0);
    let (spec_width, spec_height) =
        resolved_size_specs(style.as_ref(), available, edge_base, viewport, fonts);
    if unforced && let Some(metrics) = cached_intrinsic {
        let mut size = finish_intrinsic_size(
            style.as_ref(),
            fonts,
            viewport,
            available,
            edge_base,
            chrome,
            parent_direction,
            metrics.preferred.inline,
            metrics.preferred.block,
        );
        transfer_natural_ratio(
            style.as_ref(),
            nodes.world.replaced_natural_size(id),
            chrome,
            available,
            viewport,
            fonts,
            &mut size,
        );
        cache.insert(cache_key, size);
        return Ok(StartOutcome::Fast(size));
    }
    if spec_width.is_some() && spec_height.is_some() {
        let size = finish_intrinsic_size(
            style.as_ref(),
            fonts,
            viewport,
            available,
            edge_base,
            chrome,
            parent_direction,
            0.0,
            0.0,
        );
        if unforced {
            cache.insert(cache_key, size);
            if !depends_on_used_basis(style.width) && !depends_on_used_basis(style.height) {
                publish_atom_baseline(
                    id,
                    size,
                    writing.inline_size(available.width, available.height),
                    viewport,
                    parent_font_px,
                    nodes,
                    cache,
                    scope,
                )?;
                let baseline = nodes.baseline_metrics(
                    id,
                    fonts.element_px,
                    Some(writing.inline_size(available.width, available.height)),
                    Some(size),
                );
                cache.insert_intrinsic_bounds(
                    cache_key,
                    0.0,
                    size.width,
                    0.0,
                    Some(size.height),
                    size,
                    baseline.first,
                    baseline.last,
                    style
                        .aspect_ratio
                        .filter(|ratio| ratio.is_finite() && *ratio > 0.0),
                );
            }
        }
        cache.note_measure_node();
        return Ok(StartOutcome::Fast(size));
    }

    let retained_plan = if unforced {
        scope.and_then(|scope| scope.retained.measure_plans.get(&id)?.get(available))
    } else {
        None
    };
    let had_plan = retained_plan.is_some();
    let visual = visual_content(id, nodes.world);
    let lead = visual.lead.unwrap_or_default();
    let lead_inset = lead.inset();
    content_available = Size::new(
        (content_available.width - lead_inset.width).max(0.0),
        (content_available.height - lead_inset.height).max(0.0),
    );
    let header_chrome = nodes
        .world
        .widget_visual_ref(id)
        .is_some_and(crate::WidgetVisualHandle::uses_card_title_typography);
    let natural = nodes
        .world
        .replaced_natural_size(id)
        .filter(|natural| natural.width > 0.0 && natural.height > 0.0);

    let common = Box::new(ContainerCommon {
        id,
        unforced,
        style,
        child_ids,
        text_metrics,
        text_natural_width,
        text_wrap_limit,
        writing,
        available,
        parent_direction,
        viewport,
        parent_font_px,
        child_font_px,
        fonts,
        edge_base,
        chrome,
        content_available,
        spec_width,
        spec_height,
        cache_key,
        plain_leaf,
        had_plan,
        visual,
        lead,
        lead_inset,
        header_chrome,
        natural,
    });

    if let Some(scope) = scope
        && let Some(plan) = retained_plan
        && !plan.stale
        && plan.inputs_match(
            available,
            parent_direction,
            stretches_inline,
            viewport,
            parent_font_px,
            &common.style,
            text_metrics,
            text_natural_width,
            text_wrap_limit,
            nodes.world.block_strut(id),
            visual,
            writing,
        )
        && nodes.world.children_layout_style_is_local(id)
        && natural.is_none()
        && nodes.world.inline_record(id).is_none()
    {
        match start_plan_attempt(common, plan, nodes, cache, scope)? {
            PlanStart::Hit(size) => return Ok(StartOutcome::Fast(size)),
            PlanStart::Push(frame) => return Ok(StartOutcome::Push(frame)),
            PlanStart::Fallthrough(common) => {
                return start_container_frame(common, nodes, cache, Some(scope));
            }
        }
    }

    start_container_frame(common, nodes, cache, scope)
}

enum PlanStart<'s> {
    Hit(Size),
    Push(MeasureFrame<'s>),
    Fallthrough(Box<ContainerCommon>),
}

fn finish_plan_reused(
    common: &ContainerCommon,
    size: Size,
    patched: bool,
    cache: &mut PassIntrinsicCache,
) -> Size {
    if patched {
        cache.retire_intrinsic_facts(common.id);
    } else {
        cache.note_unchanged_plan_node(common.id);
    }
    cache.note_plan_consulted(common.id, common.available);
    cache.note_measure_plan_reused();
    cache.note_measure_cache_hit();
    cache.insert(common.cache_key, size);
    size
}

fn start_plan_attempt<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<PlanStart<'s>, UiWorldError> {
    if Arc::ptr_eq(&plan.children, &common.child_ids) {
        let affected = scope.affected_children(common.id);
        match advance_unchanged(common, plan, affected, 0, nodes, cache, scope)? {
            Ok(PlanStart::Hit(size)) => return Ok(PlanStart::Hit(size)),
            Ok(PlanStart::Push(frame)) => return Ok(PlanStart::Push(frame)),
            Ok(PlanStart::Fallthrough(_)) => unreachable!(),
            Err(common) => return try_plan_sequential(common, plan, nodes, cache, scope),
        }
    }
    try_plan_sequential(common, plan, nodes, cache, scope)
}

fn advance_unchanged<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    affected: &'s [StableNodeId],
    mut cursor: usize,
    nodes: &LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<Result<PlanStart<'s>, Box<ContainerCommon>>, UiWorldError> {
    while cursor < affected.len() {
        let aff = affected[cursor];
        cursor += 1;
        let Some(entry) = plan.entry(aff) else {
            continue;
        };
        if !retained_style_matches(nodes.style_ref(entry.child).as_ref(), entry.style.as_ref()) {
            return Ok(Err(common));
        }
        if !scope.measure.contains(&aff) {
            continue;
        }
        if let Some(cached) = entry.intrinsic.get() {
            let req = MeasureRequest {
                id: entry.child,
                forced: None,
                available: plan.child_available,
                parent_direction: Some(plan.child_direction),
                viewport: common.viewport,
                parent_font_px: common.child_font_px,
            };
            return Ok(Ok(PlanStart::Push(MeasureFrame {
                batch: PendingBatch::new(vec![req], false),
                body: Box::new(FrameBody::Plan(PlanState {
                    common,
                    plan,
                    wait: PlanWait::Unchanged {
                        affected,
                        step: UnchangedStep::Scoped {
                            cursor,
                            entry_child: entry.child,
                            cached,
                            baseline: entry.baseline.get(),
                            at_main: entry.at_main.as_deref().copied(),
                        },
                    },
                })),
            })));
        }
        if let Some(&(main, cached_main)) = entry.at_main.as_deref() {
            let Some(req) = at_main_request(
                entry.child,
                main,
                plan.child_direction,
                plan.child_available,
                common.viewport,
                common.child_font_px,
                nodes,
            ) else {
                if Size::default() != cached_main {
                    return Ok(Err(common));
                }
                continue;
            };
            return Ok(Ok(PlanStart::Push(MeasureFrame {
                batch: PendingBatch::new(vec![req], false),
                body: Box::new(FrameBody::Plan(PlanState {
                    common,
                    plan,
                    wait: PlanWait::Unchanged {
                        affected,
                        step: UnchangedStep::AtMain {
                            cursor,
                            cached_main,
                        },
                    },
                })),
            })));
        }
    }
    let size = finish_plan_reused(&common, plan.size, false, cache);
    Ok(Ok(PlanStart::Hit(size)))
}

fn start_container_frame<'w, 's>(
    common: Box<ContainerCommon>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<StartOutcome<'s>, UiWorldError> {
    if common.had_plan {
        cache.note_plan_miss();
    }
    cache.note_measure_node();
    if scope.is_some() {
        cache.note_measure_cache_miss();
    }
    cache.record_full_subtree();
    let id = common.id;
    let style = common.style.as_ref();
    let native_inline = nodes.world.inline_record(id).is_some();
    if native_inline {
        let mut requests = Vec::new();
        for &child in common.child_ids.iter() {
            if nodes.world.inline_parent(child) != Some(id) {
                continue;
            }
            if let Some(child_style) = nodes.style_ref(child)
                && style.height.is_none_or(LengthSpec::is_content_sized)
                && reads_offered_block_extent(&child_style)
            {
                return Err(UiWorldError::InvalidInlineContent(id));
            }
            requests.push(MeasureRequest {
                id: child,
                forced: None,
                available: common.content_available,
                parent_direction: None,
                viewport: common.viewport,
                parent_font_px: common.child_font_px,
            });
        }
        let context = EstablishedContext {
            kind: crate::FormattingContextKind::Inline,
            grid_2d: false,
        };
        nodes.contexts.insert(id, context.kind);
        let direction = used_flow_direction(style, common.writing, true);
        let wrap = style.flex_wrap;
        let state = ContainerState {
            common,
            flow_children: Vec::new(),
            descendant_dependent_flow: false,
            context,
            direction,
            wrap,
            wrapping: true,
            grid_tracks: None,
            #[cfg(feature = "benchmark")]
            child_phase: None,
            phase: ContainerPhase::NativeInline,
        };
        if requests.is_empty() {
            let size = finish_measure_container(
                state,
                Vec::new(),
                Vec::new(),
                Size::default(),
                0.0,
                None,
                Vec::new(),
                0.0,
                (0.0, 0),
                nodes,
                cache,
                scope,
            )?;
            return Ok(StartOutcome::Fast(size));
        }
        return Ok(StartOutcome::Push(MeasureFrame {
            batch: PendingBatch::new(requests, true),
            body: Box::new(FrameBody::Container(state)),
        }));
    }

    let (mut flow_children, descendant_dependent_flow) =
        collect_flow_children_reporting(&common.child_ids, nodes, style)?;
    let context = establish_context(id, style, &flow_children, nodes);
    nodes.contexts.insert(id, context.kind);
    let grid_measure = context.grid_2d;
    let ifc = context.inline();
    let direction = used_flow_direction(style, common.writing, ifc);
    let wrap = style.flex_wrap;
    let wrapping = ifc
        || match direction {
            FlexDirection::Row => matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse),
            FlexDirection::Column => {
                matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse)
                    && common.content_available.height > 0.5
            }
        };
    let grid_tracks = match direction {
        FlexDirection::Row => style.active_grid_columns().map(|t| Arc::new(t.to_vec())),
        FlexDirection::Column => style.active_grid_rows().map(|t| Arc::new(t.to_vec())),
    };
    if grid_measure || wrapping || grid_tracks.as_deref().is_some_and(|t| !t.is_empty()) {
        sort_by_order(&mut flow_children, nodes);
    }
    #[cfg(feature = "benchmark")]
    let child_phase = (flow_children.len() > 64).then(super::plan_stats::PhaseClock::start);
    let mut requests = Vec::with_capacity(flow_children.len());
    for &child in &flow_children {
        let child_available = if grid_measure {
            nodes
                .style(child)
                .map(|child_style| {
                    grid_item_measure_available(child_style.as_ref(), common.content_available)
                })
                .unwrap_or(common.content_available)
        } else {
            common.content_available
        };
        requests.push(MeasureRequest {
            id: child,
            forced: None,
            available: child_available,
            parent_direction: Some(direction),
            viewport: common.viewport,
            parent_font_px: common.child_font_px,
        });
    }
    let state = ContainerState {
        common,
        flow_children,
        descendant_dependent_flow,
        context,
        direction,
        wrap,
        wrapping,
        grid_tracks,
        #[cfg(feature = "benchmark")]
        child_phase,
        phase: ContainerPhase::FlowChildren,
    };
    if requests.is_empty() {
        return Ok(
            match step_container_frame(state, Vec::new(), nodes, cache, scope)? {
                FrameOutcome::Done(s) => StartOutcome::Fast(s),
                FrameOutcome::Push { frame } => StartOutcome::Push(frame),
            },
        );
    }
    Ok(StartOutcome::Push(MeasureFrame {
        batch: PendingBatch::new(requests, true),
        body: Box::new(FrameBody::Container(state)),
    }))
}

fn step_measure_frame<'w, 's>(
    body: FrameBody<'s>,
    results: Vec<Size>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    match body {
        FrameBody::Plan(plan_state) => step_plan_frame(plan_state, results, nodes, cache, scope),
        FrameBody::Container(state) => step_container_frame(state, results, nodes, cache, scope),
    }
}

fn step_container_frame<'w, 's>(
    mut state: ContainerState,
    results: Vec<Size>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    let phase = std::mem::replace(&mut state.phase, ContainerPhase::NativeInline);
    match phase {
        ContainerPhase::NativeInline => {
            let flow_children = std::mem::take(&mut state.flow_children);
            let size = finish_measure_container(
                state,
                flow_children,
                Vec::new(),
                Size::default(),
                0.0,
                None,
                Vec::new(),
                0.0,
                (0.0, 0),
                nodes,
                cache,
                scope,
            )?;
            Ok(FrameOutcome::Done(size))
        }
        ContainerPhase::FlowChildren => {
            #[cfg(feature = "benchmark")]
            if let Some(clock) = state.child_phase.as_mut() {
                clock.lap(4);
            }
            let flow_children = std::mem::take(&mut state.flow_children);
            let mut child_sizes = nodes.scratch.sizes(results.len());
            child_sizes.extend(results);
            let ifc = state.context.inline();
            let baseline_lines = ifc.then(|| {
                line_strut(
                    state.common.id,
                    state.common.style.as_ref(),
                    state.common.writing,
                    state.common.child_font_px,
                    nodes,
                )
            });
            if baseline_lines.is_some() {
                prime_atom_baselines(
                    flow_children
                        .iter()
                        .copied()
                        .zip(child_sizes.iter().copied()),
                    state
                        .common
                        .writing
                        .logical_size(
                            state.common.content_available.width,
                            state.common.content_available.height,
                        )
                        .0,
                    state.common.viewport,
                    state.common.child_font_px,
                    nodes,
                    cache,
                    scope,
                )?;
            }
            after_flow_children_step(
                state,
                flow_children,
                child_sizes,
                baseline_lines,
                nodes,
                cache,
                scope,
            )
        }
        ContainerPhase::GridPass2 {
            flow_children,
            child_sizes,
            state: mut grid_prep,
        } => {
            super::grid::apply_grid_row_baseline_shim(
                state.common.style.as_ref(),
                &mut grid_prep,
                &results,
                state.common.viewport,
                state.common.child_font_px,
                nodes,
                cache,
                scope,
            )?;
            let grid = layout_grid_2d_finish(grid_prep, &results);
            let subgrid =
                state.common.style.is_subgrid_columns() || state.common.style.is_subgrid_rows();
            let recorded_grid = (!subgrid).then(|| Box::new(GridTrackPlan::from_layout(&grid)));
            let (width, height) = state.common.writing.physical_size(
                grid_axis_extent(&grid.col_sizes, grid.col_gap),
                grid_axis_extent(&grid.row_sizes, grid.row_gap),
            );
            finish_children_layout(
                state,
                flow_children,
                child_sizes,
                Size::new(width, height),
                recorded_grid,
                Vec::new(),
                0.0,
                (0.0, 0),
                nodes,
                cache,
                scope,
            )
        }
        ContainerPhase::GridAutoTracks {
            flow_children,
            child_sizes,
            tracks,
            state: auto_prep,
        } => {
            let auto_sizes = auto_track_contributions_finish(auto_prep, &results);
            let parent_box =
                gap_containing_block(state.common.style.as_ref(), state.common.content_available);
            let gap = state.common.style.main_gap_against_fonts(
                state.direction,
                parent_box,
                state.common.fonts,
            );
            let child_edge_base = state
                .common
                .writing
                .logical_size(
                    state.common.content_available.width,
                    state.common.content_available.height,
                )
                .0;
            let budget = main_extent(state.common.content_available, state.direction);
            let resolved = resolve_grid_track_sizes(&tracks, budget, gap, &auto_sizes);
            let children_size = grid_intrinsic_size(
                state.direction,
                &resolved,
                &child_sizes,
                &flow_children,
                child_edge_base,
                gap,
                state.common.child_font_px,
                nodes,
            );
            finish_children_layout(
                state,
                flow_children,
                child_sizes,
                children_size,
                None,
                Vec::new(),
                0.0,
                (0.0, 0),
                nodes,
                cache,
                scope,
            )
        }
        ContainerPhase::EqualWrap {
            flow_children,
            child_sizes,
            state: eq_prep,
        } => {
            let children_size = measure_equal_wrap_finish(eq_prep, &results);
            finish_children_layout(
                state,
                flow_children,
                child_sizes,
                children_size,
                None,
                Vec::new(),
                0.0,
                (0.0, 0),
                nodes,
                cache,
                scope,
            )
        }
        ContainerPhase::DefiniteMain {
            flow_children,
            child_sizes,
            mut used_sizes,
            pending,
        } => {
            let mut hypothetical = Vec::with_capacity(pending.len());
            for ((index, child, main), &at_main) in pending.iter().zip(&results) {
                set_cross_extent(
                    &mut used_sizes[*index],
                    state.direction,
                    cross_extent(at_main, state.direction),
                );
                hypothetical.push((*child, *main, at_main));
            }
            finish_sequential_children(
                state,
                flow_children,
                child_sizes,
                &used_sizes,
                hypothetical,
                nodes,
                cache,
                scope,
            )
        }
    }
}

fn after_flow_children_step<'w, 's>(
    mut state: ContainerState,
    flow_children: Vec<StableNodeId>,
    child_sizes: Vec<Size>,
    baseline_lines: Option<super::LineStrut>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    let style = &state.common.style;
    let writing = state.common.writing;
    let content_available = state.common.content_available;
    let fonts = state.common.fonts;
    let direction = state.direction;
    let parent_box = gap_containing_block(style.as_ref(), content_available);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    let child_edge_base = writing
        .logical_size(content_available.width, content_available.height)
        .0;
    let grid_measure = state.context.grid_2d;

    if grid_measure {
        match layout_grid_2d_begin(
            style.as_ref(),
            writing,
            &flow_children,
            &child_sizes,
            content_available,
            fonts,
            None,
            nodes,
        ) {
            super::grid::Grid2dStart::Ready(mut grid_prep) => {
                super::grid::apply_grid_row_baseline_shim(
                    style.as_ref(),
                    &mut grid_prep,
                    &[],
                    state.common.viewport,
                    state.common.child_font_px,
                    nodes,
                    cache,
                    scope,
                )?;
                let grid = layout_grid_2d_finish(grid_prep, &[]);
                let subgrid = style.is_subgrid_columns() || style.is_subgrid_rows();
                let recorded_grid = (!subgrid).then(|| Box::new(GridTrackPlan::from_layout(&grid)));
                let (width, height) = writing.physical_size(
                    grid_axis_extent(&grid.col_sizes, grid.col_gap),
                    grid_axis_extent(&grid.row_sizes, grid.row_gap),
                );
                return finish_children_layout(
                    state,
                    flow_children,
                    child_sizes,
                    Size::new(width, height),
                    recorded_grid,
                    Vec::new(),
                    0.0,
                    (0.0, 0),
                    nodes,
                    cache,
                    scope,
                );
            }
            super::grid::Grid2dStart::NeedsPass2(grid_prep) => {
                let requests = grid_prep
                    .queries
                    .iter()
                    .map(|&(_, item_id, avail)| MeasureRequest {
                        id: item_id,
                        forced: None,
                        available: avail,
                        parent_direction: Some(FlexDirection::Row),
                        viewport: state.common.viewport,
                        parent_font_px: state.common.child_font_px,
                    })
                    .collect();
                state.phase = ContainerPhase::GridPass2 {
                    flow_children,
                    child_sizes,
                    state: grid_prep,
                };
                return Ok(FrameOutcome::Push {
                    frame: MeasureFrame {
                        batch: PendingBatch::new(requests, false),
                        body: Box::new(FrameBody::Container(state)),
                    },
                });
            }
        }
    }

    if let Some(tracks) = state
        .grid_tracks
        .as_ref()
        .filter(|tracks| !tracks.is_empty())
        .cloned()
    {
        let auto_prep = auto_track_contributions_begin(
            &flow_children,
            &tracks,
            content_available,
            direction == FlexDirection::Column,
            state.common.viewport,
            state.common.child_font_px,
            nodes,
        );
        if auto_prep.queries.is_empty() {
            let auto_sizes = auto_track_contributions_finish(auto_prep, &[]);
            let budget = main_extent(content_available, direction);
            let resolved = resolve_grid_track_sizes(&tracks, budget, gap, &auto_sizes);
            let children_size = grid_intrinsic_size(
                direction,
                &resolved,
                &child_sizes,
                &flow_children,
                child_edge_base,
                gap,
                state.common.child_font_px,
                nodes,
            );
            return finish_children_layout(
                state,
                flow_children,
                child_sizes,
                children_size,
                None,
                Vec::new(),
                0.0,
                (0.0, 0),
                nodes,
                cache,
                scope,
            );
        }
        let requests = auto_prep
            .queries
            .iter()
            .map(|&(_, child, available)| MeasureRequest {
                id: child,
                forced: None,
                available,
                parent_direction: Some(auto_prep.direction),
                viewport: state.common.viewport,
                parent_font_px: state.common.child_font_px,
            })
            .collect();
        state.phase = ContainerPhase::GridAutoTracks {
            flow_children,
            child_sizes,
            tracks,
            state: auto_prep,
        };
        return Ok(FrameOutcome::Push {
            frame: MeasureFrame {
                batch: PendingBatch::new(requests, false),
                body: Box::new(FrameBody::Container(state)),
            },
        });
    }

    if state.wrapping && style.equal_wrap {
        let eq_prep = measure_equal_wrap_begin(
            &flow_children,
            &child_sizes,
            direction,
            content_available,
            child_edge_base,
            gap,
            cross_gap,
            state.common.child_font_px,
            nodes,
        );
        let mut requests = Vec::with_capacity(eq_prep.queries.len());
        let mut quick_zero = true;
        for &(child, share) in &eq_prep.queries {
            if let Some(req) = at_main_request(
                child,
                share,
                direction,
                content_available,
                state.common.viewport,
                state.common.child_font_px,
                nodes,
            ) {
                requests.push(req);
                quick_zero = false;
            } else {
                // Missing node: synthesize a 0-sized request or skip? Push a request for the id
                // which `start_measure_request` resolves quickly to Size::default().
                requests.push(MeasureRequest {
                    id: child,
                    forced: None,
                    available: content_available,
                    parent_direction: Some(direction),
                    viewport: state.common.viewport,
                    parent_font_px: state.common.child_font_px,
                });
            }
        }
        if requests.is_empty() || quick_zero && eq_prep.queries.is_empty() {
            let children_size = measure_equal_wrap_finish(eq_prep, &[]);
            return finish_children_layout(
                state,
                flow_children,
                child_sizes,
                children_size,
                None,
                Vec::new(),
                0.0,
                (0.0, 0),
                nodes,
                cache,
                scope,
            );
        }
        state.phase = ContainerPhase::EqualWrap {
            flow_children,
            child_sizes,
            state: eq_prep,
        };
        return Ok(FrameOutcome::Push {
            frame: MeasureFrame {
                batch: PendingBatch::new(requests, false),
                body: Box::new(FrameBody::Container(state)),
            },
        });
    }

    if state.wrapping {
        let flex_baseline = (!state.context.inline()
            && style.display.is_some_and(DisplaySpec::is_flex_container))
        .then_some((style.as_ref(), &mut *cache, scope));
        let children_size = wrap_intrinsic_size(
            direction,
            state.wrap,
            &flow_children,
            &child_sizes,
            content_available,
            child_edge_base,
            gap,
            cross_gap,
            state.grid_tracks.as_deref().map(Vec::as_slice),
            state.common.viewport,
            state.common.child_font_px,
            nodes,
            writing,
            baseline_lines,
            flex_baseline,
        )?;
        return finish_children_layout(
            state,
            flow_children,
            child_sizes,
            children_size,
            None,
            Vec::new(),
            0.0,
            (0.0, 0),
            nodes,
            cache,
            scope,
        );
    }

    let definite_main = match direction {
        FlexDirection::Row => state.common.spec_width.is_some(),
        FlexDirection::Column => state.common.spec_height.is_some(),
    };
    let mut used_sizes = child_sizes.clone();
    if definite_main {
        let line = super::dynamic::DynamicLine::of(
            state.common.id,
            style.as_ref(),
            writing,
            true,
            &mut cache.dynamic,
            scope.map(|scope| scope.retained),
            false,
        );
        let adjust = distribute_flex_main(
            &flow_children,
            &mut used_sizes,
            direction,
            content_available,
            child_edge_base,
            gap,
            state.common.viewport,
            state.common.child_font_px,
            nodes,
            line,
        );
        let mut pending = Vec::new();
        let mut requests = Vec::new();
        for (index, child) in flow_children.iter().enumerate() {
            let Some(child_style) = nodes.style_ref(*child) else {
                continue;
            };
            if adjust.compressed(index) {
                continue;
            }
            if !cross_follows_used_main(
                &child_style,
                direction,
                child_sizes[index],
                used_sizes[index],
            ) {
                continue;
            }
            let main = main_extent(used_sizes[index], direction);
            if let Some(req) = at_main_request(
                *child,
                main,
                direction,
                content_available,
                state.common.viewport,
                state.common.child_font_px,
                nodes,
            ) {
                pending.push((index, *child, main));
                requests.push(req);
            } else {
                let at_main = Size::default();
                set_cross_extent(
                    &mut used_sizes[index],
                    direction,
                    cross_extent(at_main, direction),
                );
            }
        }
        if !requests.is_empty() {
            state.phase = ContainerPhase::DefiniteMain {
                flow_children,
                child_sizes,
                used_sizes,
                pending,
            };
            return Ok(FrameOutcome::Push {
                frame: MeasureFrame {
                    batch: PendingBatch::new(requests, false),
                    body: Box::new(FrameBody::Container(state)),
                },
            });
        }
    }

    finish_sequential_children(
        state,
        flow_children,
        child_sizes,
        &used_sizes,
        Vec::new(),
        nodes,
        cache,
        scope,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_sequential_children<'w, 's>(
    state: ContainerState,
    flow_children: Vec<StableNodeId>,
    child_sizes: Vec<Size>,
    used_sizes: &[Size],
    hypothetical: Vec<(StableNodeId, f32, Size)>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    let parent_box =
        gap_containing_block(state.common.style.as_ref(), state.common.content_available);
    let gap =
        state
            .common
            .style
            .main_gap_against_fonts(state.direction, parent_box, state.common.fonts);
    let child_edge_base = state
        .common
        .writing
        .logical_size(
            state.common.content_available.width,
            state.common.content_available.height,
        )
        .0;
    let gaps = gap * flow_children.len().saturating_sub(1) as f32;
    let mut main = f64::from(gaps);
    let mut cross = 0.0f32;
    let mut at_cross = 0usize;
    for ((child, size), used) in flow_children.iter().zip(&child_sizes).zip(used_sizes) {
        let margin = child_margin_of(*child, child_edge_base, state.common.child_font_px, nodes);
        main += sequential_main_term(*size, margin, state.direction);
        let reach = cross_extent(*used, state.direction) + cross_margin(margin, state.direction);
        if reach > cross {
            at_cross = 1;
        } else if reach == cross {
            at_cross += 1;
        }
        cross = cross.max(reach);
    }
    let sequential_main_sum = main;
    if !state.context.inline()
        && state
            .common
            .style
            .display
            .is_some_and(DisplaySpec::is_flex_container)
        && state.direction == state.common.writing.inline_flex_direction()
        && flow_children.iter().any(|&child| {
            nodes.style_ref(child).is_some_and(|child_style| {
                child_style.resolved_align_self(state.common.style.child_align_items())
                    == AlignSpec::Baseline
            })
        })
    {
        super::inline::prime_first_baselines(
            flow_children
                .iter()
                .copied()
                .zip(used_sizes.iter().copied()),
            child_edge_base,
            state.common.viewport,
            state.common.child_font_px,
            nodes,
            cache,
            scope,
        )?;
        let cross_axis = match state.direction {
            FlexDirection::Row => FlexDirection::Column,
            FlexDirection::Column => FlexDirection::Row,
        };
        let cross_reversed = state.common.writing.physical_axis_reversed(cross_axis)
            ^ matches!(state.wrap, FlexWrap::WrapReverse);
        if let Some((_, grown)) = super::inline::flex_baseline_line_metrics(
            flow_children
                .iter()
                .copied()
                .zip(used_sizes.iter().copied()),
            state.common.style.as_ref(),
            state.common.writing,
            state.direction,
            cross_reversed,
            child_edge_base,
            state.common.child_font_px,
            nodes,
            cache,
        ) && grown > cross
        {
            cross = grown;
            at_cross = 0;
        }
    }
    let sequential_cross = (cross, at_cross);
    let main = main as f32;
    let children_size = match state.direction {
        FlexDirection::Row => Size::new(main.max(0.0), cross),
        FlexDirection::Column => Size::new(cross, main.max(0.0)),
    };
    finish_children_layout(
        state,
        flow_children,
        child_sizes,
        children_size,
        None,
        hypothetical,
        sequential_main_sum,
        sequential_cross,
        nodes,
        cache,
        scope,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_children_layout<'w, 's>(
    state: ContainerState,
    flow_children: Vec<StableNodeId>,
    child_sizes: Vec<Size>,
    children_size: Size,
    recorded_grid: Option<Box<GridTrackPlan>>,
    hypothetical: Vec<(StableNodeId, f32, Size)>,
    sequential_main_sum: f64,
    sequential_cross: (f32, usize),
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    let size = finish_measure_container(
        state,
        flow_children,
        child_sizes,
        children_size,
        0.0,
        recorded_grid,
        hypothetical,
        sequential_main_sum,
        sequential_cross,
        nodes,
        cache,
        scope,
    )?;
    Ok(FrameOutcome::Done(size))
}

#[allow(clippy::too_many_arguments)]
fn finish_measure_container(
    #[allow(unused_mut)] mut state: ContainerState,
    flow_children: Vec<StableNodeId>,
    child_sizes: Vec<Size>,
    children: Size,
    _widest_float: f32,
    recorded_grid: Option<Box<GridTrackPlan>>,
    hypothetical: Vec<(StableNodeId, f32, Size)>,
    sequential_main_sum: f64,
    sequential_cross: (f32, usize),
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Size, UiWorldError> {
    #[cfg(feature = "benchmark")]
    if let Some(clock) = state.child_phase.as_mut() {
        clock.lap(5);
    }
    let common = &state.common;
    let id = common.id;
    let style = common.style.as_ref();
    let world = nodes.world;
    let text_metrics = common.text_metrics;
    let text_natural_width = common.text_natural_width;
    let text_wrap_limit = common.text_wrap_limit;
    let writing = common.writing;
    let available = common.available;
    let parent_direction = common.parent_direction;
    let viewport = common.viewport;
    let parent_font_px = common.parent_font_px;
    let child_font_px = common.child_font_px;
    let fonts = common.fonts;
    let edge_base = common.edge_base;
    let chrome = common.chrome;
    let content_available = common.content_available;
    let child_ids = &common.child_ids;
    let visual = common.visual;
    let lead = common.lead;
    let lead_inset = common.lead_inset;
    let header_chrome = common.header_chrome;
    let natural = common.natural;
    let ifc = state.context.inline();
    let grid_measure = state.context.grid_2d;
    let direction = state.direction;
    let wrapping = state.wrapping;
    let child_edge_base = writing
        .logical_size(content_available.width, content_available.height)
        .0;
    let text = text_metrics.unwrap_or_default();
    let text_width = text_inline_size(
        text,
        text_natural_width,
        text_wrap_limit,
        if header_chrome {
            content_available.width
        } else {
            (content_available.width - visual.width).max(0.0)
        },
    ) + if header_chrome { visual.width } else { 0.0 };
    let text_width = text_width.max(
        visual
            .text
            .min((content_available.width - visual.width).max(0.0)),
    );
    let mut content = Size::new(
        children.width.max(text_width),
        children.height.max(text.height),
    );
    #[cfg(feature = "rich-text")]
    if let Some(visual) = nodes
        .world
        .widget_visual_as::<crate::NativeMarkdownVisual>(id)
    {
        let geometry = nodes.world.markdown_layout(
            id,
            &visual.blocks,
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: content_available.width,
                height: 0.0,
            },
        );
        content = Size::new(
            crate::rich_text::markdown_content_width_with_strings(
                &visual.blocks,
                &geometry,
                nodes.world.framework_strings(),
            ),
            geometry.bounds.height,
        );
    }
    if let Some(natural) = natural {
        content = Size::new(
            content.width.max(natural.width),
            content.height.max(natural.height),
        );
    }
    if text_metrics.is_none()
        && flow_children.is_empty()
        && let Some(fs) = style.font_size.filter(|value| *value > 0.0)
    {
        content.height = content
            .height
            .max(text_line_box_height_px(fs, style.line_height));
    }
    if !header_chrome {
        content.width += visual.width;
    }
    content.height = content.height.max(visual.height);
    let lead_floor = lead.floor();
    content = Size::new(
        content.width.max(lead_floor.width) + lead_inset.width,
        content.height.max(lead_floor.height) + lead_inset.height,
    );
    let max_content_w = content.width + chrome.width;
    let stacked_min_w = child_sizes
        .iter()
        .zip(&flow_children)
        .map(|(size, child)| {
            let margin = child_margin_of(*child, child_edge_base, child_font_px, nodes);
            size.width + margin.left + margin.right
        })
        .fold(lead_floor.width, f32::max)
        + lead_inset.width
        + chrome.width;
    let fit_min_w = if !state.context.grid_2d && !wrapping && !direction.is_column() {
        let gap_box = gap_containing_block(style, content_available);
        let gap = style.main_gap_against_fonts(direction, gap_box, fonts);
        let vp = Some((viewport.width, viewport.height));
        let children_min_sum: f32 = child_sizes
            .iter()
            .zip(&flow_children)
            .map(|(size, &child)| {
                let margin = child_margin_of(child, child_edge_base, child_font_px, nodes);
                let base_w = match nodes.style_ref(child) {
                    Some(child_style)
                        if child_style.flex_shrink.unwrap_or(0.0) > 0.0
                            && child_style
                                .width
                                .is_none_or(|w| !matches!(w, LengthSpec::Px(_))) =>
                    {
                        let child_fonts = fonts_of(child_style.as_ref(), child_font_px);
                        child_style
                            .resolved_min_width_fonts(
                                Some(content_available.width),
                                vp,
                                child_fonts,
                            )
                            .min(size.width)
                    }
                    _ => size.width,
                };
                base_w + margin.left + margin.right
            })
            .sum();
        let gaps_w = if flow_children.len() > 1 {
            gap * (flow_children.len() - 1) as f32
        } else {
            0.0
        };
        let visual_w = if !header_chrome { visual.width } else { 0.0 };
        (children_min_sum + gaps_w + visual_w).max(lead_floor.width)
            + lead_inset.width
            + chrome.width
    } else {
        stacked_min_w
    };
    let min_content_w = if wrapping || direction.is_column() {
        stacked_min_w
    } else {
        max_content_w
    };
    let parent_stretches =
        parent_stretches_inline_auto_width(id, style, writing, parent_direction, nodes.world);
    let default_width = match style.width {
        Some(LengthSpec::MinContent) => min_content_w,
        Some(LengthSpec::MaxContent) | Some(LengthSpec::Shrink) => max_content_w,
        Some(LengthSpec::FitContent) => max_content_w.min(available.width).max(fit_min_w),
        _ if parent_stretches
            && (!flow_children.is_empty() || style.display == Some(DisplaySpec::Grid)) =>
        {
            available.width
        }
        _ => max_content_w,
    };
    let default_height = content.height + chrome.height;
    let mut size = finish_intrinsic_size(
        style,
        fonts,
        viewport,
        available,
        edge_base,
        chrome,
        parent_direction,
        default_width,
        default_height,
    );
    transfer_natural_ratio(
        style, natural, chrome, available, viewport, fonts, &mut size,
    );
    let bare_leaf = child_ids.is_empty()
        && text_metrics.is_none()
        && nodes.world.widget_visual_ref(id).is_none()
        && natural.is_none();
    if common.unforced && cache.records_measure_plans && !common.plain_leaf && !bare_leaf {
        let hugs = |spec: Option<LengthSpec>| {
            spec.is_none_or(|spec| matches!(spec, LengthSpec::Auto | LengthSpec::Shrink))
        };
        let plain_main = match direction {
            FlexDirection::Column => {
                hugs(style.height) && style.min_height.is_none() && style.max_height.is_none()
            }
            FlexDirection::Row => {
                hugs(style.width) && style.min_width.is_none() && style.max_width.is_none()
            }
        };
        let ifc_local = ifc
            && text_metrics.is_none()
            && nodes.world.widget_visual_ref(id).is_none()
            && ifc_local_measure(style, writing, &flow_children, child_ids.as_slice(), nodes)?;
        let cacheable = !style.equal_wrap
            && (!state.descendant_dependent_flow || ifc_local)
            && (!grid_measure || recorded_grid.is_some())
            && (!ifc || ifc_local)
            && state
                .grid_tracks
                .as_deref()
                .is_none_or(|tracks| tracks.is_empty())
            && nodes.world.children_layout_style_is_local(id)
            && natural.is_none();
        let sequential = cacheable
            && recorded_grid.is_none()
            && plain_main
            && !wrapping
            && hypothetical.is_empty()
            && style.justify_content == JustifySpec::Start
            && style.child_align_items() != AlignSpec::Baseline
            && style.aspect_ratio.is_none()
            && text_metrics.is_none()
            && nodes.world.widget_visual_ref(id).is_none()
            && flow_children.iter().all(|child| {
                nodes.style_ref(*child).is_some_and(|child_style| {
                    child_style.flex_grow.unwrap_or(0.0) <= 0.0
                        && child_style.flex_shrink.unwrap_or(0.0) <= 0.0
                        && child_style.align_self != Some(AlignSpec::Baseline)
                })
            });
        let has_container_baseline_items = !ifc
            && ((style.display.is_some_and(DisplaySpec::is_flex_container)
                && direction == writing.inline_flex_direction())
                || recorded_grid.is_some())
            && flow_children.iter().any(|&child| {
                nodes.style_ref(child).is_some_and(|cs| {
                    cs.resolved_align_self(style.child_align_items()) == AlignSpec::Baseline
                })
            });
        if cacheable && has_container_baseline_items {
            let baseline_items: Vec<(StableNodeId, Size)> = flow_children
                .iter()
                .copied()
                .zip(child_sizes.iter().copied())
                .filter(|(child, _)| {
                    nodes.style_ref(*child).is_some_and(|cs| {
                        cs.resolved_align_self(style.child_align_items()) == AlignSpec::Baseline
                    })
                })
                .collect();
            super::inline::prime_first_baselines(
                baseline_items,
                child_edge_base,
                viewport,
                child_font_px,
                nodes,
                cache,
                scope,
            )?;
        }
        let recorded = cacheable.then(|| {
            let ifc_strut = ifc.then(|| {
                crate::inline_content::ParentLineStrut::from(line_strut(
                    id,
                    &common.style,
                    writing,
                    child_font_px,
                    nodes,
                ))
            });
            let mut flow_cursor = 0usize;
            let mut entries: Vec<MeasuredChild> = child_ids
                .iter()
                .copied()
                .map(|child| {
                    let intrinsic = (flow_children.get(flow_cursor) == Some(&child)).then(|| {
                        let size = child_sizes[flow_cursor];
                        flow_cursor += 1;
                        size
                    });
                    let child_style_ref = nodes.style_ref(child);
                    let baseline = if let Some(strut) = ifc_strut {
                        intrinsic
                            .zip(child_style_ref.as_deref())
                            .and_then(|(size, cs)| {
                                ifc_atom_metrics(
                                    child,
                                    cs,
                                    size,
                                    writing,
                                    child_edge_base,
                                    child_font_px,
                                    strut,
                                    viewport,
                                    nodes,
                                )
                            })
                            .and_then(|placement| match placement {
                                crate::inline_content::AtomLinePlacement::Baseline {
                                    above,
                                    ..
                                } => Some(above),
                                crate::inline_content::AtomLinePlacement::Top { .. }
                                | crate::inline_content::AtomLinePlacement::Bottom { .. } => None,
                            })
                    } else if has_container_baseline_items {
                        intrinsic
                            .zip(child_style_ref.as_deref())
                            .filter(|(_, cs)| {
                                cs.resolved_align_self(style.child_align_items())
                                    == AlignSpec::Baseline
                            })
                            .and_then(|(size, cs)| {
                                let cf = fonts_of(cs, child_font_px);
                                nodes
                                    .baseline_metrics(
                                        child,
                                        cf.element_px,
                                        Some(child_edge_base),
                                        Some(size),
                                    )
                                    .first
                            })
                    } else {
                        None
                    };
                    MeasuredChild {
                        child,
                        style: child_style_ref.map(|s| s.to_arc()),
                        intrinsic: PackedOptSize::from_opt(intrinsic),
                        at_main: hypothetical
                            .iter()
                            .find(|(item, _, _)| *item == child)
                            .map(|(_, main, size)| Box::new((*main, *size))),
                        baseline: PackedOptF32::from_opt(baseline),
                    }
                })
                .collect();
            entries.sort_unstable_by_key(|entry| entry.child);
            MeasurePlan {
                available,
                parent_direction,
                parent_stretches,
                viewport,
                parent_font_px,
                style: Arc::clone(&common.style),
                writing,
                children: Arc::clone(child_ids),
                own: PlanOwnContent {
                    text_metrics,
                    text_natural_width,
                    text_wrap_limit,
                    strut: world.block_strut(id),
                    visual,
                }
                .boxed(),
                child_available: content_available,
                child_direction: direction,
                entries,
                size,
                sequential,
                main_sum: sequential_main_sum,
                flow_len: flow_children.len(),
                children_cross: sequential_cross.0,
                children_at_cross: sequential_cross.1,
                default_cross: cross_extent(Size::new(default_width, default_height), direction),
                grid: recorded_grid,
                stale: false,
            }
        });
        if common.had_plan && recorded.is_some() {
            cache.note_plan_rebuilt();
        }
        let slots = nodes.measure_plans.entry(id).or_default();
        match recorded {
            Some(plan) => slots.insert(plan),
            None => slots.clear(),
        }
    } else if common.unforced
        && (common.plain_leaf || bare_leaf)
        && scope.is_some_and(|scope| scope.retained.measure_plans.contains_key(&id))
    {
        nodes.measure_plans.entry(id).or_default().clear();
    }
    nodes.scratch.give_ids(flow_children);
    nodes.scratch.give_sizes(child_sizes);
    if common.unforced {
        cache.insert(common.cache_key, size);
        publish_atom_baseline(
            id,
            size,
            writing.inline_size(available.width, available.height),
            viewport,
            parent_font_px,
            nodes,
            cache,
            scope,
        )?;
        let baseline = nodes.baseline_metrics(
            id,
            fonts.element_px,
            Some(writing.inline_size(available.width, available.height)),
            Some(size),
        );
        cache.insert_intrinsic_bounds(
            common.cache_key,
            min_content_w.min(default_width),
            max_content_w.max(default_width),
            0.0,
            Some(default_height),
            Size::new(default_width, default_height),
            baseline.first,
            baseline.last,
            style
                .aspect_ratio
                .filter(|ratio| ratio.is_finite() && *ratio > 0.0),
        );
    }
    Ok(size)
}

#[allow(clippy::too_many_arguments)]
fn publish_atom_baseline(
    id: StableNodeId,
    size: Size,
    inline_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    if nodes.world.inline_parent(id).is_none() {
        return prime_first_baselines(
            [(id, size)],
            inline_base,
            viewport,
            parent_font_px,
            nodes,
            cache,
            scope,
        );
    }
    prime_atom_baselines(
        [(id, size)],
        inline_base,
        viewport,
        parent_font_px,
        nodes,
        cache,
        scope,
    )
}

/// Whether `child`, which changed this pass, may have changed what `plan`
/// holds for it: what [`measure_plan_children_unchanged`] asks of a plan a
/// pass consults, asked of one the pass did not, without measuring. Its
/// style is no longer the recorded one, or it is on the measure frontier and
/// its size is not a length of its own -- one that reads neither its
/// containing block nor the viewport measures the same under any constraint.
/// A child the plan holds no entry for arrives in the plan's own child-list
/// edit, and one the flow dropped adds nothing to it.
pub(super) fn plan_missed_change(
    plan: &MeasurePlan,
    child: StableNodeId,
    nodes: &LayoutInputMap<'_>,
    measure: &crate::NodeSet,
) -> bool {
    let Some(entry) = plan.entry(child) else {
        return false;
    };
    let current = nodes.style_ref(child);
    if !retained_style_matches(current.as_ref(), entry.style.as_ref()) {
        return true;
    }
    if !measure.contains(&child) || (entry.intrinsic.is_none() && entry.at_main.is_none()) {
        return false;
    }
    // Its baseline can move in a box of its own size.
    if entry.baseline.is_some() {
        return true;
    }
    let Some(style) = current else {
        return true;
    };
    let (own_width, own_height) = sizes_own_axes(&style, nodes.world.layout_writing(child));
    !(own_width && own_height && !style.depends_on_viewport())
}

pub(super) fn retained_style_matches(
    current: Option<&StyleRef<'_>>,
    cached: Option<&Arc<nana_ui_core::LayoutStyle>>,
) -> bool {
    match (current, cached) {
        (None, None) => true,
        (Some(current), Some(cached)) => {
            Arc::ptr_eq(current.arc(), cached) || layout_inputs_equal(current, cached)
        }
        _ => false,
    }
}

fn start_to_frame_outcome<'s>(outcome: StartOutcome<'s>) -> FrameOutcome<'s> {
    match outcome {
        StartOutcome::Fast(size) => FrameOutcome::Done(size),
        StartOutcome::Push(frame) => FrameOutcome::Push { frame },
    }
}

fn plan_start_to_frame_outcome<'w, 's>(
    start: PlanStart<'s>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    Ok(match start {
        PlanStart::Hit(size) => FrameOutcome::Done(size),
        PlanStart::Push(frame) => FrameOutcome::Push { frame },
        PlanStart::Fallthrough(common) => {
            start_to_frame_outcome(start_container_frame(common, nodes, cache, Some(scope))?)
        }
    })
}

fn unchanged_result_to_frame_outcome<'w, 's>(
    res: Result<PlanStart<'s>, Box<ContainerCommon>>,
    plan: &'s MeasurePlan,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    match res {
        Ok(start) => plan_start_to_frame_outcome(start, nodes, cache, scope),
        Err(common) => {
            let next = try_plan_sequential(common, plan, nodes, cache, scope)?;
            plan_start_to_frame_outcome(next, nodes, cache, scope)
        }
    }
}

fn step_plan_frame<'w, 's>(
    plan_state: PlanState<'s>,
    results: Vec<Size>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&'s ScopeContext<'s>>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    let PlanState { common, plan, wait } = plan_state;
    let Some(scope) = scope else {
        return Ok(start_to_frame_outcome(start_container_frame(
            common, nodes, cache, None,
        )?));
    };
    match wait {
        PlanWait::Unchanged { affected, step } => match step {
            UnchangedStep::Scoped {
                cursor,
                entry_child,
                cached,
                baseline,
                at_main,
            } => {
                let measured = results[0];
                let ifc = nodes
                    .contexts
                    .get(&common.id)
                    .copied()
                    .or_else(|| nodes.world.established_layout_context(common.id))
                    .is_some_and(|kind| kind == crate::FormattingContextKind::Inline);
                let edge_base = plan
                    .writing
                    .logical_size(plan.child_available.width, plan.child_available.height)
                    .0;
                let baseline_changed = if ifc {
                    prime_atom_baselines(
                        [(entry_child, measured)],
                        edge_base,
                        common.viewport,
                        common.child_font_px,
                        nodes,
                        cache,
                        Some(scope),
                    )?;
                    let parent_strut: crate::inline_content::ParentLineStrut = line_strut(
                        common.id,
                        &common.style,
                        plan.writing,
                        common.child_font_px,
                        nodes,
                    )
                    .into();
                    let current = nodes
                        .style_ref(entry_child)
                        .and_then(|child_style| {
                            ifc_atom_metrics(
                                entry_child,
                                child_style.as_ref(),
                                measured,
                                plan.writing,
                                edge_base,
                                common.child_font_px,
                                parent_strut,
                                common.viewport,
                                nodes,
                            )
                        })
                        .and_then(|placement| match placement {
                            crate::inline_content::AtomLinePlacement::Baseline {
                                above, ..
                            } => Some(above),
                            crate::inline_content::AtomLinePlacement::Top { .. }
                            | crate::inline_content::AtomLinePlacement::Bottom { .. } => None,
                        });
                    current != baseline
                } else if ((plan
                    .style
                    .display
                    .is_some_and(DisplaySpec::is_flex_container)
                    && plan.child_direction == plan.writing.inline_flex_direction())
                    || plan.grid.is_some())
                    && (baseline.is_some()
                        || nodes.style_ref(entry_child).is_some_and(|cs| {
                            cs.resolved_align_self(plan.style.child_align_items())
                                == AlignSpec::Baseline
                        }))
                {
                    super::inline::prime_first_baselines(
                        [(entry_child, measured)],
                        edge_base,
                        common.viewport,
                        common.child_font_px,
                        nodes,
                        cache,
                        Some(scope),
                    )?;
                    let current = nodes.style_ref(entry_child).and_then(|cs| {
                        let cf = fonts_of(cs.as_ref(), common.child_font_px);
                        nodes
                            .baseline_metrics(
                                entry_child,
                                cf.element_px,
                                Some(edge_base),
                                Some(measured),
                            )
                            .first
                    });
                    current != baseline
                } else {
                    false
                };
                if baseline_changed && !ifc {
                    return Ok(start_to_frame_outcome(start_container_frame(
                        common,
                        nodes,
                        cache,
                        Some(scope),
                    )?));
                }
                if measured != cached || baseline_changed {
                    let next = try_plan_sequential(common, plan, nodes, cache, scope)?;
                    return plan_start_to_frame_outcome(next, nodes, cache, scope);
                }
                if let Some((main, cached_main)) = at_main {
                    let Some(req) = at_main_request(
                        entry_child,
                        main,
                        plan.child_direction,
                        plan.child_available,
                        common.viewport,
                        common.child_font_px,
                        nodes,
                    ) else {
                        if Size::default() != cached_main {
                            let next = try_plan_sequential(common, plan, nodes, cache, scope)?;
                            return plan_start_to_frame_outcome(next, nodes, cache, scope);
                        }
                        let res =
                            advance_unchanged(common, plan, affected, cursor, nodes, cache, scope)?;
                        return unchanged_result_to_frame_outcome(res, plan, nodes, cache, scope);
                    };
                    return Ok(FrameOutcome::Push {
                        frame: MeasureFrame {
                            batch: PendingBatch::new(vec![req], false),
                            body: Box::new(FrameBody::Plan(PlanState {
                                common,
                                plan,
                                wait: PlanWait::Unchanged {
                                    affected,
                                    step: UnchangedStep::AtMain {
                                        cursor,
                                        cached_main,
                                    },
                                },
                            })),
                        },
                    });
                }
                let res = advance_unchanged(common, plan, affected, cursor, nodes, cache, scope)?;
                unchanged_result_to_frame_outcome(res, plan, nodes, cache, scope)
            }
            UnchangedStep::AtMain {
                cursor,
                cached_main,
            } => {
                let measured = results[0];
                if measured != cached_main {
                    let next = try_plan_sequential(common, plan, nodes, cache, scope)?;
                    return plan_start_to_frame_outcome(next, nodes, cache, scope);
                }
                let res = advance_unchanged(common, plan, affected, cursor, nodes, cache, scope)?;
                unchanged_result_to_frame_outcome(res, plan, nodes, cache, scope)
            }
        },
        PlanWait::Sequential(mut seq) => {
            let step = std::mem::replace(
                &mut seq.step,
                SequentialStep::_Marker(std::marker::PhantomData),
            );
            match step {
                SequentialStep::Affected {
                    cursor,
                    entry_child,
                    old,
                    current_style,
                } => {
                    let measured = results[0];
                    let old_style = plan.entry(entry_child).and_then(|e| e.style.clone());
                    let old_margin =
                        seq_margin_of(&old_style, seq.child_edge_base, common.child_font_px);
                    let new_margin =
                        seq_margin_of(&current_style, seq.child_edge_base, common.child_font_px);
                    if !seq.cross_own
                        && (cross_extent(measured, seq.direction)
                            + cross_margin(new_margin, seq.direction))
                            != (cross_extent(old, seq.direction)
                                + cross_margin(old_margin, seq.direction))
                    {
                        let next = try_plan_flex_line(common, plan, nodes, cache, scope)?;
                        return plan_start_to_frame_outcome(next, nodes, cache, scope);
                    }
                    seq.main_sum += sequential_main_term(measured, new_margin, seq.direction)
                        - sequential_main_term(old, old_margin, seq.direction);
                    seq.patches.push((entry_child, measured, current_style));
                    seq.step = SequentialStep::Affected {
                        cursor,
                        entry_child,
                        old: Size::default(),
                        current_style: None,
                    };
                    let next = advance_sequential_plan(common, plan, seq, nodes, cache, scope)?;
                    plan_start_to_frame_outcome(next, nodes, cache, scope)
                }
                SequentialStep::Arrival {
                    cursor,
                    child,
                    child_style,
                } => {
                    let measured = results[0];
                    let margin =
                        seq_margin_of(&child_style, seq.child_edge_base, common.child_font_px);
                    if !seq.cross_own {
                        let reached = cross_extent(measured, seq.direction)
                            + cross_margin(margin, seq.direction);
                        if reached > plan.children_cross {
                            let next = try_plan_flex_line(common, plan, nodes, cache, scope)?;
                            return plan_start_to_frame_outcome(next, nodes, cache, scope);
                        }
                        if reached == plan.children_cross {
                            seq.at_cross += 1;
                        }
                    }
                    seq.main_sum += sequential_main_term(measured, margin, seq.direction);
                    seq.flow_len += 1;
                    seq.arrived.push(MeasuredChild {
                        child,
                        style: child_style,
                        intrinsic: PackedOptSize::from_size(measured),
                        at_main: None,
                        baseline: PackedOptF32::NONE,
                    });
                    let next = advance_sequential_arrivals(
                        common, plan, seq, cursor, nodes, cache, scope,
                    )?;
                    plan_start_to_frame_outcome(next, nodes, cache, scope)
                }
                SequentialStep::_Marker(_) => unreachable!(),
            }
        }
        PlanWait::FlexLine(mut flex_state) => {
            let measured = results[0];
            let index = flex_state.active_index;
            if main_extent(measured, flex_state.direction).to_bits()
                != main_extent(flex_state.sizes[index], flex_state.direction).to_bits()
            {
                let next = try_plan_ifc_line(common, plan, nodes, cache, scope)?;
                return plan_start_to_frame_outcome(next, nodes, cache, scope);
            }
            if measured != flex_state.sizes[index] {
                flex_state.sizes[index] = measured;
                flex_state.patched = true;
            }
            let next = advance_flex_line_delta(common, plan, flex_state, nodes, cache, scope)?;
            plan_start_to_frame_outcome(next, nodes, cache, scope)
        }
        PlanWait::IfcLine(ifc_state) => {
            if let Some(size) = super::inline::ifc_line_measure_delta_finish(
                common.id,
                plan,
                ifc_state,
                &results,
                common.viewport,
                common.child_font_px,
                nodes,
                cache,
                scope,
            )? {
                return Ok(FrameOutcome::Done(finish_plan_reused(
                    &common, size, true, cache,
                )));
            }
            let next = try_plan_grid(common, plan, nodes, cache, scope)?;
            plan_start_to_frame_outcome(next, nodes, cache, scope)
        }
        PlanWait::Grid(mut grid_state) => match grid_state.step {
            GridDeltaStep::Children => {
                for ((index, style_same), measured) in grid_state.queries.drain(..).zip(results) {
                    if measured != grid_state.sizes[index] || !style_same {
                        grid_state.sizes[index] = measured;
                        grid_state.patched = true;
                    }
                }
                if !grid_state.patched {
                    let edge_base = plan
                        .writing
                        .logical_size(plan.child_available.width, plan.child_available.height)
                        .0;
                    let align_items = plan.style.child_align_items();
                    let baseline_items: Vec<(StableNodeId, Size)> = grid_state
                        .flow
                        .iter()
                        .copied()
                        .zip(grid_state.sizes.iter().copied())
                        .filter(|&(child, _)| {
                            nodes.style_ref(child).is_some_and(|cs| {
                                cs.resolved_align_self(align_items) == AlignSpec::Baseline
                            })
                        })
                        .collect();
                    if !baseline_items.is_empty() {
                        super::inline::prime_first_baselines(
                            baseline_items.iter().copied(),
                            edge_base,
                            common.viewport,
                            common.child_font_px,
                            nodes,
                            cache,
                            Some(scope),
                        )?;
                        for (child, size) in baseline_items {
                            let cached_base = plan.entry(child).and_then(|e| e.baseline.get());
                            let current_base = nodes.style_ref(child).and_then(|cs| {
                                let cf = fonts_of(cs.as_ref(), common.child_font_px);
                                nodes
                                    .baseline_metrics(
                                        child,
                                        cf.element_px,
                                        Some(edge_base),
                                        Some(size),
                                    )
                                    .first
                            });
                            if current_base != cached_base {
                                grid_state.patched = true;
                                break;
                            }
                        }
                    }
                }
                if !grid_state.patched {
                    return Ok(start_to_frame_outcome(start_container_frame(
                        common,
                        nodes,
                        cache,
                        Some(scope),
                    )?));
                }
                start_grid_delta_pass2(common, plan, grid_state, nodes, cache, scope)
            }
            GridDeltaStep::Pass2(mut prepared) => {
                super::grid::apply_grid_row_baseline_shim(
                    common.style.as_ref(),
                    &mut prepared,
                    &results,
                    common.viewport,
                    common.child_font_px,
                    nodes,
                    cache,
                    Some(scope),
                )?;
                let solved = layout_grid_2d_finish(prepared, &results);
                finish_grid_delta(
                    common,
                    plan,
                    grid_state.flow,
                    grid_state.sizes,
                    solved,
                    nodes,
                    cache,
                    scope,
                )
            }
        },
    }
}

fn seq_margin_of(
    style: &Option<Arc<nana_ui_core::LayoutStyle>>,
    edge_base: f32,
    child_font_px: f32,
) -> nana_ui_core::PaddingSpec {
    style.as_ref().map_or_else(Default::default, |style| {
        style.resolved_margin_against_fonts(Some(edge_base), fonts_of(style, child_font_px))
    })
}

/// Apply measured children's main-size deltas to a sequential container.
///
/// Returns a [`PlanStart`] either hitting with a patched size, pushing a child
/// query frame, or falling through to `try_plan_flex_line`.
#[allow(clippy::too_many_arguments)]
fn try_plan_sequential<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<PlanStart<'s>, UiWorldError> {
    let id = common.id;
    if !plan.sequential {
        return try_plan_flex_line(common, plan, nodes, cache, scope);
    }
    let flex_or_grid = plan
        .style
        .display
        .is_some_and(|display| display.is_flex_container() || display.is_grid_container());
    if !flex_or_grid
        && scope.affected_children(id).iter().any(|&child| {
            nodes.world.parent_id(child) == Some(id)
                && nodes.style_ref(child).is_some_and(|style| {
                    !crate::layout_node::leaves_parent_flow(Some(&plan.style), &style)
                        && style.is_inline_level()
                })
        })
    {
        return try_plan_flex_line(common, plan, nodes, cache, scope);
    }
    let direction = plan.child_direction;
    let child_edge_base = plan
        .writing
        .logical_size(plan.child_available.width, plan.child_available.height)
        .0;
    let (own_width, own_height) = sizes_own_axes(plan.style.as_ref(), plan.writing);
    let cross_own = match direction {
        FlexDirection::Column => own_width,
        FlexDirection::Row => own_height,
    };
    let edit = (!Arc::ptr_eq(&plan.children, &common.child_ids))
        .then(|| ChildListEdit::between(&plan.children, &common.child_ids));
    let seq = SequentialDeltaState {
        affected: scope.affected_children(id),
        direction,
        child_edge_base,
        cross_own,
        edit,
        main_sum: plan.main_sum,
        patches: Vec::new(),
        flow_len: plan.flow_len,
        at_cross: plan.children_at_cross,
        arrivals: Vec::new(),
        arrived: Vec::new(),
        step: SequentialStep::Affected {
            cursor: 0,
            entry_child: id,
            old: Size::default(),
            current_style: None,
        },
    };
    advance_sequential_plan(common, plan, seq, nodes, cache, scope)
}

fn advance_sequential_plan<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    mut seq: SequentialDeltaState<'s>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<PlanStart<'s>, UiWorldError> {
    let mut cursor = match &seq.step {
        SequentialStep::Affected { cursor, .. } => *cursor,
        SequentialStep::Arrival { .. } | SequentialStep::_Marker(_) => seq.affected.len(),
    };
    let id = common.id;
    let world = nodes.world;
    while cursor < seq.affected.len() {
        let affected = seq.affected[cursor];
        cursor += 1;
        let Some(entry) = plan.entry(affected) else {
            continue;
        };
        if let Some(edit) = seq.edit
            && (world.parent_id(affected) != Some(id) || edit.arrived(world, id, affected))
        {
            continue;
        }
        let current_style = nodes.style_ref(entry.child);
        let context = crate::FormattingContextKind::from_display(&plan.style);
        let participation = |style: Option<&LayoutStyle>| {
            style.map(|style| nodes.participation(entry.child, style, context))
        };
        if participation(current_style.as_deref()) != participation(entry.style.as_deref()) {
            return try_plan_flex_line(common, plan, nodes, cache, scope);
        }
        let Some(old) = entry.intrinsic.get() else {
            continue;
        };
        if !scope.measure.contains(&affected) {
            if !retained_style_matches(current_style.as_ref(), entry.style.as_ref()) {
                return try_plan_flex_line(common, plan, nodes, cache, scope);
            }
            continue;
        }
        let req = MeasureRequest {
            id: entry.child,
            forced: None,
            available: plan.child_available,
            parent_direction: Some(seq.direction),
            viewport: common.viewport,
            parent_font_px: common.child_font_px,
        };
        seq.step = SequentialStep::Affected {
            cursor,
            entry_child: entry.child,
            old,
            current_style: current_style.map(|s| s.to_arc()),
        };
        return Ok(PlanStart::Push(MeasureFrame {
            batch: PendingBatch::new(vec![req], false),
            body: Box::new(FrameBody::Plan(PlanState {
                common,
                plan,
                wait: PlanWait::Sequential(seq),
            })),
        }));
    }

    if let Some(edit) = seq.edit {
        if plan
            .style
            .display
            .is_some_and(DisplaySpec::is_grid_container)
        {
            return try_plan_flex_line(common, plan, nodes, cache, scope);
        }
        let reach = |size: Size, margin| {
            cross_extent(size, seq.direction) + cross_margin(margin, seq.direction)
        };
        for &child in edit.left(&plan.children) {
            let Some(entry) = plan.entry(child) else {
                return try_plan_flex_line(common, plan, nodes, cache, scope);
            };
            let Some(old) = entry.intrinsic.get() else {
                continue;
            };
            let margin = seq_margin_of(&entry.style, seq.child_edge_base, common.child_font_px);
            if !seq.cross_own {
                let reached = reach(old, margin);
                if reached > plan.children_cross {
                    return try_plan_flex_line(common, plan, nodes, cache, scope);
                }
                if reached == plan.children_cross {
                    let Some(fewer) = seq.at_cross.checked_sub(1) else {
                        return try_plan_flex_line(common, plan, nodes, cache, scope);
                    };
                    seq.at_cross = fewer;
                }
            }
            seq.main_sum -= sequential_main_term(old, margin, seq.direction);
            let Some(fewer) = seq.flow_len.checked_sub(1) else {
                return try_plan_flex_line(common, plan, nodes, cache, scope);
            };
            seq.flow_len = fewer;
        }
        seq.arrivals = edit.arrivals(&common.child_ids).to_vec();
        return advance_sequential_arrivals(common, plan, seq, 0, nodes, cache, scope);
    } else if seq.patches.is_empty() {
        return try_plan_flex_line(common, plan, nodes, cache, scope);
    }
    finish_sequential_delta(common, plan, seq, nodes, cache, scope)
}

fn advance_sequential_arrivals<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    mut seq: SequentialDeltaState<'s>,
    mut cursor: usize,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<PlanStart<'s>, UiWorldError> {
    let style = plan.style.as_ref();
    let flex = style.display.is_some_and(DisplaySpec::is_flex_container);
    while cursor < seq.arrivals.len() {
        let child = seq.arrivals[cursor];
        cursor += 1;
        let child_style = nodes.style_ref(child);
        let in_flow = match child_style.as_deref() {
            None => false,
            Some(arrival)
                if arrival.omits_box()
                    || crate::layout_node::leaves_parent_flow(Some(style), arrival) =>
            {
                false
            }
            Some(arrival)
                if (!flex && arrival.is_inline_level())
                    || arrival.flex_grow.unwrap_or(0.0) > 0.0
                    || arrival.flex_shrink.unwrap_or(0.0) > 0.0 =>
            {
                return try_plan_flex_line(common, plan, nodes, cache, scope);
            }
            Some(_) => true,
        };
        if in_flow {
            let req = MeasureRequest {
                id: child,
                forced: None,
                available: plan.child_available,
                parent_direction: Some(seq.direction),
                viewport: common.viewport,
                parent_font_px: common.child_font_px,
            };
            seq.step = SequentialStep::Arrival {
                cursor,
                child,
                child_style: child_style.map(|s| s.to_arc()),
            };
            return Ok(PlanStart::Push(MeasureFrame {
                batch: PendingBatch::new(vec![req], false),
                body: Box::new(FrameBody::Plan(PlanState {
                    common,
                    plan,
                    wait: PlanWait::Sequential(seq),
                })),
            }));
        }
        seq.arrived.push(MeasuredChild {
            child,
            style: child_style.map(|s| s.to_arc()),
            intrinsic: PackedOptSize::NONE,
            at_main: None,
            baseline: PackedOptF32::NONE,
        });
    }
    if plan.flow_len == 0
        || seq.flow_len == 0
        || (!seq.cross_own && plan.children_at_cross > 0 && seq.at_cross == 0)
    {
        return try_plan_flex_line(common, plan, nodes, cache, scope);
    }
    let fonts = fonts_of(style, plan.parent_font_px);
    let gap = style.main_gap_against_fonts(
        seq.direction,
        gap_containing_block(style, plan.child_available),
        fonts,
    );
    let gaps = |count: usize| f64::from(gap * count.saturating_sub(1) as f32);
    seq.main_sum += gaps(seq.flow_len) - gaps(plan.flow_len);
    finish_sequential_delta(common, plan, seq, nodes, cache, scope)
}

fn finish_sequential_delta<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    seq: SequentialDeltaState<'s>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<PlanStart<'s>, UiWorldError> {
    let id = common.id;
    let main = (seq.main_sum as f32).max(0.0);
    let Some(size) = finish_planned_size(id, plan, nodes, |chrome| match seq.direction {
        FlexDirection::Column => (plan.default_cross, main + chrome.height),
        FlexDirection::Row if plan.parent_direction.is_none() => {
            (plan.available.width, plan.default_cross)
        }
        FlexDirection::Row => (main + chrome.width, plan.default_cross),
    })?
    else {
        return try_plan_flex_line(common, plan, nodes, cache, scope);
    };
    let mut updated = plan.clone();
    updated.size = size;
    updated.main_sum = seq.main_sum;
    if let Some(edit) = seq.edit {
        updated.children = Arc::clone(&common.child_ids);
        updated.flow_len = seq.flow_len;
        updated.children_at_cross = seq.at_cross;
        for child in edit.left(&plan.children) {
            if let Ok(slot) = updated
                .entries
                .binary_search_by_key(child, |entry| entry.child)
            {
                updated.entries.remove(slot);
            }
        }
        for entry in seq.arrived {
            match updated
                .entries
                .binary_search_by_key(&entry.child, |held| held.child)
            {
                Ok(slot) => updated.entries[slot] = entry,
                Err(slot) => updated.entries.insert(slot, entry),
            }
        }
    }
    for (child, intrinsic, style) in seq.patches {
        if let Ok(slot) = updated
            .entries
            .binary_search_by_key(&child, |entry| entry.child)
        {
            updated.entries[slot].intrinsic = PackedOptSize::from_size(intrinsic);
            if let Some(style) = style {
                updated.entries[slot].style = Some(style);
            }
        }
    }
    nodes.measure_plans.entry(id).or_default().insert(updated);
    Ok(PlanStart::Hit(finish_plan_reused(
        &common, size, true, cache,
    )))
}

/// One in-flow child's share of a sequential container's main size: its
/// border box and both main margins. Every share is a few `f32` and every
/// total a sum of them, both exact in `f64` at any size a document reaches,
/// so taking one child's share out of a retained total and putting its new
/// one in gives the bits summing every child would. In `f32` the patched
/// total drifted from the summed one by an ulp an edit.
fn sequential_main_term(
    size: Size,
    margin: nana_ui_core::PaddingSpec,
    direction: FlexDirection,
) -> f64 {
    f64::from(main_extent(size, direction))
        + f64::from(main_start_margin(margin, direction))
        + f64::from(main_end_margin(margin, direction))
}

/// A planned container's used size, the final step the full measure takes:
/// `defaults` turns the container's chrome into its content-derived default
/// width and height. `None` when the node is gone.
fn finish_planned_size(
    id: StableNodeId,
    plan: &MeasurePlan,
    nodes: &mut LayoutInputMap<'_>,
    defaults: impl FnOnce(Size) -> (f32, f32),
) -> Result<Option<Size>, UiWorldError> {
    let Some(node) = nodes.get(id)? else {
        return Ok(None);
    };
    let edge_base = node
        .containing_writing
        .inline_size(plan.available.width, plan.available.height);
    let style = plan.style.as_ref();
    let fonts = fonts_of(style, plan.parent_font_px);
    let padding = style.resolved_padding_against_fonts(Some(edge_base), fonts);
    let border = style.resolved_border_edges();
    let chrome = Size::new(
        padding.left + padding.right + border.left + border.right,
        padding.top + padding.bottom + border.top + border.bottom,
    );
    let (default_width, default_height) = defaults(chrome);
    Ok(Some(finish_intrinsic_size(
        style,
        fonts,
        plan.viewport,
        plan.available,
        edge_base,
        chrome,
        plan.parent_direction,
        default_width,
        default_height,
    )))
}

/// Recompute a wrapping flex from the lines whose items changed.
///
/// Returns `PlanStart::Fallthrough` (after trying `try_plan_ifc_line`) when
/// line membership cannot be proved from the cached main sizes.
fn try_plan_flex_line<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<PlanStart<'s>, UiWorldError> {
    let id = common.id;
    let style = plan.style.as_ref();
    if plan.sequential
        || !Arc::ptr_eq(&plan.children, &common.child_ids)
        || !flex_line_local_style(style)
    {
        return try_plan_ifc_line(common, plan, nodes, cache, scope);
    }
    if plan.text_metrics().is_some() || nodes.world.widget_visual_ref(id).is_some() {
        return try_plan_ifc_line(common, plan, nodes, cache, scope);
    }
    let direction = plan.child_direction;
    match direction {
        FlexDirection::Row
            if style.height.is_some()
                || style.min_height.is_some()
                || style.max_height.is_some() =>
        {
            return try_plan_ifc_line(common, plan, nodes, cache, scope);
        }
        FlexDirection::Column
            if style.width.is_some() || style.min_width.is_some() || style.max_width.is_some() =>
        {
            return try_plan_ifc_line(common, plan, nodes, cache, scope);
        }
        _ => {}
    }
    let tracks = match direction {
        FlexDirection::Row => style.active_grid_columns(),
        FlexDirection::Column => style.active_grid_rows(),
    };
    if tracks.is_some_and(|tracks| !tracks.is_empty()) {
        return try_plan_ifc_line(common, plan, nodes, cache, scope);
    }
    let mut flow = Vec::new();
    let mut sizes = Vec::new();
    for child in plan.children.iter().copied() {
        let Some(entry) = plan.entry(child) else {
            return try_plan_ifc_line(common, plan, nodes, cache, scope);
        };
        let Some(intrinsic) = entry.intrinsic.get() else {
            continue;
        };
        let Some(child_style) = entry.style.as_deref() else {
            return try_plan_ifc_line(common, plan, nodes, cache, scope);
        };
        if child_blocks_flex_line_local(child_style) {
            return try_plan_ifc_line(common, plan, nodes, cache, scope);
        }
        let cross = match direction {
            FlexDirection::Row => child_style.height,
            FlexDirection::Column => child_style.width,
        };
        if matches!(cross, Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)) {
            return try_plan_ifc_line(common, plan, nodes, cache, scope);
        }
        flow.push(child);
        sizes.push(intrinsic);
    }
    if flow.is_empty() {
        return try_plan_ifc_line(common, plan, nodes, cache, scope);
    }
    let old_sizes = sizes.clone();
    let flex_state = FlexLineDeltaState {
        affected: scope.affected_children(id),
        direction,
        flow,
        old_sizes,
        sizes,
        patched: false,
        cursor: 0,
        active_index: 0,
    };
    advance_flex_line_delta(common, plan, flex_state, nodes, cache, scope)
}

fn advance_flex_line_delta<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    mut flex_state: FlexLineDeltaState<'s>,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<PlanStart<'s>, UiWorldError> {
    let id = common.id;
    let direction = flex_state.direction;
    while flex_state.cursor < flex_state.affected.len() {
        let affected = flex_state.affected[flex_state.cursor];
        flex_state.cursor += 1;
        let Some(index) = flex_state.flow.iter().position(|child| *child == affected) else {
            continue;
        };
        if !scope.measure.contains(&affected) {
            let Some(entry) = plan.entry(affected) else {
                continue;
            };
            if !retained_style_matches(nodes.style_ref(affected).as_ref(), entry.style.as_ref()) {
                return try_plan_ifc_line(common, plan, nodes, cache, scope);
            }
            continue;
        }
        if let Some(entry) = plan.entry(affected)
            && let Some(cached) = entry.style.as_ref()
            && !nodes.style_ref(affected).is_some_and(|current| {
                Arc::ptr_eq(current.arc(), cached)
                    || super::same_but_cross(&current, cached, direction)
            })
        {
            return try_plan_ifc_line(common, plan, nodes, cache, scope);
        }
        let req = MeasureRequest {
            id: affected,
            forced: None,
            available: plan.child_available,
            parent_direction: Some(direction),
            viewport: common.viewport,
            parent_font_px: common.child_font_px,
        };
        flex_state.active_index = index;
        return Ok(PlanStart::Push(MeasureFrame {
            batch: PendingBatch::new(vec![req], false),
            body: Box::new(FrameBody::Plan(PlanState {
                common,
                plan,
                wait: PlanWait::FlexLine(flex_state),
            })),
        }));
    }

    if !flex_state.patched {
        return try_plan_ifc_line(common, plan, nodes, cache, scope);
    }
    let style = plan.style.as_ref();
    let fonts = fonts_of(style, plan.parent_font_px);
    let parent_box = gap_containing_block(style, plan.child_available);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    let edge_base = plan
        .writing
        .logical_size(plan.child_available.width, plan.child_available.height)
        .0;
    let old_lines = pack_wrap_lines(
        &flex_state.flow,
        &flex_state.old_sizes,
        direction,
        plan.child_available,
        edge_base,
        gap,
        None,
        common.viewport,
        common.child_font_px,
        nodes,
        false,
    );
    let new_lines = pack_wrap_lines(
        &flex_state.flow,
        &flex_state.sizes,
        direction,
        plan.child_available,
        edge_base,
        gap,
        None,
        common.viewport,
        common.child_font_px,
        nodes,
        false,
    );
    if old_lines != new_lines {
        return try_plan_ifc_line(common, plan, nodes, cache, scope);
    }
    let old_content = wrap_intrinsic_size(
        direction,
        FlexWrap::Wrap,
        &flex_state.flow,
        &flex_state.old_sizes,
        plan.child_available,
        edge_base,
        gap,
        cross_gap,
        None,
        common.viewport,
        common.child_font_px,
        nodes,
        plan.writing,
        None,
        Some((&plan.style, &mut *cache, Some(scope))),
    )?;
    let new_content = wrap_intrinsic_size(
        direction,
        FlexWrap::Wrap,
        &flex_state.flow,
        &flex_state.sizes,
        plan.child_available,
        edge_base,
        gap,
        cross_gap,
        None,
        common.viewport,
        common.child_font_px,
        nodes,
        plan.writing,
        None,
        Some((&plan.style, &mut *cache, Some(scope))),
    )?;
    let mut size = plan.size;
    match direction {
        FlexDirection::Row => size.height += new_content.height - old_content.height,
        FlexDirection::Column => size.width += new_content.width - old_content.width,
    }
    let mut updated = plan.clone();
    updated.size = size;
    for (child, measured) in flex_state.flow.into_iter().zip(flex_state.sizes) {
        let Ok(slot) = updated
            .entries
            .binary_search_by_key(&child, |entry| entry.child)
        else {
            continue;
        };
        updated.entries[slot].intrinsic = PackedOptSize::from_size(measured);
        if let Some(child_style) = nodes.style(child) {
            updated.entries[slot].style = Some(child_style);
        }
    }
    nodes.measure_plans.entry(id).or_default().insert(updated);
    Ok(PlanStart::Hit(finish_plan_reused(
        &common, size, true, cache,
    )))
}

fn try_plan_ifc_line<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<PlanStart<'s>, UiWorldError> {
    let Some(ifc_state) =
        super::inline::ifc_line_measure_delta_begin(common.id, plan, nodes, scope)
    else {
        return try_plan_grid(common, plan, nodes, cache, scope);
    };
    let requests = ifc_state
        .queries
        .iter()
        .map(|&(_, child)| MeasureRequest {
            id: child,
            forced: None,
            available: plan.child_available,
            parent_direction: Some(plan.child_direction),
            viewport: common.viewport,
            parent_font_px: common.child_font_px,
        })
        .collect();
    Ok(PlanStart::Push(MeasureFrame {
        batch: PendingBatch::new(requests, false),
        body: Box::new(FrameBody::Plan(PlanState {
            common,
            plan,
            wait: PlanWait::IfcLine(ifc_state),
        })),
    }))
}

fn frame_to_plan_start<'s>(outcome: FrameOutcome<'s>) -> PlanStart<'s> {
    match outcome {
        FrameOutcome::Done(size) => PlanStart::Hit(size),
        FrameOutcome::Push { frame } => PlanStart::Push(frame),
    }
}

/// Re-solve a grid from cached contributions, measuring only items whose
/// contribution this pass can change.
fn try_plan_grid<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<PlanStart<'s>, UiWorldError> {
    let id = common.id;
    if plan.grid.is_none()
        || !Arc::ptr_eq(&plan.children, &common.child_ids)
        || plan
            .own
            .as_ref()
            .is_some_and(|own| own.visual.lead.is_some())
    {
        return Ok(PlanStart::Fallthrough(common));
    }
    let style = plan.style.as_ref();
    if style.is_subgrid_columns() || style.is_subgrid_rows() {
        return Ok(PlanStart::Fallthrough(common));
    }
    if plan.text_metrics().is_some() || nodes.world.widget_visual_ref(id).is_some() {
        return Ok(PlanStart::Fallthrough(common));
    }
    if style.width.is_some_and(LengthSpec::is_content_sized)
        || style.height.is_some_and(LengthSpec::is_content_sized)
    {
        return Ok(PlanStart::Fallthrough(common));
    }
    let mut flow = Vec::new();
    let mut sizes = Vec::new();
    let mut patched = false;
    for child in plan.children.iter().copied() {
        let Some(child_style) = nodes.style_ref(child) else {
            continue;
        };
        if !grid_child_in_flow(child_style.as_ref()) {
            continue;
        }
        let Some(entry) = plan.entry(child) else {
            return Ok(PlanStart::Fallthrough(common));
        };
        if entry.intrinsic.is_none() && !scope.measure.contains(&child) {
            return Ok(PlanStart::Fallthrough(common));
        }
        flow.push(child);
        sizes.push(entry.intrinsic.get().unwrap_or_default());
    }
    if flow.is_empty() {
        return Ok(PlanStart::Fallthrough(common));
    }
    let mut queries = Vec::new();
    let mut requests = Vec::new();
    for (index, &child) in flow.iter().enumerate() {
        if !scope.affected.contains(&child) {
            continue;
        }
        let current = nodes.style_ref(child);
        let style_same = plan
            .entry(child)
            .is_some_and(|entry| retained_style_matches(current.as_ref(), entry.style.as_ref()));
        if !scope.measure.contains(&child) {
            if !style_same {
                patched = true;
            }
            continue;
        }
        let available = current
            .as_ref()
            .map_or(plan.child_available, |child_style| {
                grid_item_measure_available(child_style.as_ref(), plan.child_available)
            });
        queries.push((index, style_same));
        requests.push(MeasureRequest {
            id: child,
            forced: None,
            available,
            parent_direction: Some(plan.child_direction),
            viewport: common.viewport,
            parent_font_px: common.child_font_px,
        });
    }
    if requests.is_empty() {
        if !patched {
            return Ok(PlanStart::Fallthrough(common));
        }
        let grid_state = GridDeltaState {
            flow,
            sizes,
            queries,
            patched,
            step: GridDeltaStep::Children,
        };
        return Ok(frame_to_plan_start(start_grid_delta_pass2(
            common, plan, grid_state, nodes, cache, scope,
        )?));
    }
    Ok(PlanStart::Push(MeasureFrame {
        batch: PendingBatch::new(requests, false),
        body: Box::new(FrameBody::Plan(PlanState {
            common,
            plan,
            wait: PlanWait::Grid(GridDeltaState {
                flow,
                sizes,
                queries,
                patched,
                step: GridDeltaStep::Children,
            }),
        })),
    }))
}

fn start_grid_delta_pass2<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    mut grid_state: GridDeltaState,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    sort_ids_with_sizes(&mut grid_state.flow, &mut grid_state.sizes, nodes);
    let style = plan.style.as_ref();
    let fonts = fonts_of(style, plan.parent_font_px);
    match layout_grid_2d_begin(
        style,
        plan.writing,
        &grid_state.flow,
        &grid_state.sizes,
        plan.child_available,
        fonts,
        None,
        nodes,
    ) {
        Grid2dStart::Ready(mut prepared) => {
            super::grid::apply_grid_row_baseline_shim(
                style,
                &mut prepared,
                &[],
                common.viewport,
                fonts.element_px,
                nodes,
                cache,
                Some(scope),
            )?;
            let solved = layout_grid_2d_finish(prepared, &[]);
            finish_grid_delta(
                common,
                plan,
                grid_state.flow,
                grid_state.sizes,
                solved,
                nodes,
                cache,
                scope,
            )
        }
        Grid2dStart::NeedsPass2(prepared) => {
            let requests = prepared
                .queries
                .iter()
                .map(|&(_, item_id, avail)| MeasureRequest {
                    id: item_id,
                    forced: None,
                    available: avail,
                    parent_direction: Some(FlexDirection::Row),
                    viewport: common.viewport,
                    parent_font_px: fonts.element_px,
                })
                .collect();
            grid_state.step = GridDeltaStep::Pass2(prepared);
            Ok(FrameOutcome::Push {
                frame: MeasureFrame {
                    batch: PendingBatch::new(requests, false),
                    body: Box::new(FrameBody::Plan(PlanState {
                        common,
                        plan,
                        wait: PlanWait::Grid(grid_state),
                    })),
                },
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_grid_delta<'w, 's>(
    common: Box<ContainerCommon>,
    plan: &'s MeasurePlan,
    flow: Vec<StableNodeId>,
    sizes: Vec<Size>,
    solved: super::grid::Grid2DLayout,
    nodes: &mut LayoutInputMap<'w>,
    cache: &mut PassIntrinsicCache,
    scope: &'s ScopeContext<'s>,
) -> Result<FrameOutcome<'s>, UiWorldError> {
    let id = common.id;
    let (width, height) = plan.writing.physical_size(
        grid_axis_extent(&solved.col_sizes, solved.col_gap),
        grid_axis_extent(&solved.row_sizes, solved.row_gap),
    );
    let content = Size::new(width, height);
    let parent_stretches = parent_stretches_inline_auto_width(
        id,
        &common.style,
        plan.writing,
        plan.parent_direction,
        nodes.world,
    );
    let Some(size) = finish_planned_size(id, plan, nodes, |chrome| {
        let default_width = if parent_stretches {
            plan.available.width
        } else {
            content.width + chrome.width
        };
        (default_width, content.height + chrome.height)
    })?
    else {
        return Ok(start_to_frame_outcome(start_container_frame(
            common,
            nodes,
            cache,
            Some(scope),
        )?));
    };
    let mut updated = plan.clone();
    updated.size = size;
    updated.grid = Some(Box::new(GridTrackPlan::from_layout(&solved)));
    for entry in &mut updated.entries {
        if entry.intrinsic.is_some() && !flow.contains(&entry.child) {
            entry.intrinsic = PackedOptSize::NONE;
        }
    }
    for (child, measured) in flow.into_iter().zip(sizes) {
        let Ok(slot) = updated
            .entries
            .binary_search_by_key(&child, |entry| entry.child)
        else {
            continue;
        };
        updated.entries[slot].intrinsic = PackedOptSize::from_size(measured);
        if let Some(child_style) = nodes.style(child) {
            updated.entries[slot].style = Some(child_style);
        }
    }
    nodes.measure_plans.entry(id).or_default().insert(updated);
    Ok(FrameOutcome::Done(finish_plan_reused(
        &common, size, true, cache,
    )))
}
