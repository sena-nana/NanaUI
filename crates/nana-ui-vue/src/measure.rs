//! [`LayoutStyle`] → 可序列化布局盒（测试 / 诊断 / **预绘制回退**）。
//!
//! **Adapter internals.** Application hosts should use [`crate::prelude`].
//!
//! ## SoT（与 Scene 对照）
//!
//! | 数据 | 权威 | 本模块 |
//! |------|------|--------|
//! | 产品几何盒 | Runtime [`nana_ui_runtime::RuntimeLayoutEngine`] → UiScene | 否 |
//! | 预绘制 / css-parity | **共享算法** [`nana_ui_runtime::RuntimeLayoutEngine::layout_style_tree`] | 适配器 |
//! | hit-test 预绘制盒 | 同上 | 适配器 |
//!
//! `layoutBox` / `getBoundingClientRect` **优先**读 Scene 盒；本模块把
//! [`LayoutNode`] 交给同一个 `RuntimeLayoutEngine`，供
//! `VueHost::resolve_layout` 在尚未 paint 时填充文档缓存，并与 css-parity 对齐。
//! 产品 Vue 混合树走 `RuntimeDocument::flush` 文本+布局，不再另写一套 measure。
//!
//! 盒边 / content-box / inset / gap 解析消费 `nana-ui-core::box_layout`。
//! 布局算法本身只在 Runtime 引擎里实现一次（wrap / 2D grid / auto-fill /
//! percent / calc / absolute / fixed）。

#[cfg(test)]
use crate::css_map::AlignSpec;
use crate::css_map::{FlexDirection, GridTrack, LayoutStyle, LayoutStyleCss, LengthSpec};
use nana_ui_core::DisplaySpec;
use nana_ui_runtime::{LayoutViewport, RuntimeLayoutEngine, StyleLayoutNode};

/// 待测布局树节点。
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
    crate::css_map::with_active_viewport(vw, vh, || {
        crate::css_map::with_active_font_sizes(crate::css_map::FontSizeContext::default(), || {
            RuntimeLayoutEngine
                .layout_style_tree(&to_style_tree(root), LayoutViewport::new(vw, vh))
                .into_iter()
                .map(|(id, box_)| {
                    (
                        id,
                        MeasuredBox::new(box_.x, box_.y, box_.width, box_.height),
                    )
                })
                .collect()
        })
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
    use crate::css_map::{DisplaySpec, VisibilitySpec, WhiteSpaceSpec};
    use std::collections::BTreeMap;

    fn map_of(root: &LayoutNode, w: f32, h: f32) -> BTreeMap<String, MeasuredBox> {
        measure_layout(root, w, h).into_iter().collect()
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
    fn four_side_border_widths_shrink_content_under_border_box() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:300px;height:100px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text(
            "display:flex;flex-direction:row;width:100px;height:40px;padding:0;box-sizing:border-box;border-top-width:1px;border-right-width:2px;border-bottom-width:3px;border-left-width:4px",
            None,
            None,
        );
        let mut inner = LayoutStyle::default();
        inner.apply_css_text("width:100%;height:100%", None, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::with_children(
                "a",
                a,
                vec![LayoutNode::leaf("inner", inner)],
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
        assert!(!map.contains_key("hidden"));
        assert!((map["b"].x - 60.0).abs() < 0.01);
    }

    #[test]
    fn visibility_hidden_occupies_layout_placeholder() {
        // T-V02: visibility:hidden keeps flex gap slot (CSS placeholder).
        let mut gone = LayoutStyle::default();
        gone.apply_css_text("visibility:hidden;width:50px;height:40px", None, None);
        assert_eq!(gone.paint.visibility, Some(VisibilitySpec::Hidden));
        assert!(!gone.hidden);
        assert!(!gone.omits_box());
        assert!(!gone.is_paint_visible());
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
        assert!(map.contains_key("gone"));
        assert!((map["gone"].x - 60.0).abs() < 0.01);
        assert!((map["b"].x - 120.0).abs() < 0.01);
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
    fn unspecified_flex_shrink_does_not_crush_definite_row() {
        // Issue #22: Vue CSS that omits flex-shrink stays 0, not CSS initial 1.
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:150px;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:150px;height:40px", None, None);
        assert_eq!(a.flex_shrink, None);
        assert_eq!(b.flex_shrink, None);
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 240.0, 100.0);
        assert!((map["a"].width - 150.0).abs() < 0.01);
        assert!((map["b"].width - 150.0).abs() < 0.01);
        assert!((map["b"].x - 150.0).abs() < 0.01);
    }

    #[test]
    fn flex_initial_shrinks_overflowing_definite_row() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:150px;flex:initial;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:150px;flex:initial;height:40px", None, None);
        assert_eq!(a.flex_shrink, Some(1.0));
        assert_eq!(b.flex_shrink, Some(1.0));
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
    fn flex_none_keeps_overflowing_definite_row() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:150px;flex:none;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:150px;flex:none;height:40px", None, None);
        assert_eq!(a.flex_grow, Some(0.0));
        assert_eq!(a.flex_shrink, Some(0.0));
        let root = LayoutNode::with_children(
            "root",
            root_s,
            vec![LayoutNode::leaf("a", a), LayoutNode::leaf("b", b)],
        );
        let map = map_of(&root, 240.0, 100.0);
        assert!((map["a"].width - 150.0).abs() < 0.01);
        assert!((map["b"].width - 150.0).abs() < 0.01);
        assert!((map["b"].x - 150.0).abs() < 0.01);
    }

    #[test]
    fn flex_auto_shrinks_overflowing_definite_row() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("width:150px;flex:auto;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("width:150px;flex:auto;height:40px", None, None);
        assert_eq!(a.flex_grow, Some(1.0));
        assert_eq!(a.flex_shrink, Some(1.0));
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
    fn flex_shorthand_grow_basis_omits_shrink_one() {
        let mut root_s = LayoutStyle::default();
        root_s.apply_css_text(
            "display:flex;flex-direction:row;align-items:flex-start;width:200px;height:80px;gap:0",
            None,
            None,
        );
        let mut a = LayoutStyle::default();
        a.apply_css_text("flex:1 150px;height:40px", None, None);
        let mut b = LayoutStyle::default();
        b.apply_css_text("flex:1 150px;height:40px", None, None);
        assert_eq!(a.flex_shrink, Some(1.0));
        assert_eq!(b.flex_shrink, Some(1.0));
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

    #[test]
    fn white_space_pre_text_grows_block_height() {
        let mut style = LayoutStyle::default();
        style.apply_css_text(
            "display:block;width:200px;font-size:16px;line-height:20px;white-space:pre",
            None,
            None,
        );
        let mut node = LayoutNode::with_children("root", style, Vec::new());
        node.text = Some("ab\ncd".into());
        let map = map_of(&node, 200.0, 80.0);
        assert!(
            (map["root"].height - 40.0).abs() < 0.01,
            "pre + 2 lines × 20px must be 40, got {}",
            map["root"].height
        );
    }

    #[test]
    fn white_space_pre_wrap_text_wraps_long_line() {
        let mut style = LayoutStyle::default();
        style.apply_css_text(
            "display:block;width:200px;font-size:16px;line-height:20px;white-space:pre-wrap",
            None,
            None,
        );
        assert_eq!(style.white_space, WhiteSpaceSpec::PreWrap);
        let mut node = LayoutNode::with_children("root", style, Vec::new());
        node.text = Some("ab\ncd".into());
        let map = map_of(&node, 200.0, 80.0);
        assert!(
            (map["root"].height - 40.0).abs() < 0.01,
            "pre-wrap must keep explicit newlines (not Normal), got {}",
            map["root"].height
        );
    }
}
