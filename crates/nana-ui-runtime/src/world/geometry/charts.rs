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

pub(in crate::world) fn timestamp_series_geometry(
    bounds: LayoutBox,
    samples: &[(i64, Option<f64>)],
    unit: Option<&str>,
    time_labels: Option<&(Arc<str>, Arc<str>)>,
    mode: ThemeMode,
) -> crate::ComponentGeometry {
    let chart = crate::TimeSeriesChart::from_samples(samples.iter().copied());
    let paint = crate::time_series_paint(mode);
    // Reserve vertical space for the scale and endpoint labels.
    let plot = LayoutBox {
        x: bounds.x,
        y: bounds.y + 18.0,
        width: bounds.width,
        height: (bounds.height - 38.0).max(1.0),
    };
    let segments: Vec<Vec<[f32; 2]>> = chart
        .segments(plot)
        .into_iter()
        .map(|run| {
            run.into_iter()
                .map(|(x, y)| [plot.x + x, plot.y + y])
                .collect()
        })
        .collect();
    let baseline = plot.y
        + (plot.height - crate::TimeSeriesChart::INSET_Y).max(crate::TimeSeriesChart::INSET_Y);
    let area = segments
        .iter()
        .flat_map(|run| area_under_polyline(run, baseline))
        .collect();
    let grid = crate::TimeSeriesChart::grid_ys(plot)
        .into_iter()
        .map(|y| LayoutBox {
            x: plot.x + crate::TimeSeriesChart::INSET_X,
            y: plot.y + y,
            width: (plot.width - 2.0 * crate::TimeSeriesChart::INSET_X).max(0.0),
            height: 1.0,
        })
        .collect();
    let label = |text: Arc<str>, x: f32, y: f32, width: f32| crate::ComponentTextRegion {
        bounds: LayoutBox {
            x,
            y,
            width,
            height: 16.0,
        },
        content: text,
        color: Some(mode.palette().muted.as_rgba_array()),
        font_size: 11.0,
        font_weight: None,
    };
    let maximum = samples
        .iter()
        .filter_map(|sample| sample.1)
        .filter(|v| v.is_finite())
        .fold(1.0_f64, f64::max);
    let mut labels = vec![label(
        if samples
            .iter()
            .any(|sample| sample.1.is_some_and(f64::is_finite))
        {
            format!("0 – {maximum:.0} {}", unit.unwrap_or("")).into()
        } else {
            format!("— {}", unit.unwrap_or("")).into()
        },
        bounds.x + 8.0,
        bounds.y,
        (bounds.width - 16.0).max(0.0),
    )];
    if let Some((start, end)) = time_labels {
        let half = (bounds.width * 0.5 - 8.0).max(0.0);
        labels.push(label(
            start.clone(),
            bounds.x + 8.0,
            bounds.y + bounds.height - 16.0,
            half,
        ));
        labels.push(label(
            end.clone(),
            bounds.x + bounds.width * 0.5,
            bounds.y + bounds.height - 16.0,
            half,
        ));
    }
    crate::ComponentGeometry::TimestampSeriesChart {
        grid,
        area,
        segments,
        labels,
        grid_color: paint.grid.as_rgba_array(),
        area_color: paint.area.as_rgba_array(),
        line_color: paint.line.as_rgba_array(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_geometry_leaves_missing_interval_unfilled() {
        let geometry = timestamp_series_geometry(
            LayoutBox {
                x: 30.0,
                y: 20.0,
                width: 216.0,
                height: 148.0,
            },
            &[
                (0, Some(1.0)),
                (10, Some(2.0)),
                (20, None),
                (90, Some(2.0)),
                (100, Some(1.0)),
            ],
            Some("people"),
            Some(&(Arc::from("10:00"), Arc::from("11:00"))),
            ThemeMode::Dark,
        );
        let crate::ComponentGeometry::TimestampSeriesChart {
            area,
            segments,
            labels,
            ..
        } = geometry
        else {
            panic!("timestamp geometry");
        };
        assert_eq!(segments.len(), 2);
        let gap_start = segments[0].last().unwrap()[0];
        let gap_end = segments[1][0][0];
        assert!(gap_end > gap_start);
        assert!(!area.is_empty());
        assert!(
            area.iter()
                .all(|rect| rect.x + rect.width <= gap_start + 1.0 || rect.x >= gap_end)
        );
        assert_eq!(labels.len(), 3);
        assert!(
            labels.iter().all(
                |label| label.bounds.y >= 20.0 && label.bounds.y + label.bounds.height <= 168.0
            )
        );
    }
}
