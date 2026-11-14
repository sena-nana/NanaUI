//! [`LayoutStyle`] → 可序列化布局盒（css-parity 与布局测试专用）。
//!
//! 产品几何盒的权威是 Runtime [`nana_ui_runtime::RuntimeLayoutEngine`] → UiScene。
//! 本模块把 **style-only adapter** [`LayoutNode`] 交给同一个
//! [`nana_ui_runtime::RuntimeLayoutEngine::layout_style_tree`]，CSS 文本经
//! `nana-ui-css` 映射成 [`LayoutStyle`]，不另写一套 measure。
//! 它不是 NanaUI 的 canonical `nana_ui_runtime::LayoutNode`，也不持有 retained
//! identity、participation、metrics 或 geometry authority。
//!
//! 盒边 / content-box / inset / gap 解析消费 `nana-ui-core::box_layout`。
//! 布局算法本身只在 Runtime 引擎里实现一次（wrap / 2D grid / auto-fill /
//! percent / calc / absolute / fixed）。

// Layout fixtures read as "default, then the properties under test".
#![allow(clippy::field_reassign_with_default)]

use nana_ui_core::DisplaySpec;
#[cfg(test)]
use nana_ui_css::css_map::AlignSpec;
use nana_ui_css::css_map::{FlexDirection, GridTrack, LayoutStyle, LayoutStyleCss, LengthSpec};
use nana_ui_runtime::{LayoutViewport, RuntimeLayoutEngine, StyleLayoutNode};

/// 预绘制/css-parity 的 style-only 适配节点。
///
/// The canonical layout view lives in `nana_ui_runtime::LayoutNode`; this
/// short-lived string-keyed tree is lowered to `StyleLayoutNode` solely for
/// the shared Runtime algorithm.
#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub id: String,
    pub style: LayoutStyle,
    pub children: Vec<LayoutNode>,
    pub text: Option<String>,
}

impl LayoutNode {
    pub fn leaf(id: impl Into<String>, style: LayoutStyle) -> Self {
        Self {
            id: id.into(),
            style,
            children: Vec::new(),
            text: None,
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
            text: None,
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

fn to_style_tree(node: &LayoutNode) -> StyleLayoutNode {
    StyleLayoutNode {
        id: node.id.clone(),
        style: node.style.clone(),
        children: node.children.iter().map(to_style_tree).collect(),
        text: node.text.clone(),
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
    nana_ui_css::css_map::with_active_viewport(vw, vh, || {
        nana_ui_css::css_map::with_active_font_sizes(
            nana_ui_css::css_map::FontSizeContext::default(),
            || {
                RuntimeLayoutEngine
                    .layout_style_tree(
                        &to_style_tree(root),
                        LayoutViewport::new(vw, vh),
                        &mut nana_ui::NanaTextShaper::default(),
                    )
                    .into_iter()
                    .map(|(id, box_)| {
                        (
                            id,
                            MeasuredBox::new(box_.x, box_.y, box_.width, box_.height),
                        )
                    })
                    .collect()
            },
        )
    })
}

/// Intrinsic main-axis size for one grid item under an `auto` track.
pub fn measure_grid_auto_contribution(
    node: &LayoutNode,
    content_w: f32,
    content_h: f32,
    column_main: bool,
) -> f32 {
    let mut parent = LayoutStyle::default();
    parent.display = Some(DisplaySpec::Grid);
    parent.width = Some(LengthSpec::Px(content_w.max(0.0)));
    parent.height = Some(LengthSpec::Px(content_h.max(0.0)));
    if column_main {
        parent.direction = Some(FlexDirection::Column);
        parent.grid_rows = Some(vec![GridTrack::Auto]);
    } else {
        parent.direction = Some(FlexDirection::Row);
        parent.grid_columns = Some(vec![GridTrack::Auto]);
    }
    let root = LayoutNode::with_children("grid", parent, vec![node.clone()]);
    let id = node.id.clone();
    measure_layout(&root, content_w.max(1.0), content_h.max(1.0))
        .into_iter()
        .find(|(got, _)| *got == id)
        .map(
            |(_, box_)| {
                if column_main { box_.height } else { box_.width }
            },
        )
        .unwrap_or(0.0)
}

/// Flex children main-axis sizes after grow + shrink (test helper onto the engine).
#[cfg(test)]
pub(crate) fn resolve_flex_children_main_sizes(
    styles: &[&LayoutStyle],
    direction: FlexDirection,
    content_main: f32,
    _margin_percent_base: Option<f32>,
    gap: f32,
) -> Vec<f32> {
    let mut parent = LayoutStyle::default();
    parent.direction = Some(direction);
    match direction {
        FlexDirection::Row => {
            parent.width = Some(LengthSpec::Px(content_main));
            parent.height = Some(LengthSpec::Px(content_main.max(1.0)));
        }
        FlexDirection::Column => {
            parent.height = Some(LengthSpec::Px(content_main));
            parent.width = Some(LengthSpec::Px(content_main.max(1.0)));
        }
    }
    parent.gap = Some(LengthSpec::Px(gap));
    parent.align_items = AlignSpec::Start;
    let children = styles
        .iter()
        .enumerate()
        .map(|(index, style)| LayoutNode::leaf(index.to_string(), (*style).clone()))
        .collect();
    let root = LayoutNode::with_children("flex", parent, children);
    let boxes: std::collections::BTreeMap<_, _> =
        measure_layout(&root, content_main.max(1.0), content_main.max(1.0))
            .into_iter()
            .collect();
    (0..styles.len())
        .map(|index| {
            let box_ = boxes.get(&index.to_string());
            match direction {
                FlexDirection::Row => box_.map(|box_| box_.width).unwrap_or(0.0),
                FlexDirection::Column => box_.map(|box_| box_.height).unwrap_or(0.0),
            }
        })
        .collect()
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
    use nana_ui_css::css_map::{DisplaySpec, VisibilitySpec, WhiteSpaceSpec};
    use std::collections::BTreeMap;

    fn map_of(root: &LayoutNode, w: f32, h: f32) -> BTreeMap<String, MeasuredBox> {
        measure_layout(root, w, h).into_iter().collect()
    }

    fn style(css: &str) -> LayoutStyle {
        let mut style = LayoutStyle::default();
        style.apply_css_text(css, None, None);
        style
    }

    fn leaf(id: &str, css: &str) -> LayoutNode {
        LayoutNode::leaf(id, style(css))
    }

    #[test]
    fn inline_important_width_measures_100px() {
        let node = node_from_css(
            "box",
            "width:100px !important;height:40px",
            &[],
            None,
            None,
            vec![],
        );
        let map = map_of(&node, 400.0, 80.0);
        let box_ = map.get("box").expect("box");
        assert!(
            (box_.width - 100.0).abs() < 0.5,
            "inline-only width:100px !important must measure 100, got {box_:?}"
        );
        assert!((box_.height - 40.0).abs() < 0.5, "height={box_:?}");
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
    fn grid_template_rows_auto_auto_1fr_sizes_auto_to_content() {
        let root_s = style(
            "display:grid;grid-template-rows:auto auto minmax(0,1fr);width:300px;height:400px;gap:10px",
        );
        let root = LayoutNode::with_children(
            "page",
            root_s,
            vec![
                leaf("hdr", "width:300px;height:40px"),
                leaf("mid", "width:300px;height:60px"),
                leaf("tail", "width:300px;height:100%"),
            ],
        );
        let map = map_of(&root, 300.0, 400.0);
        assert!((map["hdr"].height - 40.0).abs() < 0.01);
        assert!((map["mid"].height - 60.0).abs() < 0.01);
        // 400 - 40 - 60 - 2*10 gap = 280 for the 1fr row.
        assert!((map["tail"].height - 280.0).abs() < 0.01);
        assert!((map["tail"].y - 120.0).abs() < 0.01);
    }

    #[test]
    fn grid_auto_track_ignores_nested_height_fill_against_grid() {
        // Nested height:100%/Fill must not inflate auto tracks to the grid size.
        let root_s = style(
            "display:grid;grid-template-rows:auto minmax(0,1fr);width:300px;height:400px;gap:0",
        );
        let mid = LayoutNode::with_children(
            "mid",
            style("width:300px;height:100%"),
            vec![leaf("inner", "width:300px;height:48px")],
        );
        let root = LayoutNode::with_children(
            "page",
            root_s,
            vec![mid, leaf("tail", "width:300px;height:100%")],
        );
        let map = map_of(&root, 300.0, 400.0);
        assert!((map["mid"].height - 48.0).abs() < 1.0);
        assert!((map["tail"].height - 352.0).abs() < 1.0);
    }

    #[test]
    fn four_side_border_widths_shrink_content_under_border_box() {
        let root_s = style(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:100px;gap:0",
        );
        let a = style(
            "display:flex;flex-direction:row;width:100px;height:40px;padding:0;box-sizing:border-box;border-top-width:1px;border-right-width:2px;border-bottom-width:3px;border-left-width:4px",
        );
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::with_children(
                "a",
                a,
                vec![leaf("inner", "width:100%;height:100%")],
            )],
        );
        let map = map_of(&root, 320.0, 120.0);
        assert!((map["a"].width - 100.0).abs() < 0.01);
        assert!((map["a"].height - 40.0).abs() < 0.01);
        assert!((map["inner"].x - 4.0).abs() < 0.01);
        assert!((map["inner"].y - 1.0).abs() < 0.01);
        assert!((map["inner"].width - 94.0).abs() < 0.01);
        assert!((map["inner"].height - 36.0).abs() < 0.01);
    }

    #[test]
    fn visibility_hidden_occupies_layout_placeholder() {
        let gone = style("visibility:hidden;width:50px;height:40px");
        assert_eq!(gone.paint.visibility, Some(VisibilitySpec::Hidden));
        assert!(!gone.hidden);
        assert!(!gone.omits_box());
        assert!(!gone.is_paint_visible());

        let root_s = style(
            "display:flex;flex-direction:row;gap:10px;width:400px;height:80px;align-items:flex-start",
        );
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                leaf("a", "width:50px;height:40px"),
                LayoutNode::leaf("gone", gone),
                leaf("b", "width:50px;height:40px"),
            ],
        );
        let map = map_of(&root, 400.0, 100.0);
        assert!(map.contains_key("gone"));
        assert!((map["gone"].x - 60.0).abs() < 0.01);
        assert!((map["b"].x - 120.0).abs() < 0.01);
    }

    #[test]
    fn flex_order_sorts_and_breaks_ties_before_row_reverse() {
        let root_s = style(
            "display:flex;flex-direction:row;gap:10px;align-items:flex-start;width:220px;height:80px",
        );
        let a = style("width:40px;height:40px;order:2");
        let b = style("width:40px;height:40px;order:-1");
        let c = style("width:40px;height:40px");
        assert_eq!((a.order, b.order, c.order), (2, -1, 0));
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

        // Equal order keeps source order; then row-reverse flips the ordered list.
        let root_rev =
            style("display:flex;flex-direction:row-reverse;gap:10px;width:200px;height:40px");
        assert!(root_rev.flex_reverse);
        let rev = LayoutNode::with_children(
            "root",
            root_rev,
            vec![
                leaf("x", "width:40px;height:40px;order:1"),
                leaf("y", "width:40px;height:40px;order:1"),
            ],
        );
        let map = map_of(&rev, 200.0, 40.0);
        assert!((map["y"].x - 110.0).abs() < 0.01);
        assert!((map["x"].x - 160.0).abs() < 0.01);
    }

    #[test]
    fn flex_basis_sidebar_without_width() {
        let root_s = style(
            "display:flex;flex-direction:row;width:800px;height:400px;align-items:stretch;gap:0",
        );
        let side = style("flex:0 0 220px;height:400px");
        assert!(side.width.is_none());
        assert_eq!(side.flex_basis, Some(LengthSpec::Px(220.0)));
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![
                LayoutNode::leaf("sidebar", side),
                leaf("primary", "flex:1;height:400px"),
            ],
        );
        let map = map_of(&root, 800.0, 400.0);
        assert!((map["sidebar"].width - 220.0).abs() < 0.01);
        assert!((map["primary"].width - 580.0).abs() < 0.01);
        assert!((map["primary"].x - 220.0).abs() < 0.01);
    }

    #[test]
    fn row_auto_without_grow_does_not_share_free_space() {
        let root_s = style(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
        );
        let a = style("height:40px;flex-grow:0;min-width:40px");
        assert!(!a.grows());
        assert!(a.width.is_none());
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), leaf("b", "flex:1;height:40px")],
        );
        let map = map_of(&root, 320.0, 100.0);
        assert!((map["a"].width - 40.0).abs() < 0.01);
        assert!((map["b"].width - 260.0).abs() < 0.01);
        assert!((map["b"].x - 40.0).abs() < 0.01);
    }

    #[test]
    fn fill_without_grow_does_not_collapse_beside_grow_sibling() {
        let root_s = style(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
        );
        let a = style("width:100%;flex-grow:0;flex-shrink:0;height:40px");
        let b = style("flex:1;height:40px;min-width:0");
        assert!(!a.grows());
        assert_eq!(a.width, Some(LengthSpec::Fill));
        assert!(b.grows());
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 320.0, 100.0);
        assert!((map["a"].width - 300.0).abs() < 0.01);
        assert!(map["b"].width.abs() < 0.01);
    }

    #[test]
    fn fill_without_grow_content_box_adds_padding_like_px() {
        let root_s = style(
            "display:flex;flex-direction:row;align-items:flex-start;width:400px;height:80px;gap:0",
        );
        let a = style(
            "width:100%;flex-grow:0;flex-shrink:0;height:40px;padding:10px;box-sizing:content-box",
        );
        let b = style("width:50px;flex-shrink:0;height:40px");
        assert!(!a.grows());
        assert_eq!(a.width, Some(LengthSpec::Fill));
        let styles = [&a, &b];
        let mains =
            resolve_flex_children_main_sizes(&styles, FlexDirection::Row, 400.0, Some(400.0), 0.0);
        assert!((mains[0] - 420.0).abs() < 0.01);
        assert!((mains[1] - 50.0).abs() < 0.01);

        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 420.0, 100.0);
        assert!((map["a"].width - 420.0).abs() < 0.01);
        assert!((map["b"].x - 420.0).abs() < 0.01);
    }

    #[test]
    fn row_default_auto_siblings_do_not_equal_split() {
        let root_s = style(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:80px;gap:0",
        );
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![leaf("a", "height:40px"), leaf("b", "height:40px")],
        );
        let map = map_of(&root, 320.0, 100.0);
        assert!(map["a"].width.abs() < 0.01 && map["b"].width.abs() < 0.01);
    }

    #[test]
    fn flex_shrink_weighted_and_zero_modes() {
        // Weighted shrink 1 vs 2, bases 150+150 @240 -> overflow 60 -> 130+110.
        let weighted_root = LayoutNode::with_children(
            "root",
            style(
                "display:flex;flex-direction:row;align-items:flex-start;width:240px;height:80px;gap:0",
            ),
            vec![
                leaf("a", "width:150px;flex-shrink:1;height:40px"),
                leaf("b", "width:150px;flex-shrink:2;height:40px"),
            ],
        );
        let map = map_of(&weighted_root, 280.0, 100.0);
        assert!((map["a"].width - 130.0).abs() < 0.01);
        assert!((map["b"].width - 110.0).abs() < 0.01);
        assert!((map["b"].x - 130.0).abs() < 0.01);

        // shrink:0 keeps 150; b absorbs overflow 100 -> 50.
        let zero_root = LayoutNode::with_children(
            "root",
            style(
                "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
            ),
            vec![
                leaf("a", "width:150px;flex-shrink:0;height:40px"),
                leaf("b", "width:150px;flex-shrink:1;height:40px"),
            ],
        );
        let map = map_of(&zero_root, 240.0, 100.0);
        assert!((map["a"].width - 150.0).abs() < 0.01);
        assert!((map["b"].width - 50.0).abs() < 0.01);
    }

    #[test]
    fn flex_shorthand_modes_control_shrink() {
        let cases = [
            ("width:150px;height:40px", None, None, 150.0, 150.0),
            (
                "width:150px;flex:initial;height:40px",
                Some(0.0),
                Some(1.0),
                100.0,
                100.0,
            ),
            (
                "width:150px;flex:none;height:40px",
                Some(0.0),
                Some(0.0),
                150.0,
                150.0,
            ),
            (
                "width:150px;flex:auto;height:40px",
                Some(1.0),
                Some(1.0),
                100.0,
                100.0,
            ),
            (
                "flex:1 150px;height:40px",
                Some(1.0),
                Some(1.0),
                100.0,
                100.0,
            ),
        ];
        for (item_css, expect_grow, expect_shrink, expect_w, expect_bx) in cases {
            let a = style(item_css);
            let b = style(item_css);
            assert_eq!(a.flex_grow, expect_grow, "{item_css}");
            assert_eq!(a.flex_shrink, expect_shrink, "{item_css}");
            let root = LayoutNode::with_children(
                "root",
                style(
                    "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
                ),
                vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
            );
            let map = map_of(&root, 240.0, 100.0);
            assert!((map["a"].width - expect_w).abs() < 0.01, "{item_css}");
            assert!((map["b"].width - expect_w).abs() < 0.01, "{item_css}");
            assert!((map["b"].x - expect_bx).abs() < 0.01, "{item_css}");
        }
    }

    #[test]
    fn display_flex_ignores_grid_template_tracks() {
        let root = style(
            "display:flex;flex-direction:row;width:100%;height:100%;gap:0;\
             grid-template-columns:minmax(180px, 220px) minmax(320px, 1fr);\
             grid-template-rows:minmax(0, 1fr)",
        );
        assert_eq!(root.display, Some(DisplaySpec::Flex));
        assert!(root.grid_columns.is_none());
        assert!(root.active_grid_columns().is_none());

        let tree = LayoutNode::with_children(
            "ws",
            root,
            vec![
                leaf("side", "width:260px;height:100%;background:#dfe3eb"),
                leaf("main", "flex:1;width:100%;height:100%;background:#f5f6f9"),
            ],
        );
        let map = map_of(&tree, 960.0, 640.0);
        assert!((map["side"].width - 260.0).abs() < 0.01);
        assert!((map["main"].x - 260.0).abs() < 0.01);
        assert!((map["main"].width - 700.0).abs() < 0.01);
    }

    #[test]
    fn orphaned_single_child_keeps_first_track_without_collapse() {
        let root = style(
            "display:grid;grid-template-columns:220px minmax(0,1fr);width:800px;height:100px;gap:0",
        );
        let tree = LayoutNode::with_children(
            "body",
            root,
            vec![leaf("primary", "width:100%;height:100%")],
        );
        let map = map_of(&tree, 800.0, 100.0);
        assert!((map["primary"].width - 220.0).abs() < 0.01);
    }

    #[test]
    fn white_space_pre_and_pre_wrap_preserve_newlines() {
        for (keyword, expected) in [
            ("pre", WhiteSpaceSpec::Pre),
            ("pre-wrap", WhiteSpaceSpec::PreWrap),
        ] {
            let s = style(&format!(
                "display:block;width:200px;font-size:16px;line-height:20px;white-space:{keyword}"
            ));
            assert_eq!(s.white_space, expected);
            let mut node = LayoutNode::with_children("root", s, Vec::new());
            node.text = Some("ab\ncd".into());
            let map = map_of(&node, 200.0, 80.0);
            assert!(
                (map["root"].height - 40.0).abs() < 0.01,
                "{keyword} + 2 lines * 20px must be 40, got {}",
                map["root"].height
            );
        }
    }
}
