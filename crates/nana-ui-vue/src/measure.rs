//! [`LayoutStyle`] → 可序列化布局盒（测试 / 诊断 / **预绘制回退**，非 CSS 引擎）。
//!
//! ## SoT（与 Scene 对照）
//!
//! | 数据 | 权威 | 本模块 |
//! |------|------|--------|
//! | 产品几何盒 | Scene paint → [`crate::LayoutBoxStore`] | 否 |
//! | 预绘制 / css-parity | **本模块** [`measure_layout`] | 是 |
//! | hit-test 预绘制盒 | **本模块** [`measure_layout`] | 是 |
//!
//! `layoutBox` / `getBoundingClientRect` **优先**读 Scene 盒；本模块供
//! `VueHost::resolve_layout` 在尚未 paint 时填充文档缓存，并与 css-parity 对齐。
//! `RuntimeLayoutEngine` 不写 Vue 混合树。
//!
//! 盒边 / content-box / inset / gap 解析消费 `nana-ui-core::box_layout`
//!（[`LayoutStyle::resolve_content_box`]、[`LayoutStyle::resolve_inset`]、
//! `resolved_*_against`）。新的共享几何算法优先下沉 core，而不是在此与
//! Scene host 各写一份。
//!
//! 语义对齐 Scene `row`/`column` + `LayoutStyle` 子集：gap、padding、**margin**、
//! 定宽高、`max-width`/`max-height`、`%`（相对父 content box）、
//! `flex-grow` 主轴 Fill（[`LayoutStyle::child_main_length`]）、
//! `align-items`、`justify-content`（含 [`JustifySpec::SpaceBetween`]）、
//! `flex-wrap`（Row 折行 / Column 折列；[`FlexWrap::WrapReverse`]）、
//! `grid-template-columns` / `grid-template-rows` 轻量轨（Px / fr / minmax，与 Scene 一致）、
//! `calc(P% ± Npx)` 宽度/高度、`position: relative` + inset、
//! `position: absolute` 最小子集（脱流 + 相对 nearest positioned padding box）、
//! `position: fixed` 视口子集（脱流 + 相对当前 viewport）。
//! `sticky` 仍 defer。

use crate::css_map::{
    AlignSpec, BoxSizing, FlexDirection, FlexWrap, GridTrack, JustifySpec, LayoutStyle,
    LayoutStyleCss, LengthSpec, ParentBox, resolve_grid_track_sizes,
};

/// Absolute 定位 containing block（padding box，逻辑像素）。
#[derive(Debug, Clone, Copy)]
struct ContainingBlock {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// 待测布局树节点。
#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub id: String,
    pub style: LayoutStyle,
    pub children: Vec<LayoutNode>,
}

impl LayoutNode {
    pub fn leaf(id: impl Into<String>, style: LayoutStyle) -> Self {
        Self {
            id: id.into(),
            style,
            children: Vec::new(),
        }
    }

    pub fn with_children(
        id: impl Into<String>,
        style: LayoutStyle,
        children: Vec<LayoutNode>,
    ) -> Self {
        Self {
            id: id.into(),
            style,
            children,
        }
    }
}

/// 测量结果盒（border box，逻辑像素）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasuredBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl MeasuredBox {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width: width.max(0.0),
            height: height.max(0.0),
        }
    }
}

/// 自 `root` 起量测全部可见节点；视口作为根 `%` / Fill / absolute / **fixed** 初始 CB。
pub fn measure_layout(
    root: &LayoutNode,
    viewport_w: f32,
    viewport_h: f32,
) -> Vec<(String, MeasuredBox)> {
    let vw = viewport_w.max(1.0);
    let vh = viewport_h.max(1.0);
    crate::css_map::with_active_viewport(vw, vh, || {
        crate::css_map::with_active_font_sizes(crate::css_map::FontSizeContext::default(), || {
            let mut out = Vec::new();
            let initial_cb = ContainingBlock {
                x: 0.0,
                y: 0.0,
                width: vw,
                height: vh,
            };
            measure_node(
                root, 0.0, 0.0, vw, vh, None, None, initial_cb, initial_cb, &mut out,
            );
            out
        })
    })
}

fn resolve_len(spec: LengthSpec, percent_base: Option<f32>) -> Option<f32> {
    spec.resolve_with_fonts(
        percent_base,
        crate::css_map::active_viewport(),
        crate::css_map::active_font_sizes(),
    )
    .map(|v| v.max(0.0))
}

fn measure_node(
    node: &LayoutNode,
    x: f32,
    y: f32,
    // Containing block for `%` resolution.
    avail_w: f32,
    avail_h: f32,
    // Flex parent allocated size (skip re-resolving `%`).
    definite_w: Option<f32>,
    definite_h: Option<f32>,
    // Inherited absolute CB (nearest positioned ancestor padding box).
    abs_cb: ContainingBlock,
    // Viewport CB for `position:fixed` (window / content viewport).
    viewport_cb: ContainingBlock,
    out: &mut Vec<(String, MeasuredBox)>,
) -> MeasuredBox {
    if node.style.hidden {
        return MeasuredBox::new(x, y, 0.0, 0.0);
    }

    let parent_fonts = crate::css_map::active_font_sizes();
    let fonts = crate::css_map::FontSizeContext::new(
        parent_fonts.root_px,
        node.style.font_size.unwrap_or(parent_fonts.element_px),
    );
    crate::css_map::with_active_font_sizes(fonts, || {
        measure_node_inner(
            node,
            x,
            y,
            avail_w,
            avail_h,
            definite_w,
            definite_h,
            abs_cb,
            viewport_cb,
            out,
        )
    })
}

fn measure_node_inner(
    node: &LayoutNode,
    x: f32,
    y: f32,
    // Containing block for `%` resolution.
    avail_w: f32,
    avail_h: f32,
    // Flex parent allocated size (skip re-resolving `%`).
    definite_w: Option<f32>,
    definite_h: Option<f32>,
    // Inherited absolute CB (nearest positioned ancestor padding box).
    abs_cb: ContainingBlock,
    // Viewport CB for `position:fixed` (window / content viewport).
    viewport_cb: ContainingBlock,
    out: &mut Vec<(String, MeasuredBox)>,
) -> MeasuredBox {
    let pad = node.style.resolved_padding_against(Some(avail_w));
    let bw = node.style.resolved_border_width();
    let chrome_w = pad.left + pad.right + 2.0 * bw;
    let chrome_h = pad.top + pad.bottom + 2.0 * bw;
    let content_box = matches!(node.style.box_sizing, BoxSizing::ContentBox);
    let mut width = definite_w.unwrap_or_else(|| match node.style.width {
        None | Some(LengthSpec::Fill) | Some(LengthSpec::Auto) => avail_w,
        Some(LengthSpec::Shrink) => 0.0,
        Some(other) => resolve_len(other, Some(avail_w)).unwrap_or(avail_w),
    });
    let mut height = definite_h.unwrap_or_else(|| match node.style.height {
        Some(LengthSpec::Fill) => avail_h,
        None | Some(LengthSpec::Auto) | Some(LengthSpec::Shrink) => {
            if node.style.grows() {
                avail_h
            } else {
                0.0
            }
        }
        Some(other) => resolve_len(other, Some(avail_h)).unwrap_or(0.0),
    });
    let mw = node
        .style
        .resolved_min_width(Some(avail_w), crate::css_map::active_viewport());
    width = width.max(mw);
    if let Some(mw) = node
        .style
        .resolved_max_width(Some(avail_w), crate::css_map::active_viewport())
    {
        width = width.min(mw);
    }
    let mh = node
        .style
        .resolved_min_height(Some(avail_h), crate::css_map::active_viewport());
    height = height.max(mh);
    if let Some(mh) = node
        .style
        .resolved_max_height(Some(avail_h), crate::css_map::active_viewport())
    {
        height = height.min(mh);
    }

    // content-box: declared sizes are content → border box += padding + border.
    // Parent-allocated `definite_*` are already border-box (flex adds chrome up-front).
    let content_w = if content_box
        && definite_w.is_none()
        && node
            .style
            .width
            .is_some_and(LengthSpec::is_definite_declared)
    {
        let cw = width;
        width = cw + chrome_w;
        cw
    } else {
        (width - chrome_w).max(0.0)
    };
    let mut content_h = if content_box
        && definite_h.is_none()
        && node
            .style
            .height
            .is_some_and(LengthSpec::is_definite_declared)
    {
        let ch = height;
        height = ch + chrome_h;
        ch
    } else {
        (height - chrome_h).max(0.0)
    };
    let inner_x = x + bw + pad.left;
    let inner_y = y + bw + pad.top;
    let direction = node.style.direction.unwrap_or(FlexDirection::Column);
    let gap_cb = ParentBox::new(Some(content_w), Some(content_h));
    let main_gap = node.style.main_gap_against(direction, gap_cb);
    let cross_gap = node.style.cross_gap_against(direction, gap_cb);

    let visible: Vec<&LayoutNode> = node.children.iter().filter(|c| !c.style.hidden).collect();
    let (mut in_flow, out_of_flow): (Vec<&LayoutNode>, Vec<&LayoutNode>) =
        visible.into_iter().partition(|c| !c.style.is_out_of_flow());
    let (absolute, fixed): (Vec<&LayoutNode>, Vec<&LayoutNode>) =
        out_of_flow.into_iter().partition(|c| c.style.is_absolute());
    // CSS `order`：升序；同值保留源序（`sort_by_key` 稳定）。再应用 `*-reverse`。
    in_flow.sort_by_key(|c| c.style.order);
    // `*-reverse`：反转子项序，并将 Start↔End justify 对调（对齐 CSS main-start）。
    let mut justify = node.style.justify_content;
    if node.style.flex_reverse {
        in_flow.reverse();
        justify = flip_justify_for_reverse(justify);
    }

    // Absolute descendants of in-flow children use this CB if we establish one
    // (after relative offset); otherwise inherit.
    let (rdx, rdy) = node
        .style
        .relative_offset_against(Some(avail_w), Some(avail_h));
    let child_abs_cb = if node.style.position.establishes_containing_block() {
        ContainingBlock {
            x: x + rdx + bw + pad.left,
            y: y + rdy + bw + pad.top,
            width: content_w,
            height: content_h,
        }
    } else {
        abs_cb
    };

    if !in_flow.is_empty() {
        let wrap = node.style.flex_wrap;
        let bottoms = match direction {
            FlexDirection::Row if matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse) => {
                layout_row_wrap(
                    &in_flow,
                    inner_x,
                    inner_y,
                    content_w,
                    content_h,
                    main_gap,
                    cross_gap,
                    node.style.align_items,
                    justify,
                    matches!(wrap, FlexWrap::WrapReverse),
                    node.style.active_grid_columns(),
                    child_abs_cb,
                    viewport_cb,
                    out,
                )
            }
            FlexDirection::Row => layout_row(
                &in_flow,
                inner_x,
                inner_y,
                content_w,
                content_h,
                main_gap,
                node.style.align_items,
                justify,
                node.style.active_grid_columns(),
                child_abs_cb,
                viewport_cb,
                out,
            ),
            // Column wrap needs a definite main size; auto-height (≈0) falls back.
            FlexDirection::Column
                if matches!(wrap, FlexWrap::Wrap | FlexWrap::WrapReverse) && content_h > 0.5 =>
            {
                layout_column_wrap(
                    &in_flow,
                    inner_x,
                    inner_y,
                    content_w,
                    content_h,
                    main_gap,
                    cross_gap,
                    node.style.align_items,
                    justify,
                    matches!(wrap, FlexWrap::WrapReverse),
                    node.style.active_grid_rows(),
                    child_abs_cb,
                    viewport_cb,
                    out,
                )
            }
            FlexDirection::Column => layout_column(
                &in_flow,
                inner_x,
                inner_y,
                content_w,
                content_h,
                main_gap,
                node.style.align_items,
                justify,
                node.style.active_grid_rows(),
                child_abs_cb,
                viewport_cb,
                out,
            ),
        };
        if !matches!(
            node.style.height,
            Some(LengthSpec::Fill) | Some(LengthSpec::Px(_))
        ) && node
            .style
            .height
            .and_then(|h| resolve_len(h, Some(avail_h)))
            .is_none()
        {
            let content_bottom = bottoms.into_iter().fold(inner_y, f32::max);
            height = (content_bottom - y + pad.bottom + bw).max(height).max(
                node.style
                    .resolved_min_height(Some(avail_h), crate::css_map::active_viewport()),
            );
            content_h = (height - chrome_h).max(0.0);
        }
    } else if definite_h.is_none()
        && !matches!(
            node.style.height,
            Some(LengthSpec::Fill) | Some(LengthSpec::Px(_))
        )
    {
        // Typography leaf under height:auto — letter-spacing glyph rows
        // otherwise collapse; match label_text line-box height.
        if let Some(fs) = node.style.font_size.filter(|v| *v > 0.0) {
            let line = crate::css_map::text_line_box_height_px(fs, node.style.line_height);
            height = height.max(line + chrome_h).max(
                node.style
                    .resolved_min_height(Some(avail_h), crate::css_map::active_viewport()),
            );
            content_h = (height - chrome_h).max(0.0);
        }
    }

    // Place absolute children against finalized CB (this node if positioned).
    let place_cb = if node.style.position.establishes_containing_block() {
        ContainingBlock {
            x: x + rdx + bw + pad.left,
            y: y + rdy + bw + pad.top,
            width: content_w,
            height: content_h,
        }
    } else {
        abs_cb
    };
    for child in absolute {
        place_positioned_child(child, place_cb, viewport_cb, out);
    }
    // Fixed: always against the viewport CB (transform/filter containing blocks defer).
    for child in fixed {
        place_positioned_child(child, viewport_cb, viewport_cb, out);
    }

    // Flow box drives parent advancement; reported box includes relative inset.
    let flow = MeasuredBox::new(x, y, width, height);
    let reported = if rdx != 0.0 || rdy != 0.0 {
        MeasuredBox::new(x + rdx, y + rdy, width, height)
    } else {
        flow
    };
    out.push((node.id.clone(), reported));
    flow
}

/// Minimal out-of-flow placement（`absolute` / `fixed`）：脱流；相对给定 CB。
/// 无 inset → CB 原点（静态落点）；inset 支持 px / `%`（相对 CB 宽/高）。
/// 嵌套 `fixed` 仍钉视口；嵌套 `absolute` 相对本盒 padding box。
fn place_positioned_child(
    node: &LayoutNode,
    cb: ContainingBlock,
    viewport_cb: ContainingBlock,
    out: &mut Vec<(String, MeasuredBox)>,
) {
    let left = LayoutStyle::resolve_inset(node.style.offset_left, cb.width);
    let right = LayoutStyle::resolve_inset(node.style.offset_right, cb.width);
    let top = LayoutStyle::resolve_inset(node.style.offset_top, cb.height);
    let bottom = LayoutStyle::resolve_inset(node.style.offset_bottom, cb.height);

    let mut width = node
        .style
        .width
        .and_then(|w| resolve_len(w, Some(cb.width)))
        .unwrap_or(0.0);
    if let (Some(l), Some(r)) = (left, right) {
        if node.style.width.is_none()
            || matches!(
                node.style.width,
                Some(LengthSpec::Auto) | Some(LengthSpec::Fill) | Some(LengthSpec::Shrink)
            )
        {
            width = (cb.width - l - r).max(0.0);
        }
    }
    let mw = node
        .style
        .resolved_min_width(Some(cb.width), crate::css_map::active_viewport());
    width = width.max(mw);
    if let Some(mw) = node
        .style
        .resolved_max_width(Some(cb.width), crate::css_map::active_viewport())
    {
        width = width.min(mw);
    }

    let mut height = node
        .style
        .height
        .and_then(|h| resolve_len(h, Some(cb.height)))
        .unwrap_or(0.0);
    if let (Some(t), Some(b)) = (top, bottom) {
        if node.style.height.is_none()
            || matches!(
                node.style.height,
                Some(LengthSpec::Auto) | Some(LengthSpec::Fill) | Some(LengthSpec::Shrink)
            )
        {
            height = (cb.height - t - b).max(0.0);
        }
    }
    let mh = node
        .style
        .resolved_min_height(Some(cb.height), crate::css_map::active_viewport());
    height = height.max(mh);
    if let Some(mh) = node
        .style
        .resolved_max_height(Some(cb.height), crate::css_map::active_viewport())
    {
        height = height.min(mh);
    }

    // No horizontal inset → static start of CB (not "auto" center).
    let x = if let Some(l) = left {
        cb.x + l
    } else if let Some(r) = right {
        cb.x + cb.width - r - width
    } else {
        cb.x
    };
    let y = if let Some(t) = top {
        cb.y + t
    } else if let Some(b) = bottom {
        cb.y + cb.height - b - height
    } else {
        cb.y
    };

    // Positioned box establishes CB for nested absolute descendants (content box).
    let pad = node.style.resolved_padding_against(Some(cb.width));
    let bw = node.style.resolved_border_width();
    let nested_cb = ContainingBlock {
        x: x + bw + pad.left,
        y: y + bw + pad.top,
        width: (width - pad.left - pad.right - 2.0 * bw).max(0.0),
        height: (height - pad.top - pad.bottom - 2.0 * bw).max(0.0),
    };
    for child in &node.children {
        if child.style.hidden {
            continue;
        }
        if child.style.is_fixed() {
            place_positioned_child(child, viewport_cb, viewport_cb, out);
        } else if child.style.is_absolute() {
            place_positioned_child(child, nested_cb, viewport_cb, out);
        } else {
            // In-flow children of positioned: lay out inside the positioned box.
            measure_node(
                child,
                nested_cb.x,
                nested_cb.y,
                nested_cb.width,
                nested_cb.height,
                None,
                None,
                nested_cb,
                viewport_cb,
                out,
            );
        }
    }

    out.push((node.id.clone(), MeasuredBox::new(x, y, width, height)));
}

fn layout_row(
    children: &[&LayoutNode],
    origin_x: f32,
    origin_y: f32,
    content_w: f32,
    content_h: f32,
    gap: f32,
    align: AlignSpec,
    justify: JustifySpec,
    grid_columns: Option<&[GridTrack]>,
    abs_cb: ContainingBlock,
    viewport_cb: ContainingBlock,
    out: &mut Vec<(String, MeasuredBox)>,
) -> Vec<f32> {
    layout_row_line(
        children,
        origin_x,
        origin_y,
        content_w,
        content_h,
        gap,
        align,
        justify,
        grid_columns,
        abs_cb,
        viewport_cb,
        out,
    )
}

/// Column flex-wrap：按主轴定高折列；Fill 子项独占一列（剩余高）。
/// `main_gap` = row-gap；`cross_gap` = column-gap（列间）。
fn layout_column_wrap(
    children: &[&LayoutNode],
    origin_x: f32,
    origin_y: f32,
    content_w: f32,
    content_h: f32,
    main_gap: f32,
    cross_gap: f32,
    align: AlignSpec,
    justify: JustifySpec,
    reverse_lines: bool,
    grid_rows: Option<&[GridTrack]>,
    abs_cb: ContainingBlock,
    viewport_cb: ContainingBlock,
    out: &mut Vec<(String, MeasuredBox)>,
) -> Vec<f32> {
    let mut lines: Vec<Vec<&LayoutNode>> = Vec::new();
    let mut current: Vec<&LayoutNode> = Vec::new();
    let mut line_main = 0.0f32;

    for (i, child) in children.iter().enumerate() {
        let m = child.style.resolved_margin_against(Some(content_w));
        let main = column_child_main_length(child, i, grid_rows);
        let h = resolve_child_main(main, content_h).unwrap_or(content_h);
        let outer = h + m.top + m.bottom;
        let need = if current.is_empty() {
            outer
        } else {
            line_main + main_gap + outer
        };
        if !current.is_empty() && need > content_h + 0.5 {
            lines.push(std::mem::take(&mut current));
            line_main = 0.0;
        }
        if current.is_empty() {
            line_main = outer;
        } else {
            line_main += main_gap + outer;
        }
        current.push(child);
    }
    if !current.is_empty() {
        lines.push(current);
    }

    // Original DOM start index per flex line (for grid_rows); paint order may reverse.
    let mut line_starts: Vec<usize> = Vec::with_capacity(lines.len());
    let mut acc = 0usize;
    for line in &lines {
        line_starts.push(acc);
        acc += line.len();
    }
    // Match row wrap-reverse: reverse flex-line order, then pack LTR.
    // First DOM line ends at the cross-end (right).
    if reverse_lines {
        lines.reverse();
        line_starts.reverse();
    }

    let mut bottoms = Vec::new();
    let mut cx = origin_x;
    for (line, &dom_start) in lines.iter().zip(line_starts.iter()) {
        let line_tracks = grid_rows.map(|tracks| {
            let end = (dom_start + line.len()).min(tracks.len());
            let start = dom_start.min(end);
            &tracks[start..end]
        });
        let line_bottoms = layout_column(
            line,
            cx,
            origin_y,
            content_w,
            content_h,
            main_gap,
            align,
            justify,
            line_tracks,
            abs_cb,
            viewport_cb,
            out,
        );
        bottoms.extend(line_bottoms);
        let line_cross = line
            .iter()
            .map(|child| {
                let m = child.style.resolved_margin_against(Some(content_w));
                let child_align = child.style.resolved_align_self(align);
                let cw = resolve_cross_border_size(
                    &child.style,
                    child.style.width,
                    content_w,
                    child_align,
                    Some(content_w),
                    true,
                );
                cw + m.left + m.right
            })
            .fold(0.0f32, f32::max);
        cx += line_cross + cross_gap;
    }
    bottoms
}

fn column_child_main_length(
    child: &LayoutNode,
    index: usize,
    grid_rows: Option<&[GridTrack]>,
) -> Option<LengthSpec> {
    child
        .style
        .child_main_length(FlexDirection::Column)
        .or_else(|| {
            grid_rows
                .and_then(|tracks| tracks.get(index).copied())
                .map(GridTrack::as_row_main_length)
        })
}

/// Row flex-wrap：按主轴定宽折行；Fill 子项独占一行（剩余宽）。
/// `main_gap` = column-gap；`cross_gap` = row-gap（行间）。
fn layout_row_wrap(
    children: &[&LayoutNode],
    origin_x: f32,
    origin_y: f32,
    content_w: f32,
    content_h: f32,
    main_gap: f32,
    cross_gap: f32,
    align: AlignSpec,
    justify: JustifySpec,
    reverse_lines: bool,
    grid_columns: Option<&[GridTrack]>,
    abs_cb: ContainingBlock,
    viewport_cb: ContainingBlock,
    out: &mut Vec<(String, MeasuredBox)>,
) -> Vec<f32> {
    let mut lines: Vec<Vec<&LayoutNode>> = Vec::new();
    let mut current: Vec<&LayoutNode> = Vec::new();
    let mut line_main = 0.0f32;

    for (i, child) in children.iter().enumerate() {
        let m = child.style.resolved_margin_against(Some(content_w));
        let main = row_child_main_length(child, i, grid_columns);
        let w = resolve_child_main(main, content_w).unwrap_or(content_w);
        // Match layout_row_line: outer main size includes horizontal margin.
        let outer = w + m.left + m.right;
        let need = if current.is_empty() {
            outer
        } else {
            line_main + main_gap + outer
        };
        if !current.is_empty() && need > content_w + 0.5 {
            lines.push(std::mem::take(&mut current));
            line_main = 0.0;
        }
        if current.is_empty() {
            line_main = outer;
        } else {
            line_main += main_gap + outer;
        }
        current.push(child);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if reverse_lines {
        lines.reverse();
    }

    let mut bottoms = Vec::new();
    let mut cy = origin_y;
    let mut child_index = 0usize;
    for line in &lines {
        let line_tracks = grid_columns.map(|tracks| {
            let start = child_index;
            let end = (start + line.len()).min(tracks.len());
            &tracks[start..end]
        });
        let line_bottoms = layout_row_line(
            line,
            origin_x,
            cy,
            content_w,
            content_h,
            main_gap,
            align,
            justify,
            line_tracks,
            abs_cb,
            viewport_cb,
            out,
        );
        child_index += line.len();
        let line_bottom = line_bottoms.into_iter().fold(cy, f32::max);
        bottoms.push(line_bottom);
        cy = line_bottom + cross_gap;
    }
    bottoms
}

fn row_child_main_length(
    child: &LayoutNode,
    index: usize,
    grid_columns: Option<&[GridTrack]>,
) -> Option<LengthSpec> {
    child
        .style
        .child_main_length(FlexDirection::Row)
        .or_else(|| {
            grid_columns
                .and_then(|tracks| tracks.get(index).copied())
                .map(GridTrack::as_row_main_length)
        })
}

fn layout_row_line(
    children: &[&LayoutNode],
    origin_x: f32,
    origin_y: f32,
    content_w: f32,
    content_h: f32,
    gap: f32,
    align: AlignSpec,
    justify: JustifySpec,
    grid_columns: Option<&[GridTrack]>,
    abs_cb: ContainingBlock,
    viewport_cb: ContainingBlock,
    out: &mut Vec<(String, MeasuredBox)>,
) -> Vec<f32> {
    let n = children.len();
    let gap_total = gap * n.saturating_sub(1) as f32;
    let mut margins = Vec::with_capacity(n);
    for child in children {
        margins.push(child.style.resolved_margin_against(Some(content_w)));
    }

    // Grid tracks: shared resolver (weighted fr + minmax freeze).
    let widths: Vec<f32> = if let Some(tracks) = grid_columns.filter(|t| !t.is_empty()) {
        let track_n = n.min(tracks.len());
        let margin_total: f32 = margins.iter().map(|m| m.left + m.right).sum();
        let budget = (content_w - margin_total).max(0.0);
        let auto_sizes = auto_track_contributions(
            &children[..track_n],
            &tracks[..track_n],
            content_w,
            content_h,
            false,
            abs_cb,
            viewport_cb,
        );
        let mut resolved = resolve_grid_track_sizes(&tracks[..track_n], budget, gap, &auto_sizes);
        if resolved.len() < n {
            // Extra children beyond tracks: share leftover as equal Fill.
            let used: f32 =
                resolved.iter().sum::<f32>() + gap * resolved.len().saturating_sub(1) as f32;
            let rem = (budget - used).max(0.0);
            let extra = n - resolved.len();
            let each = if extra > 0 { rem / extra as f32 } else { 0.0 };
            resolved.extend(std::iter::repeat(each).take(extra));
        }
        resolved
    } else {
        let styles: Vec<&LayoutStyle> = children.iter().map(|c| &c.style).collect();
        resolve_flex_children_main_sizes(
            &styles,
            FlexDirection::Row,
            content_w,
            Some(content_w),
            gap,
        )
    };

    let used = widths
        .iter()
        .zip(margins.iter())
        .map(|(w, m)| w + m.left + m.right)
        .sum::<f32>()
        + gap_total;
    let free = (content_w - used).max(0.0);
    let (start_x, between) = justify_main_offsets(origin_x, free, n, justify);

    let mut bottoms = Vec::new();
    let mut cx = start_x;
    for (i, ((child, &cw), m)) in children
        .iter()
        .zip(widths.iter())
        .zip(margins.iter())
        .enumerate()
    {
        let ch = resolve_cross_border_size(
            &child.style,
            child.style.height,
            content_h,
            child.style.resolved_align_self(align),
            Some(content_w),
            false,
        );
        let child_align = child.style.resolved_align_self(align);
        let cy = origin_y
            + m.top
            + match child_align {
                AlignSpec::Center => ((content_h - ch - m.top - m.bottom) * 0.5).max(0.0),
                AlignSpec::End => (content_h - ch - m.top - m.bottom).max(0.0),
                AlignSpec::Start | AlignSpec::Stretch => 0.0,
            };
        let child_h = if child.style.height.is_some() {
            ch
        } else if matches!(child_align, AlignSpec::Stretch) {
            (content_h - m.top - m.bottom).max(0.0)
        } else {
            ch.max(0.0)
        };
        let box_ = measure_node(
            child,
            cx + m.left,
            cy,
            content_w,
            content_h,
            Some(cw),
            Some(child_h.max(ch)),
            abs_cb,
            viewport_cb,
            out,
        );
        bottoms.push(box_.y + box_.height + m.bottom);
        cx += m.left + cw + m.right + gap;
        if i + 1 < n {
            cx += between;
        }
    }
    bottoms
}

fn layout_column(
    children: &[&LayoutNode],
    origin_x: f32,
    origin_y: f32,
    content_w: f32,
    content_h: f32,
    gap: f32,
    align: AlignSpec,
    justify: JustifySpec,
    grid_rows: Option<&[GridTrack]>,
    abs_cb: ContainingBlock,
    viewport_cb: ContainingBlock,
    out: &mut Vec<(String, MeasuredBox)>,
) -> Vec<f32> {
    let n = children.len();
    let gap_total = gap * n.saturating_sub(1) as f32;
    let mut margins = Vec::with_capacity(n);
    for child in children {
        margins.push(child.style.resolved_margin_against(Some(content_w)));
    }

    // Grid rows: reuse column-track resolver on the block axis (+ row-gap).
    let heights: Vec<f32> = if let Some(tracks) = grid_rows.filter(|t| !t.is_empty()) {
        let track_n = n.min(tracks.len());
        let margin_total: f32 = margins.iter().map(|m| m.top + m.bottom).sum();
        let budget = (content_h - margin_total).max(0.0);
        let auto_sizes = auto_track_contributions(
            &children[..track_n],
            &tracks[..track_n],
            content_w,
            content_h,
            true,
            abs_cb,
            viewport_cb,
        );
        let mut resolved = resolve_grid_track_sizes(&tracks[..track_n], budget, gap, &auto_sizes);
        if resolved.len() < n {
            let used: f32 =
                resolved.iter().sum::<f32>() + gap * resolved.len().saturating_sub(1) as f32;
            let rem = (budget - used).max(0.0);
            let extra = n - resolved.len();
            let each = if extra > 0 { rem / extra as f32 } else { 0.0 };
            resolved.extend(std::iter::repeat(each).take(extra));
        }
        resolved
    } else {
        let styles: Vec<&LayoutStyle> = children.iter().map(|c| &c.style).collect();
        resolve_flex_children_main_sizes(
            &styles,
            FlexDirection::Column,
            content_h,
            Some(content_w),
            gap,
        )
    };

    let used = heights
        .iter()
        .zip(margins.iter())
        .map(|(h, m)| h + m.top + m.bottom)
        .sum::<f32>()
        + gap_total;
    let free = (content_h - used).max(0.0);
    let (start_y, between) = justify_main_offsets(origin_y, free, n, justify);

    let mut bottoms = Vec::new();
    let mut cy = start_y;
    for (i, ((child, &ch), m)) in children
        .iter()
        .zip(heights.iter())
        .zip(margins.iter())
        .enumerate()
    {
        let child_align = child.style.resolved_align_self(align);
        let cw = resolve_cross_border_size(
            &child.style,
            child.style.width,
            content_w,
            child_align,
            Some(content_w),
            true,
        );
        let cx = origin_x
            + m.left
            + match child_align {
                AlignSpec::Center => ((content_w - cw - m.left - m.right) * 0.5).max(0.0),
                AlignSpec::End => (content_w - cw - m.left - m.right).max(0.0),
                AlignSpec::Start | AlignSpec::Stretch => 0.0,
            };
        let avail_w = if matches!(child.style.width, Some(LengthSpec::Fill))
            || (child.style.width.is_none() && matches!(child_align, AlignSpec::Stretch))
        {
            (content_w - m.left - m.right).max(0.0)
        } else {
            cw
        };
        let box_ = measure_node(
            child,
            cx,
            cy + m.top,
            content_w,
            content_h,
            Some(avail_w.max(cw)),
            Some(ch.max(0.0)),
            abs_cb,
            viewport_cb,
            out,
        );
        bottoms.push(box_.y + box_.height + m.bottom);
        cy += m.top + ch + m.bottom + gap;
        if i + 1 < n {
            cy += between;
        }
    }
    bottoms
}

/// Returns `(start_offset, extra_between_each_pair)`.
fn justify_main_offsets(origin: f32, free: f32, n: usize, justify: JustifySpec) -> (f32, f32) {
    if n == 0 {
        return (origin, 0.0);
    }
    match justify {
        JustifySpec::Start => (origin, 0.0),
        JustifySpec::End => (origin + free, 0.0),
        JustifySpec::Center => (origin + free * 0.5, 0.0),
        JustifySpec::SpaceBetween if n > 1 => (origin, free / (n - 1) as f32),
        JustifySpec::SpaceBetween => (origin, 0.0),
        JustifySpec::SpaceAround => {
            let each = free / n as f32;
            (origin + each * 0.5, each)
        }
        JustifySpec::SpaceEvenly => {
            let each = free / (n + 1) as f32;
            (origin + each, each)
        }
    }
}

/// `flex-direction: *-reverse` 时 Start↔End；其余分布对称，保持不变。
fn flip_justify_for_reverse(j: JustifySpec) -> JustifySpec {
    match j {
        JustifySpec::Start => JustifySpec::End,
        JustifySpec::End => JustifySpec::Start,
        other => other,
    }
}

/// Flex children main-axis sizes after grow + shrink.
///
/// Only `flex-grow>0` (`grows()`) enters the flex free-space pool. `Fill` /
/// `width|height:100%` with `flex-grow:0` resolve to definite main (`content_main`)
/// so weight-0 never collapses beside grow siblings. Default auto stays at min.
/// After shrink, remaining items keep a definite main size (T-F18/F19).
pub(crate) fn resolve_flex_children_main_sizes(
    styles: &[&LayoutStyle],
    direction: FlexDirection,
    content_main: f32,
    margin_percent_base: Option<f32>,
    gap: f32,
) -> Vec<f32> {
    let n = styles.len();
    let gap_total = gap * n.saturating_sub(1) as f32;
    let mut margin_mains = Vec::with_capacity(n);
    let mut fixed_or_fill: Vec<Option<f32>> = Vec::with_capacity(n);
    let mut mins = Vec::with_capacity(n);
    let mut maxs = Vec::with_capacity(n);
    let mut grows = Vec::with_capacity(n);
    let mut shrinks = Vec::with_capacity(n);
    for style in styles {
        let m = style.resolved_margin_against(margin_percent_base);
        let vp = crate::css_map::active_viewport();
        let (margin_main, min_main, max_main) = match direction {
            FlexDirection::Row => (
                m.left + m.right,
                style.resolved_min_width(margin_percent_base, vp),
                style.resolved_max_width(margin_percent_base, vp),
            ),
            FlexDirection::Column => (
                m.top + m.bottom,
                style.resolved_min_height(Some(content_main), vp),
                style.resolved_max_height(Some(content_main), vp),
            ),
        };
        margin_mains.push(margin_main);
        mins.push(min_main);
        maxs.push(max_main);
        // Pool weights: only `grows()` slots are flexible; others ignore this.
        grows.push(style.flex_grow.unwrap_or(0.0).max(0.0));
        // CSS initial flex-shrink is 1.
        shrinks.push(style.flex_shrink.unwrap_or(1.0).max(0.0));
        let main = style.child_main_length(direction);
        match resolve_child_main(main, content_main) {
            Some(v) => {
                let mut v = v.max(min_main);
                if let Some(max) = max_main {
                    v = v.min(max);
                }
                // content-box: flex main size is border-box (content + padding + border).
                v = content_box_main_border_size(style, direction, margin_percent_base, v);
                fixed_or_fill.push(Some(v));
            }
            None => {
                if style.grows() {
                    // flex-grow>0: share free space (T-F17 / S13 / S14).
                    fixed_or_fill.push(None);
                } else if matches!(main, Some(LengthSpec::Fill)) {
                    // Fill / 100% with grow≤0: definite main, not weight-0 pool slot.
                    // content-box: 100%/Fill is content size — expand to border-box
                    // like the Px/% branch above (do not skip chrome).
                    let mut v = content_main.max(min_main);
                    if let Some(max) = max_main {
                        v = v.min(max);
                    }
                    v = content_box_main_border_size(style, direction, margin_percent_base, v);
                    fixed_or_fill.push(Some(v));
                } else {
                    // Default auto / flex-grow:0 without Fill: stay at min.
                    fixed_or_fill.push(Some(min_main));
                }
            }
        }
    }
    let mut sizes = resolve_flex_fill_sizes(
        content_main,
        gap_total,
        &margin_mains,
        &fixed_or_fill,
        &mins,
        &maxs,
        &grows,
    );
    apply_flex_shrink(
        content_main,
        gap_total,
        &margin_mains,
        &mut sizes,
        &mins,
        &shrinks,
    );
    sizes
}

/// Weighted flex Fill main sizes (`flex-grow`) with min/max freeze + redistribute
/// (same multi-freeze idea as [`resolve_grid_column_widths`]).
///
/// `fixed_or_fill[i] = Some(px)` → fixed main; `None` → flexible Fill.
/// `grows[i]` is the flex-grow weight for flexible slots (fixed slots ignore it).
/// `margin_mains` are axis margins (horizontal for row, vertical for column).
fn resolve_flex_fill_sizes(
    content_main: f32,
    gap_total: f32,
    margin_mains: &[f32],
    fixed_or_fill: &[Option<f32>],
    mins: &[f32],
    maxs: &[Option<f32>],
    grows: &[f32],
) -> Vec<f32> {
    let n = fixed_or_fill.len();
    debug_assert_eq!(n, margin_mains.len());
    debug_assert_eq!(n, mins.len());
    debug_assert_eq!(n, maxs.len());
    debug_assert_eq!(n, grows.len());
    let mut sizes = vec![0.0f32; n];
    // (child_index, grow_weight)
    let mut active: Vec<(usize, f32)> = Vec::new();
    let mut occupied = gap_total;
    for i in 0..n {
        occupied += margin_mains[i].max(0.0);
        if let Some(w) = fixed_or_fill[i] {
            sizes[i] = w.max(0.0);
            occupied += sizes[i];
        } else {
            active.push((i, grows[i].max(0.0)));
        }
    }
    let mut free = (content_main - occupied).max(0.0);

    loop {
        if active.is_empty() {
            break;
        }
        let fr_total: f32 = active.iter().map(|(_, w)| *w).sum();
        if fr_total <= 1e-6 {
            // All grow weights 0: share equally (or freeze at min).
            let share = free / active.len() as f32;
            let mut freeze: Vec<(usize, f32)> = Vec::new();
            for (fi, &(ci, _)) in active.iter().enumerate() {
                let min = mins[ci].max(0.0);
                if share + 1e-3 < min {
                    freeze.push((fi, min));
                } else if let Some(max) = maxs[ci] {
                    if share > max + 1e-3 {
                        freeze.push((fi, max.max(0.0)));
                    }
                }
            }
            if freeze.is_empty() {
                for (ci, _) in active.drain(..) {
                    let mut w = share.max(mins[ci].max(0.0));
                    if let Some(max) = maxs[ci] {
                        w = w.min(max.max(0.0));
                    }
                    sizes[ci] = w;
                }
                break;
            }
            freeze.sort_by_key(|(fi, _)| *fi);
            for (fi, frozen_w) in freeze.into_iter().rev() {
                let (ci, _) = active.remove(fi);
                sizes[ci] = frozen_w;
                free = (free - frozen_w).max(0.0);
            }
            continue;
        }

        let mut freeze: Vec<(usize, f32)> = Vec::new();
        for (fi, &(ci, w)) in active.iter().enumerate() {
            let share = free * (w / fr_total);
            let min = mins[ci].max(0.0);
            if share + 1e-3 < min {
                freeze.push((fi, min));
            } else if let Some(max) = maxs[ci] {
                if share > max + 1e-3 {
                    freeze.push((fi, max.max(0.0)));
                }
            }
        }
        if freeze.is_empty() {
            for (ci, w) in active.drain(..) {
                let mut width = (free * (w / fr_total)).max(mins[ci].max(0.0));
                if let Some(max) = maxs[ci] {
                    width = width.min(max.max(0.0));
                }
                sizes[ci] = width;
            }
            break;
        }
        freeze.sort_by_key(|(fi, _)| *fi);
        for (fi, frozen_w) in freeze.into_iter().rev() {
            let (ci, _) = active.remove(fi);
            sizes[ci] = frozen_w;
            free = (free - frozen_w).max(0.0);
        }
    }
    sizes
}

/// CSS-like flex-shrink when main-axis used size exceeds the container.
///
/// Scaled factor is `flex-shrink * base_size`; items at `min` freeze and remaining
/// overflow is redistributed (same freeze loop idea as grow).
fn apply_flex_shrink(
    content_main: f32,
    gap_total: f32,
    margin_mains: &[f32],
    sizes: &mut [f32],
    mins: &[f32],
    shrinks: &[f32],
) {
    let n = sizes.len();
    debug_assert_eq!(n, margin_mains.len());
    debug_assert_eq!(n, mins.len());
    debug_assert_eq!(n, shrinks.len());
    // Auto-sized flex containers start with content_main≈0 before content
    // height expansion; shrinking there collapses definite children (T-S09/S11).
    if content_main <= 1e-3 {
        return;
    }
    let margin_total: f32 = margin_mains.iter().map(|m| m.max(0.0)).sum();
    let used = sizes.iter().sum::<f32>() + margin_total + gap_total;
    let mut overflow = used - content_main;
    if overflow <= 1e-3 {
        return;
    }

    // Indices still allowed to shrink.
    let mut active: Vec<usize> = (0..n)
        .filter(|&i| shrinks[i] > 1e-6 && sizes[i] > mins[i].max(0.0) + 1e-3)
        .collect();

    loop {
        if active.is_empty() || overflow <= 1e-3 {
            break;
        }
        let fr_total: f32 = active
            .iter()
            .map(|&i| shrinks[i].max(0.0) * sizes[i].max(0.0))
            .sum();
        if fr_total <= 1e-6 {
            break;
        }

        let mut freeze: Vec<(usize, f32)> = Vec::new();
        for (fi, &ci) in active.iter().enumerate() {
            let factor = shrinks[ci].max(0.0) * sizes[ci].max(0.0);
            let reduction = overflow * (factor / fr_total);
            let min = mins[ci].max(0.0);
            let proposed = sizes[ci] - reduction;
            if proposed + 1e-3 < min {
                freeze.push((fi, min));
            }
        }

        if freeze.is_empty() {
            for &ci in &active {
                let factor = shrinks[ci].max(0.0) * sizes[ci].max(0.0);
                let reduction = overflow * (factor / fr_total);
                let min = mins[ci].max(0.0);
                sizes[ci] = (sizes[ci] - reduction).max(min);
            }
            break;
        }

        freeze.sort_by_key(|(fi, _)| *fi);
        for (fi, frozen_min) in freeze.into_iter().rev() {
            let ci = active.remove(fi);
            let reduced = (sizes[ci] - frozen_min).max(0.0);
            sizes[ci] = frozen_min;
            overflow = (overflow - reduced).max(0.0);
        }
    }
}

fn resolve_child_main(spec: Option<LengthSpec>, percent_base: f32) -> Option<f32> {
    match spec {
        None | Some(LengthSpec::Fill) | Some(LengthSpec::Shrink) | Some(LengthSpec::Auto) => None,
        Some(other) => resolve_len(other, Some(percent_base)),
    }
}

/// During `auto` track intrinsic sizing, `height:100%` / `Fill` must not resolve
/// against the grid's definite size (that inflates the auto track and collapses
/// `1fr`). Treat them as content-sized for the throwaway measure pass.
fn demote_fill_main_for_intrinsic(node: &mut LayoutNode, column_main: bool) {
    let spec = if column_main {
        &mut node.style.height
    } else {
        &mut node.style.width
    };
    match *spec {
        Some(LengthSpec::Fill) => *spec = None,
        Some(LengthSpec::Percent(p)) if (p - 100.0).abs() < 0.5 => *spec = None,
        Some(LengthSpec::CalcPercentOffset { percent, offset_px })
            if (percent - 100.0).abs() < 0.5 && offset_px <= 0.0 =>
        {
            *spec = None
        }
        _ => {}
    }
    for child in &mut node.children {
        demote_fill_main_for_intrinsic(child, column_main);
    }
}

/// Content contributions for `GridTrack::Auto` (others → 0). Intrinsic measure
/// uses a throwaway out-list so the real placement pass is not polluted.
fn auto_track_contributions(
    children: &[&LayoutNode],
    tracks: &[GridTrack],
    content_w: f32,
    content_h: f32,
    column_main: bool,
    abs_cb: ContainingBlock,
    viewport_cb: ContainingBlock,
) -> Vec<f32> {
    let n = tracks.len().min(children.len());
    let mut sizes = vec![0.0f32; n];
    if !tracks.iter().any(|t| matches!(t, GridTrack::Auto)) {
        return sizes;
    }
    for i in 0..n {
        if !matches!(tracks[i], GridTrack::Auto) {
            continue;
        }
        let child = children[i];
        let percent_base = if column_main { content_h } else { content_w };
        let axis_spec = if column_main {
            child.style.height
        } else {
            child.style.width
        };
        if let Some(px) = resolve_child_main(axis_spec, percent_base) {
            sizes[i] = px.max(0.0);
            continue;
        }
        let mut demoted = child.clone();
        demote_fill_main_for_intrinsic(&mut demoted, column_main);
        let mut tmp = Vec::new();
        let (def_w, def_h) = if column_main {
            (Some(content_w), None)
        } else {
            (None, Some(content_h).filter(|h| *h > 0.0))
        };
        // Use a near-zero avail on the main axis so Fill/ residual paths cannot
        // snap to the grid's definite size after demotion.
        let (avail_w, avail_h) = if column_main {
            (content_w, 0.0)
        } else {
            (0.0, content_h)
        };
        let box_ = measure_node(
            &demoted,
            0.0,
            0.0,
            avail_w,
            avail_h,
            def_w,
            def_h,
            abs_cb,
            viewport_cb,
            &mut tmp,
        );
        sizes[i] = if column_main { box_.height } else { box_.width }.max(0.0);
    }
    sizes
}

/// Intrinsic main-axis size for one grid item under an `auto` track.
pub fn measure_grid_auto_contribution(
    node: &LayoutNode,
    content_w: f32,
    content_h: f32,
    column_main: bool,
) -> f32 {
    let cb = ContainingBlock {
        x: 0.0,
        y: 0.0,
        width: content_w.max(0.0),
        height: content_h.max(0.0),
    };
    let tracks = [GridTrack::Auto];
    let children = [node];
    auto_track_contributions(
        &children,
        &tracks,
        content_w.max(0.0),
        content_h.max(0.0),
        column_main,
        cb,
        cb,
    )
    .into_iter()
    .next()
    .unwrap_or(0.0)
}

/// content-box: flex main size is border-box = content + axis padding + border.
fn content_box_main_border_size(
    style: &LayoutStyle,
    direction: FlexDirection,
    margin_percent_base: Option<f32>,
    content_main: f32,
) -> f32 {
    if !matches!(style.box_sizing, BoxSizing::ContentBox) {
        return content_main;
    }
    let pad = style.resolved_padding_against(margin_percent_base);
    let bw = style.resolved_border_width();
    content_main
        + match direction {
            FlexDirection::Row => pad.left + pad.right + 2.0 * bw,
            FlexDirection::Column => pad.top + pad.bottom + 2.0 * bw,
        }
}

fn resolve_cross_size(spec: Option<LengthSpec>, container: f32, align: AlignSpec) -> f32 {
    match spec {
        Some(LengthSpec::Fill) => container,
        Some(other) => resolve_len(other, Some(container)).unwrap_or_else(|| {
            if matches!(align, AlignSpec::Stretch) {
                container
            } else {
                0.0
            }
        }),
        None if matches!(align, AlignSpec::Stretch) => container,
        None => 0.0,
    }
}

/// Cross-axis border-box size passed as `definite_*` into `measure_node`.
/// content-box + declared Px/%/calc → expand by axis padding + border (Fill/stretch stay as-is).
fn resolve_cross_border_size(
    style: &LayoutStyle,
    cross_spec: Option<LengthSpec>,
    container: f32,
    align: AlignSpec,
    margin_percent_base: Option<f32>,
    horizontal_cross: bool,
) -> f32 {
    let mut size = resolve_cross_size(cross_spec, container, align);
    if matches!(style.box_sizing, BoxSizing::ContentBox)
        && cross_spec.is_some_and(LengthSpec::is_definite_declared)
    {
        let pad = style.resolved_padding_against(margin_percent_base);
        let bw = style.resolved_border_width();
        size += if horizontal_cross {
            pad.left + pad.right + 2.0 * bw
        } else {
            pad.top + pad.bottom + 2.0 * bw
        };
    }
    size
}

/// 从 inline `style` + optional `class` 构建节点（公开 API 入口）。
pub fn node_from_css(
    id: impl Into<String>,
    style: &str,
    class_names: &[&str],
    percent_w: Option<f32>,
    percent_h: Option<f32>,
    children: Vec<LayoutNode>,
) -> LayoutNode {
    let mut layout = LayoutStyle::default();
    let classes: Vec<String> = class_names.iter().map(|s| (*s).to_string()).collect();
    layout.apply_class_layout_hints(&classes);
    layout.apply_css_text(style, percent_w, percent_h);
    LayoutNode::with_children(id, layout, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_map::DisplaySpec;
    use std::collections::BTreeMap;

    fn map_of(root: &LayoutNode, w: f32, h: f32) -> BTreeMap<String, MeasuredBox> {
        measure_layout(root, w, h).into_iter().collect()
    }

    #[test]
    fn typography_leaf_auto_height_uses_line_box() {
        let leaf = node_from_css(
            "h2",
            "font-size:13px;line-height:1.55;letter-spacing:0.5px",
            &[],
            None,
            None,
            vec![],
        );
        let map = map_of(&leaf, 200.0, 100.0);
        let box_ = map.get("h2").expect("leaf");
        // 13 * 1.55 = 20.15 — must not collapse to ~0 under height:auto.
        assert!(
            box_.height >= 13.0,
            "typography leaf height collapsed: {}",
            box_.height
        );
    }

    #[test]
    fn row_gap_places_children() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;gap:12px;width:400px;height:80px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:50px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b"), mk("c")]);
        let map = map_of(&root, 400.0, 80.0);
        assert!((map["b"].x - 62.0).abs() < 0.01);
        assert!((map["c"].x - 124.0).abs() < 0.01);
    }

    #[test]
    fn row_percent_gap_uses_content_width() {
        // gap:10% @ content_w 200 → 20; a@0 b@60 (T-F13)
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;gap:10%;width:200px;height:80px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:40px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b")]);
        let map = map_of(&root, 220.0, 100.0);
        assert!((map["a"].x - 0.0).abs() < 0.01);
        assert!((map["b"].x - 60.0).abs() < 0.01);
    }

    #[test]
    fn column_percent_gap_uses_content_height() {
        // gap:10% @ content_h 300 → 30; a@y0 b@y70 (T-F14)
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;gap:10%;width:200px;height:300px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:40px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b")]);
        let map = map_of(&root, 220.0, 320.0);
        assert!((map["a"].y - 0.0).abs() < 0.01);
        assert!((map["b"].y - 70.0).abs() < 0.01);
    }

    #[test]
    fn space_between_distributes_free() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;justify-content:space-between;width:400px;height:80px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:40px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b"), mk("c")]);
        let map = map_of(&root, 400.0, 80.0);
        assert!((map["a"].x - 0.0).abs() < 0.01);
        assert!((map["b"].x - 180.0).abs() < 0.01);
        assert!((map["c"].x - 360.0).abs() < 0.01);
    }

    #[test]
    fn flex1_column_splits_height() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;width:200px;height:400px",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("flex:1;width:200px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b")]);
        let map = map_of(&root, 200.0, 400.0);
        assert!((map["a"].height - 200.0).abs() < 0.01);
        assert!((map["b"].y - 200.0).abs() < 0.01);
    }

    #[test]
    fn flex_wrap_cross_gap_percent_falls_back_to_width() {
        // T-W05: gap 10% 12px; auto height → cross=20, main=12; line2 @ y60
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;flex-wrap:wrap;gap:10% 12px;width:200px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:80px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root =
            LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b"), mk("c"), mk("d")]);
        let map = map_of(&root, 200.0, 160.0);
        assert!((map["b"].x - 92.0).abs() < 0.01);
        assert!((map["c"].y - 60.0).abs() < 0.01);
        assert!((map["d"].y - 60.0).abs() < 0.01);
        assert!((map["root"].height - 100.0).abs() < 0.01);
    }

    #[test]
    fn flex_wrap_reverse_cross_gap_percent() {
        // T-W06: same packing as T-W05, lines reversed → cd @y0, ab @y60
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;flex-wrap:wrap-reverse;gap:10% 12px;width:200px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:80px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root =
            LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b"), mk("c"), mk("d")]);
        let map = map_of(&root, 200.0, 160.0);
        assert!((map["c"].y - 0.0).abs() < 0.01);
        assert!((map["d"].x - 92.0).abs() < 0.01);
        assert!((map["a"].y - 60.0).abs() < 0.01);
        assert!((map["b"].y - 60.0).abs() < 0.01);
        assert!((map["root"].height - 100.0).abs() < 0.01);
    }

    #[test]
    fn flex_wrap_row_breaks_to_next_line() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;flex-wrap:wrap;gap:8px;width:200px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:80px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root =
            LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b"), mk("c"), mk("d")]);
        let map = map_of(&root, 200.0, 160.0);
        assert!((map["a"].x - 0.0).abs() < 0.01);
        assert!((map["b"].x - 88.0).abs() < 0.01);
        assert!((map["c"].x - 0.0).abs() < 0.01);
        assert!((map["c"].y - 48.0).abs() < 0.01);
        assert!((map["d"].x - 88.0).abs() < 0.01);
        assert!((map["d"].y - 48.0).abs() < 0.01);
        assert!((map["root"].height - 88.0).abs() < 0.01);
    }

    #[test]
    fn flex_wrap_column_breaks_to_next_column() {
        // T-W07: column wrap @ height 100 → a/b @x0, c/d @x88
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;flex-wrap:wrap;gap:8px;width:200px;height:100px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:80px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root =
            LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b"), mk("c"), mk("d")]);
        let map = map_of(&root, 220.0, 120.0);
        assert!((map["a"].x - 0.0).abs() < 0.01);
        assert!((map["a"].y - 0.0).abs() < 0.01);
        assert!((map["b"].x - 0.0).abs() < 0.01);
        assert!((map["b"].y - 48.0).abs() < 0.01);
        assert!((map["c"].x - 88.0).abs() < 0.01);
        assert!((map["c"].y - 0.0).abs() < 0.01);
        assert!((map["d"].x - 88.0).abs() < 0.01);
        assert!((map["d"].y - 48.0).abs() < 0.01);
    }

    #[test]
    fn flex_wrap_column_counts_vertical_margin() {
        // T-W09: outer 72; 72+8+72>100 → second column @x88
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;flex-wrap:wrap;gap:8px;width:200px;height:100px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:80px;height:40px;margin:16px 0", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b")]);
        let map = map_of(&root, 220.0, 120.0);
        assert!((map["a"].x - 0.0).abs() < 0.01);
        assert!((map["a"].y - 16.0).abs() < 0.01);
        assert!((map["b"].x - 88.0).abs() < 0.01);
        assert!((map["b"].y - 16.0).abs() < 0.01);
    }

    #[test]
    fn flex_wrap_column_reverse_swaps_column_order() {
        // T-W08: same packing as T-W07, column order reversed → cd@x0 / ab@x88
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;flex-wrap:wrap-reverse;gap:8px;width:200px;height:100px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:80px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root =
            LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b"), mk("c"), mk("d")]);
        let map = map_of(&root, 220.0, 120.0);
        assert!((map["c"].x - 0.0).abs() < 0.01);
        assert!((map["d"].x - 0.0).abs() < 0.01);
        assert!((map["d"].y - 48.0).abs() < 0.01);
        assert!((map["a"].x - 88.0).abs() < 0.01);
        assert!((map["b"].x - 88.0).abs() < 0.01);
        assert!((map["b"].y - 48.0).abs() < 0.01);
    }

    #[test]
    fn flex_wrap_row_counts_horizontal_margin() {
        // Without margin: 80+8+80=168 ≤ 200 → same line.
        // With margin 0 16px: outer=112; 112+8+112=232 > 200 → wrap.
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;flex-wrap:wrap;gap:8px;width:200px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:80px;height:40px;margin:0 16px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b")]);
        let map = map_of(&root, 200.0, 160.0);
        assert!((map["a"].x - 16.0).abs() < 0.01);
        assert!((map["a"].y - 0.0).abs() < 0.01);
        assert!((map["b"].x - 16.0).abs() < 0.01);
        assert!((map["b"].y - 48.0).abs() < 0.01);
        assert!((map["root"].height - 88.0).abs() < 0.01);
    }

    #[test]
    fn grid_weighted_fr_splits_free_space() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:grid;grid-template-columns:100px 1fr 2fr;width:700px;height:160px;gap:0",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("height:160px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("body", root_s, vec![mk("nav"), mk("a"), mk("b")]);
        let map = map_of(&root, 700.0, 200.0);
        assert!((map["nav"].width - 100.0).abs() < 0.01);
        assert!((map["a"].width - 200.0).abs() < 0.01);
        assert!((map["b"].width - 400.0).abs() < 0.01);
        assert!((map["b"].x - 300.0).abs() < 0.01);
    }

    #[test]
    fn grid_template_rows_auto_auto_1fr_sizes_auto_to_content() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:grid;grid-template-rows:auto auto minmax(0,1fr);width:300px;height:400px;gap:10px",
            None,
            None,
        );
        let mk = |id: &str, h: f32| {
            let mut s = LayoutStyle::default();
            s.apply_css_text(&format!("width:300px;height:{h}px"), None, None);
            LayoutNode::leaf(id, s)
        };
        let mut fr = LayoutStyle::default();
        fr.apply_css_text("width:300px;height:100%", None, None);
        let root = LayoutNode::with_children(
            "page",
            root_s,
            vec![
                mk("hdr", 40.0),
                mk("mid", 60.0),
                LayoutNode::leaf("tail", fr),
            ],
        );
        let map = map_of(&root, 300.0, 400.0);
        assert!((map["hdr"].height - 40.0).abs() < 0.01);
        assert!((map["mid"].height - 60.0).abs() < 0.01);
        // 400 - 40 - 60 - 2*10 gap = 280 for the 1fr row.
        assert!(
            (map["tail"].height - 280.0).abs() < 0.01,
            "got {}",
            map["tail"].height
        );
        assert!(
            (map["tail"].y - 120.0).abs() < 0.01,
            "got {}",
            map["tail"].y
        );
    }

    #[test]
    fn grid_auto_track_ignores_nested_height_fill_against_grid() {
        // Nested height:100%/Fill must not inflate auto tracks to the grid size.
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:grid;grid-template-rows:auto minmax(0,1fr);width:300px;height:400px;gap:0",
            None,
            None,
        );
        let mut mid_s = LayoutStyle::default();
        mid_s.apply_css_text("width:300px;height:100%", None, None);
        let mut leaf_s = LayoutStyle::default();
        leaf_s.apply_css_text("width:300px;height:48px", None, None);
        let mid = LayoutNode::with_children("mid", mid_s, vec![LayoutNode::leaf("inner", leaf_s)]);
        let mut fr = LayoutStyle::default();
        fr.apply_css_text("width:300px;height:100%", None, None);
        let root =
            LayoutNode::with_children("page", root_s, vec![mid, LayoutNode::leaf("tail", fr)]);
        let map = map_of(&root, 300.0, 400.0);
        assert!(
            (map["mid"].height - 48.0).abs() < 1.0,
            "auto track must size to content, got {}",
            map["mid"].height
        );
        assert!(
            (map["tail"].height - 352.0).abs() < 1.0,
            "1fr gets remainder, got {}",
            map["tail"].height
        );
    }

    #[test]
    fn grid_multi_max_freeze_same_pass() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:grid;grid-template-columns:minmax(0,100px) minmax(0,100px) 1fr;width:400px;height:160px;gap:0",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("height:160px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("body", root_s, vec![mk("a"), mk("b"), mk("c")]);
        let map = map_of(&root, 400.0, 200.0);
        assert!((map["a"].width - 100.0).abs() < 0.01);
        assert!((map["b"].width - 100.0).abs() < 0.01);
        assert!((map["c"].width - 200.0).abs() < 0.01);
        assert!((map["c"].x - 200.0).abs() < 0.01);
    }

    #[test]
    fn grid_template_rows_uses_row_gap_from_two_value_gap() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:grid;grid-template-rows:100px 1fr 1fr;width:300px;height:400px;gap:20px 40px",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:300px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("body", root_s, vec![mk("a"), mk("b"), mk("c")]);
        let map = map_of(&root, 300.0, 420.0);
        assert!((map["a"].height - 100.0).abs() < 0.01);
        assert!((map["b"].y - 120.0).abs() < 0.01);
        assert!((map["b"].height - 130.0).abs() < 0.01);
        assert!((map["c"].y - 270.0).abs() < 0.01);
        assert!((map["c"].height - 130.0).abs() < 0.01);
    }

    #[test]
    fn grid_rows_minmax_min_and_max_freeze() {
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:300px", None, None);
            LayoutNode::leaf(id, s)
        };
        let mut min_s = LayoutStyle::default();
        min_s.apply_css_text(
            "display:grid;grid-template-rows:minmax(200px,1fr) 1fr;width:300px;height:300px;gap:0",
            None,
            None,
        );
        let min_root = LayoutNode::with_children("body", min_s, vec![mk("a"), mk("b")]);
        let min_map = map_of(&min_root, 300.0, 320.0);
        assert!((min_map["a"].height - 200.0).abs() < 0.01);
        assert!((min_map["b"].height - 100.0).abs() < 0.01);

        let mut max_s = LayoutStyle::default();
        max_s.apply_css_text(
            "display:grid;grid-template-rows:minmax(50px,120px) 1fr;width:300px;height:400px;gap:0",
            None,
            None,
        );
        let max_root = LayoutNode::with_children("body", max_s, vec![mk("a"), mk("b")]);
        let max_map = map_of(&max_root, 300.0, 420.0);
        assert!((max_map["a"].height - 120.0).abs() < 0.01);
        assert!((max_map["b"].height - 280.0).abs() < 0.01);
        assert!((max_map["b"].y - 120.0).abs() < 0.01);
    }

    #[test]
    fn percent_width_uses_parent_content_box() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;width:400px;height:100px",
            None,
            None,
        );
        let mut child_s = LayoutStyle::default();
        child_s.apply_css_text("width:50%;height:40px", None, None);
        let root =
            LayoutNode::with_children("root", root_s, vec![LayoutNode::leaf("child", child_s)]);
        let map = map_of(&root, 400.0, 100.0);
        assert!((map["child"].width - 200.0).abs() < 0.01);
    }

    #[test]
    fn border_width_shrinks_content_under_border_box() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:100px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text(
            "display:flex;flex-direction:row;width:100px;height:40px;padding:10px;border-width:5px;box-sizing:border-box",
            None,
            None,
        );
        let mut inner = LayoutStyle::default();
        inner.apply_css_text("width:100%;height:100%", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:50px;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::with_children("a", a, vec![LayoutNode::leaf("inner", inner)]),
                LayoutNode::leaf("b", b),
            ],
        );
        let map = map_of(&root, 320.0, 120.0);
        assert!((map["a"].width - 100.0).abs() < 0.01);
        assert!((map["a"].height - 40.0).abs() < 0.01);
        assert!((map["inner"].x - 15.0).abs() < 0.01);
        assert!((map["inner"].y - 15.0).abs() < 0.01);
        assert!((map["inner"].width - 70.0).abs() < 0.01);
        assert!((map["inner"].height - 10.0).abs() < 0.01);
        assert!((map["b"].x - 100.0).abs() < 0.01);
    }

    #[test]
    fn content_box_width_plus_padding_expands_border_box() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:100px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text(
            "width:100px;height:40px;padding:10px;box-sizing:content-box",
            None,
            None,
        );
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:50px;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 320.0, 120.0);
        assert!((map["a"].width - 120.0).abs() < 0.01);
        assert!((map["a"].height - 60.0).abs() < 0.01);
        assert!((map["b"].x - 120.0).abs() < 0.01);
    }

    #[test]
    fn nested_padding_percent_chain_resolves_width_percent() {
        // root pad 20 → content 360; mid pad 10%→36 → content 288; leaf 50% → 144
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;width:400px;height:200px;padding:20px;align-items:flex-start;gap:0",
            None,
            None,
        );
        let mut mid_s = LayoutStyle::default();
        mid_s.apply_css_text(
            "display:flex;flex-direction:column;width:100%;padding:10%;align-items:flex-start;gap:0",
            None,
            None,
        );
        let mut leaf_s = LayoutStyle::default();
        leaf_s.apply_css_text("width:50%;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::with_children(
                "mid",
                mid_s,
                vec![LayoutNode::leaf("leaf", leaf_s)],
            )],
        );
        let map = map_of(&root, 400.0, 220.0);
        assert!((map["mid"].x - 20.0).abs() < 0.01);
        assert!((map["mid"].width - 360.0).abs() < 0.01);
        assert!((map["leaf"].x - 56.0).abs() < 0.01);
        assert!((map["leaf"].y - 56.0).abs() < 0.01);
        assert!((map["leaf"].width - 144.0).abs() < 0.01);
        assert!((map["mid"].height - 112.0).abs() < 0.01);
    }

    #[test]
    fn display_none_skips_gap_slot() {
        let mut hidden = LayoutStyle::default();
        hidden.apply_css_text("display:none;width:50px;height:40px", None, None);
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:50px;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:50px;height:40px", None, None);
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;gap:10px;width:400px;height:80px",
            None,
            None,
        );
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("a", a),
                LayoutNode::leaf("hidden", hidden),
                LayoutNode::leaf("b", b),
            ],
        );
        let map = map_of(&root, 400.0, 80.0);
        assert!(map.get("hidden").is_none());
        assert!((map["b"].x - 60.0).abs() < 0.01);
    }

    #[test]
    fn visibility_hidden_skips_like_display_none() {
        // T-V02: Nana treats visibility:hidden as layout skip (not CSS placeholder).
        let mut gone = LayoutStyle::default();
        gone.apply_css_text("visibility:hidden;width:50px;height:40px", None, None);
        assert!(gone.hidden);
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:50px;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:50px;height:40px", None, None);
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;gap:10px;width:400px;height:80px;align-items:flex-start",
            None,
            None,
        );
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("a", a),
                LayoutNode::leaf("gone", gone),
                LayoutNode::leaf("b", b),
            ],
        );
        let map = map_of(&root, 400.0, 100.0);
        assert!(map.get("gone").is_none());
        assert!((map["b"].x - 60.0).abs() < 0.01);
    }

    #[test]
    fn align_items_flex_end_packs_to_cross_end() {
        // T-F16: tall@y20 short@y80 in 100px cross size
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;gap:10px;align-items:flex-end;width:300px;height:100px",
            None,
            None,
        );
        let mut tall = LayoutStyle::default();
        tall.apply_css_text("width:40px;height:80px", None, None);
        let mut short = LayoutStyle::default();
        short.apply_css_text("width:40px;height:20px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("tall", tall),
                LayoutNode::leaf("short", short),
            ],
        );
        let map = map_of(&root, 300.0, 100.0);
        assert!((map["tall"].y - 20.0).abs() < 0.01);
        assert!((map["short"].y - 80.0).abs() < 0.01);
        assert!((map["short"].x - 50.0).abs() < 0.01);
    }

    #[test]
    fn align_self_overrides_align_items() {
        // T-F20: parent flex-start; short align-self:flex-end → y80
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;gap:10px;align-items:flex-start;width:300px;height:100px",
            None,
            None,
        );
        let mut tall = LayoutStyle::default();
        tall.apply_css_text("width:40px;height:80px", None, None);
        let mut short = LayoutStyle::default();
        short.apply_css_text("width:40px;height:20px;align-self:flex-end", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("tall", tall),
                LayoutNode::leaf("short", short),
            ],
        );
        let map = map_of(&root, 300.0, 100.0);
        assert!((map["tall"].y - 0.0).abs() < 0.01);
        assert!((map["short"].y - 80.0).abs() < 0.01);
        assert!((map["short"].x - 50.0).abs() < 0.01);
    }

    #[test]
    fn flex_direction_row_reverse_packs_from_main_end() {
        // T-F21: a at right (x160), b at x110
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row-reverse;gap:10px;align-items:flex-start;width:200px;height:80px",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:40px;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:40px;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 200.0, 80.0);
        assert!((map["a"].x - 160.0).abs() < 0.01);
        assert!((map["b"].x - 110.0).abs() < 0.01);
        assert!(root.style.flex_reverse);
    }

    #[test]
    fn flex_order_sorts_before_source_and_reverse() {
        // T-F22: source a(order:2), b(-1), c(0) → paint b, c, a
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;gap:10px;align-items:flex-start;width:220px;height:80px",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:40px;height:40px;order:2", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:40px;height:40px;order:-1", None, None);
        let mut c = LayoutStyle::default();
        c.apply_css_text("width:40px;height:40px", None, None);
        assert_eq!(a.order, 2);
        assert_eq!(b.order, -1);
        assert_eq!(c.order, 0);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("a", a),
                LayoutNode::leaf("b", b),
                LayoutNode::leaf("c", c),
            ],
        );
        let map = map_of(&root, 220.0, 80.0);
        assert!((map["b"].x - 0.0).abs() < 0.01);
        assert!((map["c"].x - 50.0).abs() < 0.01);
        assert!((map["a"].x - 100.0).abs() < 0.01);

        // Same order keeps source order; then row-reverse flips the ordered list.
        let mut root_rev = LayoutStyle::default();
        root_rev.apply_css_text(
            "display:flex;flex-direction:row-reverse;gap:10px;width:200px;height:40px",
            None,
            None,
        );
        let mut x = LayoutStyle::default();
        x.apply_css_text("width:40px;height:40px;order:1", None, None);
        let mut y = LayoutStyle::default();
        y.apply_css_text("width:40px;height:40px;order:1", None, None);
        let rev = LayoutNode::with_children(
            "root",
            root_rev,
            vec![LayoutNode::leaf("x", x), LayoutNode::leaf("y", y)],
        );
        let map = map_of(&rev, 200.0, 40.0);
        // Equal order → source [x,y] → reverse [y,x]; End pack (T-F21) → y@110 x@160.
        assert!((map["y"].x - 110.0).abs() < 0.01, "y={}", map["y"].x);
        assert!((map["x"].x - 160.0).abs() < 0.01, "x={}", map["x"].x);
    }

    #[test]
    fn margin_advances_next_sibling() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:320px;height:120px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:40px;height:30px;margin:4px 8px 6px 2px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:40px;height:30px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 320.0, 120.0);
        assert!((map["a"].x - 2.0).abs() < 0.01);
        assert!((map["a"].y - 4.0).abs() < 0.01);
        assert!((map["b"].x - 50.0).abs() < 0.01);
    }

    #[test]
    fn percent_padding_and_margin_advance_siblings() {
        // root pad 10% of 400 → 40; content 320; a margin 0 10% → 32; a@72 b@184
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:400px;height:120px;padding:10%;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:80px;height:40px;margin:0 10%", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:80px;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 400.0, 160.0);
        assert!((map["a"].x - 72.0).abs() < 0.01);
        assert!((map["a"].y - 40.0).abs() < 0.01);
        assert!((map["b"].x - 184.0).abs() < 0.01);
        assert!((map["b"].y - 40.0).abs() < 0.01);
    }

    #[test]
    fn column_percent_margin_uses_containing_block_width() {
        // Vertical margin % → width base 200 → 20; a@y20 b@y80
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;align-items:flex-start;width:200px;height:300px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:100px;height:40px;margin:10% 0", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:100px;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 200.0, 320.0);
        assert!((map["a"].y - 20.0).abs() < 0.01);
        assert!((map["b"].y - 80.0).abs() < 0.01);
    }

    #[test]
    fn absolute_static_origin_without_inset() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "position:relative;padding:10px;width:200px;height:120px",
            None,
            None,
        );
        let mut abs = LayoutStyle::default();
        abs.apply_css_text("position:absolute;width:40px;height:24px", None, None);
        let root =
            LayoutNode::with_children("root", root_s, vec![LayoutNode::leaf("static-abs", abs)]);
        let map = map_of(&root, 200.0, 120.0);
        assert!((map["static-abs"].x - 10.0).abs() < 0.01);
        assert!((map["static-abs"].y - 10.0).abs() < 0.01);
    }

    #[test]
    fn absolute_mixed_percent_px_inset_stretch() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text("position:relative;width:200px;height:100px", None, None);
        let mut mixed = LayoutStyle::default();
        mixed.apply_css_text("position:absolute;inset:10% 8px", None, None);
        let root =
            LayoutNode::with_children("root", root_s, vec![LayoutNode::leaf("mixed", mixed)]);
        let map = map_of(&root, 200.0, 100.0);
        assert!((map["mixed"].x - 8.0).abs() < 0.01);
        assert!((map["mixed"].y - 10.0).abs() < 0.01);
        assert!((map["mixed"].width - 184.0).abs() < 0.01);
        assert!((map["mixed"].height - 80.0).abs() < 0.01);
    }

    #[test]
    fn max_height_clamps_fill_column_child() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;width:200px;height:200px",
            None,
            None,
        );
        let mut tall = LayoutStyle::default();
        tall.apply_css_text("flex:1;width:80px;max-height:60px", None, None);
        let root = LayoutNode::with_children("root", root_s, vec![LayoutNode::leaf("tall", tall)]);
        let map = map_of(&root, 200.0, 200.0);
        assert!((map["tall"].height - 60.0).abs() < 0.01);
    }

    #[test]
    fn absolute_percent_inset_against_cb() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text("position:relative;width:200px;height:100px", None, None);
        let mut pct = LayoutStyle::default();
        pct.apply_css_text(
            "position:absolute;left:10%;top:20%;width:40px;height:20px",
            None,
            None,
        );
        let root = LayoutNode::with_children("root", root_s, vec![LayoutNode::leaf("pct", pct)]);
        let map = map_of(&root, 200.0, 100.0);
        assert!((map["pct"].x - 20.0).abs() < 0.01);
        assert!((map["pct"].y - 20.0).abs() < 0.01);
    }

    #[test]
    fn absolute_left_right_and_top_bottom_stretch() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text("position:relative;width:200px;height:120px", None, None);
        let mut stretch = LayoutStyle::default();
        stretch.apply_css_text(
            "position:absolute;left:16px;right:24px;top:8px;bottom:12px",
            None,
            None,
        );
        let root =
            LayoutNode::with_children("root", root_s, vec![LayoutNode::leaf("stretch", stretch)]);
        let map = map_of(&root, 200.0, 120.0);
        assert!((map["stretch"].width - 160.0).abs() < 0.01);
        assert!((map["stretch"].height - 100.0).abs() < 0.01);
        assert!((map["stretch"].x - 16.0).abs() < 0.01);
        assert!((map["stretch"].y - 8.0).abs() < 0.01);
    }

    #[test]
    fn absolute_uses_padded_containing_block() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "position:relative;padding:12px 8px;width:220px;height:140px",
            None,
            None,
        );
        let mut badge = LayoutStyle::default();
        badge.apply_css_text(
            "position:absolute;top:4px;left:6px;width:40px;height:20px",
            None,
            None,
        );
        let mut flow = LayoutStyle::default();
        flow.apply_css_text("width:50px;height:30px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("badge", badge),
                LayoutNode::leaf("flow", flow),
            ],
        );
        let map = map_of(&root, 220.0, 140.0);
        assert!((map["flow"].x - 8.0).abs() < 0.01);
        assert!((map["flow"].y - 12.0).abs() < 0.01);
        assert!((map["badge"].x - 14.0).abs() < 0.01);
        assert!((map["badge"].y - 16.0).abs() < 0.01);
    }

    #[test]
    fn absolute_out_of_flow_against_relative_cb() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "position:relative;display:flex;flex-direction:column;align-items:flex-start;width:280px;height:160px;gap:0",
            None,
            None,
        );
        let mut flow = LayoutStyle::default();
        flow.apply_css_text("width:60px;height:40px", None, None);
        let mut badge = LayoutStyle::default();
        badge.apply_css_text(
            "position:absolute;top:8px;left:100px;width:40px;height:24px",
            None,
            None,
        );
        let mut after = LayoutStyle::default();
        after.apply_css_text("width:60px;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("flow", flow),
                LayoutNode::leaf("badge", badge),
                LayoutNode::leaf("after", after),
            ],
        );
        let map = map_of(&root, 280.0, 160.0);
        assert!((map["flow"].y - 0.0).abs() < 0.01);
        assert!(
            (map["after"].y - 40.0).abs() < 0.01,
            "absolute must not occupy flow"
        );
        assert!((map["badge"].x - 100.0).abs() < 0.01);
        assert!((map["badge"].y - 8.0).abs() < 0.01);
    }

    #[test]
    fn relative_inset_offsets_reported_box() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:column;align-items:flex-start;width:280px;height:160px;gap:0",
            None,
            None,
        );
        let mut flow = LayoutStyle::default();
        flow.apply_css_text("width:60px;height:40px", None, None);
        let mut shifted = LayoutStyle::default();
        shifted.apply_css_text(
            "position:relative;top:12px;left:20px;width:60px;height:40px",
            None,
            None,
        );
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("flow", flow),
                LayoutNode::leaf("shifted", shifted),
            ],
        );
        let map = map_of(&root, 280.0, 160.0);
        assert!((map["shifted"].x - 20.0).abs() < 0.01);
        assert!((map["shifted"].y - 52.0).abs() < 0.01);
    }

    #[test]
    fn fixed_out_of_flow_against_viewport() {
        // Anonymous fixed box: leaves flow; CB = viewport (not relative ancestor).
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "position:relative;display:flex;flex-direction:column;align-items:flex-start;width:280px;height:160px;gap:0;padding:20px",
            None,
            None,
        );
        let mut flow = LayoutStyle::default();
        flow.apply_css_text("width:60px;height:40px", None, None);
        let mut pin = LayoutStyle::default();
        pin.apply_css_text(
            "position:fixed;top:8px;right:12px;width:40px;height:24px",
            None,
            None,
        );
        let mut after = LayoutStyle::default();
        after.apply_css_text("width:60px;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("flow", flow),
                LayoutNode::leaf("pin", pin),
                LayoutNode::leaf("after", after),
            ],
        );
        let map = map_of(&root, 280.0, 160.0);
        assert!(
            (map["after"].y - map["flow"].y - 40.0).abs() < 0.01,
            "fixed must not occupy flow"
        );
        // right:12 against viewport 280 → x = 280 - 12 - 40 = 228 (not relative padding).
        assert!((map["pin"].x - 228.0).abs() < 0.01);
        assert!((map["pin"].y - 8.0).abs() < 0.01);
        assert!((map["pin"].width - 40.0).abs() < 0.01);
        assert!((map["pin"].height - 24.0).abs() < 0.01);
    }

    #[test]
    fn fixed_inset_percent_against_viewport() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text("width:200px;height:100px", None, None);
        let mut pin = LayoutStyle::default();
        pin.apply_css_text(
            "position:fixed;left:10%;top:20%;width:40px;height:20px",
            None,
            None,
        );
        let root = LayoutNode::with_children("root", root_s, vec![LayoutNode::leaf("pin", pin)]);
        // Viewport larger than root — fixed % uses viewport, not root.
        let map = map_of(&root, 400.0, 300.0);
        assert!((map["pin"].x - 40.0).abs() < 0.01);
        assert!((map["pin"].y - 60.0).abs() < 0.01);
    }

    #[test]
    fn min_width_floors_percent_width() {
        // T-S12: 10% of 300 = 30 → min-width 80
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:10%;min-width:80px;height:40px", None, None);
        let root = LayoutNode::with_children("root", root_s, vec![LayoutNode::leaf("a", a)]);
        let map = map_of(&root, 320.0, 100.0);
        assert!((map["a"].width - 80.0).abs() < 0.01);
    }

    #[test]
    fn justify_flex_end_packs_to_main_end() {
        // T-F15: free=310 → a@310 b@360
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;justify-content:flex-end;gap:10px;width:400px;height:80px;align-items:flex-start",
            None,
            None,
        );
        let mk = |id: &str| {
            let mut s = LayoutStyle::default();
            s.apply_css_text("width:40px;height:40px", None, None);
            LayoutNode::leaf(id, s)
        };
        let root = LayoutNode::with_children("root", root_s, vec![mk("a"), mk("b")]);
        let map = map_of(&root, 400.0, 100.0);
        assert!((map["a"].x - 310.0).abs() < 0.01);
        assert!((map["b"].x - 360.0).abs() < 0.01);
    }

    #[test]
    fn max_width_clamps_fill_child() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px",
            None,
            None,
        );
        let mut wide = LayoutStyle::default();
        wide.apply_css_text("flex:1;max-width:120px;height:40px", None, None);
        let root = LayoutNode::with_children("root", root_s, vec![LayoutNode::leaf("wide", wide)]);
        let map = map_of(&root, 300.0, 80.0);
        assert!((map["wide"].width - 120.0).abs() < 0.01);
    }

    #[test]
    fn flex_min_width_freezes_and_redistributes() {
        // T-S13: equal Fill would be 150/150; min 200 freezes a → b gets 100
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("flex:1;min-width:200px;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("flex:1;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 320.0, 100.0);
        assert!((map["a"].width - 200.0).abs() < 0.01);
        assert!((map["b"].width - 100.0).abs() < 0.01);
        assert!((map["b"].x - 200.0).abs() < 0.01);
    }

    #[test]
    fn flex_max_width_freezes_and_redistributes() {
        // T-S14: equal Fill 150/150; max 100 freezes a → b gets 200
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("flex:1;max-width:100px;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("flex:1;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 320.0, 100.0);
        assert!((map["a"].width - 100.0).abs() < 0.01);
        assert!((map["b"].width - 200.0).abs() < 0.01);
        assert!((map["b"].x - 100.0).abs() < 0.01);
    }

    #[test]
    fn flex_basis_sidebar_without_width() {
        // T-L04: flex:0 0 220px (no width) + flex:1 @800 → 220+580
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;width:800px;height:400px;align-items:stretch;gap:0",
            None,
            None,
        );
        let mut side = LayoutStyle::default();
        side.apply_css_text("flex:0 0 220px;height:400px", None, None);
        let mut primary = LayoutStyle::default();
        primary.apply_css_text("flex:1;height:400px", None, None);
        assert!(side.width.is_none());
        assert_eq!(side.flex_basis, Some(LengthSpec::Px(220.0)));
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("sidebar", side),
                LayoutNode::leaf("primary", primary),
            ],
        );
        let map = map_of(&root, 800.0, 400.0);
        assert!((map["sidebar"].width - 220.0).abs() < 0.01);
        assert!((map["primary"].width - 580.0).abs() < 0.01);
        assert!((map["primary"].x - 220.0).abs() < 0.01);
    }

    #[test]
    fn flex_grow_weights_free_space() {
        // T-F17: flex:1 + flex:2 @300 → 100 + 200
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("flex:1;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("flex:2;height:40px", None, None);
        assert_eq!(a.flex_grow, Some(1.0));
        assert_eq!(b.flex_grow, Some(2.0));
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 320.0, 100.0);
        assert!((map["a"].width - 100.0).abs() < 0.01);
        assert!((map["b"].width - 200.0).abs() < 0.01);
        assert!((map["b"].x - 100.0).abs() < 0.01);
    }

    #[test]
    fn row_auto_without_grow_does_not_share_free_space() {
        // Default auto / flex-grow:0 must not enter flex-fill; flex:1 sibling takes remainder.
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("height:40px;flex-grow:0;min-width:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("flex:1;height:40px", None, None);
        assert!(!a.grows());
        assert!(a.width.is_none());
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 320.0, 100.0);
        assert!(
            (map["a"].width - 40.0).abs() < 0.01,
            "non-grow auto stays at min-width, got {}",
            map["a"].width
        );
        assert!(
            (map["b"].width - 260.0).abs() < 0.01,
            "flex:1 takes remainder, got {}",
            map["b"].width
        );
        assert!((map["b"].x - 40.0).abs() < 0.01);
    }

    #[test]
    fn fill_without_grow_does_not_collapse_beside_grow_sibling() {
        // Bugbot: flex-grow:0 + width:100%/Fill must not enter the grow pool at
        // weight 0 (would resolve to main 0 next to flex:1). Resolve Fill to
        // definite content_main instead; shrink:0 keeps the full main.
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text(
            "width:100%;flex-grow:0;flex-shrink:0;height:40px",
            None,
            None,
        );
        let mut b = LayoutStyle::default();
        b.apply_css_text("flex:1;height:40px;min-width:0", None, None);
        assert!(!a.grows());
        assert_eq!(a.width, Some(LengthSpec::Fill));
        assert!(b.grows());
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 320.0, 100.0);
        assert!(
            (map["a"].width - 300.0).abs() < 0.01,
            "Fill+grow:0 must keep content_main, got {}",
            map["a"].width
        );
        assert!(
            map["b"].width.abs() < 0.01,
            "flex:1 with no free space after definite Fill stays at 0, got {}",
            map["b"].width
        );
    }

    #[test]
    fn fill_without_grow_content_box_adds_padding_like_px() {
        // Bugbot: grow:0 + Fill/100% must expand content-box chrome the same way
        // as the definite Px branch (flex main is border-box).
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:400px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text(
            "width:100%;flex-grow:0;flex-shrink:0;height:40px;padding:10px;box-sizing:content-box",
            None,
            None,
        );
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:50px;flex-shrink:0;height:40px", None, None);
        assert!(!a.grows());
        assert_eq!(a.width, Some(LengthSpec::Fill));
        let styles = [&a, &b];
        let mains =
            resolve_flex_children_main_sizes(&styles, FlexDirection::Row, 400.0, Some(400.0), 0.0);
        assert!(
            (mains[0] - 420.0).abs() < 0.01,
            "content-box Fill main = content_main 400 + 20pad, got {}",
            mains[0]
        );
        assert!((mains[1] - 50.0).abs() < 0.01);

        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 420.0, 100.0);
        assert!(
            (map["a"].width - 420.0).abs() < 0.01,
            "measured border-box must include content-box pad, got {}",
            map["a"].width
        );
        assert!((map["b"].x - 420.0).abs() < 0.01);
    }

    #[test]
    fn row_default_auto_siblings_do_not_equal_split() {
        // Two default-auto children must not be treated as equal Fill shares.
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 320.0, 100.0);
        assert!(
            map["a"].width.abs() < 0.01 && map["b"].width.abs() < 0.01,
            "default auto must not equal-split remaining width (got {} / {})",
            map["a"].width,
            map["b"].width
        );
    }

    #[test]
    fn flex_shrink_equal_overflow() {
        // T-F18: 150+150 @200 → 100+100
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:150px;flex-shrink:1;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:150px;flex-shrink:1;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 240.0, 100.0);
        assert!((map["a"].width - 100.0).abs() < 0.01);
        assert!((map["b"].width - 100.0).abs() < 0.01);
        assert!((map["b"].x - 100.0).abs() < 0.01);
    }

    #[test]
    fn flex_shrink_weighted_overflow() {
        // shrink 1 vs 2, bases 150+150 @240 → overflow 60 → 130+110
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:240px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:150px;flex-shrink:1;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:150px;flex-shrink:2;height:40px", None, None);
        assert_eq!(a.flex_shrink, Some(1.0));
        assert_eq!(b.flex_shrink, Some(2.0));
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 280.0, 100.0);
        assert!((map["a"].width - 130.0).abs() < 0.01);
        assert!((map["b"].width - 110.0).abs() < 0.01);
        assert!((map["b"].x - 130.0).abs() < 0.01);
    }

    #[test]
    fn flex_shrink_zero_skips_item() {
        // a shrink:0 keeps 150; b absorbs overflow 100 → 50
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:150px;flex-shrink:0;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:150px;flex-shrink:1;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 240.0, 100.0);
        assert!((map["a"].width - 150.0).abs() < 0.01);
        assert!((map["b"].width - 50.0).abs() < 0.01);
    }

    #[test]
    fn flex_shrink_min_width_freezes_and_redistributes() {
        // T-F19: equal shrink would be 100/100; min 120 freezes a → b gets 80
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text(
            "width:150px;flex-shrink:1;min-width:120px;height:40px",
            None,
            None,
        );
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:150px;flex-shrink:1;height:40px", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 240.0, 100.0);
        assert!((map["a"].width - 120.0).abs() < 0.01);
        assert!((map["b"].width - 80.0).abs() < 0.01);
        assert!((map["b"].x - 120.0).abs() < 0.01);
    }

    #[test]
    fn display_flex_ignores_grid_template_tracks() {
        // Honest CSS: grid-template-* is inert under display:flex. Competing
        // width:260 vs minmax(...,220px) must not invent a track-sized seam.
        let mut root = LayoutStyle::default();
        root.apply_css_text(
            "display:flex;flex-direction:row;width:100%;height:100%;gap:0;\
             grid-template-columns:minmax(180px, 220px) minmax(320px, 1fr);\
             grid-template-rows:minmax(0, 1fr)",
            None,
            None,
        );
        assert_eq!(root.display, Some(DisplaySpec::Flex));
        assert!(
            root.grid_columns.is_none(),
            "tracks cleared/ignored under flex"
        );
        assert!(root.active_grid_columns().is_none());

        let mut side = LayoutStyle::default();
        side.apply_css_text("width:260px;height:100%;background:#dfe3eb", None, None);
        let mut main = LayoutStyle::default();
        main.apply_css_text(
            "flex:1;width:100%;height:100%;background:#f5f6f9",
            None,
            None,
        );
        let tree = LayoutNode::with_children(
            "ws",
            root,
            vec![
                LayoutNode::leaf("side", side),
                LayoutNode::leaf("main", main),
            ],
        );
        let map = map_of(&tree, 960.0, 640.0);
        assert!((map["side"].width - 260.0).abs() < 0.01);
        assert!((map["main"].x - 260.0).abs() < 0.01);
        assert!((map["main"].width - 700.0).abs() < 0.01);
        let gap = map["main"].x - (map["side"].x + map["side"].width);
        assert!(
            gap.abs() < 0.5,
            "no workspace seam from inert grid tracks, gap={gap}"
        );
    }

    #[test]
    fn orphaned_single_child_keeps_first_track_without_collapse() {
        // Document measure's raw behavior before region_views collapse: 2 tracks
        // + 1 child → first track only (the squeeze bug). Collapse lives in
        // SemanticSnapshot::excluding_ids, not measure.
        let mut root = LayoutStyle::default();
        root.apply_css_text(
            "display:grid;grid-template-columns:220px minmax(0,1fr);width:800px;height:100px;gap:0",
            None,
            None,
        );
        let mut main = LayoutStyle::default();
        main.apply_css_text("width:100%;height:100%", None, None);
        let tree = LayoutNode::with_children("body", root, vec![LayoutNode::leaf("primary", main)]);
        let map = map_of(&tree, 800.0, 100.0);
        assert!(
            (map["primary"].width - 220.0).abs() < 0.01,
            "raw measure still uses first track; region_views must collapse"
        );
    }
}
