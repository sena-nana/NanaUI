//! Backend-neutral drawings for Markdown mathematics and diagrams.
use crate::{LayoutBox, MarkdownBlock, MarkdownBlockKind};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownDrawingCommand {
    Image {
        bounds: LayoutBox,
        source: Arc<str>,
    },
    Svg {
        bounds: LayoutBox,
        source: Arc<str>,
    },
    Text {
        bounds: LayoutBox,
        text: Arc<str>,
        size: f32,
        weight: u16,
        italic: bool,
        line_through: bool,
        underline: bool,
        code: bool,
        /// A theme role already resolved (a code token, an alert title);
        /// `None` paints in the node's text color.
        color: Option<[f32; 4]>,
    },
    Line {
        points: Vec<[f32; 2]>,
        width: f32,
        /// `None` strokes in the node's text color.
        color: Option<[f32; 4]>,
    },
    Box {
        bounds: LayoutBox,
        radius: f32,
    },
    /// A filled, outlined surface behind a block, such as a code fence or an
    /// alert tint (`color: Some(...)`).
    Panel {
        bounds: LayoutBox,
        radius: f32,
        /// `None` uses the node's text color at low opacity with a border
        /// (code fences); `Some` paints a borderless tinted wash (GFM alerts).
        color: Option<[f32; 4]>,
    },
    /// A native diagram, recorded at the size it is shown, drawn at
    /// `bounds`' origin.
    Diagram {
        bounds: LayoutBox,
        drawing: crate::markdown_diagram::DiagramDrawing,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkdownDrawing {
    pub width: f32,
    pub height: f32,
    pub commands: Vec<MarkdownDrawingCommand>,
}

pub(crate) fn text_advance(grapheme: &str, size: f32) -> f32 {
    if grapheme.is_ascii() {
        grapheme.len() as f32 * size * 0.6
    } else {
        size
    }
}

pub(crate) fn text_style(kind: MarkdownBlockKind) -> (f32, u16) {
    match kind {
        MarkdownBlockKind::Heading(level) => (
            (24.0 - f32::from(level.saturating_sub(1)) * 1.5).max(16.0) * 0.85,
            600,
        ),
        _ => (13.0, 400),
    }
}

impl MarkdownDrawing {
    fn text(&mut self, x: f32, y: f32, text: impl Into<Arc<str>>, size: f32, weight: u16) {
        let text = text.into();
        use unicode_segmentation::UnicodeSegmentation;
        let width = text
            .graphemes(true)
            .map(|grapheme| text_advance(grapheme, size))
            .sum::<f32>();
        self.commands.push(MarkdownDrawingCommand::Text {
            bounds: LayoutBox {
                x,
                y,
                width: width.max(1.0),
                height: size * 1.35,
            },
            text,
            size,
            weight,
            italic: false,
            line_through: false,
            underline: false,
            code: false,
            color: None,
        });
    }
    fn line(&mut self, points: Vec<[f32; 2]>, width: f32) {
        self.commands.push(MarkdownDrawingCommand::Line {
            points,
            width,
            color: None,
        });
    }
    fn append(&mut self, other: &Self, x: f32, y: f32, scale: f32) {
        for command in &other.commands {
            self.commands.push(match command {
                MarkdownDrawingCommand::Image { bounds, source } => MarkdownDrawingCommand::Image {
                    bounds: transform_box(*bounds, x, y, scale),
                    source: source.clone(),
                },
                MarkdownDrawingCommand::Svg { bounds, source } => MarkdownDrawingCommand::Svg {
                    bounds: transform_box(*bounds, x, y, scale),
                    source: source.clone(),
                },
                MarkdownDrawingCommand::Text {
                    bounds,
                    text,
                    size,
                    weight,
                    italic,
                    line_through,
                    underline,
                    code,
                    color,
                } => MarkdownDrawingCommand::Text {
                    bounds: transform_box(*bounds, x, y, scale),
                    text: text.clone(),
                    size: size * scale,
                    weight: *weight,
                    italic: *italic,
                    line_through: *line_through,
                    underline: *underline,
                    code: *code,
                    color: *color,
                },
                MarkdownDrawingCommand::Line {
                    points,
                    width,
                    color,
                } => MarkdownDrawingCommand::Line {
                    points: points
                        .iter()
                        .map(|p| [x + p[0] * scale, y + p[1] * scale])
                        .collect(),
                    width: width * scale,
                    color: *color,
                },
                MarkdownDrawingCommand::Box { bounds, radius } => MarkdownDrawingCommand::Box {
                    bounds: transform_box(*bounds, x, y, scale),
                    radius: radius * scale,
                },
                MarkdownDrawingCommand::Panel {
                    bounds,
                    radius,
                    color,
                } => MarkdownDrawingCommand::Panel {
                    bounds: transform_box(*bounds, x, y, scale),
                    radius: radius * scale,
                    color: *color,
                },
                // Diagrams are placed once, at the top level, and never
                // nested in another drawing.
                MarkdownDrawingCommand::Diagram { bounds, drawing } => {
                    MarkdownDrawingCommand::Diagram {
                        bounds: transform_box(*bounds, x, y, 1.0),
                        drawing: drawing.clone(),
                    }
                }
            });
        }
    }
    pub(crate) fn placed(&self, bounds: LayoutBox) -> Self {
        let scale = (bounds.width.max(1.0) / self.width.max(1.0)).min(1.0);
        let mut output = Self {
            width: self.width * scale,
            height: self.height * scale,
            ..Self::default()
        };
        output.append(
            self,
            bounds.x + (bounds.width - output.width).max(0.0) * 0.5,
            bounds.y,
            scale,
        );
        output
    }
}
fn transform_box(b: LayoutBox, x: f32, y: f32, scale: f32) -> LayoutBox {
    LayoutBox {
        x: x + b.x * scale,
        y: y + b.y * scale,
        width: b.width * scale,
        height: b.height * scale,
    }
}

fn cached(
    key: String,
    render: impl FnOnce() -> Option<MarkdownDrawing>,
) -> Option<MarkdownDrawing> {
    use std::collections::VecDeque;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<VecDeque<(String, Option<MarkdownDrawing>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(cache) = cache.lock()
        && let Some((_, drawing)) = cache.iter().find(|(entry, _)| entry == &key)
    {
        return drawing.clone();
    }
    let drawing = render();
    if let Ok(mut cache) = cache.lock() {
        cache.push_back((key, drawing.clone()));
        while cache.len() > 200 {
            cache.pop_front();
        }
    }
    drawing
}

pub(crate) fn math(source: &str, size: f32) -> MarkdownDrawing {
    if source.len() > 16_384 {
        return math_fallback(source, size);
    }
    cached(format!("math:{size}:{source}"), || {
        let ast = ratex_parser::parse(source).ok()?;
        let mut options = ratex_layout::LayoutOptions::default();
        options.color = ratex_types::color::Color::new(1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0, 1.0);
        if size <= 13.0 {
            options.style = ratex_types::math_style::MathStyle::Text;
        }
        let layout = ratex_layout::layout(&ast, &options);
        let list = ratex_layout::to_display_list(&layout);
        let options = ratex_svg::SvgOptions {
            font_size: f64::from(size),
            padding: 2.0,
            stroke_width: 1.0,
            embed_glyphs: true,
            ..Default::default()
        };
        let svg = ratex_svg::render_to_svg_with_color_syntax(
            &list,
            &options,
            ratex_svg::SvgColorSyntax::Rgb,
        );
        let width = (list.width * size as f64 + 4.0) as f32;
        let height = ((list.height + list.depth) * size as f64 + 4.0) as f32;
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return None;
        }
        Some(MarkdownDrawing {
            width,
            height,
            commands: vec![MarkdownDrawingCommand::Svg {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height,
                },
                source: svg.into(),
            }],
        })
    })
    .unwrap_or_else(|| math_fallback(source, size))
}

fn math_fallback(source: &str, size: f32) -> MarkdownDrawing {
    let mut d = MarkdownDrawing::default();
    d.text(0.0, 0.0, source, size, 400);
    d.width = source.chars().count() as f32 * size * 0.6;
    d.height = size * 1.35;
    d
}

/// A display formula's drawing. Diagrams are laid out by
/// [`crate::markdown_diagram`] and arrive through the block geometry.
pub(crate) fn block_drawing(block: &MarkdownBlock) -> Option<MarkdownDrawing> {
    match block {
        MarkdownBlock::DisplayMath(source) => Some(math(source, 19.0)),
        _ => None,
    }
}

pub(crate) fn document(blocks: &[MarkdownBlock], bounds: LayoutBox) -> MarkdownDrawing {
    let geometry = crate::rich_text::layout_markdown(blocks, bounds);
    let theme = nana_ui_core::builtin_theme(nana_ui_core::ThemeAppearance::Light);
    let strings = nana_ui_core::FrameworkStrings::default();
    document_with_geometry(
        blocks,
        bounds,
        &geometry,
        &MarkdownPaint {
            metrics: nana_ui_core::UI_METRICS,
            record: &|scene, scale| crate::markdown_diagram::record(scene, theme, scale),
            color: &|role| theme.style_model().color(role).as_rgba_array(),
            strings: &strings,
            code_spans: &|_, _| None,
        },
    )
}

/// What a markdown drawing reads besides its blocks and geometry: the
/// world's theme, strings and registered presenters, or their defaults.
pub(crate) struct MarkdownPaint<'a> {
    pub metrics: nana_ui_core::ThemeMetrics,
    pub record: &'a DiagramRecorder<'a>,
    /// A theme role as the color it paints.
    pub color: &'a dyn Fn(nana_ui_core::SemanticColorRole) -> [f32; 4],
    pub strings: &'a nana_ui_core::FrameworkStrings,
    /// Spans the code presenter gives a fence `(language, source)`.
    pub code_spans: &'a dyn Fn(&str, &str) -> Option<Arc<[crate::TextSpan]>>,
}

/// How a laid-out diagram becomes a recording: against the world's theme,
/// through its recording cache.
pub(crate) type DiagramRecorder<'a> = dyn Fn(&Arc<nana_ui_diagram::DiagramScene>, f32) -> crate::markdown_diagram::DiagramDrawing
    + 'a;

pub(crate) fn document_with_geometry(
    blocks: &[MarkdownBlock],
    bounds: LayoutBox,
    geometry: &crate::rich_text::MarkdownGeometry,
    paint: &MarkdownPaint<'_>,
) -> MarkdownDrawing {
    use crate::rich_text::{BlockArt, DiagramArt};
    let metrics = paint.metrics;
    let mut output = MarkdownDrawing {
        width: bounds.width,
        height: geometry.bounds.height,
        ..Default::default()
    };
    let mut previous: Option<&MarkdownBlock> = None;
    for (block, g) in blocks.iter().zip(&geometry.blocks) {
        let before = previous.replace(block);
        if let MarkdownBlock::Text {
            kind: MarkdownBlockKind::Alert { kind, opens },
            ..
        } = block
        {
            alert_chrome(&mut output, paint, g.bounds, *kind, *opens, before);
        }
        match &g.art {
            BlockArt::Diagram(DiagramArt(scene)) => {
                let scale = (g.bounds.width.max(1.0) / scene.size[0].max(1.0)).min(1.0);
                let drawing = (paint.record)(scene, scale);
                let [width, height] = drawing.size;
                output.commands.push(MarkdownDrawingCommand::Diagram {
                    bounds: LayoutBox {
                        x: g.bounds.x + (g.bounds.width - width).max(0.0) * 0.5,
                        y: g.bounds.y,
                        width,
                        height,
                    },
                    drawing,
                });
                continue;
            }
            BlockArt::Drawing(drawing) => {
                output.commands.extend(drawing.placed(g.bounds).commands);
                continue;
            }
            BlockArt::None | BlockArt::Code => {}
        }
        if matches!(block, MarkdownBlock::Code { .. }) || g.art == BlockArt::Code {
            output.commands.push(MarkdownDrawingCommand::Panel {
                bounds: g.bounds,
                radius: nana_ui_core::RadiusTier::Sm.resolve(metrics),
                color: None,
            });
        }
        if matches!(block, MarkdownBlock::Rule) {
            output.line(
                vec![
                    [g.bounds.x, g.bounds.y],
                    [g.bounds.x + g.bounds.width, g.bounds.y],
                ],
                1.0,
            );
            continue;
        }
        let (size, weight) = match block {
            MarkdownBlock::Text { kind, .. } => text_style(*kind),
            _ => (13.0, 400),
        };
        let token_colors = code_token_colors(block, g, paint);
        let token = |index: usize| token_colors.as_ref().and_then(|colors| colors[index]);
        let mut start = 0;
        while start < g.graphemes.len() {
            let first = &g.graphemes[start];
            let mut end = start + 1;
            while end < g.graphemes.len()
                && g.graphemes[end].span_index == first.span_index
                && g.graphemes[end].bounds.y == first.bounds.y
                && token(end) == token(start)
            {
                end += 1
            }
            let text = g.graphemes[start..end]
                .iter()
                .map(|v| v.grapheme.as_ref())
                .collect::<String>();
            let span = match block {
                MarkdownBlock::Text { spans, .. } => spans.get(first.span_index),
                MarkdownBlock::Table(table) => table
                    .header
                    .iter()
                    .chain(table.rows.iter().flatten())
                    .flatten()
                    .nth(first.span_index),
                _ => None,
            };
            if let Some(resource) = span
                .and_then(|span| span.image_resource.as_ref())
                .filter(|resource| resource.width > 0 && resource.height > 0)
            {
                let last = &g.graphemes[end - 1];
                output.commands.push(MarkdownDrawingCommand::Image {
                    bounds: LayoutBox {
                        x: first.bounds.x,
                        y: first.bounds.y,
                        width: last.bounds.x + last.bounds.width - first.bounds.x,
                        height: first.bounds.height,
                    },
                    source: resource.source.clone(),
                });
            } else if span.is_some_and(|span| span.inline_math) {
                let drawing = math(&text, 13.0);
                output.append(
                    &drawing,
                    first.bounds.x,
                    first.bounds.y,
                    (g.bounds.width / drawing.width.max(1.0)).min(1.0),
                )
            } else {
                output.text(
                    first.bounds.x,
                    first.bounds.y,
                    text,
                    size,
                    if span.is_some_and(|s| s.strong) {
                        700
                    } else {
                        weight
                    },
                );
                if let Some(MarkdownDrawingCommand::Text {
                    bounds,
                    italic,
                    line_through,
                    underline,
                    code,
                    color,
                    ..
                }) = output.commands.last_mut()
                {
                    *color = token(start);
                    *italic = span.is_some_and(|s| s.emphasis);
                    *line_through = span.is_some_and(|s| s.strikethrough);
                    *underline = span.is_some_and(|s| s.link.is_some());
                    // Inline code washes its own run; a fence's lines sit on
                    // the block panel instead.
                    *code = span.is_some_and(|s| s.code)
                        && !matches!(block, MarkdownBlock::Code { .. });
                    bounds.width = g.graphemes[start..end]
                        .iter()
                        .map(|item| item.bounds.x + item.bounds.width)
                        .fold(first.bounds.x, f32::max)
                        - first.bounds.x;
                }
            }
            start = end;
        }
    }
    output
}

/// The bar down an alert paragraph's left edge (bridging the gap to the
/// paragraph above in the same alert) and, on its first paragraph, the
/// title, both in the alert's color.
fn alert_chrome(
    output: &mut MarkdownDrawing,
    paint: &MarkdownPaint<'_>,
    bounds: LayoutBox,
    kind: crate::MarkdownAlertKind,
    opens: bool,
    before: Option<&MarkdownBlock>,
) {
    let color = (paint.color)(kind.color());
    let continues = !opens
        && matches!(
            before,
            Some(MarkdownBlock::Text {
                kind: MarkdownBlockKind::Alert { kind: above, .. },
                ..
            }) if *above == kind
        );
    let top = if continues {
        bounds.y - crate::rich_text::BLOCK_GAP
    } else {
        bounds.y
    };
    output.commands.push(MarkdownDrawingCommand::Panel {
        bounds: LayoutBox {
            x: bounds.x,
            y: top,
            width: bounds.width,
            height: bounds.y + bounds.height - top,
        },
        radius: if continues {
            0.0
        } else {
            nana_ui_core::RadiusTier::Sm.resolve(paint.metrics)
        },
        color: Some(color),
    });
    let x = bounds.x + crate::rich_text::ALERT_BAR_WIDTH * 0.5;
    output.commands.push(MarkdownDrawingCommand::Line {
        points: vec![[x, top], [x, bounds.y + bounds.height]],
        width: crate::rich_text::ALERT_BAR_WIDTH,
        color: Some(color),
    });
    if opens {
        let (size, _) = text_style(MarkdownBlockKind::Paragraph);
        let indent = crate::rich_text::text_indent(MarkdownBlockKind::Alert { kind, opens });
        output.text(
            bounds.x + indent,
            bounds.y,
            Arc::clone(kind.title(paint.strings)),
            size,
            600,
        );
        if let Some(MarkdownDrawingCommand::Text {
            bounds: title_bounds,
            color: title,
            ..
        }) = output.commands.last_mut()
        {
            title_bounds.width = title_bounds.width.max((bounds.width - indent).max(1.0));
            *title = Some(color);
        }
    }
}

/// Per grapheme of a fence, the color of the code presenter's span it falls
/// in, or `None` for a block that is not fenced code or a presenter that
/// colors nothing.
fn code_token_colors(
    block: &MarkdownBlock,
    geometry: &crate::rich_text::MarkdownBlockGeometry,
    paint: &MarkdownPaint<'_>,
) -> Option<Vec<Option<[f32; 4]>>> {
    use unicode_segmentation::UnicodeSegmentation;
    let MarkdownBlock::Code {
        language: Some(language),
        source,
    } = block
    else {
        return None;
    };
    if language.is_empty() || geometry.art != crate::rich_text::BlockArt::None {
        return None;
    }
    let spans = (paint.code_spans)(language, source)?;
    // Layout places every non-empty grapheme of the source in order.
    let mut colors = Vec::with_capacity(geometry.graphemes.len());
    let mut span = 0;
    let mut offset = 0;
    for grapheme in source.graphemes(true).filter(|value| !value.is_empty()) {
        while spans.get(span).is_some_and(|s| s.end <= offset) {
            span += 1;
        }
        colors.push(
            spans
                .get(span)
                .filter(|s| s.start <= offset)
                .map(|s| (paint.color)(s.color)),
        );
        offset += grapheme.len();
    }
    (colors.len() == geometry.graphemes.len()).then_some(colors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport, NativeMarkdown, Stack};

    #[test]
    fn inline_styles_reach_the_drawing_commands() {
        let markdown = NativeMarkdown::from_source(
            "*italic* ~~strike~~ `code` [link](https://example.test) **bold**",
        );
        let drawing = markdown.drawing(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 600.0,
            height: 100.0,
        });
        for (needle, flag) in [
            ("italic", 0),
            ("strike", 1),
            ("code", 2),
            ("link", 3),
            ("bold", 4),
        ] {
            assert!(drawing.commands.iter().any(|command| matches!(command, MarkdownDrawingCommand::Text { text, italic, line_through, code, underline, weight, .. } if text.as_ref() == needle && [*italic, *line_through, *code, *underline, *weight == 700][flag])));
        }
    }

    #[test]
    fn fenced_code_sits_inset_on_one_panel_without_inline_washes() {
        let markdown =
            NativeMarkdown::from_source("before\n\n```rust\nfn main() {}\nlet x = 1;\n```");
        let drawing = markdown.drawing(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 200.0,
        });
        let panels = drawing
            .commands
            .iter()
            .filter_map(|command| match command {
                MarkdownDrawingCommand::Panel { bounds, .. } => Some(*bounds),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(panels.len(), 1);
        let panel = panels[0];
        let code_lines = drawing
            .commands
            .iter()
            .filter_map(|command| match command {
                MarkdownDrawingCommand::Text {
                    bounds, text, code, ..
                } if text.contains("main") || text.contains("let") => Some((*bounds, *code)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!code_lines.is_empty());
        for (bounds, code) in code_lines {
            assert!(!code, "fence lines rely on the panel, not a per-run wash");
            assert!(bounds.x >= panel.x + crate::rich_text::CODE_PANEL_PADDING_X - 0.5);
            assert!(bounds.y >= panel.y + crate::rich_text::CODE_PANEL_PADDING_Y - 0.5);
            assert!(bounds.y + bounds.height <= panel.y + panel.height + 0.5);
        }
    }

    #[test]
    fn a_code_panel_rounds_at_the_installed_small_radius() {
        let id = DocumentId::new(82).unwrap();
        let mut context = AppContext::new();
        let markdown = context
            .create_component(
                id,
                NativeMarkdown::from_source("```rust\nfn main() {}\n```"),
            )
            .unwrap();
        context
            .layout_document(id, LayoutViewport::new(300.0, 200.0))
            .unwrap();
        let panel_radius = |context: &AppContext| {
            let drawing = context
                .world()
                .native_markdown_layout(markdown.stable_id())
                .expect("markdown layout")
                .drawing;
            drawing.commands.iter().find_map(|command| match command {
                MarkdownDrawingCommand::Panel { radius, .. } => Some(*radius),
                _ => None,
            })
        };
        assert_eq!(
            panel_radius(&context),
            Some(nana_ui_core::UI_METRICS.radius_sm)
        );

        let mut metrics = nana_ui_core::UI_METRICS;
        metrics.radius_sm = 2.0;
        assert!(
            context
                .set_style_tokens(
                    nana_ui_core::ThemeAppearance::Dark,
                    metrics,
                    nana_ui_core::SemanticPalette::dark(),
                    nana_ui_core::SemanticPalette::dark().surface,
                )
                .unwrap()
        );
        assert_eq!(
            panel_radius(&context),
            Some(2.0),
            "a code panel must round at the installed metrics, not the default"
        );
    }

    #[test]
    fn chinese_text_and_inline_formula_use_one_advance_contract() {
        let markdown = NativeMarkdown::from_source("行内公式 $E=mc^2$。");
        let bounds = LayoutBox {
            x: 317.0,
            y: 280.0,
            width: 700.0,
            height: 100.0,
        };
        let drawing = markdown.drawing(bounds);
        let MarkdownDrawingCommand::Text {
            bounds: before,
            text,
            ..
        } = &drawing.commands[0]
        else {
            panic!("leading text");
        };
        let MarkdownDrawingCommand::Svg {
            bounds: formula, ..
        } = &drawing.commands[1]
        else {
            panic!("formula");
        };
        let MarkdownDrawingCommand::Text { bounds: after, .. } = &drawing.commands[2] else {
            panic!("trailing text");
        };
        assert_eq!(text.as_ref(), "行内公式 ");
        assert!(
            before.width >= 4.0 * 13.0,
            "CJK text cannot fit in fixed 8px cells: {before:?}"
        );
        assert!(
            (formula.x - before.x - before.width).abs() < 0.001,
            "{before:?} {formula:?}"
        );
        assert!(
            (after.x - formula.x - formula.width).abs() < 0.001,
            "{formula:?} {after:?}"
        );
        let narrow = markdown.layout(LayoutBox {
            width: before.width + formula.width - 1.0,
            ..bounds
        });
        let inline = narrow.blocks[0]
            .graphemes
            .iter()
            .find(|g| g.span_index == 1)
            .unwrap();
        assert!(
            inline.bounds.y > formula.y,
            "wrap must use same visual advance"
        );
    }

    #[test]
    fn shaped_advances_determine_formula_origin_and_drawing_bounds() {
        let markdown = NativeMarkdown::from_source("Wi 中文 $x$ end");
        let bounds = LayoutBox {
            x: 10.0,
            y: 0.0,
            width: 400.0,
            height: 100.0,
        };
        let measure = |value: &str, _: f32, _: u16, _: bool| {
            use unicode_segmentation::UnicodeSegmentation as _;
            value
                .graphemes(true)
                .map(|grapheme| match grapheme {
                    "W" => 12.0,
                    "i" => 3.0,
                    " " => 4.0,
                    "中" | "文" => 13.0,
                    _ => 7.0,
                })
                .collect::<Vec<f32>>()
        };
        let geometry = crate::rich_text::layout_markdown_measured(
            markdown.blocks(),
            bounds,
            Some(&measure),
            1,
        );
        let theme = nana_ui_core::builtin_theme(nana_ui_core::ThemeAppearance::Light);
        let strings = nana_ui_core::FrameworkStrings::default();
        let drawing = document_with_geometry(
            markdown.blocks(),
            bounds,
            &geometry,
            &MarkdownPaint {
                metrics: nana_ui_core::UI_METRICS,
                record: &|scene, scale| crate::markdown_diagram::record(scene, theme, scale),
                color: &|role| theme.style_model().color(role).as_rgba_array(),
                strings: &strings,
                code_spans: &|_, _| None,
            },
        );
        let MarkdownDrawingCommand::Text { bounds: before, .. } = &drawing.commands[0] else {
            panic!("text");
        };
        let MarkdownDrawingCommand::Svg {
            bounds: formula, ..
        } = &drawing.commands[1]
        else {
            panic!("math");
        };
        assert!((before.width - 49.0).abs() < 0.001);
        assert!((formula.x - 59.0).abs() < 0.001);
        assert_eq!(geometry.blocks[0].graphemes[0].bounds.width, 12.0);
        assert_eq!(geometry.blocks[0].graphemes[1].bounds.width, 3.0);
    }

    #[test]
    fn formulas_have_native_glyph_outlines_and_structural_extents() {
        for source in [
            r"E=mc^2",
            r"\frac{1}{2}",
            r"\sqrt{x_1^2+x_2^2}",
            r"\sum_{i=0}^{n}\frac{1}{i^2}",
            r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
            r"\int_0^\infty e^{-x}\,dx",
        ] {
            let drawing = math(source, 19.0);
            let Some(MarkdownDrawingCommand::Svg { bounds, .. }) = drawing.commands.first() else {
                panic!("formula did not typeset: {source}")
            };
            assert!(bounds.width > 4.0 && bounds.height > 10.0);
        }
        assert!(math(r"\frac{1}{2}", 19.0).height > math("x", 19.0).height * 1.5);
        assert!(math("x^2", 19.0).height > math("x", 19.0).height);
    }

    #[test]
    fn every_mermaid_family_lays_out_natively() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mermaid");
        let mut count = 0;
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("mmd") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let measurer = crate::markdown_diagram::Measurer {
                advances: None,
                key: 0,
            };
            let crate::markdown_diagram::Outcome::Native(scene) =
                crate::markdown_diagram::layout(&source, &measurer)
            else {
                panic!("cannot lay out {}", path.display());
            };
            assert!(scene.size.iter().all(|v| v.is_finite() && *v > 0.0));
            assert!(!scene.items.is_empty());
            count += 1;
        }
        assert!(count >= 23);
    }

    #[test]
    fn historical_math_boundaries_tables_and_copy_are_preserved() {
        let markdown = NativeMarkdown::from_source(
            "行内 \\(E=mc^2\\) 与 \\(\\frac{1}{2}\\)。\n\n| 比例 |\n| --- |\n| \\(\\frac{1}{2}\\) |\n\n$$\na^2+b^2=c^2\n$$\n\n```mermaid\ngraph TD\nA-->B\n```",
        );
        let bounds = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 430.0,
            height: 2000.0,
        };
        let drawing = markdown.drawing(bounds);
        let count = |f: fn(&MarkdownDrawingCommand) -> bool| {
            drawing.commands.iter().filter(|c| f(c)).count()
        };
        assert_eq!(
            count(|c| matches!(c, MarkdownDrawingCommand::Svg { .. })),
            4
        );
        assert_eq!(
            count(|c| matches!(c, MarkdownDrawingCommand::Diagram { .. })),
            1
        );
        assert!(markdown.pointer_down(-1.0, -1.0, bounds));
        markdown.pointer_up(430.0, drawing.height + 20.0, bounds);
        let copied = markdown.copy_snapshot().unwrap().text;
        assert_eq!(copied, markdown.plain_text());
    }

    #[test]
    fn unreadable_diagrams_read_as_code_blocks() {
        let markdown = NativeMarkdown::from_source("```mermaid\nnotADiagram\n  x --> y\n```");
        let drawing = markdown.drawing(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 400.0,
        });
        assert!(matches!(
            drawing.commands.first(),
            Some(MarkdownDrawingCommand::Panel { .. })
        ));
        assert!(drawing.commands.iter().any(|c| matches!(c, MarkdownDrawingCommand::Text { text, .. } if text.contains("notADiagram"))));
    }

    #[test]
    fn diagrams_scale_down_into_narrow_columns_and_never_up() {
        let markdown = NativeMarkdown::from_source(
            "```mermaid\nflowchart LR\nA[A long first label] --> B[A long second label] --> C[A long third label]\n```",
        );
        let wide = markdown.drawing(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 4000.0,
            height: 400.0,
        });
        let narrow = markdown.drawing(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 400.0,
        });
        let scale = |d: &MarkdownDrawing| match d.commands.first() {
            Some(MarkdownDrawingCommand::Diagram { drawing, bounds }) => {
                (drawing.scale, bounds.width)
            }
            _ => panic!("diagram"),
        };
        assert_eq!(scale(&wide).0, 1.0);
        let (s, w) = scale(&narrow);
        assert!(s < 1.0 && (w - 200.0).abs() < 0.01);
    }

    #[test]
    fn formula_and_diagram_height_participate_in_real_layout() {
        let id = DocumentId::new(81).unwrap();
        let context = &mut AppContext::new();
        let root = context.create_component(id, Stack::column(0.0)).unwrap();
        let markdown = NativeMarkdown::from_source(
            "$$\\frac{1}{\\sqrt{x^2+1}}$$\n\n```mermaid\nflowchart TD\nA-->B-->C\n```",
        );
        let entity = context
            .create_detached_component(id, markdown.clone())
            .unwrap();
        context.append_child(root, entity).unwrap();
        context
            .layout_document(id, LayoutViewport::new(430.0, 1200.0))
            .unwrap();
        let bounds = context
            .world()
            .canonical_layout_box(entity.stable_id())
            .unwrap();
        assert!((bounds.height - markdown.layout(bounds).bounds.height).abs() < 0.5);
        assert!(
            bounds.height > 150.0,
            "diagram may not collapse into one text line"
        );
    }
    #[test]
    fn image_resolution_updates_all_occurrences_and_uses_the_same_geometry_for_paint_and_input() {
        let mut markdown = NativeMarkdown::from_source(
            "[ordinary](image.png) ![picture](image.png)\n\n| image |\n| --- |\n| ![table](image.png) |\n\n![](image.png)",
        );
        let bounds = LayoutBox {
            x: 30.0,
            y: 40.0,
            width: 100.0,
            height: 800.0,
        };
        assert!(!markdown.resolve_image("image.png", "data:image/png;base64,test", 0, 20));
        assert!(markdown.resolve_image("image.png", "data:image/png;base64,test", 200, 80));
        assert!(!markdown.resolve_image("image.png", "data:image/png;base64,test", 200, 80));
        let drawing = markdown.drawing(bounds);
        let images = drawing
            .commands
            .iter()
            .filter_map(|command| match command {
                MarkdownDrawingCommand::Image { bounds, .. } => Some(*bounds),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(images.len(), 3);
        for image in images {
            assert!(image.width <= bounds.width && image.height > 0.0);
            assert!((image.width / image.height - 2.5).abs() < 0.001);
            let (x, y) = (image.x + image.width * 0.5, image.y + image.height * 0.5);
            markdown.clear_selection();
            assert!(markdown.pointer_down(x, y, bounds));
            assert!(
                matches!(markdown.pointer_up(x, y, bounds), Some(crate::RichTextEvent::ImageActivated(image)) if image.source == "image.png")
            );
        }
        let geometry = markdown.layout(bounds);
        let link = &geometry.blocks[0].graphemes[0].bounds;
        let (x, y) = (link.x + link.width * 0.5, link.y + link.height * 0.5);
        markdown.clear_selection();
        markdown.pointer_down(x, y, bounds);
        assert!(
            matches!(markdown.pointer_up(x, y, bounds), Some(crate::RichTextEvent::LinkActivated(link)) if link.as_ref() == "image.png")
        );
    }

    fn column() -> LayoutBox {
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 800.0,
        }
    }

    /// `> [!KIND]` opens an alert: its paragraphs carry the kind, only the
    /// first opens it, a title-only alert still shows, and a plain quote
    /// stays a quote.
    #[test]
    fn gfm_alerts_parse_into_alert_paragraphs() {
        use crate::MarkdownAlertKind;
        let markdown = NativeMarkdown::from_source(concat!(
            "> [!NOTE]\n> First.\n>\n> Second.\n\n",
            "> [!WARNING]\n\n",
            "> plain quote\n\n",
            "> [!TIP]\n> tip\n",
        ));
        let kinds = markdown
            .blocks()
            .iter()
            .map(|block| match block {
                MarkdownBlock::Text { kind, .. } => *kind,
                other => panic!("text blocks only: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                MarkdownBlockKind::Alert {
                    kind: MarkdownAlertKind::Note,
                    opens: true
                },
                MarkdownBlockKind::Alert {
                    kind: MarkdownAlertKind::Note,
                    opens: false
                },
                MarkdownBlockKind::Alert {
                    kind: MarkdownAlertKind::Warning,
                    opens: true
                },
                MarkdownBlockKind::Quote,
                MarkdownBlockKind::Alert {
                    kind: MarkdownAlertKind::Tip,
                    opens: true
                },
            ]
        );
        assert!(!markdown.plain_text().contains("[!"));
    }

    /// An alert draws a bar in its color down every paragraph, joined across
    /// the gap between them, and its title (from the framework strings) once,
    /// above the first paragraph's text.
    #[test]
    fn gfm_alerts_draw_a_colored_bar_and_one_title() {
        let markdown = NativeMarkdown::from_source("> [!CAUTION]\n> One.\n>\n> Two.");
        let geometry = markdown.layout(column());
        let drawing = markdown.drawing(column());
        let theme = nana_ui_core::builtin_theme(nana_ui_core::ThemeAppearance::Light);
        let danger = theme
            .style_model()
            .color(nana_ui_core::SemanticColorRole::Danger)
            .as_rgba_array();
        let title = &nana_ui_core::FrameworkStrings::default().markdown_alert_caution;
        let titles = drawing
            .commands
            .iter()
            .filter_map(|command| match command {
                MarkdownDrawingCommand::Text {
                    text,
                    color,
                    bounds,
                    ..
                } if text == title => Some((*color, *bounds)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(titles.len(), 1, "one title per alert");
        assert_eq!(titles[0].0, Some(danger));
        let first_text = geometry.blocks[0].graphemes[0].bounds;
        assert!(titles[0].1.y + crate::rich_text::ALERT_TITLE_HEIGHT <= first_text.y + 0.01);
        let bars = drawing
            .commands
            .iter()
            .filter_map(|command| match command {
                MarkdownDrawingCommand::Line { points, color, .. } => Some((points, *color)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(bars.len(), 2);
        assert!(bars.iter().all(|(_, color)| *color == Some(danger)));
        let first_bottom = bars[0].0[1][1];
        let second_top = bars[1].0[0][1];
        assert!(
            (first_bottom - second_top).abs() < 0.01,
            "the bar runs on through the gap ({first_bottom} vs {second_top})"
        );
    }

    fn keyword_presenter(name: &'static str) -> Box<dyn crate::TextPresenter> {
        Box::new(crate::TextPresenterFn::new(
            name,
            |text: &str, request: &crate::HighlightRequest| {
                assert_eq!(request.language.as_ref(), "rust");
                text.match_indices("fn")
                    .map(|(start, word)| crate::TextSpan {
                        start,
                        end: start + word.len(),
                        color: nana_ui_core::SemanticColorRole::Keyword,
                    })
                    .collect()
            },
        ))
    }

    fn painted_runs(
        context: &AppContext,
        id: crate::StableNodeId,
    ) -> Vec<(String, Option<[f32; 4]>)> {
        context
            .world()
            .native_markdown_layout(id)
            .expect("markdown layout")
            .drawing
            .commands
            .into_iter()
            .filter_map(|command| match command {
                MarkdownDrawingCommand::Text { text, color, .. } => Some((text.to_string(), color)),
                _ => None,
            })
            .collect()
    }

    /// Fenced code paints through the registered presenter its markdown
    /// names: runs split at span edges and take the span's theme color.
    /// Without that presenter the fence stays one color; registering it
    /// later colors the fence on the next paint.
    #[test]
    fn fenced_code_is_colored_by_the_registered_presenter() {
        let document = DocumentId::new(82).unwrap();
        let mut context = AppContext::new();
        let markdown = context
            .create_component(
                document,
                NativeMarkdown::from_source("```rust\nfn main() {}\n```\n\n```\nfn plain\n```")
                    .code_presenter("test-keywords"),
            )
            .unwrap();
        let id = markdown.stable_id();
        context
            .layout_document(document, LayoutViewport::new(400.0, 400.0))
            .unwrap();
        assert!(
            painted_runs(&context, id)
                .iter()
                .all(|(_, color)| color.is_none()),
            "no presenter, no colors"
        );

        context
            .register_presenter(keyword_presenter("test-keywords"))
            .unwrap();
        let keyword = context
            .world()
            .node_style_model(id)
            .color(nana_ui_core::SemanticColorRole::Keyword)
            .as_rgba_array();
        let runs = painted_runs(&context, id);
        assert!(
            runs.iter()
                .any(|(text, color)| text == "fn" && *color == Some(keyword)),
            "the keyword is its own run in the keyword color: {runs:?}"
        );
        assert!(
            runs.iter()
                .any(|(text, color)| text.starts_with(" main") && color.is_none()),
            "the rest of the line keeps the text color: {runs:?}"
        );
        assert!(
            runs.iter()
                .any(|(text, color)| text.contains("fn plain") && color.is_none()),
            "a fence without a language is not handed to the presenter: {runs:?}"
        );
    }

    /// A click on a diagram that is shown smaller than its natural size
    /// reports it with its source; a drag across it selects instead.
    #[test]
    fn clicking_a_diagram_reports_it_and_its_placement() {
        let source = "flowchart LR\nA[A long first label] --> B[A long second label] --> C[A long third label]";
        let markdown = NativeMarkdown::from_source(&format!("Intro\n\n```mermaid\n{source}\n```"));
        let bounds = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 200.0,
            height: 800.0,
        };
        let geometry = markdown.layout(bounds);
        let placement = geometry.blocks[1]
            .diagram_placement()
            .expect("a native diagram");
        assert_eq!(placement.block_index, 1);
        assert!(placement.scale < 1.0 && placement.natural_size[0] > bounds.width);
        assert!(geometry.blocks[0].diagram_placement().is_none());

        let (x, y) = (
            placement.bounds.x + placement.bounds.width * 0.5,
            placement.bounds.y + placement.bounds.height * 0.5,
        );
        assert!(markdown.pointer_down(x, y, bounds));
        assert_eq!(
            markdown.pointer_up(x, y, bounds),
            Some(crate::RichTextEvent::DiagramActivated(
                crate::MarkdownDiagram {
                    block_index: 1,
                    source: Arc::from(source),
                }
            ))
        );

        let intro = geometry.blocks[0].graphemes[0].bounds;
        markdown.clear_selection();
        assert!(markdown.pointer_down(intro.x + 1.0, intro.y + 1.0, bounds));
        markdown.pointer_move(x, y, bounds);
        assert!(matches!(
            markdown.pointer_up(x, y, bounds),
            Some(crate::RichTextEvent::SelectionChanged(Some(_)))
        ));
    }

    /// The world answers where a laid-out node's diagram shows.
    #[test]
    fn a_laid_out_markdown_reports_its_diagram_placement() {
        let document = DocumentId::new(83).unwrap();
        let mut context = AppContext::new();
        let markdown = context
            .create_component(
                document,
                NativeMarkdown::from_source("```mermaid\nflowchart TD\nA-->B\n```\n\nText"),
            )
            .unwrap();
        let id = markdown.stable_id();
        context
            .layout_document(document, LayoutViewport::new(400.0, 600.0))
            .unwrap();
        let world = context.world();
        let placement = world
            .markdown_diagram_placement(id, 0)
            .expect("diagram placement");
        assert_eq!(placement.scale, 1.0);
        let content = world.component_content_box(id).unwrap();
        assert!(placement.bounds.y >= content.y - 0.01);
        assert!(placement.bounds.x >= content.x - 0.01);
        assert!(world.markdown_diagram_placement(id, 1).is_none());
    }

    #[test]
    fn dense_diagram_click_survives_small_jitter_and_requires_same_press_target() {
        let source = "flowchart LR\n".to_owned()
            + &(0..30)
                .map(|i| format!("N{i}[LongNodeLabel{i}] --> N{}", i + 1))
                .collect::<Vec<_>>()
                .join("\n");
        let markdown = NativeMarkdown::from_source(&format!(
            "Intro\n\n```mermaid\n{source}\n```\n\n```mermaid\nflowchart TD\nX-->Y\n```"
        ));
        let bounds = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 180.0,
            height: 1200.0,
        };
        let geometry = markdown.layout(bounds);
        let first = geometry.blocks[1]
            .diagram_placement()
            .expect("first diagram");
        let second = geometry.blocks[2]
            .diagram_placement()
            .expect("second diagram");
        let (x, y) = (
            first.bounds.x + first.bounds.width * 0.5,
            first.bounds.y + first.bounds.height * 0.5,
        );

        // Jitter <= 4px inside the dense diagram must still activate rather than select.
        assert!(markdown.pointer_down(x, y, bounds));
        assert!(!markdown.pointer_move(x + 2.0, y, bounds));
        assert_eq!(
            markdown.pointer_up(x + 2.0, y, bounds),
            Some(crate::RichTextEvent::DiagramActivated(
                crate::MarkdownDiagram {
                    block_index: 1,
                    source: Arc::from(source.as_str()),
                }
            ))
        );
        assert!(markdown.selection_snapshot().is_none());

        // Dragging past the click threshold selects source text instead of activating.
        markdown.clear_selection();
        assert!(markdown.pointer_down(x, y, bounds));
        let _ = markdown.pointer_move(x + 25.0, y, bounds);
        assert!(matches!(
            markdown.pointer_up(x + 25.0, y, bounds),
            Some(crate::RichTextEvent::SelectionChanged(_))
        ));

        // Pressing outside and releasing on the diagram must not activate it.
        markdown.clear_selection();
        assert!(markdown.pointer_down(bounds.x + 1.0, bounds.y + 1.0, bounds));
        assert!(!matches!(
            markdown.pointer_up(x, y, bounds),
            Some(crate::RichTextEvent::DiagramActivated(_))
        ));

        // Pressing diagram 1 and releasing on diagram 2 must not activate either.
        markdown.clear_selection();
        assert!(markdown.pointer_down(x, y, bounds));
        let (x2, y2) = (
            second.bounds.x + second.bounds.width * 0.5,
            second.bounds.y + second.bounds.height * 0.5,
        );
        assert!(!matches!(
            markdown.pointer_up(x2, y2, bounds),
            Some(crate::RichTextEvent::DiagramActivated(_))
        ));
    }

    #[test]
    fn alert_with_code_or_table_first_and_nested_alerts_emit_ordered_headers_and_tint() {
        use crate::MarkdownAlertKind;
        let code_first = NativeMarkdown::from_source("> [!NOTE]\n> ```rust\n> let x = 1;\n> ```");
        assert!(matches!(
            code_first.blocks(),
            [
                MarkdownBlock::Text {
                    kind: MarkdownBlockKind::Alert {
                        kind: MarkdownAlertKind::Note,
                        opens: true,
                    },
                    spans,
                },
                MarkdownBlock::Code {
                    language: Some(lang),
                    ..
                },
            ] if spans.is_empty() && lang == "rust"
        ));

        let table_first = NativeMarkdown::from_source("> [!TIP]\n> | A |\n> | - |\n> | 1 |");
        assert!(matches!(
            table_first.blocks(),
            [
                MarkdownBlock::Text {
                    kind: MarkdownBlockKind::Alert {
                        kind: MarkdownAlertKind::Tip,
                        opens: true,
                    },
                    spans,
                },
                MarkdownBlock::Table(_),
            ] if spans.is_empty()
        ));

        let nested =
            NativeMarkdown::from_source("> [!WARNING]\n> Outer\n>\n> > [!CAUTION]\n> > Inner");
        let kinds = nested
            .blocks()
            .iter()
            .map(|b| match b {
                MarkdownBlock::Text { kind, .. } => *kind,
                other => panic!("unexpected {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                MarkdownBlockKind::Alert {
                    kind: MarkdownAlertKind::Warning,
                    opens: true,
                },
                MarkdownBlockKind::Alert {
                    kind: MarkdownAlertKind::Caution,
                    opens: true,
                },
            ]
        );

        let title_only = NativeMarkdown::from_source("> [!IMPORTANT]");
        let narrow = title_only.layout(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 200.0,
        });
        assert!(
            crate::rich_text::markdown_content_width(title_only.blocks(), &narrow)
                > crate::rich_text::text_indent(MarkdownBlockKind::Alert {
                    kind: MarkdownAlertKind::Important,
                    opens: true,
                }) + 20.0
        );

        let drawing = code_first.drawing(column());
        let theme = nana_ui_core::builtin_theme(nana_ui_core::ThemeAppearance::Light);
        let accent = theme
            .style_model()
            .color(nana_ui_core::SemanticColorRole::Accent)
            .as_rgba_array();
        assert!(matches!(
            drawing.commands.first(),
            Some(MarkdownDrawingCommand::Panel { color: Some(c), .. }) if *c == accent
        ));
    }

    #[test]
    fn replace_presenter_swaps_default_highlighter_and_updates_markdown_and_text_area() {
        let document = DocumentId::new(84).unwrap();
        let mut context = AppContext::new();
        let initial = Box::new(crate::TextPresenterFn::new("swap-lang", |text: &str, _| {
            text.match_indices("let")
                .map(|(start, word)| crate::TextSpan {
                    start,
                    end: start + word.len(),
                    color: nana_ui_core::SemanticColorRole::Keyword,
                })
                .collect()
        }));
        context.register_presenter(initial).unwrap();

        let markdown = context
            .create_component(
                document,
                NativeMarkdown::from_source("```rust\nlet x = 1;\n```").code_presenter("swap-lang"),
            )
            .unwrap();
        let mut area_view = crate::TextArea::new("let x = 1;");
        area_view.highlight = Some(crate::HighlightRequest::new("swap-lang", "rust"));
        let area = context.create_component(document, area_view).unwrap();
        context
            .layout_document(document, LayoutViewport::new(400.0, 400.0))
            .unwrap();
        context
            .world_mut()
            .resolve_presentations(&[area.stable_id()])
            .unwrap();

        let keyword = context
            .world()
            .node_style_model(markdown.stable_id())
            .color(nana_ui_core::SemanticColorRole::Keyword)
            .as_rgba_array();
        let string_role = context
            .world()
            .node_style_model(markdown.stable_id())
            .color(nana_ui_core::SemanticColorRole::Accent)
            .as_rgba_array();
        assert!(
            painted_runs(&context, markdown.stable_id())
                .iter()
                .any(|(text, color)| text == "let" && *color == Some(keyword))
        );
        assert_eq!(
            context
                .world()
                .text_presentation(area.stable_id())
                .unwrap()
                .spans,
            vec![crate::TextSpan {
                start: 0,
                end: 3,
                color: nana_ui_core::SemanticColorRole::Keyword,
            }]
        );

        let replacement = Box::new(crate::TextPresenterFn::new("swap-lang", |text: &str, _| {
            text.match_indices("let")
                .map(|(start, word)| crate::TextSpan {
                    start,
                    end: start + word.len(),
                    color: nana_ui_core::SemanticColorRole::Accent,
                })
                .collect()
        }));
        let prev = context.replace_presenter(replacement).unwrap();
        assert!(prev.is_some());
        context
            .world_mut()
            .resolve_presentations(&[area.stable_id()])
            .unwrap();

        assert!(
            painted_runs(&context, markdown.stable_id())
                .iter()
                .any(|(text, color)| text == "let" && *color == Some(string_role))
        );
        assert_eq!(
            context
                .world()
                .text_presentation(area.stable_id())
                .unwrap()
                .spans,
            vec![crate::TextSpan {
                start: 0,
                end: 3,
                color: nana_ui_core::SemanticColorRole::Accent,
            }]
        );
    }
}
