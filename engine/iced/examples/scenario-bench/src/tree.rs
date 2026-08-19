//! Shared complete-binary-heap tree used by Nana `tree_mutations` and this bench.
//!
//! `parent(i)=i/2`, root `1`, element-div. StaticTree has no text. Mutation/Hover
//! decorate a single known node; they do not change topology.

use iced::widget::{column, container, mouse_area, row, space, text};
use iced::{Background, Color, Element, Event, Length, Rectangle, Size, Theme};
use iced_wgpu::Renderer;
use iced_winit::core::Shell;
use iced_winit::core::layout;
use iced_winit::core::mouse;
use iced_winit::core::renderer;
use iced_winit::core::widget::{self, Widget};
use iced_winit::core::window;

use std::ops::Range;

pub type BenchElement<'a> = Element<'a, (), Theme, Renderer>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationKind {
    PaintOnly,
    Text,
    LayoutStyle,
    Visibility,
    Transform,
    Accessibility,
}

impl MutationKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "PaintOnly" => Ok(Self::PaintOnly),
            "Text" => Ok(Self::Text),
            "LayoutStyle" => Ok(Self::LayoutStyle),
            "Visibility" => Ok(Self::Visibility),
            "Transform" => Ok(Self::Transform),
            "Accessibility" => Ok(Self::Accessibility),
            other => Err(format!(
                "unknown Mutation.params.kind {other:?}; expected PaintOnly, Text, \
                 LayoutStyle, Visibility, Transform, or Accessibility"
            )),
        }
    }

    pub fn is_same_scenario(self) -> bool {
        matches!(self, Self::PaintOnly | Self::Text | Self::LayoutStyle)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PaintOnly => "PaintOnly",
            Self::Text => "Text",
            Self::LayoutStyle => "LayoutStyle",
            Self::Visibility => "Visibility",
            Self::Transform => "Transform",
            Self::Accessibility => "Accessibility",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MutationSpec {
    pub kind: MutationKind,
    pub target: usize,
    pub even: bool,
}

/// 1-based heap index mutated for this kind. Matches Nana `measure_single_node_mutations`
/// plus PaintOnly on the last node (`local_paint` target).
pub fn mutation_target(kind: MutationKind, nodes: usize) -> usize {
    match kind {
        MutationKind::PaintOnly => nodes,
        MutationKind::Text => (nodes / 2).max(2),
        MutationKind::LayoutStyle => (nodes / 2 + 1).max(3),
        MutationKind::Visibility => (nodes / 2 + 2).max(4),
        MutationKind::Transform => (nodes / 2 + 3).max(5),
        MutationKind::Accessibility => (nodes / 2 + 4).max(6),
    }
}

pub fn static_tree_parent(index: usize) -> Option<usize> {
    (index > 1).then_some(index / 2)
}

pub fn sample_parents(nodes: usize) -> serde_json::Value {
    let mut indexes = vec![1usize, 2, 3];
    if nodes >= 50 {
        indexes.push(50);
    }
    indexes.push(nodes);
    indexes.sort_unstable();
    indexes.dedup();
    serde_json::Value::Array(
        indexes
            .into_iter()
            .filter(|index| *index >= 1 && *index <= nodes)
            .map(|index| {
                serde_json::json!({
                    "index": index,
                    "parent": static_tree_parent(index),
                })
            })
            .collect(),
    )
}

pub fn tree_provenance(nodes: usize) -> serde_json::Value {
    serde_json::json!({
        "generation": "complete-binary-heap",
        "parent_rule": "parent(i)=i//2, root=1",
        "node_kind": "element-div",
        "text": null,
        "sample_parents": sample_parents(nodes),
    })
}

pub fn static_tree<'a>(nodes: usize) -> BenchElement<'a> {
    heap_node(1, nodes, None, None, &[])
}

pub fn mutation_tree<'a>(nodes: usize, spec: MutationSpec) -> BenchElement<'a> {
    heap_node(1, nodes, Some(spec), None, &[])
}

pub fn hover_tree<'a>(nodes: usize, hovered: Option<usize>) -> BenchElement<'a> {
    let last = nodes;
    let previous = nodes.saturating_sub(1).max(1);
    heap_node(1, nodes, None, hovered, &[previous, last])
}

fn heap_node<'a>(
    index: usize,
    nodes: usize,
    mutation: Option<MutationSpec>,
    hovered: Option<usize>,
    hover_targets: &[usize],
) -> BenchElement<'a> {
    let mut children = Vec::new();
    let left = index * 2;
    let right = left + 1;
    if left <= nodes {
        children.push(heap_node(left, nodes, mutation, hovered, hover_targets));
    }
    if right <= nodes {
        children.push(heap_node(right, nodes, mutation, hovered, hover_targets));
    }

    let text_target =
        mutation.filter(|spec| spec.kind == MutationKind::Text && spec.target == index);
    let inner: BenchElement<'a> = match (children.is_empty(), text_target) {
        (true, Some(spec)) => text(if spec.even { "nana" } else { "ui!!" }).into(),
        (true, None) => space().width(1).height(1).into(),
        (false, Some(spec)) => {
            let mut all = vec![text(if spec.even { "nana" } else { "ui!!" }).into()];
            all.extend(children);
            column(all).into()
        }
        (false, None) => column(children).into(),
    };

    let mut box_ = container(inner);
    if let Some(spec) = mutation.filter(|spec| spec.target == index) {
        box_ = apply_mutation(box_, spec);
    }
    if hovered == Some(index) {
        box_ = box_.style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgb(0.2, 0.5, 0.9))),
            ..container::Style::default()
        });
    }

    let element: BenchElement<'a> = box_.into();
    if hover_targets.contains(&index) {
        mouse_area(element).into()
    } else {
        element
    }
}

fn apply_mutation<'a>(
    box_: iced::widget::Container<'a, (), Theme, Renderer>,
    spec: MutationSpec,
) -> iced::widget::Container<'a, (), Theme, Renderer> {
    match spec.kind {
        MutationKind::PaintOnly => box_.style(move |_| {
            let color = if spec.even {
                Color::from_rgb(0.2, 0.4, 0.8)
            } else {
                Color::from_rgb(0.8, 0.4, 0.2)
            };
            container::Style {
                background: Some(Background::Color(color)),
                ..container::Style::default()
            }
        }),
        MutationKind::Text => box_,
        MutationKind::LayoutStyle => {
            box_.width(Length::Fixed(if spec.even { 120.0 } else { 140.0 }))
        }
        MutationKind::Visibility | MutationKind::Transform | MutationKind::Accessibility => {
            unreachable!("unsupported mutation kinds are refused before apply_mutation")
        }
    }
}

pub fn virtual_window(
    items: usize,
    scroll_px: f32,
    visible: usize,
    overscan: usize,
    item_extent: f32,
) -> Range<usize> {
    if items == 0 || item_extent <= 0.0 {
        return 0..0;
    }
    let viewport = visible as f32 * item_extent;
    let overscan_px = overscan as f32 * item_extent;
    let start_off = (scroll_px - overscan_px).max(0.0);
    let end_off = scroll_px + viewport + overscan_px;
    let start = ((start_off / item_extent).floor() as usize).min(items);
    let end = ((end_off / item_extent).ceil() as usize).clamp(start, items);
    start..end
}

pub fn live_ui_entities_bound(visible: usize, overscan: usize) -> usize {
    visible.saturating_add(2 * overscan).saturating_add(2)
}

pub fn virtual_list_view<'a>(
    range: Range<usize>,
    text_len: usize,
    item_extent: f32,
    leading: f32,
    trailing: f32,
) -> BenchElement<'a> {
    let mut children: Vec<BenchElement<'a>> = Vec::new();
    if leading > 0.0 {
        children.push(space().height(Length::Fixed(leading)).into());
    }
    for index in range {
        children.push(
            container(text(item_label(index, text_len)))
                .height(Length::Fixed(item_extent))
                .width(Length::Fill)
                .into(),
        );
    }
    if trailing > 0.0 {
        children.push(space().height(Length::Fixed(trailing)).into());
    }
    column(children).into()
}

pub const TABLE_ROW_EXTENT_PX: f32 = 20.0;
pub const TABLE_COLUMN_EXTENT_PX: f32 = 80.0;
pub const TABLE_VISIBLE_BAND: usize = 40;
pub const TABLE_WRAPPED_CELLS: usize = 4;
pub const TABLE_SHORT_CELL_LEN: usize = 8;
pub const TABLE_WRAPPED_CELL_LEN: usize = 256;

pub fn table_axis_window(
    items: usize,
    scroll_px: f32,
    visible: usize,
    overscan: usize,
    extent: f32,
) -> Range<usize> {
    virtual_window(items, scroll_px, visible, overscan, extent)
}

pub fn table_live_ui_entities_bound(
    visible_rows: usize,
    overscan_rows: usize,
    visible_columns: usize,
    overscan_columns: usize,
) -> usize {
    let rows = live_ui_entities_bound(visible_rows, overscan_rows);
    let columns = live_ui_entities_bound(visible_columns, overscan_columns);
    rows + rows * columns
}

fn is_wrapped_table_cell(row: usize, column: usize) -> bool {
    column == 0 && row % TABLE_VISIBLE_BAND < TABLE_WRAPPED_CELLS
}

fn padded_table_cell_text(prefix: &str, row: usize, column: usize, len: usize) -> String {
    let mut text = format!("{prefix}{row}:{column}");
    if text.len() < len {
        text.extend(std::iter::repeat_n('x', len - text.len()));
    }
    text.truncate(len);
    text
}

pub fn table_cell_text(row: usize, column: usize) -> String {
    if is_wrapped_table_cell(row, column) {
        padded_table_cell_text("wrap ", row, column, TABLE_WRAPPED_CELL_LEN)
    } else {
        padded_table_cell_text("", row, column, TABLE_SHORT_CELL_LEN)
    }
}

pub fn virtual_table_view<'a>(
    row_range: Range<usize>,
    col_range: Range<usize>,
    row_extent: f32,
    col_extent: f32,
    leading_y: f32,
    trailing_y: f32,
) -> BenchElement<'a> {
    let mut rows: Vec<BenchElement<'a>> = Vec::new();
    if leading_y > 0.0 {
        rows.push(space().height(Length::Fixed(leading_y)).into());
    }
    for row_index in row_range {
        let mut cells: Vec<BenchElement<'a>> = Vec::new();
        for col_index in col_range.clone() {
            cells.push(
                container(text(table_cell_text(row_index, col_index)))
                    .width(Length::Fixed(col_extent))
                    .height(Length::Fixed(row_extent))
                    .into(),
            );
        }
        rows.push(row(cells).into());
    }
    if trailing_y > 0.0 {
        rows.push(space().height(Length::Fixed(trailing_y)).into());
    }
    column(rows).into()
}

fn item_label(index: usize, text_len: usize) -> String {
    let base = format!("row-{index}");
    if text_len == 0 {
        return String::new();
    }
    if base.len() >= text_len {
        base.chars().take(text_len).collect()
    } else {
        format!("{base}{}", "x".repeat(text_len - base.len()))
    }
}

/// Widget that requests another UI frame on every `RedrawRequested`.
/// Used to prove `frames_after_idle` is not hardcoded to 0.
pub struct BusyPulse;

pub fn busy_pulse<'a>() -> BenchElement<'a> {
    BusyPulse.into()
}

impl<Message> Widget<Message, Theme, Renderer> for BusyPulse {
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fixed(1.0),
            height: Length::Fixed(1.0),
        }
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fixed(1.0), Length::Fixed(1.0))
    }

    fn update(
        &mut self,
        _tree: &mut widget::Tree,
        event: &Event,
        _layout: layout::Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        _tree: &widget::Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: layout::Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }
}

impl<'a, Message> From<BusyPulse> for Element<'a, Message, Theme, Renderer> {
    fn from(pulse: BusyPulse) -> Self {
        Element::new(pulse)
    }
}
