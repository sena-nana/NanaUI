#![allow(unused_imports)]
use std::sync::Arc;

use super::super::ComponentView;
use super::super::component_view::{
    TextFieldProjection, apply_input_recipe, compact_icon_style, control_layout,
    format_range_value, project_common, project_text_field, range_field_style, replace_padding_xy,
    shared_label, text_area_footer_extent, text_field_style, tool_box, with_unit,
};
use super::super::events::*;
use crate::layout_edits::{LayoutEdit, LengthBits, StackPreset, edit_layout};
use crate::{
    AccessibilityRole, AccessibilityState, HighlightRequest, InteractionState, MutationQueue,
    NodeKind, NodeStyle, OverlayHostState, ScrollOffset, SemanticPaint, StableNodeId, TextCodeFold,
    TextColorSwatchSpan, TextCompletion, TextContent, TextDiagnosticSpan, TextEditorRenderOptions,
    TextGitMark, TextHorizontalAlignment, TextHover, TextInlay, TextInputState, TextMatchSpan,
    TextSignatureHelp, TextVerticalAlignment, UiWorld,
};

#[derive(Debug, Clone, PartialEq)]
pub struct List {
    pub label: Option<Arc<str>>,
    /// More than one item can be selected (`aria-multiselectable`), and the
    /// list raises [`crate::ListSelectionRequested`] for clicks and selection
    /// keys. The selection itself stays in the application's
    /// [`nana_ui_core::SelectionState`].
    pub multiselectable: bool,
    /// Columns per row when this list lays out a grid (`EachVirtual::grid`);
    /// 1 for a plain list.
    pub(crate) grid_columns: usize,
    pub style: NodeStyle,
}

impl List {
    pub fn new() -> Self {
        Self {
            label: None,
            multiselectable: false,
            grid_columns: 1,
            style: NodeStyle::default(),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn multiselectable(mut self, multiselectable: bool) -> Self {
        self.multiselectable = multiselectable;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl Default for List {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for List {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::LIST.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(|caps| {
            caps.list = Some(&crate::list_selection::LIST_CAPABILITIES);
        }),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "list".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::List,
                label: self.label.clone(),
                multiselectable: self.multiselectable,
                ..AccessibilityState::default()
            },
        );
    }
}

/// 通用布局容器：预设覆盖常用 flex；完整字段走 [`nana_ui_core::LayoutStyle`]。
///
/// 预设构造器覆盖常用排版，语义与布局引擎默认值对齐过的产物一致：
/// - [`Stack::row`]：水平排列，宽度随内容（工具条、按钮组）。
/// - [`Stack::fill_row`]：水平排列并占满父容器剩余宽度。
/// - [`Stack::column`]：竖直排列，高度随内容（页面纵向结构）。
/// - [`Stack::fill_column`]：竖直排列并占满父容器剩余高度（主内容区）。
/// - [`Stack::bar`]：水平排列占满整行但不伸展（顶栏、底栏）。
/// - [`Stack::layers`]：叠层，子项共用同一块区域、各自对齐（ZStack / Box）。
///
/// CSS 路径用 [`Stack::from_layout`] 写入已解析的 [`nana_ui_core::LayoutStyle`]。
/// grid / position / overflow / paint 用 [`Self::with_layout`]，不要另造布局控件。
///
/// Rust 预设容器默认不参与命中测试；[`Self::from_layout`] 与 CSS 布局盒默认可点。
#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    /// Shared with the node's record and with equal stacks once created
    /// ([`ComponentView::share_style`]): a page of cells holds one style,
    /// not a copy a node.
    style: Arc<NodeStyle>,
    hittable: bool,
    /// The container a keyed list or a conditional block puts its rows in:
    /// layout only, so a [`crate::SettingsGroup`] reads through it to the
    /// rows themselves.
    structural: bool,
    focusable: bool,
}

/// [`edit_layout`] for a length edit: remembered when the length compares
/// to the bit ([`LengthBits`]), a plain copy-on-write edit otherwise.
fn edit_length(
    layout: &mut Arc<nana_ui_core::LayoutStyle>,
    length: nana_ui_core::LengthSpec,
    edit: fn(LengthBits) -> LayoutEdit,
    apply: impl FnOnce(&mut nana_ui_core::LayoutStyle),
) {
    match LengthBits::of(length) {
        Some(bits) => edit_layout(layout, edit(bits), apply),
        None => apply(Arc::make_mut(layout)),
    }
}

impl Stack {
    /// 承接已解析的 [`nana_ui_core::LayoutStyle`]（CSS / 直写字段）。
    ///
    /// 不套用 [`Self::row`] 一类预设默认。默认可点，与 CSS 布局盒一致。
    pub fn from_layout(layout: impl Into<Arc<nana_ui_core::LayoutStyle>>) -> Self {
        let mut style = NodeStyle::default();
        style.layout = layout.into();
        Self {
            style: Arc::new(style),
            hittable: true,
            structural: false,
            focusable: false,
        }
    }

    /// 就地改 [`nana_ui_core::LayoutStyle`] 全字段。
    pub fn with_layout(mut self, f: impl FnOnce(&mut nana_ui_core::LayoutStyle)) -> Self {
        f(Arc::make_mut(&mut self.style_mut().layout));
        self
    }

    /// [`Self::with_layout`] 的借用版：`bind` 闭包里拿到的是 `&mut Stack`，
    /// 直接改字段，不必把整个容器换掉再重建。
    pub fn layout_mut(&mut self) -> &mut nana_ui_core::LayoutStyle {
        Arc::make_mut(&mut self.style_mut().layout)
    }

    #[inline]
    pub(crate) fn elided_flags(&self) -> u8 {
        1 | (u8::from(self.hittable) << 1)
            | (u8::from(self.structural) << 2)
            | (u8::from(self.focusable) << 3)
    }

    #[inline]
    pub(crate) fn from_elided(style: Arc<NodeStyle>, flags: u8) -> Self {
        Self {
            style,
            hittable: (flags & 0b010) != 0,
            structural: (flags & 0b100) != 0,
            focusable: (flags & 0b1000) != 0,
        }
    }

    pub(crate) fn style_ref(&self) -> &NodeStyle {
        &self.style
    }

    pub(crate) fn style_mut(&mut self) -> &mut NodeStyle {
        Arc::make_mut(&mut self.style)
    }

    /// Replace the style with what `edit` makes of it.
    fn restyle(&mut self, edit: impl FnOnce(NodeStyle) -> NodeStyle) {
        let style = self.style_mut();
        *style = edit(std::mem::take(style));
    }

    /// One of the presets below, built as one remembered edit of the shared
    /// default ([`edit_layout`]) and cached per `(preset, gap)`: every row or
    /// column a page builds shares one `Arc<NodeStyle>` with zero allocation.
    fn preset(preset: StackPreset, gap: f32) -> Self {
        use nana_ui_core::{AlignSpec, FlexDirection, JustifySpec, LengthSpec};
        const SLOTS: usize = 16;
        thread_local! {
            static STACK_PRESET_CACHE: std::cell::RefCell<([Option<(StackPreset, u32, Arc<NodeStyle>)>; SLOTS], usize)> =
                const { std::cell::RefCell::new(([const { None }; SLOTS], 0)) };
        }
        let gap = gap.max(0.0);
        let gap_bits = gap.to_bits();
        let cached = STACK_PRESET_CACHE.with(|cache| {
            let (entries, _) = &*cache.borrow();
            entries.iter().flatten().find_map(|(p, g, style)| {
                (*p == preset && *g == gap_bits).then(|| Arc::clone(style))
            })
        });
        if let Some(style) = cached {
            return Self {
                style,
                hittable: false,
                structural: false,
                focusable: false,
            };
        }
        let mut style = NodeStyle::default();
        edit_layout(
            &mut style.layout,
            LayoutEdit::StackPreset(preset, gap_bits),
            |layout| {
                let (direction, align) = match preset {
                    StackPreset::Row | StackPreset::FillRow | StackPreset::Bar => {
                        (FlexDirection::Row, AlignSpec::Center)
                    }
                    StackPreset::Column | StackPreset::FillColumn => {
                        (FlexDirection::Column, AlignSpec::Stretch)
                    }
                };
                layout.display = Some(nana_ui_core::DisplaySpec::Flex);
                layout.direction = Some(direction);
                layout.gap = Some(LengthSpec::Px(gap));
                layout.align_items = align;
                layout.justify_content = JustifySpec::Start;
                let (width, height, min, flex) = match preset {
                    StackPreset::Row => (LengthSpec::Shrink, None, false, None),
                    StackPreset::FillRow => (LengthSpec::Fill, None, false, Some(1.0)),
                    StackPreset::Bar => (LengthSpec::Fill, None, false, Some(0.0)),
                    StackPreset::Column => {
                        (LengthSpec::Fill, Some(LengthSpec::Shrink), true, Some(0.0))
                    }
                    StackPreset::FillColumn => {
                        (LengthSpec::Fill, Some(LengthSpec::Fill), true, Some(1.0))
                    }
                };
                layout.width = Some(width);
                if let Some(height) = height {
                    layout.height = Some(height);
                }
                if min {
                    // `min_width(Px(0.0))`: read as CSS reads `min-width: 0`.
                    layout.allow_shrink = true;
                    layout.min_width = Some(LengthSpec::Px(0.0));
                }
                if preset == StackPreset::FillColumn {
                    layout.min_height = Some(LengthSpec::Px(0.0));
                }
                if let Some(flex) = flex {
                    layout.flex_grow = Some(flex);
                    layout.flex_shrink = Some(flex);
                }
            },
        );
        let style = Arc::new(style);
        STACK_PRESET_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            let slot = cache.1;
            cache.0[slot] = Some((preset, gap_bits, Arc::clone(&style)));
            cache.1 = (slot + 1) % SLOTS;
        });
        Self {
            style,
            hittable: false,
            structural: false,
            focusable: false,
        }
    }

    /// 水平排列，宽度随内容收缩，子项垂直居中。
    pub fn row(gap: f32) -> Self {
        Self::preset(StackPreset::Row, gap)
    }

    /// 水平排列并占满父容器剩余宽度，子项可收缩。
    pub fn fill_row(gap: f32) -> Self {
        Self::preset(StackPreset::FillRow, gap)
    }

    /// 竖直排列，高度随内容，宽度占满父容器。
    pub fn column(gap: f32) -> Self {
        Self::preset(StackPreset::Column, gap)
    }

    /// 竖直排列并占满父容器剩余高度，用于主内容区。
    pub fn fill_column(gap: f32) -> Self {
        Self::preset(StackPreset::FillColumn, gap)
    }

    /// 水平排列占满整行但不伸展（顶栏、底栏）。
    pub fn bar(gap: f32) -> Self {
        Self::preset(StackPreset::Bar, gap)
    }

    /// 弹性空隙：把其后的兄弟推到行尾。基准宽度必须是 0，否则会铺满把兄弟挤出。
    pub fn spacer() -> Self {
        Self::row(0.0).with_layout(|layout| {
            layout.width = Some(nana_ui_core::LengthSpec::Px(0.0));
            layout.min_width = Some(nana_ui_core::LengthSpec::Px(0.0));
            layout.flex_grow = Some(1.0);
            layout.flex_shrink = Some(1.0);
        })
    }

    /// 自由容器：每个直接子项按自己的显式矩形落位
    /// （[`crate::MutationQueue::set_explicit_rect`]），矩形是这个容器 padding box
    /// 内的逻辑像素，原点在左上、y 向下。子项之间没有布局依赖，也不撑大容器；
    /// 容器自己的尺寸来自自己的样式与父级约束。CSS 写作 `display: free`。
    /// 白板、节点图、时间轴、缩略图墙、地图钉用它，外壳仍可以是 flex。
    pub fn free() -> Self {
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).display = Some(nana_ui_core::DisplaySpec::Free);
        Self {
            style: Arc::new(style),
            hittable: false,
            structural: false,
            focusable: false,
        }
    }

    /// 叠层：子项共用同一块区域，后写的盖在先写的上面。每个子项按
    /// [`Self::align`]（竖直）和 [`Self::justify`]（水平）在这块区域里对齐，
    /// 默认左上；容器随最大的子项。等同 SwiftUI `ZStack`、Compose `Box`、
    /// Flutter `Stack`，CSS 写作 `display: layers`。脱流的子项照常按包含块定位，
    /// 不撑大容器。
    pub fn layers() -> Self {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.display = Some(nana_ui_core::DisplaySpec::Layers);
        layout.direction = Some(nana_ui_core::FlexDirection::Row);
        layout.align_items = nana_ui_core::AlignSpec::Start;
        layout.justify_items = Some(nana_ui_core::AlignSpec::Start);
        Self {
            style: Arc::new(style),
            hittable: false,
            structural: false,
            focusable: false,
        }
        .width(nana_ui_core::LengthSpec::Shrink)
    }

    /// 舞台叠层：一个铺满已定位父级的 [`Self::layers`]，脱流、不命中、裁剪。
    /// 弹幕/HUD 容器用这个，节点池与每帧内容仍由应用挂。
    pub fn overlay_layer() -> Self {
        Self::layers().with_layout(|layout| {
            layout.position = nana_ui_core::PositionSpec::Absolute;
            layout.offset_top = Some(nana_ui_core::LengthSpec::Px(0.0));
            layout.offset_left = Some(nana_ui_core::LengthSpec::Px(0.0));
            layout.width = Some(nana_ui_core::LengthSpec::Percent(100.0));
            layout.height = Some(nana_ui_core::LengthSpec::Percent(100.0));
            layout.overflow_x = nana_ui_core::OverflowSpec::Hidden;
            layout.overflow_y = nana_ui_core::OverflowSpec::Hidden;
            layout.pointer_events = Some(nana_ui_core::PointerEventsSpec::None);
        })
    }

    /// 自绘这个容器的外观，见 [`crate::Painter`]。
    pub fn painter(mut self, painter: impl Into<crate::NodePainter>) -> Self {
        self.style_mut().painter = Some(painter.into());
        self
    }

    /// A standard gap that follows the installed spacing scale.
    pub fn gap_theme(mut self, step: nana_ui_core::SpacingStep) -> Self {
        let layout = Arc::make_mut(&mut self.style_mut().layout);
        layout.gap = Some(nana_ui_core::LengthSpec::Px(
            nana_ui_core::SpacingTokens::DEFAULT.resolve(step),
        ));
        layout.theme.gap = Some(nana_ui_core::ThemeToken::Space(step).into());
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        let gap = gap.max(0.0);
        edit_layout(
            &mut self.style_mut().layout,
            LayoutEdit::Gap(gap.to_bits()),
            |layout| {
                layout.theme.gap = None;
                layout.gap = Some(nana_ui_core::LengthSpec::Px(gap));
            },
        );
        self
    }

    pub fn wrap(mut self, wrap: bool) -> Self {
        let wrap = if wrap {
            nana_ui_core::FlexWrap::Wrap
        } else {
            nana_ui_core::FlexWrap::NoWrap
        };
        edit_layout(
            &mut self.style_mut().layout,
            LayoutEdit::Wrap(wrap),
            |layout| {
                layout.flex_wrap = wrap;
            },
        );
        self
    }

    pub fn align(mut self, align: nana_ui_core::AlignSpec) -> Self {
        edit_layout(
            &mut self.style_mut().layout,
            LayoutEdit::Align(align),
            |layout| {
                layout.align_items = align;
            },
        );
        self
    }

    /// 主轴分布。[`Self::layers`] 没有主轴，这里改为子项的水平对齐
    /// （`justify-items`）；`Space*` 在一格里没有意义，按起点对齐。
    pub fn justify(mut self, justify: nana_ui_core::JustifySpec) -> Self {
        use nana_ui_core::{AlignSpec, JustifySpec};
        let layout = Arc::make_mut(&mut self.style_mut().layout);
        layout.justify_content = justify;
        if layout
            .display
            .is_some_and(nana_ui_core::DisplaySpec::is_layers)
        {
            layout.justify_items = Some(match justify {
                JustifySpec::Center => AlignSpec::Center,
                JustifySpec::End => AlignSpec::End,
                JustifySpec::Stretch => AlignSpec::Stretch,
                JustifySpec::Start
                | JustifySpec::SpaceBetween
                | JustifySpec::SpaceAround
                | JustifySpec::SpaceEvenly => AlignSpec::Start,
            });
        }
        self
    }

    pub fn width(mut self, width: nana_ui_core::LengthSpec) -> Self {
        edit_length(
            &mut self.style_mut().layout,
            width,
            LayoutEdit::Width,
            |layout| {
                layout.width = Some(width);
            },
        );
        self
    }

    pub fn height(mut self, height: nana_ui_core::LengthSpec) -> Self {
        edit_length(
            &mut self.style_mut().layout,
            height,
            LayoutEdit::Height,
            |layout| {
                layout.height = Some(height);
            },
        );
        self
    }

    pub fn min_width(mut self, min_width: nana_ui_core::LengthSpec) -> Self {
        edit_length(
            &mut self.style_mut().layout,
            min_width,
            LayoutEdit::MinWidth,
            |layout| {
                // Read as CSS reads `min-width: 0`, so both spellings state one intent.
                layout.allow_shrink |= matches!(
                    min_width,
                    nana_ui_core::LengthSpec::Px(0.0) | nana_ui_core::LengthSpec::Percent(0.0)
                );
                layout.min_width = Some(min_width);
            },
        );
        self
    }

    pub fn min_height(mut self, min_height: nana_ui_core::LengthSpec) -> Self {
        edit_length(
            &mut self.style_mut().layout,
            min_height,
            LayoutEdit::MinHeight,
            |layout| layout.min_height = Some(min_height),
        );
        self
    }

    pub fn max_width(mut self, max_width: f32) -> Self {
        let max_width = max_width.max(0.0);
        edit_layout(
            &mut self.style_mut().layout,
            LayoutEdit::MaxWidth(max_width.to_bits()),
            |layout| layout.max_width = Some(nana_ui_core::LengthSpec::Px(max_width)),
        );
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        edit_layout(
            &mut self.style_mut().layout,
            LayoutEdit::Grow(grow.to_bits()),
            |layout| layout.flex_grow = Some(grow),
        );
        self
    }

    pub fn shrink(mut self, shrink: f32) -> Self {
        edit_layout(
            &mut self.style_mut().layout,
            LayoutEdit::Shrink(shrink.to_bits()),
            |layout| layout.flex_shrink = Some(shrink),
        );
        self
    }

    /// 四边统一内边距，覆盖先前物理及逻辑边声明。
    pub fn padding(self, padding: f32) -> Self {
        self.padding_xy(padding, padding)
    }

    /// 水平与垂直内边距，后调用者覆盖四边。
    pub fn padding_xy(mut self, x: f32, y: f32) -> Self {
        replace_padding_xy(Arc::make_mut(&mut self.style_mut().layout), x, y);
        self
    }

    /// 取出容器样式，可复用到其他控件的 `.style(...)` 上。
    pub fn node_style(&self) -> NodeStyle {
        (*self.style).clone()
    }

    /// 语义背景色。
    pub fn surface(mut self, role: nana_ui_core::SemanticColorRole) -> Self {
        self.style_mut().background = Some(role);
        self
    }

    /// 边框（CSS `border`）：语义色角色与宽度一次写全。边框占布局空间，
    /// 改宽度会挪动内容；见 [`NodeStyle::border`]。
    pub fn border(mut self, role: nana_ui_core::SemanticColorRole, width: f32) -> Self {
        self.restyle(|style| style.border(role, width));
        self
    }

    /// 外环（CSS `outline`）：画在边框盒之外，不占布局空间，宽度为 0 即去掉；
    /// 见 [`NodeStyle::ring`]。
    pub fn ring(mut self, role: nana_ui_core::SemanticColorRole, width: f32) -> Self {
        self.restyle(|style| style.ring(role, width));
        self
    }

    /// 跟随主题的混合背景（如强调色 12% 的淡底），见 [`NodeStyle::surface_mix`]。
    pub fn surface_mix(mut self, mix: nana_ui_core::SemanticColorMix) -> Self {
        self.restyle(|style| style.surface_mix(mix));
        self
    }

    /// 跟随主题的混合边框色与宽度，见 [`NodeStyle::border_mix`]。
    pub fn border_mix(mut self, mix: nana_ui_core::SemanticColorMix, width: f32) -> Self {
        self.restyle(|style| style.border_mix(mix, width));
        self
    }

    /// 圆角半径（物理 px）——一次性覆盖，不跟随主题。
    pub fn radius_px(mut self, radius: f32) -> Self {
        self.restyle(|style| style.radius_px(radius));
        self
    }

    /// 圆角档位，跟随安装的 `ThemeMetrics`。
    pub fn radius(mut self, tier: nana_ui_core::RadiusTier) -> Self {
        self.restyle(|style| style.radius(tier));
        self
    }

    /// 圆角取哪一档由安装主题的 `ShapeRecipe` 按组件角色决定，见 [`NodeStyle::shape`]。
    pub fn shape(mut self, role: nana_ui_core::ShapeRole) -> Self {
        self.restyle(|style| style.shape(role));
        self
    }

    /// 方形最小盒档位（最小宽高），跟随安装的 `ThemeMetrics`。
    pub fn square(mut self, size: nana_ui_core::SquareSize) -> Self {
        self.restyle(|style| style.square(size));
        self
    }

    /// 子项间距档位（或代表它的 `SpacingRole`），跟随安装主题的间距刻度，覆盖构造时给的 `gap`。
    pub fn gap_step(mut self, step: impl Into<nana_ui_core::SpacingStep>) -> Self {
        self.restyle(|style| style.gap_step(step));
        self
    }

    /// 内边距档位（水平、垂直），跟随安装主题的间距刻度。
    pub fn padding_step(
        mut self,
        x: nana_ui_core::SpacingStep,
        y: nana_ui_core::SpacingStep,
    ) -> Self {
        self.restyle(|style| style.padding_step(x, y));
        self
    }

    /// Marks the container of a keyed list or conditional block.
    pub(crate) fn structural(mut self) -> Self {
        self.structural = true;
        self
    }

    pub(crate) fn is_structural(&self) -> bool {
        self.structural
    }

    /// CSS 布局盒：参与命中。Rust 预设默认可点关闭。
    pub fn hittable(mut self) -> Self {
        self.hittable = true;
        self
    }

    /// Make this container focusable (e.g. a selectable card in a virtual list or grid).
    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = Arc::new(style);
        self
    }
}

impl ComponentView for Stack {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::STACK.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        let built = Arc::clone(&self.style.layout);
        share(&mut self.style_mut().layout);
        crate::layout_edits::adopt_shared(&built, &self.style.layout);
    }

    fn share_style(&mut self, share: &mut dyn FnMut(&mut Arc<NodeStyle>)) {
        share(&mut self.style);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "stack".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let hittable = self.hittable
            && !self.style.layout.hidden
            && self.style.layout.pointer_events != Some(nana_ui_core::PointerEventsSpec::None);
        let focusable = self.focusable && !self.style.layout.hidden;
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: hittable,
                focusable,
            },
            AccessibilityState::default(),
        );
    }
}
