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
use nana_ui_core::{CompiledTheme, SemanticColorRole, SemanticPalette};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, CustomGpuPayload, CustomRenderNode,
    InteractionState, LayoutBox, LineCap, MutationQueue, NodeKind, NodeStyle, StableNodeId,
    TextHorizontalAlignment, TextVerticalAlignment, UiWorld, WidgetBatchColors, WidgetPaintCtx,
    WidgetPaintOp, WidgetPaintOutput, WidgetQuadBatch, WidgetStroke, WidgetText, WidgetVisual,
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
    appearance: nana_ui_core::ThemeAppearance,
    font_size: f32,
    font_weight: u16,
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

    /// The layout at `size` under the theme's colors and label typography. The
    /// first layout of a spec starts its motion at `now`; a later one (a
    /// resize, a theme change) settles at once.
    pub(crate) fn layout_for(
        &self,
        size: [f32; 2],
        compiled_theme: &CompiledTheme,
        style_model: nana_ui_core::StyleModelRef,
        measure: crate::text_width::ChromeTextMeasure<'_>,
        now: Duration,
    ) -> Arc<ChartLayout> {
        let palette = style_model.palette;
        let appearance = style_model.theme_appearance;
        let typography = compiled_theme.typography();
        let measure_identity = measure.engine_identity();
        let mut memo = self
            .memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(memo) = memo.as_ref()
            && memo.size == size
            && memo.palette == palette
            && memo.appearance == appearance
            && memo.font_size == typography.meta
            && memo.font_weight == typography.regular
            && memo.measure == measure_identity
        {
            return Arc::clone(&memo.layout);
        }
        let theme = ChartTheme::new(&palette, &self.option, |role| {
            style_model.color(role).as_rgba_array()
        })
        .with_typography(typography);
        let measure = ChartLabelMeasure(measure, theme.font_weight);
        let mut fresh = layout(&LayoutInput {
            option: &self.option,
            size,
            theme: &theme,
            measure: &measure,
            state: &self.view,
        });
        for text in &mut fresh.texts {
            text.weight.get_or_insert(theme.font_weight);
        }
        if memo.is_none() && self.animate {
            transition::begin(&mut fresh, self.previous.as_deref(), &self.option, now);
        } else {
            transition::settle(&mut fresh);
        }
        let fresh = Arc::new(fresh);
        *memo = Some(ChartMemo {
            size,
            palette,
            appearance,
            font_size: theme.font_size,
            font_weight: theme.font_weight,
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
        layout.width.get_or_insert(crate::LengthSpec::Fill);
        layout
            .height
            .get_or_insert(crate::LengthSpec::Px(Self::INTRINSIC_HEIGHT));
        style
    }

    fn spec(&self, world: &UiWorld, id: StableNodeId) -> Arc<ChartSpec> {
        let current = world
            .widget_visual_as::<ChartVisual>(id)
            .map(|visual| Arc::clone(&visual.spec));
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
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::CHART.type_id);

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
        let visual = ChartVisual {
            spec: self.spec(world, id),
            hover: self.hover,
        };
        if world.widget_visual_as::<ChartVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
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

/// Chart axes measure at the same regular weight their text regions paint.
struct ChartLabelMeasure<'a>(crate::text_width::ChromeTextMeasure<'a>, u16);

impl LabelMeasure for ChartLabelMeasure<'_> {
    fn measure(&self, text: &str, size: f32) -> [f32; 2] {
        [self.0.width(text, size, Some(self.1)), (size * 1.35).ceil()]
    }
}

/// A chart's tooltip: a floating panel with a title and one row per item,
/// each with its color, name and value.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartTooltip {
    pub content: Arc<TooltipContent>,
    pub overlay_mount: nana_ui_core::OverlayMount,
    pub style: NodeStyle,
}

impl ChartTooltip {
    pub const PADDING_X: f32 = nana_ui_core::space::LG;
    pub const PADDING_Y: f32 = nana_ui_core::space::MD;
    pub const ROW_GAP: f32 = nana_ui_core::space::XS;
    pub const DOT: f32 = nana_ui_core::space::MD;
    pub const COLUMN_GAP: f32 = nana_ui_core::space::XXL;
    pub const FONT: f32 = nana_ui_core::TypographyTokens::DEFAULT.meta;

    pub(crate) fn new(content: Arc<TooltipContent>) -> Self {
        Self {
            content,
            overlay_mount: nana_ui_core::OverlayMount::NearestModal,
            style: tooltip_style(),
        }
    }

    /// Set where the floating chart tooltip mounts visually in the scene tree
    /// (defaults to [`nana_ui_core::OverlayMount::NearestModal`]).
    pub fn overlay_mount(mut self, mount: nana_ui_core::OverlayMount) -> Self {
        self.overlay_mount = mount;
        self
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
        if world.overlay_mount(id) != Some(self.overlay_mount) {
            mutations.set_overlay_mount(id, Some(self.overlay_mount));
        }
        let visual = ChartTooltipVisual {
            content: Arc::clone(&self.content),
        };
        if world.widget_visual_as::<ChartTooltipVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
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

/// Visual state attached to a [`Chart`] node on the open [`WidgetVisual`] path.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartVisual {
    pub spec: Arc<ChartSpec>,
    pub hover: ChartHoverVisual,
}

impl WidgetVisual for ChartVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::CHART
            .union(nana_ui_core::ThemeDependencyMask::COLOR_ANY)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
    }

    fn measures_text(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        let mut output = WidgetPaintOutput {
            suppress_root_surface: true,
            ..WidgetPaintOutput::new()
        };
        let origin = [ctx.bounds.x, ctx.bounds.y];
        let at = |p: [f32; 2]| [origin[0] + p[0], origin[1] + p[1]];
        let layout = self.spec.layout_for(
            [ctx.bounds.width, ctx.bounds.height],
            ctx.theme,
            ctx.style_model,
            ctx.world.chrome_text_measure(ctx.id),
            ctx.world.animation_now(),
        );
        let [line_color, band_color] = layout.pointer_colors;
        if let Some(nana_ui_charts::PointerGeometry::Band(band)) = self.hover.pointer {
            let [x0, y0] = at([band[0], band[1]]);
            let [x1, y1] = at([band[2], band[3]]);
            output.push(
                10,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: ctx.bounds,
                    rects: Arc::from([LayoutBox {
                        x: x0,
                        y: y0,
                        width: x1 - x0,
                        height: y1 - y0,
                    }]),
                    colors: WidgetBatchColors::Uniform(band_color),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            );
        }
        if !layout.marks.is_empty() {
            let mut node = CustomRenderNode::new(crate::CHART_RENDERER, "", 0);
            node.payload = Some(CustomGpuPayload::new(nana_ui_charts::ChartGpuPayload {
                origin,
                marks: layout.marks.clone(),
                hover: nana_ui_charts::ChartGpuHover {
                    current: self.hover.current,
                    previous: self.hover.previous,
                    focus: self.hover.focus,
                    since: self.hover.since,
                    duration: nana_ui_charts::hit::HOVER_DURATION,
                    growth: nana_ui_charts::hit::EMPHASIS_GROWTH,
                },
            }));
            let until = layout
                .marks
                .transition
                .map_or(Duration::ZERO, |motion| motion.end())
                .max(self.hover.live_until());
            node.transition_until = (until > Duration::ZERO).then_some(until);
            node.ink_bounds = layout.marks.extent.map(|[x0, y0, x1, y1]| {
                let reach = nana_ui_charts::hit::EMPHASIS_GROWTH + 2.0;
                nana_ui_core::LogicalRect {
                    x: origin[0] + x0 - reach,
                    y: origin[1] + y0 - reach,
                    width: (x1 - x0) + reach * 2.0,
                    height: (y1 - y0) + reach * 2.0,
                }
            });
            output.push(
                11,
                WidgetPaintOp::CustomGpu {
                    bounds: ctx.bounds,
                    node,
                },
            );
        }
        match self.hover.pointer {
            Some(nana_ui_charts::PointerGeometry::Line { from, to }) => {
                output.push(
                    12,
                    WidgetPaintOp::Stroke(WidgetStroke {
                        points: vec![at(from), at(to)],
                        width: ctx.theme.border().hairline,
                        color: line_color,
                        cap: LineCap::Butt,
                        dash: Some(vec![4.0, 3.0]),
                        snap_hairline: true,
                    }),
                );
            }
            Some(nana_ui_charts::PointerGeometry::Cross { center, plot }) => {
                let c = at(center);
                let (x0, y0) = (origin[0] + plot[0], origin[1] + plot[1]);
                let (x1, y1) = (origin[0] + plot[2], origin[1] + plot[3]);
                let hairline = ctx.theme.border().hairline;
                output.push(
                    12,
                    WidgetPaintOp::Stroke(WidgetStroke {
                        points: vec![[c[0], y0], [c[0], y1]],
                        width: hairline,
                        color: line_color,
                        cap: LineCap::Butt,
                        dash: Some(vec![4.0, 3.0]),
                        snap_hairline: true,
                    }),
                );
                output.push(
                    13,
                    WidgetPaintOp::Stroke(WidgetStroke {
                        points: vec![[x0, c[1]], [x1, c[1]]],
                        width: hairline,
                        color: line_color,
                        cap: LineCap::Butt,
                        dash: Some(vec![4.0, 3.0]),
                        snap_hairline: true,
                    }),
                );
            }
            _ => {}
        }
        for (index, text) in layout.texts.iter().enumerate() {
            output.push(
                (41_u64 << 32) | (index as u64),
                WidgetPaintOp::Text(WidgetText {
                    bounds: LayoutBox {
                        x: origin[0] + text.rect[0],
                        y: origin[1] + text.rect[1],
                        width: (text.rect[2] - text.rect[0]).max(0.0),
                        height: (text.rect[3] - text.rect[1]).max(0.0),
                    },
                    content: text.text.clone().into(),
                    color: Some(text.color),
                    paint_color: ctx.matching_text_paint(text.color),
                    font_size: text.size,
                    font_weight: text.weight,
                    horizontal_alignment: TextHorizontalAlignment::Start,
                    vertical_alignment: TextVerticalAlignment::Center,
                    ellipsis: false,
                    wrap: false,
                    max_lines: None,
                    italic: None,
                    underline: None,
                    line_through: None,
                    suppress_spans: false,
                    preserve_lines: None,
                    unstyled_glyph: false,
                    line_height: None,
                    retained_text: crate::WidgetRetainedText::None,
                    tabular_nums: false,
                    ..Default::default()
                }),
            );
        }
        output
    }
}

/// Visual state attached to a [`ChartTooltip`] node on the open [`WidgetVisual`] path.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartTooltipVisual {
    pub content: Arc<TooltipContent>,
}

impl WidgetVisual for ChartTooltipVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::CHART
            .union(nana_ui_core::ThemeDependencyMask::PALETTE)
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
    }

    fn measures_text(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        let mut output = WidgetPaintOutput::new();
        let measure = ChromeLabelMeasure(ctx.world.chrome_text_measure(ctx.id));
        let palette = ctx.style_model.palette;
        let typography = ctx.theme.typography();
        let font_size = typography.meta;
        let line = font_size + ctx.theme.spacing().xs;
        let left = ctx.bounds.x + ChartTooltip::PADDING_X;
        let right = ctx.bounds.x + ctx.bounds.width - ChartTooltip::PADDING_X;
        let mut y = ctx.bounds.y + ChartTooltip::PADDING_Y;
        let mut dot_rects = Vec::new();
        let mut dot_colors = Vec::new();
        let mut text_index = 0usize;
        let mut push_text =
            |x: f32, y: f32, width: f32, text: &Arc<str>, color: [f32; 4], weight| {
                output.push(
                    (42_u64 << 32) | (text_index as u64),
                    WidgetPaintOp::Text(WidgetText {
                        bounds: LayoutBox {
                            x,
                            y,
                            width: width.max(0.0),
                            height: line,
                        },
                        content: text.clone().into(),
                        color: Some(color),
                        paint_color: ctx.matching_text_paint(color),
                        font_size,
                        font_weight: weight,
                        horizontal_alignment: TextHorizontalAlignment::Start,
                        vertical_alignment: TextVerticalAlignment::Center,
                        ellipsis: true,
                        wrap: false,
                        max_lines: None,
                        italic: None,
                        underline: None,
                        line_through: None,
                        suppress_spans: false,
                        preserve_lines: None,
                        unstyled_glyph: false,
                        line_height: None,
                        retained_text: crate::WidgetRetainedText::None,
                        tabular_nums: false,
                        ..Default::default()
                    }),
                );
                text_index += 1;
            };
        if !self.content.title.is_empty() {
            push_text(
                left,
                y,
                right - left,
                &self.content.title,
                palette.text.as_rgba_array(),
                Some(typography.medium),
            );
            y += line + ChartTooltip::ROW_GAP;
        }
        for row in &self.content.rows {
            let mut x = left;
            if let Some(color) = row.color {
                let size = ChartTooltip::DOT;
                dot_rects.push(LayoutBox {
                    x,
                    y: y + (line - size) * 0.5,
                    width: size,
                    height: size,
                });
                dot_colors.push(color);
                x += size + ctx.theme.spacing().sm;
            }
            let value_width = measure.measure(&row.value, font_size)[0];
            push_text(
                x,
                y,
                right - x - value_width,
                &row.name,
                palette.muted.as_rgba_array(),
                None,
            );
            if !row.value.is_empty() {
                push_text(
                    right - value_width,
                    y,
                    value_width + 1.0,
                    &row.value,
                    palette.text.as_rgba_array(),
                    Some(typography.medium),
                );
            }
            y += line + ChartTooltip::ROW_GAP;
        }
        if !dot_rects.is_empty() {
            output.push(
                10,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: ctx.bounds,
                    rects: Arc::from(dot_rects),
                    colors: WidgetBatchColors::PerQuad(Arc::from(dot_colors)),
                    fill_paint: None,
                    border: None,
                    corner_radius: [ChartTooltip::DOT * 0.5; 4],
                }),
            );
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport, LengthSpec};
    use nana_ui_charts::{Axis, ChartGpuPayload, ChartOption, LineSeries};

    #[test]
    fn chart_extracts_custom_gpu_payload_and_labels() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(300.0));
        layout.height = Some(LengthSpec::Px(200.0));
        let option = ChartOption::new()
            .x_axis(Axis::category(["Mon", "Tue", "Wed"]))
            .y_axis(Axis::value())
            .series(LineSeries::new("v", [1.0, 3.0, 2.0]));
        let chart = context
            .create_component(document, Chart::new(option).style(style))
            .unwrap();
        context
            .layout_document(document, LayoutViewport::new(400.0, 300.0))
            .unwrap();

        let extracted = context.world().extract_nodes(&[chart.stable_id()]);
        let paint = extracted[0]
            .widget_paint
            .as_deref()
            .expect("chart widget_paint");
        assert!(paint.suppress_root_surface);
        let custom: Vec<_> = paint
            .ops
            .iter()
            .filter_map(|op| match &op.op {
                WidgetPaintOp::CustomGpu { node, .. } => Some((op.slot, node)),
                _ => None,
            })
            .collect();
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].0, 11);
        assert_eq!(custom[0].1.renderer.as_ref(), crate::CHART_RENDERER);
        let payload = custom[0]
            .1
            .payload
            .as_ref()
            .and_then(|p| p.downcast_ref::<ChartGpuPayload>())
            .expect("chart GPU payload");
        assert_eq!(payload.origin, [0.0, 0.0]);
        assert!(!payload.marks.is_empty());
        assert!(custom[0].1.transition_until.is_some());

        let text_slots: Vec<_> = paint
            .ops
            .iter()
            .filter_map(|op| match op.op {
                WidgetPaintOp::Text(_) => Some(op.slot),
                _ => None,
            })
            .collect();
        assert!(text_slots.len() > 2);
        assert!(text_slots.iter().all(|slot| *slot >= (41_u64 << 32)));
    }
}
