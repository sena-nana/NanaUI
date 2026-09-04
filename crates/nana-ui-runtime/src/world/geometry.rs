//! Component geometry derived from committed Runtime data.

#[cfg(feature = "calendar")]
mod calendar;
#[cfg(feature = "calendar")]
pub(super) use calendar::*;

#[cfg(feature = "charts")]
mod charts;
#[cfg(feature = "charts")]
pub(super) use charts::*;

#[cfg(feature = "controls")]
mod controls;
#[cfg(feature = "controls")]
use controls::*;

#[cfg(feature = "rich-text")]
mod rich_text;
#[cfg(feature = "rich-text")]
use rich_text::*;

#[cfg(feature = "graph-canvas")]
mod graph_canvas;
#[cfg(feature = "graph-canvas")]
pub(super) use graph_canvas::*;

#[cfg(feature = "image-viewer")]
mod image_viewer;
#[cfg(feature = "image-viewer")]
use image_viewer::*;

use super::*;

pub(super) fn key_capture_geometry(
    content: LayoutBox,
    recording: bool,
    palette: &SemanticPalette,
) -> crate::ComponentGeometry {
    let label: Arc<str> = if recording {
        Arc::from("Recording")
    } else {
        Arc::from("Idle")
    };
    crate::ComponentGeometry::KeyCaptureLayer {
        badge: key_badge_region(content, &label, !recording, palette),
        background: Some(if recording {
            palette.accent_soft.as_rgba_array()
        } else {
            palette.subtle.as_rgba_array()
        }),
    }
}

pub(super) fn keymap_geometry(
    content: LayoutBox,
    palette: &SemanticPalette,
) -> crate::ComponentGeometry {
    crate::ComponentGeometry::KeymapLayer {
        badge: key_badge_region(content, "Keymap", false, palette),
    }
}

pub(super) fn key_badge_region(
    origin: LayoutBox,
    label: &str,
    muted: bool,
    palette: &SemanticPalette,
) -> crate::ComponentTextRegion {
    const HEIGHT: f32 = 28.0;
    const PAD: f32 = 8.0;
    let font_size = 12.0;
    crate::ComponentTextRegion {
        bounds: LayoutBox {
            x: origin.x,
            y: origin.y,
            width: (estimated_text_width(label, font_size) + PAD * 2.0).max(64.0),
            height: HEIGHT.min(origin.height.max(HEIGHT)),
        },
        content: Arc::from(label),
        color: Some(if muted {
            palette.muted.as_rgba_array()
        } else {
            palette.text.as_rgba_array()
        }),
        font_size,
        font_weight: Some(600),
    }
}

pub(super) fn estimated_text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|ch| {
            if ch.is_ascii() {
                font_size * 0.62
            } else {
                font_size
            }
        })
        .sum::<f32>()
        .max(font_size)
}

#[cfg(feature = "charts")]
pub(super) fn area_under_polyline(points: &[[f32; 2]], baseline: f32) -> Vec<LayoutBox> {
    const STRIP: f32 = 2.0;
    let mut strips = Vec::new();
    for pair in points.windows(2) {
        let [x0, y0] = pair[0];
        let [x1, y1] = pair[1];
        let span = x1 - x0;
        if !span.is_finite() || span.abs() < f32::EPSILON {
            continue;
        }
        let left = x0.min(x1);
        let right = x0.max(x1);
        let mut x = left;
        while x < right {
            let width = STRIP.min(right - x);
            let mid = x + width * 0.5;
            let t = (mid - x0) / span;
            let y = y0 + (y1 - y0) * t;
            let top = y.min(baseline);
            let height = (baseline - top).max(0.0);
            if height > 0.0 {
                strips.push(LayoutBox {
                    x,
                    y: top,
                    width,
                    height,
                });
            }
            x += STRIP;
        }
    }
    strips
}

impl UiWorld {
    pub(super) fn derive_component_geometry(
        &self,
        id: StableNodeId,
        visual: &StandardVisual,
        style: &ComputedStyle,
    ) -> Option<crate::ComponentGeometry> {
        let (bounds, source) = {
            let node = self.nodes.get(id)?;
            (node.layout, node.style.clone())
        };
        let padding = self.used_layout_padding(id);
        let border = source.layout.resolved_border_width();
        let content = LayoutBox {
            x: bounds.x + border + padding.left,
            y: bounds.y + border + padding.top,
            width: (bounds.width - border * 2.0 - padding.left - padding.right).max(0.0),
            height: (bounds.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
        };
        let text_region = |bounds, content: Arc<str>, muted: bool, size: f32, weight| {
            crate::ComponentTextRegion {
                bounds,
                content,
                color: Some(if muted {
                    self.style_model.palette.muted.as_rgba_array()
                } else {
                    style
                        .color
                        .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array())
                }),
                font_size: size,
                font_weight: weight,
            }
        };
        match visual {
            StandardVisual::ModalFrame {
                title,
                description,
                body_text,
                kind,
                slots,
                ..
            } => {
                let presentation = self.nodes.modal_text(id).copied().unwrap_or_default();
                let has_close = slots.close_action.is_some();
                let has_footer = slots.footer.is_some() || !slots.actions.is_empty();
                let chrome = crate::overlay_surfaces::ModalChrome::measure(
                    *kind,
                    presentation.title,
                    presentation.description,
                    has_close,
                    has_footer,
                );
                let body_copy = presentation.body.map_or(0.0, |metrics| metrics.height);
                let body_slot = slots
                    .body
                    .and_then(|id| self.layout_box(id))
                    .map_or(0.0, |region| region.height);
                let body_gap = if body_copy > 0.0 && body_slot > 0.0 {
                    8.0
                } else {
                    0.0
                };
                let intrinsic_height = chrome.chrome_height(body_copy + body_gap + body_slot);
                let surface = crate::overlay_surfaces::modal_surface_bounds(
                    bounds,
                    *kind,
                    Some(intrinsic_height),
                );
                let LayoutBox { x, y, width, .. } = surface;
                let text_width = chrome.text_width(width, *kind, has_close);
                let body = chrome.body_box(surface);
                let text_block = presentation.title.height
                    + presentation.description.map_or(0.0, |metrics| {
                        crate::overlay_surfaces::MODAL_TITLE_DESC_GAP + metrics.height
                    });
                let title_y = match kind {
                    crate::ModalSurfaceKind::Drawer(_) => {
                        y + (chrome.header_height - text_block) / 2.0
                    }
                    _ => y + crate::overlay_surfaces::MODAL_HEADER_PAD_TOP,
                };
                let shadow_alpha = if self.style_model.palette.background.as_rgba_array()[0] > 0.5 {
                    0.28
                } else {
                    0.45
                };
                Some(crate::ComponentGeometry::ModalFrame {
                    scrim: bounds,
                    surface,
                    body,
                    title: text_region(
                        LayoutBox {
                            x: x + chrome.pad_x,
                            y: title_y,
                            width: text_width,
                            height: presentation.title.height,
                        },
                        Arc::clone(title),
                        false,
                        14.0,
                        Some(600),
                    ),
                    description: description.as_ref().map(|description| {
                        text_region(
                            LayoutBox {
                                x: x + chrome.pad_x,
                                y: title_y
                                    + presentation.title.height
                                    + crate::overlay_surfaces::MODAL_TITLE_DESC_GAP,
                                width: text_width,
                                height: presentation.description.unwrap_or_default().height,
                            },
                            Arc::clone(description),
                            true,
                            12.0,
                            None,
                        )
                    }),
                    body_text: body_text.as_ref().map(|message| {
                        text_region(
                            LayoutBox {
                                x: body.x,
                                y: body.y,
                                width: body.width,
                                height: presentation.body.unwrap_or_default().height,
                            },
                            Arc::clone(message),
                            false,
                            crate::overlay_surfaces::MODAL_BODY_TEXT_SIZE,
                            None,
                        )
                    }),
                    background: self.style_model.palette.surface.as_rgba_array(),
                    border: [0.0; 4],
                    elevation: crate::ComponentElevation {
                        color: [0.0, 0.0, 0.0, shadow_alpha],
                        offset_x: 0.0,
                        offset_y: 14.0,
                        blur_radius: 30.0,
                        spread_radius: 0.0,
                        inset: false,
                    },
                })
            }
            StandardVisual::Button {
                label,
                size,
                loading,
                ..
            } => {
                // Loading reserves 20px through symmetric intrinsic padding in
                // the layout pass. That reservation grows the outer button; it
                // is not additional visual padding, so return it to the inline
                // content box before centering spinner + label.
                let button_content = if *loading {
                    LayoutBox {
                        x: content.x - 10.0,
                        width: content.width + 20.0,
                        ..content
                    }
                } else {
                    content
                };
                let label_width = self
                    .text_metrics(id)
                    .map_or(0.0, |metrics| metrics.width.min(button_content.width));
                let spinner_extent = size.icon_size().min(button_content.height);
                let gap = if *loading { 6.0 } else { 0.0 };
                let group_width = (label_width + if *loading { spinner_extent + gap } else { 0.0 })
                    .min(button_content.width);
                let group_x = button_content.x + (button_content.width - group_width) / 2.0;
                let spinner = (*loading).then_some(LayoutBox {
                    x: group_x,
                    y: button_content.y + (button_content.height - spinner_extent) / 2.0,
                    width: spinner_extent,
                    height: spinner_extent,
                });
                let label_x = group_x + if *loading { spinner_extent + gap } else { 0.0 };
                Some(crate::ComponentGeometry::Button {
                    label: text_region(
                        LayoutBox {
                            x: label_x,
                            y: button_content.y,
                            width: (group_x + group_width - label_x).max(0.0),
                            height: button_content.height,
                        },
                        Arc::clone(label),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    spinner,
                    background: style.background,
                    border: style.border_color,
                    border_width: if style.border_color.is_some() {
                        source.layout.resolved_border_width()
                    } else {
                        0.0
                    },
                    focus_ring: None,
                })
            }
            StandardVisual::TextInput {
                size,
                invalid,
                steppers,
                ..
            } => {
                let presentation = self.nodes.text_input_presentation(id)?;
                let accessibility = self.nodes.get(id).map(|n| &n.accessibility);
                let disabled = accessibility.is_some_and(|state| state.disabled);
                let steppers = steppers
                    .then(|| {
                        let band = (size.height() / 2.0).min(content.height / 2.0);
                        let width = size.indicator_size();
                        if band <= 0.0 || width <= 0.0 || content.width <= width {
                            return None;
                        }
                        let x = content.x + content.width - width;
                        let numeric = accessibility.map(|state| {
                            (
                                state.numeric_value.unwrap_or_default(),
                                state.numeric_minimum,
                                state.numeric_maximum,
                            )
                        });
                        let can_increment = !disabled
                            && numeric.is_none_or(|(value, _, maximum)| {
                                maximum.is_none_or(|maximum| value < maximum)
                            });
                        let can_decrement = !disabled
                            && numeric.is_none_or(|(value, minimum, _)| {
                                minimum.is_none_or(|minimum| value > minimum)
                            });
                        let active = self.style_model.palette.muted.as_rgba_array();
                        let inert = self.style_model.palette.faint.as_rgba_array();
                        Some(crate::NumberSteppers {
                            increment: LayoutBox {
                                x,
                                y: content.y + (content.height - band * 2.0) / 2.0,
                                width,
                                height: band,
                            },
                            decrement: LayoutBox {
                                x,
                                y: content.y + (content.height - band * 2.0) / 2.0 + band,
                                width,
                                height: band,
                            },
                            increment_color: if can_increment { active } else { inert },
                            decrement_color: if can_decrement { active } else { inert },
                            increment_enabled: can_increment,
                            decrement_enabled: can_decrement,
                            glyph_size: (width - 2.0).max(1.0),
                        })
                    })
                    .flatten();
                let content = match steppers {
                    Some(steppers) => LayoutBox {
                        width: (content.width - steppers.increment.width - 4.0).max(0.0),
                        ..content
                    },
                    None => content,
                };
                let focused = self.input.focused.get(&self.record(id).document) == Some(&id);
                let metrics = self.text_metrics(id).unwrap_or_default();
                let multiline = accessibility.is_some_and(|state| state.multiline);
                let requested_scroll = self.record(id).scroll_offset;
                // 多行编辑器的内容高度按逻辑行数推导；text_metrics.height
                // 是单行度量，不能作为滚动上限。
                let total_text_height = if multiline {
                    let lines = presentation.display_value.matches('\n').count() + 1;
                    lines as f32 * presentation.line_height.max(1.0)
                } else {
                    metrics.height
                };
                let mut scroll_x = if multiline {
                    requested_scroll.x
                } else {
                    (presentation.caret_x - content.width + 1.0).max(0.0)
                }
                .min((metrics.width - content.width).max(0.0));
                let line_height = if multiline {
                    presentation.line_height
                } else {
                    size.line_height()
                }
                .max(1.0)
                .min(content.height.max(1.0));
                let mut scroll_y = if multiline {
                    requested_scroll
                        .y
                        .min((total_text_height - content.height).max(0.0))
                } else {
                    0.0
                };
                if multiline && focused {
                    // minimap 视口钉住：显式导航（minimap 点击/拖动）期间
                    // 光标 reveal 让位，视口停在用户导航到的位置。宿主改写
                    // 滚动偏移或光标移动（shape 趟清除钉住）都会使钉住失效，
                    // reveal 恢复权威——未钉住时行为与既有语义完全一致。
                    let viewport_pinned = self.text_viewport_pin(id) == Some(requested_scroll);
                    if !viewport_pinned {
                        if presentation.caret_x < scroll_x {
                            scroll_x = presentation.caret_x;
                        } else if presentation.caret_x + 1.0 > scroll_x + content.width {
                            scroll_x = presentation.caret_x + 1.0 - content.width;
                        }
                        if presentation.caret_y < scroll_y {
                            scroll_y = presentation.caret_y;
                        } else if presentation.caret_y + line_height > scroll_y + content.height {
                            scroll_y = presentation.caret_y + line_height - content.height;
                        }
                    }
                }
                scroll_x = scroll_x.clamp(0.0, (metrics.width - content.width).max(0.0));
                scroll_y = scroll_y.clamp(0.0, (total_text_height - content.height).max(0.0));
                let line_y = if multiline {
                    content.y - scroll_y
                } else {
                    content.y + (content.height - line_height) / 2.0
                };
                let field_x = |offset: f32| content.x + offset - scroll_x;
                let (selection, preedit) = text_input_decorations(
                    presentation,
                    multiline,
                    content,
                    line_y,
                    line_height,
                    scroll_x,
                    scroll_y,
                );
                let caret = focused.then_some(LayoutBox {
                    x: field_x(presentation.caret_x),
                    y: line_y + presentation.caret_y,
                    width: 1.0,
                    height: line_height,
                });
                // 附加多光标：与主光标同形，用主光标色的半透明变体区分；
                // 只随焦点出现（多行编辑器才有附加光标）。
                let caret_color = style
                    .color
                    .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array());
                let additional_caret_color = {
                    let mut color = caret_color;
                    color[3] *= 0.55;
                    color
                };
                let additional_carets = if focused {
                    presentation
                        .additional_carets
                        .iter()
                        .map(|(x, y)| LayoutBox {
                            x: field_x(*x),
                            y: line_y + *y,
                            width: 1.0,
                            height: line_height,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let marker_color = |severity| match severity {
                    crate::TextDiagnosticSeverity::Error => {
                        self.style_model.palette.danger.as_rgba_array()
                    }
                    crate::TextDiagnosticSeverity::Warning => {
                        self.style_model.palette.warning.as_rgba_array()
                    }
                };
                let diagnostic_markers = presentation
                    .diagnostic_marks
                    .iter()
                    .map(|mark| {
                        (
                            LayoutBox {
                                x: field_x(mark.rect.x),
                                y: content.y + mark.rect.y - scroll_y,
                                width: mark.rect.width,
                                height: mark.rect.height,
                            },
                            marker_color(mark.severity),
                        )
                    })
                    .collect();
                // 查找匹配：普通匹配用 accent 软色令牌，当前匹配用同一 accent
                // 色相加深（本地强调系数，与诊断条带的 2px 常量同级的局部约定）。
                let match_color = |current: bool| -> [f32; 4] {
                    if current {
                        let accent = self.style_model.palette.accent.as_rgba_array();
                        [accent[0], accent[1], accent[2], 0.45]
                    } else {
                        self.style_model.palette.accent_soft_hover.as_rgba_array()
                    }
                };
                let match_markers = presentation
                    .match_marks
                    .iter()
                    .map(|mark| TextMatchMarker {
                        rect: LayoutBox {
                            x: field_x(mark.rect.x),
                            y: content.y + mark.rect.y - scroll_y,
                            width: mark.rect.width,
                            height: mark.rect.height,
                        },
                        color: match_color(mark.current),
                        current: mark.current,
                    })
                    .collect();
                // 颜色装饰 swatch：随滚动平移并按内容区纵向裁剪——只服务
                // 可见行，视口外的 swatch 不产生图元。颜色按宿主给定值直传
                // （半透明由绘制层常规 alpha 合成）。
                let swatch_markers = presentation
                    .swatch_marks
                    .iter()
                    .filter_map(|mark| {
                        let y = content.y + mark.rect.y - scroll_y;
                        (y + mark.rect.height >= content.y && y <= content.y + content.height)
                            .then_some((
                                LayoutBox {
                                    x: field_x(mark.rect.x),
                                    y,
                                    width: mark.rect.width,
                                    height: mark.rect.height,
                                },
                                mark.color,
                            ))
                    })
                    .collect();
                // 当前行条：聚焦多行且选区收起时，光标所在视觉行画低对比
                // 背景条（与选区层互斥，同用 slot 1 绘制层级）。
                let caret_line = if multiline && focused && presentation.selection.is_none() {
                    Some((
                        LayoutBox {
                            x: content.x,
                            y: line_y + presentation.caret_y - scroll_y,
                            width: content.width,
                            height: line_height,
                        },
                        self.style_model.palette.hover.as_rgba_array(),
                    ))
                } else {
                    None
                };
                // 括号匹配描边框：跟随聚焦光标；两端共用 accent 色。
                let bracket_markers = if focused {
                    presentation
                        .bracket_marks
                        .iter()
                        .map(|rect| {
                            (
                                LayoutBox {
                                    x: field_x(rect.x),
                                    y: content.y + rect.y - scroll_y,
                                    width: rect.width,
                                    height: rect.height,
                                },
                                self.style_model.palette.accent.as_rgba_array(),
                            )
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                // 出现高亮：淡底色填充（accent_soft，弱于查找匹配的
                // accent_soft_hover / accent@0.45），跟随聚焦光标。
                let occurrence_markers = if focused {
                    presentation
                        .occurrence_marks
                        .iter()
                        .map(|rect| {
                            (
                                LayoutBox {
                                    x: field_x(rect.x),
                                    y: content.y + rect.y - scroll_y,
                                    width: rect.width,
                                    height: rect.height,
                                },
                                self.style_model.palette.accent_soft.as_rgba_array(),
                            )
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                // 空白字符标记：静态结构标记（不随焦点变化），按视口裁剪
                // ——标记只服务可见行，视口外的空白不产生图元。
                let whitespace_color = self.style_model.palette.faint.as_rgba_array();
                let whitespace_marks = presentation
                    .whitespace_marks
                    .iter()
                    .filter_map(|mark| {
                        let y = content.y + mark.rect.y - scroll_y;
                        (y + mark.rect.height >= content.y && y <= content.y + content.height)
                            .then_some((
                                LayoutBox {
                                    x: field_x(mark.rect.x),
                                    y,
                                    width: mark.rect.width,
                                    height: mark.rect.height,
                                },
                                mark.kind,
                            ))
                    })
                    .collect();
                // wrap guide：全高竖线（贯穿内容区，随横向滚动平移）。
                // 与缩进参考线（slot 10，仅行内缩进深度）区分：列位置由
                // 选项给定且贯穿全高。
                let wrap_guides = presentation
                    .wrap_guides
                    .iter()
                    .map(|&x| {
                        (
                            LayoutBox {
                                x: field_x(x),
                                y: content.y,
                                width: 1.0,
                                height: content.height,
                            },
                            self.style_model.palette.faint.as_rgba_array(),
                        )
                    })
                    .collect();
                // 缩进参考线：静态结构标记，不随焦点变化。
                let indent_guides = presentation
                    .indent_guides
                    .iter()
                    .map(|rect| {
                        (
                            LayoutBox {
                                x: field_x(rect.x),
                                y: content.y + rect.y - scroll_y,
                                width: rect.width,
                                height: rect.height,
                            },
                            self.style_model.palette.border.as_rgba_array(),
                        )
                    })
                    .collect();
                let line_labels = if multiline {
                    presentation
                        .line_tops
                        .iter()
                        .enumerate()
                        .map(|(index, top)| crate::LineLabel {
                            y: content.y + top - scroll_y,
                            height: line_height,
                            // 折叠隐藏行后由 presentation 携带原始行号；
                            // 无折叠时行号就是显示索引 + 1。
                            number: presentation
                                .line_numbers
                                .get(index)
                                .copied()
                                .unwrap_or(index as u32 + 1),
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                // git gutter 竖条：gutter 最左侧 2px（宿主预留 padding-left
                // 的区域），随滚动平移并按内容区纵向裁剪。颜色取语义令牌：
                // 新增 = success、修改 = warning、删除 = danger（调色板的
                // 绿/黄/红三档，与诊断条带同源的语义配色，对应 git 惯例）。
                let git_color = |kind| match kind {
                    crate::TextGitMarkKind::Added => {
                        self.style_model.palette.success.as_rgba_array()
                    }
                    crate::TextGitMarkKind::Modified => {
                        self.style_model.palette.warning.as_rgba_array()
                    }
                    crate::TextGitMarkKind::Deleted => {
                        self.style_model.palette.danger.as_rgba_array()
                    }
                };
                let mut git_geometry = crate::TextGitGutterGeometry {
                    added_color: git_color(crate::TextGitMarkKind::Added),
                    modified_color: git_color(crate::TextGitMarkKind::Modified),
                    deleted_color: git_color(crate::TextGitMarkKind::Deleted),
                    ..crate::TextGitGutterGeometry::default()
                };
                for mark in &presentation.git_marks {
                    let y = content.y + mark.y - scroll_y;
                    // 视口外（上/下越界）的标记不产生图元。
                    if y + mark.height < content.y || y > content.y + content.height {
                        continue;
                    }
                    let rect = LayoutBox {
                        x: bounds.x + border,
                        y,
                        width: 2.0,
                        height: mark.height,
                    };
                    match mark.kind {
                        crate::TextGitMarkKind::Added => git_geometry.added.push(rect),
                        crate::TextGitMarkKind::Modified => git_geometry.modified.push(rect),
                        crate::TextGitMarkKind::Deleted => git_geometry.deleted.push(rect),
                    }
                }
                // 折叠几何：宿主喂入的每个折叠区间在起始行 gutter 画一个
                // 可点击箭头（折叠态右箭头/展开态下箭头）；折叠态区间在
                // 起始行行尾还有摘要标记命中框。gutter 空间不足（padding
                // 左侧小于 18px）时不画箭头，摘要标记仍可点击。
                let mut fold_geometry = crate::TextFoldGeometry::default();
                if multiline
                    && let Some(input) = self.nodes.text_input(id)
                    && let Some(entry) = self.nodes.text_fold_view(id)
                {
                    let offered: Arc<[crate::TextCodeFold]> = match visual {
                        StandardVisual::TextInput { folds, .. } => Arc::clone(folds),
                        _ => Arc::from([]),
                    };
                    if !offered.is_empty() {
                        let view = self.text_display_view(id);
                        let display_of = |offset: usize| match &view {
                            Some(view) => view
                                .display_of(offset)
                                .min(presentation.display_value.len()),
                            None => offset.min(presentation.display_value.len()),
                        };
                        let gutter_width = if padding.left >= 18.0 {
                            (padding.left - 4.0).min(14.0)
                        } else {
                            0.0
                        };
                        for fold in offered.iter() {
                            let collapsed = entry.collapsed.contains(fold);
                            let fold_start = fold.start.min(input.value.len());
                            let display_offset = display_of(fold_start);
                            // 嵌套折叠：起始行被父折叠隐藏时（显示映射钳到
                            // 别处）不画箭头，避免在父折叠起始行叠加幽灵箭头。
                            if !collapsed && display_offset != fold_start {
                                continue;
                            }
                            let display_line = presentation.display_value.as_str()
                                [..display_offset]
                                .matches('\n')
                                .count();
                            if let Some(&top) = presentation.line_tops.get(display_line)
                                && gutter_width > 0.0
                            {
                                let extent = gutter_width.min(line_height);
                                fold_geometry.gutters.push(crate::TextFoldGutter {
                                    bounds: LayoutBox {
                                        x: bounds.x + border + 2.0,
                                        y: content.y + top - scroll_y
                                            + (line_height - extent).max(0.0) / 2.0,
                                        width: extent,
                                        height: extent,
                                    },
                                    fold: *fold,
                                    collapsed,
                                    color: self.style_model.palette.faint.as_rgba_array(),
                                });
                            }
                        }
                        for mark in &presentation.fold_marks {
                            fold_geometry
                                .markers
                                .push(crate::components::TextFoldMarker {
                                    bounds: LayoutBox {
                                        x: field_x(mark.rect.x),
                                        y: content.y + mark.rect.y - scroll_y,
                                        width: mark.rect.width + 2.0,
                                        height: mark.rect.height,
                                    },
                                    fold: mark.fold,
                                });
                        }
                    }
                }
                // sticky scroll：滚动视口顶部落在宿主喂入的折叠区间内部时，
                // 在内容区顶部钉住显示该区间头行（首视觉行，软换行只钉第一
                // 行）。嵌套区间钉最内层（头行顶最大、最靠近视口顶的候选）；
                // 头行仍自然可见（head_top >= scroll_y）或区间末行已完全滚
                // 过视口顶（end_top + line_height <= scroll_y）时不钉——头行
                // 滚回自然位置钉住行瞬时消失。折叠语义不改变派生：折叠态区
                // 间头照常可钉，头行落在折叠隐藏区间内部的跳过（没有可见头
                // 行可钉）。派生每帧
                // 线性扫区间表（区间数小，不加缓存）；钉住行不做内容偏移，
                // 视口下缘被覆盖的内容照常滚动（非 VSCode 推开式布局）。
                let sticky_line = if multiline
                    && let StandardVisual::TextInput {
                        folds: offered,
                        editor_options:
                            crate::TextEditorRenderOptions {
                                sticky_scroll: true,
                                ..
                            },
                        ..
                    } = visual
                    && let Some(input) = self.nodes.text_input(id)
                {
                    let view = self.text_display_view(id);
                    let display_of = |offset: usize| match &view {
                        Some(view) => view
                            .display_of(offset)
                            .min(presentation.display_value.len()),
                        None => offset.min(presentation.display_value.len()),
                    };
                    let display_line_of = |display_offset: usize| {
                        presentation.display_value.as_str()[..display_offset]
                            .matches('\n')
                            .count()
                    };
                    let value = presentation.display_value.as_str();
                    let mut candidate: Option<(f32, usize)> = None;
                    for fold in offered.iter() {
                        let head = fold.start.min(input.value.len());
                        // 头行落在某个折叠态区间的隐藏范围内（值空间被钳制
                        // 映射到替代文本）：无可见头行可钉。不能用显示偏移
                        // 不等判定——头行在其前折叠区间之后时显示偏移同样
                        // 平移，但头行本身可见。
                        let head_hidden = view.as_ref().is_some_and(|view| {
                            view.spans
                                .iter()
                                .any(|span| head > span.value_start && head < span.value_end)
                        });
                        if head_hidden {
                            continue;
                        }
                        let head_display = display_of(head);
                        let end_display = display_of(fold.end.min(input.value.len()));
                        let (Some(&head_top), Some(&end_top)) = (
                            presentation.line_tops.get(display_line_of(head_display)),
                            presentation.line_tops.get(display_line_of(end_display)),
                        ) else {
                            continue;
                        };
                        // 头行未滚出视口顶，或区间末行已完全滚过视口顶（按
                        // 末行底缘计：末行仍跨视口顶时视口顶行仍在区间内）：
                        // 不钉。
                        if head_top >= scroll_y || end_top + line_height <= scroll_y {
                            continue;
                        }
                        if candidate.is_none_or(|(top, _)| head_top > top) {
                            candidate = Some((head_top, head_display));
                        }
                    }
                    candidate.map(|(_, head_display)| {
                        let head_line_start = value[..head_display]
                            .rfind('\n')
                            .map_or(0, |index| index + 1);
                        let head_line_end = value[head_line_start..]
                            .find('\n')
                            .map_or(value.len(), |index| head_line_start + index);
                        crate::TextStickyLineGeometry {
                            panel: LayoutBox {
                                x: content.x,
                                y: content.y,
                                width: content.width,
                                height: line_height,
                            },
                            divider: LayoutBox {
                                x: content.x,
                                y: content.y + line_height - 1.0,
                                width: content.width,
                                height: 1.0,
                            },
                            text: crate::ComponentTextRegion {
                                bounds: LayoutBox {
                                    x: content.x,
                                    y: content.y,
                                    width: content.width,
                                    height: line_height,
                                },
                                content: Arc::from(&value[head_line_start..head_line_end]),
                                color: Some(caret_color),
                                font_size: size.text_size(),
                                font_weight: style.font_weight,
                            },
                            background: style.background.unwrap_or_else(|| {
                                self.style_model.palette.surface.as_rgba_array()
                            }),
                            divider_color: self.style_model.palette.border.as_rgba_array(),
                        }
                    })
                } else {
                    None
                };
                // minimap 竖条：内容区右缘 64px 覆盖条 + 1px 分隔线 + 行条
                // 与视口指示器。文本行宽计算不变（极长行会被条遮挡——声明
                // 取舍）；折叠只影响主视图，行条按原始逻辑行全长计算。禁用
                // 态与窄内容不产生条。
                let minimap = if multiline
                    && !disabled
                    && content.width > crate::components::TEXT_MINIMAP_STRIP_WIDTH
                    && !presentation.minimap_line_lengths.is_empty()
                {
                    Some(text_minimap_geometry(
                        &presentation.minimap_line_lengths,
                        content,
                        scroll_y,
                        line_height,
                        &self.style_model.palette,
                    ))
                } else {
                    None
                };
                // 拖拽移动选中文本的落点指示线：框架侧拖拽态写入的文本
                // 空间细竖线（2px），随滚动平移；纯视觉指示，无命中框。
                let drop_indicator = self.text_drop_indicator(id).map(|rect| {
                    (
                        LayoutBox {
                            x: field_x(rect.x),
                            y: content.y + rect.y - scroll_y,
                            width: rect.width,
                            height: rect.height,
                        },
                        self.style_model.palette.accent.as_rgba_array(),
                    )
                });
                Some(crate::ComponentGeometry::TextInput {
                    diagnostic_markers,
                    match_markers,
                    swatch_markers,
                    swatch_border_color: self.style_model.palette.border.as_rgba_array(),
                    caret_line,
                    bracket_markers,
                    occurrence_markers,
                    drop_indicator,
                    whitespace_marks,
                    whitespace_color,
                    wrap_guides,
                    indent_guides,
                    line_labels,
                    folds: fold_geometry,
                    git_marks: git_geometry,
                    line_labels_color: self.style_model.palette.faint.as_rgba_array(),
                    line_labels_font_size: (size.text_size() - 1.0).max(10.0),
                    text: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: content.x - scroll_x,
                            y: line_y,
                            width: metrics.width.max(content.width),
                            height: if multiline {
                                metrics.height.max(content.height)
                            } else {
                                line_height
                            },
                        },
                        content: Arc::from(presentation.display_value.as_str()),
                        color: Some(if presentation.placeholder {
                            text_input_placeholder_color(
                                &source.layout,
                                self.style_model.palette.faint.as_rgba_array(),
                            )
                        } else {
                            style
                                .color
                                .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array())
                        }),
                        font_size: size.text_size(),
                        font_weight: style.font_weight,
                    },
                    multiline,
                    selection,
                    caret,
                    additional_carets,
                    preedit,
                    completion_popup: {
                        // 补全弹层：聚焦多行编辑器 + 未关闭的非空候选会话；
                        // 锚定主光标行，与其他编辑器覆盖层共用一套定位翻
                        // 转策略（见 `anchored_overlay_panel`）。
                        if multiline && focused {
                            self.nodes
                                .text_completion_view(id)
                                .filter(|state| !state.dismissed)
                                .zip(presentation.overlay_metrics.completion.as_ref())
                                .and_then(|(state, metrics)| {
                                    completion_popup_geometry(
                                        state,
                                        metrics,
                                        OverlayAnchor {
                                            x: field_x(presentation.caret_x),
                                            line_top: line_y + presentation.caret_y,
                                            line_height,
                                        },
                                        bounds,
                                        size.text_size(),
                                        &self.style_model.palette,
                                    )
                                })
                        } else {
                            None
                        }
                    },
                    hover_popup: {
                        // hover 浮窗：宿主喂入即显示（不要求焦点），纯展示。
                        presentation
                            .overlay_metrics
                            .hover_anchor
                            .zip(self.nodes.text_hover_view(id))
                            .and_then(|((hover_x, hover_y), state)| {
                                hover_popup_geometry(
                                    state,
                                    OverlayAnchor {
                                        x: field_x(hover_x),
                                        line_top: line_y + hover_y,
                                        line_height,
                                    },
                                    bounds,
                                    size.text_size(),
                                    &self.style_model.palette,
                                )
                            })
                    },
                    minimap,
                    sticky_line,
                    background: style.background,
                    border: style.border_color,
                    border_width: {
                        let width = if style.border_color.is_some() {
                            source.layout.resolved_border_width()
                        } else {
                            0.0
                        };
                        if multiline && focused && *invalid {
                            width.max(2.0)
                        } else {
                            width
                        }
                    },
                    focus_ring: (!multiline && focused).then(|| {
                        if *invalid {
                            self.style_model.palette.danger.as_rgba_array()
                        } else {
                            self.style_model.palette.accent.as_rgba_array()
                        }
                    }),
                    selection_color: self.style_model.palette.accent_soft.as_rgba_array(),
                    caret_color,
                    additional_caret_color,
                    preedit_color: self.style_model.palette.accent.as_rgba_array(),
                    steppers,
                })
            }
            StandardVisual::Switch {
                thumb_progress,
                label,
                hint,
                checked,
                control_position,
                size,
                invalid,
                ..
            } => {
                let control = LayoutBox {
                    x: match control_position {
                        SwitchControlPosition::Start => content.x,
                        SwitchControlPosition::End => content.x + (content.width - 30.0).max(0.0),
                    },
                    y: content.y + (content.height - 16.0) / 2.0,
                    width: 30.0_f32.min(content.width),
                    height: 16.0_f32.min(content.height),
                };
                let text_x = if *control_position == SwitchControlPosition::Start {
                    control.x + control.width + 8.0
                } else {
                    content.x
                };
                let text_right = if *control_position == SwitchControlPosition::End {
                    control.x - 8.0
                } else {
                    content.x + content.width
                };
                let text_width = (text_right - text_x).max(0.0);
                let (label_bounds, hint_bounds) = if hint.is_some() {
                    (
                        LayoutBox {
                            x: text_x,
                            y: content.y,
                            width: text_width,
                            height: 18.0_f32.min(content.height),
                        },
                        Some(LayoutBox {
                            x: text_x,
                            y: content.y + 20.0,
                            width: text_width,
                            height: (content.height - 20.0).max(0.0),
                        }),
                    )
                } else {
                    (
                        LayoutBox {
                            x: text_x,
                            y: content.y,
                            width: text_width,
                            height: content.height,
                        },
                        None,
                    )
                };
                let palette = self.style_model.palette;
                let hovered = self
                    .input
                    .pointer_hover
                    .values()
                    .any(|target| *target == id);
                let pressed = self
                    .input
                    .pointer_press
                    .values()
                    .any(|target| *target == id);
                let disabled = self.record(id).accessibility.disabled;
                let mix = |foreground: [f32; 4], background: [f32; 4], amount: f32| {
                    let amount = amount.clamp(0.0, 1.0);
                    std::array::from_fn(|channel| {
                        foreground[channel] * amount + background[channel] * (1.0 - amount)
                    })
                };
                let fade = |mut color: [f32; 4]| {
                    if disabled {
                        color[3] *= 0.55;
                    }
                    color
                };
                let track_background = if *checked {
                    if pressed {
                        palette.accent_strong.as_rgba_array()
                    } else {
                        palette.accent.as_rgba_array()
                    }
                } else {
                    mix(
                        palette.hover.as_rgba_array(),
                        palette.background.as_rgba_array(),
                        0.78,
                    )
                };
                let track_border = if *invalid {
                    palette.danger.as_rgba_array()
                } else if *checked {
                    if hovered || pressed {
                        palette.accent_strong.as_rgba_array()
                    } else {
                        palette.accent.as_rgba_array()
                    }
                } else if hovered || pressed {
                    mix(
                        palette.accent.as_rgba_array(),
                        palette.border_strong.as_rgba_array(),
                        if pressed { 0.70 } else { 0.42 },
                    )
                } else {
                    palette.border_strong.as_rgba_array()
                };
                let thumb_background = if *checked {
                    palette.accent_text.as_rgba_array()
                } else {
                    mix(
                        palette.faint.as_rgba_array(),
                        palette.background.as_rgba_array(),
                        0.70,
                    )
                };
                Some(crate::ComponentGeometry::Switch {
                    thumb_progress: *thumb_progress,
                    label: text_region(
                        label_bounds,
                        Arc::clone(label),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    hint: hint.as_ref().zip(hint_bounds).map(|(hint, bounds)| {
                        text_region(
                            bounds,
                            Arc::clone(hint),
                            true,
                            (size.text_size() - 1.0).max(10.0),
                            None,
                        )
                    }),
                    control,
                    track_background: fade(track_background),
                    track_border: fade(track_border),
                    thumb_background: fade(thumb_background),
                })
            }
            StandardVisual::Scrollbar {
                axes,
                visibility,
                revealed,
                dragging,
            } => {
                if !revealed || matches!(visibility, nana_ui_core::ScrollbarVisibility::Hidden) {
                    return None;
                }
                let metrics = self.scroll_metrics(id)?;
                let offset = self.scroll_offset(id).unwrap_or_default();
                let skin = source.layout.paint.scrollbar;
                let chrome = skin
                    .map(|skin| skin.metrics(nana_ui_core::SCROLLBAR_METRICS))
                    .unwrap_or(nana_ui_core::SCROLLBAR_METRICS);
                let palette = &self.style_model.palette;
                let track_background = skin.and_then(|skin| skin.track_color).or_else(|| {
                    matches!(visibility, nana_ui_core::ScrollbarVisibility::Always)
                        .then(|| palette.subtle.as_rgba_array())
                });
                let scrolls = |axis: nana_ui_core::ScrollbarAxis| match axis {
                    nana_ui_core::ScrollbarAxis::Horizontal => {
                        axes.horizontal() && metrics.content_width > metrics.viewport_width
                    }
                    nana_ui_core::ScrollbarAxis::Vertical => {
                        axes.vertical() && metrics.content_height > metrics.viewport_height
                    }
                };
                let bar = |axis: nana_ui_core::ScrollbarAxis| {
                    if !scrolls(axis) {
                        return None;
                    }
                    let horizontal = matches!(axis, nana_ui_core::ScrollbarAxis::Horizontal);
                    // Give up the far corner only when the other axis also has
                    // a bar, so the two never overlap.
                    let corner = if scrolls(match axis {
                        nana_ui_core::ScrollbarAxis::Horizontal => {
                            nana_ui_core::ScrollbarAxis::Vertical
                        }
                        nana_ui_core::ScrollbarAxis::Vertical => {
                            nana_ui_core::ScrollbarAxis::Horizontal
                        }
                    }) {
                        chrome.thickness
                    } else {
                        0.0
                    };
                    let along = if horizontal {
                        (bounds.width - corner).max(0.0)
                    } else {
                        (bounds.height - corner).max(0.0)
                    };
                    let track = nana_ui_core::scrollbar_track(
                        if horizontal {
                            metrics.viewport_width
                        } else {
                            metrics.viewport_height
                        },
                        if horizontal {
                            metrics.content_width
                        } else {
                            metrics.content_height
                        },
                        if horizontal { offset.x } else { offset.y },
                        if horizontal { bounds.x } else { bounds.y },
                        along,
                        chrome,
                    )?;
                    let active = *dragging == Some(axis);
                    let thumb_inset = ((chrome.thickness - chrome.thumb_thickness) / 2.0).max(0.0);
                    let (track_box, thumb_box) = if horizontal {
                        let track_y = bounds.y + bounds.height - chrome.thickness;
                        (
                            LayoutBox {
                                x: track.origin,
                                y: track_y,
                                width: track.length,
                                height: chrome.thickness,
                            },
                            LayoutBox {
                                x: track.thumb_origin,
                                y: track_y + thumb_inset,
                                width: track.thumb_length,
                                height: chrome.thumb_thickness,
                            },
                        )
                    } else {
                        let track_x = bounds.x + bounds.width - chrome.thickness;
                        (
                            LayoutBox {
                                x: track_x,
                                y: track.origin,
                                width: chrome.thickness,
                                height: track.length,
                            },
                            LayoutBox {
                                x: track_x + thumb_inset,
                                y: track.thumb_origin,
                                width: chrome.thumb_thickness,
                                height: track.thumb_length,
                            },
                        )
                    };
                    Some(crate::ScrollbarBar {
                        track: track_box,
                        thumb: thumb_box,
                        track_background,
                        thumb_background: skin.and_then(|skin| skin.thumb_color).unwrap_or_else(
                            || {
                                if active {
                                    palette.muted.as_rgba_array()
                                } else {
                                    palette.border_strong.as_rgba_array()
                                }
                            },
                        ),
                        thumb_radius: chrome.thumb_radius(),
                        max_offset: track.max_offset,
                    })
                };
                let horizontal = bar(nana_ui_core::ScrollbarAxis::Horizontal);
                let vertical = bar(nana_ui_core::ScrollbarAxis::Vertical);
                (horizontal.is_some() || vertical.is_some()).then_some(
                    crate::ComponentGeometry::Scrollbar {
                        horizontal,
                        vertical,
                    },
                )
            }
            StandardVisual::Range {
                label,
                value,
                unit,
                size,
                ..
            } => {
                let gap = match size {
                    nana_ui_core::ControlSize::Small => 6.0,
                    nana_ui_core::ControlSize::Medium => 8.0,
                    nana_ui_core::ControlSize::Large => 10.0,
                };
                let label_width = label
                    .as_ref()
                    .map_or(0.0, |_| 84.0_f32.min(content.width * 0.28));
                let unit_width = unit.as_ref().map_or(0.0, |_| 32.0_f32.min(content.width));
                let value_width = 60.0_f32.min((content.width - unit_width).max(0.0));
                let trailing_width = value_width + unit_width;
                let track_x = content.x + label_width + if label.is_some() { gap } else { 0.0 };
                let track_right = content.x + content.width
                    - trailing_width
                    - if trailing_width > 0.0 { gap } else { 0.0 };
                let thumb = size.icon_size();
                let track = LayoutBox {
                    x: track_x + thumb / 2.0,
                    y: content.y + (content.height - thumb) / 2.0,
                    width: (track_right - track_x - thumb).max(0.0),
                    height: thumb.min(content.height),
                };
                Some(crate::ComponentGeometry::Range {
                    label: label.as_ref().map(|label| {
                        text_region(
                            LayoutBox {
                                x: content.x,
                                y: content.y,
                                width: label_width,
                                height: content.height,
                            },
                            Arc::clone(label),
                            false,
                            size.text_size(),
                            Some(500),
                        )
                    }),
                    value: text_region(
                        LayoutBox {
                            x: content.x + content.width - value_width - unit_width,
                            y: content.y,
                            width: value_width,
                            height: content.height,
                        },
                        Arc::clone(value),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    unit: unit.as_ref().map(|unit| {
                        text_region(
                            LayoutBox {
                                x: content.x + content.width - unit_width,
                                y: content.y,
                                width: unit_width,
                                height: content.height,
                            },
                            Arc::clone(unit),
                            true,
                            (size.text_size() - 1.0).max(10.0),
                            None,
                        )
                    }),
                    track,
                })
            }
            StandardVisual::Card {
                title,
                kind,
                loading,
                ..
            } => {
                let shaped_title_width = self
                    .text_metrics(id)
                    .map_or(0.0, |metrics| metrics.width.min(content.width));
                let title_width = (content.width - if *loading { 22.0 } else { 0.0 }).max(0.0);
                let title_y = bounds.y + border + (padding.top - 24.0).max(0.0);
                Some(crate::ComponentGeometry::Card {
                    title: title.as_ref().map(|title| {
                        text_region(
                            LayoutBox {
                                x: bounds.x + border + padding.left,
                                y: title_y,
                                width: title_width,
                                height: 18.0,
                            },
                            Arc::clone(title),
                            false,
                            13.0,
                            Some(600),
                        )
                    }),
                    content,
                    elevation: (*kind == nana_ui_core::CardKind::Raised).then_some(
                        crate::ComponentElevation::surface_shadow(self.style_model.theme_mode),
                    ),
                    spinner: (*loading).then_some(LayoutBox {
                        x: (bounds.x + border + padding.left + shaped_title_width + 8.0)
                            .min(content.x + content.width - 14.0),
                        y: title_y + 2.0,
                        width: 14.0,
                        height: 14.0,
                    }),
                })
            }
            StandardVisual::ListItem {
                leading,
                content: content_slot,
                trailing,
                detail,
            } => {
                // 隐藏的槽位子节点已退出 flex 流，其盒几何是陈旧的，不得
                // 参与行文本区间的兜底计算。
                let slot_box = |id: StableNodeId| -> Option<LayoutBox> {
                    if self
                        .node_style(id)
                        .is_some_and(|style| style.layout.omits_box())
                    {
                        return None;
                    }
                    self.layout_box(id)
                };
                let leading = leading.and_then(slot_box);
                let trailing = trailing.and_then(slot_box);
                let fallback_x = leading.map_or(content.x, |leading| {
                    leading.x
                        + leading.width
                        + source.layout.main_gap_against(
                            nana_ui_core::FlexDirection::Row,
                            nana_ui_core::ParentBox::from_viewport(content.width, content.height),
                        )
                });
                let fallback_right = trailing.map_or(content.x + content.width, |trailing| {
                    trailing.x
                        - source.layout.main_gap_against(
                            nana_ui_core::FlexDirection::Row,
                            nana_ui_core::ParentBox::from_viewport(content.width, content.height),
                        )
                });
                let mut label_rect = content_slot.and_then(slot_box).unwrap_or(LayoutBox {
                    x: fallback_x,
                    y: content.y,
                    width: (fallback_right - fallback_x).max(0.0),
                    height: content.height,
                });
                // 单行 detail：小字号 muted 文本右对齐；label 估宽避让，
                // 放不下时 detail 占剩余宽度、超出交给省略号（与 LabeledValue
                // 的值侧同款规则）。
                let detail_region =
                    detail
                        .clone()
                        .filter(|detail| !detail.is_empty())
                        .map(|detail| {
                            let label_size = style.font_size;
                            let detail_size = (label_size - 1.0).max(10.0);
                            let gap = 8.0_f32;
                            let label_natural =
                                estimated_text_width(self.text(id).unwrap_or_default(), label_size);
                            let detail_natural = estimated_text_width(&detail, detail_size);
                            let min_detail_visible = 16.0_f32;
                            let detail_width =
                                if label_natural + gap + detail_natural <= label_rect.width {
                                    detail_natural
                                } else {
                                    (label_rect.width - label_natural - gap)
                                        .max(min_detail_visible)
                                        .min(label_rect.width)
                                };
                            let detail_x =
                                (label_rect.x + label_rect.width - detail_width).max(label_rect.x);
                            let detail_height =
                                (detail_size * 1.2).min(label_rect.height.max(detail_size));
                            let detail_y =
                                label_rect.y + (label_rect.height - detail_height).max(0.0) / 2.0;
                            label_rect.width = ((detail_x - gap) - label_rect.x).max(0.0);
                            crate::ComponentTextRegion {
                                bounds: LayoutBox {
                                    x: detail_x,
                                    y: detail_y,
                                    width: detail_width,
                                    height: detail_height,
                                },
                                content: detail,
                                color: Some(self.style_model.palette.muted.as_rgba_array()),
                                font_size: detail_size,
                                font_weight: None,
                            }
                        });
                Some(crate::ComponentGeometry::ListItem {
                    leading,
                    content: Some(label_rect),
                    trailing,
                    detail: detail_region,
                })
            }
            StandardVisual::StatusBadge {
                label,
                tone,
                compact,
            } => {
                let (horizontal, indicator_slot, gap, text_size) = if *compact {
                    (7.0, 6.0, 5.0, 11.0)
                } else {
                    (8.0, 8.0, 6.0, 12.0)
                };
                let diameter = indicator_slot * 10.0 / 24.0;
                let foreground = self
                    .style_model
                    .palette
                    .get(status_tone_role(*tone))
                    .as_rgba_array();
                let mut background = foreground;
                background[3] *= 0.12;
                Some(crate::ComponentGeometry::StatusBadge {
                    indicator: LayoutBox {
                        x: bounds.x + horizontal + (indicator_slot - diameter) / 2.0,
                        y: bounds.y + (bounds.height - diameter) / 2.0,
                        width: diameter,
                        height: diameter,
                    },
                    label: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: bounds.x + horizontal + indicator_slot + gap,
                            y: bounds.y,
                            width: (bounds.width - horizontal * 2.0 - indicator_slot - gap)
                                .max(0.0),
                            height: bounds.height,
                        },
                        content: Arc::clone(label),
                        color: Some(foreground),
                        font_size: text_size,
                        font_weight: Some(500),
                    },
                    background,
                    foreground,
                })
            }
            StandardVisual::ValidationMessage {
                message,
                intent,
                compact,
            } => {
                let (indicator_slot, gap, text_size) = if *compact {
                    (12.0, 5.0, 11.0)
                } else {
                    (14.0, 6.0, 12.0)
                };
                let diameter = indicator_slot * 10.0 / 24.0;
                let foreground = self
                    .style_model
                    .palette
                    .get(match intent {
                        nana_ui_core::ValidationIntent::Warning => SemanticColorRole::Warning,
                        nana_ui_core::ValidationIntent::Danger => SemanticColorRole::Danger,
                    })
                    .as_rgba_array();
                Some(crate::ComponentGeometry::ValidationMessage {
                    indicator: LayoutBox {
                        x: bounds.x + (indicator_slot - diameter) / 2.0,
                        y: bounds.y + (bounds.height - diameter) / 2.0,
                        width: diameter,
                        height: diameter,
                    },
                    label: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: bounds.x + indicator_slot + gap,
                            y: bounds.y,
                            width: (bounds.width - indicator_slot - gap).max(0.0),
                            height: bounds.height,
                        },
                        content: Arc::clone(message),
                        color: Some(foreground),
                        font_size: text_size,
                        font_weight: None,
                    },
                    foreground,
                })
            }
            StandardVisual::EmptyState {
                title,
                message,
                icon,
                compact,
                action,
            } => {
                let (horizontal, vertical, title_size, message_size, spacing) = if *compact {
                    (6.0, 8.0, 12.0, 11.0, 2.0)
                } else {
                    (16.0, 24.0, 13.0, 12.0, 6.0)
                };
                let width = (bounds.width - horizontal * 2.0).max(0.0);
                let presentation = self.nodes.empty_state_text(id).copied().unwrap_or_default();
                let text_bounds = |metrics: TextMetrics, y: f32| {
                    let shaped_width = metrics.width.clamp(0.0, width);
                    crate::LayoutBox {
                        x: bounds.x
                            + horizontal
                            + if *compact {
                                0.0
                            } else {
                                (width - shaped_width) / 2.0
                            },
                        y,
                        width: shaped_width,
                        height: metrics.height,
                    }
                };
                let mut y = bounds.y + vertical;
                let icon = icon.map(|icon| {
                    let icon_width = 22.0_f32.min(width);
                    let icon_bounds = LayoutBox {
                        x: if *compact {
                            bounds.x + horizontal
                        } else {
                            bounds.x + horizontal + (width - icon_width) / 2.0
                        },
                        y,
                        width: icon_width,
                        height: 22.0,
                    };
                    y += 22.0 + spacing;
                    (
                        icon,
                        icon_bounds,
                        self.style_model.palette.faint.as_rgba_array(),
                    )
                });
                let title_region = crate::ComponentTextRegion {
                    bounds: text_bounds(presentation.title, y),
                    content: Arc::clone(title),
                    color: Some(if *compact {
                        self.style_model.palette.muted.as_rgba_array()
                    } else {
                        self.style_model.palette.text.as_rgba_array()
                    }),
                    font_size: title_size,
                    font_weight: Some(600),
                };
                y += presentation.title.height;
                let message = message.as_ref().map(|message| {
                    y += spacing;
                    crate::ComponentTextRegion {
                        bounds: text_bounds(presentation.message.unwrap_or_default(), y),
                        content: Arc::clone(message),
                        color: Some(self.style_model.palette.muted.as_rgba_array()),
                        font_size: message_size,
                        font_weight: None,
                    }
                });
                Some(crate::ComponentGeometry::EmptyState {
                    root_clip: bounds,
                    content_clip: LayoutBox {
                        x: bounds.x + horizontal,
                        y: bounds.y + vertical,
                        width,
                        height: (bounds.height - vertical * 2.0).max(0.0),
                    },
                    icon,
                    title: title_region,
                    message,
                    action: action.and_then(|action| self.layout_box(action)),
                })
            }
            StandardVisual::LabeledValue {
                label,
                value,
                value_role,
                value_weight,
                compact,
                action,
            } => {
                let gap = if *compact { 4.0 } else { 8.0 };
                let right = action
                    .and_then(|action| self.layout_box(action))
                    .map_or(bounds.x + bounds.width, |action| {
                        (action.x - gap).max(bounds.x)
                    });
                let available = (right - bounds.x).max(0.0);
                let label_size = 11.0_f32;
                let value_size = 12.0_f32;
                let label_height = (label_size * 1.2).min(bounds.height.max(label_size));
                let value_height = (value_size * 1.2).min(bounds.height.max(value_size));
                // 属性名按自身文本估宽保底,不再压缩到字号常数;两侧都放得下时各取自然宽度。
                let label_natural = if label.is_empty() {
                    0.0
                } else {
                    estimated_text_width(label, label_size)
                };
                let value_natural = estimated_text_width(value, value_size);
                let min_value_visible = 16.0_f32;
                // 放不下时值侧占满剩余宽度,超出部分由文本图元的省略号收尾;
                // 仅属性名自身就放不下(剩余为负)才回退最小可见宽度。
                let value_width = if label_natural + gap + value_natural <= available {
                    value_natural
                } else {
                    (available - label_natural - gap)
                        .max(min_value_visible)
                        .min(available)
                };
                let value_x = (right - value_width).max(bounds.x);
                let label_width = ((value_x - gap - bounds.x).max(0.0)).min(label_natural);
                let center_y = |height: f32| bounds.y + (bounds.height - height).max(0.0) / 2.0;
                Some(crate::ComponentGeometry::LabeledValue {
                    label: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: bounds.x,
                            y: center_y(label_height),
                            width: label_width,
                            height: label_height,
                        },
                        content: Arc::clone(label),
                        color: Some(self.style_model.palette.faint.as_rgba_array()),
                        font_size: label_size,
                        font_weight: None,
                    },
                    value: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: value_x,
                            y: center_y(value_height),
                            width: value_width,
                            height: value_height,
                        },
                        content: Arc::clone(value),
                        color: Some(self.style_model.color(*value_role).as_rgba_array()),
                        font_size: value_size,
                        font_weight: Some(*value_weight),
                    },
                    action: action.and_then(|action| self.layout_box(action)),
                })
            }
            StandardVisual::SelectionOption {
                label,
                icon,
                size,
                show_focus_ring,
                indicator,
                selected,
                disabled,
            } => {
                let icon_extent = size.icon_size().min(content.height);
                let base_padding = if *indicator {
                    size.radio_lead()
                } else {
                    size.padding_x() + 2.0
                };
                let ring = indicator.then(|| {
                    let extent = size.indicator_size().min(bounds.height);
                    let ring = LayoutBox {
                        x: bounds.x + nana_ui_core::RADIO_ROW_INSET,
                        y: bounds.y + (bounds.height - extent) / 2.0,
                        width: extent,
                        height: extent,
                    };
                    let (ring_color, dot_color) = if *disabled {
                        let faint = self.style_model.palette.faint.as_rgba_array();
                        (faint, faint)
                    } else if *selected {
                        let accent = self.style_model.palette.accent.as_rgba_array();
                        (accent, accent)
                    } else {
                        (
                            self.style_model.palette.border_strong.as_rgba_array(),
                            self.style_model.palette.accent.as_rgba_array(),
                        )
                    };
                    let dot_extent = extent / 2.5;
                    crate::RadioIndicator {
                        ring,
                        ring_color,
                        dot: selected.then_some((
                            LayoutBox {
                                x: ring.x + (extent - dot_extent) / 2.0,
                                y: ring.y + (extent - dot_extent) / 2.0,
                                width: dot_extent,
                                height: dot_extent,
                            },
                            dot_color,
                        )),
                    }
                });
                let icon_bounds = icon.map(|icon| {
                    (
                        icon,
                        LayoutBox {
                            x: bounds.x + base_padding,
                            y: icon_y_on_text_glyph_center(
                                content.y,
                                content.height,
                                style.font_size,
                                style.line_height,
                                matches!(
                                    source.text_vertical_alignment,
                                    TextVerticalAlignment::Center
                                ),
                                icon_extent,
                            ),
                            width: icon_extent,
                            height: icon_extent,
                        },
                        style
                            .color
                            .unwrap_or_else(|| self.style_model.palette.muted.as_rgba_array()),
                    )
                });
                let label_x = icon_bounds
                    .as_ref()
                    .map_or(content.x, |(_, icon, _)| icon.x + icon.width + 5.0);
                Some(crate::ComponentGeometry::SelectionOption {
                    icon: icon_bounds,
                    label: text_region(
                        LayoutBox {
                            x: label_x,
                            y: content.y,
                            width: (content.x + content.width - label_x).max(0.0),
                            height: content.height,
                        },
                        Arc::clone(label),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    focus_ring: (*show_focus_ring
                        && self.input.focused.get(&self.record(id).document) == Some(&id))
                    .then(|| self.style_model.palette.accent.as_rgba_array()),
                    indicator: ring,
                })
            }
            StandardVisual::Progress {
                value_ratio,
                label,
                cancellable,
            } => progress_geometry(
                bounds,
                style,
                *value_ratio,
                6.0,
                3.0,
                label.as_ref(),
                *cancellable,
                self.style_model.palette.text.as_rgba_array(),
            ),
            StandardVisual::LevelMeter {
                value_ratio, girth, ..
            } => {
                let girth = if girth.is_finite() && *girth > 0.0 {
                    *girth
                } else {
                    4.0
                };
                progress_geometry(
                    bounds,
                    style,
                    *value_ratio,
                    girth,
                    girth / 2.0,
                    None,
                    false,
                    [0.0; 4],
                )
            }
            StandardVisual::FormField {
                label,
                hint,
                error,
                size,
                control,
            } => form_field_geometry(
                bounds,
                *size,
                label,
                hint.as_ref(),
                error.as_ref(),
                *control,
                &|id| self.layout_box(id),
                &self.style_model.palette,
            ),
            StandardVisual::Toast {
                title,
                description,
                dismissible,
                ..
            } => {
                let pad_x = 12.0;
                let pad_y = 10.0;
                let indicator = 7.0;
                let gap = 8.0;
                let dismiss = if *dismissible { 28.0 } else { 0.0 };
                let copy_x = bounds.x + pad_x + indicator + gap;
                let copy_right =
                    bounds.x + bounds.width - pad_x - if *dismissible { dismiss } else { 0.0 };
                let copy_width = (copy_right - copy_x).max(0.0);
                let title_height = 12.0;
                let desc_height = 11.0;
                let has_desc = description.is_some();
                let title_y = if has_desc {
                    bounds.y + pad_y
                } else {
                    bounds.y + (bounds.height - title_height) / 2.0
                };
                Some(crate::ComponentGeometry::Toast {
                    indicator: LayoutBox {
                        x: bounds.x + pad_x,
                        y: bounds.y + (bounds.height - indicator) / 2.0,
                        width: indicator,
                        height: indicator,
                    },
                    title: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: copy_x,
                            y: title_y,
                            width: copy_width,
                            height: title_height,
                        },
                        content: Arc::clone(title),
                        color: Some(self.style_model.palette.text.as_rgba_array()),
                        font_size: 12.0,
                        font_weight: Some(600),
                    },
                    description: description.as_ref().map(|description| {
                        crate::ComponentTextRegion {
                            bounds: LayoutBox {
                                x: copy_x,
                                y: title_y + title_height + 2.0,
                                width: copy_width,
                                height: desc_height,
                            },
                            content: Arc::clone(description),
                            color: Some(self.style_model.palette.muted.as_rgba_array()),
                            font_size: 11.0,
                            font_weight: None,
                        }
                    }),
                    dismiss: dismissible.then(|| LayoutBox {
                        x: bounds.x + bounds.width - pad_x - dismiss,
                        y: bounds.y + (bounds.height - dismiss) / 2.0,
                        width: dismiss,
                        height: dismiss,
                    }),
                })
            }
            StandardVisual::XYPad { nx, ny, .. } => {
                let pad = bounds;
                let thumb = 8.0;
                let nx = nx.clamp(0.0, 1.0);
                let ny = ny.clamp(0.0, 1.0);
                Some(crate::ComponentGeometry::XYPad {
                    pad,
                    thumb: LayoutBox {
                        x: pad.x + nx * pad.width - thumb / 2.0,
                        y: pad.y + ny * pad.height - thumb / 2.0,
                        width: thumb,
                        height: thumb,
                    },
                    h_axis: LayoutBox {
                        x: pad.x,
                        y: pad.y + pad.height / 2.0 - 0.5,
                        width: pad.width,
                        height: 1.0,
                    },
                    v_axis: LayoutBox {
                        x: pad.x + pad.width / 2.0 - 0.5,
                        y: pad.y,
                        width: 1.0,
                        height: pad.height,
                    },
                    background: style.background,
                    border: style.border_color,
                    border_width: if style.border_color.is_some() {
                        source.layout.resolved_border_width()
                    } else {
                        0.0
                    },
                    thumb_color: self.style_model.palette.accent.as_rgba_array(),
                    axis_color: self.style_model.palette.border.as_rgba_array(),
                })
            }
            StandardVisual::Select {
                label,
                placeholder,
                size,
                opened,
                options,
                highlighted,
                ..
            } => Some(crate::select::select_geometry(
                bounds,
                label,
                *placeholder,
                *size,
                *opened,
                options,
                *highlighted,
                style,
                &source,
                &self.style_model.palette,
            )),
            StandardVisual::MenuSurface {
                kind: crate::MenuSurfaceKind::ContextMenu,
                query,
                rows,
                highlighted,
                ..
            } if query.is_some() || !rows.is_empty() => Some(crate::menus::context_menu_geometry(
                bounds,
                query.as_ref(),
                rows,
                *highlighted,
                &self.style_model.palette,
            )),
            StandardVisual::MenuSurface {
                trigger,
                trigger_icon,
                gap,
                ..
            } => Some(crate::popover::menu_surface_geometry(
                bounds,
                trigger.as_ref(),
                *trigger_icon,
                *gap,
                style,
                &self.style_model.palette,
            )),
            StandardVisual::ActionMenuItem {
                label,
                hint,
                icon,
                danger,
                disabled,
                size,
                ..
            } => Some(crate::menus::action_menu_item_geometry(
                bounds,
                label,
                hint.as_ref(),
                *icon,
                *danger,
                *disabled,
                *size,
                style,
                &self.style_model.palette,
            )),
            StandardVisual::TreeView { rows, size } => Some(crate::tree_view::tree_view_geometry(
                bounds,
                rows,
                *size,
                &self.style_model.palette,
            )),
            StandardVisual::CommandPalette {
                title,
                query,
                placeholder,
                empty,
                rows,
            } => Some(crate::command_palette::command_palette_geometry(
                bounds,
                title,
                query,
                placeholder,
                empty.as_ref(),
                rows,
                &self.style_model.palette,
            )),
            StandardVisual::QrCode { modules, width } => {
                let (module_size, (ox, oy)) = crate::qr_code::module_geometry(bounds, *width);
                let quiet = crate::qr_code::QUIET_ZONE_MODULES as f32;
                let dark = modules
                    .iter()
                    .enumerate()
                    .filter(|(_, dark)| **dark)
                    .map(|(index, _)| {
                        let x = index % *width;
                        let y = index / *width;
                        LayoutBox {
                            x: bounds.x + ox + (x as f32 + quiet) * module_size,
                            y: bounds.y + oy + (y as f32 + quiet) * module_size,
                            width: module_size,
                            height: module_size,
                        }
                    })
                    .collect();
                Some(crate::ComponentGeometry::QrCode {
                    field: LayoutBox {
                        x: bounds.x + ox,
                        y: bounds.y + oy,
                        width: module_size * (*width as f32 + quiet * 2.0),
                        height: module_size * (*width as f32 + quiet * 2.0),
                    },
                    module_size,
                    dark,
                })
            }
            #[cfg(feature = "calendar")]
            StandardVisual::CalendarHeatmap {
                cells,
                month_labels,
                day_labels,
                cell_size,
                max_level,
                active,
                active_title,
                ..
            } => Some(calendar_heatmap_geometry(
                bounds,
                cells,
                month_labels,
                day_labels,
                *cell_size,
                *max_level,
                *active,
                active_title.as_deref(),
                self.style_model.theme_mode,
                &self.style_model.palette,
            )),
            #[cfg(feature = "charts")]
            StandardVisual::TimeSeriesChart { values } => Some(time_series_geometry(
                bounds,
                values,
                self.style_model.theme_mode,
            )),
            #[cfg(feature = "controls")]
            StandardVisual::ReorderList {
                rows,
                size,
                spacing,
                insert,
            } => {
                // Live row children own their label painting; a stale retained
                // `rows` (projected before the children were attached) must not
                // paint a second copy underneath them.
                let rows = if self
                    .nodes
                    .get(id)
                    .is_some_and(|node| !node.hierarchy.children.is_empty())
                {
                    Arc::<[crate::ReorderRowPaint]>::from([])
                } else {
                    rows.clone()
                };
                Some(reorder_list_geometry(
                    bounds,
                    &rows,
                    *size,
                    *spacing,
                    *insert,
                    &self.style_model.palette,
                ))
            }
            #[cfg(feature = "rich-text")]
            StandardVisual::NativeMarkdown { text, selection } => {
                let (text, selection, selection_color) = selectable_text_regions(
                    content,
                    text,
                    *selection,
                    style,
                    &self.style_model.palette,
                );
                Some(crate::ComponentGeometry::NativeMarkdown {
                    text,
                    selection,
                    selection_color,
                })
            }
            #[cfg(feature = "rich-text")]
            StandardVisual::SelectableRichText { text, selection } => {
                let (text, selection, selection_color) = selectable_text_regions(
                    content,
                    text,
                    *selection,
                    style,
                    &self.style_model.palette,
                );
                Some(crate::ComponentGeometry::SelectableRichText {
                    text,
                    selection,
                    selection_color,
                })
            }
            #[cfg(feature = "graph-canvas")]
            StandardVisual::GraphCanvas {
                nodes,
                ports,
                edges,
                connecting,
                grid_spacing,
                viewport_offset_x,
                viewport_offset_y,
                viewport_zoom,
            } => Some(graph_canvas_geometry(
                bounds,
                nodes,
                ports,
                edges,
                connecting.as_ref(),
                *grid_spacing,
                *viewport_offset_x,
                *viewport_offset_y,
                *viewport_zoom,
                &self.style_model.palette,
            )),
            #[cfg(feature = "graph-canvas")]
            StandardVisual::GraphMinimap {
                bounds: model_bounds,
                nodes,
                indicator,
                node_fill,
            } => Some(graph_minimap_geometry(
                bounds,
                *model_bounds,
                nodes,
                indicator.as_ref(),
                *node_fill,
                &self.style_model.palette,
            )),
            #[cfg(feature = "image-viewer")]
            StandardVisual::ImageViewer {
                name,
                metadata,
                zoom,
                offset_x,
                offset_y,
            } => Some(image_viewer_geometry(
                bounds,
                name.as_ref(),
                metadata.as_ref(),
                *zoom,
                *offset_x,
                *offset_y,
                &self.style_model.palette,
            )),
            StandardVisual::KeyCaptureLayer { recording } => Some(key_capture_geometry(
                content,
                *recording,
                &self.style_model.palette,
            )),
            StandardVisual::KeymapLayer => {
                Some(keymap_geometry(content, &self.style_model.palette))
            }
            _ => None,
        }
    }
}
