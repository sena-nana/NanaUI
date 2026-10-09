//! Charts: an ECharts-shaped [`ChartOption`] the application owns, laid out
//! by `nana-ui-charts` and drawn by the chart shaders.
//!
//! The component owns interaction, which is presentation and never touches
//! the option: hover (emphasis, axis pointer, tooltip), legend selection
//! and the zoom window. A layout depends on the option, the view state, the
//! box size and the theme; hover is answered from the finished layout and
//! drawn from a uniform, so a moving pointer never lays the chart out.

use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use nana_ui_charts::hit::HoverState as ChartHoverVisual;
use nana_ui_charts::hit::TooltipContent;
use nana_ui_charts::{
    ChartLayout, ChartOption, ChartTheme, ChartViewState, LabelMeasure, LayoutInput, layout,
    transition,
};
use nana_ui_core::{SemanticColorRole, SemanticPalette};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, LengthSpec,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, StandardVisual, UiWorld,
};

/// One option and view, laid out once per box size and theme.
///
/// A new spec is made whenever the option or the view state changes; it
/// keeps the layout that was on screen before it, so its first layout can
/// move from there.
pub struct ChartSpec {
    pub option: Arc<ChartOption>,
    pub view: ChartViewState,
    animate: bool,
    previous: Option<Arc<ChartLayout>>,
    memo: Mutex<Option<ChartMemo>>,
}

struct ChartMemo {
    size: [f32; 2],
    palette: SemanticPalette,
    /// The text engine that measured the labels.
    measure: usize,
    layout: Arc<ChartLayout>,
}

impl std::fmt::Debug for ChartSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartSpec")
            .field("series", &self.option.series.len())
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

impl PartialEq for ChartSpec {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl ChartSpec {
    fn new(
        option: Arc<ChartOption>,
        view: ChartViewState,
        previous: Option<Arc<ChartLayout>>,
        animate: bool,
    ) -> Self {
        Self {
            option,
            view,
            animate,
            previous,
            memo: Mutex::new(None),
        }
    }

    /// The layout last made of this spec.
    pub fn shown(&self) -> Option<Arc<ChartLayout>> {
        self.memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|memo| Arc::clone(&memo.layout))
    }

    /// The layout at `size` under `palette`, made at most once per pair. The
    /// first layout of a spec starts its motion at `now`; a later one (a
    /// resize, a theme change) settles at once.
    pub(crate) fn layout_for(
        &self,
        size: [f32; 2],
        palette: &SemanticPalette,
        resolve: impl Fn(SemanticColorRole) -> [f32; 4],
        measure: &dyn LabelMeasure,
        measure_identity: usize,
        now: Duration,
    ) -> Arc<ChartLayout> {
        let mut memo = self
            .memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(memo) = memo.as_ref()
            && memo.size == size
            && memo.palette == *palette
            && memo.measure == measure_identity
        {
            return Arc::clone(&memo.layout);
        }
        let theme = ChartTheme::new(palette, &self.option, resolve);
        let mut fresh = layout(&LayoutInput {
            option: &self.option,
            size,
            theme: &theme,
            measure,
            state: &self.view,
        });
        if memo.is_none() && self.animate {
            transition::begin(&mut fresh, self.previous.as_deref(), &self.option, now);
        } else {
            transition::settle(&mut fresh);
        }
        let fresh = Arc::new(fresh);
        *memo = Some(ChartMemo {
            size,
            palette: *palette,
            measure: measure_identity,
            layout: Arc::clone(&fresh),
        });
        fresh
    }
}

/// What a chart tells its application.
#[derive(Debug, Clone, PartialEq)]
pub enum ChartEvent {
    /// A press and release on an item, without a drag between.
    Click { series: usize, index: usize },
    /// The legend switched a series (pie: a slice) on or off.
    LegendSelect { name: Arc<str>, selected: bool },
    /// The zoom window of `option.data_zoom[index]` moved, in percent.
    DataZoom { index: usize, start: f64, end: f64 },
}

/// What a press on the chart started.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ChartDrag {
    /// A press that is a click unless it moves.
    Press { x: f32, y: f32, hover: [u32; 2] },
    /// Moving the zoom window `zoom` (or one of its ends) from `window`,
    /// by `per_px` percent for each px the pointer moves from `x`: panning
    /// inside the plot, or dragging the slider.
    Zoom {
        zoom: usize,
        grab: ZoomGrab,
        x: f32,
        window: (f64, f64),
        per_px: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ZoomGrab {
    Window,
    Start,
    End,
}

/// A chart. Applications give it an option and replace the option to
/// change it; the chart animates from what it showed.
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    pub option: Arc<ChartOption>,
    /// The accessible name. Charts have no visible title of their own.
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
    /// Legend selection and zoom, as the user left them.
    pub view: ChartViewState,
    pub(crate) hover: ChartHoverVisual,
    pub(crate) drag: Option<ChartDrag>,
}

impl Chart {
    /// The box height a chart takes when the application sets none.
    pub const INTRINSIC_HEIGHT: f32 = 300.0;

    pub fn new(option: impl Into<Arc<ChartOption>>) -> Self {
        Self {
            option: option.into(),
            label: None,
            style: NodeStyle::default(),
            view: ChartViewState::default(),
            hover: ChartHoverVisual::default(),
            drag: None,
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// What the pointer emphasises now.
    pub fn hover_state(&self) -> ChartHoverVisual {
        self.hover
    }

    /// Replaces the option; the chart moves from what it shows to it.
    pub fn set_option(&mut self, option: impl Into<Arc<ChartOption>>) {
        self.option = option.into();
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width.get_or_insert(LengthSpec::Fill);
        layout
            .height
            .get_or_insert(LengthSpec::Px(Self::INTRINSIC_HEIGHT));
        style
    }

    fn spec(&self, world: &UiWorld, id: StableNodeId) -> Arc<ChartSpec> {
        let current = match world.standard_visual(id) {
            Some(StandardVisual::Chart { spec, .. }) => Some(spec),
            _ => None,
        };
        match current {
            Some(spec) if Arc::ptr_eq(&spec.option, &self.option) && spec.view == self.view => spec,
            current => {
                // Moving the zoom window redraws at once; everything else
                // animates as the option says.
                let zoom_only = current.as_ref().is_some_and(|spec| {
                    Arc::ptr_eq(&spec.option, &self.option) && spec.view.hidden == self.view.hidden
                });
                Arc::new(ChartSpec::new(
                    Arc::clone(&self.option),
                    self.view.clone(),
                    current.and_then(|spec| spec.shown()),
                    !zoom_only,
                ))
            }
        }
    }
}

impl ComponentView for Chart {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        pointer: Some(&crate::framework::CHART_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "chart".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::Chart {
            spec: self.spec(world, id),
            hover: self.hover,
        };
        if world.standard_visual(id).as_ref() != Some(&visual) {
            mutations.set_standard_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Image,
                label: self.label.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

/// [`LabelMeasure`] over the world's text engine.
pub(crate) struct ChromeLabelMeasure<'a>(pub crate::text_width::ChromeTextMeasure<'a>);

impl LabelMeasure for ChromeLabelMeasure<'_> {
    fn measure(&self, text: &str, size: f32) -> [f32; 2] {
        [self.0.width(text, size, None), (size * 1.35).ceil()]
    }
}

/// A chart's tooltip: a floating panel with a title and one row per item,
/// each with its color, name and value.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartTooltip {
    pub content: Arc<TooltipContent>,
    pub style: NodeStyle,
}

impl ChartTooltip {
    pub const PADDING_X: f32 = nana_ui_core::space::LG;
    pub const PADDING_Y: f32 = nana_ui_core::space::MD;
    pub const ROW_GAP: f32 = nana_ui_core::space::XS;
    pub const DOT: f32 = nana_ui_core::space::MD;
    pub const COLUMN_GAP: f32 = nana_ui_core::space::XXL;
    pub const FONT: f32 = nana_ui_core::type_scale::META;

    pub(crate) fn new(content: Arc<TooltipContent>) -> Self {
        Self {
            content,
            style: tooltip_style(),
        }
    }

    pub(crate) fn line_height() -> f32 {
        (Self::FONT * 1.35).ceil()
    }

    /// `[width, height]` of the panel, padding included.
    pub(crate) fn measure(content: &TooltipContent, measure: &dyn LabelMeasure) -> [f32; 2] {
        let line = Self::line_height();
        let mut width = if content.title.is_empty() {
            0.0
        } else {
            measure.measure(&content.title, Self::FONT)[0]
        };
        for row in &content.rows {
            let name = measure.measure(&row.name, Self::FONT)[0];
            let value = measure.measure(&row.value, Self::FONT)[0];
            let dot = if row.color.is_some() {
                Self::DOT + nana_ui_core::space::SM
            } else {
                0.0
            };
            let gap = if row.value.is_empty() {
                0.0
            } else {
                Self::COLUMN_GAP
            };
            width = width.max(dot + name + gap + value);
        }
        let lines = content.rows.len() + usize::from(!content.title.is_empty());
        let height = lines as f32 * line + (lines.saturating_sub(1)) as f32 * Self::ROW_GAP;
        [
            (width + Self::PADDING_X * 2.0).ceil(),
            (height + Self::PADDING_Y * 2.0).ceil(),
        ]
    }
}

fn tooltip_style() -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            position: nana_ui_core::PositionSpec::Fixed,
            border_width: Some(nana_ui_core::HAIRLINE),
            border_radius: Some(nana_ui_core::TooltipConfig::RADIUS),
            pointer_events: Some(nana_ui_core::PointerEventsSpec::None),
            z_index: Some(1_000),
            ..nana_ui_core::LayoutStyle::default()
        }),
        background: Some(SemanticColorRole::Surface),
        border: Some(SemanticColorRole::BorderSoft),
        foreground: Some(SemanticColorRole::Text),
        ..NodeStyle::default()
    }
}

impl ComponentView for ChartTooltip {
    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "chart-tooltip".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::ChartTooltip {
            content: Arc::clone(&self.content),
        };
        if world.standard_visual(id).as_ref() != Some(&visual) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let label: Arc<str> = std::iter::once(self.content.title.to_string())
            .chain(
                self.content
                    .rows
                    .iter()
                    .map(|row| format!("{} {}", row.name, row.value)),
            )
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
            .into();
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState::default(),
            AccessibilityState {
                role: AccessibilityRole::Tooltip,
                label: Some(label),
                ..AccessibilityState::default()
            },
        );
    }
}
