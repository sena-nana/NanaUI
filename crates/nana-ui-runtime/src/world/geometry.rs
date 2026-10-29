//! Component geometry derived from committed Runtime data.

#[cfg(feature = "calendar")]
mod calendar;
#[cfg(feature = "calendar")]
pub(super) use calendar::*;

#[cfg(feature = "charts")]
mod charts;

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

/// Block extent of a progress bar.
pub(crate) const PROGRESS_GIRTH: f32 = nana_ui_core::space::SM;

pub(super) fn key_capture_geometry(
    content: LayoutBox,
    recording: bool,
    style_model: StyleModelRef,
    measure: crate::text_width::ChromeTextMeasure<'_>,
) -> crate::ComponentGeometry {
    let palette = style_model;
    let label: Arc<str> = if recording {
        Arc::from("Recording")
    } else {
        Arc::from("Idle")
    };
    crate::ComponentGeometry::KeyCaptureLayer {
        badge: key_badge_region(content, &label, !recording, style_model, measure),
        background: Some(if recording {
            palette.color(SemanticColorRole::AccentSoft).as_rgba_array()
        } else {
            palette.color(SemanticColorRole::Subtle).as_rgba_array()
        }),
    }
}

pub(super) fn keymap_geometry(
    content: LayoutBox,
    style_model: StyleModelRef,
    measure: crate::text_width::ChromeTextMeasure<'_>,
) -> crate::ComponentGeometry {
    crate::ComponentGeometry::KeymapLayer {
        badge: key_badge_region(content, "Keymap", false, style_model, measure),
    }
}

pub(super) fn key_badge_region(
    origin: LayoutBox,
    label: &str,
    muted: bool,
    palette: StyleModelRef,
    measure: crate::text_width::ChromeTextMeasure<'_>,
) -> crate::ComponentTextRegion {
    let metrics = palette.metrics;
    let height = nana_ui_core::ControlSize::Small.height_in(metrics);
    const PAD: f32 = nana_ui_core::space::MD;
    // Four of the widest spacing step: the narrowest badge still has to read
    // as a key cap rather than as a letter with a box round it.
    const MIN_WIDTH: f32 = nana_ui_core::space::XXXL * 4.0;
    let font_size = nana_ui_core::type_scale::META;
    const FONT_WEIGHT: u16 = 600;
    crate::ComponentTextRegion {
        bounds: LayoutBox {
            x: origin.x,
            y: origin.y,
            width: (measure.width(label, font_size, Some(FONT_WEIGHT)) + PAD * 2.0).max(MIN_WIDTH),
            height: height.min(origin.height.max(height)),
        },
        content: crate::TextValue::from(label),
        color: Some(if muted {
            palette.color(SemanticColorRole::Muted).as_rgba_array()
        } else {
            palette.color(SemanticColorRole::Text).as_rgba_array()
        }),
        font_size,
        font_weight: Some(FONT_WEIGHT),
    }
}

impl UiWorld {
    pub(super) fn derive_component_geometry(
        &self,
        id: StableNodeId,
        visual: &StandardVisual,
        style: &ComputedStyle,
    ) -> Option<crate::ComponentGeometry> {
        // Keep component-owned chrome on the same geometry accessor as the
        // rest of runtime interaction code.  `component_layout_box` uses the
        // published result whenever the writeback window is closed and only
        // falls back to the retained box while a pass is still publishing.
        // Reading `node.layout` directly here would create a second geometry
        // authority for component projections.
        let bounds = self.component_layout_box(id)?;
        let mut source = self.nodes.get(id)?.style.clone();
        source.layout = Arc::clone(&self.record(id).resolved_layout);
        let padding = self.used_layout_padding(id);
        let border = source.layout.resolved_border_width();
        let content = self.component_content_box(id)?;
        // Both branches now answer "what colour is this node's text, in the
        // state it is in": the primary from the resolved style, the secondary
        // from the `foreground_secondary` role the same state machine picks.
        // The secondary used to be `palette.muted` read straight off the
        // palette, which is how a focused row ended up with a grey detail line
        // on a near-solid `focus_surface` fill.
        let secondary = self.secondary_text_color(id);
        let text_region = |bounds, content: Arc<str>, muted: bool, size: f32, weight| {
            crate::ComponentTextRegion {
                bounds,
                content: content.into(),
                color: Some(if muted {
                    secondary
                } else {
                    style.color.unwrap_or_else(|| {
                        self.style_model
                            .color(SemanticColorRole::Text)
                            .as_rgba_array()
                    })
                }),
                font_size: size,
                font_weight: weight,
            }
        };
        match visual {
            StandardVisual::PathText { value } => {
                let value = crate::path_text::shorten_path(value, content.width, |text| {
                    self.chrome_text_measure(id)
                        .width(text, style.font_size, style.font_weight)
                });
                Some(crate::ComponentGeometry::PathText {
                    text: text_region(
                        content,
                        value.into(),
                        false,
                        style.font_size,
                        style.font_weight,
                    ),
                })
            }
            StandardVisual::ModalFrame {
                title,
                description,
                body_text,
                kind,
                slots,
                danger,
                surface_painter,
                ..
            } => {
                let mut presentation = self.nodes.modal_text(id).copied().unwrap_or_default();
                // A title content slot that lays out stands in the title's
                // place: the row reads its height as it would the title's,
                // and the title text is not drawn.
                let title_content = slots
                    .title_content
                    .filter(|slot| {
                        !self
                            .node_style(*slot)
                            .is_some_and(|style| style.layout.omits_box())
                    })
                    .and_then(|slot| self.component_layout_box(slot));
                if let Some(content) = title_content {
                    presentation.title = crate::TextMetrics {
                        width: content.width,
                        height: content.height,
                        ascent: None,
                    };
                }
                let recipe = self.theme.recipes().dialog();
                let chrome = crate::overlay_surfaces::ModalChrome::measure_in(
                    *kind,
                    presentation.title,
                    presentation.description,
                    slots,
                    recipe,
                    self.theme.border().hairline,
                    &self.theme,
                );
                let body_copy = presentation.body.map_or(0.0, |metrics| metrics.height);
                let body_slot = slots
                    .body
                    .and_then(|id| self.component_layout_box(id))
                    .map_or(0.0, |region| region.height);
                let body_gap = if body_copy > 0.0 && body_slot > 0.0 {
                    self.theme.spacing().md
                } else {
                    0.0
                };
                let intrinsic_height = chrome.chrome_height(body_copy + body_gap + body_slot);
                let surface = crate::overlay_surfaces::modal_surface_bounds(
                    bounds,
                    *kind,
                    Some(intrinsic_height),
                    recipe,
                );
                let text_width = chrome.text_width(surface, bounds);
                let title_type = crate::overlay_surfaces::modal_title_type(
                    *kind,
                    recipe,
                    self.theme.typography(),
                );
                let body = chrome.body_box(surface);
                let (title_x, title_y) = chrome.title_origin(surface);
                let surface_paint = surface_painter.as_ref().map(|painter| {
                    let local = |section: LayoutBox| LayoutBox {
                        x: section.x - surface.x,
                        y: section.y - surface.y,
                        ..section
                    };
                    let title_height = presentation.title.height
                        + presentation.description.map_or(0.0, |description| {
                            self.theme.spacing().xs + description.height
                        });
                    // The header band stops where its divider starts, so a
                    // shape drawn to its height meets the rule, not its far
                    // edge.
                    let header_height = chrome
                        .header_divider_box(surface)
                        .map_or(chrome.header_height, |rule| rule.y - surface.y);
                    let sections = crate::ModalSurfaceSections {
                        header: LayoutBox {
                            x: 0.0,
                            y: 0.0,
                            width: surface.width,
                            height: header_height,
                        },
                        title: local(LayoutBox {
                            x: title_x,
                            y: title_y,
                            width: text_width,
                            height: title_height,
                        }),
                        icon: chrome.icon_box(surface).map(local),
                        close: chrome.close_box(surface).map(local),
                        body: local(body),
                        footer: (chrome.footer_height > 0.0).then_some(LayoutBox {
                            x: 0.0,
                            y: surface.height - chrome.footer_height,
                            width: surface.width,
                            height: chrome.footer_height,
                        }),
                    };
                    self.modal_surface_paint(id, painter, surface, sections)
                });
                // A danger dialog speaks in the status recipe's danger tone,
                // over whatever colour the node's text would take, unless
                // the recipe marks danger some other way.
                let title_color = if *danger && recipe.danger_title {
                    self.style_model
                        .color(
                            self.theme
                                .recipes()
                                .status()
                                .role(nana_ui_core::StatusTone::Danger),
                        )
                        .as_rgba_array()
                } else {
                    style.color.unwrap_or_else(|| {
                        self.style_model
                            .color(
                                self.theme
                                    .recipes()
                                    .foreground(nana_ui_core::ComponentRecipeId::Overlay, false),
                            )
                            .as_rgba_array()
                    })
                };
                // Was `if background.r > 0.5 { 0.28 } else { 0.45 }` — the
                // modal's lift decided by sniffing how bright the page is. It
                // is an elevation token now, so each theme states it and a
                // mid-grey background no longer picks the wrong branch.
                let overlay_shadow = self.theme.shadow(nana_ui_core::ElevationRole::Overlay);
                // The scrim rides the card's move on the frame's own layer,
                // so it reaches past the frame by as much as the move and
                // the entrance scale could pull its edge in: an even colour
                // that still covers the window wherever the card is.
                let motion = recipe.motion;
                let scrim = match kind {
                    crate::ModalSurfaceKind::Drawer(_) => bounds,
                    _ if motion.moves() => {
                        let scale = motion.enter_scale.max(f32::EPSILON);
                        let reach = bounds.width.max(bounds.height) * (1.0 / scale - 1.0).max(0.0)
                            + motion.enter_offset_y.abs() / scale
                            + 1.0;
                        LayoutBox {
                            x: bounds.x - reach,
                            y: bounds.y - reach,
                            width: bounds.width + reach * 2.0,
                            height: bounds.height + reach * 2.0,
                        }
                    }
                    _ => bounds,
                };
                Some(crate::ComponentGeometry::ModalFrame {
                    scrim,
                    surface,
                    body,
                    title: crate::ComponentTextRegion {
                        color: Some(title_color),
                        ..text_region(
                            LayoutBox {
                                x: title_x,
                                y: title_y,
                                width: text_width,
                                height: presentation.title.height,
                            },
                            if title_content.is_some() {
                                Arc::from("")
                            } else {
                                Arc::clone(title)
                            },
                            false,
                            title_type.size,
                            title_type.weight,
                        )
                    },
                    description: description.as_ref().map(|description| {
                        text_region(
                            LayoutBox {
                                x: title_x,
                                y: title_y + presentation.title.height + self.theme.spacing().xs,
                                width: text_width,
                                height: presentation.description.unwrap_or_default().height,
                            },
                            Arc::clone(description),
                            true,
                            self.theme.typography().meta,
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
                            self.theme.typography().body,
                            None,
                        )
                    }),
                    background: self
                        .style_model
                        .color(SemanticColorRole::Surface)
                        .as_rgba_array(),
                    border: [0.0; 4],
                    elevation: crate::ComponentElevation::from_shadow(overlay_shadow),
                    corner_radius: match kind {
                        crate::ModalSurfaceKind::Drawer(_) => self.style_model.metrics.radius_md,
                        _ => recipe.radius.resolve(self.style_model.metrics),
                    },
                    header_divider: chrome.header_divider_box(surface).zip(
                        recipe
                            .header_divider
                            .map(|role| self.style_model.color(role).as_rgba_array()),
                    ),
                    footer_divider: chrome.footer_divider_box(surface).zip(
                        recipe
                            .footer_divider
                            .map(|role| self.style_model.color(role).as_rgba_array()),
                    ),
                    scrim_color: self.theme.effects().modal_scrim.as_rgba_array(),
                    scrim_blur: self.theme.effects().modal_scrim_blur,
                    surface_paint,
                    scrim_opacity: self.scrim_presence(id),
                })
            }
            StandardVisual::Button {
                label,
                loading,
                icon,
                trailing_icon,
                ..
            } => {
                use crate::LayoutPartKind;
                let leading = self.content_part_box(id, LayoutPartKind::LeadingContent);
                let trailing = self.content_part_box(id, LayoutPartKind::TrailingContent);
                let spinner = self.content_part_box(id, LayoutPartKind::Spinner);
                let label_bounds = self
                    .content_part_box(id, LayoutPartKind::TextContent)
                    .unwrap_or(LayoutBox {
                        width: 0.0,
                        ..content
                    });
                Some(crate::ComponentGeometry::Button {
                    icon: if *loading { None } else { icon.zip(leading) },
                    trailing_icon: trailing_icon.zip(trailing),
                    label: text_region(
                        label_bounds,
                        Arc::clone(label),
                        false,
                        style.font_size,
                        style.font_weight,
                    ),
                    spinner,
                    background: style
                        .paint_colors
                        .background
                        .map(nana_ui_core::PaintColor::to_srgb)
                        .or(style.background),
                    border: style
                        .paint_colors
                        .border
                        .map(nana_ui_core::PaintColor::to_srgb)
                        .or(style.border_color),
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
                // Fold gutters and sticky headers share the displayed logical-line
                // index. Counting every prefix separately makes many folds quadratic.
                let display_newlines: Vec<usize> = match visual {
                    StandardVisual::TextInput { folds, .. } if !folds.is_empty() => presentation
                        .display_value
                        .bytes()
                        .enumerate()
                        .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset))
                        .collect(),
                    _ => Vec::new(),
                };
                let display_line_of =
                    |offset: usize| display_newlines.partition_point(|newline| *newline < offset);
                let accessibility = self.nodes.get(id).map(|n| &n.accessibility);
                let disabled = accessibility.is_some_and(|state| state.disabled);
                // A read-only field refuses every step too, so its spinner
                // halves are inert: drawn and hit as they behave.
                let inert = disabled || accessibility.is_some_and(|state| !state.editable);
                let steppers = steppers
                    .then(|| {
                        let band = (size.height_in(self.style_model.metrics) / 2.0)
                            .min(content.height / 2.0);
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
                        let can_increment = !inert
                            && numeric.is_none_or(|(value, _, maximum)| {
                                maximum.is_none_or(|maximum| value < maximum)
                            });
                        let can_decrement = !inert
                            && numeric.is_none_or(|(value, minimum, _)| {
                                minimum.is_none_or(|minimum| value > minimum)
                            });
                        let active = self
                            .style_model
                            .color(SemanticColorRole::Muted)
                            .as_rgba_array();
                        let inert = self
                            .style_model
                            .color(SemanticColorRole::Faint)
                            .as_rgba_array();
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
                // Where the text sits in the field — the content box less the
                // steppers, the scroll, a single-line field's centring and its
                // inline-start edge — is the editor frame's, the same one a
                // pointer hit is resolved through.
                let frame = self.editor_frame(id)?;
                let content = frame.content;
                let focused = self.input.focused.get(&self.record(id).document) == Some(&id);
                let multiline = accessibility.is_some_and(|state| state.multiline);
                let line_height = frame.line;
                // Horizontal text space: `x` along the line, `y` across. A
                // single-line field has no block scroll of its own; its
                // centring is where its line top lands.
                let scroll_x = frame.inline_scroll;
                let scroll_y = if multiline { frame.block_scroll } else { 0.0 };
                let line_y = content.y - frame.block_scroll;
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
                let caret = focused.then(|| {
                    caret_rule(
                        field_x(presentation.caret_x),
                        line_y + presentation.caret_y,
                        line_height,
                    )
                });
                // 附加多光标：与主光标同形，用主光标色的半透明变体区分；
                // 只随焦点出现（多行编辑器才有附加光标）。
                let caret_color = style.color.unwrap_or_else(|| {
                    self.style_model
                        .color(SemanticColorRole::Text)
                        .as_rgba_array()
                });
                let additional_caret_color = {
                    let mut color = caret_color;
                    color[3] *= 0.55;
                    color
                };
                let additional_carets = if focused {
                    presentation
                        .additional_carets
                        .iter()
                        .map(|(x, y)| caret_rule(field_x(*x), line_y + *y, line_height))
                        .collect()
                } else {
                    Vec::new()
                };
                let marker_color = |severity| match severity {
                    crate::TextDiagnosticSeverity::Error => self
                        .style_model
                        .color(SemanticColorRole::Danger)
                        .as_rgba_array(),
                    crate::TextDiagnosticSeverity::Warning => self
                        .style_model
                        .color(SemanticColorRole::Warning)
                        .as_rgba_array(),
                    crate::TextDiagnosticSeverity::Information
                    | crate::TextDiagnosticSeverity::Hint => self
                        .style_model
                        .color(SemanticColorRole::Muted)
                        .as_rgba_array(),
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
                let diagnostic_labels = presentation
                    .diagnostic_labels
                    .iter()
                    .map(|label| crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: field_x(label.rect.x),
                            y: content.y + label.rect.y - scroll_y,
                            width: label.rect.width,
                            height: label.rect.height,
                        },
                        content: crate::TextValue::from(label.text.as_str()),
                        color: Some(marker_color(label.severity)),
                        font_size: size.text_size_in(self.theme.typography()),
                        font_weight: None,
                    })
                    .collect();
                // 查找匹配：普通匹配用 accent 软色令牌，当前匹配用同一 accent
                // 色相加深（本地强调系数，与诊断条带的 2px 常量同级的局部约定）。
                let match_color = |current: bool| -> [f32; 4] {
                    if current {
                        let accent = self
                            .style_model
                            .color(SemanticColorRole::Accent)
                            .as_rgba_array();
                        [accent[0], accent[1], accent[2], 0.45]
                    } else {
                        self.style_model
                            .color(SemanticColorRole::AccentSoftHover)
                            .as_rgba_array()
                    }
                };
                let map_chip_box = |rect: LayoutBox| LayoutBox {
                    x: field_x(rect.x),
                    y: content.y + rect.y - scroll_y,
                    width: rect.width,
                    height: rect.height,
                };
                let atom_chips = presentation
                    .atom_chips
                    .iter()
                    .map(|chip| {
                        let mut mapped = chip.clone();
                        mapped.bounds = map_chip_box(chip.bounds);
                        mapped.close = map_chip_box(chip.close);
                        mapped.icon_bounds = map_chip_box(chip.icon_bounds);
                        mapped.label.bounds = map_chip_box(chip.label.bounds);
                        if mapped.label.color.is_none() {
                            mapped.label.color = Some(
                                self.style_model
                                    .color(SemanticColorRole::Text)
                                    .as_rgba_array(),
                            );
                        }
                        mapped.background = self
                            .style_model
                            .color(SemanticColorRole::Subtle)
                            .as_rgba_array();
                        mapped.border = self
                            .style_model
                            .color(SemanticColorRole::Border)
                            .as_rgba_array();
                        mapped
                    })
                    .collect();
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
                        self.style_model
                            .color(SemanticColorRole::Hover)
                            .as_rgba_array(),
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
                                self.style_model
                                    .color(SemanticColorRole::Accent)
                                    .as_rgba_array(),
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
                                self.style_model
                                    .color(SemanticColorRole::AccentSoft)
                                    .as_rgba_array(),
                            )
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                // 空白字符标记：静态结构标记（不随焦点变化），按视口裁剪
                // ——标记只服务可见行，视口外的空白不产生图元。
                let whitespace_color = self
                    .style_model
                    .color(SemanticColorRole::Faint)
                    .as_rgba_array();
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
                            self.style_model
                                .color(SemanticColorRole::Faint)
                                .as_rgba_array(),
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
                            self.style_model
                                .color(SemanticColorRole::Border)
                                .as_rgba_array(),
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
                    crate::TextGitMarkKind::Added => self
                        .style_model
                        .color(SemanticColorRole::Success)
                        .as_rgba_array(),
                    crate::TextGitMarkKind::Modified => self
                        .style_model
                        .color(SemanticColorRole::Warning)
                        .as_rgba_array(),
                    crate::TextGitMarkKind::Deleted => self
                        .style_model
                        .color(SemanticColorRole::Danger)
                        .as_rgba_array(),
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
                            let display_line = display_line_of(display_offset);
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
                                    color: self
                                        .style_model
                                        .color(SemanticColorRole::Faint)
                                        .as_rgba_array(),
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
                                content: crate::TextValue::from(
                                    &value[head_line_start..head_line_end],
                                ),
                                color: Some(caret_color),
                                font_size: size.text_size_in(self.theme.typography()),
                                font_weight: style.font_weight,
                            },
                            background: style
                                .paint_colors
                                .background
                                .map(nana_ui_core::PaintColor::to_srgb)
                                .or(style.background)
                                .unwrap_or_else(|| {
                                    self.style_model
                                        .color(SemanticColorRole::Surface)
                                        .as_rgba_array()
                                }),
                            divider_color: self
                                .style_model
                                .color(SemanticColorRole::Border)
                                .as_rgba_array(),
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
                        self.style_model
                            .color(SemanticColorRole::Accent)
                            .as_rgba_array(),
                    )
                });
                let geometry = crate::ComponentGeometry::TextInput {
                    character_counter: self.text_input_character_counter(id),
                    resize_grip: matches!(visual, StandardVisual::TextInput { editor_options, .. } if editor_options.resize_vertical).then_some(LayoutBox {
                        x: bounds.x + (bounds.width - 14.0).max(0.0), y: bounds.y + (bounds.height - 14.0).max(0.0),
                        width: bounds.width.min(14.0), height: bounds.height.min(14.0),
                    }),
                    diagnostic_markers,
                    diagnostic_labels,
                    match_markers,
                    swatch_markers,
                    swatch_border_color: self.style_model.color(SemanticColorRole::Border).as_rgba_array(),
                    atom_chips,
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
                    line_labels_color: self.style_model.color(SemanticColorRole::Faint).as_rgba_array(),
                    line_labels_font_size: size.caption_size_in(self.theme.typography()),
                    text: crate::ComponentTextRegion {
                        bounds: frame.text_bounds(
                            presentation.content_size.width,
                            presentation.content_size.height,
                            multiline,
                        ),
                        content: presentation.display_value.clone(),
                        color: Some(if presentation.placeholder {
                            text_input_placeholder_color(
                                &source.layout,
                                self.style_model.color(SemanticColorRole::Faint).as_rgba_array(),
                            )
                        } else {
                            style
                                .color
                                .unwrap_or_else(|| self.style_model.color(SemanticColorRole::Text).as_rgba_array())
                        }),
                        font_size: size.text_size_in(self.theme.typography()),
                        font_weight: style.font_weight,
                    },
                    multiline,
                    scroll: multiline.then(|| {
                        frame.scroll_x(
                            presentation.content_size.width,
                            presentation.content_size.height,
                            multiline,
                        )
                    }),
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
                                        size.text_size_in(self.theme.typography()),
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
                                    size.text_size_in(self.theme.typography()),
                                    &self.style_model.palette,
                                )
                            })
                    },
                    signature_popup: {
                        // 签名帮助：宿主喂入即显示，锚定 caret；占位符与
                        // IME 组合期不弹出（与 hover 同一打字态门）。
                        if multiline && !presentation.placeholder && presentation.preedit.is_none()
                        {
                            self.nodes.text_signature(id).and_then(|help| {
                                signature_popup_geometry(
                                    help,
                                    OverlayAnchor {
                                        x: field_x(presentation.caret_x),
                                        line_top: line_y + presentation.caret_y,
                                        line_height,
                                    },
                                    bounds,
                                    size.text_size_in(self.theme.typography()),
                                    &self.style_model.palette,
                                    self.chrome_text_measure(id),
                                )
                            })
                        } else {
                            None
                        }
                    },
                    minimap,
                    sticky_line,
                    background: style.paint_colors.background.map(nana_ui_core::PaintColor::to_srgb).or(style.background),
                    border: style.paint_colors.border.map(nana_ui_core::PaintColor::to_srgb).or(style.border_color),
                    border_width: {
                        let width = if style.border_color.is_some() {
                            source.layout.resolved_border_width()
                        } else {
                            0.0
                        };
                        if focused && *invalid {
                            width.max(2.0)
                        } else {
                            width
                        }
                    },
                    focus_ring: None,
                    selection_color: style
                        .paint_colors
                        .selection_background
                        .map(nana_ui_core::PaintColor::to_srgb)
                        .or(style.selection_background)
                        .unwrap_or_else(|| {
                            self.style_model
                                .color(SemanticColorRole::AccentSoft)
                                .as_rgba_array()
                        }),
                    caret_color,
                    additional_caret_color,
                    preedit_color: self.style_model.color(SemanticColorRole::Accent).as_rgba_array(),
                    steppers,
                };
                Some(if frame.writing.is_vertical() {
                    vertical_text_input_geometry(geometry, &frame, presentation, multiline, focused)
                } else {
                    geometry
                })
            }
            StandardVisual::Checkbox { .. } => {
                // Both boxes are the parts layout published (#205); the label
                // of an empty checkbox has none and keeps the content box.
                use crate::LayoutPartKind;
                let indicator = self
                    .content_part_box(id, LayoutPartKind::LeadingContent)
                    .unwrap_or(LayoutBox {
                        width: 0.0,
                        height: 0.0,
                        ..content
                    });
                Some(crate::ComponentGeometry::Checkbox {
                    border_width: self.theme.border().hairline,
                    check_weight: self.theme.typography().bold,
                    indicator,
                    label: text_region(
                        self.content_part_box(id, LayoutPartKind::TextContent)
                            .unwrap_or(content),
                        self.text(id).unwrap_or_default().into(),
                        false,
                        style.font_size,
                        style.font_weight,
                    ),
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
                // The track, label and hint are the parts layout published
                // (#205); a switch with nothing beside its track has no label
                // part and draws an empty label at the content box's start.
                use crate::LayoutPartKind;
                let control = self
                    .content_part_box(
                        id,
                        if *control_position == SwitchControlPosition::End {
                            LayoutPartKind::TrailingContent
                        } else {
                            LayoutPartKind::LeadingContent
                        },
                    )
                    .unwrap_or(LayoutBox {
                        width: 0.0,
                        height: 0.0,
                        ..content
                    });
                let label_bounds =
                    self.content_part_box(id, LayoutPartKind::Label)
                        .unwrap_or(LayoutBox {
                            width: 0.0,
                            ..content
                        });
                let hint_bounds = self.content_part_box(id, LayoutPartKind::Hint);
                let palette = self.style_model;
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
                        palette
                            .color(SemanticColorRole::AccentStrong)
                            .as_rgba_array()
                    } else {
                        palette.color(SemanticColorRole::Accent).as_rgba_array()
                    }
                } else {
                    mix(
                        palette.color(SemanticColorRole::Hover).as_rgba_array(),
                        palette.color(SemanticColorRole::Background).as_rgba_array(),
                        0.78,
                    )
                };
                let track_border = if *invalid {
                    palette.color(SemanticColorRole::Danger).as_rgba_array()
                } else if *checked {
                    if hovered || pressed {
                        palette
                            .color(SemanticColorRole::AccentStrong)
                            .as_rgba_array()
                    } else {
                        palette.color(SemanticColorRole::Accent).as_rgba_array()
                    }
                } else if hovered || pressed {
                    mix(
                        palette.color(SemanticColorRole::Accent).as_rgba_array(),
                        palette
                            .color(SemanticColorRole::BorderStrong)
                            .as_rgba_array(),
                        if pressed { 0.70 } else { 0.42 },
                    )
                } else {
                    palette
                        .color(SemanticColorRole::BorderStrong)
                        .as_rgba_array()
                };
                let thumb_background = if *checked {
                    palette.color(SemanticColorRole::AccentText).as_rgba_array()
                } else {
                    mix(
                        palette.color(SemanticColorRole::Faint).as_rgba_array(),
                        palette.color(SemanticColorRole::Background).as_rgba_array(),
                        0.70,
                    )
                };
                Some(crate::ComponentGeometry::Switch {
                    border_width: self.theme.border().hairline,
                    thumb_progress: *thumb_progress,
                    label: text_region(
                        label_bounds,
                        Arc::clone(label),
                        false,
                        size.text_size_in(self.theme.typography()),
                        Some(self.theme.typography().medium),
                    ),
                    hint: hint.as_ref().zip(hint_bounds).map(|(hint, bounds)| {
                        text_region(
                            bounds,
                            Arc::clone(hint),
                            true,
                            size.caption_size_in(self.theme.typography()),
                            Some(self.theme.typography().regular),
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
                dragging,
                presence,
                expand,
                ..
            } => {
                let presence = match visibility {
                    nana_ui_core::ScrollbarVisibility::AutoHide => *presence,
                    nana_ui_core::ScrollbarVisibility::Always => 1.0,
                    nana_ui_core::ScrollbarVisibility::Hidden => return None,
                };
                // A bar that has faded out is gone: it neither paints nor
                // takes the pointer from the content under it.
                if presence.is_nan() || presence <= 0.0 {
                    return None;
                }
                let metrics = self.scroll_metrics(id)?;
                let offset = self.scroll_offset(id).unwrap_or_default();
                let skin = source.layout.paint.scrollbar;
                // The installed theme, not the constant: this is the one
                // scrollbar metric read the runtime makes, so a density change
                // has to arrive here or it never reaches a scrollbar.
                let base = self.style_model.metrics.scrollbar;
                let chrome = skin.map(|skin| skin.metrics(base)).unwrap_or(base);
                let recipe = self.theme.recipes().scrollbar();
                // The lane runs inside the border, over the padding: a
                // container that leaves end padding keeps its bar off its
                // content without the bar taking any layout.
                let edges = source.layout.resolved_border_edges();
                let padding_box = nana_ui_core::ScrollbarRect {
                    x: bounds.x + edges.left,
                    y: bounds.y + edges.top,
                    width: (bounds.width - edges.left - edges.right).max(0.0),
                    height: (bounds.height - edges.top - edges.bottom).max(0.0),
                };
                let scrolls = |axis: nana_ui_core::ScrollbarAxis| match axis {
                    nana_ui_core::ScrollbarAxis::Horizontal => {
                        axes.horizontal() && metrics.content_width > metrics.viewport_width
                    }
                    nana_ui_core::ScrollbarAxis::Vertical => {
                        axes.vertical() && metrics.content_height > metrics.viewport_height
                    }
                };
                let layout_box = |rect: nana_ui_core::ScrollbarRect| LayoutBox {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                };
                let bar = |axis: nana_ui_core::ScrollbarAxis| {
                    if !scrolls(axis) {
                        return None;
                    }
                    let horizontal = matches!(axis, nana_ui_core::ScrollbarAxis::Horizontal);
                    // Give up the far corner only when the other axis also has
                    // a bar, so the two never overlap.
                    let corner = scrolls(if horizontal {
                        nana_ui_core::ScrollbarAxis::Vertical
                    } else {
                        nana_ui_core::ScrollbarAxis::Horizontal
                    });
                    let extent = if horizontal {
                        nana_ui_core::ScrollbarExtent {
                            viewport: metrics.viewport_width,
                            content: metrics.content_width,
                            offset: offset.x,
                            min_offset: metrics.origin_x,
                        }
                    } else {
                        nana_ui_core::ScrollbarExtent {
                            viewport: metrics.viewport_height,
                            content: metrics.content_height,
                            offset: offset.y,
                            min_offset: metrics.origin_y,
                        }
                    };
                    let grown = expand[usize::from(!horizontal)];
                    let layout = nana_ui_core::scrollbar_layout(
                        axis,
                        padding_box,
                        corner,
                        extent,
                        grown,
                        chrome,
                    )?;
                    let paint = recipe.paint(
                        self.style_model,
                        skin,
                        nana_ui_core::ScrollbarPaintState {
                            presence,
                            expand: grown,
                            dragging: *dragging == Some(axis),
                            resident: matches!(
                                visibility,
                                nana_ui_core::ScrollbarVisibility::Always
                            ),
                        },
                    );
                    Some(crate::ScrollbarBar {
                        lane: layout_box(layout.lane),
                        track: layout_box(layout.track),
                        thumb: layout_box(layout.thumb),
                        track_background: paint.track,
                        thumb_background: paint.thumb,
                        track_radius: layout.track_radius,
                        thumb_radius: layout.thumb_radius,
                        min_offset: layout.axis.min_offset,
                        max_offset: layout.axis.max_offset,
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
                rail: Some(girth),
                value,
                size,
                ..
            } => Some(crate::ComponentGeometry::Range {
                border_width: self.theme.border().hairline,
                label: None,
                value: text_region(
                    // A rail draws no readout: an empty region at its end.
                    LayoutBox {
                        x: content.x + content.width,
                        y: content.y,
                        height: content.height,
                        ..LayoutBox::default()
                    },
                    Arc::clone(value),
                    false,
                    size.text_size_in(self.theme.typography()),
                    Some(self.theme.typography().medium),
                ),
                unit: None,
                // Keep the full horizontal pointer range while the thumb
                // band follows the same live type extent as an ordinary range.
                track: LayoutBox {
                    y: content.y
                        + (content.height
                            - size
                                .text_size_in(self.theme.typography())
                                .min(content.height))
                            / 2.0,
                    height: size
                        .text_size_in(self.theme.typography())
                        .min(content.height),
                    ..content
                },
                track_girth: girth.min(content.height),
                thumb_ring: None,
            }),
            StandardVisual::Range {
                label,
                value,
                unit,
                size,
                track_gradient,
                ..
            } => {
                // The spacing ladder, not a second one: these were 6 / 8 / 10
                // as bare numbers sitting next to `SpacingTokens`' own SM / MD
                // / LG, which are the same three values.
                let gap = match size {
                    nana_ui_core::ControlSize::Small => self.theme.spacing().sm,
                    nana_ui_core::ControlSize::Medium => self.theme.spacing().md,
                    nana_ui_core::ControlSize::Large => self.theme.spacing().lg,
                };
                let measure = self.chrome_text_measure(id);
                let label_width = label.as_ref().map_or(0.0, |label| {
                    measure
                        .width(label, style.font_size, style.font_weight)
                        .min(content.width * 0.28)
                });
                let show_value = !value.is_empty();
                let unit_width = if show_value {
                    unit.as_ref().map_or(0.0, |unit| {
                        measure
                            .width(
                                unit,
                                size.caption_size_in(self.theme.typography()),
                                Some(self.theme.typography().regular),
                            )
                            .min(content.width)
                    })
                } else {
                    0.0
                };
                let value_width = if show_value {
                    // Reserve four digits so ordinary value changes keep the
                    // track stable; longer readouts retain their natural width.
                    measure
                        .width(value, style.font_size, style.font_weight)
                        .max(measure.width("0000", style.font_size, style.font_weight))
                        .min((content.width - unit_width).max(0.0))
                } else {
                    0.0
                };
                let trailing_width = value_width + unit_width;
                let track_x = content.x + label_width + if label.is_some() { gap } else { 0.0 };
                let track_right = content.x + content.width
                    - trailing_width
                    - if trailing_width > 0.0 { gap } else { 0.0 };
                // A gradient track's thumb is a ring that stands proud of the
                // strip; the band is the ring's height so it fits.
                let thumb = if track_gradient.is_some() {
                    self.theme.spacing().xxxl
                } else {
                    size.text_size_in(self.theme.typography())
                }
                .min(content.height);
                let track = LayoutBox {
                    x: track_x + thumb / 2.0,
                    y: content.y + (content.height - thumb) / 2.0,
                    width: (track_right - track_x - thumb).max(0.0),
                    height: thumb.min(content.height),
                };
                Some(crate::ComponentGeometry::Range {
                    border_width: self.theme.border().hairline,
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
                            style.font_size,
                            style.font_weight,
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
                        style.font_size,
                        style.font_weight,
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
                            size.caption_size_in(self.theme.typography()),
                            Some(self.theme.typography().regular),
                        )
                    }),
                    track,
                    track_girth: if track_gradient.is_some() {
                        self.theme.spacing().xl
                    } else {
                        self.theme.spacing().xs
                    }
                    .min(track.height),
                    thumb_ring: track_gradient
                        .is_some()
                        .then(|| crate::ContrastRing::from_effects(self.theme.effects())),
                })
            }
            StandardVisual::Card { title, kind, .. } => {
                let parts = self.component_content_parts(id);
                let header =
                    crate::layout_engine::parts::part_bounds(&parts, crate::LayoutPartKind::Header);
                Some(crate::ComponentGeometry::Card {
                    title: title.as_ref().zip(header).map(|(title, bounds)| {
                        text_region(
                            bounds,
                            Arc::clone(title),
                            false,
                            crate::view_components::CARD_TITLE_SIZE,
                            Some(crate::view_components::CARD_TITLE_WEIGHT),
                        )
                    }),
                    content,
                    elevation: (*kind == nana_ui_core::CardKind::Raised).then_some(
                        crate::ComponentElevation::from_shadow(
                            self.theme.shadow(nana_ui_core::ElevationRole::Surface),
                        ),
                    ),
                    spinner: crate::layout_engine::parts::part_bounds(
                        &parts,
                        crate::LayoutPartKind::Spinner,
                    ),
                })
            }
            StandardVisual::ListItem {
                leading,
                content: content_slot,
                trailing,
                note,
                detail,
            } => {
                let slot_box = |id: StableNodeId| -> Option<LayoutBox> {
                    if self
                        .node_style(id)
                        .is_some_and(|style| style.layout.omits_box())
                    {
                        return None;
                    }
                    self.component_layout_box(id)
                };
                let parts = self.component_content_parts(id);
                let part = |kind| crate::layout_engine::parts::part_bounds(&parts, kind);
                let secondary_region = |text: &Option<Arc<str>>, kind| {
                    text.as_ref()
                        .zip(part(kind))
                        .map(|(text, bounds)| crate::ComponentTextRegion {
                            bounds,
                            content: Arc::clone(text).into(),
                            color: Some(secondary),
                            font_size: self.theme.typography().meta,
                            font_weight: Some(self.theme.typography().regular),
                        })
                };
                Some(crate::ComponentGeometry::ListItem {
                    leading: leading.and_then(slot_box),
                    content: content_slot
                        .and_then(slot_box)
                        .or_else(|| part(crate::LayoutPartKind::TextContent))
                        .or(Some(content)),
                    trailing: trailing.and_then(slot_box),
                    note: secondary_region(note, crate::LayoutPartKind::Note),
                    detail: secondary_region(detail, crate::LayoutPartKind::Detail),
                })
            }
            StandardVisual::StatusBadge {
                label,
                tone,
                compact,
            } => {
                // The dot and the label are the parts layout published (#205).
                // The label's single centred line is painted across the
                // pill's whole height, as it always was; across the content
                // box it centres at the same place.
                use crate::LayoutPartKind;
                let indicator = self
                    .content_part_box(id, LayoutPartKind::LeadingContent)
                    .unwrap_or(LayoutBox {
                        width: 0.0,
                        height: 0.0,
                        ..content
                    });
                let label_bounds = self
                    .content_part_box(id, LayoutPartKind::TextContent)
                    .map_or(content, |part| LayoutBox {
                        y: bounds.y,
                        height: bounds.height,
                        ..part
                    });
                let text_size = if *compact {
                    self.theme.typography().hint
                } else {
                    self.theme.typography().meta
                };
                let foreground = self
                    .style_model
                    .color(self.theme.recipes().status().role(*tone))
                    .as_rgba_array();
                let mut background = foreground;
                background[3] *= 0.12;
                Some(crate::ComponentGeometry::StatusBadge {
                    indicator,
                    label: crate::ComponentTextRegion {
                        bounds: label_bounds,
                        content: Arc::clone(label).into(),
                        color: Some(foreground),
                        font_size: text_size,
                        font_weight: Some(self.theme.typography().medium),
                    },
                    background,
                    foreground,
                })
            }
            StandardVisual::ValidationMessage {
                message, intent, ..
            } => {
                // The dot and the message are the parts layout published (#205).
                use crate::LayoutPartKind;
                let indicator = self
                    .content_part_box(id, LayoutPartKind::LeadingContent)
                    .unwrap_or(LayoutBox {
                        width: 0.0,
                        height: 0.0,
                        ..content
                    });
                let label_bounds = self
                    .content_part_box(id, LayoutPartKind::TextContent)
                    .unwrap_or(content);
                let foreground = self
                    .style_model
                    .color(match intent {
                        nana_ui_core::ValidationIntent::Warning => SemanticColorRole::Warning,
                        nana_ui_core::ValidationIntent::Danger => SemanticColorRole::Danger,
                    })
                    .as_rgba_array();
                Some(crate::ComponentGeometry::ValidationMessage {
                    indicator,
                    label: crate::ComponentTextRegion {
                        bounds: label_bounds,
                        content: Arc::clone(message).into(),
                        color: Some(foreground),
                        font_size: style.font_size,
                        font_weight: style.font_weight,
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
                    (
                        self.theme.spacing().sm,
                        self.theme.spacing().md,
                        self.theme.typography().meta,
                        self.theme.typography().hint,
                        self.theme.spacing().xxs,
                    )
                } else {
                    (
                        self.theme.spacing().xxxl,
                        self.theme.spacing().page,
                        self.theme.typography().body,
                        self.theme.typography().meta,
                        self.theme.spacing().sm,
                    )
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
                    let icon_width =
                        (self.theme.spacing().page_tight + self.theme.spacing().xxs).min(width);
                    let icon_bounds = LayoutBox {
                        x: if *compact {
                            bounds.x + horizontal
                        } else {
                            bounds.x + horizontal + (width - icon_width) / 2.0
                        },
                        y,
                        width: icon_width,
                        height: (self.theme.spacing().page_tight + self.theme.spacing().xxs),
                    };
                    y += (self.theme.spacing().page_tight + self.theme.spacing().xxs) + spacing;
                    (
                        icon,
                        icon_bounds,
                        self.style_model
                            .color(SemanticColorRole::Faint)
                            .as_rgba_array(),
                    )
                });
                let title_region = crate::ComponentTextRegion {
                    bounds: text_bounds(presentation.title, y),
                    content: Arc::clone(title).into(),
                    color: Some(if *compact {
                        secondary
                    } else {
                        self.style_model
                            .color(SemanticColorRole::Text)
                            .as_rgba_array()
                    }),
                    font_size: title_size,
                    font_weight: Some(self.theme.typography().semibold),
                };
                y += presentation.title.height;
                let message = message.as_ref().map(|message| {
                    y += spacing;
                    crate::ComponentTextRegion {
                        bounds: text_bounds(presentation.message.unwrap_or_default(), y),
                        content: Arc::clone(message).into(),
                        color: Some(secondary),
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
                    action: action.and_then(|action| self.component_layout_box(action)),
                })
            }
            StandardVisual::LabeledValue {
                label,
                value,
                value_is_path,
                value_role,
                value_weight,
                action,
                ..
            } => {
                let parts = self.component_content_parts(id);
                let label_bounds =
                    crate::layout_engine::parts::part_bounds(&parts, crate::LayoutPartKind::Label)?;
                let value_bounds =
                    crate::layout_engine::parts::part_bounds(&parts, crate::LayoutPartKind::Value)?;
                let value_size = self.theme.typography().meta;
                Some(crate::ComponentGeometry::LabeledValue {
                    label: crate::ComponentTextRegion {
                        bounds: label_bounds,
                        content: Arc::clone(label).into(),
                        color: Some(
                            self.style_model
                                .color(SemanticColorRole::Faint)
                                .as_rgba_array(),
                        ),
                        font_size: self.theme.typography().hint,
                        font_weight: None,
                    },
                    value: crate::ComponentTextRegion {
                        bounds: value_bounds,
                        content: if *value_is_path {
                            let measure = self.chrome_text_measure(id);
                            crate::path_text::shorten_path(value, value_bounds.width, |text| {
                                measure.width(text, value_size, Some(*value_weight))
                            })
                            .into()
                        } else {
                            Arc::clone(value).into()
                        },
                        color: Some(self.style_model.color(*value_role).as_rgba_array()),
                        font_size: value_size,
                        font_weight: Some(*value_weight),
                    },
                    action: action
                        .filter(|id| {
                            self.node_style(*id)
                                .is_none_or(|style| !style.layout.omits_box())
                        })
                        .and_then(|action| self.component_layout_box(action)),
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
                let icon_extent = if *indicator {
                    size.icon_size()
                } else {
                    style.font_size
                }
                .min(content.height);
                let base_padding = if *indicator {
                    size.radio_lead()
                } else {
                    size.padding_x_in(self.style_model.metrics) + self.theme.spacing().xxs
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
                        let faint = self
                            .style_model
                            .color(SemanticColorRole::Faint)
                            .as_rgba_array();
                        (faint, faint)
                    } else if *selected {
                        let accent = self
                            .style_model
                            .color(SemanticColorRole::Accent)
                            .as_rgba_array();
                        (accent, accent)
                    } else {
                        (
                            self.style_model
                                .color(SemanticColorRole::BorderStrong)
                                .as_rgba_array(),
                            self.style_model
                                .color(SemanticColorRole::Accent)
                                .as_rgba_array(),
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
                        style.color.unwrap_or_else(|| {
                            self.style_model
                                .color(SemanticColorRole::Muted)
                                .as_rgba_array()
                        }),
                    )
                });
                // The label is the content box: `SegmentedOption::project`
                // already spent the icon and its gap in the leading padding, so
                // re-deriving the label origin from the icon only invents a
                // second gap that layout never reserved.
                Some(crate::ComponentGeometry::SelectionOption {
                    icon: icon_bounds,
                    label: text_region(
                        LayoutBox {
                            x: content.x,
                            y: content.y,
                            width: content.width,
                            height: content.height,
                        },
                        Arc::clone(label),
                        false,
                        style.font_size,
                        style.font_weight,
                    ),
                    // The radio ring is the one indicator that predates the
                    // focus tokens, and it follows the same rule: a clicked
                    // control is focused and does not announce it.
                    focus_ring: (*show_focus_ring
                        && self.focus_visible(self.record(id).document) == Some(id))
                    .then(|| {
                        self.style_model
                            .color(SemanticColorRole::FocusBorder)
                            .as_rgba_array()
                    }),
                    indicator: ring,
                })
            }
            StandardVisual::Progress {
                value_ratio,
                label,
                cancellable,
                mode,
                thickness,
                ..
            } => progress_geometry(
                bounds,
                style,
                *value_ratio,
                *mode,
                *thickness,
                // A pill, derived — not the literal `3.0` it used to be. The
                // two were equal and free to drift, and only `LevelMeter`
                // right below said so in code.
                *thickness / 2.0,
                label.as_ref(),
                *cancellable,
                self.style_model
                    .color(SemanticColorRole::Text)
                    .as_rgba_array(),
                self.theme.spacing().sm,
            ),
            StandardVisual::LevelMeter {
                value_ratio, girth, ..
            } => {
                let girth = if girth.is_finite() && *girth > 0.0 {
                    *girth
                } else {
                    self.theme.spacing().xs
                };
                progress_geometry(
                    bounds,
                    style,
                    *value_ratio,
                    crate::ProgressMode::Value,
                    girth,
                    girth / 2.0,
                    None,
                    false,
                    [0.0; 4],
                    self.theme.spacing().xs,
                )
            }
            StandardVisual::FormField {
                label,
                hint,
                error,
                size,
                control,
            } => {
                let parts = self.component_content_parts(id);
                form_field_geometry(
                    bounds,
                    *size,
                    label,
                    hint.as_ref(),
                    error.as_ref(),
                    *control,
                    &parts,
                    &|id| self.component_layout_box(id),
                    self.style_model,
                )
            }
            StandardVisual::Toast {
                title,
                description,
                tone,
                dismissible,
                assembled,
                icon,
            } => {
                // The recipe the layout side reserved by, so the two cannot
                // drift; the dismiss reserve follows the themeable control
                // height of its size rather than a literal.
                let recipe = self.theme.recipes().toast();
                let pad_x = recipe.padding.inline;
                let indicator = recipe.indicator.size();
                let glyph = match recipe.indicator {
                    nana_ui_core::ToastIndicator::Glyph { glyphs, .. } => {
                        Some(icon.unwrap_or_else(|| glyphs.get(*tone)))
                    }
                    _ => None,
                };
                let elevation = recipe
                    .elevation
                    .map(|role| crate::ComponentElevation::from_shadow(self.theme.shadow(role)));
                if *assembled {
                    // The copy is laid out inside the border and the padding
                    // the toast reserves the mark's square in; the mark
                    // stands in the title's first line, not in the middle
                    // of a toast that wraps.
                    let line = crate::toast::line_box(recipe.title.size);
                    return Some(crate::ComponentGeometry::Toast {
                        indicator: (indicator > 0.0).then(|| LayoutBox {
                            x: bounds.x + border + pad_x,
                            y: bounds.y + border + recipe.padding.top + (line - indicator) / 2.0,
                            width: indicator,
                            height: indicator,
                        }),
                        glyph,
                        title: None,
                        description: None,
                        dismiss: None,
                        elevation,
                    });
                }
                // No mark, no gap after it: the copy starts at the inset.
                let gap = match recipe.indicator {
                    nana_ui_core::ToastIndicator::None => 0.0,
                    _ => recipe.gap,
                };
                let dismiss = if *dismissible {
                    recipe.control_size.height_in(self.style_model.metrics)
                } else {
                    0.0
                };
                let copy_x = bounds.x + pad_x + indicator + gap;
                let copy_right = bounds.x + bounds.width - pad_x - dismiss;
                let copy_width = (copy_right - copy_x).max(0.0);
                let title_height = recipe.title.size;
                let desc_height = recipe.description.size;
                let title_y = if description.is_some() {
                    bounds.y + recipe.padding.top
                } else {
                    bounds.y + (bounds.height - title_height) / 2.0
                };
                Some(crate::ComponentGeometry::Toast {
                    indicator: (indicator > 0.0).then(|| LayoutBox {
                        x: bounds.x + pad_x,
                        y: bounds.y + (bounds.height - indicator) / 2.0,
                        width: indicator,
                        height: indicator,
                    }),
                    glyph,
                    title: Some(crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: copy_x,
                            y: title_y,
                            width: copy_width,
                            height: title_height,
                        },
                        content: Arc::clone(title).into(),
                        color: Some(
                            self.style_model
                                .color(SemanticColorRole::Text)
                                .as_rgba_array(),
                        ),
                        font_size: recipe.title.size,
                        font_weight: recipe.title.weight,
                    }),
                    description: description.as_ref().map(|description| {
                        crate::ComponentTextRegion {
                            bounds: LayoutBox {
                                x: copy_x,
                                y: title_y + title_height + recipe.copy_gap,
                                width: copy_width,
                                height: desc_height,
                            },
                            content: Arc::clone(description).into(),
                            color: Some(secondary),
                            font_size: recipe.description.size,
                            font_weight: recipe.description.weight,
                        }
                    }),
                    dismiss: dismissible.then(|| LayoutBox {
                        x: bounds.x + bounds.width - pad_x - dismiss,
                        y: bounds.y + (bounds.height - dismiss) / 2.0,
                        width: dismiss,
                        height: dismiss,
                    }),
                    elevation,
                })
            }
            StandardVisual::XYPad {
                nx, ny, picture, ..
            } => {
                let pad = bounds;
                // A ring has to be wide enough to stay open around the colour
                // it marks; a dot only has to be seen.
                let thumb = if *picture {
                    self.theme.spacing().xxl
                } else {
                    self.theme.spacing().md
                };
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
                    h_axis: (!*picture).then(|| LayoutBox {
                        x: pad.x,
                        y: pad.y + pad.height / 2.0 - nana_ui_core::HAIRLINE / 2.0,
                        width: pad.width,
                        height: nana_ui_core::HAIRLINE,
                    }),
                    v_axis: (!*picture).then(|| LayoutBox {
                        x: pad.x + pad.width / 2.0 - nana_ui_core::HAIRLINE / 2.0,
                        y: pad.y,
                        width: nana_ui_core::HAIRLINE,
                        height: pad.height,
                    }),
                    thumb_ring: picture
                        .then(|| crate::ContrastRing::from_effects(self.theme.effects())),
                    background: style.background,
                    border: style.border_color,
                    border_width: if style.border_color.is_some() {
                        source.layout.resolved_border_width()
                    } else {
                        0.0
                    },
                    thumb_color: self
                        .style_model
                        .color(SemanticColorRole::Accent)
                        .as_rgba_array(),
                    axis_color: self
                        .style_model
                        .color(SemanticColorRole::Border)
                        .as_rgba_array(),
                })
            }
            StandardVisual::Select {
                label,
                hint,
                placeholder,
                size,
                opened,
                options,
                highlighted,
                menu_scroll,
                menu_room,
                checkable,
                fit_options,
                ..
            } => {
                let parts = self.component_content_parts(id);
                Some(crate::select::select_geometry(
                    bounds,
                    label,
                    hint.as_ref(),
                    *placeholder,
                    *size,
                    *opened,
                    options,
                    *highlighted,
                    *menu_scroll,
                    *menu_room,
                    style,
                    &source,
                    self.style_model,
                    self.theme.recipes().scrollbar(),
                    self.document_viewport_of(id),
                    *checkable,
                    &parts,
                    self.chrome_text_measure(id),
                    // The widest option layout measured for the field: its natural
                    // width, or the label it shows when that is the widest.
                    fit_options
                        .then(|| {
                            self.text_natural_width(id)
                                .or_else(|| self.text_metrics(id).map(|metrics| metrics.width))
                        })
                        .flatten(),
                ))
            }
            StandardVisual::MenuSurface {
                kind: crate::MenuSurfaceKind::ContextMenu,
                query,
                rows,
                highlighted,
                ..
            } if query.is_some() || !rows.is_empty() => {
                Some(crate::menus::context_menu_geometry_in(
                    bounds,
                    query.as_ref(),
                    rows,
                    *highlighted,
                    &self.style_model.palette,
                    self.style_model.metrics,
                    &self.framework_strings().menu_search,
                    self.chrome_text_measure(id),
                    &self.theme,
                ))
            }
            StandardVisual::MenuSurface {
                trigger,
                trigger_icon,
                trigger_image,
                overlay,
                ..
            } => {
                let trigger_content = overlay.and_then(|overlay| overlay.trigger_content);
                let has_trigger = trigger.is_some()
                    || trigger_icon.is_some()
                    || trigger_image.is_some()
                    || trigger_content.is_some();
                let surface = if has_trigger {
                    crate::popover::overlay_surface_from_items(self, id, overlay.as_ref())
                } else {
                    bounds
                };
                Some(crate::popover::menu_surface_geometry(
                    bounds,
                    trigger.as_ref(),
                    *trigger_icon,
                    overlay.and_then(|overlay| overlay.trigger_icon_size),
                    trigger_image.as_ref(),
                    trigger_content.is_some(),
                    style,
                    &self.style_model.palette,
                    self.style_model.metrics,
                    surface,
                ))
            }
            StandardVisual::ActionMenuItem {
                label,
                hint,
                icon,
                danger,
                disabled,
                size,
                ..
            } => {
                let parts = self.component_content_parts(id);
                Some(crate::menus::action_menu_item_geometry_from_parts(
                    label,
                    hint.as_ref(),
                    *icon,
                    *danger,
                    *disabled,
                    *size,
                    style,
                    &self.style_model.palette,
                    &self.theme,
                    &parts,
                ))
            }
            StandardVisual::TreeView {
                rows,
                size,
                columns,
                drop,
                ..
            } => Some(crate::tree_view::tree_view_geometry(
                bounds,
                rows,
                *size,
                columns,
                drop.as_ref(),
                &self.style_model.palette,
                self.style_model.metrics,
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
                self.style_model.metrics,
                self.chrome_text_measure(id),
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
                &self.style_model.palette,
                self.chrome_text_measure(id),
            )),
            #[cfg(feature = "charts")]
            StandardVisual::Chart { spec, hover } => {
                Some(self.chart_geometry(id, bounds, spec, hover))
            }
            #[cfg(feature = "charts")]
            StandardVisual::ChartTooltip { content } => {
                Some(self.chart_tooltip_geometry(id, bounds, content))
            }
            #[cfg(feature = "controls")]
            StandardVisual::ReorderList {
                rows,
                size,
                spacing,
                drag,
            } => {
                // Live row children own their label painting; a stale retained
                // `rows` (projected before the children were attached) must not
                // paint a second copy underneath them. They also own the row
                // boxes the drag feedback is placed against.
                let children = self
                    .nodes
                    .get(id)
                    .map(|node| node.hierarchy.children.clone())
                    .unwrap_or_default();
                let live = !children.is_empty();
                let painted = if live {
                    Arc::<[crate::ReorderRowPaint]>::from([])
                } else {
                    rows.clone()
                };
                let row_boxes = |count: usize| -> Vec<LayoutBox> {
                    if live {
                        children
                            .iter()
                            .filter_map(|child| self.component_layout_box(*child))
                            .collect()
                    } else {
                        let height = size.height_in(self.style_model.metrics);
                        let step = height + spacing.max(0.0);
                        (0..count)
                            .map(|index| LayoutBox {
                                x: bounds.x,
                                y: bounds.y + index as f32 * step,
                                width: bounds.width,
                                height,
                            })
                            .collect()
                    }
                };
                // A previewed drag shows itself on its rows' presentation:
                // the carried rows follow the pointer, the rest make room.
                let drag = drag.as_ref().filter(|drag| !drag.preview).map(|drag| {
                    let boxes = row_boxes(drag.rows());
                    (drag.drop_mark(bounds, &boxes), drag.carried_box(&boxes))
                });
                Some(reorder_list_geometry(
                    bounds,
                    &painted,
                    *size,
                    *spacing,
                    drag.unwrap_or_default(),
                    &self.style_model.palette,
                    self.style_model.metrics,
                ))
            }
            #[cfg(feature = "rich-text")]
            StandardVisual::NativeMarkdown {
                text,
                blocks,
                selection,
            } => {
                let selection_range = selection;
                let (text, selection_color) =
                    selectable_text_regions(content, text, style, &self.style_model.palette);
                let geometry = self.markdown_layout(id, blocks, content);
                Some(crate::ComponentGeometry::NativeMarkdown {
                    drawing: crate::markdown_drawing::document_with_geometry(
                        blocks,
                        content,
                        &geometry,
                        self.style_model.metrics,
                        &|scene, scale| self.diagram_drawing(scene, scale),
                    ),
                    text,
                    selection: match selection_range {
                        Some((start, end)) => geometry
                            .blocks
                            .into_iter()
                            .flat_map(|block| block.graphemes)
                            .filter(|g| g.index >= *start.min(end) && g.index < *start.max(end))
                            .map(|g| g.bounds)
                            .collect(),
                        _ => Vec::new(),
                    },
                    selection_color,
                })
            }
            #[cfg(feature = "rich-text")]
            StandardVisual::SelectableRichText { text, selection } => {
                let selection = self.selectable_rich_text_highlights(id, content, text, *selection);
                let (text, selection_color) =
                    selectable_text_regions(content, text, style, &self.style_model.palette);
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
                self.style_model.opacity,
            )),
            #[cfg(feature = "image-viewer")]
            StandardVisual::ImageViewer {
                intrinsic_size,
                name,
                metadata,
                zoom,
                offset_x,
                offset_y,
            } => Some(image_viewer_geometry(
                bounds,
                *intrinsic_size,
                name.as_ref(),
                metadata.as_ref(),
                *zoom,
                *offset_x,
                *offset_y,
                &self.style_model.palette,
                self.theme.effects().media_scrim,
                self.style_model.metrics,
            )),
            StandardVisual::KeyCaptureLayer { recording } => Some(key_capture_geometry(
                content,
                *recording,
                self.style_model,
                self.chrome_text_measure(id),
            )),
            StandardVisual::KeymapLayer => Some(keymap_geometry(
                content,
                self.style_model,
                self.chrome_text_measure(id),
            )),
            _ => None,
        }
    }

    /// The open Select, Dropdown and SearchDropdown menus of `document`
    /// whose room is no longer the one [`Self::select_menu_room`] gives
    /// now, each with its visual carrying the room it has now.
    ///
    /// A field projects its menu's room from the boxes the last layout gave
    /// it; a layout pass that moves those boxes or the viewport, but not the
    /// field's own state, leaves that room behind. Fed back after every
    /// layout pass, the menu that is painted, hit and read is placed against
    /// the layout just published.
    pub(crate) fn stale_select_menu_rooms(
        &self,
        document: DocumentId,
    ) -> Vec<(StableNodeId, StandardVisual)> {
        let mut stale = self
            .nodes
            .open_select_menus()
            .filter(|id| self.document_of(*id) == Some(document) && self.is_mounted(*id))
            .filter_map(|id| {
                let Some(StandardVisual::Select { menu_room, .. }) = self.nodes.visual(id) else {
                    return None;
                };
                let room = self.select_menu_room(id);
                (*menu_room != room).then(|| {
                    let mut visual = self.nodes.visual(id).cloned()?;
                    if let StandardVisual::Select { menu_room, .. } = &mut visual {
                        *menu_room = room;
                    }
                    Some((id, visual))
                })?
            })
            .collect::<Vec<_>>();
        stale.sort_unstable_by_key(|(id, _)| *id);
        stale
    }

    /// Where a menu hanging off `id` can be seen, in `id`'s own layout
    /// coordinates: the window and every clip on `id`'s clip chain, each moved
    /// through the scroll offsets between that box and `id`. The chain is the
    /// one paint and hit testing follow ([`Self::clip_ancestors`], #201): an
    /// escaped box passes over the containers it escapes, and a `fixed` box,
    /// the field or one above it, ends it. Transforms are left out: they bend
    /// where the menu is placed, not whether what it draws matches what it
    /// hits.
    ///
    /// `None` until the document is laid out, or when nothing is left to see.
    pub(crate) fn select_menu_room(&self, id: StableNodeId) -> Option<crate::SelectMenuRoom> {
        let viewport = self.document_viewport_of(id)?;
        let mut room = crate::SelectMenuRoom {
            top: f32::NEG_INFINITY,
            bottom: f32::INFINITY,
            left: f32::NEG_INFINITY,
            right: f32::INFINITY,
        };
        let mut clip = |bounds: LayoutBox, shift: crate::ScrollOffset| {
            room.top = room.top.max(bounds.y + shift.y);
            room.bottom = room.bottom.min(bounds.y + bounds.height + shift.y);
            room.left = room.left.max(bounds.x + shift.x);
            room.right = room.right.min(bounds.x + bounds.width + shift.x);
        };
        // Scroll offsets between the box being looked at and `id`.
        let mut shift = crate::ScrollOffset::default();
        // The link of the chain below `ancestor`.
        let mut below = id;
        for ancestor in self.clip_ancestors(id) {
            let offset = self.scroll_offset(ancestor).unwrap_or_default();
            shift.x += offset.x;
            shift.y += offset.y;
            let style = self.hit_motion_layout(ancestor);
            let layout = self.component_layout_box(ancestor)?;
            if let Some((x, y, width, height)) =
                style.overflow_clip_box(layout.x, layout.y, layout.width, layout.height)
            {
                clip(
                    LayoutBox {
                        x,
                        y,
                        width,
                        height,
                    },
                    shift,
                );
            }
            if self.clip_visuals != 0 {
                match self.nodes.visual(ancestor) {
                    Some(StandardVisual::EmptyState { .. }) => clip(layout, shift),
                    Some(StandardVisual::ModalFrame { slots, .. }) => {
                        let body_root = slots.body == Some(below);
                        if let Some(crate::ComponentGeometry::ModalFrame {
                            surface, body, ..
                        }) = self.component_geometry(ancestor)
                        {
                            clip(surface, shift);
                            if body_root {
                                clip(body, shift);
                            }
                        }
                    }
                    _ => {}
                }
            }
            below = ancestor;
        }
        clip(
            LayoutBox {
                width: viewport.width,
                height: viewport.height,
                ..LayoutBox::default()
            },
            shift,
        );
        (room.top < room.bottom && room.left < room.right).then_some(room)
    }
}

/// A vertical editor's component geometry (#59): the value, caret, selection
/// and preedit turned from text space onto the page through `frame`, and the
/// code-editor chrome dropped.
///
/// Gutters, guides, minimaps, inline diagnostics and anchored popups are laid
/// out against horizontal lines — a line-number column, a label after the
/// line's end, a popup under the caret's line — and there is no honest column
/// version of them to fall back to, so a vertical editor has none rather than
/// ones drawn across its columns.
fn vertical_text_input_geometry(
    geometry: crate::ComponentGeometry,
    frame: &EditorFrame,
    presentation: &TextInputPresentation,
    multiline: bool,
    focused: bool,
) -> crate::ComponentGeometry {
    let crate::ComponentGeometry::TextInput {
        resize_grip,
        character_counter,
        mut text,
        additional_caret_color,
        swatch_border_color,
        whitespace_color,
        line_labels_color,
        line_labels_font_size,
        background,
        border,
        border_width,
        focus_ring,
        selection_color,
        caret_color,
        preedit_color,
        steppers,
        ..
    } = geometry
    else {
        return geometry;
    };
    let line = presentation.line_height.max(1.0);
    let caret_at = |(inline, block): (f32, f32)| frame.field_rect(caret_rule(inline, block, line));
    // Along the columns is the page's height, across them its width.
    let (inline_extent, block_extent) = (
        presentation.content_size.height,
        presentation.content_size.width,
    );
    text.bounds = frame.text_bounds(inline_extent, block_extent, multiline);
    // The preedit underline runs beside the column, on its block-start side:
    // the right of a `vertical-rl` column.
    let preedit = presentation
        .preedit_lines
        .iter()
        .map(|rect| frame.field_rect(preedit_rule(*rect, false)))
        .collect();
    crate::ComponentGeometry::TextInput {
        resize_grip,
        character_counter,
        text,
        multiline,
        scroll: multiline.then(|| frame.scroll_x(inline_extent, block_extent, multiline)),
        selection: presentation
            .selection_lines
            .iter()
            .map(|rect| frame.field_rect(*rect))
            .collect(),
        caret: focused.then(|| caret_at((presentation.caret_x, presentation.caret_y))),
        additional_carets: if focused {
            presentation
                .additional_carets
                .iter()
                .copied()
                .map(caret_at)
                .collect()
        } else {
            Vec::new()
        },
        additional_caret_color,
        preedit,
        diagnostic_markers: Vec::new(),
        diagnostic_labels: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color,
        atom_chips: Vec::new(),
        caret_line: None,
        bracket_markers: Vec::new(),
        occurrence_markers: Vec::new(),
        drop_indicator: None,
        whitespace_marks: Vec::new(),
        whitespace_color,
        wrap_guides: Vec::new(),
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        folds: crate::components::TextFoldGeometry::default(),
        git_marks: crate::components::TextGitGutterGeometry::default(),
        line_labels_color,
        line_labels_font_size,
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        minimap: None,
        sticky_line: None,
        background,
        border,
        border_width,
        focus_ring,
        selection_color,
        caret_color,
        preedit_color,
        steppers,
    }
}
