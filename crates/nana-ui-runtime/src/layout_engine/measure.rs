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
fn block_containing_size_affects_leaf(
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
fn sizes_own_axes(
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
        || style.active_grid_rows().is_some()
        || style.active_grid_columns().is_some()
        || style.grid_rows_repeat.is_some()
        || style.grid_columns_repeat.is_some()
        || style.grid_rows_subgrid
        || style.grid_columns_subgrid
        || style
            .grid_auto_rows
            .as_ref()
            .is_some_and(|tracks| !tracks.is_empty())
        || style
            .grid_auto_columns
            .as_ref()
            .is_some_and(|tracks| !tracks.is_empty())
        || style.grid_template_areas.is_some()
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
        LengthSpec::Px(_)
            | LengthSpec::Em(_)
            | LengthSpec::Rem(_)
            | LengthSpec::CalcEmOffset { .. }
            | LengthSpec::CalcRemOffset { .. }
            | LengthSpec::Viewport { .. }
            | LengthSpec::CalcViewportOffset { .. }
    )
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
            LengthSpec::Px(_)
                | LengthSpec::Em(_)
                | LengthSpec::Rem(_)
                | LengthSpec::CalcEmOffset { .. }
                | LengthSpec::CalcRemOffset { .. }
        )
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
        let mut content_w =
            if width_from_spec || stretch_fit_width || (!height_from_spec && width > 0.0) {
                Some((width - chrome.width).max(0.0))
            } else {
                None
            };
        let mut content_h = if height_from_spec {
            Some((height - chrome.height).max(0.0))
        } else {
            None
        };
        style.apply_aspect_ratio_used(&mut content_w, &mut content_h);
        if let Some(content_w) = content_w {
            width = content_w + chrome.width;
        }
        if let Some(content_h) = content_h {
            height = content_h + chrome.height;
        }
        width = width.max(min_width);
        height = height.max(min_height);
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
    let Some(style) = nodes.style(id) else {
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
    let Some(style) = nodes.style(id) else {
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
}

fn visual_content(id: StableNodeId, world: &UiWorld) -> VisualContent {
    match world.standard_visual_ref(id) {
        Some(crate::StandardVisual::Button {
            label,
            icon,
            trailing_icon,
            icon_size,
            icon_gap,
            loading,
            ..
        }) => {
            let leading = *loading || icon.is_some();
            let trailing = trailing_icon.is_some();
            // Each glyph brings its own size, and a gap to whatever it stands
            // beside: the label, or the other glyph when there is no label.
            let glyphs = usize::from(leading) + usize::from(trailing);
            let parts = glyphs + usize::from(!label.is_empty());
            if glyphs == 0 {
                return VisualContent::default();
            }
            VisualContent {
                width: glyphs as f32 * icon_size + (parts - 1) as f32 * icon_gap,
                height: *icon_size,
            }
        }
        Some(crate::StandardVisual::Checkbox { size, .. }) => VisualContent {
            width: size.indicator_size()
                + if world.text(id).is_some_and(|label| !label.is_empty()) {
                    size.indicator_gap()
                } else {
                    0.0
                },
            height: size.indicator_size(),
        },
        // Its text already asks for the widest option (`text_natural_width`);
        // the arrow stands beside it.
        Some(crate::StandardVisual::Select {
            fit_options: true, ..
        }) => VisualContent {
            width: crate::select::HANDLE_WIDTH,
            ..VisualContent::default()
        },
        _ => VisualContent::default(),
    }
}

/// A childless box whose own specs cannot move its border box off its text.
///
/// Grid tracks, padding, border, min/max, and aspect ratio all can. Margin
/// does not: the border box returned below never includes it. Logical edges
/// stay on the general path because their physical padding depends on the
/// writing context.
fn plain_childless_content(style: &nana_ui_core::LayoutStyle) -> bool {
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
        && style.border_top_width.is_none()
        && style.border_right_width.is_none()
        && style.border_bottom_width.is_none()
        && style.border_left_width.is_none()
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
    style: &nana_ui_core::LayoutStyle,
    text_metrics: Option<crate::TextMetrics>,
    font_px: f32,
    inline_base: f32,
    used: Size,
    nodes: &LayoutInputMap<'_>,
) -> BaselineMetrics {
    let replaced = style.paint.content_image.is_some()
        || style.paint.skipped_replaced.is_some()
        || nodes.world.custom_render(id).is_some()
        || {
            #[cfg(feature = "image-viewer")]
            {
                matches!(
                    nodes.world.standard_visual_ref(id),
                    Some(crate::StandardVisual::ImageViewer { .. })
                )
            }
            #[cfg(not(feature = "image-viewer"))]
            {
                false
            }
        };
    if replaced {
        return nodes.baseline_metrics(id, font_px, Some(inline_base), Some(used));
    }
    if let Some((_, layout)) = nodes.world.text_layout(id)
        && !layout.is_vertical()
        && !layout.lines.is_empty()
    {
        let first = layout.lines.first().map(|line| line.metrics.baseline_y_px);
        let last = layout.lines.last().map(|line| line.metrics.baseline_y_px);
        return BaselineMetrics { first, last };
    }
    let ascent = text_metrics
        .and_then(|metrics| metrics.ascent)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(font_px * nana_ui_core::TEXT_APPROX_ASCENT_EM);
    let first = Some(ascent);
    BaselineMetrics { first, last: first }
}

/// Whether a container whose height follows its content measures the same
/// whatever height it is offered: nothing in its subtree reads that extent
/// (`world/block_reads.rs`), and its page and containing block are
/// horizontal, so its edges resolve against width. Its key then drops the
/// offered height, and a height-only resize finds its measurement.
fn block_extent_unread(id: StableNodeId, node: &LayoutInput, world: &UiWorld) -> bool {
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
#[allow(clippy::too_many_arguments)]
fn measure_node(
    id: StableNodeId,
    forced: Option<Arc<nana_ui_core::LayoutStyle>>,
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: Option<&ScopeContext<'_>>,
) -> Result<Size, UiWorldError> {
    let unforced = forced.is_none();
    let world = nodes.world;
    let Some(node) = nodes.get(id)? else {
        return Ok(Size::default());
    };
    // A root container with children can derive its default width from the
    // containing block, unlike a child whose parent flow supplies the
    // constraint. Keep that root-vs-child dependency in the identity while
    // ordinary child facts remain reusable across formatting contexts.
    let direction_sensitive = parent_direction.is_none()
        && !node.children.is_empty()
        && !node.style.width.is_some_and(LengthSpec::is_content_sized);
    // Place remeasures a child against the used content box. That block size
    // differs from the viewport budget used while measuring, so a leaf whose
    // block specs ignore the containing block would miss and be measured
    // twice. Only the key drops the axis; the measure below still uses
    // `available`.
    let mut keyed_available = available;
    if node.children.is_empty()
        && !block_containing_size_affects_leaf(node.style.as_ref(), node.writing)
    {
        keyed_available.height = 0.0;
    }
    // A box whose size on an axis is its own -- a definite length, limits and
    // box edges that read no containing block -- measures the same whatever
    // extent it is offered on that axis: what it hands its children comes
    // from that length. A resize that reaches its parent then finds its
    // measurement instead of walking its children again.
    let (own_width, own_height) = sizes_own_axes(node.style.as_ref(), node.writing);
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
        super::viewport_basis(node.style.as_ref(), viewport),
        parent_font_px,
        node.writing,
        node.containing_writing,
        measurement_constraint_class(node.style.as_ref(), keyed_available),
        direction_sensitive,
    );
    let context = parent_direction.map(|direction| {
        crate::FormattingContextId::new(match direction {
            FlexDirection::Row => 1,
            FlexDirection::Column => 2,
        })
    });
    // Retained intrinsic facts are shared across formatting contexts. They
    // seed the pass authority for baseline and metrics consumers; the
    // containing-block-specific used size below remains a separate memo.
    if unforced
        && let Some(scope) = scope
        && !scope.affected.contains(&id)
        && let Some(metrics) = scope
            .retained
            .intrinsic_metrics
            .get(&PassIntrinsicCache::intrinsic_key(cache_key))
    {
        cache.seed_intrinsic(
            PassIntrinsicCache::intrinsic_key(cache_key),
            metrics,
            context,
        );
    }
    // The used-size memo answers this query. Intrinsic facts are read only
    // on a miss: a hit has nothing left to resolve, and the lookup existed
    // only so a counter could observe a reuse the caller did not use.
    if unforced && let Some(size) = cache.get(&cache_key) {
        cache.note_measure_cache_hit();
        return Ok(size);
    }
    // Copy the node facts out before any further world lookup. `node` borrows
    // the input map, and the style closure below is that borrow's last use.
    let style_arc = forced.unwrap_or_else(|| Arc::clone(&node.style));
    let child_ids = Arc::clone(&node.children);
    let text_metrics = node.text_metrics;
    let (writing, containing_writing) = (node.writing, node.containing_writing);
    // A full pass measures every childless text row through the container
    // path: empty flow, then the same unset padding, border, and min/max
    // specs. Those specs do not change the border box, so the text box is
    // the result. A scoped pass still takes the general path; it records a
    // measure plan this shortcut does not.
    if unforced
        && scope.is_none()
        && child_ids.is_empty()
        && plain_childless_content(style_arc.as_ref())
        && nodes.world.standard_visual_ref(id).is_none()
        && nodes.world.replaced_natural_size(id).is_none()
    {
        #[cfg(test)]
        super::measure_trace::record(id);
        return measure_plain_childless(
            id,
            cache_key,
            style_arc.as_ref(),
            text_metrics,
            writing,
            available,
            parent_font_px,
            nodes,
            cache,
        );
    }
    let cached_intrinsic = if unforced {
        cache.get_intrinsic(cache_key, context)
    } else {
        None
    };
    // A subtree outside the affected closure has no change inside it, so its
    // intrinsic size under the same constraints is unchanged.
    if unforced
        && let Some(scope) = scope
        && (!scope.measure.contains(&id)
            && !(scope.affected.contains(&id) && scope.retained.measure_plans.contains_key(&id)))
        && let Some(size) = scope
            .retained
            .intrinsics
            .get(&id)
            .and_then(|memo| memo.get(cache_key))
    {
        cache.note_measure_cache_hit();
        cache.insert(cache_key, size);
        return Ok(size);
    }
    #[cfg(test)]
    super::measure_trace::record(id);
    let text_natural_width = text_metrics.and_then(|_| nodes.world.text_natural_width(id));
    let text_wrap_limit = text_metrics.and_then(|_| nodes.world.text_wrap_limit(id));
    let style = style_arc.as_ref();
    if style.omits_box() {
        return Ok(Size::default());
    }
    let fonts = fonts_of(style, parent_font_px);
    let child_font_px = fonts.element_px;
    // Percent edges and sizes resolve against the containing block's inline
    // size, in the containing block's writing mode.
    let edge_base = containing_writing.inline_size(available.width, available.height);
    let padding = style.resolved_padding_against_fonts(Some(edge_base), fonts);
    let border = style.resolved_border_edges();
    let chrome = Size::new(
        padding.left + padding.right + border.left + border.right,
        padding.top + padding.bottom + border.top + border.bottom,
    );
    // Measure descendants against this node's declared content box, not its
    // parent's full budget. Percent padding still resolves against the parent.
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
    // Width limits constrain wrapping descendants before their heights are
    // measured. Applying them only to the final box leaves an auto-height
    // parent sized for wider text than its children can actually use.
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
    // A node whose own width and height both resolve from its style needs no
    // measurement of its children: the content-derived defaults below are
    // consumed only through `unwrap_or`, so they would be discarded.
    //
    // This is what made a dirty frame O(document). Layout invalidation
    // propagates to ancestors, so a single edit puts every container above it
    // in the change closure, and each one dropped its cached intrinsic and
    // re-measured all of its children -- a full sibling scan per level, to
    // arrive at a size its own style had already fixed.
    let (spec_width, spec_height) =
        resolved_size_specs(style, available, edge_base, viewport, fonts);
    if unforced && let Some(metrics) = cached_intrinsic {
        // The intrinsic entry contains natural facts only. Resolve those facts
        // through the current style/containing block to produce this query's
        // used size; no adjusted value is written back to the authority.
        let size = finish_intrinsic_size(
            style,
            fonts,
            viewport,
            available,
            edge_base,
            chrome,
            parent_direction,
            metrics.preferred.inline,
            metrics.preferred.block,
        );
        cache.insert(cache_key, size);
        return Ok(size);
    }
    if spec_width.is_some() && spec_height.is_some() {
        let size = finish_intrinsic_size(
            style,
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
        return Ok(size);
    }

    // The container is content-sized on at least one axis, so it owes a look at
    // its children. Everything it needs from them may still be unchanged; see
    // [`MeasurePlan`].
    let retained_plan = if unforced {
        scope.and_then(|scope| scope.retained.measure_plans.get(&id)?.get(available))
    } else {
        None
    };
    let had_plan = retained_plan.is_some();
    let visual = visual_content(id, nodes.world);
    // A replaced box takes its resource's natural size where nothing else
    // sizes it (Issue #263); with one axis set, the other follows the natural
    // aspect ratio below.
    let natural = nodes
        .world
        .replaced_natural_size(id)
        .filter(|natural| natural.width > 0.0 && natural.height > 0.0);
    if let Some(scope) = scope
        && let Some(plan) = retained_plan
        && plan.inputs_match(
            available,
            parent_direction,
            viewport,
            parent_font_px,
            &style_arc,
            &child_ids,
            text_metrics,
            text_natural_width,
            text_wrap_limit,
            visual,
            writing,
        )
        // An ancestor can rewrite a child's effective style without touching
        // the child (overlay hosting, an open menu surface), which would move
        // the measurement with every per-child input still comparing equal.
        && nodes.world.children_layout_style_is_local(id)
        // A box a replaced resource sizes records no plan (see `cacheable`);
        // one recorded before its natural size arrived no longer holds.
        && natural.is_none()
    {
        let reused =
            if measure_plan_children_unchanged(plan, viewport, child_font_px, nodes, cache, scope)?
            {
                Some(plan.size)
            } else if let Some(size) =
                sequential_measure_delta(id, plan, viewport, child_font_px, nodes, cache, scope)?
            {
                Some(size)
            } else if let Some(size) =
                flex_line_measure_delta(id, plan, viewport, child_font_px, nodes, cache, scope)?
            {
                Some(size)
            } else if let Some(size) =
                ifc_line_measure_delta(id, plan, viewport, child_font_px, nodes, cache, scope)?
            {
                Some(size)
            } else {
                grid_measure_delta(id, plan, viewport, child_font_px, nodes, cache, scope)?
            };
        if let Some(size) = reused {
            cache.note_measure_plan_reused();
            cache.note_measure_cache_hit();
            cache.insert(cache_key, size);
            return Ok(size);
        }
    }

    // We reached the actual child traversal. Fixed-size nodes, retained used
    // sizes, measure plans, and shared intrinsic facts all return above this
    // point and therefore do not count as full-subtree work.
    if had_plan {
        cache.note_plan_miss();
    }
    cache.note_measure_node();
    if scope.is_some() {
        cache.note_measure_cache_miss();
    }
    cache.record_full_subtree();
    let (mut flow_children, descendant_dependent_flow) =
        collect_flow_children_reporting(&child_ids, nodes, style.display)?;
    let grid_measure = uses_2d_grid(style, &flow_children, nodes);
    let ifc = !grid_measure
        && !style
            .display
            .is_some_and(|d| d.is_flex_container() || d.is_grid_container())
        && flow_children
            .iter()
            .any(|id| nodes.style(*id).is_some_and(|s| s.is_inline_level()));
    let direction = used_flow_direction(style, writing, ifc);
    let wrap = style.flex_wrap;
    let wrapping = ifc
        || match direction {
            FlexDirection::Row => matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse),
            FlexDirection::Column => {
                matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse)
                    && content_available.height > 0.5
            }
        };
    let grid_tracks = match direction {
        FlexDirection::Row => style.active_grid_columns(),
        FlexDirection::Column => style.active_grid_rows(),
    };
    // Line breaks and grid auto-placement depend on which item comes next, so
    // measure those in the order placement lays the children out; measuring
    // in document order while placing by `order` sizes the container for lines
    // it never has. A single unwrapped line sums the same in any order.
    if grid_measure || wrapping || grid_tracks.is_some_and(|tracks| !tracks.is_empty()) {
        sort_by_order(&mut flow_children, nodes);
    }
    let mut child_sizes = Vec::with_capacity(flow_children.len());
    #[cfg(feature = "benchmark")]
    let mut child_phase = (flow_children.len() > 64).then(super::plan_stats::PhaseClock::start);
    for child in &flow_children {
        // Resolving the child style is a map lookup plus an `Arc` clone, so keep
        // it behind the grid check rather than filtering it away afterwards.
        let child_available = if grid_measure {
            nodes
                .style(*child)
                .map(|child_style| {
                    grid_item_measure_available(child_style.as_ref(), content_available)
                })
                .unwrap_or(content_available)
        } else {
            content_available
        };
        cache.note_child_measured();
        child_sizes.push(intrinsic_size_scoped(
            *child,
            child_available,
            Some(direction),
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
        )?);
    }
    #[cfg(feature = "benchmark")]
    if let Some(clock) = child_phase.as_mut() {
        clock.lap(4);
    }
    let parent_box = gap_containing_block(style, content_available);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    // This node is every child's containing block.
    let child_edge_base = writing
        .logical_size(content_available.width, content_available.height)
        .0;
    let child_margin = |child, nodes: &LayoutInputMap<'_>| {
        nodes
            .style(child)
            .map(|style| {
                style.resolved_margin_against_fonts(
                    Some(child_edge_base),
                    fonts_of(&style, child_font_px),
                )
            })
            .unwrap_or_default()
    };
    // Items measured again at the main size their line gave them:
    // `(child, used main, size)`, re-checked by a measure plan.
    let mut hypothetical: Vec<(StableNodeId, f32, Size)> = Vec::new();
    let mut recorded_grid = None;
    let mut sequential_main_sum = 0.0f64;
    let children = if uses_2d_grid(style, &flow_children, nodes) {
        let grid = layout_grid_2d(
            style,
            writing,
            &flow_children,
            &child_sizes,
            content_available,
            fonts,
            None,
            viewport,
            nodes,
            cache,
            scope,
        )?;
        // Subgrid tracks belong to the parent. Re-solving them from this
        // container's template would invent a different grid.
        if !style.is_subgrid_columns() && !style.is_subgrid_rows() {
            recorded_grid = Some(GridTrackPlan::from_layout(&grid));
        }
        // Columns run along the inline axis, rows along the block one.
        let (width, height) = writing.physical_size(
            grid_axis_extent(&grid.col_sizes, grid.col_gap),
            grid_axis_extent(&grid.row_sizes, grid.row_gap),
        );
        Size::new(width, height)
    } else if let Some(tracks) = grid_tracks.filter(|tracks| !tracks.is_empty()) {
        let auto_sizes = auto_track_contributions(
            &flow_children,
            tracks,
            content_available,
            direction == FlexDirection::Column,
            viewport,
            child_font_px,
            nodes,
            cache,
            scope,
        )?;
        let budget = main_extent(content_available, direction);
        let resolved = resolve_grid_track_sizes(tracks, budget, gap, &auto_sizes);
        grid_intrinsic_size(
            direction,
            &resolved,
            &child_sizes,
            &flow_children,
            child_edge_base,
            gap,
            child_font_px,
            nodes,
        )
    } else if wrapping {
        wrap_intrinsic_size(
            direction,
            wrap,
            &flow_children,
            &child_sizes,
            content_available,
            child_edge_base,
            gap,
            cross_gap,
            grid_tracks,
            viewport,
            child_font_px,
            nodes,
        )
    } else {
        // With a definite main size the line hands out free space (or takes
        // back overflow), and an item's cross size is its content's at the
        // main size it ends up with, as placement lays it out.
        let definite_main = match direction {
            FlexDirection::Row => spec_width.is_some(),
            FlexDirection::Column => spec_height.is_some(),
        };
        let mut used_sizes = child_sizes.clone();
        if definite_main {
            let line = super::dynamic::DynamicLine::of(
                id,
                style,
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
                viewport,
                child_font_px,
                nodes,
                line,
            );
            for (index, child) in flow_children.iter().enumerate() {
                let Some(child_style) = nodes.style(*child) else {
                    continue;
                };
                // An item that gave up only its own chrome keeps its content
                // box, and so its cross size: nothing to measure again.
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
                let at_main = intrinsic_size_at_main(
                    *child,
                    main_extent(used_sizes[index], direction),
                    direction,
                    content_available,
                    viewport,
                    child_font_px,
                    nodes,
                    cache,
                    scope,
                )?;
                set_cross_extent(
                    &mut used_sizes[index],
                    direction,
                    cross_extent(at_main, direction),
                );
                hypothetical.push((*child, main_extent(used_sizes[index], direction), at_main));
            }
        }
        let gaps = gap * flow_children.len().saturating_sub(1) as f32;
        // Summed exactly: a plan that patches one child's share into the
        // total then reaches the bits this sum does. See
        // `sequential_main_term`.
        let mut main = f64::from(gaps);
        let mut cross = 0.0f32;
        for ((child, size), used) in flow_children.iter().zip(&child_sizes).zip(&used_sizes) {
            let margin = child_margin(*child, nodes);
            main += sequential_main_term(*size, margin, direction);
            cross = cross.max(cross_extent(*used, direction) + cross_margin(margin, direction));
        }
        sequential_main_sum = main;
        let main = main as f32;
        match direction {
            FlexDirection::Row => Size::new(main.max(0.0), cross),
            FlexDirection::Column => Size::new(cross, main.max(0.0)),
        }
    };
    #[cfg(feature = "benchmark")]
    if let Some(clock) = child_phase.as_mut() {
        clock.lap(5);
    }
    let text = text_metrics.unwrap_or_default();
    let text_width = text_inline_size(
        text,
        text_natural_width,
        text_wrap_limit,
        content_available.width,
    );
    let mut content = Size::new(
        children.width.max(text_width),
        children.height.max(text.height),
    );
    #[cfg(feature = "rich-text")]
    if let Some(crate::StandardVisual::NativeMarkdown { blocks, .. }) =
        nodes.world.standard_visual(id)
    {
        let geometry = nodes.world.markdown_layout(
            id,
            &blocks,
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: content_available.width,
                height: 0.0,
            },
        );
        content = Size::new(
            crate::rich_text::markdown_content_width(&blocks, &geometry),
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
    content.width += visual.width;
    content.height = content.height.max(visual.height);
    let max_content_w = content.width + chrome.width;
    let stacked_min_w = child_sizes
        .iter()
        .zip(&flow_children)
        .map(|(size, child)| {
            let margin = child_margin(*child, nodes);
            size.width + margin.left + margin.right
        })
        .fold(0.0f32, f32::max)
        + chrome.width;
    // nowrap row: min-content cannot be narrower than the packed sum.
    // wrap / column / block: min-content is the largest child (plus chrome).
    let min_content_w = if wrapping || direction.is_column() {
        stacked_min_w
    } else {
        max_content_w
    };
    let default_width = match style.width {
        Some(LengthSpec::MinContent) => min_content_w,
        Some(LengthSpec::MaxContent) | Some(LengthSpec::Shrink) => max_content_w,
        Some(LengthSpec::FitContent) => max_content_w.min(available.width).max(stacked_min_w),
        _ if parent_direction.is_none()
            && !style.width.is_some_and(LengthSpec::is_content_sized)
            && !flow_children.is_empty() =>
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
    if let Some(natural) = natural
        && style.aspect_ratio.is_none()
    {
        // One axis set and the other not: the natural aspect ratio carries
        // the set one over, content box to content box.
        let auto = |spec: Option<LengthSpec>| matches!(spec, None | Some(LengthSpec::Auto));
        match (auto(style.width), auto(style.height)) {
            (false, true) => {
                let inner = (size.width - chrome.width).max(0.0);
                size.height = inner * natural.height / natural.width + chrome.height;
            }
            (true, false) => {
                let inner = (size.height - chrome.height).max(0.0);
                size.width = inner * natural.width / natural.height + chrome.width;
            }
            _ => {}
        }
    }
    // Record only on a scoped pass. A full pass rebuilds every container in
    // the document, so recording there costs an `Arc` clone per child and a
    // sort per container across the whole tree -- 1.79 -> 2.55 ms on the
    // 5,000-node canonical layout, measured. It buys one frame: the next
    // scoped pass would have found a plan waiting. `layout_document`
    // (css-parity, `layout_style_tree`, Vue `measure_layout`) throws the map
    // away entirely, and `force_full` has just cleared the retained cache, so
    // in both cases the plans would be built for nobody.
    if unforced && scope.is_some() {
        // The plain in-flow path and a 2D grid are cacheable. One-axis grid
        // tracks stay out: `auto_track_contributions` measures children
        // against constraints other than `content_available`. A subgrid is
        // not recorded above, so it keeps measuring its own formatting context.
        let plain_main = match direction {
            FlexDirection::Column => {
                style.height.is_none() && style.min_height.is_none() && style.max_height.is_none()
            }
            FlexDirection::Row => {
                style.width.is_none() && style.min_width.is_none() && style.max_width.is_none()
            }
        };
        // A simple horizontal IFC of atomic inline children records the same
        // measure plan. Line membership is recomputed from those intrinsics.
        // Floats, unboxing, and a non-local child fall through and measure
        // this formatting context only.
        let ifc_local = ifc
            && text_metrics.is_none()
            && nodes.world.standard_visual_ref(id).is_none()
            && ifc_local_measure(style, writing, &flow_children, child_ids.as_slice(), nodes)?;
        let cacheable = (!descendant_dependent_flow || ifc_local)
            && (!grid_measure || recorded_grid.is_some())
            && (!ifc || ifc_local)
            && grid_tracks.is_none_or(|tracks| tracks.is_empty())
            && nodes.world.children_layout_style_is_local(id)
            // Plan deltas rebuild a size from the children alone, not the natural size above.
            && natural.is_none();
        let sequential = cacheable
            && recorded_grid.is_none()
            && plain_main
            && !wrapping
            && hypothetical.is_empty()
            && style.justify_content == JustifySpec::Start
            && style.aspect_ratio.is_none()
            && text_metrics.is_none()
            && nodes.world.standard_visual_ref(id).is_none()
            && flow_children.iter().all(|child| {
                nodes.style(*child).is_some_and(|child_style| {
                    child_style.flex_grow.unwrap_or(0.0) <= 0.0
                        && child_style.flex_shrink.unwrap_or(0.0) <= 0.0
                })
            });
        let recorded = cacheable.then(|| {
            // `flow_children` is a subsequence of `child_ids` -- that is what
            // `descendant_dependent_flow` being false means -- so one cursor
            // pairs each direct child with its measurement, if it has one.
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
                    MeasuredChild {
                        child,
                        style: nodes.style(child),
                        intrinsic,
                        at_main: hypothetical
                            .iter()
                            .find(|(item, _, _)| *item == child)
                            .map(|(_, main, size)| (*main, *size)),
                    }
                })
                .collect();
            entries.sort_unstable_by_key(|entry| entry.child);
            MeasurePlan {
                available,
                parent_direction,
                viewport,
                parent_font_px,
                style: Arc::clone(&style_arc),
                writing,
                children: Arc::clone(&child_ids),
                text_metrics,
                text_natural_width,
                text_wrap_limit,
                visual,
                child_available: content_available,
                child_direction: direction,
                entries,
                size,
                sequential,
                main_sum: sequential_main_sum,
                default_cross: cross_extent(Size::new(default_width, default_height), direction),
                grid: recorded_grid,
            }
        });
        if had_plan && recorded.is_some() {
            cache.note_plan_rebuilt();
        }
        let slots = nodes.measure_plans.entry(id).or_default();
        match recorded {
            Some(plan) => slots.insert(plan),
            // Leaving the entry empty retires the plans recorded while this
            // container was still on the cacheable path.
            None => slots.clear(),
        }
    }
    if unforced {
        cache.insert(cache_key, size);
        let baseline = nodes.baseline_metrics(
            id,
            fonts.element_px,
            Some(writing.inline_size(available.width, available.height)),
            Some(size),
        );
        cache.insert_intrinsic_bounds(
            cache_key,
            // A constrained query may legitimately use less than the
            // subtree's min-content width. Do not normalize that used query
            // upward when publishing facts for the same constraint key.
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

/// Re-check only the children the change closure reaches.
///
/// Everything outside the closure is unchanged by construction: a
/// layout-affecting `set_style` marks the node LAYOUT-dirty, which is what puts
/// it in the closure, and the caller has already confirmed that no
/// ancestor-derived adjustment can move a child's style without touching the
/// child.
///
/// Both halves matter. The intrinsic size alone misses a child whose margins
/// changed -- margins are part of the container's content extent but not of the
/// child's own measurement. The style alone misses a child that grew because
/// its OWN content grew, which is the case `content_growth_under_a_child_moves_
/// its_siblings` exists to hold.
fn measure_plan_children_unchanged(
    plan: &MeasurePlan,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: &ScopeContext<'_>,
) -> Result<bool, UiWorldError> {
    for affected in scope.affected.iter().copied() {
        let Some(entry) = plan.entry(affected) else {
            continue;
        };
        if !retained_style_matches(&nodes.style(entry.child), &entry.style) {
            return Ok(false);
        }
        if !scope.measure.contains(&affected) {
            continue;
        }
        // A child the flow collection dropped contributes nothing, and its
        // style just compared equal, so it is still dropped.
        if let Some(cached) = entry.intrinsic {
            let measured = intrinsic_size_scoped(
                entry.child,
                plan.child_available,
                Some(plan.child_direction),
                viewport,
                child_font_px,
                nodes,
                cache,
                Some(scope),
            )?;
            if measured != cached {
                return Ok(false);
            }
        }
        // An item the line gave another main size is as tall as its content
        // at that size, which can change while its measurement above did not.
        if let Some((main, cached)) = entry.at_main {
            let measured = intrinsic_size_at_main(
                entry.child,
                main,
                plan.child_direction,
                plan.child_available,
                viewport,
                child_font_px,
                nodes,
                cache,
                Some(scope),
            )?;
            if measured != cached {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(super) fn retained_style_matches(
    current: &Option<Arc<nana_ui_core::LayoutStyle>>,
    cached: &Option<Arc<nana_ui_core::LayoutStyle>>,
) -> bool {
    match (current, cached) {
        (None, None) => true,
        (Some(current), Some(cached)) => {
            Arc::ptr_eq(current, cached) || layout_inputs_equal(current, cached)
        }
        _ => false,
    }
}

/// Apply one measured child's main-size delta to a sequential container.
///
/// Returns `None` when the cached sum is not a safe description of the used
/// size. Placement-only siblings keep their cached intrinsic.
fn sequential_measure_delta(
    id: StableNodeId,
    plan: &MeasurePlan,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: &ScopeContext<'_>,
) -> Result<Option<Size>, UiWorldError> {
    if !plan.sequential {
        return Ok(None);
    }
    let direction = plan.child_direction;
    let edge_base = plan
        .writing
        .logical_size(plan.child_available.width, plan.child_available.height)
        .0;
    let margin_of = |style: &Option<Arc<nana_ui_core::LayoutStyle>>| {
        style.as_ref().map_or_else(Default::default, |style| {
            style.resolved_margin_against_fonts(Some(edge_base), fonts_of(style, child_font_px))
        })
    };
    let mut main_sum = plan.main_sum;
    // A container whose cross size is its own length does not read its
    // children's cross extents: a child that grew sideways changes nothing
    // the patch below cannot express.
    let (own_width, own_height) = sizes_own_axes(plan.style.as_ref(), plan.writing);
    let cross_own = match direction {
        FlexDirection::Column => own_width,
        FlexDirection::Row => own_height,
    };
    let mut patches: Vec<(StableNodeId, Size, Option<Arc<nana_ui_core::LayoutStyle>>)> = Vec::new();
    for affected in scope.affected.iter().copied() {
        let Some(entry) = plan.entry(affected) else {
            continue;
        };
        let Some(old) = entry.intrinsic else {
            continue;
        };
        let current_style = nodes.style(entry.child);
        if !scope.measure.contains(&affected) {
            if !retained_style_matches(&current_style, &entry.style) {
                return Ok(None);
            }
            continue;
        }
        let measured = intrinsic_size_scoped(
            entry.child,
            plan.child_available,
            Some(direction),
            viewport,
            child_font_px,
            nodes,
            cache,
            Some(scope),
        )?;
        let (old_margin, new_margin) = (margin_of(&entry.style), margin_of(&current_style));
        if !cross_own
            && (cross_extent(measured, direction) + cross_margin(new_margin, direction))
                != (cross_extent(old, direction) + cross_margin(old_margin, direction))
        {
            return Ok(None);
        }
        // Exact, as the full sum is: the patched total has the bits summing
        // every child again would, however many edits came before.
        main_sum += sequential_main_term(measured, new_margin, direction)
            - sequential_main_term(old, old_margin, direction);
        patches.push((entry.child, measured, current_style));
    }
    if patches.is_empty() {
        return Ok(None);
    }
    let main = (main_sum as f32).max(0.0);
    let Some(size) = finish_planned_size(id, plan, nodes, |chrome| match direction {
        FlexDirection::Column => (plan.default_cross, main + chrome.height),
        // A root with no parent flow stretches to the available width.
        FlexDirection::Row if plan.parent_direction.is_none() => {
            (plan.available.width, plan.default_cross)
        }
        FlexDirection::Row => (main + chrome.width, plan.default_cross),
    })?
    else {
        return Ok(None);
    };
    let mut updated = plan.clone();
    updated.size = size;
    updated.main_sum = main_sum;
    for (child, intrinsic, style) in patches {
        if let Ok(slot) = updated
            .entries
            .binary_search_by_key(&child, |entry| entry.child)
        {
            updated.entries[slot].intrinsic = Some(intrinsic);
            if let Some(style) = style {
                updated.entries[slot].style = Some(style);
            }
        }
    }
    nodes.measure_plans.entry(id).or_default().insert(updated);
    Ok(Some(size))
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
/// Returns `None` when line membership cannot be proved from the cached main
/// sizes. The caller then measures this flex formatting context, not the document.
fn flex_line_measure_delta(
    id: StableNodeId,
    plan: &MeasurePlan,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: &ScopeContext<'_>,
) -> Result<Option<Size>, UiWorldError> {
    let style = plan.style.as_ref();
    if plan.sequential || !flex_line_local_style(style) {
        return Ok(None);
    }
    if plan.text_metrics.is_some() || nodes.world.standard_visual_ref(id).is_some() {
        return Ok(None);
    }
    let direction = plan.child_direction;
    match direction {
        FlexDirection::Row
            if style.height.is_some()
                || style.min_height.is_some()
                || style.max_height.is_some() =>
        {
            return Ok(None);
        }
        FlexDirection::Column
            if style.width.is_some() || style.min_width.is_some() || style.max_width.is_some() =>
        {
            return Ok(None);
        }
        _ => {}
    }
    let tracks = match direction {
        FlexDirection::Row => style.active_grid_columns(),
        FlexDirection::Column => style.active_grid_rows(),
    };
    if tracks.is_some_and(|tracks| !tracks.is_empty()) {
        return Ok(None);
    }
    let mut flow = Vec::new();
    let mut sizes = Vec::new();
    for child in plan.children.iter().copied() {
        let Some(entry) = plan.entry(child) else {
            return Ok(None);
        };
        let Some(intrinsic) = entry.intrinsic else {
            continue;
        };
        let Some(child_style) = entry.style.as_deref() else {
            return Ok(None);
        };
        if child_blocks_flex_line_local(child_style) {
            return Ok(None);
        }
        let cross = match direction {
            FlexDirection::Row => child_style.height,
            FlexDirection::Column => child_style.width,
        };
        if matches!(cross, Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)) {
            return Ok(None);
        }
        flow.push(child);
        sizes.push(intrinsic);
    }
    if flow.is_empty() {
        return Ok(None);
    }
    let old_sizes = sizes.clone();
    let mut patched = false;
    for affected in scope.affected.iter().copied() {
        let Some(index) = flow.iter().position(|child| *child == affected) else {
            continue;
        };
        if !scope.measure.contains(&affected) {
            let Some(entry) = plan.entry(affected) else {
                continue;
            };
            if !retained_style_matches(&nodes.style(affected), &entry.style) {
                return Ok(None);
            }
            continue;
        }
        // Restyled beyond its cross size, it moves more than its line.
        if let Some(entry) = plan.entry(affected)
            && let Some(cached) = entry.style.as_ref()
            && !nodes.style(affected).is_some_and(|current| {
                Arc::ptr_eq(&current, cached) || super::same_but_cross(&current, cached, direction)
            })
        {
            return Ok(None);
        }
        let measured = intrinsic_size_scoped(
            affected,
            plan.child_available,
            Some(direction),
            viewport,
            child_font_px,
            nodes,
            cache,
            Some(scope),
        )?;
        if main_extent(measured, direction).to_bits()
            != main_extent(sizes[index], direction).to_bits()
        {
            return Ok(None);
        }
        if measured != sizes[index] {
            sizes[index] = measured;
            patched = true;
        }
    }
    if !patched {
        return Ok(None);
    }
    let fonts = fonts_of(style, plan.parent_font_px);
    let parent_box = gap_containing_block(style, plan.child_available);
    let gap = style.main_gap_against_fonts(direction, parent_box, fonts);
    let cross_gap = style.cross_gap_against_fonts(direction, parent_box, fonts);
    let edge_base = plan
        .writing
        .logical_size(plan.child_available.width, plan.child_available.height)
        .0;
    let old_lines = pack_wrap_lines(
        &flow,
        &old_sizes,
        direction,
        plan.child_available,
        edge_base,
        gap,
        None,
        viewport,
        child_font_px,
        nodes,
        false,
    );
    let new_lines = pack_wrap_lines(
        &flow,
        &sizes,
        direction,
        plan.child_available,
        edge_base,
        gap,
        None,
        viewport,
        child_font_px,
        nodes,
        false,
    );
    if old_lines != new_lines {
        return Ok(None);
    }
    let old_content = wrap_intrinsic_size(
        direction,
        FlexWrap::Wrap,
        &flow,
        &old_sizes,
        plan.child_available,
        edge_base,
        gap,
        cross_gap,
        None,
        viewport,
        child_font_px,
        nodes,
    );
    let new_content = wrap_intrinsic_size(
        direction,
        FlexWrap::Wrap,
        &flow,
        &sizes,
        plan.child_available,
        edge_base,
        gap,
        cross_gap,
        None,
        viewport,
        child_font_px,
        nodes,
    );
    let mut size = plan.size;
    match direction {
        FlexDirection::Row => size.height += new_content.height - old_content.height,
        FlexDirection::Column => size.width += new_content.width - old_content.width,
    }
    let mut updated = plan.clone();
    updated.size = size;
    for (child, measured) in flow.into_iter().zip(sizes) {
        let Ok(slot) = updated
            .entries
            .binary_search_by_key(&child, |entry| entry.child)
        else {
            continue;
        };
        updated.entries[slot].intrinsic = Some(measured);
        if let Some(child_style) = nodes.style(child) {
            updated.entries[slot].style = Some(child_style);
        }
    }
    nodes.measure_plans.entry(id).or_default().insert(updated);
    Ok(Some(size))
}

/// Re-solve a grid from cached contributions, measuring only items whose
/// contribution this pass can change.
///
/// Returns `None` when the cached contributions are not a complete description
/// of the grid (subgrid, a child entering flow, content-sized keywords on the
/// container). The caller then measures this grid formatting context.
fn grid_measure_delta(
    id: StableNodeId,
    plan: &MeasurePlan,
    viewport: LayoutViewport,
    child_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    cache: &mut PassIntrinsicCache,
    scope: &ScopeContext<'_>,
) -> Result<Option<Size>, UiWorldError> {
    if plan.grid.is_none() {
        return Ok(None);
    }
    let style = plan.style.as_ref();
    if style.is_subgrid_columns() || style.is_subgrid_rows() {
        return Ok(None);
    }
    if plan.text_metrics.is_some() || nodes.world.standard_visual_ref(id).is_some() {
        return Ok(None);
    }
    if style.width.is_some_and(LengthSpec::is_content_sized)
        || style.height.is_some_and(LengthSpec::is_content_sized)
    {
        return Ok(None);
    }
    let mut flow = Vec::new();
    let mut sizes = Vec::new();
    let mut patched = false;
    for child in plan.children.iter().copied() {
        let Some(child_style) = nodes.style(child) else {
            continue;
        };
        if !grid_child_in_flow(child_style.as_ref()) {
            continue;
        }
        let Some(entry) = plan.entry(child) else {
            return Ok(None);
        };
        if entry.intrinsic.is_none() && !scope.measure.contains(&child) {
            return Ok(None);
        }
        flow.push(child);
        sizes.push(entry.intrinsic.unwrap_or_default());
    }
    if flow.is_empty() {
        return Ok(None);
    }
    for index in 0..flow.len() {
        let child = flow[index];
        if !scope.affected.contains(&child) {
            continue;
        }
        let current = nodes.style(child);
        let style_same = plan
            .entry(child)
            .is_some_and(|entry| retained_style_matches(&current, &entry.style));
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
        let measured = intrinsic_size_scoped(
            child,
            available,
            Some(plan.child_direction),
            viewport,
            child_font_px,
            nodes,
            cache,
            Some(scope),
        )?;
        if measured != sizes[index] || !style_same {
            sizes[index] = measured;
            patched = true;
        }
    }
    if !patched {
        return Ok(None);
    }
    sort_ids_with_sizes(&mut flow, &mut sizes, nodes);
    let fonts = fonts_of(style, plan.parent_font_px);
    let solved = layout_grid_2d(
        style,
        plan.writing,
        &flow,
        &sizes,
        plan.child_available,
        fonts,
        None,
        viewport,
        nodes,
        cache,
        Some(scope),
    )?;
    let (width, height) = plan.writing.physical_size(
        grid_axis_extent(&solved.col_sizes, solved.col_gap),
        grid_axis_extent(&solved.row_sizes, solved.row_gap),
    );
    let content = Size::new(width, height);
    // A root with no parent flow stretches to the available width. Every other
    // container uses the track extent plus chrome, matching the full measure.
    let Some(size) = finish_planned_size(id, plan, nodes, |chrome| {
        let default_width = if plan.parent_direction.is_none() {
            plan.available.width
        } else {
            content.width + chrome.width
        };
        (default_width, content.height + chrome.height)
    })?
    else {
        return Ok(None);
    };
    let mut updated = plan.clone();
    updated.size = size;
    updated.grid = Some(GridTrackPlan::from_layout(&solved));
    for entry in &mut updated.entries {
        if entry.intrinsic.is_some() && !flow.contains(&entry.child) {
            entry.intrinsic = None;
        }
    }
    for (child, measured) in flow.into_iter().zip(sizes) {
        let Ok(slot) = updated
            .entries
            .binary_search_by_key(&child, |entry| entry.child)
        else {
            continue;
        };
        updated.entries[slot].intrinsic = Some(measured);
        if let Some(child_style) = nodes.style(child) {
            updated.entries[slot].style = Some(child_style);
        }
    }
    nodes.measure_plans.entry(id).or_default().insert(updated);
    Ok(Some(size))
}
