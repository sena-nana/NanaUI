//! charts geometry from committed node data.

use super::*;

pub(in crate::world) fn time_series_geometry(
    bounds: LayoutBox,
    values: &[f64],
    mode: ThemeMode,
) -> crate::ComponentGeometry {
    let chart = crate::TimeSeriesChart::new(values.iter().copied());
    let paint = crate::time_series_paint(mode);
    let local = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: bounds.width,
        height: bounds.height,
    };
    let inset_x = crate::TimeSeriesChart::INSET_X;
    let grid = crate::TimeSeriesChart::grid_ys(local)
        .into_iter()
        .map(|y| LayoutBox {
            x: bounds.x + inset_x,
            y: bounds.y + y,
            width: (bounds.width - inset_x * 2.0).max(0.0),
            height: 1.0,
        })
        .collect();
    let points = chart
        .points(local)
        .into_iter()
        .map(|(x, y)| [bounds.x + x, bounds.y + y])
        .collect::<Vec<_>>();
    let baseline = bounds.y
        + (bounds.height - crate::TimeSeriesChart::INSET_Y).max(crate::TimeSeriesChart::INSET_Y);
    crate::ComponentGeometry::TimeSeriesChart {
        grid,
        area: area_under_polyline(&points, baseline),
        line: points,
        grid_color: paint.grid.as_rgba_array(),
        area_color: paint.area.as_rgba_array(),
        line_color: paint.line.as_rgba_array(),
    }
}
