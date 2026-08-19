//! Shared complete-binary-heap tree used by Nana `tree_mutations` and Iced/GPUI
//! Issue #12 observation benches.
//!
//! `parent(i)=i/2`, root `1`, element-div. StaticTree has no text. Mutation/Hover
//! decorate a single known node; they do not change topology.

use gpui::prelude::*;
use gpui::{AnyElement, SharedString, div, px, rgb};

use std::ops::Range;

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

pub fn static_tree(nodes: usize) -> AnyElement {
    heap_node(1, nodes, None, None)
}

pub fn mutation_tree(nodes: usize, spec: MutationSpec) -> AnyElement {
    heap_node(1, nodes, Some(spec), None)
}

pub fn hover_tree(nodes: usize, hovered: Option<usize>) -> AnyElement {
    heap_node(1, nodes, None, hovered)
}

fn heap_node(
    index: usize,
    nodes: usize,
    mutation: Option<MutationSpec>,
    hovered: Option<usize>,
) -> AnyElement {
    let left = index * 2;
    let right = left + 1;
    let mut children: Vec<AnyElement> = Vec::new();
    if left <= nodes {
        children.push(heap_node(left, nodes, mutation, hovered));
    }
    if right <= nodes {
        children.push(heap_node(right, nodes, mutation, hovered));
    }

    let text_target =
        mutation.filter(|spec| spec.kind == MutationKind::Text && spec.target == index);
    let inner: AnyElement = match (children.is_empty(), text_target) {
        (true, Some(spec)) => div()
            .child(SharedString::from(if spec.even { "nana" } else { "ui!!" }))
            .into_any_element(),
        (true, None) => div().w(px(1.)).h(px(1.)).into_any_element(),
        (false, Some(spec)) => {
            let mut all = vec![
                div()
                    .child(SharedString::from(if spec.even { "nana" } else { "ui!!" }))
                    .into_any_element(),
            ];
            all.extend(children);
            div().flex().flex_col().children(all).into_any_element()
        }
        (false, None) => div()
            .flex()
            .flex_col()
            .children(children)
            .into_any_element(),
    };

    let mut box_ = div().child(inner);
    if let Some(spec) = mutation.filter(|spec| spec.target == index) {
        box_ = apply_mutation(box_, spec);
    }
    if hovered == Some(index) {
        box_ = box_.bg(rgb(0x3380e6));
    }
    box_.into_any_element()
}

fn apply_mutation(box_: gpui::Div, spec: MutationSpec) -> gpui::Div {
    match spec.kind {
        MutationKind::PaintOnly => {
            let color = if spec.even {
                rgb(0x3366cc)
            } else {
                rgb(0xcc6633)
            };
            box_.bg(color)
        }
        MutationKind::Text => box_,
        MutationKind::LayoutStyle => box_.w(px(if spec.even { 120. } else { 140. })),
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

pub fn virtual_list_view(
    range: Range<usize>,
    text_len: usize,
    item_extent: f32,
    leading: f32,
    trailing: f32,
) -> AnyElement {
    let mut children: Vec<AnyElement> = Vec::new();
    if leading > 0.0 {
        children.push(div().h(px(leading)).into_any_element());
    }
    for index in range {
        children.push(
            div()
                .h(px(item_extent))
                .w_full()
                .child(SharedString::from(item_label(index, text_len)))
                .into_any_element(),
        );
    }
    if trailing > 0.0 {
        children.push(div().h(px(trailing)).into_any_element());
    }
    div()
        .flex()
        .flex_col()
        .children(children)
        .into_any_element()
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

pub fn virtual_table_view(
    row_range: Range<usize>,
    col_range: Range<usize>,
    row_extent: f32,
    col_extent: f32,
    leading_y: f32,
    trailing_y: f32,
) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    if leading_y > 0.0 {
        rows.push(div().h(px(leading_y)).into_any_element());
    }
    for row_index in row_range {
        let mut cells: Vec<AnyElement> = Vec::new();
        for col_index in col_range.clone() {
            cells.push(
                div()
                    .w(px(col_extent))
                    .h(px(row_extent))
                    .child(SharedString::from(table_cell_text(row_index, col_index)))
                    .into_any_element(),
            );
        }
        rows.push(div().flex().children(cells).into_any_element());
    }
    if trailing_y > 0.0 {
        rows.push(div().h(px(trailing_y)).into_any_element());
    }
    div().flex().flex_col().children(rows).into_any_element()
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
