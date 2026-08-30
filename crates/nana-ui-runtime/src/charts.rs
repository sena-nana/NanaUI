//! Compact non-interactive time series. Application owns values and labels.

use std::sync::Arc;

use nana_ui_core::{SemanticColor, ThemeMode};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, LayoutBox, LengthSpec,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, StandardVisual, UiWorld,
};

const DEFAULT_LABEL: &str = "Time series";

/// Backend-neutral time-series geometry. Scene paint of the grid/area/line is not here.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeSeriesChart {
    pub values: Vec<f64>,
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSeriesPaint {
    pub grid: SemanticColor,
    pub area: SemanticColor,
    pub line: SemanticColor,
}

impl TimeSeriesChart {
    pub const INTRINSIC_HEIGHT: f32 = 148.0;
    pub const INSET_X: f32 = 8.0;
    pub const INSET_Y: f32 = 10.0;
    pub const GRID_LINE_COUNT: usize = 4;
    pub const LINE_WIDTH: f32 = 2.0;

    pub fn new(values: impl IntoIterator<Item = f64>) -> Self {
        Self {
            values: values.into_iter().map(sanitize_value).collect(),
            label: None,
            style: NodeStyle::default(),
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        let label = label.into();
        self.label = Some(if label.is_empty() {
            Arc::from(DEFAULT_LABEL)
        } else {
            label
        });
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Local points using inset (`INSET_X=8`, `INSET_Y=10`).
    pub fn points(&self, bounds: LayoutBox) -> Vec<(f32, f32)> {
        let values: Vec<f64> = self.values.iter().copied().map(sanitize_value).collect();
        if values.is_empty() {
            return Vec::new();
        }
        let width = (bounds.width - Self::INSET_X * 2.0).max(1.0);
        let height = (bounds.height - Self::INSET_Y * 2.0).max(1.0);
        let maximum = values.iter().copied().fold(0.0_f64, f64::max).max(1.0);
        let denominator = values.len().saturating_sub(1).max(1) as f32;
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let x = Self::INSET_X + width * index as f32 / denominator;
                let normalized = (*value / maximum).clamp(0.0, 1.0) as f32;
                (x, Self::INSET_Y + height * (1.0 - normalized))
            })
            .collect()
    }

    /// Four horizontal grid-line Y coordinates (`0..=3`).
    pub fn grid_ys(bounds: LayoutBox) -> [f32; 4] {
        let span = (bounds.height - Self::INSET_Y * 2.0).max(1.0);
        core::array::from_fn(|division| Self::INSET_Y + span * division as f32 / 3.0)
    }

    fn resolved_label(&self) -> Arc<str> {
        self.label
            .clone()
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| Arc::from(DEFAULT_LABEL))
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(Self::INTRINSIC_HEIGHT));
        layout.min_height = Some(LengthSpec::Px(Self::INTRINSIC_HEIGHT));
        style
    }
}

/// Sparkline colors: grid `border_soft` at 0.55, area accent at 0.16, line `accent_strong`.
pub fn time_series_paint(mode: ThemeMode) -> TimeSeriesPaint {
    let palette = mode.palette();
    TimeSeriesPaint {
        grid: SemanticColor {
            a: 0.55,
            ..palette.border_soft
        },
        area: SemanticColor {
            a: 0.16,
            ..palette.accent
        },
        line: palette.accent_strong,
    }
}

fn sanitize_value(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn inert() -> InteractionState {
    InteractionState {
        pointer_events: false,
        focusable: false,
    }
}

impl ComponentView for TimeSeriesChart {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "time-series-chart".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::TimeSeriesChart {
            values: self.values.clone().into(),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            inert(),
            AccessibilityState {
                role: AccessibilityRole::Image,
                label: Some(self.resolved_label()),
                ..AccessibilityState::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::AppContext;
    use crate::{DocumentId, NodeKind, StandardVisual};

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

    #[test]
    fn empty_series_has_no_points() {
        let chart = TimeSeriesChart::new([]);
        assert!(chart.values.is_empty());
        assert!(chart.points(bounds(108.0, 120.0)).is_empty());
        assert_eq!(TimeSeriesChart::grid_ys(bounds(108.0, 120.0)).len(), 4);
    }

    #[test]
    fn single_value_sits_on_the_left_inset() {
        let chart = TimeSeriesChart::new([10.0]);
        let points = chart.points(bounds(108.0, 120.0));
        assert_eq!(points.len(), 1);
        assert_eq!(points[0], (8.0, 10.0));
    }

    #[test]
    fn multiple_values_span_and_scale_to_the_largest() {
        let chart = TimeSeriesChart::new([0.0, 5.0, 10.0]);
        let points = chart.points(bounds(108.0, 120.0));
        assert_eq!(points.len(), 3);
        assert_eq!(points[0], (8.0, 110.0));
        assert_eq!(points[1], (54.0, 60.0));
        assert_eq!(points[2], (100.0, 10.0));
    }

    #[test]
    fn non_finite_and_negative_values_become_zero() {
        let chart = TimeSeriesChart::new([f64::NAN, -2.0, 4.0, f64::INFINITY]);
        assert_eq!(chart.values, vec![0.0, 0.0, 4.0, 0.0]);
        let points = chart.points(bounds(108.0, 120.0));
        assert_eq!(points.len(), 4);
        assert_eq!(points[2].1, 10.0);
        assert!(points[0].1 > points[2].1);
        assert_eq!(points[0].1, points[1].1);
        assert_eq!(points[0].1, points[3].1);
    }

    #[test]
    fn higher_values_have_smaller_y() {
        let chart = TimeSeriesChart::new([1.0, 3.0, 2.0]);
        let points = chart.points(bounds(108.0, 120.0));
        assert_eq!(points.len(), 3);
        assert!(points[1].1 < points[2].1);
        assert!(points[2].1 < points[0].1);
    }

    #[test]
    fn chart_projects_a_fill_width_inert_leaf() {
        let mut context = AppContext::new();
        let chart = context
            .create_component(document(), TimeSeriesChart::new([1.0, 2.0, 3.0]))
            .unwrap();
        let id = chart.stable_id();
        assert!(matches!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element { tag } if tag == "time-series-chart"
        ));
        assert!(matches!(
            context.world().standard_visual(id),
            Some(StandardVisual::TimeSeriesChart { .. })
        ));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.layout.width, Some(LengthSpec::Fill));
        assert_eq!(
            style.layout.height,
            Some(LengthSpec::Px(TimeSeriesChart::INTRINSIC_HEIGHT))
        );
        assert_eq!(context.world().interaction(id), Some(inert()));
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::Image);
        assert_eq!(accessibility.label.as_deref(), Some(DEFAULT_LABEL));
    }

    #[test]
    fn chart_commits_time_series_standard_visual() {
        let mut context = AppContext::new();
        let view = TimeSeriesChart::new([1.0, 2.0, 3.0]);
        let expected = StandardVisual::TimeSeriesChart {
            values: view.values.clone().into(),
        };
        let chart = context.create_component(document(), view).unwrap();
        assert_eq!(
            context.world().standard_visual(chart.stable_id()),
            Some(expected)
        );
    }
}
