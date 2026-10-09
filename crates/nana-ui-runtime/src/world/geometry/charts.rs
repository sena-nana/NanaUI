//! Chart geometry: the layout of a chart's option at its box, made once per
//! size and theme, and its tooltip's rows.

use super::*;
use crate::charts::{ChartTooltip, ChromeLabelMeasure};
use nana_ui_charts::LabelMeasure as _;
use nana_ui_charts::hit::TooltipContent;

impl UiWorld {
    pub(in crate::world) fn chart_geometry(
        &self,
        id: StableNodeId,
        bounds: LayoutBox,
        spec: &crate::ChartSpec,
        hover: &crate::ChartHoverVisual,
    ) -> crate::ComponentGeometry {
        let style_model = self.style_model;
        let measure = self.chrome_text_measure(id);
        let layout = spec.layout_for(
            [bounds.width, bounds.height],
            &style_model.palette,
            |role| style_model.color(role).as_rgba_array(),
            &ChromeLabelMeasure(measure),
            measure.engine_identity(),
            self.animation_now(),
        );
        crate::ComponentGeometry::Chart {
            layout,
            hover: *hover,
            origin: [bounds.x, bounds.y],
        }
    }

    pub(in crate::world) fn chart_tooltip_geometry(
        &self,
        id: StableNodeId,
        bounds: LayoutBox,
        content: &TooltipContent,
    ) -> crate::ComponentGeometry {
        let measure = ChromeLabelMeasure(self.chrome_text_measure(id));
        let palette = self.style_model.palette;
        let line = ChartTooltip::line_height();
        let left = bounds.x + ChartTooltip::PADDING_X;
        let right = bounds.x + bounds.width - ChartTooltip::PADDING_X;
        let mut y = bounds.y + ChartTooltip::PADDING_Y;
        let mut dots = Vec::new();
        let mut texts = Vec::new();
        let region = |x: f32, y: f32, width: f32, text: &Arc<str>, color, weight| {
            crate::ComponentTextRegion {
                bounds: LayoutBox {
                    x,
                    y,
                    width: width.max(0.0),
                    height: line,
                },
                content: text.clone().into(),
                color: Some(color),
                font_size: ChartTooltip::FONT,
                font_weight: weight,
            }
        };
        if !content.title.is_empty() {
            texts.push(region(
                left,
                y,
                right - left,
                &content.title,
                palette.text.as_rgba_array(),
                Some(nana_ui_core::type_scale::MEDIUM),
            ));
            y += line + ChartTooltip::ROW_GAP;
        }
        for row in &content.rows {
            let mut x = left;
            if let Some(color) = row.color {
                let size = ChartTooltip::DOT;
                dots.push((
                    LayoutBox {
                        x,
                        y: y + (line - size) * 0.5,
                        width: size,
                        height: size,
                    },
                    color,
                ));
                x += size + nana_ui_core::space::SM;
            }
            let value_width = measure.measure(&row.value, ChartTooltip::FONT)[0];
            texts.push(region(
                x,
                y,
                right - x - value_width,
                &row.name,
                palette.muted.as_rgba_array(),
                None,
            ));
            if !row.value.is_empty() {
                texts.push(region(
                    right - value_width,
                    y,
                    value_width + 1.0,
                    &row.value,
                    palette.text.as_rgba_array(),
                    Some(nana_ui_core::type_scale::MEDIUM),
                ));
            }
            y += line + ChartTooltip::ROW_GAP;
        }
        crate::ComponentGeometry::ChartTooltip { dots, texts }
    }
}
