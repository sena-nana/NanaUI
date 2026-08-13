use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use iced::alignment::Horizontal;
use iced::font;
use iced::widget::{column, container, row, scrollable, span, svg, text};
use iced::{Alignment, Element, Font, Length, Padding};
use pulldown_cmark::{
    Alignment as CmarkAlignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

use crate::{SelectableRichText, TextSelectionGroup, TextSelectionSnapshot, ThemeTokens, ui_font};

type VectorCache = Arc<Mutex<BTreeMap<VectorCacheKey, Option<Vec<u8>>>>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownSpan {
    pub text: String,
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub inline_math: bool,
    pub link: Option<String>,
    pub image: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownImage {
    pub source: String,
    pub alt: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownBlockKind {
    Paragraph,
    Heading(u8),
    Quote,
    ListItem { depth: usize },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownTableAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownTable {
    pub alignments: Vec<MarkdownTableAlignment>,
    pub header: Vec<Vec<MarkdownSpan>>,
    pub rows: Vec<Vec<Vec<MarkdownSpan>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownBlock {
    Text {
        kind: MarkdownBlockKind,
        spans: Vec<MarkdownSpan>,
    },
    Code {
        language: Option<String>,
        source: String,
    },
    DisplayMath(String),
    Mermaid(String),
    Table(MarkdownTable),
    Rule,
}

#[derive(Clone, Debug, Default)]
pub struct NativeMarkdown {
    blocks: Vec<MarkdownBlock>,
    vector_cache: VectorCache,
    selection_group: TextSelectionGroup,
}

impl PartialEq for NativeMarkdown {
    fn eq(&self, other: &Self) -> bool {
        self.blocks == other.blocks
    }
}

impl Eq for NativeMarkdown {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VectorCacheKey {
    block_index: usize,
    fragment_index: usize,
    theme: u64,
    source: u64,
}

impl NativeMarkdown {
    pub fn parse(source: &str) -> Self {
        let mut parser = MarkdownParser::default();
        let options = Options::ENABLE_GFM
            | Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_MATH;
        for event in Parser::new_ext(source, options) {
            parser.push(event);
        }
        parser.finish()
    }

    pub fn blocks(&self) -> &[MarkdownBlock] {
        &self.blocks
    }

    pub fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .map(markdown_block_plain_text)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection_group.selected_text()
    }

    pub fn clear_selection(&self) {
        self.selection_group.clear();
    }

    pub fn images(&self) -> Vec<MarkdownImage> {
        let mut images = Vec::new();
        for block in &self.blocks {
            match block {
                MarkdownBlock::Text { spans, .. } => collect_markdown_images(spans, &mut images),
                MarkdownBlock::Table(table) => {
                    for cell in table.header.iter().chain(table.rows.iter().flatten()) {
                        collect_markdown_images(cell, &mut images);
                    }
                }
                _ => {}
            }
        }
        images
    }

    pub fn view<Message>(
        &self,
        tokens: ThemeTokens,
        on_link: impl Fn(String) -> Message + Clone + 'static,
    ) -> Element<'static, Message>
    where
        Message: Clone + 'static,
    {
        native_markdown(self, tokens, on_link)
    }

    pub fn view_with_selection<Message>(
        &self,
        tokens: ThemeTokens,
        on_link: impl Fn(String) -> Message + Clone + 'static,
        on_selection_change: impl Fn(Option<String>) -> Message + 'static,
    ) -> Element<'static, Message>
    where
        Message: Clone + 'static,
    {
        self.view_with_selection_snapshot(tokens, on_link, move |selection| {
            on_selection_change(selection.map(|selection| selection.text))
        })
    }

    pub fn view_with_selection_snapshot<Message>(
        &self,
        tokens: ThemeTokens,
        on_link: impl Fn(String) -> Message + Clone + 'static,
        on_selection_change: impl Fn(Option<TextSelectionSnapshot>) -> Message + 'static,
    ) -> Element<'static, Message>
    where
        Message: Clone + 'static,
    {
        render_document(
            self,
            tokens,
            on_link,
            None,
            Some(Rc::new(on_selection_change)),
        )
    }

    pub fn view_with_images<Message>(
        &self,
        tokens: ThemeTokens,
        on_link: impl Fn(String) -> Message + Clone + 'static,
        render_image: impl Fn(MarkdownImage) -> Element<'static, Message> + 'static,
    ) -> Element<'static, Message>
    where
        Message: Clone + 'static,
    {
        render_document(self, tokens, on_link, Some(Rc::new(render_image)), None)
    }

    pub fn view_with_media<Message>(
        &self,
        tokens: ThemeTokens,
        on_link: impl Fn(String) -> Message + Clone + 'static,
        render_image: impl Fn(MarkdownImage) -> Element<'static, Message> + 'static,
        on_selection_change: impl Fn(Option<String>) -> Message + 'static,
    ) -> Element<'static, Message>
    where
        Message: Clone + 'static,
    {
        self.view_with_media_selection_snapshot(tokens, on_link, render_image, move |selection| {
            on_selection_change(selection.map(|selection| selection.text))
        })
    }

    pub fn view_with_media_selection_snapshot<Message>(
        &self,
        tokens: ThemeTokens,
        on_link: impl Fn(String) -> Message + Clone + 'static,
        render_image: impl Fn(MarkdownImage) -> Element<'static, Message> + 'static,
        on_selection_change: impl Fn(Option<TextSelectionSnapshot>) -> Message + 'static,
    ) -> Element<'static, Message>
    where
        Message: Clone + 'static,
    {
        render_document(
            self,
            tokens,
            on_link,
            Some(Rc::new(render_image)),
            Some(Rc::new(on_selection_change)),
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum PendingTextKind {
    Paragraph,
    Heading(u8),
    ListItem { depth: usize },
}

#[derive(Debug, Default)]
struct MarkdownParser {
    blocks: Vec<MarkdownBlock>,
    spans: Vec<MarkdownSpan>,
    pending_kind: Option<PendingTextKind>,
    strong_depth: usize,
    emphasis_depth: usize,
    strike_depth: usize,
    link: Option<String>,
    image: Option<String>,
    code_block: Option<(Option<String>, String)>,
    quote_depth: usize,
    lists: Vec<Option<u64>>,
    table: Option<MarkdownTableBuilder>,
    table_header: bool,
    table_row: Vec<Vec<MarkdownSpan>>,
    in_table_cell: bool,
}

#[derive(Debug)]
struct MarkdownTableBuilder {
    alignments: Vec<MarkdownTableAlignment>,
    header: Option<Vec<Vec<MarkdownSpan>>>,
    rows: Vec<Vec<Vec<MarkdownSpan>>>,
}

impl MarkdownParser {
    fn push(&mut self, event: Event<'_>) {
        if self.code_block.is_some() {
            match event {
                Event::Text(value) | Event::Code(value) => {
                    self.code_block.as_mut().unwrap().1.push_str(&value);
                }
                Event::End(TagEnd::CodeBlock) => self.finish_code_block(),
                _ => {}
            }
            return;
        }

        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(value) => self.push_span(value.into_string(), false, false),
            Event::Code(value) => self.push_span(value.into_string(), true, false),
            Event::InlineMath(value) => self.push_span(value.into_string(), false, true),
            Event::DisplayMath(value) => {
                self.flush_text();
                self.blocks
                    .push(MarkdownBlock::DisplayMath(value.into_string()));
            }
            Event::SoftBreak | Event::HardBreak => self.push_span("\n".to_owned(), false, false),
            Event::Rule => {
                self.flush_text();
                self.blocks.push(MarkdownBlock::Rule);
            }
            Event::TaskListMarker(done) => {
                self.push_span(if done { "☑ " } else { "☐ " }.to_owned(), false, false)
            }
            Event::Html(value) | Event::InlineHtml(value) => {
                self.push_span(value.into_string(), false, false)
            }
            Event::FootnoteReference(value) => {
                self.push_span(format!("[{}]", value.into_string()), false, false)
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.pending_kind.get_or_insert(PendingTextKind::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.flush_text();
                self.pending_kind = Some(PendingTextKind::Heading(heading_level(level)));
            }
            Tag::BlockQuote(_) => {
                self.flush_text();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush_text();
                let language = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(value) => value
                        .split_ascii_whitespace()
                        .next()
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned),
                };
                self.code_block = Some((language, String::new()));
            }
            Tag::List(start) => self.lists.push(start),
            Tag::Item => {
                self.flush_text();
                self.pending_kind = Some(PendingTextKind::ListItem {
                    depth: self.lists.len().max(1),
                });
                let prefix = match self.lists.last_mut() {
                    Some(Some(next)) => {
                        let prefix = format!("{next}. ");
                        *next = next.saturating_add(1);
                        prefix
                    }
                    _ => "• ".to_owned(),
                };
                self.push_span(prefix, false, false);
            }
            Tag::Emphasis => self.emphasis_depth += 1,
            Tag::Strong => self.strong_depth += 1,
            Tag::Strikethrough => self.strike_depth += 1,
            Tag::Link { dest_url, .. } => {
                self.link = Some(dest_url.into_string());
            }
            Tag::Image { dest_url, .. } => self.image = Some(dest_url.into_string()),
            Tag::Table(alignments) => {
                self.flush_text();
                self.table_row.clear();
                self.table = Some(MarkdownTableBuilder {
                    alignments: alignments.into_iter().map(table_alignment).collect(),
                    header: None,
                    rows: Vec::new(),
                });
            }
            Tag::TableHead => self.table_header = true,
            Tag::TableRow => self.table_row.clear(),
            Tag::TableCell => {
                self.spans.clear();
                self.in_table_cell = true;
            }
            Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item => self.flush_text(),
            TagEnd::BlockQuote(_) => {
                self.flush_text();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.flush_text();
                self.lists.pop();
            }
            TagEnd::Emphasis => self.emphasis_depth = self.emphasis_depth.saturating_sub(1),
            TagEnd::Strong => self.strong_depth = self.strong_depth.saturating_sub(1),
            TagEnd::Strikethrough => self.strike_depth = self.strike_depth.saturating_sub(1),
            TagEnd::Link => self.link = None,
            TagEnd::Image => self.image = None,
            TagEnd::TableCell => {
                self.table_row.push(std::mem::take(&mut self.spans));
                self.in_table_cell = false;
            }
            TagEnd::TableRow => self.finish_table_row(),
            TagEnd::TableHead => {
                self.finish_table_row();
                self.table_header = false;
            }
            TagEnd::Table => {
                self.finish_table_row();
                self.finish_table();
            }
            TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::MetadataBlock(_)
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript => {}
        }
    }

    fn push_span(&mut self, value: String, code: bool, inline_math: bool) {
        if value.is_empty() {
            return;
        }
        if self.pending_kind.is_none() && !self.in_table_cell {
            self.pending_kind = Some(PendingTextKind::Paragraph);
        }
        let next = MarkdownSpan {
            text: value,
            strong: self.strong_depth > 0,
            emphasis: self.emphasis_depth > 0,
            strikethrough: self.strike_depth > 0,
            code,
            inline_math,
            link: self.link.clone(),
            image: self.image.clone(),
        };
        if let Some(last) = self.spans.last_mut()
            && last.strong == next.strong
            && last.emphasis == next.emphasis
            && last.strikethrough == next.strikethrough
            && last.code == next.code
            && last.inline_math == next.inline_math
            && last.link == next.link
            && last.image == next.image
        {
            last.text.push_str(&next.text);
        } else {
            self.spans.push(next);
        }
    }

    fn flush_text(&mut self) {
        if self.in_table_cell || self.spans.is_empty() {
            return;
        }
        let pending = self
            .pending_kind
            .take()
            .unwrap_or(PendingTextKind::Paragraph);
        let kind = if self.quote_depth > 0 {
            MarkdownBlockKind::Quote
        } else {
            match pending {
                PendingTextKind::Paragraph => MarkdownBlockKind::Paragraph,
                PendingTextKind::Heading(level) => MarkdownBlockKind::Heading(level),
                PendingTextKind::ListItem { depth } => MarkdownBlockKind::ListItem { depth },
            }
        };
        self.blocks.push(MarkdownBlock::Text {
            kind,
            spans: std::mem::take(&mut self.spans),
        });
    }

    fn finish_code_block(&mut self) {
        let Some((language, source)) = self.code_block.take() else {
            return;
        };
        let source = source.trim_end_matches(['\r', '\n']).to_owned();
        if language
            .as_deref()
            .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "mermaid" | "mmd"))
        {
            self.blocks.push(MarkdownBlock::Mermaid(source));
        } else if language.as_deref().is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "math" | "latex" | "tex"
            )
        }) {
            self.blocks.push(MarkdownBlock::DisplayMath(source));
        } else {
            self.blocks.push(MarkdownBlock::Code { language, source });
        }
    }

    fn finish_table_row(&mut self) {
        if self.table_row.is_empty() {
            return;
        }
        let Some(table) = self.table.as_mut() else {
            self.table_row.clear();
            return;
        };
        let row = normalize_table_row(std::mem::take(&mut self.table_row), table.alignments.len());
        if self.table_header && table.header.is_none() {
            table.header = Some(row);
        } else {
            table.rows.push(row);
        }
    }

    fn finish_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        let header = table.header.unwrap_or_default();
        if header.is_empty() && table.rows.is_empty() {
            return;
        }
        self.blocks.push(MarkdownBlock::Table(MarkdownTable {
            alignments: table.alignments,
            header,
            rows: table.rows,
        }));
    }

    fn finish(mut self) -> NativeMarkdown {
        self.flush_text();
        NativeMarkdown {
            blocks: self.blocks,
            vector_cache: Arc::default(),
            selection_group: TextSelectionGroup::default(),
        }
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn table_alignment(alignment: CmarkAlignment) -> MarkdownTableAlignment {
    match alignment {
        CmarkAlignment::None | CmarkAlignment::Left => MarkdownTableAlignment::Left,
        CmarkAlignment::Center => MarkdownTableAlignment::Center,
        CmarkAlignment::Right => MarkdownTableAlignment::Right,
    }
}

fn normalize_table_row(
    mut row: Vec<Vec<MarkdownSpan>>,
    column_count: usize,
) -> Vec<Vec<MarkdownSpan>> {
    row.truncate(column_count);
    row.resize_with(column_count, Vec::new);
    row
}

fn markdown_block_plain_text(block: &MarkdownBlock) -> String {
    match block {
        MarkdownBlock::Text { spans, .. } => markdown_spans_plain_text(spans),
        MarkdownBlock::Code { source, .. }
        | MarkdownBlock::DisplayMath(source)
        | MarkdownBlock::Mermaid(source) => source.clone(),
        MarkdownBlock::Table(table) => std::iter::once(table.header.as_slice())
            .chain(table.rows.iter().map(Vec::as_slice))
            .filter(|row| !row.is_empty())
            .map(|row| {
                row.iter()
                    .map(|cell| markdown_spans_plain_text(cell))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        MarkdownBlock::Rule => "---".to_owned(),
    }
}

fn markdown_spans_plain_text(spans: &[MarkdownSpan]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

fn collect_markdown_images(spans: &[MarkdownSpan], images: &mut Vec<MarkdownImage>) {
    for span in spans {
        let Some(source) = &span.image else {
            continue;
        };
        let image = MarkdownImage {
            source: source.clone(),
            alt: span.text.clone(),
        };
        if !images.contains(&image) {
            images.push(image);
        }
    }
}

pub fn native_markdown<Message>(
    document: &NativeMarkdown,
    tokens: ThemeTokens,
    on_link: impl Fn(String) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    render_document(document, tokens, on_link, None, None)
}

type SelectionCallback<Message> = Option<Rc<dyn Fn(Option<TextSelectionSnapshot>) -> Message>>;
type ImageRenderer<Message> = Option<Rc<dyn Fn(MarkdownImage) -> Element<'static, Message>>>;
type LinkHandler<Message> = Rc<dyn Fn(String) -> Message>;

#[derive(Clone)]
struct MarkdownRenderContext<Message> {
    vector_cache: VectorCache,
    selection_group: TextSelectionGroup,
    tokens: ThemeTokens,
    on_link: LinkHandler<Message>,
    image_renderer: ImageRenderer<Message>,
    on_selection_change: SelectionCallback<Message>,
}

fn render_document<Message>(
    document: &NativeMarkdown,
    tokens: ThemeTokens,
    on_link: impl Fn(String) -> Message + Clone + 'static,
    image_renderer: ImageRenderer<Message>,
    on_selection_change: SelectionCallback<Message>,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let context = MarkdownRenderContext {
        vector_cache: document.vector_cache.clone(),
        selection_group: document.selection_group.clone(),
        tokens,
        on_link: Rc::new(on_link),
        image_renderer,
        on_selection_change,
    };
    let mut content = column![].spacing(9).width(Length::Fill);
    for (block_index, block) in document.blocks.iter().cloned().enumerate() {
        content = content.push(render_block(block_index, block, context.clone()));
    }
    content.into()
}

fn render_block<Message>(
    block_index: usize,
    block: MarkdownBlock,
    context: MarkdownRenderContext<Message>,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let MarkdownRenderContext {
        vector_cache,
        selection_group,
        tokens,
        on_link,
        image_renderer,
        on_selection_change,
    } = context;
    let colors = tokens.colors;
    match block {
        MarkdownBlock::Text { kind, spans } => {
            let (size, weight, left_padding, color) = match kind {
                MarkdownBlockKind::Heading(level) => (
                    21.0 - f32::from(level.saturating_sub(1)) * 1.5,
                    font::Weight::Semibold,
                    0.0,
                    colors.text,
                ),
                MarkdownBlockKind::Quote => (13.0, font::Weight::Normal, 12.0, colors.muted),
                MarkdownBlockKind::ListItem { depth } => (
                    13.0,
                    font::Weight::Normal,
                    (depth.saturating_sub(1) as f32) * 14.0,
                    colors.text,
                ),
                MarkdownBlockKind::Paragraph => (13.0, font::Weight::Normal, 0.0, colors.text),
            };
            let has_inline_media = spans
                .iter()
                .any(|item| item.inline_math || item.image.is_some());
            let line: Element<'static, Message> = if has_inline_media {
                let mut fragments = row![].spacing(1).align_y(Alignment::Center);
                let mut first_text_fragment = true;
                for (fragment_index, item) in spans.into_iter().enumerate() {
                    if let Some(source) = item.image.clone() {
                        let image = MarkdownImage {
                            source: source.clone(),
                            alt: item.text.clone(),
                        };
                        if let Some(render_image) = &image_renderer {
                            fragments = fragments.push(render_image(image));
                        } else {
                            let value = span(item.text)
                                .color(colors.accent)
                                .underline(true)
                                .link(source);
                            fragments = fragments.push(
                                selectable_markdown_text(
                                    vec![value],
                                    selection_group.clone(),
                                    selection_order(block_index, fragment_index),
                                    if first_text_fragment {
                                        block_separator(block_index)
                                    } else {
                                        ""
                                    },
                                    on_selection_change.clone(),
                                )
                                .size(size)
                                .selection_color(selection_color(tokens))
                                .on_link_click(link_callback(on_link.clone())),
                            );
                            first_text_fragment = false;
                        }
                        continue;
                    }
                    if item.inline_math {
                        fragments = fragments.push(render_inline_math(
                            block_index,
                            fragment_index,
                            item.text,
                            vector_cache.clone(),
                            tokens,
                        ));
                        continue;
                    }
                    let mut value = span(item.text)
                        .font(if item.code {
                            Font::MONOSPACE
                        } else {
                            ui_font(if item.strong {
                                font::Weight::Bold
                            } else {
                                weight
                            })
                        })
                        .color(if item.code { colors.accent } else { color })
                        .strikethrough(item.strikethrough);
                    if item.emphasis {
                        value = value.font(Font {
                            style: font::Style::Italic,
                            weight: if item.strong {
                                font::Weight::Bold
                            } else {
                                weight
                            },
                            ..Font::default()
                        });
                    }
                    if let Some(link) = item.link {
                        value = value.color(colors.accent).underline(true).link(link);
                    }
                    fragments = fragments.push(
                        selectable_markdown_text(
                            vec![value],
                            selection_group.clone(),
                            selection_order(block_index, fragment_index),
                            if first_text_fragment {
                                block_separator(block_index)
                            } else {
                                ""
                            },
                            on_selection_change.clone(),
                        )
                        .size(size)
                        .selection_color(selection_color(tokens))
                        .on_link_click(link_callback(on_link.clone())),
                    );
                    first_text_fragment = false;
                }
                fragments.wrap().into()
            } else {
                let spans = spans
                    .into_iter()
                    .map(|item| {
                        let mut value = span(item.text)
                            .font(if item.code {
                                Font::MONOSPACE
                            } else {
                                ui_font(if item.strong {
                                    font::Weight::Bold
                                } else {
                                    weight
                                })
                            })
                            .color(if item.code { colors.accent } else { color })
                            .strikethrough(item.strikethrough);
                        if item.emphasis {
                            value = value.font(Font {
                                style: font::Style::Italic,
                                weight: if item.strong {
                                    font::Weight::Bold
                                } else {
                                    weight
                                },
                                ..Font::default()
                            });
                        }
                        if let Some(link) = item.link {
                            value = value.color(colors.accent).underline(true).link(link);
                        }
                        value
                    })
                    .collect::<Vec<_>>();
                selectable_markdown_text(
                    spans,
                    selection_group,
                    selection_order(block_index, 0),
                    block_separator(block_index),
                    on_selection_change,
                )
                .size(size)
                .selection_color(selection_color(tokens))
                .on_link_click(link_callback(on_link))
                .width(Length::Fill)
                .into()
            };
            let line = container(line)
                .width(Length::Fill)
                .padding(Padding::from([0.0, left_padding]));
            if matches!(kind, MarkdownBlockKind::Quote) {
                container(row![text("│").color(colors.border_strong), line].spacing(8))
                    .width(Length::Fill)
                    .into()
            } else {
                line.into()
            }
        }
        MarkdownBlock::Code { language, source } => selectable_code_block(
            language.as_deref(),
            source,
            block_index,
            selection_group,
            on_selection_change,
            tokens,
        ),
        MarkdownBlock::DisplayMath(source) => {
            render_math(block_index, source, vector_cache, tokens)
        }
        MarkdownBlock::Mermaid(source) => render_mermaid(block_index, source, vector_cache, tokens),
        MarkdownBlock::Table(table) => render_table(
            block_index,
            table,
            MarkdownRenderContext {
                vector_cache,
                selection_group,
                tokens,
                on_link,
                image_renderer,
                on_selection_change,
            },
        ),
        MarkdownBlock::Rule => container(text(""))
            .width(Length::Fill)
            .height(Length::Fixed(1.0))
            .style(move |_theme| {
                iced::widget::container::Style::default().background(colors.border_soft)
            })
            .into(),
    }
}

fn selectable_code_block<Message>(
    language: Option<&str>,
    source: String,
    block_index: usize,
    selection_group: TextSelectionGroup,
    on_selection_change: SelectionCallback<Message>,
    tokens: ThemeTokens,
) -> Element<'static, Message>
where
    Message: 'static,
{
    let colors = tokens.colors;
    let mut body = column![].spacing(7).width(Length::Fill);
    if let Some(language) = language {
        body = body.push(text(language.to_owned()).size(10).color(colors.faint));
    }
    body = body.push(
        selectable_markdown_text(
            vec![span(source).font(Font::MONOSPACE).color(colors.text)],
            selection_group,
            selection_order(block_index, 0),
            block_separator(block_index),
            on_selection_change,
        )
        .size(12)
        .width(Length::Fill)
        .selection_color(selection_color(tokens)),
    );
    container(body)
        .width(Length::Fill)
        .padding(12)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(colors.subtle)
                .border(iced::Border {
                    color: colors.border_soft,
                    width: 1.0,
                    radius: tokens.metrics.radius_sm.into(),
                })
        })
        .into()
}

fn code_block<Message>(
    language: Option<&str>,
    source: String,
    tokens: ThemeTokens,
) -> Element<'static, Message>
where
    Message: 'static,
{
    selectable_code_block(
        language,
        source,
        0,
        TextSelectionGroup::default(),
        None,
        tokens,
    )
}

fn render_table<Message>(
    block_index: usize,
    table: MarkdownTable,
    context: MarkdownRenderContext<Message>,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    const COLUMN_WIDTH: f32 = 148.0;
    let MarkdownRenderContext {
        vector_cache,
        selection_group,
        tokens,
        on_link,
        image_renderer,
        on_selection_change,
    } = context;

    let column_count = table
        .alignments
        .len()
        .max(table.header.len())
        .max(table.rows.iter().map(Vec::len).max().unwrap_or_default());
    if column_count == 0 {
        return container(text("")).into();
    }

    let mut alignments = table.alignments;
    alignments.resize(column_count, MarkdownTableAlignment::Left);
    let context = TableRenderContext {
        block_index,
        vector_cache,
        selection_group,
        tokens,
        on_link,
        image_renderer,
        on_selection_change,
        column_width: COLUMN_WIDTH,
    };
    let mut body = column![].spacing(0).width(Length::Shrink);
    if !table.header.is_empty() {
        body = body.push(render_table_row(
            context.clone(),
            0,
            normalize_table_row(table.header, column_count),
            &alignments,
            true,
        ));
    }
    for (row_index, cells) in table.rows.into_iter().enumerate() {
        body = body.push(render_table_row(
            context.clone(),
            row_index.saturating_add(1),
            normalize_table_row(cells, column_count),
            &alignments,
            false,
        ));
    }

    scrollable(body)
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new().width(10).scroller_width(3),
        ))
        .width(Length::Fill)
        .into()
}

#[derive(Clone)]
struct TableRenderContext<Message> {
    block_index: usize,
    vector_cache: VectorCache,
    selection_group: TextSelectionGroup,
    tokens: ThemeTokens,
    on_link: LinkHandler<Message>,
    image_renderer: ImageRenderer<Message>,
    on_selection_change: SelectionCallback<Message>,
    column_width: f32,
}

fn render_table_row<Message>(
    context: TableRenderContext<Message>,
    row_index: usize,
    cells: Vec<Vec<MarkdownSpan>>,
    alignments: &[MarkdownTableAlignment],
    header: bool,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let mut rendered = row![].spacing(0).width(Length::Shrink);
    for (column_index, spans) in cells.into_iter().enumerate() {
        rendered = rendered.push(render_table_cell(
            context.clone(),
            row_index,
            column_index,
            spans,
            alignments.get(column_index).copied().unwrap_or_default(),
            header,
        ));
    }
    rendered.into()
}

fn render_table_cell<Message>(
    context: TableRenderContext<Message>,
    row_index: usize,
    column_index: usize,
    spans: Vec<MarkdownSpan>,
    alignment: MarkdownTableAlignment,
    header: bool,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let TableRenderContext {
        block_index,
        vector_cache,
        selection_group,
        tokens,
        on_link,
        image_renderer,
        on_selection_change,
        column_width,
    } = context;
    let colors = tokens.colors;
    let weight = if header {
        font::Weight::Semibold
    } else {
        font::Weight::Normal
    };
    let horizontal = match alignment {
        MarkdownTableAlignment::Left => Horizontal::Left,
        MarkdownTableAlignment::Center => Horizontal::Center,
        MarkdownTableAlignment::Right => Horizontal::Right,
    };
    let has_inline_media = spans
        .iter()
        .any(|span| span.inline_math || span.image.is_some());
    let content: Element<'static, Message> = if has_inline_media {
        let mut fragments = row![].spacing(1).align_y(Alignment::Center);
        let mut first_text_fragment = true;
        for (fragment_index, item) in spans.into_iter().enumerate() {
            if let Some(source) = item.image.clone() {
                let image = MarkdownImage {
                    source: source.clone(),
                    alt: item.text.clone(),
                };
                if let Some(render_image) = &image_renderer {
                    fragments = fragments.push(render_image(image));
                } else {
                    fragments = fragments.push(
                        selectable_markdown_text(
                            vec![
                                span(item.text)
                                    .color(colors.accent)
                                    .underline(true)
                                    .link(source),
                            ],
                            selection_group.clone(),
                            table_selection_order(
                                block_index,
                                row_index,
                                column_index,
                                fragment_index,
                            ),
                            table_separator(
                                block_index,
                                row_index,
                                column_index,
                                first_text_fragment,
                            ),
                            on_selection_change.clone(),
                        )
                        .size(12)
                        .selection_color(selection_color(tokens))
                        .on_link_click(link_callback(on_link.clone())),
                    );
                    first_text_fragment = false;
                }
            } else if item.inline_math {
                let cache_index = row_index
                    .saturating_mul(10_000)
                    .saturating_add(column_index.saturating_mul(100))
                    .saturating_add(fragment_index)
                    .saturating_add(1);
                fragments = fragments.push(render_inline_math(
                    block_index,
                    cache_index,
                    item.text,
                    vector_cache.clone(),
                    tokens,
                ));
            } else {
                fragments = fragments.push(
                    selectable_markdown_text(
                        vec![markdown_text_span(item, weight, colors.text, colors.accent)],
                        selection_group.clone(),
                        table_selection_order(block_index, row_index, column_index, fragment_index),
                        table_separator(block_index, row_index, column_index, first_text_fragment),
                        on_selection_change.clone(),
                    )
                    .size(12)
                    .selection_color(selection_color(tokens))
                    .on_link_click(link_callback(on_link.clone())),
                );
                first_text_fragment = false;
            }
        }
        container(fragments.wrap())
            .width(Length::Fill)
            .align_x(horizontal)
            .into()
    } else {
        let spans = spans
            .into_iter()
            .map(|item| markdown_text_span(item, weight, colors.text, colors.accent))
            .collect();
        selectable_markdown_text(
            spans,
            selection_group,
            table_selection_order(block_index, row_index, column_index, 0),
            table_separator(block_index, row_index, column_index, true),
            on_selection_change,
        )
        .size(12)
        .width(Length::Fill)
        .align_x(horizontal)
        .selection_color(selection_color(tokens))
        .on_link_click(link_callback(on_link))
        .into()
    };
    let background = if header {
        colors.subtle
    } else {
        colors.surface
    };
    container(content)
        .width(Length::Fixed(column_width))
        .padding(Padding::from([7.0, 8.0]))
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(background)
                .border(iced::Border {
                    color: colors.border_soft,
                    width: 0.5,
                    radius: 0.0.into(),
                })
        })
        .into()
}

fn selectable_markdown_text<Message>(
    spans: Vec<iced::widget::text::Span<'static, String, Font>>,
    selection_group: TextSelectionGroup,
    order: u64,
    separator_before: &str,
    on_selection_change: SelectionCallback<Message>,
) -> SelectableRichText<Message>
where
    Message: 'static,
{
    let mut value =
        SelectableRichText::new(spans).selection_group(selection_group, order, separator_before);
    if let Some(callback) = on_selection_change {
        value = value.on_selection_snapshot(move |selected| callback(selected));
    }
    value
}

fn link_callback<Message>(handler: LinkHandler<Message>) -> impl Fn(String) -> Message {
    move |value| handler(value)
}

fn selection_order(block_index: usize, fragment_index: usize) -> u64 {
    (block_index as u64)
        .saturating_mul(1_000_000)
        .saturating_add(fragment_index as u64)
}

fn table_selection_order(
    block_index: usize,
    row_index: usize,
    column_index: usize,
    fragment_index: usize,
) -> u64 {
    selection_order(block_index, 100_000)
        .saturating_add((row_index as u64).saturating_mul(10_000))
        .saturating_add((column_index as u64).saturating_mul(100))
        .saturating_add(fragment_index as u64)
}

fn block_separator(block_index: usize) -> &'static str {
    if block_index == 0 { "" } else { "\n\n" }
}

fn table_separator(
    block_index: usize,
    row_index: usize,
    column_index: usize,
    first_text_fragment: bool,
) -> &'static str {
    if !first_text_fragment {
        ""
    } else if row_index == 0 && column_index == 0 {
        block_separator(block_index)
    } else if column_index == 0 {
        "\n"
    } else {
        "\t"
    }
}

fn markdown_text_span(
    item: MarkdownSpan,
    weight: font::Weight,
    color: iced::Color,
    accent: iced::Color,
) -> iced::widget::text::Span<'static, String, Font> {
    let mut value = span(item.text)
        .font(if item.code {
            Font::MONOSPACE
        } else {
            ui_font(if item.strong {
                font::Weight::Bold
            } else {
                weight
            })
        })
        .color(if item.code { accent } else { color })
        .strikethrough(item.strikethrough);
    if item.emphasis {
        value = value.font(Font {
            style: font::Style::Italic,
            weight: if item.strong {
                font::Weight::Bold
            } else {
                weight
            },
            ..Font::default()
        });
    }
    if let Some(link) = item.link {
        value = value.color(accent).underline(true).link(link);
    }
    value
}

#[cfg(feature = "math")]
fn render_math<Message>(
    block_index: usize,
    source: String,
    cache: VectorCache,
    tokens: ThemeTokens,
) -> Element<'static, Message>
where
    Message: 'static,
{
    let result = cached_vector(&cache, block_index, 0, &source, tokens, || {
        render_math_svg(&source, 20.0, 6.0, tokens)
    });
    vector_or_source(result, source, 72.0, tokens)
}

#[cfg(feature = "math")]
fn render_inline_math<Message>(
    block_index: usize,
    fragment_index: usize,
    source: String,
    cache: VectorCache,
    tokens: ThemeTokens,
) -> Element<'static, Message>
where
    Message: 'static,
{
    let result = cached_vector(
        &cache,
        block_index,
        fragment_index.saturating_add(1),
        &source,
        tokens,
        || render_math_svg(&source, 16.0, 1.0, tokens),
    );
    match result {
        Some(value) => svg(svg::Handle::from_memory(value))
            .width(Length::Shrink)
            .height(Length::Fixed(24.0))
            .into(),
        None => text(format!("${source}$"))
            .font(Font::MONOSPACE)
            .size(12)
            .color(tokens.colors.danger)
            .into(),
    }
}

#[cfg(not(feature = "math"))]
fn render_inline_math<Message>(
    _block_index: usize,
    _fragment_index: usize,
    source: String,
    _cache: VectorCache,
    tokens: ThemeTokens,
) -> Element<'static, Message>
where
    Message: 'static,
{
    text(format!("${source}$"))
        .font(Font::MONOSPACE)
        .size(12)
        .color(tokens.colors.text)
        .into()
}

#[cfg(feature = "math")]
fn render_math_svg(
    source: &str,
    font_size: f64,
    padding: f64,
    tokens: ThemeTokens,
) -> Result<String, String> {
    let ast = ratex_parser::parse(source.trim()).map_err(|error| error.to_string())?;
    let color = tokens.colors.text;
    let options = ratex_layout::LayoutOptions::default().with_color(
        ratex_types::color::Color::new(color.r, color.g, color.b, color.a),
    );
    let layout = ratex_layout::layout(&ast, &options);
    let display = ratex_layout::to_display_list(&layout);
    Ok(ratex_svg::render_to_svg(
        &display,
        &ratex_svg::SvgOptions {
            font_size,
            padding,
            embed_glyphs: true,
            ..ratex_svg::SvgOptions::default()
        },
    ))
}

#[cfg(not(feature = "math"))]
fn render_math<Message>(
    _block_index: usize,
    source: String,
    _cache: VectorCache,
    tokens: ThemeTokens,
) -> Element<'static, Message>
where
    Message: 'static,
{
    code_block(Some("math"), source, tokens)
}

#[cfg(feature = "diagrams")]
fn render_mermaid<Message>(
    block_index: usize,
    source: String,
    cache: VectorCache,
    tokens: ThemeTokens,
) -> Element<'static, Message>
where
    Message: 'static,
{
    let color = |value: iced::Color| {
        format!(
            "#{:02x}{:02x}{:02x}",
            (value.r * 255.0).round() as u8,
            (value.g * 255.0).round() as u8,
            (value.b * 255.0).round() as u8
        )
    };
    let result = cached_vector(&cache, block_index, 0, &source, tokens, || {
        let config = merman::MermaidConfig::from_value(serde_json::json!({
            "securityLevel": "strict",
            "theme": "base",
            "themeVariables": {
                "background": "transparent",
                "mainBkg": color(tokens.colors.subtle),
                "primaryColor": color(tokens.colors.subtle),
                "primaryTextColor": color(tokens.colors.text),
                "textColor": color(tokens.colors.text),
                "lineColor": color(tokens.colors.muted),
                "nodeBorder": color(tokens.colors.border_strong),
                "edgeLabelBackground": color(tokens.colors.surface)
            }
        }));
        merman::render::HeadlessRenderer::new()
            .with_site_config(config)
            .with_strict_parsing()
            .with_vendored_text_measurer()
            .with_diagram_id(&format!("nana-{:x}", stable_hash(&source)))
            .render_svg_resvg_safe_sync(&source)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "source is not a Mermaid diagram".to_owned())
    });
    vector_or_source(result, source, 280.0, tokens)
}

#[cfg(not(feature = "diagrams"))]
fn render_mermaid<Message>(
    _block_index: usize,
    source: String,
    _cache: VectorCache,
    tokens: ThemeTokens,
) -> Element<'static, Message>
where
    Message: 'static,
{
    code_block(Some("mermaid"), source, tokens)
}

fn vector_or_source<Message>(
    result: Option<Vec<u8>>,
    source: String,
    height: f32,
    tokens: ThemeTokens,
) -> Element<'static, Message>
where
    Message: 'static,
{
    match result {
        Some(value) => container(
            svg(svg::Handle::from_memory(value))
                .width(Length::Fill)
                .height(Length::Fixed(height)),
        )
        .width(Length::Fill)
        .padding(8)
        .into(),
        None => code_block(None, source, tokens),
    }
}

fn cached_vector(
    cache: &VectorCache,
    block_index: usize,
    fragment_index: usize,
    source: &str,
    tokens: ThemeTokens,
    render: impl FnOnce() -> Result<String, String>,
) -> Option<Vec<u8>> {
    let key = VectorCacheKey {
        block_index,
        fragment_index,
        theme: theme_hash(tokens),
        source: stable_hash(source),
    };
    if let Ok(cache) = cache.lock()
        && let Some(value) = cache.get(&key)
    {
        return value.clone();
    }
    let rendered = render().ok().map(String::into_bytes);
    if let Ok(mut cache) = cache.lock() {
        cache.insert(key, rendered.clone());
    }
    rendered
}

fn theme_hash(tokens: ThemeTokens) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for color in [
        tokens.colors.text,
        tokens.colors.muted,
        tokens.colors.surface,
        tokens.colors.subtle,
        tokens.colors.border_strong,
    ] {
        for component in [color.r, color.g, color.b, color.a] {
            hash ^= u64::from(component.to_bits());
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

fn selection_color(tokens: ThemeTokens) -> iced::Color {
    iced::Color {
        a: 0.28,
        ..tokens.colors.accent
    }
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_preserves_markdown_structure_math_and_mermaid_as_native_blocks() {
        let document = NativeMarkdown::parse(
            "# Title\n\nUse **bold** and $x^2$.\n\n$$\\frac{1}{2}$$\n\n```mermaid\nflowchart LR\nA-->B\n```\n",
        );

        assert!(matches!(
            document.blocks()[0],
            MarkdownBlock::Text {
                kind: MarkdownBlockKind::Heading(1),
                ..
            }
        ));
        assert!(document.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::Text { spans, .. }
                if spans.iter().any(|span| span.inline_math && span.text == "x^2")
        )));
        assert!(document.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::DisplayMath(source) if source == "\\frac{1}{2}"
        )));
        assert!(document.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::Mermaid(source) if source.contains("A-->B")
        )));
    }

    #[test]
    fn tables_preserve_cells_alignment_and_nested_inline_semantics() {
        let document = NativeMarkdown::parse(
            "| Name | Count | Ratio |\n| :--- | ---: | :---: |\n| **Native** | 42 | $1/2$ |\n\n1. First\n   - Child\n",
        );
        let table = document
            .blocks()
            .iter()
            .find_map(|block| match block {
                MarkdownBlock::Table(table) => Some(table),
                _ => None,
            })
            .expect("table block");
        assert_eq!(
            table.alignments,
            [
                MarkdownTableAlignment::Left,
                MarkdownTableAlignment::Right,
                MarkdownTableAlignment::Center,
            ]
        );
        assert_eq!(markdown_spans_plain_text(&table.header[0]), "Name");
        assert!(table.rows[0][0].iter().any(|span| span.strong));
        assert!(table.rows[0][2].iter().any(|span| span.inline_math));
        assert!(document.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::Text {
                kind: MarkdownBlockKind::ListItem { depth: 2 },
                ..
            }
        )));
    }

    #[test]
    fn plain_text_copies_complete_document_and_table_cells_without_markdown_delimiters() {
        let document = NativeMarkdown::parse(
            "# 标题\n\n正文 **加粗**。\n\n| 名称 | 数量 |\n| --- | ---: |\n| Alpha | 42 |\n\n```rust\nlet value = 1;\n```",
        );

        assert_eq!(
            document.plain_text(),
            "标题\n\n正文 加粗。\n\n名称\t数量\nAlpha\t42\n\nlet value = 1;"
        );
    }

    #[test]
    fn parser_preserves_inline_images_for_application_owned_media_resolution() {
        let document = NativeMarkdown::parse(
            "Before ![diagram](https://example.com/diagram.png) after\n\n| Media |\n| --- |\n| ![thumb](file:///tmp/thumb.png) |",
        );

        assert_eq!(
            document.images(),
            [
                MarkdownImage {
                    source: "https://example.com/diagram.png".to_owned(),
                    alt: "diagram".to_owned(),
                },
                MarkdownImage {
                    source: "file:///tmp/thumb.png".to_owned(),
                    alt: "thumb".to_owned(),
                },
            ]
        );
        assert!(document.plain_text().contains("Before diagram after"));
    }

    #[cfg(feature = "math")]
    #[test]
    fn ratex_produces_self_contained_svg_paths() {
        let ast = ratex_parser::parse(r"\frac{a}{b}").unwrap();
        let layout = ratex_layout::layout(&ast, &ratex_layout::LayoutOptions::default());
        let svg = ratex_svg::render_to_svg(
            &ratex_layout::to_display_list(&layout),
            &ratex_svg::SvgOptions {
                embed_glyphs: true,
                ..ratex_svg::SvgOptions::default()
            },
        );
        assert!(svg.contains("<svg"));
        assert!(svg.contains("<path") || svg.contains("<image"));
    }

    #[cfg(feature = "diagrams")]
    #[test]
    fn merman_renders_without_a_browser() {
        let svg = merman::render::HeadlessRenderer::new()
            .with_strict_parsing()
            .with_vendored_text_measurer()
            .render_svg_resvg_safe_sync("flowchart LR\nA-->B")
            .unwrap()
            .unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("A"));
        assert!(svg.contains("B"));
    }
}
