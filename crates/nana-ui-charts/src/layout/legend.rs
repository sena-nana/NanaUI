//! The legend: one entry per series (pie: per slice), in rows that wrap,
//! taking its room off the edge it sits on.

use std::sync::Arc;

use super::builder::{fill_style, with_alpha};
use super::{ChartText, DrawKey, DrawPart, LABEL_GAP, LayoutInput, LegendItem, MarkBuilder};
use crate::marks::{GpuShape, ShapeKind};
use crate::option::{LegendPosition, Series};

const SWATCH: [f32; 2] = [14.0, 8.0];
const ITEM_GAP: f32 = nana_ui_core::space::LG;
const ROW_GAP: f32 = nana_ui_core::space::XS;

pub(super) fn layout(
    input: &LayoutInput<'_>,
    frame: &mut [f32; 4],
    builder: &mut MarkBuilder,
    texts: &mut Vec<ChartText>,
) -> Vec<LegendItem> {
    let Some(legend) = &input.option.legend else {
        return Vec::new();
    };
    if !legend.show {
        return Vec::new();
    }
    let theme = input.theme;
    // Entries: series names, except pies, whose slices are the entries.
    let mut entries: Vec<(Arc<str>, [f32; 4])> = Vec::new();
    for (index, series) in input.option.series.iter().enumerate() {
        match series {
            Series::Pie(pie) => {
                for (item_index, item) in pie.data.iter().enumerate() {
                    let color = item
                        .color
                        .map_or_else(|| theme.series_color(item_index), |c| theme.resolve(c));
                    entries.push((item.name.clone(), color));
                }
            }
            Series::Radar(radar) => {
                for (item_index, item) in radar.data.iter().enumerate() {
                    let color = item.color.map_or_else(
                        || theme.series_color(index + item_index),
                        |c| theme.resolve(c),
                    );
                    entries.push((item.name.clone(), color));
                }
            }
            Series::Gauge(_) => {}
            series => {
                let color = series
                    .explicit_color()
                    .map_or_else(|| theme.series_color(index), |c| theme.resolve(c));
                entries.push((series.name().clone(), color));
            }
        }
    }
    entries.dedup_by(|a, b| a.0 == b.0);
    if entries.is_empty() {
        return Vec::new();
    }
    let size = theme.font_size;
    let measure = input.measure;
    let sized: Vec<(Arc<str>, [f32; 4], [f32; 2])> = entries
        .into_iter()
        .map(|(name, color)| {
            let text = measure.measure(&name, size);
            (name, color, text)
        })
        .collect();
    let line = sized.iter().map(|(_, _, s)| s[1]).fold(SWATCH[1], f32::max);
    let vertical = matches!(
        legend.position,
        LegendPosition::Left | LegendPosition::Right
    );
    let width = frame[2] - frame[0];
    // Rows of entries that fit the width (one entry per row when vertical).
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut row_width = 0.0;
    let entry_width = |text: [f32; 2]| SWATCH[0] + nana_ui_core::space::XS + text[0];
    for (index, (_, _, text)) in sized.iter().enumerate() {
        let w = entry_width(*text);
        let row = rows.last_mut().unwrap();
        if !row.is_empty() && (vertical || row_width + ITEM_GAP + w > width) {
            rows.push(vec![index]);
            row_width = w;
        } else {
            row_width += if row.is_empty() { w } else { ITEM_GAP + w };
            row.push(index);
        }
    }
    let block_height = rows.len() as f32 * line + (rows.len() - 1) as f32 * ROW_GAP;
    let block_width = if vertical {
        sized
            .iter()
            .map(|(_, _, t)| entry_width(*t))
            .fold(0.0, f32::max)
    } else {
        width
    };
    let origin_y = match legend.position {
        LegendPosition::Bottom => frame[3] - block_height,
        LegendPosition::Top => frame[1],
        _ => (frame[1] + frame[3] - block_height) * 0.5,
    };
    let origin_x = match legend.position {
        LegendPosition::Left => frame[0],
        LegendPosition::Right => frame[2] - block_width,
        _ => frame[0],
    };
    match legend.position {
        LegendPosition::Top => frame[1] += block_height + LABEL_GAP * 2.0,
        LegendPosition::Bottom => frame[3] -= block_height + LABEL_GAP * 2.0,
        LegendPosition::Left => frame[0] += block_width + LABEL_GAP * 2.0,
        LegendPosition::Right => frame[2] -= block_width + LABEL_GAP * 2.0,
    }
    let mut items = Vec::new();
    let key = DrawKey {
        series: u32::MAX,
        part: DrawPart::Guide,
        run: 1,
    };
    for (row_index, row) in rows.iter().enumerate() {
        let row_width: f32 = row.iter().map(|i| entry_width(sized[*i].2)).sum::<f32>()
            + ITEM_GAP * (row.len() as f32 - 1.0);
        let mut x = if vertical {
            origin_x
        } else {
            origin_x + (width - row_width) * 0.5
        };
        let y = origin_y + row_index as f32 * (line + ROW_GAP);
        for index in row {
            let (name, color, text) = &sized[*index];
            let selected = !input.state.is_hidden(name);
            let swatch_color = if selected {
                *color
            } else {
                with_alpha(theme.muted, 0.5)
            };
            let style = builder.style(fill_style(swatch_color, swatch_color));
            let mut swatch = GpuShape::new(ShapeKind::Rect, 0, style, u32::MAX, 0);
            let sy = y + (line - SWATCH[1]) * 0.5;
            swatch.to = [x, sy, x + SWATCH[0], sy + SWATCH[1]];
            swatch.from = swatch.to;
            swatch.extra = [SWATCH[1] * 0.5; 4];
            builder.shape(swatch, key, 0);
            let tx = x + SWATCH[0] + nana_ui_core::space::XS;
            texts.push(ChartText {
                rect: [
                    tx,
                    y + (line - text[1]) * 0.5,
                    tx + text[0] + 1.0,
                    y + (line + text[1]) * 0.5,
                ],
                text: name.clone(),
                color: if selected { theme.text } else { theme.muted },
                size,
                weight: None,
            });
            let w = entry_width(*text);
            items.push(LegendItem {
                name: name.clone(),
                rect: [x, y, x + w, y + line],
                selected,
            });
            x += w + ITEM_GAP;
        }
    }
    builder.close_shapes();
    items
}
