//! The NanaUI chart model: an ECharts-shaped [`ChartOption`], laid out on
//! the CPU into text, hit-test data and [`ChartMarks`], the arrays the chart
//! shaders draw from.
//!
//! Nothing here touches the retained tree or the GPU. `nana-ui-runtime`
//! owns the `Chart` component and its interaction, `nana-ui-scene` carries
//! the marks, and `nana-ui` draws them.

pub mod hit;
pub mod layout;
pub mod marks;
pub mod option;
pub mod sample;
pub mod scale;
pub mod smooth;
pub mod stack;
pub mod theme;
pub mod transition;

pub use hit::{ChartHover, PointerGeometry, TooltipContent, TooltipRow};
pub use layout::{
    ApproximateMeasure, ChartLayout, ChartText, ChartViewState, LabelMeasure, LayoutInput,
    LegendItem, SliderLayout, TextAlign, layout,
};
pub use marks::*;
pub use option::*;
pub use theme::ChartTheme;
