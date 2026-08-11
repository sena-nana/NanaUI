// L1 chart paint exception: path-leaf helpers + deferred canvas heatmap.
// Prefer [`crate::svg_icon`] (resvg) for structural <svg> charts. Canvas path-d
// below is a legacy fallback when no SVG chart root is present — do not extend.
// See docs/css-layout-engine-boundary.md (L2 / heatmap single-track).

fn looks_like_svg_path(s: &str) -> bool {
    let t = s.trim();
    (t.starts_with('M') || t.starts_with('m'))
        && t.chars().any(|c| c.is_ascii_digit())
        && t.contains(|c: char| c == ' ' || c == ',' || c == '.')
}

/// Path `d` for an SVG path-shaped leaf (`value` or `label`).
///
/// Paint may come from CSS `fill` → [`LayoutStyle::background`]; unresolved
/// `var(--…)` still yields a path leaf (default fill in the canvas path).
fn svg_path_d(props: &WidgetProps) -> Option<&str> {
    for candidate in [props.value.as_str(), props.label.as_str()] {
        if !candidate.is_empty() && looks_like_svg_path(candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Filled path leaf: has path `d` and an explicit fill/background (or color).
fn filled_svg_path_d(props: &WidgetProps) -> Option<&str> {
    let d = svg_path_d(props)?;
    if props.layout.background.is_some() || props.layout.color.is_some() {
        Some(d)
    } else {
        // Still a path leaf — callers that need paint use [`svg_path_fill_color`].
        Some(d)
    }
}

fn svg_path_fill_color(props: &WidgetProps) -> Color {
    // SVG `fill` maps to `layout.background`. Do **not** fall back to
    // `layout.color` (inherited text color) — that paints near-black cells.
    if let Some(c) = props.layout.background {
        return rgba_color(c);
    }
    for raw in [&props.inline_style, &props.prop_style] {
        for decl in raw.split(';') {
            let decl = decl.trim();
            let Some((name, val)) = decl.split_once(':') else {
                continue;
            };
            if !name.trim().eq_ignore_ascii_case("fill")
                && !name.trim().eq_ignore_ascii_case("background")
                && !name.trim().eq_ignore_ascii_case("background-color")
            {
                continue;
            }
            if let Some(c) = crate::css_map::resolve_paint_color(val.trim()) {
                return rgba_color(c);
            }
        }
    }
    // Unresolved fill → light empty-cell gray (not text color).
    Color::from_rgb(0.937, 0.949, 0.961)
}

/// DEFER — legacy canvas heatmap from SVG path-d aggregates.
///
/// Preferred track is [`crate::svg_icon::try_svg_chart_element`] (resvg). Keep
/// this path only while orphan path leaves still lack an `<svg>` chart root.
/// Do not extend the parser; sink to L3 `CalendarHeatmap` / SvgChart.
fn heatmap_level_canvas<'a, Message: 'a>(widget: &SemanticWidget) -> Element<'a, Message> {
    let d = svg_path_d(&widget.props).unwrap_or(widget.props.value.as_str());
    heatmap_level_canvas_from(
        d,
        widget.props.layout.background.or(widget.props.layout.color),
        widget.props.layout.width,
        widget.props.layout.height,
    )
}

fn heatmap_level_canvas_owned<Message: 'static>(props: &WidgetProps) -> Element<'static, Message> {
    let d = svg_path_d(props).unwrap_or(props.value.as_str());
    heatmap_level_canvas_from(
        d,
        props.layout.background.or(props.layout.color),
        props.layout.width,
        props.layout.height,
    )
}

fn heatmap_level_canvas_from<'a, Message: 'a>(
    path_d: &str,
    background: Option<[f32; 4]>,
    width: Option<LengthSpec>,
    height: Option<LengthSpec>,
) -> Element<'a, Message> {
    let color = rgba_color(background.unwrap_or([0.55, 0.72, 0.92, 1.0]));
    let cells = parse_heatmap_cells(path_d)
        .into_iter()
        .map(|c| HeatmapCell { color, ..c })
        .collect();
    heatmap_canvas_from_cells(cells, width, height)
}

/// DEFER — composite path-d canvas when children are filled SVG path leaves.
/// Prefer structural SVG chart serialization; do not grow this bypass.
fn try_composite_filled_svg_paths<'a, Message: 'a>(
    snap: &SemanticSnapshot,
    widget: &SemanticWidget,
) -> Option<Element<'a, Message>> {
    try_composite_filled_svg_paths_from(snap, &widget.children, &widget.props)
}

fn try_composite_filled_svg_paths_owned<'a, Message: 'a>(
    snap: &SemanticSnapshot,
    children: &[WidgetId],
    props: &WidgetProps,
) -> Option<Element<'a, Message>> {
    try_composite_filled_svg_paths_from(snap, children, props)
}

fn try_composite_filled_svg_paths_from<'a, Message: 'a>(
    snap: &SemanticSnapshot,
    children: &[WidgetId],
    props: &WidgetProps,
) -> Option<Element<'a, Message>> {
    if children.is_empty() {
        return None;
    }
    let mut cells = Vec::new();
    let mut path_leaves = 0usize;
    for &id in children {
        let Some(w) = snap.get(id) else {
            continue;
        };
        if w.props.layout.hidden {
            continue;
        }
        if let Some(d) = svg_path_d(&w.props) {
            path_leaves += 1;
            let color = svg_path_fill_color(&w.props);
            cells.extend(
                parse_heatmap_cells(d)
                    .into_iter()
                    .map(|c| HeatmapCell { color, ..c }),
            );
        }
    }
    if path_leaves == 0 || cells.is_empty() {
        return None;
    }
    // SVG / sized surface with path children: one canvas (text siblings are
    // measure-only in iced — path cells previously collapsed to empty Columns).
    let svgish = props.element_tag.eq_ignore_ascii_case("svg")
        || props.element_tag.eq_ignore_ascii_case("g")
        || (props.layout.width.is_some() && props.layout.height.is_some());
    if !svgish && path_leaves * 2 < children.len() {
        return None;
    }
    Some(heatmap_canvas_from_cells(
        cells,
        props.layout.width,
        props.layout.height,
    ))
}

fn heatmap_canvas_from_cells<'a, Message: 'a>(
    cells: Vec<HeatmapCell>,
    width: Option<LengthSpec>,
    height: Option<LengthSpec>,
) -> Element<'a, Message> {
    let (max_x, max_y) = cells.iter().fold((0.0f32, 0.0f32), |acc, c| {
        (acc.0.max(c.x + c.size), acc.1.max(c.y + c.size))
    });
    let w = match width {
        Some(LengthSpec::Px(px)) => Length::Fixed(px),
        _ if max_x > 0.0 => Length::Fixed(max_x + 2.0),
        _ => Length::Fill,
    };
    let h = match height {
        Some(LengthSpec::Px(px)) => Length::Fixed(px),
        _ if max_y > 0.0 => Length::Fixed(max_y + 2.0),
        _ => Length::Fixed(112.0),
    };
    Canvas::new(HeatmapCells { cells })
        .width(w)
        .height(h)
        .into()
}

#[derive(Debug, Clone)]
struct HeatmapCell {
    x: f32,
    y: f32,
    size: f32,
    radius: f32,
    color: Color,
}

#[derive(Debug, Clone)]
struct HeatmapCells {
    cells: Vec<HeatmapCell>,
}

impl<Message> canvas::Program<Message> for HeatmapCells {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        for cell in &self.cells {
            let path = CanvasPath::rounded_rectangle(
                Point::new(cell.x, cell.y),
                Size::new(cell.size, cell.size),
                iced::border::Radius::from(cell.radius),
            );
            frame.fill(&path, cell.color);
        }
        vec![frame.into_geometry()]
    }
}

/// Parse contribution-heatmap rounded-rect path aggregates (`M x y … Z` per cell).
fn parse_heatmap_cells(d: &str) -> Vec<HeatmapCell> {
    let mut cells = Vec::new();
    let bytes = d.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'M' || bytes[i] == b'm' {
            i += 1;
            let (x, ni) = take_svg_number(d, i);
            i = ni;
            let (y, ni) = take_svg_number(d, i);
            i = ni;
            let mut size = 10.0f32;
            let mut radius = 2.0f32;
            // Peek ahead for `H` to recover cell width ≈ size - 2*radius.
            let rest = &d[i..];
            if let Some(h_pos) = rest.find('H').or_else(|| rest.find('h')) {
                let (hx, _) = take_svg_number(d, i + h_pos + 1);
                let span = (hx - x).abs();
                if span > 1.0 {
                    // YM: M(e+r,y) H(e+size-r) ⇒ span = size - 2r; assume r≈2.
                    radius = 2.0;
                    size = span + 2.0 * radius;
                }
            }
            cells.push(HeatmapCell {
                x: (x - radius).max(0.0),
                y,
                size,
                radius,
                // Neutral fallback when CSS fill is unresolved (not a product accent).
                color: Color::from_rgb(0.75, 0.77, 0.80),
            });
        } else {
            i += 1;
        }
    }
    cells
}

fn take_svg_number(s: &str, start: usize) -> (f32, usize) {
    let bytes = s.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let from = i;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    let n = s.get(from..i).and_then(|t| t.parse().ok()).unwrap_or(0.0);
    (n, i)
}
