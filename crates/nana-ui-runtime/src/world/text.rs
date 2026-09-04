//! Text shaping, editor presentation and display mappings.

use super::*;

pub(super) struct CountingShaper<'a, S: TextShaper> {
    pub(super) inner: &'a mut S,
    pub(super) cache: &'a mut crate::text_layout_cache::TextLayoutCache,
    pub(super) glyphs: &'a mut crate::GlyphCache,
    pub(super) runs: usize,
    pub(super) wrap_layouts: usize,
}

impl<'a, S: TextShaper> CountingShaper<'a, S> {
    pub(super) fn new(
        inner: &'a mut S,
        cache: &'a mut crate::text_layout_cache::TextLayoutCache,
        glyphs: &'a mut crate::GlyphCache,
    ) -> Self {
        Self {
            inner,
            cache,
            glyphs,
            runs: 0,
            wrap_layouts: 0,
        }
    }
}

impl<S: TextShaper> TextShaper for CountingShaper<'_, S> {
    fn shape(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
    ) -> TextMetrics {
        let key = crate::text_layout_cache::TextLayoutKey::new(text, style, constraints);
        if let Some(metrics) = self.cache.lookup(&key) {
            return metrics;
        }
        self.runs = self.runs.saturating_add(1);
        if constraints.wrap {
            self.wrap_layouts = self.wrap_layouts.saturating_add(1);
        }
        let metrics = self
            .inner
            .shape_cached(id, text, style, constraints, self.glyphs);
        self.cache.insert(key, metrics);
        metrics
    }

    fn shape_cached(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
        _glyphs: &mut crate::GlyphCache,
    ) -> TextMetrics {
        self.shape(id, text, style, constraints)
    }
}

pub(super) fn shape_empty_state_text(
    id: StableNodeId,
    visual: &StandardVisual,
    inherited: &ComputedStyle,
    max_width: Option<f32>,
    shaper: &mut impl TextShaper,
) -> EmptyStateTextPresentation {
    let StandardVisual::EmptyState {
        title,
        message,
        compact,
        ..
    } = visual
    else {
        return EmptyStateTextPresentation::default();
    };
    let mut title_style = inherited.clone();
    title_style.font_size = if *compact { 12.0 } else { 13.0 };
    title_style.font_weight = Some(600);
    title_style.line_height = None;
    let mut message_style = inherited.clone();
    message_style.font_size = if *compact { 11.0 } else { 12.0 };
    message_style.font_weight = None;
    message_style.line_height = None;
    let constraints = crate::TextShapeConstraints {
        max_width,
        wrap: max_width.is_some(),
        shaping: crate::TextShaping::Auto,
        ..crate::TextShapeConstraints::default()
    };
    EmptyStateTextPresentation {
        title: shaper.shape(
            id,
            &TextContent {
                value: title.to_string(),
            },
            &title_style,
            constraints,
        ),
        message: message.as_ref().map(|message| {
            shaper.shape(
                id,
                &TextContent {
                    value: message.to_string(),
                },
                &message_style,
                constraints,
            )
        }),
    }
}

pub(super) fn shape_modal_text(
    id: StableNodeId,
    visual: &StandardVisual,
    inherited: &ComputedStyle,
    max_width: Option<f32>,
    shaper: &mut impl TextShaper,
) -> ModalTextPresentation {
    let StandardVisual::ModalFrame {
        title,
        description,
        body_text,
        ..
    } = visual
    else {
        return ModalTextPresentation::default();
    };
    let constraints = crate::TextShapeConstraints {
        max_width,
        wrap: max_width.is_some(),
        shaping: crate::TextShaping::Auto,
        ..Default::default()
    };
    let mut title_style = inherited.clone();
    title_style.font_size = 14.0;
    title_style.font_weight = Some(600);
    title_style.line_height = None;
    let mut description_style = inherited.clone();
    description_style.font_size = 12.0;
    description_style.font_weight = None;
    description_style.line_height = None;
    let mut body_style = inherited.clone();
    body_style.font_size = crate::overlay_surfaces::MODAL_BODY_TEXT_SIZE;
    body_style.font_weight = None;
    body_style.line_height = None;
    ModalTextPresentation {
        title: shaper.shape(
            id,
            &TextContent {
                value: title.to_string(),
            },
            &title_style,
            constraints,
        ),
        description: description.as_ref().map(|value| {
            shaper.shape(
                id,
                &TextContent {
                    value: value.to_string(),
                },
                &description_style,
                constraints,
            )
        }),
        body: body_text.as_ref().map(|value| {
            shaper.shape(
                id,
                &TextContent {
                    value: value.to_string(),
                },
                &body_style,
                constraints,
            )
        }),
    }
}

pub(super) fn progress_geometry(
    bounds: LayoutBox,
    style: &ComputedStyle,
    value_ratio: f32,
    girth: f32,
    corner_radius: f32,
    label: Option<&Arc<str>>,
    cancellable: bool,
    default_label_color: [f32; 4],
) -> Option<crate::ComponentGeometry> {
    let ratio = value_ratio.clamp(0.0, 1.0);
    let girth = if girth.is_finite() && girth > 0.0 {
        girth
    } else {
        6.0
    };
    let cancel_size = 24.0_f32.min(bounds.height).min(bounds.width);
    let heading = if label.is_some() || cancellable {
        12.0_f32.max(if cancellable { cancel_size } else { 0.0 })
    } else {
        0.0
    };
    let cancel = cancellable.then(|| LayoutBox {
        x: bounds.x + (bounds.width - cancel_size).max(0.0),
        y: bounds.y + (heading - cancel_size).max(0.0) / 2.0,
        width: cancel_size,
        height: cancel_size,
    });
    let label_width = cancel
        .map(|cancel| (cancel.x - bounds.x - 8.0).max(0.0))
        .unwrap_or(bounds.width);
    let label_region = label.map(|label| crate::ComponentTextRegion {
        bounds: LayoutBox {
            x: bounds.x,
            y: bounds.y + (heading - 12.0).max(0.0) / 2.0,
            width: label_width,
            height: 12.0_f32.min(bounds.height),
        },
        content: Arc::clone(label),
        color: Some(style.color.unwrap_or(default_label_color)),
        font_size: 12.0,
        font_weight: Some(500),
    });
    let track = if heading > 0.0 {
        let track_y = bounds.y + heading + 6.0;
        LayoutBox {
            x: bounds.x,
            y: track_y,
            width: bounds.width,
            height: girth.min((bounds.y + bounds.height - track_y).max(0.0)),
        }
    } else {
        LayoutBox {
            x: bounds.x,
            y: bounds.y + (bounds.height - girth).max(0.0) / 2.0,
            width: bounds.width,
            height: girth.min(bounds.height),
        }
    };
    Some(crate::ComponentGeometry::Progress {
        fill: LayoutBox {
            width: track.width * ratio,
            ..track
        },
        track,
        label: label_region,
        cancel,
        corner_radius: corner_radius.max(0.0),
    })
}

pub(super) fn form_field_geometry(
    bounds: LayoutBox,
    size: ControlSize,
    label: &Arc<str>,
    hint: Option<&Arc<str>>,
    error: Option<&Arc<str>>,
    control: Option<crate::StableNodeId>,
    layout_box: &dyn Fn(crate::StableNodeId) -> Option<LayoutBox>,
    palette: &SemanticPalette,
) -> Option<crate::ComponentGeometry> {
    let (label_size, _gap, label_role, label_weight) =
        crate::form_surfaces::form_field_density(size);
    let label_height = label_size * 1.2;
    let support = error.or(hint);
    let support_role = if error.is_some() {
        SemanticColorRole::Danger
    } else {
        SemanticColorRole::Muted
    };
    let support_height = 12.0_f32.min(bounds.height);
    let support_y = (bounds.y + bounds.height - support_height).max(bounds.y);
    let (indicator, support_x) = if error.is_some() {
        let slot = 12.0;
        let diameter = slot * 10.0 / 24.0;
        (
            Some((
                LayoutBox {
                    x: bounds.x + (slot - diameter) / 2.0,
                    y: support_y + (support_height - diameter) / 2.0,
                    width: diameter,
                    height: diameter,
                },
                palette.get(support_role).as_rgba_array(),
            )),
            bounds.x + slot + 5.0,
        )
    } else {
        (None, bounds.x)
    };
    Some(crate::ComponentGeometry::FormField {
        label: crate::ComponentTextRegion {
            bounds: LayoutBox {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: label_height.min(bounds.height),
            },
            content: Arc::clone(label),
            color: Some(palette.get(label_role).as_rgba_array()),
            font_size: label_size,
            font_weight: Some(label_weight),
        },
        support: support.map(|message| crate::ComponentTextRegion {
            bounds: LayoutBox {
                x: support_x,
                y: support_y,
                width: (bounds.x + bounds.width - support_x).max(0.0),
                height: support_height,
            },
            content: Arc::clone(message),
            color: Some(palette.get(support_role).as_rgba_array()),
            font_size: 11.0,
            font_weight: None,
        }),
        indicator,
        control: control.and_then(layout_box),
    })
}

pub(super) fn text_input_placeholder_color(layout: &LayoutStyle, faint: [f32; 4]) -> [f32; 4] {
    let mut color = layout.placeholder_color.unwrap_or(faint);
    if let Some(opacity) = layout.placeholder_opacity {
        color[3] = (color[3] * opacity).clamp(0.0, 1.0);
    }
    color
}

pub(super) fn status_tone_role(tone: nana_ui_core::StatusTone) -> SemanticColorRole {
    crate::components::status_tone_role(tone)
}

/// 折叠摘要标记前缀：折叠起始行行尾显示 ` …N`（N 为隐藏行数）。
pub(super) const TEXT_FOLD_MARK_PREFIX: &str = " …";

/// 一个折叠态区间的值空间↔显示空间映射片段。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextDisplaySpan {
    /// 该片段对应的折叠区间（值空间）。
    pub fold: crate::TextCodeFold,
    /// 值空间中被隐藏的字节区间 `[hidden_start, fold.end)`。
    pub value_start: usize,
    pub value_end: usize,
    /// 显示空间中替代文本（` …N`）的起始偏移。
    pub display_start: usize,
    /// 替代文本的字节长度。
    pub display_len: usize,
    /// 该折叠隐藏的逻辑行数（摘要标记中的 N）。
    pub hidden_lines: u32,
}

/// 折叠后的显示视图：`value` 是把折叠态区间替换为 ` …N` 摘要后的显示
/// 文本；`spans` 按值空间顺序列出每个替换片段。几何、点击命中、光标
/// 移动都以显示视图为准；编辑命令仍按原始值语义处理（折叠不改值）。
#[derive(Debug, Clone)]
pub(crate) struct TextDisplayView {
    pub value: String,
    pub spans: Vec<TextDisplaySpan>,
}

impl TextDisplayView {
    /// 值空间偏移 → 显示空间偏移。落在隐藏区间内部时钳制到该折叠的
    /// 替代文本起点（即折叠起始行的行尾）。
    pub fn display_of(&self, offset: usize) -> usize {
        let mut delta = 0isize;
        for span in &self.spans {
            if offset <= span.value_start {
                break;
            }
            if offset >= span.value_end {
                delta += span.display_len as isize - (span.value_end - span.value_start) as isize;
            } else {
                return span.display_start;
            }
        }
        ((offset as isize + delta).max(0)) as usize
    }

    /// 显示空间偏移 → 值空间偏移。落在替代文本内部时钳制到折叠起始行
    /// 的行尾（值空间中该折叠的隐藏起点）。
    pub fn value_of(&self, display: usize) -> usize {
        let mut delta = 0isize;
        for span in &self.spans {
            let display_end = span.display_start + span.display_len;
            if display <= span.display_start {
                break;
            }
            if display >= display_end {
                delta += span.display_len as isize - (span.value_end - span.value_start) as isize;
            } else {
                return span.value_start;
            }
        }
        ((display as isize - delta).max(0)) as usize
    }

    /// 值空间偏移是否严格落在该片段的隐藏区间内部（折叠起始行行尾不算）。
    pub fn span_hides(&self, span: &TextDisplaySpan, offset: usize) -> bool {
        offset > span.value_start && offset < span.value_end
    }
}

/// 由折叠态区间构建显示视图；`collapsed` 为空时返回 `None`（零分配短路）。
///
/// 嵌套折叠：子折叠的隐藏区间与前一个已接受区间重叠（即完全落在父折叠
/// 的隐藏范围内）时跳过——父折叠已经把这些行隐藏。
pub(super) fn build_text_display_view(
    value: &str,
    collapsed: &[crate::TextCodeFold],
) -> Option<TextDisplayView> {
    if collapsed.is_empty() {
        return None;
    }
    let mut display = String::with_capacity(value.len());
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    for &fold in collapsed {
        if fold.start >= fold.end || fold.end > value.len() {
            continue;
        }
        let hidden_start = fold.hidden_start_in(value);
        if hidden_start >= fold.end || hidden_start < cursor {
            // 单行区间没有可隐藏的行；与前一个折叠重叠的子折叠不重复隐藏。
            continue;
        }
        display.push_str(&value[cursor..hidden_start]);
        let display_start = display.len();
        let hidden_lines = value[hidden_start..fold.end].matches('\n').count();
        display.push_str(TEXT_FOLD_MARK_PREFIX);
        display.push_str(&hidden_lines.to_string());
        spans.push(TextDisplaySpan {
            fold,
            value_start: hidden_start,
            value_end: fold.end,
            display_start,
            display_len: display.len() - display_start,
            hidden_lines: hidden_lines as u32,
        });
        cursor = fold.end;
    }
    if spans.is_empty() {
        return None;
    }
    display.push_str(&value[cursor..]);
    Some(TextDisplayView {
        value: display,
        spans,
    })
}

/// 最小变更区间：`(old 中被替换的 start, old 中被替换的 end, 长度差)`。
/// 与 [`crate::text_editing`] 的 transform diff 同构：按公共前后缀夹取。
pub(super) fn value_edit_span(old: &str, new: &str) -> (usize, usize, isize) {
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(current, candidate)| current == candidate)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(current, candidate)| current == candidate)
        .map(|(_, character)| character.len_utf8())
        .sum::<usize>();
    let suffix = suffix.min(old.len() - prefix).min(new.len() - prefix);
    (
        prefix,
        old.len() - suffix,
        new.len() as isize - old.len() as isize,
    )
}

/// 值被编辑后的折叠态重映射（确定性策略）：
/// 1. 折叠区间与被编辑区间相交 → 受影响折叠自动展开；
/// 2. 完全在被编辑区间之后的折叠按长度差整体平移；
/// 3. 其余保持不动。平移后再按新值校验可折叠性，失效的展开。
pub(super) fn remap_collapsed_after_edit(
    collapsed: &[crate::TextCodeFold],
    new_value: &str,
    changed_start: usize,
    changed_end: usize,
    delta: isize,
) -> Vec<crate::TextCodeFold> {
    let mut next = Vec::with_capacity(collapsed.len());
    for &fold in collapsed {
        if fold.end > changed_start && fold.start < changed_end {
            continue;
        }
        let shift = if fold.start >= changed_end { delta } else { 0 };
        let start = (fold.start as isize + shift).max(0) as usize;
        let end = (fold.end as isize + shift).max(0) as usize;
        let fold = crate::TextCodeFold::new(start.min(new_value.len()), end.min(new_value.len()));
        if fold.collapsible_in(new_value) {
            next.push(fold);
        }
    }
    next.sort_by_key(|fold| (fold.start, fold.end));
    next.dedup();
    next
}

/// 值被编辑后的 snippet 跳位重映射：跳位落在被编辑区间内 → 会话失效
/// （`None`）；否则按长度差平移并钳制到新值的字符边界。
pub(super) fn remap_snippet_session(
    session: &crate::components::TextSnippetSession,
    new_value: &str,
    changed_start: usize,
    changed_end: usize,
    delta: isize,
) -> Option<crate::components::TextSnippetSession> {
    let mut stops = Vec::with_capacity(session.stops.len());
    for &stop in &session.stops {
        if stop > changed_start && stop < changed_end {
            return None;
        }
        let mapped = if stop >= changed_end {
            (stop as isize + delta).max(0) as usize
        } else {
            stop
        };
        let mapped = mapped.min(new_value.len());
        if !new_value.is_char_boundary(mapped) {
            return None;
        }
        stops.push(mapped);
    }
    Some(crate::components::TextSnippetSession {
        stops,
        index: session.index,
    })
}

/// 宿主重喂折叠区间后的折叠态保留策略（确定性）：
/// 1. 与新区间完全一致的条目保留；
/// 2. 其余条目尝试整体位移匹配：上一次喂入与本次喂入数量相等、逐位
///    配对长度相等且 start 差唯一非零（典型场景：折叠区上方的编辑使
///    所有区间平移同一偏移）时，把条目按该位移平移，命中新区间的保留；
/// 3. 其余失效条目自动展开。
pub(super) fn reconcile_collapsed_folds(
    previous_offered: &[crate::TextCodeFold],
    offered: &[crate::TextCodeFold],
    collapsed: &[crate::TextCodeFold],
) -> Vec<crate::TextCodeFold> {
    if collapsed.is_empty() {
        return Vec::new();
    }
    let mut next: Vec<crate::TextCodeFold> = collapsed
        .iter()
        .filter(|fold| offered.contains(fold))
        .copied()
        .collect();
    let shift = (previous_offered.len() == offered.len())
        .then(|| {
            let first = offered.first()?.start as isize - previous_offered.first()?.start as isize;
            (first != 0
                && previous_offered
                    .iter()
                    .zip(offered.iter())
                    .all(|(previous, current)| {
                        current.start as isize - previous.start as isize == first
                            && current.end - current.start == previous.end - previous.start
                    }))
            .then_some(first)
        })
        .flatten();
    if let Some(shift) = shift {
        for &fold in collapsed {
            if next.contains(&fold) {
                continue;
            }
            let shifted = crate::TextCodeFold::new(
                (fold.start as isize + shift).max(0) as usize,
                (fold.end as isize + shift).max(0) as usize,
            );
            if offered.contains(&shifted) && !next.contains(&shifted) {
                next.push(shifted);
            }
        }
    }
    next.sort_by_key(|fold| (fold.start, fold.end));
    next.dedup();
    next
}

#[derive(Debug, Clone)]
pub(super) struct TextInputPresentationSource {
    pub(super) text: TextContent,
    pub(super) placeholder: bool,
    pub(super) selection: Option<(usize, usize)>,
    pub(super) caret: usize,
    /// 附加多光标的显示空间 `(start, end)` 区间（收起时光标也在 `caret` 表）。
    pub(super) additional: Vec<(usize, usize)>,
    pub(super) preedit: Option<(usize, usize)>,
    pub(super) multiline: bool,
    /// 代码编辑器扩展：诊断标记 / 查找匹配高亮 / 行号栏（占位符态跳过行号）。
    pub(super) diagnostics: Arc<[TextDiagnosticSpan]>,
    pub(super) matches: Arc<[TextMatchSpan]>,
    /// 颜色装饰 span（宿主喂入；仅多行态派生几何）。
    pub(super) color_swatches: Arc<[TextColorSwatchSpan]>,
    pub(super) line_numbers: bool,
    pub(super) indent_guides: Option<Arc<str>>,
    /// git gutter 标记：宿主行号已校验并映射为显示行索引（0 基）；行号
    /// 无效或所在行被折叠隐藏的标记在构建时剔除。
    pub(super) git_marks: Arc<[(u32, TextGitMarkKind)]>,
    /// 内部派生渲染选项（出现高亮、相对行号、空白显示、wrap guide）。
    /// 占位符/IME 组合态置默认（全部关闭）。
    pub(super) editor: TextEditorRenderOptions,
    /// 节点是否持有文档焦点。出现高亮按聚焦派生（不聚焦零分配跳过，
    /// 绘制层再按焦点门控一次）。
    pub(super) focused: bool,
    /// 折叠显示视图（存在折叠态区间时 Some；`text` 等偏移已在显示空间）。
    pub(super) fold: Option<TextDisplayView>,
    /// 补全候选（宿主过滤后的非空列表；占位符/组合态不弹出）。
    pub(super) completions: Option<Arc<[crate::TextCompletion]>>,
    /// hover 文档（宿主喂入时 Some）。
    pub(super) hover: Option<crate::TextHover>,
    /// minimap 行条长度表（原始文档每逻辑行的非空白字符数，含折叠隐藏
    /// 行）。仅开启选项的多行态收集，其余为空向量（零分配短路）。
    pub(super) minimap_line_lengths: Vec<u32>,
    /// 括号配对着色 span 表 `(start, end, depth)`（显示空间；未配对括号
    /// depth 为 [`TEXT_BRACKET_UNMATCHED_DEPTH`]）。仅多行且开启选项的
    /// 非占位/非组合态收集，随文本版本 memo。
    pub(super) bracket_color_spans: Arc<[(usize, usize, usize)]>,
}

/// 从 [`StandardVisual::TextInput`] 提取的代码编辑器扩展。
#[derive(Debug, Clone, Default)]
pub(super) struct TextInputEditorExtras {
    pub(super) diagnostics: Arc<[TextDiagnosticSpan]>,
    pub(super) matches: Arc<[TextMatchSpan]>,
    pub(super) color_swatches: Arc<[TextColorSwatchSpan]>,
    pub(super) line_numbers: bool,
    pub(super) indent_guides: Option<Arc<str>>,
    pub(super) git_marks: Arc<[TextGitMark]>,
    pub(super) editor: TextEditorRenderOptions,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_text_input_presentation_source(
    state: &TextInputState,
    ime: Option<&ImeComposition>,
    placeholder: &str,
    secure: bool,
    multiline: bool,
    extras: TextInputEditorExtras,
    focused: bool,
    fold: Option<TextDisplayView>,
    completions: Option<Arc<[crate::TextCompletion]>>,
    hover: Option<crate::TextHover>,
) -> TextInputPresentationSource {
    use unicode_segmentation::UnicodeSegmentation;

    // 浮层是打字态的编辑辅助：占位符与 IME 组合期间一律不弹出。
    let (completions, hover) = if !placeholder.is_empty() || ime.is_some() {
        (None, None)
    } else {
        (completions, hover)
    };

    // minimap 行长收集不在源构造内进行：占位符与 IME 组合态的编辑器选项
    // 归默认（不显示 minimap），收集结果只会被丢弃；仅多行且开启选项时
    // 由 [`UiWorld::text_input_presentation_source`] 在早退判定之后收集。

    let mask = |value: &str| {
        if secure {
            "•".repeat(value.graphemes(true).count())
        } else {
            value.to_owned()
        }
    };
    let display_offset = |value: &str, offset: usize| {
        if secure {
            value[..offset].graphemes(true).count() * "•".len()
        } else {
            offset
        }
    };
    if state.value.is_empty() && ime.is_none() && !placeholder.is_empty() {
        return TextInputPresentationSource {
            text: TextContent {
                value: placeholder.to_owned(),
            },
            placeholder: true,
            selection: None,
            caret: 0,
            additional: Vec::new(),
            preedit: None,
            multiline,
            diagnostics: extras.diagnostics,
            matches: extras.matches,
            // 占位文本不是文档内容，颜色装饰 span 不派生几何。
            color_swatches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            // 占位符态没有真实文档内容，git 标记随行号栏一并跳过（避免在
            // 占位文本旁渲染指向不存在行的标记）。
            git_marks: Arc::from([]),
            editor: TextEditorRenderOptions::default(),
            focused: false,
            fold: None,
            completions: None,
            hover: None,
            minimap_line_lengths: Vec::new(),
            bracket_color_spans: Arc::from([]),
        };
    }

    // 折叠视图：secure 掩码与折叠互斥（折叠是代码编辑器特性）；诊断/
    // 匹配 span 完全落在隐藏区间内时随行隐藏（丢弃，不强制展开）。
    let (fold_view, base_value): (Option<TextDisplayView>, String) = match fold {
        Some(view) if !secure => (Some(view.clone()), mask(&view.value)),
        _ => (None, mask(&state.value)),
    };
    let map_offset = |offset: usize| -> usize {
        match &fold_view {
            Some(view) => view.display_of(offset),
            None => offset,
        }
    };
    let selection = if state.selection.is_valid_for(&state.value) {
        state.selection
    } else {
        crate::TextSelection::caret(state.value.len())
    };
    if let Some(ime) = ime {
        let replaced = selection.ordered();
        // 折叠态：组合拼接在显示视图上进行；普通态保持原语义（先切片后
        // 掩码，安全输入的显示偏移按字形重算）。
        let (prefix, suffix) = if let Some(view) = &fold_view {
            let start = view.display_of(replaced.start).min(base_value.len());
            let end = view.display_of(replaced.end).min(base_value.len());
            (base_value[..start].to_owned(), base_value[end..].to_owned())
        } else {
            (
                mask(&state.value[..replaced.start]),
                mask(&state.value[replaced.end..]),
            )
        };
        let preedit_start = prefix.len();
        let preedit_end = preedit_start + ime.text.len();
        let ime_focus = ime
            .selection
            .map(|(_, focus)| focus)
            .filter(|focus| *focus <= ime.text.len() && ime.text.is_char_boundary(*focus))
            .unwrap_or(ime.text.len());
        // 多光标限制：组合输入只挂在主光标上，组合期隐藏附加光标。
        let composed = format!("{prefix}{}{suffix}", ime.text);
        return TextInputPresentationSource {
            text: TextContent {
                value: composed.clone(),
            },
            placeholder: false,
            selection: None,
            caret: preedit_start + ime_focus,
            additional: Vec::new(),
            preedit: Some((preedit_start, preedit_end)),
            multiline,
            diagnostics: extras.diagnostics,
            matches: extras.matches,
            // 组合文本改变了字节布局，宿主偏移失效；组合期不派生 swatch。
            color_swatches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            // 组合期标记按原值行号继续锚定（与诊断一致，宿主拥有生命周期）。
            git_marks: map_git_marks(
                &state.value,
                &composed,
                extras.git_marks,
                fold_view.as_ref(),
            ),
            editor: TextEditorRenderOptions::default(),
            focused,
            fold: fold_view,
            completions: None,
            hover: None,
            minimap_line_lengths: Vec::new(),
            bracket_color_spans: Arc::from([]),
        };
    }

    let anchor = map_offset(display_offset(&state.value, selection.anchor));
    let focus = map_offset(display_offset(&state.value, selection.focus));
    // 附加光标：校验 + 显示空间映射；单光标快速路径下向量为空、零分配。
    let additional = state
        .additional_selections
        .iter()
        .filter(|selection| selection.is_valid_for(&state.value))
        .map(|selection| {
            let start = map_offset(display_offset(&state.value, selection.anchor));
            let end = map_offset(display_offset(&state.value, selection.focus));
            (start.min(end), start.max(end))
        })
        .collect();
    // 诊断/匹配 span 端点映射到显示空间；完全被隐藏的 span 丢弃。
    let map_span = |span_offset: usize, length: usize| -> Option<(usize, usize)> {
        let start = span_offset.min(state.value.len());
        let end = span_offset
            .saturating_add(length.max(1))
            .min(state.value.len());
        if end <= start {
            return None;
        }
        if let Some(view) = &fold_view
            && view
                .spans
                .iter()
                .any(|span| start >= span.value_start && end <= span.value_end)
        {
            return None;
        }
        Some((map_offset(start), map_offset(end)))
    };
    let diagnostics = extras
        .diagnostics
        .iter()
        .filter_map(|span| map_span(span.offset, span.length).map(|_| span.clone()))
        .collect::<Vec<_>>();
    let matches = extras
        .matches
        .iter()
        .filter_map(|span| map_span(span.offset, span.length).map(|_| span.clone()))
        .collect::<Vec<_>>();
    let color_swatches = extras
        .color_swatches
        .iter()
        .filter_map(|span| map_span(span.offset, span.length).map(|_| span.clone()))
        .collect::<Vec<_>>();
    // git gutter 标记：宿主行号校验 + 折叠隐藏行剔除后映射为显示行索引。
    let git_marks = map_git_marks(
        &state.value,
        &base_value,
        extras.git_marks,
        fold_view.as_ref(),
    );
    TextInputPresentationSource {
        text: TextContent { value: base_value },
        placeholder: false,
        selection: (anchor != focus).then_some((anchor.min(focus), anchor.max(focus))),
        caret: focus,
        additional,
        preedit: None,
        multiline,
        diagnostics: Arc::from(diagnostics),
        matches: Arc::from(matches),
        color_swatches: Arc::from(color_swatches),
        git_marks,
        line_numbers: extras.line_numbers,
        indent_guides: extras.indent_guides,
        editor: extras.editor,
        focused,
        fold: fold_view,
        completions,
        hover,
        minimap_line_lengths: Vec::new(),
        bracket_color_spans: Arc::from([]),
    }
}

/// git gutter 标记的行号映射：宿主行号（1 基）→ 显示行索引（0 基）。
/// `value` 是行号语义所属的原始值（行起点按它定位），`display` 是实际
/// 排版的显示值（行索引按它计数）。行号 0、超过文档逻辑行数（尾随换行
/// 不产生幻影行，与行号栏语义一致）或所在行被折叠隐藏的标记静默跳过。
/// 空列表原样返回（零分配零遍历）。
pub(super) fn map_git_marks(
    value: &str,
    display: &str,
    marks: Arc<[TextGitMark]>,
    view: Option<&TextDisplayView>,
) -> Arc<[(u32, TextGitMarkKind)]> {
    if marks.is_empty() {
        return Arc::from([]);
    }
    // 单趟构建行起点表：每标记按表定位行起点、按表计数显示行索引，不再
    // 逐标记从字节 0 回放（O(文档·标记数) → O(文档 + 标记·log 行)）。
    let value_line_starts = line_starts(value);
    let display_line_starts = line_starts(display);
    // 行数语义与行号栏一致：尾随换行不产生幻影行。
    let line_count = if value.ends_with('\n') {
        value_line_starts.len() - 1
    } else {
        value_line_starts.len()
    };
    marks
        .iter()
        .filter_map(|mark| {
            let line_index = usize::try_from(mark.line).ok()?.checked_sub(1)?;
            if line_index >= line_count {
                return None;
            }
            let line_start = value_line_starts[line_index];
            if view.is_some_and(|view| {
                view.spans
                    .iter()
                    .any(|span| view.span_hides(span, line_start))
            }) {
                return None;
            }
            let display_start = match view {
                Some(view) => view.display_of(line_start),
                None => line_start,
            }
            .min(display.len());
            // 显示行索引 = 显示起点前的换行数 = 非零行起点中 ≤ 起点的个数。
            let display_line =
                display_line_starts[1..].partition_point(|&start| start <= display_start);
            Some((display_line as u32, mark.kind))
        })
        .collect()
}

/// 文档行起点表：首项 0，其后每项为对应换行符后的第一个字节偏移。
pub(super) fn line_starts(value: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(value.match_indices('\n').map(|(index, _)| index + 1))
        .collect()
}

/// 括号配对着色的深度色阶：按嵌套深度循环取 5 个主题语义色
/// （accent 蓝 / success 绿 / warning 黄 / danger 红 / muted 中性灰），
/// 与调色板同源保证明暗主题都和谐；未配对括号用 faint 淡化前景（与
/// 语法高亮对 punctuation 的弱化一致）。
pub(super) fn bracket_depth_color(
    palette: &nana_ui_core::SemanticPalette,
    depth: usize,
) -> [f32; 4] {
    if depth == crate::components::TEXT_BRACKET_UNMATCHED_DEPTH {
        return palette.faint.as_rgba_array();
    }
    match depth % 5 {
        0 => palette.accent,
        1 => palette.success,
        2 => palette.warning,
        3 => palette.danger,
        _ => palette.muted,
    }
    .as_rgba_array()
}

/// 把括号配对着色 span 合并进语法高亮 span：括号字符的覆盖色优先，
/// 与括号重叠的语法 span 在括号边界处切分。两侧输入各自不重叠且有序；
/// 输出按起点有序、互不重叠（场景文本渲染按游标推进消费 span）。
pub(super) fn merge_bracket_glyph_spans(
    mut spans: Vec<ExtractedTextSpan>,
    brackets: &[(usize, usize, usize)],
    bracket_color: impl Fn(usize) -> [f32; 4],
) -> Vec<ExtractedTextSpan> {
    if brackets.is_empty() {
        return spans;
    }
    spans.sort_unstable_by_key(|span| (span.start, span.end));
    let mut merged: Vec<ExtractedTextSpan> = Vec::with_capacity(spans.len() + brackets.len());
    let mut bracket_index = 0usize;
    for span in spans.drain(..) {
        while bracket_index < brackets.len() && brackets[bracket_index].1 <= span.start {
            let &(start, end, depth) = &brackets[bracket_index];
            if start < end {
                merged.push(ExtractedTextSpan {
                    start,
                    end,
                    color: bracket_color(depth),
                });
            }
            bracket_index += 1;
        }
        let mut cursor = span.start;
        while bracket_index < brackets.len() && brackets[bracket_index].0 < span.end {
            let &(start, end, depth) = &brackets[bracket_index];
            if start > cursor {
                merged.push(ExtractedTextSpan {
                    start: cursor,
                    end: start,
                    color: span.color,
                });
            }
            merged.push(ExtractedTextSpan {
                start: start.max(cursor),
                end,
                color: bracket_color(depth),
            });
            cursor = end.max(cursor);
            bracket_index += 1;
        }
        if cursor < span.end {
            merged.push(ExtractedTextSpan {
                start: cursor,
                end: span.end,
                color: span.color,
            });
        }
    }
    while bracket_index < brackets.len() {
        let &(start, end, depth) = &brackets[bracket_index];
        if start < end {
            merged.push(ExtractedTextSpan {
                start,
                end,
                color: bracket_color(depth),
            });
        }
        bracket_index += 1;
    }
    merged
}

/// minimap 行长收集：每个逻辑行的非空白字符数（O(文档) 单趟扫描）。
/// 行数与滚动换算的 `matches('\n') + 1` 语义一致（尾随换行是可滚动到
/// 的空逻辑行）；空白行计 0（绘制层不产生行条）。
pub(super) fn collect_non_whitespace_line_lengths(value: &str) -> Vec<u32> {
    value
        .split('\n')
        .map(|line| {
            line.chars()
                .filter(|character| !character.is_whitespace())
                .count() as u32
        })
        .collect()
}

pub(super) fn shape_text_input_presentation(
    id: StableNodeId,
    source: TextInputPresentationSource,
    style: &ComputedStyle,
    constraints: crate::TextShapeConstraints,
    previous_overlays: &crate::components::TextOverlayMetrics,
    shaper: &mut impl TextShaper,
) -> TextInputPresentation {
    // Editing geometry must remain available outside a clipped viewport so the
    // Runtime can scroll the caret into view. Single-line fields retain their
    // unwrapped presentation even if their authored style omits nowrap.
    let presentation_constraints = crate::TextShapeConstraints {
        max_width: if source.multiline {
            constraints.max_width
        } else {
            None
        },
        max_height: None,
        wrap: source.multiline && constraints.wrap,
        ellipsis: false,
        max_lines: None,
        shaping: constraints.shaping,
        preserve_lines: constraints.preserve_lines,
        wrap_break: constraints.wrap_break,
    };
    let (caret_x, caret_y, line_height) = shaper.text_position(
        id,
        &source.text,
        source.caret,
        style,
        presentation_constraints,
    );
    // 选区条带：主选区在前，附加光标选区紧随（多光标选区集互不重叠，
    // 条带天然不重叠，可安全合入同一批次）。
    let mut selection_lines = source.selection.map_or_else(Vec::new, |selection| {
        shaper.text_highlights(id, &source.text, selection, style, presentation_constraints)
    });
    for &(start, end) in &source.additional {
        selection_lines.extend(shaper.text_highlights(
            id,
            &source.text,
            (start, end),
            style,
            presentation_constraints,
        ));
    }
    let preedit_lines = source.preedit.map_or_else(Vec::new, |preedit| {
        shaper.text_highlights(id, &source.text, preedit, style, presentation_constraints)
    });

    // 编辑器扩展：诊断下划线 / 滚动意图几何 / 行号 y 表（仅多行态）。
    // 边界钳制复用 [`crate::text_editing::clamp_boundary`]。
    let diagnostic_marks = if source.multiline {
        let mut marks = Vec::new();
        for span in source.diagnostics.iter() {
            let start = clamp_boundary(&source.text.value, span.offset);
            let end = clamp_boundary(&source.text.value, span.offset + span.length.max(1));
            if end <= start {
                continue;
            }
            for rect in shaper.text_highlights(
                id,
                &source.text,
                (start, end),
                style,
                presentation_constraints,
            ) {
                marks.push(TextDiagnosticMark {
                    rect: LayoutBox {
                        x: rect.x,
                        y: rect.y + rect.height - 2.0,
                        width: rect.width.max(1.0),
                        height: 2.0,
                    },
                    severity: span.severity,
                });
            }
        }
        marks
    } else {
        Vec::new()
    };
    // 查找匹配高亮：与选区一致的整行高条带（非诊断式下划线）。
    let match_marks = if source.multiline {
        let mut marks = Vec::new();
        for span in source.matches.iter() {
            let start = clamp_boundary(&source.text.value, span.offset);
            let end = clamp_boundary(&source.text.value, span.offset + span.length.max(1));
            if end <= start {
                continue;
            }
            for rect in shaper.text_highlights(
                id,
                &source.text,
                (start, end),
                style,
                presentation_constraints,
            ) {
                marks.push(TextMatchMark {
                    rect,
                    current: span.current,
                });
            }
        }
        marks
    } else {
        Vec::new()
    };
    // 颜色装饰 swatch：每个 span 取末显示行，在行内 span 末端画一个行高
    // 65% 的覆盖方块（垂直居中）。纯装饰：不改变布局测量，也无命中框；
    // 覆盖式绘制（半透明合成到字形之上）避免引入任何水平布局位移。
    let swatch_marks = if source.multiline {
        let mut marks = Vec::new();
        for span in source.color_swatches.iter() {
            let start = clamp_boundary(&source.text.value, span.offset);
            let end = clamp_boundary(&source.text.value, span.offset + span.length.max(1));
            if end <= start {
                continue;
            }
            if let Some(rect) = shaper
                .text_highlights(
                    id,
                    &source.text,
                    (start, end),
                    style,
                    presentation_constraints,
                )
                .last()
            {
                // span 末行可能因软换行只剩很小的尾段：方块尺寸仍按整行高
                // 缩放，右缘钳在 span 末行条带右缘（不越过行尾）；尾段比
                // 方块窄时向左扩展，不压到 span 之后的文本。
                let extent = (line_height * 0.65).clamp(6.0, 18.0);
                marks.push(TextSwatchMark {
                    rect: LayoutBox {
                        x: rect.x + rect.width - extent,
                        y: rect.y + (rect.height - extent).max(0.0) * 0.5,
                        width: extent,
                        height: extent,
                    },
                    color: span.color,
                });
            }
        }
        marks
    } else {
        Vec::new()
    };
    // 括号匹配：光标相邻括号与其配对端各一个字符框（描边绘制）。
    let bracket_marks = if source.multiline {
        let value = source.text.value.as_str();
        crate::text_editing::matching_bracket_pair(value, source.caret)
            .map(|(open, close)| {
                [open, close]
                    .into_iter()
                    .map(|offset| {
                        let (x, y, height) = shaper.text_position(
                            id,
                            &source.text,
                            offset,
                            style,
                            presentation_constraints,
                        );
                        let (end_x, _, _) = shaper.text_position(
                            id,
                            &source.text,
                            offset + 1,
                            style,
                            presentation_constraints,
                        );
                        LayoutBox {
                            x,
                            y,
                            width: (end_x - x).max(1.0),
                            height,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // 出现高亮：聚焦且主光标处有词（[A-Za-z0-9_]，全词匹配）或主选区为
    // 非空单行选区时，扫描全文档找出其余出现（大小写敏感；选中文本按
    // 子串匹配）。主光标所在出现不画（选区/当前行条已覆盖）；附加光标
    // 存在时仍按主光标派生。上限 [`crate::text_editing::
    // OCCURRENCE_HIGHLIGHT_LIMIT`] 处防病态文档；未聚焦、无词、多行选区
    // 或 IME 组合期零分配跳过。
    let occurrence_marks = if source.multiline
        && source.focused
        && source.editor.occurrence_highlight
        && source.preedit.is_none()
    {
        let value = source.text.value.as_str();
        match crate::text_editing::occurrence_query_at(value, source.selection, source.caret) {
            Some((query, whole_word)) => {
                let (ranges, _) = crate::text_editing::find_matches_capped(
                    value,
                    &query,
                    crate::text_editing::TextSearchOptions {
                        case_sensitive: true,
                        whole_word,
                    },
                    crate::text_editing::OCCURRENCE_HIGHLIGHT_LIMIT,
                );
                let selection_range = source.selection.map(|(start, end)| start..end);
                ranges
                    .into_iter()
                    .filter(|found| {
                        Some(found) != selection_range.as_ref()
                            && !(found.start <= source.caret && source.caret <= found.end)
                    })
                    .flat_map(|found| {
                        shaper.text_highlights(
                            id,
                            &source.text,
                            (found.start, found.end),
                            style,
                            presentation_constraints,
                        )
                    })
                    .collect()
            }
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };
    // 空白字符显示：空格与 Tab 各给一个文本空间字符单元标记（绘制层画
    // 中点/箭头）。每个连续空白 run 做两次端点探测，run 内按线性插值定
    // 位：等宽字体（代码编辑场景）下精确，比例字体下为近似。端点探测
    // 走 shaper 整段布局缓存，同一布局输入只在首次探针时布局一次。
    // 行首缩进与行尾空白一并可见；未开启选项零分配跳过。
    let whitespace_marks = if source.multiline && source.editor.show_whitespace {
        let value = source.text.value.as_str();
        let mut marks = Vec::new();
        let mut offset = 0usize;
        while offset < value.len() {
            if !matches!(value.as_bytes()[offset], b' ' | b'\t') {
                offset += 1;
                continue;
            }
            let run_start = offset;
            while offset < value.len() && matches!(value.as_bytes()[offset], b' ' | b'\t') {
                offset += 1;
            }
            let (start_x, y, height) =
                shaper.text_position(id, &source.text, run_start, style, presentation_constraints);
            let (end_x, _, _) =
                shaper.text_position(id, &source.text, offset, style, presentation_constraints);
            let run = &value[run_start..offset];
            let cell = ((end_x - start_x) / run.len() as f32).max(1.0);
            for (index, &byte) in run.as_bytes().iter().enumerate() {
                marks.push(TextWhitespaceMark {
                    rect: LayoutBox {
                        x: start_x + cell * index as f32,
                        y,
                        width: cell,
                        height,
                    },
                    kind: if byte == b'\t' {
                        TextWhitespaceKind::Tab
                    } else {
                        TextWhitespaceKind::Space
                    },
                });
            }
        }
        marks
    } else {
        Vec::new()
    };
    // wrap guide 列参考线：列宽按 '0' 字形宽度估算（等宽字体假设）；文
    // 档最宽行不足该列时不画。'0' 与整段宽度度量都落在 shaper 的整段布
    // 展缓存上：同一布局输入每趟至多一次真实布局，其后探针为缓存命中
    // （'0' 是单字符键，文档键与光标/高亮探针共享）。
    let wrap_guides = if source.multiline && !source.editor.wrap_guides.is_empty() {
        let unit = TextContent {
            value: "0".to_owned(),
        };
        let char_width = shaper.horizontal_offset(id, &unit, 1, style).max(1.0);
        let text_width = shaper
            .shape(id, &source.text, style, presentation_constraints)
            .width;
        source
            .editor
            .wrap_guides
            .iter()
            .filter_map(|&column| {
                let x = column as f32 * char_width;
                (column > 0 && x < text_width).then_some(x)
            })
            .collect()
    } else {
        Vec::new()
    };
    // 缩进参考线：每个逻辑行的前导空白内按缩进单位宽度画竖线。
    let indent_guides = if source.multiline {
        source
            .indent_guides
            .as_deref()
            .map(|unit| {
                let unit_content = TextContent {
                    value: unit.to_owned(),
                };
                let unit_width = shaper
                    .horizontal_offset(id, &unit_content, unit.len(), style)
                    .max(1.0);
                let value = source.text.value.as_str();
                let mut guides = Vec::new();
                let mut cursor = 0usize;
                loop {
                    let line_end = value[cursor..]
                        .find('\n')
                        .map_or(value.len(), |index| cursor + index);
                    let content_start =
                        crate::text_editing::line_content_start(value, cursor).min(line_end);
                    if content_start > cursor {
                        let (content_x, line_y, height) = shaper.text_position(
                            id,
                            &source.text,
                            content_start,
                            style,
                            presentation_constraints,
                        );
                        let levels = ((content_x / unit_width) + f32::EPSILON).floor().max(0.0);
                        for level in 0..levels as usize {
                            guides.push(LayoutBox {
                                x: level as f32 * unit_width,
                                y: line_y,
                                width: 1.0,
                                height,
                            });
                        }
                    }
                    if line_end >= value.len() {
                        break;
                    }
                    cursor = line_end + 1;
                }
                guides
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // 行顶表：行号栏、git gutter 标记或 sticky scroll 需要时计算（一次/
    // 行的 text_position 探针；三者都没有时零成本短路）。git 标记与
    // sticky 钉住派生复用同一张表按显示行索引定位，软换行自然取逻辑行行首。
    let (line_tops, line_numbers) = if source.multiline
        && (source.line_numbers || !source.git_marks.is_empty() || source.editor.sticky_scroll)
    {
        let value = source.text.value.as_str();
        let mut starts: Vec<usize> = vec![0];
        for (index, byte) in value.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        if value.ends_with('\n') {
            starts.pop();
        }
        let tops: Vec<f32> = starts
            .iter()
            .map(|&start| {
                shaper
                    .text_position(id, &source.text, start, style, presentation_constraints)
                    .1
            })
            .collect();
        // 折叠隐藏行后，显示行索引不再等于原始逻辑行号：把每个折叠片段
        // 之前的隐藏行数累计回行号（无折叠时返回空表，几何层按索引 + 1）。
        let mut numbers = match &source.fold {
            Some(view) => {
                let span_lines: Vec<(usize, u32)> = view
                    .spans
                    .iter()
                    .map(|span| {
                        (
                            view.value[..span.display_start].matches('\n').count(),
                            span.hidden_lines,
                        )
                    })
                    .collect();
                starts
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        let mut number = index as u32;
                        for &(span_line, hidden) in &span_lines {
                            if span_line < index {
                                number += hidden;
                            }
                        }
                        number + 1
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        // 相对行号（Zed 惯例，见 zed-industries/zed#62311：光标行显示绝
        // 对行号，其余行显示与光标所在行的距离；"光标行显示 1" 是被报告
        // 的 bug 而非预期）。多光标按主光标；距离按显示行计（所见即所得，
        // 折叠摘要行也参与计数）。
        if source.editor.relative_line_numbers {
            let caret_line = value[..clamp_boundary(value, source.caret)]
                .matches('\n')
                .count();
            numbers = (0..tops.len())
                .map(|index| {
                    if index == caret_line {
                        numbers.get(index).copied().unwrap_or(index as u32 + 1)
                    } else {
                        (index.abs_diff(caret_line)).min(u32::MAX as usize) as u32
                    }
                })
                .collect();
        }
        (tops, numbers)
    } else {
        (Vec::new(), Vec::new())
    };
    // 折叠摘要标记：折叠起始行行尾 ` …N` 的文本框（文本空间），供几何层
    // 生成点击命中区域。
    let fold_marks = match &source.fold {
        Some(view) if source.multiline => view
            .spans
            .iter()
            .map(|span| {
                let (x, y, height) = shaper.text_position(
                    id,
                    &source.text,
                    span.display_start,
                    style,
                    presentation_constraints,
                );
                let (end_x, _, _) = shaper.text_position(
                    id,
                    &source.text,
                    span.display_start + span.display_len,
                    style,
                    presentation_constraints,
                );
                crate::components::TextFoldMark {
                    rect: LayoutBox {
                        x,
                        y,
                        width: (end_x - x).max(1.0),
                        height,
                    },
                    fold: span.fold,
                }
            })
            .collect(),
        _ => Vec::new(),
    };

    // git gutter 标记条带：显示行行顶的 2px 竖条素材（文本空间；x 与
    // 颜色由几何层按 gutter 与语义令牌解析）。越界行索引（源阶段已过滤，
    // 双重保险）静默跳过；单行态或空列表零分配短路。
    let git_marks = if source.multiline && !source.git_marks.is_empty() {
        source
            .git_marks
            .iter()
            .filter_map(|&(line, kind)| {
                let top = *line_tops.get(line as usize)?;
                Some(TextGitGutterMark {
                    y: top,
                    height: line_height,
                    kind,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    // 锚定浮层度量：补全行宽按 items 指针相等短路（列表未变零测量、
    // 零分配）；hover 锚点跟随文档偏移，每次 shape 一探（缓存字形度量）。
    let overlay_metrics = TextOverlayMetrics {
        completion: completion_popup_metrics(id, &source, previous_overlays, style, shaper),
        hover_anchor: source.hover.as_ref().map(|doc| {
            let (x, y, _) = shaper.text_position(
                id,
                &source.text,
                doc.offset,
                style,
                presentation_constraints,
            );
            (x, y)
        }),
    };

    TextInputPresentation {
        display_value: source.text.value.clone(),
        placeholder: source.placeholder,
        selection: source.selection.map(|(start, end)| {
            (
                shaper.horizontal_offset(id, &source.text, start, style),
                shaper.horizontal_offset(id, &source.text, end, style),
            )
        }),
        selection_lines: if source.multiline {
            selection_lines
        } else {
            Vec::new()
        },
        caret_x,
        caret_y: if source.multiline { caret_y } else { 0.0 },
        line_height,
        preedit: source.preedit.map(|(start, end)| {
            (
                shaper.horizontal_offset(id, &source.text, start, style),
                shaper.horizontal_offset(id, &source.text, end, style),
            )
        }),
        preedit_lines: if source.multiline {
            preedit_lines
        } else {
            Vec::new()
        },
        // 附加光标：收起态才画 caret（range 选区由条带表达）。
        additional_carets: if source.multiline {
            source
                .additional
                .iter()
                .filter(|(start, end)| start == end)
                .map(|&(offset, _)| {
                    let (x, y, _) = shaper.text_position(
                        id,
                        &source.text,
                        offset,
                        style,
                        presentation_constraints,
                    );
                    (x, y)
                })
                .collect()
        } else {
            Vec::new()
        },
        diagnostic_marks,
        match_marks,
        swatch_marks,
        bracket_marks,
        bracket_color_spans: source.bracket_color_spans,
        occurrence_marks,
        whitespace_marks,
        wrap_guides,
        indent_guides,
        line_tops,
        line_numbers,
        fold_marks,
        git_marks,
        overlay_metrics,
        minimap_line_lengths: source.minimap_line_lengths,
    }
}

/// 锚定浮层的共享输入：锚点行在节点空间的位置（x 为锚点字形左缘）。
#[derive(Debug, Clone, Copy)]
pub(super) struct OverlayAnchor {
    pub(super) x: f32,
    pub(super) line_top: f32,
    pub(super) line_height: f32,
}

/// minimap 竖条几何：内容区右缘 64px 面板、1px 分隔线、按非空白行长
/// 定宽的 2px 行条（文档超出条高容纳量时按整数步长抽稀）与跟随滚动的
/// 半透明视口指示器。绘制与指针导航共用同一投影（行换算见
/// [`TextMinimapGeometry::line_at`]）。
pub(crate) fn text_minimap_geometry(
    lengths: &[u32],
    content: LayoutBox,
    scroll_y: f32,
    line_height: f32,
    palette: &SemanticPalette,
) -> crate::TextMinimapGeometry {
    let line_count = lengths.len();
    let panel = LayoutBox {
        x: (content.x + content.width - crate::components::TEXT_MINIMAP_STRIP_WIDTH).max(content.x),
        y: content.y,
        width: content
            .width
            .min(crate::components::TEXT_MINIMAP_STRIP_WIDTH),
        height: content.height,
    };
    let separator = LayoutBox {
        x: panel.x - 1.0,
        y: panel.y,
        width: 1.0,
        height: panel.height,
    };
    let capacity =
        ((panel.height / crate::components::TEXT_MINIMAP_BAR_PITCH).floor() as usize).max(1);
    let stride = if line_count > capacity {
        line_count.div_ceil(capacity)
    } else {
        1
    };
    let max_length = lengths.iter().copied().max().unwrap_or(0).max(1) as f32;
    let bars = lengths
        .iter()
        .enumerate()
        .filter_map(|(index, &length)| {
            // 空白行不产生条；抽稀步长取整，槽位按 index / stride 落点。
            if length == 0 || (stride > 1 && index % stride != 0) {
                return None;
            }
            Some(LayoutBox {
                x: panel.x,
                y: panel.y + (index / stride) as f32 * crate::components::TEXT_MINIMAP_BAR_PITCH,
                width: (length as f32 / max_length * panel.width).max(1.0),
                height: crate::components::TEXT_MINIMAP_BAR_PITCH,
            })
        })
        .collect();
    // 视口指示器：视口行范围按同一投影换算（连续映射，随滚动平滑移动）；
    // 底缘钳到文档末行，文档在视口内放得下时不画。
    let line_height = line_height.max(1.0);
    let total_height = line_count as f32 * line_height;
    let indicator = if total_height > content.height + f32::EPSILON {
        let pitch = crate::components::TEXT_MINIMAP_BAR_PITCH / stride.max(1) as f32;
        let first_line = (scroll_y / line_height).max(0.0);
        let visible_lines = (content.height / line_height).ceil().max(1.0);
        let y = panel.y + first_line * pitch;
        let bottom = panel.y
            + (first_line + visible_lines)
                .min(line_count as f32)
                .max(first_line + 1.0)
                * pitch;
        let height = (bottom - y).clamp(2.0, panel.height);
        let y = y.clamp(panel.y, (panel.y + panel.height - height).max(panel.y));
        Some(LayoutBox {
            x: panel.x,
            y,
            width: panel.width,
            height,
        })
    } else {
        None
    };
    let accent = palette.accent.as_rgba_array();
    crate::TextMinimapGeometry {
        panel,
        separator,
        bars,
        indicator,
        panel_color: palette.subtle.as_rgba_array(),
        bar_color: palette.faint.as_rgba_array(),
        indicator_color: [accent[0], accent[1], accent[2], accent[3] * 0.2],
        stride,
        line_count,
    }
}

/// 锚定浮层的共享定位（补全弹层与 hover 浮窗共用，避免两套定位代码）：
/// 优先放在锚点行下方，视口底部放不下且上方放得下时翻转到行上方，
/// 最后整体钳进视口。返回面板矩形（节点空间）。
pub(super) fn anchored_overlay_panel(
    anchor: OverlayAnchor,
    width: f32,
    height: f32,
    viewport: LayoutBox,
    gap: f32,
) -> LayoutBox {
    const VIEWPORT_PAD: f32 = 2.0;
    let viewport_bottom = viewport.y + viewport.height;
    let min_y = viewport.y + VIEWPORT_PAD;
    let max_y = (viewport_bottom - VIEWPORT_PAD - height).max(min_y);
    let mut y = anchor.line_top + anchor.line_height + gap;
    if y > max_y {
        let flipped = anchor.line_top - gap - height;
        if flipped >= min_y {
            y = flipped;
        }
    }
    let y = y.clamp(min_y, max_y);
    let min_x = viewport.x + VIEWPORT_PAD;
    let max_x = (viewport.x + viewport.width - VIEWPORT_PAD - width).max(min_x);
    let x = anchor.x.clamp(min_x, max_x);
    LayoutBox {
        x,
        y,
        width,
        height,
    }
}

/// 补全弹层几何：面板 + 可见行（label 主文本、detail 次要说明、kind 右
/// 对齐标注）。宽度自适应最长行（label > detail > kind 依次让位，上限
/// [`crate::components::TEXT_COMPLETION_MAX_CONTENT_WIDTH`]），高度最多
/// [`crate::components::TEXT_COMPLETION_VISIBLE_ROWS`] 行。
pub(super) fn completion_popup_geometry(
    state: &crate::store::TextCompletionViewState,
    metrics: &crate::components::TextCompletionPopupMetrics,
    anchor: OverlayAnchor,
    viewport: LayoutBox,
    font_size: f32,
    palette: &SemanticPalette,
) -> Option<crate::TextCompletionPopup> {
    const GAP: f32 = 12.0;
    const V_PAD: f32 = 4.0;
    const ROW_GAP_ABOVE_BELOW: f32 = 4.0;
    let items = &state.items;
    if items.is_empty() {
        return None;
    }
    let len = items.len();
    let first_row = state.scroll.min(len.saturating_sub(1));
    let visible_rows = (len - first_row).min(crate::components::TEXT_COMPLETION_VISIBLE_ROWS);
    let row_height = anchor.line_height.max(1.0);
    let label_w = metrics.label_width;
    let mut content = label_w;
    let show_detail = metrics.detail_width > 0.0
        && content + GAP + metrics.detail_width
            <= crate::components::TEXT_COMPLETION_MAX_CONTENT_WIDTH;
    if show_detail {
        content += GAP + metrics.detail_width;
    }
    let show_kind = metrics.kind_width > 0.0
        && content + GAP + metrics.kind_width
            <= crate::components::TEXT_COMPLETION_MAX_CONTENT_WIDTH;
    if show_kind {
        content += GAP + metrics.kind_width;
    }
    let content = content.min(crate::components::TEXT_COMPLETION_MAX_CONTENT_WIDTH);
    let label_w = label_w.min(content);
    let panel = anchored_overlay_panel(
        anchor,
        (content + crate::components::TEXT_COMPLETION_PANEL_PAD * 2.0).max(0.0),
        visible_rows as f32 * row_height + V_PAD * 2.0,
        viewport,
        ROW_GAP_ABOVE_BELOW,
    );
    let rows = items[first_row..first_row + visible_rows]
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let y = panel.y + V_PAD + index as f32 * row_height;
            let label_rect_w = label_w.min(content);
            let detail_x =
                panel.x + crate::components::TEXT_COMPLETION_PANEL_PAD + label_rect_w + GAP;
            let detail_rect = show_detail
                .then_some(())
                .filter(|_| !item.detail.is_empty())
                .map(|_| crate::ComponentTextRegion {
                    bounds: LayoutBox {
                        x: detail_x,
                        y,
                        width: metrics.detail_width,
                        height: row_height,
                    },
                    content: Arc::from(item.detail.as_str()),
                    color: Some(palette.muted.as_rgba_array()),
                    font_size,
                    font_weight: None,
                });
            let kind_rect = show_kind
                .then_some(())
                .filter(|_| !item.kind_label.is_empty())
                .map(|_| crate::ComponentTextRegion {
                    bounds: LayoutBox {
                        x: panel.x + panel.width
                            - crate::components::TEXT_COMPLETION_PANEL_PAD
                            - metrics.kind_width,
                        y,
                        width: metrics.kind_width,
                        height: row_height,
                    },
                    content: Arc::from(item.kind_label.as_str()),
                    color: Some(palette.faint.as_rgba_array()),
                    font_size,
                    font_weight: None,
                });
            crate::TextCompletionRow {
                bounds: LayoutBox {
                    x: panel.x,
                    y,
                    width: panel.width,
                    height: row_height,
                },
                label: crate::ComponentTextRegion {
                    bounds: LayoutBox {
                        x: panel.x + crate::components::TEXT_COMPLETION_PANEL_PAD,
                        y,
                        width: label_rect_w,
                        height: row_height,
                    },
                    content: Arc::from(item.label.as_str()),
                    color: Some(palette.text.as_rgba_array()),
                    font_size,
                    font_weight: None,
                },
                detail: detail_rect,
                kind: kind_rect,
            }
        })
        .collect();
    Some(crate::TextCompletionPopup {
        panel,
        selected: state.selected,
        first_row,
        rows,
        background: palette.surface.as_rgba_array(),
        border: palette.border_strong.as_rgba_array(),
        selected_background: palette.hover.as_rgba_array(),
        label_color: palette.text.as_rgba_array(),
        detail_color: palette.muted.as_rgba_array(),
        kind_color: palette.faint.as_rgba_array(),
    })
}

/// hover 浮窗几何：面板 + 标题行（强调）+ 正文逻辑行切片。宽度取
/// 视口与上限的较小值；正文超出 [`crate::components::TEXT_HOVER_MAX_BODY_ROWS`] 行
/// 时滚轮滚动（切片由框架命令写回的滚动位置决定）。
pub(super) fn hover_popup_geometry(
    state: &crate::store::TextHoverViewState,
    anchor: OverlayAnchor,
    viewport: LayoutBox,
    font_size: f32,
    palette: &SemanticPalette,
) -> Option<crate::TextHoverPopup> {
    const MAX_WIDTH: f32 = 420.0;
    const H_PAD: f32 = 10.0;
    const V_PAD: f32 = 6.0;
    const TITLE_BODY_GAP: f32 = 4.0;
    const VIEWPORT_GAP: f32 = 4.0;
    let line_height = anchor.line_height.max(1.0);
    let body_lines: Vec<&str> = state.doc.body.lines().collect();
    let scroll = state.scroll.min(body_lines.len().saturating_sub(1));
    let visible = &body_lines
        [scroll..(scroll + crate::components::TEXT_HOVER_MAX_BODY_ROWS).min(body_lines.len())];
    let width = MAX_WIDTH.min(viewport.width.max(1.0));
    let title_height = line_height;
    let panel = anchored_overlay_panel(
        anchor,
        width,
        V_PAD * 2.0 + title_height + TITLE_BODY_GAP + visible.len() as f32 * line_height,
        viewport,
        VIEWPORT_GAP,
    );
    let content_width = (width - H_PAD * 2.0).max(0.0);
    let title = crate::ComponentTextRegion {
        bounds: LayoutBox {
            x: panel.x + H_PAD,
            y: panel.y + V_PAD,
            width: content_width,
            height: title_height,
        },
        content: Arc::from(state.doc.title.as_str()),
        color: Some(palette.text.as_rgba_array()),
        font_size,
        font_weight: Some(600),
    };
    let body_rows = visible
        .iter()
        .enumerate()
        .map(|(index, line)| crate::ComponentTextRegion {
            bounds: LayoutBox {
                x: panel.x + H_PAD,
                y: panel.y + V_PAD + title_height + TITLE_BODY_GAP + index as f32 * line_height,
                width: content_width,
                height: line_height,
            },
            content: Arc::from(*line),
            color: Some(palette.muted.as_rgba_array()),
            font_size,
            font_weight: None,
        })
        .collect();
    Some(crate::TextHoverPopup {
        panel,
        title,
        body_rows,
        background: palette.surface.as_rgba_array(),
        border: palette.border_strong.as_rgba_array(),
        title_color: palette.text.as_rgba_array(),
        body_color: palette.muted.as_rgba_array(),
    })
}

/// 补全弹层行宽度量。`items` 指针与上一次度量一致时整段复用（打字重
/// 喂之外的每次 shape 不再逐行测量）；测量只发生在宿主喂入新列表之后。
pub(super) fn completion_popup_metrics(
    id: StableNodeId,
    source: &TextInputPresentationSource,
    previous: &crate::components::TextOverlayMetrics,
    style: &ComputedStyle,
    shaper: &mut impl TextShaper,
) -> Option<crate::components::TextCompletionPopupMetrics> {
    let items = source.completions.as_ref()?;
    if let Some(previous) = previous
        .completion
        .as_ref()
        .filter(|previous| Arc::ptr_eq(&previous.items, items))
    {
        return Some(previous.clone());
    }
    let mut width_of = |value: &str| -> f32 {
        shaper.horizontal_offset(
            id,
            &TextContent {
                value: value.to_owned(),
            },
            value.len(),
            style,
        )
    };
    let metrics = crate::components::TextCompletionPopupMetrics {
        items: Arc::clone(items),
        label_width: items
            .iter()
            .map(|item| width_of(&item.label))
            .fold(0.0_f32, f32::max),
        detail_width: items
            .iter()
            .map(|item| width_of(&item.detail))
            .fold(0.0_f32, f32::max),
        kind_width: items
            .iter()
            .map(|item| width_of(&item.kind_label))
            .fold(0.0_f32, f32::max),
    };
    Some(metrics)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn text_input_decorations(
    presentation: &TextInputPresentation,
    multiline: bool,
    content: LayoutBox,
    line_y: f32,
    line_height: f32,
    scroll_x: f32,
    scroll_y: f32,
) -> (Vec<LayoutBox>, Vec<LayoutBox>) {
    let field_x = |offset: f32| content.x + offset - scroll_x;
    if multiline {
        let selection = presentation
            .selection_lines
            .iter()
            .map(|selection| LayoutBox {
                x: field_x(selection.x),
                y: content.y + selection.y - scroll_y,
                width: selection.width,
                height: selection.height,
            })
            .collect();
        let preedit = presentation
            .preedit_lines
            .iter()
            .map(|preedit| LayoutBox {
                x: field_x(preedit.x),
                y: content.y + preedit.y + preedit.height - scroll_y - 2.0,
                width: preedit.width.max(1.0),
                height: 2.0,
            })
            .collect();
        (selection, preedit)
    } else {
        let selection = presentation
            .selection
            .map(|(start, end)| LayoutBox {
                x: field_x(start),
                y: line_y,
                width: (end - start).max(0.0),
                height: line_height,
            })
            .into_iter()
            .collect();
        let preedit = presentation
            .preedit
            .map(|(start, end)| LayoutBox {
                x: field_x(start),
                y: line_y + line_height - 2.0,
                width: (end - start).max(1.0),
                height: 2.0,
            })
            .into_iter()
            .collect();
        (selection, preedit)
    }
}

impl UiWorld {
    /// Shape against the last published content box when it exists so wrap
    /// height can stop or propagate LAYOUT. Unmeasured nodes stay unconstrained.
    pub(super) fn text_shape_constraints(&self, id: StableNodeId) -> crate::TextShapeConstraints {
        let source = &self.record(id).style;
        let layout = self.record(id).layout;
        let presentation = self.text_input_presentation_source(id);
        let text_input_multiline = presentation.as_ref().is_some_and(|source| source.multiline);
        let is_text_input = presentation.is_some();
        let wrap = if is_text_input {
            text_input_multiline && source.layout.text_wraps()
        } else {
            source.layout.text_wraps()
        };
        let preserve_lines = source.layout.white_space.preserve_newlines();
        let wrap_break = source.layout.text_wrap_break();
        let ellipsis = !is_text_input && source.layout.uses_text_ellipsis();
        let max_lines = (!is_text_input)
            .then(|| source.layout.resolved_line_clamp())
            .flatten();
        let measured = layout.width > 0.0 || layout.height > 0.0;
        if !measured {
            return crate::TextShapeConstraints {
                wrap,
                ellipsis,
                max_lines,
                shaping: self.text_shaping(id),
                preserve_lines,
                wrap_break,
                ..crate::TextShapeConstraints::default()
            };
        }
        let padding = self.used_layout_padding(id);
        let border = source.layout.resolved_border_edges();
        let leading_visual = match self.nodes.visual(id) {
            Some(StandardVisual::Checkbox { .. }) => 24.0,
            Some(StandardVisual::Switch { .. }) => 38.0,
            _ => 0.0,
        };
        crate::TextShapeConstraints {
            max_width: if is_text_input && !text_input_multiline {
                None
            } else {
                Some(
                    (layout.width
                        - padding.left
                        - padding.right
                        - border.left
                        - border.right
                        - leading_visual)
                        .max(0.0),
                )
            },
            max_height: (!is_text_input
                && (source
                    .layout
                    .height
                    .is_some_and(nana_ui_core::LengthSpec::is_definite_declared)
                    || source
                        .layout
                        .max_height
                        .is_some_and(nana_ui_core::LengthSpec::is_definite_declared)))
            .then(|| {
                (layout.height - padding.top - padding.bottom - border.top - border.bottom).max(0.0)
            }),
            wrap,
            ellipsis,
            max_lines,
            shaping: self.text_shaping(id),
            preserve_lines,
            wrap_break,
        }
    }
}

impl UiWorld {
    pub(super) fn text_shaping(&self, id: StableNodeId) -> crate::TextShaping {
        if self.nodes.text_input(id).is_some() {
            crate::TextShaping::Advanced
        } else {
            crate::TextShaping::Auto
        }
    }
}

impl UiWorld {
    /// minimap 行长的单条缓存：值未变（纯光标/选区同步）时复用上一次
    /// O(文档) 单趟扫描结果，避免每趟 shape 全文档重扫。
    pub(super) fn minimap_line_lengths_cached(&self, value: &str) -> Vec<u32> {
        let mut cache = self.minimap_line_lengths_cache.borrow_mut();
        if let Some((cached_value, cached_lengths)) = cache.as_ref()
            && cached_value == value
        {
            return cached_lengths.clone();
        }
        let lengths = collect_non_whitespace_line_lengths(value);
        *cache = Some((value.to_owned(), lengths.clone()));
        lengths
    }
}

impl UiWorld {
    /// 括号配对着色的单条缓存：值未变（纯光标/选区同步）时复用上一次
    /// O(n) 单趟栈扫描结果，避免每趟 shape 全文档重扫。
    pub(super) fn bracket_color_spans_cached(&self, value: &str) -> Arc<[(usize, usize, usize)]> {
        let mut cache = self.bracket_color_spans_cache.borrow_mut();
        if let Some((cached_value, cached_spans)) = cache.as_ref()
            && cached_value == value
        {
            return Arc::clone(cached_spans);
        }
        let (pairs, unmatched) = crate::text_editing::bracket_pair_colorization(value);
        let mut spans = Vec::with_capacity(pairs.len() + unmatched.len());
        spans.extend(pairs);
        spans.extend(unmatched.into_iter().map(|offset| {
            (
                offset,
                offset + 1,
                crate::components::TEXT_BRACKET_UNMATCHED_DEPTH,
            )
        }));
        spans.sort_unstable_by_key(|&(start, _, _)| start);
        let spans: Arc<[(usize, usize, usize)]> = spans.into();
        *cache = Some((value.to_owned(), Arc::clone(&spans)));
        spans
    }
}

impl UiWorld {
    pub(super) fn text_input_presentation_source(
        &self,
        id: StableNodeId,
    ) -> Option<TextInputPresentationSource> {
        let StandardVisual::TextInput {
            placeholder,
            secure,
            ..
        } = self.nodes.visual(id)?
        else {
            return None;
        };
        let state = self.nodes.text_input(id)?;
        let ime = self.nodes.ime(id);
        let multiline = self
            .nodes
            .get(id)
            .is_some_and(|node| node.accessibility.multiline);
        let extras = match self.nodes.visual(id) {
            Some(StandardVisual::TextInput {
                diagnostics,
                matches,
                color_swatches,
                line_numbers,
                indent_guides,
                git_marks,
                editor_options,
                ..
            }) => TextInputEditorExtras {
                diagnostics: Arc::clone(diagnostics),
                matches: Arc::clone(matches),
                color_swatches: Arc::clone(color_swatches),
                line_numbers: *line_numbers,
                indent_guides: indent_guides.clone(),
                git_marks: Arc::clone(git_marks),
                editor: editor_options.clone(),
            },
            _ => TextInputEditorExtras::default(),
        };
        // 焦点随 source 下发：出现高亮只在聚焦编辑器上派生，未聚焦的
        // 多行编辑器零分配跳过整条扫描路径。
        let focused = self.input.focused.get(&self.record(id).document) == Some(&id);
        // 折叠显示视图：仅多行态且有折叠态区间时构建（空集合零成本短路）。
        let fold = if multiline {
            self.text_display_view(id)
        } else {
            None
        };
        // 锚定浮层输入：补全候选与 hover 文档相互独立（仅多行编辑器；
        // 单行字段没有浮层）。hover 文档按值克隆进 presentation source：
        // source 是所有权结构，被 shape 约束、几何派生与测试多处消费，
        // 引用化会把生命周期串进所有构造点；克隆仅在宿主喂入 hover 期间
        // 发生，成本与浮窗文档自身同阶。
        let (completions, hover) = if multiline {
            (
                self.nodes
                    .text_completion_view(id)
                    .map(|state| state.items.clone()),
                self.nodes.text_hover_view(id).map(|h| h.doc.clone()),
            )
        } else {
            (None, None)
        };
        let mut source = build_text_input_presentation_source(
            state,
            ime,
            placeholder,
            *secure,
            multiline,
            extras,
            focused,
            fold,
            completions,
            hover,
        );
        // minimap 行长：编辑器选项归默认（占位符/IME 组合态）或多行关闭
        // 时零扫描短路；开启时按原始值收集并走值等值缓存。
        if source.multiline && source.editor.minimap {
            source.minimap_line_lengths = self.minimap_line_lengths_cached(&state.value);
        }
        // 括号配对着色：占位符与 IME 组合态没有真实可着色文档（组合期
        // 偏移漂移），保持空表；其余多行态按显示值收集并走值等值缓存。
        if source.multiline
            && source.editor.bracket_pair_colors
            && !source.placeholder
            && source.preedit.is_none()
        {
            source.bracket_color_spans = self.bracket_color_spans_cached(&source.text.value);
        }
        Some(source)
    }
}

impl UiWorld {
    /// Publishes the shaped title/message block and reports whether anything
    /// changed, so a pass that only re-confirms the current block stays idle.
    pub(super) fn apply_empty_state_text_presentation(
        &mut self,
        id: StableNodeId,
        presentation: EmptyStateTextPresentation,
    ) -> bool {
        let mut changed = false;
        if self.nodes.empty_state_text(id) != Some(&presentation) {
            self.nodes.set_empty_state_text(id, Some(presentation));
            changed = true;
        }
        let Some(StandardVisual::EmptyState {
            icon,
            compact,
            action,
            ..
        }) = self.nodes.visual(id)
        else {
            return changed;
        };
        let spacing = if *compact { 2.0 } else { 6.0 };
        let vertical = if *compact { 8.0 } else { 24.0 };
        let mut height = presentation.title.height;
        if icon.is_some() {
            height += 22.0 + spacing;
        }
        if let Some(message) = presentation.message {
            height += spacing + message.height;
        }
        if action.is_some() {
            height += spacing + 4.0;
        }
        let padding_top = nana_ui_core::LengthSpec::Px(vertical + height);
        let mut style = self.record(id).style.clone();
        if style.layout.padding_top != Some(padding_top) {
            Arc::make_mut(&mut style.layout).padding_top = Some(padding_top);
            self.record_mut(id).style = style;
            self.mark(id, DirtyMask::LAYOUT | DirtyMask::RENDER);
            if let Some(parent) = self.node(id).and_then(|node| node.parent) {
                self.mark_ancestors(parent, DirtyMask::LAYOUT | DirtyMask::RENDER);
            }
            changed = true;
        }
        changed
    }
}

impl UiWorld {
    pub(super) fn shape_text_for_layout_impl(
        &mut self,
        ids: Vec<StableNodeId>,
        shaper: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        // Same production adapter as [`Self::shape_text`].
        let mut cache = std::mem::take(&mut self.text_layout_cache);
        let mut glyphs = std::mem::take(&mut self.glyph_cache);
        let mut shaper = CountingShaper::new(shaper, &mut cache, &mut glyphs);
        let mut shaped = Vec::new();
        let mut empty_shaped = Vec::new();
        let mut modal_shaped = Vec::new();
        for id in ids {
            let presentation = self.text_input_presentation_source(id);
            let text = presentation.as_ref().map_or_else(
                || self.record(id).text.clone(),
                |source| source.text.clone(),
            );
            self.record_string_clone(text.value.len());
            let computed = self.record(id).resolved.0.as_ref();
            if let Some(visual @ StandardVisual::EmptyState { compact, .. }) = self.nodes.visual(id)
            {
                if computed.visible {
                    let layout = self.record(id).layout;
                    let horizontal = if *compact { 6.0 } else { 16.0 };
                    let width = (layout.width - horizontal * 2.0).max(0.0);
                    let intrinsic =
                        shape_empty_state_text(id, visual, computed, Some(width), &mut shaper);
                    validate_text_metrics(id, intrinsic.title)?;
                    if let Some(message) = intrinsic.message {
                        validate_text_metrics(id, message)?;
                    }
                    // The shaped block is republished even when the metrics
                    // are unchanged: it lives in `NodeStyle`, which
                    // `EmptyState::project` rewrites from its own static
                    // style, so an unrelated re-projection can drop it.
                    empty_shaped.push((id, intrinsic));
                }
                continue;
            }
            if let Some(visual @ StandardVisual::ModalFrame { kind, slots, .. }) =
                self.nodes.visual(id)
            {
                if computed.visible {
                    let root = self.record(id).layout;
                    let surface = crate::overlay_surfaces::modal_surface_bounds(root, *kind, None);
                    let chrome = crate::overlay_surfaces::ModalChrome::measure(
                        *kind,
                        crate::TextMetrics::default(),
                        None,
                        slots.close_action.is_some(),
                        slots.footer.is_some() || !slots.actions.is_empty(),
                    );
                    let wrap_width =
                        chrome.text_width(surface.width, *kind, slots.close_action.is_some());
                    let intrinsic =
                        shape_modal_text(id, visual, computed, Some(wrap_width), &mut shaper);
                    validate_text_metrics(id, intrinsic.title)?;
                    if let Some(description) = intrinsic.description {
                        validate_text_metrics(id, description)?;
                    }
                    if let Some(body) = intrinsic.body {
                        validate_text_metrics(id, body)?;
                    }
                    if self.nodes.modal_text(id) != Some(&intrinsic) {
                        modal_shaped.push((id, intrinsic));
                    }
                }
                continue;
            }
            if text.value.is_empty() || !computed.visible {
                continue;
            }
            let constraints = self.text_shape_constraints(id);
            let metrics = shaper.shape(id, &text, computed, constraints);
            validate_text_metrics(id, metrics)?;
            let previous_overlays = self
                .nodes
                .text_input_presentation(id)
                .map(|stored| stored.overlay_metrics.clone())
                .unwrap_or_default();
            let presentation = presentation.map(|source| {
                shape_text_input_presentation(
                    id,
                    source,
                    computed,
                    constraints,
                    &previous_overlays,
                    &mut shaper,
                )
            });
            if self.record(id).text_metrics != metrics
                || presentation
                    .as_ref()
                    .is_some_and(|value| self.nodes.text_input_presentation(id) != Some(value))
            {
                shaped.push((id, metrics, presentation));
            }
        }
        let mut changed = !shaped.is_empty() || !modal_shaped.is_empty();
        for (id, metrics, presentation) in shaped {
            self.record_mut(id).text_metrics = metrics;
            if let Some(presentation) = presentation {
                self.nodes
                    .set_text_input_presentation(id, Some(presentation));
            }
        }
        for (id, presentation) in empty_shaped {
            changed |= self.apply_empty_state_text_presentation(id, presentation);
        }
        for (id, presentation) in modal_shaped {
            self.nodes.set_modal_text(id, Some(presentation));
            self.mark(id, DirtyMask::LAYOUT | DirtyMask::RENDER);
        }
        let runs = shaper.runs;
        let wrap_layouts = shaper.wrap_layouts;
        let _shaper = shaper;
        let (hits, misses, evictions) = cache.take_counters();
        let glyph_stats = glyphs.take_counters();
        self.text_layout_cache = cache;
        self.glyph_cache = glyphs;
        self.bump_last_counters(|counters| {
            counters.record_text_shape(runs, hits, misses, wrap_layouts);
            counters.record_cache_eviction(evictions);
            if let Some((glyph_hits, glyph_misses)) = glyph_stats {
                counters.record_glyph_cache(glyph_hits, glyph_misses);
            }
        });
        Ok(changed)
    }
}

impl UiWorld {
    /// Nodes currently carrying a LAYOUT-dirty bit that has not been drained
    /// by [`Self::take_system_work`] — e.g. marked by a shaping pass between
    /// drains. Sorted for determinism.
    pub fn pending_layout_dirty(&self) -> Vec<StableNodeId> {
        let mut ids = self
            .dirty_entities
            .iter()
            .copied()
            .filter(|id| {
                self.nodes
                    .get(*id)
                    .is_some_and(|node| node.dirty.has(DirtyMask::LAYOUT))
            })
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }
}

impl UiWorld {
    /// [`Self::shape_text_for_layout`] restricted to `ids` (typically the
    /// relayout scope plus nodes whose published box changed). Nodes outside
    /// the scope keep their previous shape, which already matches their
    /// unchanged constraints.
    pub fn shape_text_for_layout_scoped(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        let mut scope = ids.to_vec();
        scope.sort_unstable();
        scope.dedup();
        scope.retain(|id| self.nodes.contains(*id));
        self.shape_text_for_layout_impl(scope, shaper)
    }
}

impl UiWorld {
    /// Re-shape visible text against its resolved content box after the first
    /// layout pass. This closes wrapping/ellipsis height measurement without
    /// moving layout ownership into the renderer adapter.
    pub fn shape_text_for_layout(
        &mut self,
        document: DocumentId,
        shaper: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        self.shape_text_for_layout_impl(self.document_order(document), shaper)
    }
}

impl UiWorld {
    /// Shape only explicitly scheduled text. The runtime owns invalidation and
    /// storage while the renderer adapter supplies its real shaping backend.
    pub fn shape_text(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl TextShaper,
    ) -> Result<(), UiWorldError> {
        self.resolve_presentations(ids)?;
        // Production adapter: every host shaper (MeasureTextShaper, NanaTextShaper,
        // tests) is wrapped once so lookup/insert hit the same UiWorld caches.
        let mut cache = std::mem::take(&mut self.text_layout_cache);
        let mut glyphs = std::mem::take(&mut self.glyph_cache);
        let mut shaper = CountingShaper::new(shaper, &mut cache, &mut glyphs);
        if !ids.is_empty() {
            self.record_hot_path_allocation(
                1,
                ids.len().saturating_mul(size_of::<(
                    StableNodeId,
                    TextMetrics,
                    Option<TextInputPresentation>,
                )>()),
            );
        }
        let mut shaped = Vec::with_capacity(ids.len());
        let mut empty_shaped = Vec::new();
        let mut modal_shaped = Vec::new();
        for &id in ids {
            if !self.contains(id) {
                let _shaper = shaper;
                self.text_layout_cache = cache;
                self.glyph_cache = glyphs;
                return Err(UiWorldError::MissingNode(id));
            }
            let presentation = self.text_input_presentation_source(id);
            let text = presentation.as_ref().map_or_else(
                || self.record(id).text.clone(),
                |source| source.text.clone(),
            );
            self.record_string_clone(text.value.len());
            let style = self.record(id).resolved.0.as_ref().clone();
            if let Some(visual @ StandardVisual::EmptyState { .. }) = self.nodes.visual(id) {
                let intrinsic = shape_empty_state_text(id, visual, &style, None, &mut shaper);
                validate_text_metrics(id, intrinsic.title)?;
                if let Some(message) = intrinsic.message {
                    validate_text_metrics(id, message)?;
                }
                empty_shaped.push((id, intrinsic));
            }
            if let Some(visual @ StandardVisual::ModalFrame { .. }) = self.nodes.visual(id) {
                let intrinsic = shape_modal_text(id, visual, &style, None, &mut shaper);
                validate_text_metrics(id, intrinsic.title)?;
                if let Some(description) = intrinsic.description {
                    validate_text_metrics(id, description)?;
                }
                if let Some(body) = intrinsic.body {
                    validate_text_metrics(id, body)?;
                }
                modal_shaped.push((id, intrinsic));
            }
            let constraints = self.text_shape_constraints(id);
            let metrics = shaper.shape(id, &text, &style, constraints);
            validate_text_metrics(id, metrics)?;
            let previous_overlays = self
                .nodes
                .text_input_presentation(id)
                .map(|stored| stored.overlay_metrics.clone())
                .unwrap_or_default();
            let presentation = presentation.map(|source| {
                shape_text_input_presentation(
                    id,
                    source,
                    &style,
                    constraints,
                    &previous_overlays,
                    &mut shaper,
                )
            });
            shaped.push((id, metrics, presentation));
        }
        for (id, metrics, presentation) in shaped {
            let previous = self.record(id).text_metrics;
            self.record_mut(id).text_metrics = metrics;
            if let Some(presentation) = presentation {
                // minimap 视口钉住随光标移动失效：reveal 恢复权威。上一趟
                // 与本趟 shape 的光标位置不同即视为移动。
                if self.nodes.text_viewport_pin(id).is_some() {
                    let moved = match self.nodes.text_input_presentation(id) {
                        Some(previous_presentation) => {
                            (previous_presentation.caret_x, previous_presentation.caret_y)
                                != (presentation.caret_x, presentation.caret_y)
                        }
                        None => true,
                    };
                    if moved {
                        self.nodes.set_text_viewport_pin(id, None);
                    }
                }
                self.nodes
                    .set_text_input_presentation(id, Some(presentation));
            }
            if text_intrinsic_changed(previous, metrics) {
                self.propagate_layout_from_node(id);
            }
        }
        for (id, presentation) in empty_shaped {
            self.apply_empty_state_text_presentation(id, presentation);
        }
        for (id, presentation) in modal_shaped {
            self.nodes.set_modal_text(id, Some(presentation));
        }
        let runs = shaper.runs;
        let wrap_layouts = shaper.wrap_layouts;
        let _shaper = shaper;
        let (hits, misses, evictions) = cache.take_counters();
        let glyph_stats = glyphs.take_counters();
        self.text_layout_cache = cache;
        self.glyph_cache = glyphs;
        self.bump_last_counters(|counters| {
            counters.record_text_shape(runs, hits, misses, wrap_layouts);
            counters.record_cache_eviction(evictions);
            if let Some((glyph_hits, glyph_misses)) = glyph_stats {
                counters.record_glyph_cache(glyph_hits, glyph_misses);
            }
        });
        Ok(())
    }
}

impl UiWorld {
    /// Content box and scroll offset used to map pointer coordinates onto
    /// text-input byte offsets, mirroring the paint-side `field_x`/`line_y`.
    pub fn text_input_pointer_context(
        &self,
        id: StableNodeId,
    ) -> Option<(LayoutBox, ScrollOffset)> {
        let node = self.nodes.get(id)?;
        if !matches!(
            self.nodes.visual(id),
            Some(StandardVisual::TextInput { .. })
        ) {
            return None;
        }
        let padding = self.used_layout_padding(id);
        let border = node.style.layout.resolved_border_width();
        let content = LayoutBox {
            x: node.layout.x + border + padding.left,
            y: node.layout.y + border + padding.top,
            width: (node.layout.width - border * 2.0 - padding.left - padding.right).max(0.0),
            height: (node.layout.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
        };
        Some((content, self.record(id).scroll_offset))
    }
}

impl UiWorld {
    /// Shape inputs for text-input geometry queries: resolved style and the
    /// constraints the last layout pass shaped with.
    pub fn text_input_shape_context(
        &self,
        id: StableNodeId,
    ) -> Option<(ComputedStyle, crate::TextShapeConstraints)> {
        if !matches!(
            self.nodes.visual(id),
            Some(StandardVisual::TextInput { .. })
        ) {
            return None;
        }
        let node = self.nodes.get(id)?;
        Some((
            node.resolved.0.as_ref().clone(),
            self.text_shape_constraints(id),
        ))
    }
}

impl UiWorld {
    /// 值被编辑后重映射折叠态与 snippet 会话。
    pub(super) fn reconcile_text_view_state(
        &mut self,
        id: StableNodeId,
        old_value: &str,
        new_value: &str,
    ) {
        let (changed_start, changed_end, delta) = value_edit_span(old_value, new_value);
        if let Some(entry) = self.nodes.text_fold_view(id).cloned() {
            let next = remap_collapsed_after_edit(
                &entry.collapsed,
                new_value,
                changed_start,
                changed_end,
                delta,
            );
            if next != entry.collapsed {
                self.nodes.set_text_fold_view(
                    id,
                    Some(crate::store::TextFoldViewState {
                        offered: entry.offered,
                        collapsed: next,
                    }),
                );
                self.mark(id, DirtyMask::TEXT | DirtyMask::RENDER);
            }
        }
        if let Some(session) = self.nodes.text_snippet_session(id).cloned() {
            match remap_snippet_session(&session, new_value, changed_start, changed_end, delta) {
                Some(next) if next != session => {
                    self.nodes.set_text_snippet_session(id, Some(next));
                }
                Some(_) => {}
                None => self.nodes.set_text_snippet_session(id, None),
            }
        }
    }
}

impl UiWorld {
    /// 宿主重喂折叠区间后对账折叠态（`offered` 为 `None` 表示宿主不再
    /// 喂入折叠，整个视图状态随之移除）。
    pub(super) fn reconcile_text_fold_offered(
        &mut self,
        id: StableNodeId,
        offered: Option<Arc<[crate::TextCodeFold]>>,
    ) {
        let Some(offered) = offered else {
            if self.nodes.text_fold_view(id).is_some() {
                self.nodes.set_text_fold_view(id, None);
                self.mark(id, DirtyMask::TEXT | DirtyMask::RENDER);
            }
            return;
        };
        let next = match self.nodes.text_fold_view(id) {
            Some(entry) => {
                let collapsed =
                    reconcile_collapsed_folds(&entry.offered, &offered, &entry.collapsed);
                if collapsed == entry.collapsed && entry.offered == offered {
                    return;
                }
                collapsed
            }
            None => Vec::new(),
        };
        self.nodes.set_text_fold_view(
            id,
            Some(crate::store::TextFoldViewState {
                offered,
                collapsed: next,
            }),
        );
        self.mark(id, DirtyMask::TEXT | DirtyMask::RENDER);
    }
}

impl UiWorld {
    /// 滚轮落点命中的 hover 浮窗面板所属节点。命中测试驱动：hover 显示
    /// 不要求焦点，任意文档内编辑器的浮窗面板都可能被滚动。重叠时取最小
    /// 节点 id 保证稳定结果（浮层各自锚定自己的编辑器，正常不重叠）。
    pub(crate) fn text_hover_panel_at(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Option<StableNodeId> {
        let mut hits: Vec<StableNodeId> = self
            .nodes
            .text_hover_ids()
            .filter(|&id| {
                self.nodes
                    .get(id)
                    .is_some_and(|node| node.document == document)
            })
            .filter(|&id| self.text_hover_panel_hit(id, x, y))
            .collect();
        hits.sort_unstable();
        hits.first().copied()
    }
}

impl UiWorld {
    /// 指针是否落在 hover 浮窗面板上（含内边距）。坐标为节点空间。
    pub fn text_hover_panel_hit(&self, id: StableNodeId, x: f32, y: f32) -> bool {
        let Some(crate::ComponentGeometry::TextInput { hover_popup, .. }) =
            self.component_geometry(id)
        else {
            return false;
        };
        hover_popup
            .as_ref()
            .is_some_and(|popup| popup.panel.contains(x, y))
    }
}

impl UiWorld {
    /// 指针是否落在补全弹层面板上（含内边距）。坐标为节点空间；滚轮
    /// 路由用它把弹层内滚动与编辑器滚动分开。
    pub fn text_completion_panel_hit(&self, id: StableNodeId, x: f32, y: f32) -> bool {
        let Some(crate::ComponentGeometry::TextInput {
            completion_popup, ..
        }) = self.component_geometry(id)
        else {
            return false;
        };
        completion_popup
            .as_ref()
            .is_some_and(|popup| popup.panel.contains(x, y))
    }
}

impl UiWorld {
    /// 命中补全弹层的候选行，返回该候选的绝对下标。弹层绘制在折叠
    /// 之上，调用方（框架指针路径）应先于折叠命中查询。坐标为节点空间。
    pub fn text_completion_hit(&self, id: StableNodeId, x: f32, y: f32) -> Option<usize> {
        match self.component_geometry(id)? {
            crate::ComponentGeometry::TextInput {
                completion_popup, ..
            } => {
                let popup = completion_popup.as_ref()?;
                let row = popup
                    .rows
                    .iter()
                    .position(|row| row.bounds.contains(x, y))?;
                Some(popup.first_row + row)
            }
            _ => None,
        }
    }
}

impl UiWorld {
    /// 命中折叠交互区域（gutter 箭头优先，其次折叠起始行的摘要标记），
    /// 返回对应折叠区间。坐标为节点空间。
    pub fn text_fold_hit(&self, id: StableNodeId, x: f32, y: f32) -> Option<crate::TextCodeFold> {
        match self.component_geometry(id)? {
            crate::ComponentGeometry::TextInput { folds, .. } => folds
                .gutters
                .iter()
                .find(|gutter| gutter.bounds.contains(x, y))
                .map(|gutter| gutter.fold)
                .or_else(|| {
                    folds
                        .markers
                        .iter()
                        .find(|marker| marker.bounds.contains(x, y))
                        .map(|marker| marker.fold)
                }),
            _ => None,
        }
    }
}

impl UiWorld {
    /// hover 浮窗正文的滚动行数（宿主查询入口）；未喂入 hover 时为 0。
    pub fn text_hover_scroll(&self, id: StableNodeId) -> usize {
        self.nodes
            .text_hover_view(id)
            .map(|state| state.scroll)
            .unwrap_or(0)
    }
}

impl UiWorld {
    /// 补全会话只读快照（宿主查询入口）。无会话时为 `None`。
    pub fn text_completion_snapshot(
        &self,
        id: StableNodeId,
    ) -> Option<crate::TextCompletionSnapshot> {
        self.nodes
            .text_completion_view(id)
            .map(|state| crate::TextCompletionSnapshot {
                count: state.items.len(),
                selected: state.selected,
                scroll: state.scroll,
                dismissed: state.dismissed,
            })
    }
}

impl UiWorld {
    /// 当前喂入的 hover 文档（供组件投影做喂入去重）。
    pub(crate) fn text_hover_doc(&self, id: StableNodeId) -> Option<&crate::TextHover> {
        self.nodes.text_hover_view(id).map(|state| &state.doc)
    }
}

impl UiWorld {
    /// hover 浮窗状态快照。
    pub(crate) fn text_hover_view(
        &self,
        id: StableNodeId,
    ) -> Option<crate::store::TextHoverViewState> {
        self.nodes.text_hover_view(id).cloned()
    }
}

impl UiWorld {
    /// 当前喂入的补全候选（供组件投影做喂入去重）。
    pub(crate) fn text_completion_items(
        &self,
        id: StableNodeId,
    ) -> Option<&Arc<[crate::TextCompletion]>> {
        self.nodes
            .text_completion_view(id)
            .map(|state| &state.items)
    }
}

impl UiWorld {
    /// 补全弹层会话快照（供框架命令读取-修改-写回与几何层读取）。
    pub(crate) fn text_completion_view(
        &self,
        id: StableNodeId,
    ) -> Option<crate::store::TextCompletionViewState> {
        self.nodes.text_completion_view(id).cloned()
    }
}

impl UiWorld {
    /// snippet 会话快照。
    pub(crate) fn text_snippet_session(
        &self,
        id: StableNodeId,
    ) -> Option<crate::components::TextSnippetSession> {
        self.nodes.text_snippet_session(id).cloned()
    }
}

impl UiWorld {
    /// 折叠视图状态快照（供框架命令读取-修改-写回）。
    pub(crate) fn text_fold_view_state(
        &self,
        id: StableNodeId,
    ) -> Option<crate::store::TextFoldViewState> {
        self.nodes.text_fold_view(id).cloned()
    }
}

impl UiWorld {
    /// 折叠后的显示视图；没有折叠态区间时 `None`（零分配短路）。
    pub(crate) fn text_display_view(&self, id: StableNodeId) -> Option<TextDisplayView> {
        let state = self.nodes.text_input(id)?;
        let entry = self.nodes.text_fold_view(id)?;
        build_text_display_view(&state.value, &entry.collapsed)
    }
}

impl UiWorld {
    /// 当前折叠态的区间（值空间，按 `start` 排序）。没有折叠视图的节点
    /// 返回空表。宿主测试与状态面板的查询入口。
    pub fn text_fold_collapsed(&self, id: StableNodeId) -> Vec<crate::TextCodeFold> {
        self.nodes
            .text_fold_view(id)
            .map(|entry| entry.collapsed.clone())
            .unwrap_or_default()
    }
}

impl UiWorld {
    /// 当前落点指示线（只读，提取层翻译为节点空间图元）。
    pub(crate) fn text_drop_indicator(&self, id: StableNodeId) -> Option<LayoutBox> {
        self.nodes.text_drop_indicator(id).copied()
    }
}

impl UiWorld {
    /// 拖拽移动选中文本的落点指示线（框架侧拖拽状态机写入；文本空间
    /// 矩形）。`None` 清除指示线。
    pub(crate) fn set_text_drop_indicator(&mut self, id: StableNodeId, rect: Option<LayoutBox>) {
        self.nodes.set_text_drop_indicator(id, rect);
    }
}

impl UiWorld {
    /// 写入/清除 minimap 视口钉住。
    pub(crate) fn set_text_viewport_pin(
        &mut self,
        id: StableNodeId,
        pin: Option<crate::store::TextViewportPin>,
    ) {
        self.nodes.set_text_viewport_pin(id, pin);
    }
}

impl UiWorld {
    /// minimap 视口钉住快照（框架导航路径读取-写入）。
    pub(crate) fn text_viewport_pin(
        &self,
        id: StableNodeId,
    ) -> Option<crate::store::TextViewportPin> {
        self.nodes.text_viewport_pin(id).copied()
    }
}

impl UiWorld {
    /// minimap 导航换算：条内点击点 → 目标滚动偏移（点击行在视口居中，
    /// 钳到文档范围；横向偏移保持不变）。不在条内或编辑器无 minimap 时
    /// `None`。只读查询：调用方（框架指针路径）负责写回。
    pub fn text_minimap_scroll_target(
        &self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<ScrollOffset> {
        let Some(crate::ComponentGeometry::TextInput {
            minimap: Some(minimap),
            ..
        }) = self.component_geometry(id)
        else {
            return None;
        };
        if !minimap.panel.contains(x, y) {
            return None;
        }
        let line = minimap.line_at(y)?;
        let presentation = self.nodes.text_input_presentation(id)?;
        let line_height = presentation.line_height.max(1.0);
        let node = self.nodes.get(id)?;
        let padding = self.used_layout_padding(id);
        let border = node.style.layout.resolved_border_width();
        let content_height =
            (node.layout.height - border * 2.0 - padding.top - padding.bottom).max(0.0);
        // 行数与滚动空间同源（逻辑行数 × 行高，软折行低估为已知限制）。
        let total_height = minimap.line_count as f32 * line_height;
        let max_scroll = (total_height - content_height).max(0.0);
        let centered = line as f32 * line_height + line_height * 0.5 - content_height * 0.5;
        Some(ScrollOffset {
            x: self.record(id).scroll_offset.x,
            y: centered.clamp(0.0, max_scroll),
        })
    }
}

impl UiWorld {
    /// 计算使多行文本输入内 `offset` 所在逻辑行进入可视区所需的滚动偏移。
    /// 只读查询：不改世界状态；宿主将返回值写回组件的 `scroll_offset`。
    /// 行高按逻辑行均匀假设（忽略软折行），定位场景下足够精确。
    ///
    /// 折叠感知：存在折叠态区间时按显示视图计算行号与总高；被折叠隐藏
    /// 的偏移钳制到折叠起始行。查找导航到折叠内匹配时的自动展开由框架
    /// 命令负责（reveal 的展开语义），本查询只做几何换算。
    pub fn text_input_reveal_scroll(
        &self,
        id: StableNodeId,
        offset: usize,
    ) -> Option<ScrollOffset> {
        let state = self.nodes.text_input(id)?;
        if !self.nodes.get(id)?.accessibility.multiline {
            return None;
        }
        if !matches!(
            self.nodes.visual(id),
            Some(StandardVisual::TextInput { .. })
        ) {
            return None;
        }
        let presentation = self.nodes.text_input_presentation(id)?;
        let line_height = presentation.line_height.max(1.0);
        let offset = offset.min(state.value.len());
        let value = state.value.as_str();
        // 折叠态：显示视图内的行号才是渲染行号；隐藏偏移钳到折叠起始行。
        let (display_value, display_offset) = match self.text_display_view(id) {
            Some(view) => {
                let display_offset = view.display_of(offset).min(view.value.len());
                (view.value, display_offset)
            }
            None => (value.to_owned(), offset),
        };
        let line_index = display_value[..display_offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count() as f32;
        let reveal_y = line_index * line_height;
        let node = self.nodes.get(id)?;
        let padding = self.used_layout_padding(id);
        let border = node.style.layout.resolved_border_width();
        let content_height =
            (node.layout.height - border * 2.0 - padding.top - padding.bottom).max(0.0);
        // 逻辑行数 × 行高 = 无折行下的内容总高；软折行场景会低估（已知限制）。
        let total_height = (display_value.matches('\n').count() + 1) as f32 * line_height;
        let max_scroll = (total_height - content_height).max(0.0);
        let mut scroll_y = self.record(id).scroll_offset.y;
        if reveal_y < scroll_y {
            scroll_y = reveal_y;
        } else if reveal_y + line_height > scroll_y + content_height {
            scroll_y = reveal_y + line_height - content_height;
        }
        Some(ScrollOffset {
            x: self.record(id).scroll_offset.x,
            y: scroll_y.clamp(0.0, max_scroll),
        })
    }
}
