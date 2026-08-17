//! Backend-neutral markdown blocks and selectable rich text.
//!
//! Runtime owns the block model, grapheme selection ranges, and leaf
//! projection. Applications own link handling, image decode, mermaid/math
//! rendering, and clipboard writes. The Iced parser remains in `nana-ui`;
//! this module consumes already-parsed [`MarkdownBlock`] values.
//!
//! [`ComponentView`] projection keeps [`TextContent`] as fallback text and
//! writes [`StandardVisual::NativeMarkdown`] /
//! [`StandardVisual::SelectableRichText`]. Selection ranges are half-open
//! grapheme offsets, the same convention as [`TextSelectionSnapshot`].
//!
//! Code-block languages stay on [`MarkdownBlock::Code`]. A parent that can
//! allocate child IDs should project each fenced block as a text child with
//! [`HighlightRequest::highlight`]; this leaf does not create those children.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use unicode_segmentation::UnicodeSegmentation;

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, HighlightRequest, InteractionState,
    LayoutBox, LengthSpec, MutationQueue, NodeKind, NodeStyle, StableNodeId, StandardVisual,
    TextContent, UiWorld,
};

/// Unit advance used for backend-neutral hit testing. Scene paint owns real
/// glyph metrics.
pub const GRAPHEME_ADVANCE: f32 = 8.0;
pub const LINE_HEIGHT: f32 = 16.0;
const BLOCK_GAP: f32 = 9.0;
const LIST_INDENT: f32 = 14.0;
const QUOTE_INDENT: f32 = 12.0;

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

impl MarkdownSpan {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
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

impl MarkdownBlock {
    pub fn paragraph(spans: impl IntoIterator<Item = MarkdownSpan>) -> Self {
        Self::Text {
            kind: MarkdownBlockKind::Paragraph,
            spans: spans.into_iter().collect(),
        }
    }

    pub fn heading(level: u8, spans: impl IntoIterator<Item = MarkdownSpan>) -> Self {
        Self::Text {
            kind: MarkdownBlockKind::Heading(level),
            spans: spans.into_iter().collect(),
        }
    }

    pub fn code(language: Option<&str>, source: impl Into<String>) -> Self {
        Self::Code {
            language: language.map(str::to_owned),
            source: source.into(),
        }
    }

    pub fn language(&self) -> Option<&str> {
        match self {
            Self::Code { language, .. } => language.as_deref(),
            _ => None,
        }
    }

    /// Highlight intent for a fenced code block. Parent wiring attaches this
    /// to a child text node; the markdown leaf does not allocate that child.
    pub fn highlight_request(&self) -> Option<HighlightRequest> {
        match self {
            Self::Code {
                language: Some(language),
                ..
            } if !language.is_empty() => Some(HighlightRequest::highlight(language.as_str())),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NativeMarkdown {
    blocks: Vec<MarkdownBlock>,
    selection: TextSelectionGroup,
    pub style: NodeStyle,
}

impl PartialEq for NativeMarkdown {
    fn eq(&self, other: &Self) -> bool {
        self.blocks == other.blocks
    }
}

impl Eq for NativeMarkdown {}

impl Default for NativeMarkdown {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeMarkdown {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            selection: TextSelectionGroup::new(),
            style: NodeStyle::default(),
        }
    }

    pub fn from_blocks(blocks: impl IntoIterator<Item = MarkdownBlock>) -> Self {
        Self {
            blocks: blocks.into_iter().collect(),
            selection: TextSelectionGroup::new(),
            style: NodeStyle::default(),
        }
    }

    pub fn blocks(&self) -> &[MarkdownBlock] {
        &self.blocks
    }

    pub fn selection_group(&self) -> &TextSelectionGroup {
        &self.selection
    }

    pub fn group_id(&self) -> TextSelectionGroupId {
        self.selection.id()
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .map(markdown_block_plain_text)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
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

    pub fn code_highlights(&self) -> Vec<(usize, HighlightRequest)> {
        self.blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| block.highlight_request().map(|request| (index, request)))
            .collect()
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection.snapshot().map(|snapshot| snapshot.text)
    }

    pub fn selection_snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.selection.snapshot()
    }

    pub fn copy_snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.selection.snapshot()
    }

    pub fn clear_selection(&self) {
        self.selection.clear();
    }

    pub fn layout(&self, bounds: LayoutBox) -> MarkdownGeometry {
        layout_markdown(&self.blocks, bounds)
    }

    pub fn pointer_down(&self, x: f32, y: f32, bounds: LayoutBox) -> bool {
        let geometry = self.layout(bounds);
        self.selection.begin(&geometry.run, x, y)
    }

    pub fn pointer_move(&self, x: f32, y: f32, bounds: LayoutBox) -> bool {
        let geometry = self.layout(bounds);
        self.selection.drag(&geometry.run, x, y)
    }

    pub fn pointer_up(&self, x: f32, y: f32, bounds: LayoutBox) -> Option<RichTextEvent> {
        let geometry = self.layout(bounds);
        self.selection.finish(&geometry.run, x, y)
    }

    pub fn link_at(&self, x: f32, y: f32, bounds: LayoutBox) -> Option<Arc<str>> {
        self.layout(bounds).run.link_at(x, y)
    }

    fn intrinsic_height(&self) -> f32 {
        markdown_intrinsic_height(&self.blocks)
    }
}

impl ComponentView for NativeMarkdown {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "markdown".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let plain = self.plain_text();
        if world.text(id) != Some(plain.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: plain.clone(),
                },
            );
        }
        let visual = StandardVisual::NativeMarkdown {
            text: Arc::from(plain),
            selection: projected_selection(&self.selection),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        let height = self.intrinsic_height();
        layout.height = Some(LengthSpec::Px(height));
        layout.min_height = Some(LengthSpec::Px(height));
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::Document,
                value: self
                    .selected_text()
                    .map(Arc::from)
                    .or_else(|| Some(Arc::from(self.plain_text().as_str()))),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichSpan {
    pub text: Arc<str>,
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
    pub link: Option<Arc<str>>,
}

impl RichSpan {
    pub fn plain(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            strong: false,
            emphasis: false,
            code: false,
            link: None,
        }
    }

    pub fn link(text: impl Into<Arc<str>>, href: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            strong: false,
            emphasis: false,
            code: false,
            link: Some(href.into()),
        }
    }
}

/// Half-open grapheme range `[start, end)` in the laid-out document run.
///
/// Offsets count Unicode grapheme clusters, not UTF-8 bytes. Markdown block
/// separators (`\n\n`, `\n`, `\t`) appear in [`Self::text`] but are not
/// graphemes and do not advance `start`/`end`. [`StandardVisual::NativeMarkdown`]
/// and [`StandardVisual::SelectableRichText`] project this same range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSelectionSnapshot {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextSelectionGroupId(u64);

impl TextSelectionGroupId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// Shared document-level selection for one rich-text or markdown leaf.
#[derive(Clone)]
pub struct TextSelectionGroup {
    id: TextSelectionGroupId,
    state: Arc<Mutex<GroupState>>,
}

impl std::fmt::Debug for TextSelectionGroup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextSelectionGroup")
            .field("id", &self.id)
            .finish()
    }
}

impl Default for TextSelectionGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSelectionGroup {
    pub fn new() -> Self {
        Self {
            id: TextSelectionGroupId::next(),
            state: Arc::new(Mutex::new(GroupState::default())),
        }
    }

    pub fn id(&self) -> TextSelectionGroupId {
        self.id
    }

    pub fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = GroupState::default();
        }
    }

    pub fn snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.state.lock().ok().and_then(|state| state.snapshot())
    }

    fn begin(&self, run: &DocumentRun, x: f32, y: f32) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let caret = run.caret_at(x, y);
        state.anchor = Some(caret);
        state.focus = Some(caret);
        state.dragging = true;
        state.pressed_link = run.link_at(x, y);
        state.cached = Some(run.snapshot_range(caret, caret));
        true
    }

    fn drag(&self, run: &DocumentRun, x: f32, y: f32) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if !state.dragging {
            return false;
        }
        let caret = run.caret_at(x, y);
        if state.focus == Some(caret) {
            return false;
        }
        state.focus = Some(caret);
        if state.anchor != state.focus {
            state.pressed_link = None;
        }
        state.cached = Some(run.snapshot_range(state.anchor.unwrap_or(caret), caret));
        true
    }

    fn finish(&self, run: &DocumentRun, x: f32, y: f32) -> Option<RichTextEvent> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        if !state.dragging {
            return None;
        }
        state.dragging = false;
        let caret = run.caret_at(x, y);
        if let Some(focus) = state.focus.as_mut() {
            *focus = caret;
        }
        let snapshot = run.snapshot_range(state.anchor.unwrap_or(caret), caret);
        state.cached = Some(snapshot.clone());
        if let Some(link) = state.pressed_link.take()
            && snapshot
                .as_ref()
                .is_none_or(|value| value.start == value.end)
            && run.link_at(x, y).as_ref() == Some(&link)
        {
            state.anchor = Some(caret);
            state.focus = Some(caret);
            state.cached = None;
            return Some(RichTextEvent::LinkActivated(link));
        }
        Some(RichTextEvent::SelectionChanged(snapshot))
    }
}

#[derive(Debug, Default)]
struct GroupState {
    anchor: Option<usize>,
    focus: Option<usize>,
    dragging: bool,
    pressed_link: Option<Arc<str>>,
    cached: Option<Option<TextSelectionSnapshot>>,
}

impl GroupState {
    fn snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.cached.clone().flatten()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RichTextEvent {
    SelectionChanged(Option<TextSelectionSnapshot>),
    LinkActivated(Arc<str>),
}

#[derive(Clone, Debug)]
pub struct SelectableRichText {
    spans: Vec<RichSpan>,
    selection: TextSelectionGroup,
    pub style: NodeStyle,
}

impl PartialEq for SelectableRichText {
    fn eq(&self, other: &Self) -> bool {
        self.spans == other.spans
    }
}

impl Eq for SelectableRichText {}

impl SelectableRichText {
    pub fn new(spans: impl IntoIterator<Item = RichSpan>) -> Self {
        Self {
            spans: spans.into_iter().collect(),
            selection: TextSelectionGroup::new(),
            style: NodeStyle::default(),
        }
    }

    pub fn spans(&self) -> &[RichSpan] {
        &self.spans
    }

    pub fn selection_group(&self) -> &TextSelectionGroup {
        &self.selection
    }

    pub fn group_id(&self) -> TextSelectionGroupId {
        self.selection.id()
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn plain_text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_ref()).collect()
    }

    pub fn layout(&self, bounds: LayoutBox) -> RichTextGeometry {
        let run = layout_rich_spans(&self.spans, bounds);
        RichTextGeometry {
            bounds: run.bounds,
            graphemes: run.graphemes.clone(),
            run,
        }
    }

    pub fn pointer_down(&self, x: f32, y: f32, bounds: LayoutBox) -> bool {
        let geometry = self.layout(bounds);
        self.selection.begin(&geometry.run, x, y)
    }

    pub fn pointer_move(&self, x: f32, y: f32, bounds: LayoutBox) -> bool {
        let geometry = self.layout(bounds);
        self.selection.drag(&geometry.run, x, y)
    }

    pub fn pointer_up(&self, x: f32, y: f32, bounds: LayoutBox) -> Option<RichTextEvent> {
        let geometry = self.layout(bounds);
        self.selection.finish(&geometry.run, x, y)
    }

    pub fn link_at(&self, x: f32, y: f32, bounds: LayoutBox) -> Option<Arc<str>> {
        self.layout(bounds).run.link_at(x, y)
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection.snapshot().map(|snapshot| snapshot.text)
    }

    pub fn selection_snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.selection.snapshot()
    }

    pub fn copy_snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.selection.snapshot()
    }

    pub fn clear_selection(&self) {
        self.selection.clear();
    }

    fn intrinsic_height(&self) -> f32 {
        line_count(&self.plain_text()).max(1) as f32 * LINE_HEIGHT
    }
}

impl ComponentView for SelectableRichText {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "rich-text".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let plain = self.plain_text();
        if world.text(id) != Some(plain.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: plain.clone(),
                },
            );
        }
        let visual = StandardVisual::SelectableRichText {
            text: Arc::from(plain),
            selection: projected_selection(&self.selection),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        let height = self.intrinsic_height();
        layout.height = Some(LengthSpec::Px(height));
        layout.min_height = Some(LengthSpec::Px(height));
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                value: self
                    .selected_text()
                    .map(Arc::from)
                    .or_else(|| Some(Arc::from(self.plain_text().as_str()))),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphemeGeometry {
    pub index: usize,
    pub bounds: LayoutBox,
    pub grapheme: Arc<str>,
    pub span_index: usize,
    pub block_index: usize,
    pub link: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RichTextGeometry {
    pub bounds: LayoutBox,
    pub graphemes: Vec<GraphemeGeometry>,
    run: DocumentRun,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownBlockGeometry {
    pub index: usize,
    pub bounds: LayoutBox,
    pub language: Option<Arc<str>>,
    pub label: Option<Arc<str>>,
    pub graphemes: Vec<GraphemeGeometry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownGeometry {
    pub bounds: LayoutBox,
    pub blocks: Vec<MarkdownBlockGeometry>,
    run: DocumentRun,
}

#[derive(Clone, Debug, PartialEq)]
struct DocumentRun {
    bounds: LayoutBox,
    graphemes: Vec<GraphemeGeometry>,
    separators: Vec<&'static str>,
}

impl DocumentRun {
    fn empty(bounds: LayoutBox) -> Self {
        Self {
            bounds,
            graphemes: Vec::new(),
            separators: Vec::new(),
        }
    }

    fn caret_at(&self, x: f32, y: f32) -> usize {
        let length = self.graphemes.len();
        if length == 0 {
            return 0;
        }
        let first_top = self
            .graphemes
            .iter()
            .map(|item| item.bounds.y)
            .min_by(f32::total_cmp)
            .unwrap_or(self.bounds.y);
        let last_bottom = self
            .graphemes
            .iter()
            .map(|item| item.bounds.y + item.bounds.height)
            .max_by(f32::total_cmp)
            .unwrap_or(self.bounds.y + self.bounds.height);
        if y < first_top {
            return 0;
        }
        if y > last_bottom {
            return length;
        }
        let Some((index, bounds)) = self
            .graphemes
            .iter()
            .map(|item| (item.index, item.bounds))
            .min_by(|(_, left), (_, right)| {
                point_box_distance_sq(x, y, *left).total_cmp(&point_box_distance_sq(x, y, *right))
            })
        else {
            return 0;
        };
        if x >= bounds.x + bounds.width / 2.0 {
            index.saturating_add(1).min(length)
        } else {
            index
        }
    }

    fn link_at(&self, x: f32, y: f32) -> Option<Arc<str>> {
        self.graphemes
            .iter()
            .find(|item| item.bounds.contains(x, y))
            .and_then(|item| item.link.clone())
    }

    fn snapshot_range(&self, anchor: usize, focus: usize) -> Option<TextSelectionSnapshot> {
        let length = self.graphemes.len();
        let start = anchor.min(focus).min(length);
        let end = anchor.max(focus).min(length);
        if start == end {
            return None;
        }
        let mut text = String::new();
        for index in start..end {
            if !text.is_empty() {
                text.push_str(self.separators.get(index).copied().unwrap_or(""));
            }
            if let Some(item) = self.graphemes.get(index) {
                text.push_str(item.grapheme.as_ref());
            }
        }
        Some(TextSelectionSnapshot { start, end, text })
    }
}

struct LayoutCursor {
    origin_x: f32,
    max_width: f32,
    line_height: f32,
    x: f32,
    y: f32,
    line_start_x: f32,
}

impl LayoutCursor {
    fn new(origin_x: f32, y: f32, max_width: f32, line_height: f32) -> Self {
        Self {
            origin_x,
            max_width,
            line_height,
            x: origin_x,
            y,
            line_start_x: origin_x,
        }
    }

    fn place(&mut self, grapheme: &str) -> LayoutBox {
        if is_newline(grapheme) {
            let bounds = LayoutBox {
                x: self.x,
                y: self.y,
                width: 0.0,
                height: self.line_height,
            };
            self.x = self.line_start_x;
            self.y += self.line_height;
            return bounds;
        }
        if self.max_width.is_finite()
            && self.x > self.line_start_x
            && self.x + GRAPHEME_ADVANCE - self.origin_x > self.max_width
        {
            self.x = self.line_start_x;
            self.y += self.line_height;
        }
        let bounds = LayoutBox {
            x: self.x,
            y: self.y,
            width: GRAPHEME_ADVANCE,
            height: self.line_height,
        };
        self.x += GRAPHEME_ADVANCE;
        bounds
    }

    fn height_from(&self, top: f32) -> f32 {
        (self.y + self.line_height - top).max(self.line_height)
    }
}

fn layout_rich_spans(spans: &[RichSpan], bounds: LayoutBox) -> DocumentRun {
    let mut graphemes = Vec::new();
    let mut separators = Vec::new();
    let mut cursor = LayoutCursor::new(bounds.x, bounds.y, usable_width(bounds.width), LINE_HEIGHT);
    for (span_index, span) in spans.iter().enumerate() {
        for grapheme in span.text.graphemes(true) {
            if grapheme.is_empty() {
                continue;
            }
            let index = graphemes.len();
            separators.push("");
            graphemes.push(GraphemeGeometry {
                index,
                bounds: cursor.place(grapheme),
                grapheme: Arc::from(grapheme),
                span_index,
                block_index: 0,
                link: span.link.clone(),
            });
        }
    }
    let height = if graphemes.is_empty() {
        LINE_HEIGHT
    } else {
        cursor.height_from(bounds.y)
    };
    DocumentRun {
        bounds: LayoutBox { height, ..bounds },
        graphemes,
        separators,
    }
}

fn layout_markdown(blocks: &[MarkdownBlock], bounds: LayoutBox) -> MarkdownGeometry {
    let mut run = DocumentRun::empty(bounds);
    let mut block_geometry = Vec::with_capacity(blocks.len());
    let mut y = bounds.y;
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            y += BLOCK_GAP;
        }
        let geometry = layout_block(index, block, bounds.x, y, bounds.width, &mut run);
        y = geometry.bounds.y + geometry.bounds.height;
        block_geometry.push(geometry);
    }
    let height = (y - bounds.y).max(if blocks.is_empty() { LINE_HEIGHT } else { 0.0 });
    run.bounds = LayoutBox { height, ..bounds };
    MarkdownGeometry {
        bounds: run.bounds,
        blocks: block_geometry,
        run,
    }
}

fn layout_block(
    index: usize,
    block: &MarkdownBlock,
    x: f32,
    y: f32,
    width: f32,
    run: &mut DocumentRun,
) -> MarkdownBlockGeometry {
    match block {
        MarkdownBlock::Text { kind, spans } => {
            let indent = text_indent(*kind);
            let line_height = text_line_height(*kind);
            let mut cursor = LayoutCursor::new(
                x + indent,
                y,
                usable_width((width - indent).max(0.0)),
                line_height,
            );
            let start = run.graphemes.len();
            push_spans(
                run,
                &mut cursor,
                index,
                spans,
                if start == 0 { "" } else { "\n\n" },
            );
            labeled_block(
                index,
                x,
                y,
                width,
                cursor.height_from(y),
                None,
                None,
                run.graphemes[start..].to_vec(),
            )
        }
        MarkdownBlock::Code { language, source } => {
            let mut cursor = LayoutCursor::new(x, y, usable_width(width), LINE_HEIGHT);
            let start = run.graphemes.len();
            push_source(
                run,
                &mut cursor,
                index,
                source,
                if start == 0 { "" } else { "\n\n" },
            );
            labeled_block(
                index,
                x,
                y,
                width,
                cursor.height_from(y),
                language
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .map(Arc::from),
                None,
                run.graphemes[start..].to_vec(),
            )
        }
        MarkdownBlock::DisplayMath(source) => labeled_block(
            index,
            x,
            y,
            width,
            LINE_HEIGHT,
            None,
            Some(Arc::from(format!("math:{source}"))),
            Vec::new(),
        ),
        MarkdownBlock::Mermaid(source) => labeled_block(
            index,
            x,
            y,
            width,
            LINE_HEIGHT,
            None,
            Some(Arc::from(format!("mermaid:{source}"))),
            Vec::new(),
        ),
        MarkdownBlock::Table(table) => layout_table(index, table, x, y, width, run),
        MarkdownBlock::Rule => labeled_block(
            index,
            x,
            y,
            width,
            1.0,
            None,
            Some(Arc::from("---")),
            Vec::new(),
        ),
    }
}

fn layout_table(
    index: usize,
    table: &MarkdownTable,
    x: f32,
    y: f32,
    width: f32,
    run: &mut DocumentRun,
) -> MarkdownBlockGeometry {
    let start = run.graphemes.len();
    let mut cursor = LayoutCursor::new(x, y, usable_width(width), LINE_HEIGHT);
    let mut first_cell = true;
    for (row_index, row) in std::iter::once(table.header.as_slice())
        .chain(table.rows.iter().map(Vec::as_slice))
        .enumerate()
    {
        for (column_index, cell) in row.iter().enumerate() {
            let separator = if first_cell {
                if start == 0 { "" } else { "\n\n" }
            } else if column_index == 0 {
                "\n"
            } else {
                "\t"
            };
            if row_index > 0 && column_index == 0 {
                cursor.x = cursor.line_start_x;
                cursor.y += cursor.line_height;
            }
            push_spans(run, &mut cursor, index, cell, separator);
            first_cell = false;
        }
    }
    labeled_block(
        index,
        x,
        y,
        width,
        cursor.height_from(y),
        None,
        None,
        run.graphemes[start..].to_vec(),
    )
}

#[allow(clippy::too_many_arguments)]
fn labeled_block(
    index: usize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    language: Option<Arc<str>>,
    label: Option<Arc<str>>,
    graphemes: Vec<GraphemeGeometry>,
) -> MarkdownBlockGeometry {
    MarkdownBlockGeometry {
        index,
        bounds: LayoutBox {
            x,
            y,
            width,
            height,
        },
        language,
        label,
        graphemes,
    }
}

fn push_spans(
    run: &mut DocumentRun,
    cursor: &mut LayoutCursor,
    block_index: usize,
    spans: &[MarkdownSpan],
    first_separator: &'static str,
) {
    let mut first = true;
    for (span_index, span) in spans.iter().enumerate() {
        for grapheme in span.text.graphemes(true) {
            if grapheme.is_empty() {
                continue;
            }
            let separator = if first { first_separator } else { "" };
            first = false;
            push_grapheme(
                run,
                cursor,
                block_index,
                span_index,
                grapheme,
                span.link.as_deref().map(Arc::from),
                separator,
            );
        }
    }
}

fn push_source(
    run: &mut DocumentRun,
    cursor: &mut LayoutCursor,
    block_index: usize,
    source: &str,
    first_separator: &'static str,
) {
    let mut first = true;
    for grapheme in source.graphemes(true) {
        if grapheme.is_empty() {
            continue;
        }
        let separator = if first { first_separator } else { "" };
        first = false;
        push_grapheme(run, cursor, block_index, 0, grapheme, None, separator);
    }
}

fn push_grapheme(
    run: &mut DocumentRun,
    cursor: &mut LayoutCursor,
    block_index: usize,
    span_index: usize,
    grapheme: &str,
    link: Option<Arc<str>>,
    separator: &'static str,
) {
    let index = run.graphemes.len();
    run.separators.push(separator);
    run.graphemes.push(GraphemeGeometry {
        index,
        bounds: cursor.place(grapheme),
        grapheme: Arc::from(grapheme),
        span_index,
        block_index,
        link,
    });
}

fn projected_selection(group: &TextSelectionGroup) -> Option<(usize, usize)> {
    group
        .snapshot()
        .map(|snapshot| (snapshot.start, snapshot.end))
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

fn markdown_intrinsic_height(blocks: &[MarkdownBlock]) -> f32 {
    if blocks.is_empty() {
        return LINE_HEIGHT;
    }
    let mut height = 0.0;
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            height += BLOCK_GAP;
        }
        height += match block {
            MarkdownBlock::Text { kind, spans } => {
                line_count(&markdown_spans_plain_text(spans)).max(1) as f32
                    * text_line_height(*kind)
            }
            MarkdownBlock::Code { source, .. } => line_count(source).max(1) as f32 * LINE_HEIGHT,
            MarkdownBlock::Table(table) => {
                let rows = usize::from(!table.header.is_empty()) + table.rows.len();
                rows.max(1) as f32 * LINE_HEIGHT
            }
            MarkdownBlock::DisplayMath(_) | MarkdownBlock::Mermaid(_) => LINE_HEIGHT,
            MarkdownBlock::Rule => 1.0,
        };
    }
    height
}

fn text_indent(kind: MarkdownBlockKind) -> f32 {
    match kind {
        MarkdownBlockKind::Quote => QUOTE_INDENT,
        MarkdownBlockKind::ListItem { depth } => depth.saturating_sub(1) as f32 * LIST_INDENT,
        _ => 0.0,
    }
}

fn text_line_height(kind: MarkdownBlockKind) -> f32 {
    match kind {
        MarkdownBlockKind::Heading(level) => {
            (24.0 - f32::from(level.saturating_sub(1)) * 1.5).max(LINE_HEIGHT)
        }
        _ => LINE_HEIGHT,
    }
}

fn line_count(value: &str) -> usize {
    value.split('\n').count()
}

fn usable_width(width: f32) -> f32 {
    if width.is_finite() && width > 0.0 {
        width
    } else {
        f32::INFINITY
    }
}

fn is_newline(value: &str) -> bool {
    matches!(value, "\n" | "\r" | "\r\n" | "\n\r")
}

fn point_box_distance_sq(x: f32, y: f32, bounds: LayoutBox) -> f32 {
    let dx = if x < bounds.x {
        bounds.x - x
    } else if x > bounds.x + bounds.width {
        x - (bounds.x + bounds.width)
    } else {
        0.0
    };
    let dy = if y < bounds.y {
        bounds.y - y
    } else if y > bounds.y + bounds.height {
        y - (bounds.y + bounds.height)
    } else {
        0.0
    };
    dx.mul_add(dx, dy * dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn bounds(width: f32, height: f32) -> LayoutBox {
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }

    fn caret_x(index: usize) -> f32 {
        index as f32 * GRAPHEME_ADVANCE + 1.0
    }

    #[test]
    fn heading_and_paragraph_blocks_are_distinct() {
        let markdown = NativeMarkdown::from_blocks([
            MarkdownBlock::heading(1, [MarkdownSpan::plain("Title")]),
            MarkdownBlock::paragraph([MarkdownSpan::plain("Body")]),
            MarkdownBlock::DisplayMath("\\frac{1}{2}".into()),
            MarkdownBlock::Mermaid("flowchart LR\nA-->B".into()),
        ]);
        assert!(matches!(
            markdown.blocks()[0],
            MarkdownBlock::Text {
                kind: MarkdownBlockKind::Heading(1),
                ..
            }
        ));
        assert!(matches!(
            markdown.blocks()[1],
            MarkdownBlock::Text {
                kind: MarkdownBlockKind::Paragraph,
                ..
            }
        ));
        assert_eq!(
            markdown.plain_text(),
            "Title\n\nBody\n\n\\frac{1}{2}\n\nflowchart LR\nA-->B"
        );
        let geometry = markdown.layout(bounds(400.0, 200.0));
        assert_eq!(geometry.blocks.len(), 4);
        assert_eq!(
            geometry.blocks[2].label.as_deref(),
            Some("math:\\frac{1}{2}")
        );
        assert!(
            geometry.blocks[3]
                .label
                .as_deref()
                .is_some_and(|label| label.starts_with("mermaid:"))
        );
        assert!(geometry.blocks[2].graphemes.is_empty());
        assert!(geometry.blocks[3].graphemes.is_empty());
    }

    #[test]
    fn pointer_drag_sets_selection_start_and_end() {
        let text = SelectableRichText::new([RichSpan::plain("A你e\u{301}Hello")]);
        let area = bounds(400.0, 20.0);
        assert!(text.pointer_down(caret_x(0), 8.0, area));
        assert!(text.pointer_move(caret_x(3), 8.0, area));
        let event = text.pointer_up(caret_x(3), 8.0, area);
        assert_eq!(
            event,
            Some(RichTextEvent::SelectionChanged(Some(
                TextSelectionSnapshot {
                    start: 0,
                    end: 3,
                    text: "A你e\u{301}".into(),
                }
            )))
        );
        let snapshot = text.selection_snapshot().expect("selection");
        assert_eq!(snapshot.start, 0);
        assert_eq!(snapshot.end, 3);
        assert_eq!(snapshot.text, "A你e\u{301}");
        assert_eq!(text.copy_snapshot(), Some(snapshot));
    }

    #[test]
    fn markdown_pointer_drag_selects_across_blocks() {
        let markdown = NativeMarkdown::from_blocks([
            MarkdownBlock::paragraph([MarkdownSpan::plain("Hello")]),
            MarkdownBlock::paragraph([MarkdownSpan::plain("World")]),
        ]);
        let area = bounds(400.0, 80.0);
        let world_y = 8.0 + LINE_HEIGHT + BLOCK_GAP;
        assert!(markdown.pointer_down(caret_x(0), 8.0, area));
        assert!(markdown.pointer_move(caret_x(5), world_y, area));
        let event = markdown.pointer_up(caret_x(5), world_y, area);
        assert_eq!(
            event,
            Some(RichTextEvent::SelectionChanged(Some(
                TextSelectionSnapshot {
                    start: 0,
                    end: 10,
                    text: "Hello\n\nWorld".into(),
                }
            )))
        );
    }

    #[test]
    fn link_hit_activates_on_click_without_drag() {
        let text = SelectableRichText::new([
            RichSpan::plain("See "),
            RichSpan::link("docs", "https://example.com/docs"),
        ]);
        let area = bounds(400.0, 20.0);
        let link_x = caret_x(5);
        assert_eq!(
            text.link_at(link_x, 8.0, area).as_deref(),
            Some("https://example.com/docs")
        );
        assert!(text.pointer_down(link_x, 8.0, area));
        assert_eq!(
            text.pointer_up(link_x, 8.0, area),
            Some(RichTextEvent::LinkActivated(Arc::from(
                "https://example.com/docs"
            )))
        );
        assert_eq!(text.selection_snapshot(), None);
    }

    #[test]
    fn empty_spans_are_omitted_from_geometry_and_selection() {
        let text = SelectableRichText::new([
            RichSpan::plain(""),
            RichSpan::plain("Hi"),
            RichSpan::plain(""),
        ]);
        let area = bounds(400.0, 20.0);
        let geometry = text.layout(area);
        assert_eq!(geometry.graphemes.len(), 2);
        assert_eq!(text.plain_text(), "Hi");
        assert!(text.pointer_down(caret_x(0), 8.0, area));
        assert!(text.pointer_move(caret_x(2), 8.0, area));
        assert_eq!(
            text.pointer_up(caret_x(2), 8.0, area),
            Some(RichTextEvent::SelectionChanged(Some(
                TextSelectionSnapshot {
                    start: 0,
                    end: 2,
                    text: "Hi".into(),
                }
            )))
        );
    }

    #[test]
    fn code_block_retains_language_and_highlight_request() {
        let markdown =
            NativeMarkdown::from_blocks([MarkdownBlock::code(Some("rust"), "fn main() {}")]);
        assert_eq!(markdown.blocks()[0].language(), Some("rust"));
        assert_eq!(
            markdown.blocks()[0].highlight_request(),
            Some(HighlightRequest::highlight("rust"))
        );
        assert_eq!(
            markdown.code_highlights(),
            [(0, HighlightRequest::highlight("rust"))]
        );
        let geometry = markdown.layout(bounds(400.0, 40.0));
        assert_eq!(geometry.blocks[0].language.as_deref(), Some("rust"));
        assert_eq!(markdown.plain_text(), "fn main() {}");

        let unlabeled = NativeMarkdown::from_blocks([MarkdownBlock::code(None, "plain")]);
        assert_eq!(unlabeled.blocks()[0].language(), None);
        assert_eq!(unlabeled.blocks()[0].highlight_request(), None);
        assert!(unlabeled.code_highlights().is_empty());
    }

    #[test]
    fn markdown_projects_plain_text_on_a_retained_leaf() {
        let mut context = AppContext::new();
        let markdown = context
            .create_component(
                document(),
                NativeMarkdown::from_blocks([
                    MarkdownBlock::heading(2, [MarkdownSpan::plain("Title")]),
                    MarkdownBlock::paragraph([MarkdownSpan::plain("Body")]),
                ]),
            )
            .unwrap();
        let id = markdown.stable_id();
        assert_eq!(context.world().text(id), Some("Title\n\nBody"));
        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::NativeMarkdown {
                text: Arc::from("Title\n\nBody"),
                selection: None,
            })
        );
        assert!(context.world().highlight_request(id).is_none());
    }

    #[test]
    fn rich_text_projects_concatenated_spans_and_visual() {
        let mut context = AppContext::new();
        let text = context
            .create_component(
                document(),
                SelectableRichText::new([
                    RichSpan::plain("See "),
                    RichSpan::link("docs", "https://example.com/docs"),
                ]),
            )
            .unwrap();
        let id = text.stable_id();
        assert_eq!(context.world().text(id), Some("See docs"));
        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::SelectableRichText {
                text: Arc::from("See docs"),
                selection: None,
            })
        );
    }

    #[test]
    fn markdown_projects_selection_after_pointer_drag() {
        let mut context = AppContext::new();
        let markdown = context
            .create_component(
                document(),
                NativeMarkdown::from_blocks([
                    MarkdownBlock::paragraph([MarkdownSpan::plain("Hello")]),
                    MarkdownBlock::paragraph([MarkdownSpan::plain("World")]),
                ]),
            )
            .unwrap();
        let id = markdown.stable_id();
        let area = bounds(400.0, 80.0);
        let world_y = 8.0 + LINE_HEIGHT + BLOCK_GAP;
        context
            .update_component(markdown, |markdown, _| {
                assert!(markdown.pointer_down(caret_x(0), 8.0, area));
                assert!(markdown.pointer_move(caret_x(5), world_y, area));
                markdown.pointer_up(caret_x(5), world_y, area);
            })
            .unwrap();
        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::NativeMarkdown {
                text: Arc::from("Hello\n\nWorld"),
                selection: Some((0, 10)),
            })
        );
        assert_eq!(context.world().text(id), Some("Hello\n\nWorld"));
    }

    #[test]
    fn rich_text_projects_selection_after_pointer_drag() {
        let mut context = AppContext::new();
        let text = context
            .create_component(
                document(),
                SelectableRichText::new([RichSpan::plain("A你e\u{301}Hello")]),
            )
            .unwrap();
        let id = text.stable_id();
        let area = bounds(400.0, 20.0);
        context
            .update_component(text, |text, _| {
                assert!(text.pointer_down(caret_x(0), 8.0, area));
                assert!(text.pointer_move(caret_x(3), 8.0, area));
                text.pointer_up(caret_x(3), 8.0, area);
            })
            .unwrap();
        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::SelectableRichText {
                text: Arc::from("A你e\u{301}Hello"),
                selection: Some((0, 3)),
            })
        );
        assert_eq!(context.world().text(id), Some("A你e\u{301}Hello"));
    }
}
