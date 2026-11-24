#![allow(unused_imports)]
use std::sync::Arc;

use super::super::ComponentView;
use super::super::component_view::{
    TextFieldProjection, apply_input_recipe, compact_icon_style, control_layout,
    format_range_value, project_common, project_text_field, range_field_style, shared_label,
    text_area_footer_extent, text_field_style, tool_box, with_unit,
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
use crate::{AppContext, Entity};

/// Code-editor behaviors enabled on a [`TextArea`].
///
/// The runtime applies bracket pairing, indentation, and line comments on the
/// raw committed text; syntax coloring stays with the `"highlight"`
/// presenter.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeEditing {
    /// Line comment token for toggle commands (for example `"//"`).
    pub comment_prefix: Arc<str>,
    /// Indentation inserted by Tab and copied by auto-indent.
    pub indent_unit: Arc<str>,
}

impl CodeEditing {
    pub fn new(comment_prefix: impl Into<Arc<str>>, indent_unit: impl Into<Arc<str>>) -> Self {
        Self {
            comment_prefix: comment_prefix.into(),
            indent_unit: indent_unit.into(),
        }
    }

    /// Default WGSL-style configuration: `//` comments, tab indentation.
    pub fn wgsl() -> Self {
        Self::new("//", "\t")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextAreaResizeDrag {
    pub pointer: u64,
    pub start_y: f32,
    pub start_height: f32,
    pub previous_height: Option<f32>,
    /// `min-height` / `max-height` resolved when the drag began, in the
    /// units `start_height` is in. Every dragged height stays inside them,
    /// so the kept height is the one the field shows; the grip stays.
    pub min_height: f32,
    pub max_height: Option<f32>,
}

impl TextAreaResizeDrag {
    /// The dragged height for a pointer at `y`, held inside the limits
    /// (`min-height` wins over `max-height`, as in CSS).
    pub(crate) fn height_at(&self, y: f32) -> f32 {
        clamp_resized_height(
            self.start_height + y - self.start_y,
            self.min_height,
            self.max_height,
        )
    }
}

pub(crate) fn clamp_resized_height(height: f32, min: f32, max: Option<f32>) -> f32 {
    let height = max.map_or(height, |max| height.min(max));
    height.max(min).max(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextAreaActionSlot(pub(crate) StableNodeId);

#[derive(Debug, Clone, PartialEq)]
pub struct TextArea {
    pub state: TextInputState,
    /// Optional committed Unicode scalar count; display only, without truncation.
    pub character_counter: Option<usize>,
    /// Grow and shrink between these visual row counts; overflow scrolls.
    pub auto_grow: Option<(usize, usize)>,
    /// Enter submits; Shift+Enter still inserts a newline. IME confirmation never submits.
    pub submit_on_enter: bool,
    pub trailing_action: Option<StableNodeId>,
    pub(crate) footer_expanded: Option<bool>,
    pub resize_vertical: bool,
    pub(crate) resized_height: Option<f32>,
    pub(crate) resize_drag: Option<TextAreaResizeDrag>,
    pub label: Option<Arc<str>>,
    /// Said after the name by assistive technology (`aria-description`),
    /// never drawn. `None` says nothing more.
    pub accessible_description: Option<Arc<str>>,
    pub placeholder: Arc<str>,
    pub disabled: bool,
    /// Keeps focus, selection and copying available while rejecting user edits.
    pub read_only: bool,
    pub invalid: bool,
    pub scroll_offset: ScrollOffset,
    pub style: NodeStyle,
    pub highlight: Option<HighlightRequest>,
    /// 编译诊断 span 标记（错误/警告下划线，可选行尾文案与悬停）。宿主在
    /// 文本变化后负责更新或清除；渲染层只钳制越界偏移。
    pub diagnostics: Arc<[TextDiagnosticSpan]>,
    /// 查找匹配高亮 span（普通匹配与当前匹配）。宿主在文本变化后负责更新
    /// 或清除；渲染层只钳制越界偏移。
    pub match_spans: Arc<[TextMatchSpan]>,
    /// 颜色装饰 span（见 [`TextColorSwatchSpan`]）。宿主在文本变化后负责
    /// 更新或清除；渲染层只钳制越界偏移，纯装饰不参与命中。
    pub color_swatches: Arc<[TextColorSwatchSpan]>,
    /// Atomic inline ranges (see [`crate::TextAtomSpan`]). Hosts feed these
    /// after each value change; caret, delete, insert, and pointer hits treat
    /// each range as one unit. Empty is the ordinary text path.
    pub atom_spans: Arc<[crate::TextAtomSpan]>,
    /// 行号栏。组件自动为行号、折叠箭头和 git 标记预留左侧空间。
    pub line_numbers: bool,
    /// 代码编辑行为（括号配对、缩进、注释切换）。`None` 时为普通多行文本。
    pub code_editing: Option<CodeEditing>,
    /// 代码折叠区间（见 [`TextCodeFold`]）。宿主在每次文本变化后重新喂；
    /// 哪些区间处于折叠态由组件内部维护（宿主重喂时按区间匹配保留，
    /// 漂移的尽力平移匹配，失效的自动展开）。折叠是纯视图状态，不改值。
    pub code_folds: Arc<[TextCodeFold]>,
    /// 行内提示（inlay，见 [`TextInlay`]）。纯插入型显示 span：插入文本
    /// 参与布局测量与软换行，但不占缓冲 offset（光标/点击/查找经显示
    /// 视图映射自动免疫）。宿主在语义快照就绪时喂入；快照失配/偏好关
    /// 时撤空列表。世界校验后按 `(offset, label)` 排序去重；IME 组合期
    /// 与折叠隐藏区间内的条目不显示。空列表零成本。
    pub inlays: Arc<[TextInlay]>,
    /// git gutter 标记（见 [`TextGitMark`]）。宿主在 git 状态与文本变化后
    /// 重新喂：`line` 为 1 基逻辑行号，渲染为 gutter 最左侧 2px 竖条。
    /// 行号 0、超过文档逻辑行数或被折叠隐藏的标记静默跳过；空列表零成本。
    /// 组件自动预留左侧 gutter，不需要宿主设置 padding-left。
    pub git_gutter: Arc<[TextGitMark]>,
    /// 补全候选（见 [`TextCompletion`]）。过滤完全由宿主负责：宿主按当前
    /// 词前缀过滤后在文本/光标变化时重新喂入，非空列表激活候选会话（弹层
    /// 锚定主光标行），空列表关闭。会话由组件内部管理：键盘选中与滚动
    /// 存放在组件内部状态里，重喂相同列表（指针或内容相等）不下发变更，
    /// 选中保持、已 Esc 关闭的弹层也不复活；重喂不同列表视为新会话
    /// （选中归零、重新打开）。
    pub completions: Arc<[TextCompletion]>,
    /// hover 文档浮窗（见 [`TextHover`]）。`Some` 显示在偏移所在行附近，
    /// `None` 隐藏；触发与生命周期完全由宿主决定，文本编辑时宿主负责撤掉
    /// （组件不自动隐藏）。正文滚动存放在组件内部状态里：重喂相同文档
    /// 不下发变更（滚动位置保留），换新文档重新显示并回到顶部。
    pub hover: Option<TextHover>,
    /// 签名帮助浮窗（见 [`TextSignatureHelp`]）。`Some` 锚定 caret 显示，
    /// `None` 隐藏；触发/关闭/活动参数由宿主决定。
    pub signature: Option<TextSignatureHelp>,
    /// 光标处单词/选中文本的出现高亮（内部派生：聚焦时从主光标/选区
    /// 派生查询并扫描全文档，不给宿主增加喂入负担）。默认 `false`：
    /// 派生按出现次数做 shaper 探针（上限 200），成本高于同类的括号
    /// 匹配（常数次探针），由宿主按文档形态自行开启。
    pub occurrence_highlight: bool,
    /// 相对行号：光标行显示绝对行号，其余行显示与光标所在显示行的距离
    /// （Zed 惯例）。仅在 [`Self::line_numbers`] 开启时生效。默认 `false`。
    pub relative_line_numbers: bool,
    /// 空白字符显示：空格画中点、Tab 画箭头；行首缩进与行尾空白一并
    /// 可见。默认 `false`。
    pub show_whitespace: bool,
    /// wrap guide 列参考线：在给定字符列的 x 位置画全高竖线（1px）。列宽
    /// 按 `'0'` 字形宽度估算（等宽字体假设）；文档最宽行不足该列时不画。
    /// 默认为空。
    pub wrap_guides: Arc<[usize]>,
    /// 代码编辑器 minimap：内容区右缘 64px 覆盖竖条（与内容区间 1px
    /// 分隔线），每逻辑行一条 2px 行条（宽度 ∝ 行非空白长度），半透明
    /// 指示器跟随视口；点击条内滚动到对应行（点击行居中），按住拖动
    /// 连续跟随，滚轮落在条上仍按编辑器常规滚动。折叠只影响主视图，
    /// minimap 显示全部逻辑行；文本行宽计算不变——极长行会被条遮挡。
    /// 仅多行编辑器生效。默认 `false`。
    pub minimap: bool,
    /// sticky scroll：滚动视口顶部落在 [`Self::code_folds`] 喂入的区间
    /// 内部时，在内容区顶部钉住显示该区间头行（取首视觉行）；嵌套区间
    /// 钉最内层。区间头自然滚回视口时钉住行消失。折叠语义不改变钉住
    /// 逻辑；钉住行是纯装饰只读渲染。仅多行编辑器生效。默认 `false`。
    pub sticky_scroll: bool,
    pub(crate) style_override: bool,
}

impl TextArea {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            state: TextInputState::new(Into::<String>::into(value)),
            character_counter: None,
            auto_grow: None,
            submit_on_enter: false,
            trailing_action: None,
            footer_expanded: None,
            resize_vertical: false,
            resized_height: None,
            resize_drag: None,
            label: None,
            accessible_description: None,
            placeholder: Arc::from(""),
            disabled: false,
            read_only: false,
            invalid: false,
            scroll_offset: ScrollOffset::default(),
            style: text_field_style(true),
            highlight: None,
            diagnostics: Arc::from([]),
            match_spans: Arc::from([]),
            color_swatches: Arc::from([]),
            atom_spans: Arc::from([]),
            line_numbers: false,
            code_editing: None,
            code_folds: Arc::from([]),
            inlays: Arc::from([]),
            git_gutter: Arc::from([]),
            completions: Arc::from([]),
            hover: None,
            signature: None,
            occurrence_highlight: false,
            relative_line_numbers: false,
            show_whitespace: false,
            wrap_guides: Arc::from([]),
            minimap: false,
            sticky_scroll: false,
            style_override: false,
        }
    }

    /// Size to shaped wrapped content, bounded by visual text rows (at least one).
    /// With min_rows=1, counter/action stay on the first row until displayed
    /// text (including IME) exceeds that compact row; then they move below it.
    pub fn auto_grow(mut self, min_rows: usize, max_rows: usize) -> Self {
        let min_rows = min_rows.max(1);
        self.auto_grow = Some((min_rows, max_rows.max(min_rows)));
        if !self.style_override {
            let layout = Arc::make_mut(&mut self.style.layout);
            // Bind the compact default here, before declarative CSS can
            // override it with an authored absolute or relative line-height.
            if layout.theme.line_height.is_none()
                && layout.line_height == text_field_style(true).layout.line_height
            {
                layout.theme.line_height =
                    Some(nana_ui_core::ThemeToken::Line(nana_ui_core::LineRole::Line).into());
            }
        }
        self
    }

    pub fn character_counter(mut self, limit: usize) -> Self {
        self.character_counter = Some(limit);
        self
    }

    pub fn submit_on_enter(mut self, enabled: bool) -> Self {
        self.submit_on_enter = enabled;
        self
    }

    /// An application action in the bottom trailing corner, in a small-control slot.
    /// The view layer assembles this slot automatically; imperative users call
    /// [`crate::AppContext::assemble_text_area`].
    pub fn trailing_action(mut self, node: StableNodeId) -> Self {
        self.trailing_action = Some(node);
        self
    }

    /// The style the field projects: the authored one with the footer band,
    /// the auto-grow row budget, the marker gutter and the invalid border.
    fn effective_style(&self, id: StableNodeId, world: &UiWorld) -> NodeStyle {
        let size = nana_ui_core::ControlSize::Medium;
        let node_theme = world.node_theme(id);
        let mut effective_style = self.style.clone();
        apply_input_recipe(
            &mut effective_style,
            node_theme.recipes().input(),
            true,
            self.invalid,
            self.style_override,
            node_theme,
        );
        let expanded_footer = self.uses_expanded_footer();
        let footer_extent = if expanded_footer {
            text_area_footer_extent(
                self.character_counter.is_some(),
                self.trailing_action.is_some(),
                world.theme_metrics(),
            )
        } else {
            0.0
        };
        let footer_band = if footer_extent > 0.0 {
            footer_extent + 4.0
        } else {
            0.0
        };
        if let Some((min_rows, max_rows)) = self.auto_grow {
            let max_rows = if self.uses_adaptive_footer() && !expanded_footer {
                min_rows
            } else {
                max_rows
            };
            // Row budgets must use the same installed typography and authored
            // line-height as shaping, including relative or literal overrides.
            let layout = &effective_style.layout;
            let theme = world.node_theme(id);
            let font = layout.theme.font_size.as_ref().map_or_else(
                || {
                    layout
                        .font_size
                        .unwrap_or_else(|| size.text_size_in(theme.typography()))
                },
                |font| font.resolve(theme),
            );
            let line_height = layout
                .theme
                .line_height
                .as_ref()
                .map_or(layout.line_height, |line| {
                    Some(nana_ui_core::LineHeightSpec::Absolute(line.resolve(theme)))
                });
            let line = nana_ui_core::text_line_box_height_px(font, line_height);
            let border = layout.theme.border_width.as_ref().map_or_else(
                || layout.border_width.unwrap_or(0.0),
                |border| border.resolve(theme),
            );
            let chrome = (size.height_in(world.theme_metrics()) - line).max(2.0 * border);
            effective_style.control_padding_y = None;
            effective_style.control_height = None;
            let layout = Arc::make_mut(&mut effective_style.layout);
            layout.height = None;
            layout.min_height = Some(nana_ui_core::LengthSpec::Px(
                min_rows as f32 * line + chrome + footer_band,
            ));
            layout.max_height = Some(nana_ui_core::LengthSpec::Px(
                max_rows as f32 * line + chrome + footer_band,
            ));
            layout.padding_top = Some(nana_ui_core::LengthSpec::Px((chrome - 2.0 * border) / 2.0));
            layout.padding_bottom = layout.padding_top;
            layout.text_ext.word_break = Some(nana_ui_core::WordBreakSpec::BreakAll);
            layout.white_space = nana_ui_core::WhiteSpaceSpec::Normal;
            // Natural rows are a preferred size. A bounded flex parent may
            // reduce the text viewport down to min_rows while the footer stays.
            // Preserve an explicit consumer policy.
            layout.flex_shrink.get_or_insert(1.0);
        }
        if footer_band > 0.0 {
            let padding = effective_style
                .control_padding_y
                .map(|padding| padding.resolve(world.theme_metrics()));
            let top = padding.unwrap_or_else(|| effective_style.layout.resolved_padding().top);
            let bottom =
                padding.unwrap_or_else(|| effective_style.layout.resolved_padding().bottom);
            let layout = Arc::make_mut(&mut effective_style.layout);
            layout.position = nana_ui_core::PositionSpec::Relative;
            layout.padding_top = Some(nana_ui_core::LengthSpec::Px(top));
            layout.padding_bottom = Some(nana_ui_core::LengthSpec::Px(bottom + footer_band));
            effective_style.control_padding_y = None;
        }
        if self.uses_adaptive_footer() && !expanded_footer && self.trailing_action.is_some() {
            let padding = effective_style
                .control_padding_x
                .map(|padding| padding.resolve(world.theme_metrics()))
                .unwrap_or_else(|| effective_style.layout.resolved_padding().right);
            let left_padding = effective_style
                .control_padding_x
                .map(|padding| padding.resolve(world.theme_metrics()))
                .unwrap_or_else(|| effective_style.layout.resolved_padding().left);
            let layout = Arc::make_mut(&mut effective_style.layout);
            layout.position = nana_ui_core::PositionSpec::Relative;
            layout.padding_right = Some(nana_ui_core::LengthSpec::Px(
                padding + nana_ui_core::ControlSize::Small.height_in(world.theme_metrics()) + 4.0,
            ));
            layout.padding_left = Some(nana_ui_core::LengthSpec::Px(left_padding));
            effective_style.control_padding_x = None;
        }
        if self.resize_vertical
            && let Some(height) = self.resized_height
        {
            Arc::make_mut(&mut effective_style.layout).height =
                Some(nana_ui_core::LengthSpec::Px(height));
        }
        if self.line_numbers || !self.code_folds.is_empty() || !self.git_gutter.is_empty() {
            // Reserve the left marker lane even before fold/git results arrive,
            // so asynchronous decorations do not move the source horizontally.
            // Measured at the size the scene paints line labels at: the
            // control's caption size, independently of source text.
            let label_size = size.caption_size();
            let digits = if self.line_numbers {
                let lines = self
                    .state
                    .value
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                lines.ilog10() as usize + 1
            } else {
                0
            };
            // Each digit is given the widest digit's advance, so the gutter
            // holds while the line count stays within the same digit count.
            let digit_width = if digits == 0 {
                0.0
            } else {
                let measure = world.chrome_text_measure(id);
                (0..=9)
                    .map(|digit| measure.width(&digit.to_string(), label_size, None))
                    .fold(0.0, f32::max)
            };
            let gutter = 18.0 + digits as f32 * digit_width + 4.0;
            let named = effective_style
                .control_padding_x
                .map(|padding| padding.resolve(world.theme_metrics()))
                .unwrap_or(0.0);
            let field_pad = named.max(effective_style.layout.resolved_padding().left);
            {
                let layout = Arc::make_mut(&mut effective_style.layout);
                layout.padding_left =
                    Some(nana_ui_core::LengthSpec::Px(field_pad.max(gutter).ceil()));
                layout.padding_right = Some(nana_ui_core::LengthSpec::Px(field_pad));
            }
            // The gutter is a spent number on this axis; keep the resolver
            // from writing Field padding back over it.
            effective_style.control_padding_x = None;
        }
        effective_style
    }

    /// The padding the field ends up with on its right and bottom edges: the
    /// footer action is placed from its padding box, inside that padding.
    pub(crate) fn footer_padding(&self, id: StableNodeId, world: &UiWorld) -> (f32, f32) {
        let style = self.effective_style(id, world);
        let metrics = world.theme_metrics();
        let padding = style.layout.resolved_padding();
        (
            style
                .control_padding_x
                .map_or(padding.right, |padding| padding.resolve(metrics)),
            style
                .control_padding_y
                .map_or(padding.bottom, |padding| padding.resolve(metrics)),
        )
    }

    pub(crate) fn uses_adaptive_footer(&self) -> bool {
        matches!(self.auto_grow, Some((1, _)))
            && (self.character_counter.is_some() || self.trailing_action.is_some())
    }

    pub(crate) fn uses_expanded_footer(&self) -> bool {
        if self.uses_adaptive_footer() {
            self.footer_expanded.unwrap_or(false)
        } else {
            self.character_counter.is_some() || self.trailing_action.is_some()
        }
    }

    /// 启用代码编辑行为：括号配对、Enter 自动缩进、Tab/Shift+Tab 缩进与
    /// 注释切换。软折行保持组件原有样式。
    pub fn code_editor(mut self, enabled: bool) -> Self {
        self.code_editing = enabled.then(CodeEditing::wgsl);
        self
    }

    /// 设置诊断 span 标记（见 [`TextDiagnosticSpan`]）。带 `message` 的条目
    /// 画行尾文案，并在指针落在 span/行尾时弹出 hover。
    pub fn diagnostics(mut self, diagnostics: Arc<[TextDiagnosticSpan]>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// 设置查找匹配高亮 span（见 [`TextMatchSpan`]）。
    pub fn match_spans(mut self, match_spans: Arc<[TextMatchSpan]>) -> Self {
        self.match_spans = match_spans;
        self
    }

    /// 设置颜色装饰 span（见 [`TextColorSwatchSpan`]）。
    pub fn color_swatches(mut self, swatches: Arc<[TextColorSwatchSpan]>) -> Self {
        self.color_swatches = swatches;
        self
    }

    /// Set atomic inline ranges (see [`crate::TextAtomSpan`]).
    pub fn atom_spans(mut self, spans: impl Into<Arc<[crate::TextAtomSpan]>>) -> Self {
        self.atom_spans = spans.into();
        self
    }

    /// 启用行号栏，按文档行数自动预留 gutter；更大的自定义左内边距保留。
    pub fn line_numbers(mut self, line_numbers: bool) -> Self {
        self.line_numbers = line_numbers;
        self
    }

    /// 设置代码折叠区间（见 [`TextCodeFold`]）。宿主在文本变化后重新喂；
    /// 折叠态由组件内部维护。
    pub fn code_folds(mut self, folds: Arc<[TextCodeFold]>) -> Self {
        self.code_folds = folds;
        self
    }

    /// 设置行内提示（见 [`TextInlay`]）。快照失配/偏好关时宿主撤空列表。
    pub fn inlays(mut self, inlays: Arc<[TextInlay]>) -> Self {
        self.inlays = inlays;
        self
    }

    /// 设置 git gutter 标记（见 [`TextGitMark`]）。宿主在 git 状态与文本
    /// 变化后重新喂；行号无效或被折叠隐藏的标记静默跳过。
    pub fn git_gutter(mut self, marks: Arc<[TextGitMark]>) -> Self {
        self.git_gutter = marks;
        self
    }

    /// 设置补全候选（见 [`TextCompletion`]）。过滤由宿主负责：按当前词
    /// 前缀过滤后喂入，空列表表示关闭弹层。
    pub fn completions(mut self, items: Arc<[TextCompletion]>) -> Self {
        self.completions = items;
        self
    }

    /// 设置 hover 文档浮窗（见 [`TextHover`]）。`None` 表示隐藏。
    pub fn hover(mut self, hover: Option<TextHover>) -> Self {
        self.hover = hover;
        self
    }

    /// 设置签名帮助浮窗（见 [`TextSignatureHelp`]）。`None` 表示隐藏。
    pub fn signature(mut self, signature: Option<TextSignatureHelp>) -> Self {
        self.signature = signature;
        self
    }

    /// 开启光标处单词/选中文本的出现高亮（内部派生：聚焦时从主光标或
    /// 非空单行选区派生查询，全文档大小写敏感扫描，全词边界按词模式
    /// 生效；主光标所在出现不画）。默认关闭。
    pub fn occurrence_highlight(mut self, enabled: bool) -> Self {
        self.occurrence_highlight = enabled;
        self
    }

    /// 开启相对行号：光标行显示绝对行号，其余行显示与光标所在显示行的
    /// 距离（Zed 惯例，多光标按主光标）。仅在 [`Self::line_numbers`]
    /// 开启时生效。默认关闭。
    pub fn relative_line_numbers(mut self, enabled: bool) -> Self {
        self.relative_line_numbers = enabled;
        self
    }

    /// 开启空白字符显示：空格画中点、Tab 画箭头；行首缩进与行尾空白
    /// 一并可见。默认关闭。
    pub fn show_whitespace(mut self, enabled: bool) -> Self {
        self.show_whitespace = enabled;
        self
    }

    /// 设置 wrap guide 列参考线：在每个给定字符列（1 起）的 x 位置画
    /// 全高竖线。列宽按 `'0'` 字形宽度估算（等宽字体假设）；文档最宽行
    /// 不足该列时不画。默认为空。
    pub fn wrap_guides(mut self, columns: Arc<[usize]>) -> Self {
        self.wrap_guides = columns;
        self
    }

    /// 开启代码编辑器 minimap：内容区右缘 64px 覆盖竖条 + 视口指示器，
    /// 点击/拖动导航视口（点击行居中）。折叠只影响主视图（minimap 显示
    /// 全部逻辑行）；极长行会被条遮挡。默认关闭。
    pub fn minimap(mut self, enabled: bool) -> Self {
        self.minimap = enabled;
        self
    }

    /// 开启 sticky scroll：滚动视口顶部落在 `code_folds` 喂入的区间内部
    /// 时，在内容区顶部钉住显示该区间头行（嵌套区间钉最内层）。需要同时
    /// 喂入 `code_folds`。默认关闭。
    pub fn sticky_scroll(mut self, enabled: bool) -> Self {
        self.sticky_scroll = enabled;
        self
    }

    /// Color committed text with the registered `"highlight"` presenter.
    pub fn highlight(mut self, language: impl Into<Arc<str>>) -> Self {
        self.highlight = Some(HighlightRequest::highlight(language));
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The spoken name (`aria-label`); the same field as [`Self::label`],
    /// which this control never draws. Independent of the placeholder. An
    /// empty name is none.
    pub fn accessible_name(mut self, name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        self.label = (!name.is_empty()).then_some(name);
        self
    }

    /// See [`Self::accessible_description`](field@Self::accessible_description);
    /// an empty description is none.
    pub fn accessible_description(mut self, description: impl Into<Arc<str>>) -> Self {
        let description = description.into();
        self.accessible_description = (!description.is_empty()).then_some(description);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub(crate) fn supports_vertical_resize(&self) -> bool {
        self.resize_vertical
            && (self.style.layout.box_sizing == nana_ui_core::BoxSizing::BorderBox
                || self.resized_height.is_some()
                || matches!(
                    self.style.layout.height,
                    Some(nana_ui_core::LengthSpec::Px(_))
                ))
    }

    /// Enables a vertical grip. A drag keeps the height inside
    /// `min_height` / `max_height` rather than hiding the grip; content-box
    /// layouts need a pixel height to start from.
    pub fn resize_vertical(mut self, enabled: bool) -> Self {
        self.resize_vertical = enabled;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() {
            // Multiline fields already floor at `min_height: 96`. Clamping to
            // `Medium.height()` (32) never won and pinned the explicit height
            // to the compile-time constant.
            Arc::make_mut(&mut self.style.layout).height =
                Some(nana_ui_core::LengthSpec::Px(height));
        }
        self
    }

    /// The physical scroll offset, clamped by the Runtime to the editor's
    /// scrolling area: negative across a `vertical-rl` editor's columns.
    pub fn scroll_offset(mut self, offset: ScrollOffset) -> Self {
        if offset.x.is_finite() && offset.y.is_finite() {
            self.scroll_offset = offset;
        }
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self.style_override = true;
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        if self.disabled || self.read_only {
            return false;
        }
        let normalized = crate::text_editing::normalize_newlines(text);
        self.state.replace_selection(&normalized)
    }
}

static TEXT_AREA_POINTER_OPS: crate::framework::PointerOps = crate::framework::PointerOps {
    corner_press: Some(AppContext::begin_text_area_resize),
    note_scroll: Some(AppContext::note_text_area_scroll),
    minimap_navigate: Some(AppContext::text_editor_minimap_navigate),
    close_text_atom: Some(AppContext::close_focused_text_atom),
    ..crate::framework::PointerOps::EMPTY
};

impl ComponentView for TextArea {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::TEXT_AREA.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        editable: Some(&crate::framework::Editable::<Self>::EDITOR_HOOKS),
        slot_assembler: Some(crate::AppContext::assemble_text_area),
        lifecycle: Some(crate::AppContext::refresh_text_area_action_slot),
        pointer_ops: Some(&TEXT_AREA_POINTER_OPS),
        post_layout: Some(AppContext::refresh_text_area_compact_layout),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Keep caret, selection and undo history when a declarative snapshot
    /// writes the same value back to a retained editor.
    fn reconcile(&mut self, mut next: Self) {
        if self.state.value == next.state.value {
            next.state = self.state.clone();
        }
        next.footer_expanded = next.footer_expanded.or(self.footer_expanded);
        *self = next;
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "textarea".into(),
        }
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn wants_recipe_reproject() -> bool {
        true
    }

    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::RECIPE_INPUT
    }

    /// The line-number gutter is measured in `project`.
    fn wants_text_backend_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let size = nana_ui_core::ControlSize::Medium;
        let visual = crate::TextInputVisual {
            placeholder: Arc::clone(&self.placeholder),
            size,
            secure: false,
            invalid: self.invalid,
            steppers: false,
            character_counter: self.character_counter,
            diagnostics: Arc::clone(&self.diagnostics),
            matches: Arc::clone(&self.match_spans),
            color_swatches: Arc::clone(&self.color_swatches),
            atoms: Arc::clone(&self.atom_spans),
            line_numbers: self.line_numbers,
            indent_guides: self
                .code_editing
                .as_ref()
                .map(|code| Arc::clone(&code.indent_unit)),
            folds: Arc::clone(&self.code_folds),
            git_marks: Arc::clone(&self.git_gutter),
            editor_options: TextEditorRenderOptions {
                suffix: None,
                resize_vertical: self.supports_vertical_resize(),
                footer_action: self.trailing_action.is_some(),
                adaptive_footer: self.uses_adaptive_footer(),
                expanded_footer: self.uses_expanded_footer(),
                occurrence_highlight: self.occurrence_highlight,
                relative_line_numbers: self.relative_line_numbers,
                show_whitespace: self.show_whitespace,
                wrap_guides: Arc::clone(&self.wrap_guides),
                minimap: self.minimap,
                sticky_scroll: self.sticky_scroll,
                bracket_pair_colors: true,
            },
        };
        if world.text_input_visual_ref(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        // The field is the request; where the editor is drawn follows it.
        if world.scroll_request(id) != Some(self.scroll_offset) {
            mutations.set_scroll_offset(id, self.scroll_offset);
        }
        let snippet_choices = world
            .text_snippet_session(id)
            .and_then(|s| s.choice_items());
        let offered_completions = snippet_choices.as_ref().unwrap_or(&self.completions);
        // 补全候选喂入：列表未变（指针或内容相等）时不下发变更，会话的
        // 键盘选中/滚动原样保留；空列表由世界侧移除会话（弹层关闭）。
        {
            // No session and nothing to offer is already the fed state: an
            // empty list would only remove a session that is not there.
            let fed_unchanged = match world.text_completion_items(id) {
                Some(fed) => Arc::ptr_eq(fed, offered_completions) || fed == offered_completions,
                None => offered_completions.is_empty(),
            };
            if !fed_unchanged {
                mutations.set_text_input_completions(id, Arc::clone(offered_completions));
            }
        }
        // hover 浮窗喂入：内容未变时不下发变更。
        {
            let fed_unchanged = match (world.text_hover_doc(id), self.hover.as_ref()) {
                (Some(fed), Some(hover)) => fed == hover,
                (None, None) => true,
                _ => false,
            };
            if !fed_unchanged {
                mutations.set_text_input_hover(id, self.hover.clone());
            }
        }
        // 签名帮助喂入：内容未变时不下发变更。
        {
            let fed_unchanged = match (world.text_signature_help(id), self.signature.as_ref()) {
                (Some(fed), Some(help)) => fed == help,
                (None, None) => true,
                _ => false,
            };
            if !fed_unchanged {
                mutations.set_text_input_signature(id, self.signature.clone());
            }
        }
        // 行内提示喂入：列表未变（指针或内容相等）且空列表无条目时不下发
        // 变更；世界侧做锚点/文本校验与排序去重。
        {
            let fed_unchanged = match world.text_inlay_items(id) {
                Some(fed) => Arc::ptr_eq(fed, &self.inlays) || *fed == self.inlays,
                None => self.inlays.is_empty(),
            };
            if !fed_unchanged {
                mutations.set_text_input_inlays(id, Arc::clone(&self.inlays));
            }
        }
        let effective_style = self.effective_style(id, world);
        project_text_field(
            id,
            world,
            mutations,
            TextFieldProjection {
                state: &self.state,
                label: &self.label,
                disabled: self.disabled,
                busy: false,
                editable: !self.disabled && !self.read_only,
                invalid: self.invalid,
                multiline: true,
                style: &effective_style,
                highlight: self.highlight.as_ref(),
                numeric: None,
                description: self.accessible_description.clone(),
            },
        );
        // Clear composition while this node is still focused, then release focus.
        // The world validates both transitions atomically in queue order.
        if self.disabled
            && let Some(node) = world.node(id)
            && world.focused(node.document) == Some(id)
        {
            mutations.request_focus(node.document, None);
        }
    }
}

/// Highlighted multiline editor on the same retained [`TextInputState`] as [`TextArea`].
///
/// Official syntax color is the registered `"highlight"` presenter on committed
/// text. IME preedit stays solid. Missing presenters leave the field uncolored.
#[derive(Debug, Clone, PartialEq)]
pub struct HostedTextarea {
    inner: TextArea,
}

impl HostedTextarea {
    pub fn new(value: impl Into<String>, language: impl Into<Arc<str>>) -> Self {
        Self {
            inner: TextArea::new(value).highlight(language),
        }
    }

    pub fn language(&self) -> Option<&str> {
        self.inner
            .highlight
            .as_ref()
            .map(|request| request.language.as_ref())
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.inner = self.inner.placeholder(placeholder);
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.inner = self.inner.label(label);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.inner = self.inner.disabled(disabled);
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.inner = self.inner.read_only(read_only);
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.inner = self.inner.invalid(invalid);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.inner = self.inner.height(height);
        self
    }

    pub fn scroll_offset(mut self, offset: ScrollOffset) -> Self {
        self.inner = self.inner.scroll_offset(offset);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn into_text_area(self) -> TextArea {
        self.inner
    }
}

impl std::ops::Deref for HostedTextarea {
    type Target = TextArea;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for HostedTextarea {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl ComponentView for HostedTextarea {
    const TYPE_ID: Option<&'static str> =
        Some(crate::component_descriptors::HOSTED_TEXTAREA.type_id);

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "hosted-textarea".into(),
        }
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn wants_recipe_reproject() -> bool {
        true
    }

    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::RECIPE_INPUT
    }

    fn wants_text_backend_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.inner.project(id, world, mutations);
    }
}
