//! Rebuild Lucide / SVG icon and chart subtrees into iced `svg` handles.
//!
//! ## L1 geometry → iced adapter（非 L2 Semantics）
//! Vue emits real `<svg>` geometry in attrs + child shape nodes (path, circle,
//! g, …). This module serializes that subtree to an SVG document and caches
//! [`iced::widget::svg::Handle`] so frames do not reallocate identical docs.
//!
//! **Layer note:** L1 paint exception — iced `svg` primitive, not `nana_ui::*`.
//! It is the **preferred** heatmap / pie chart track (resvg). Do **not** grow a
//! second path-d parser here or in `iced_app::l1_charts` (that canvas path is
//! DEFER-only). Prefer a future L3 SvgChart / CalendarHeatmap.
//!
//! Lucide icons are square + `currentColor` tinted. Chart roots (heatmap, pie,
//! …) keep author fills/strokes and use explicit width×height (viewBox-aware
//! via resvg inside iced).

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer as adv_renderer};
use iced::widget::space;
use iced::widget::svg::{self, Handle};
use iced::{Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::bridge::{SemanticSnapshot, SemanticWidget, WidgetId, WidgetKind, WidgetProps};
use crate::css_map::LengthSpec;

static HANDLE_CACHE: OnceLock<Mutex<HandleCache>> = OnceLock::new();

const CACHE_CAP: usize = 512;

/// Light empty-cell gray used when chart fill is unresolved (SVG + deferred
/// canvas heatmap fallback share this tone). Must not omit `fill` (SVG defaults
/// to black).
const EMPTY_CELL_FILL: &str = "#eff2f5";

/// FIFO handle cache — evict oldest entries one-by-one, never `clear()` the table.
///
/// Chart horizontal crops must **not** mint new SVG documents (that thrashed this
/// cache on resize). Base Lucide / full-chart docs stay reusable across frames.
struct HandleCache {
    map: HashMap<u64, Handle>,
    order: VecDeque<u64>,
}

impl HandleCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get_or_insert_with(&mut self, key: u64, make: impl FnOnce() -> Handle) -> Handle {
        if let Some(existing) = self.map.get(&key) {
            return existing.clone();
        }
        while self.map.len() >= CACHE_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            } else {
                break;
            }
        }
        let handle = make();
        self.map.insert(key, handle.clone());
        self.order.push_back(key);
        handle
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    fn clear_for_test(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

/// SVG tags that participate in icon/chart geometry (plus nested groups).
const GEOMETRY_TAGS: &[&str] = &[
    "path", "circle", "ellipse", "line", "polyline", "polygon", "rect", "g",
];

/// True when this widget looks like a Lucide (or plain SVG) **icon** root.
///
/// Chart SVGs (heatmap / pie) are geometry roots but not icons — they must not
/// take the square tinted Lucide paint path.
pub fn is_svg_icon_root(w: &SemanticWidget) -> bool {
    if is_chart_svg_root(w) {
        return false;
    }
    if w.props
        .class_names
        .iter()
        .any(|c| c == "lucide" || c.starts_with("lucide-"))
    {
        return true;
    }
    matches!(w.kind, WidgetKind::Icon)
        && (w.props.element_tag.eq_ignore_ascii_case("svg")
            || attr_lookup(&w.props, &["viewBox", "viewbox", "view-box"]).is_some())
}

/// Structural `<svg>` chart root (contribution heatmap, language pie, …).
///
/// Detected by tag + viewBox / explicit size — never by product class names.
pub fn is_chart_svg_root(w: &SemanticWidget) -> bool {
    if !w.props.element_tag.eq_ignore_ascii_case("svg") {
        return false;
    }
    if w.props
        .class_names
        .iter()
        .any(|c| c == "lucide" || c.starts_with("lucide-"))
    {
        return false;
    }
    if matches!(w.kind, WidgetKind::Icon) {
        return false;
    }
    attr_lookup(&w.props, &["viewBox", "viewbox", "view-box"]).is_some()
        || (matches!(w.props.layout.width, Some(LengthSpec::Px(w)) if w > 0.0)
            && matches!(w.props.layout.height, Some(LengthSpec::Px(h)) if h > 0.0))
}

/// Serialize a widget subtree into a complete SVG document, if geometry exists.
pub fn serialize_svg_document(snap: &SemanticSnapshot, id: WidgetId) -> Option<String> {
    let root = snap.get(id)?;
    let as_icon = is_svg_icon_root(root) || matches!(root.kind, WidgetKind::Icon);
    let as_chart = is_chart_svg_root(root);
    if !as_icon && !as_chart {
        return None;
    }
    // Heatmap empty cells need an explicit light fill (SVG default is black).
    // Lucide roots set fill="none"; child paths must inherit that — never invent
    // EMPTY_CELL_FILL, which would paint stroke glyphs solid gray.
    let empty_cell_fallback = as_chart;
    let mut body = String::new();
    let mut has_geometry = false;
    for &child in &root.children {
        if let Some(chunk) = serialize_svg_node(snap, child, empty_cell_fallback) {
            has_geometry = true;
            body.push_str(&chunk);
        }
    }
    // Leaf icon may carry path `d` on itself (rare).
    if !has_geometry {
        if let Some(d) = path_d_from_props(&root.props) {
            has_geometry = true;
            body.push_str(&format!(r#"<path d="{}"/>"#, xml_escape(&d)));
        }
    }
    if !has_geometry {
        return None;
    }

    let view_box =
        attr_lookup(&root.props, &["viewBox", "viewbox", "view-box"]).unwrap_or_else(|| {
            if as_chart {
                // Prefer layout box as a synthetic viewBox when author omitted it.
                match (root.props.layout.width, root.props.layout.height) {
                    (Some(LengthSpec::Px(w)), Some(LengthSpec::Px(h))) if w > 0.0 && h > 0.0 => {
                        format!("0 0 {w} {h}")
                    }
                    _ => "0 0 24 24".to_string(),
                }
            } else {
                "0 0 24 24".to_string()
            }
        });

    let mut doc = String::with_capacity(body.len() + 192);
    doc.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox=""#);
    doc.push_str(&xml_escape(&view_box));
    doc.push('"');
    if as_icon {
        let fill = attr_lookup(&root.props, &["fill"]).unwrap_or_else(|| "none".to_string());
        let stroke =
            attr_lookup(&root.props, &["stroke"]).unwrap_or_else(|| "currentColor".to_string());
        let stroke_width = attr_lookup(&root.props, &["stroke-width", "strokeWidth"])
            .unwrap_or_else(|| "2".to_string());
        let linecap = attr_lookup(&root.props, &["stroke-linecap", "strokeLinecap"])
            .unwrap_or_else(|| "round".to_string());
        let linejoin = attr_lookup(&root.props, &["stroke-linejoin", "strokeLinejoin"])
            .unwrap_or_else(|| "round".to_string());
        push_attr(&mut doc, "fill", &fill);
        push_attr(&mut doc, "stroke", &stroke);
        push_attr(&mut doc, "stroke-width", &stroke_width);
        push_attr(&mut doc, "stroke-linecap", &linecap);
        push_attr(&mut doc, "stroke-linejoin", &linejoin);
    }
    doc.push('>');
    doc.push_str(&body);
    doc.push_str("</svg>");
    Some(doc)
}

/// Cached [`Handle`] from SVG document bytes.
pub fn handle_from_svg(doc: &str) -> Handle {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    doc.hash(&mut hasher);
    let key = hasher.finish();
    let cache = HANDLE_CACHE.get_or_init(|| Mutex::new(HandleCache::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    guard.get_or_insert_with(key, || Handle::from_memory(doc.as_bytes().to_vec()))
}

/// Build an iced SVG element tinted with `color` at `size`×`size` (Lucide).
pub fn svg_icon_element<'a, Message: 'a>(
    handle: Handle,
    size: f32,
    color: Color,
) -> Element<'a, Message> {
    let size = size.max(1.0);
    svg::Svg::new(handle)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_theme, _status| svg::Style { color: Some(color) })
        .into()
}

/// Chart SVG: author paints, viewBox-aware fit (no currentColor tint).
///
/// Wide charts (heatmap) keep 1:1 CSS pixels when the layout box is tall enough.
/// When the allocated height is shorter than the intrinsic viewBox, scale
/// uniformly so every weekday row stays visible (no vertical crop), then crop
/// from the inline-end to match `overflow:hidden` + `flex-end`.
///
/// Paint must use [`ContentFit::Fill`] into the scaled Fixed box — iced's
/// [`ContentFit::None`] always draws at the intrinsic viewBox size (and the
/// overflow is clipped by the short track), which slices weekday rows instead
/// of shrinking them. Near-square charts (pie/donut) use
/// [`ContentFit::Contain`] so the full ring stays visible inside a shorter card.
pub fn svg_chart_element<'a, Message: 'a>(
    doc: String,
    intrinsic_w: f32,
    intrinsic_h: f32,
) -> Element<'a, Message> {
    let intrinsic_w = intrinsic_w.max(1.0);
    let intrinsic_h = intrinsic_h.max(1.0);
    let wide = intrinsic_w / intrinsic_h > 2.0;
    let base_handle = handle_from_svg(&doc);
    drop(doc);
    iced::widget::responsive(move |size| -> Element<'a, Message> {
        let avail_w = size.width.max(1.0);
        let avail_h = size.height.max(1.0);
        if wide {
            let (draw_w, draw_h, crop_w, crop_h) =
                wide_chart_paint_box(intrinsic_w, intrinsic_h, avail_w, avail_h);
            // Fill (not None): rasterize into draw_* so uniform scale is real
            // paint. None keeps the viewBox pixel size and lets the short
            // overflow track clip through the middle of the weekday grid.
            let chart = svg::Svg::new(base_handle.clone())
                .width(Length::Fixed(draw_w))
                .height(Length::Fixed(draw_h))
                .content_fit(iced::ContentFit::Fill);
            if (draw_w - crop_w) < 0.5 {
                chart.into()
            } else {
                EndCrop::new(chart.into(), crop_w, crop_h).into()
            }
        } else {
            svg::Svg::new(base_handle.clone())
                .width(Length::Fixed(avail_w.min(intrinsic_w)))
                .height(Length::Fixed(avail_h.min(intrinsic_h)))
                .content_fit(iced::ContentFit::Contain)
                .into()
        }
    })
    .width(Length::Fill)
    .height(if wide {
        // Prefer author height; iced clamps Fixed to the parent max so a short
        // overflow track still reports the real avail_h into the closure above.
        Length::Fixed(intrinsic_h)
    } else {
        Length::Fill
    })
    .into()
}

/// Horizontal crop width for wide charts — never shrinks height alone.
fn wide_chart_crop_width(intrinsic_w: f32, avail_w: f32) -> f32 {
    avail_w.max(1.0).min(intrinsic_w.max(1.0))
}

/// Paint box for a wide chart: `(draw_w, draw_h, crop_w, crop_h)`.
///
/// Height is never viewBox-cropped. If `avail_h` is shorter than intrinsic,
/// scale uniformly so weekday rows fit; width crop still prefers the inline-end.
fn wide_chart_paint_box(
    intrinsic_w: f32,
    intrinsic_h: f32,
    avail_w: f32,
    avail_h: f32,
) -> (f32, f32, f32, f32) {
    let intrinsic_w = intrinsic_w.max(1.0);
    let intrinsic_h = intrinsic_h.max(1.0);
    let avail_w = avail_w.max(1.0);
    let avail_h = avail_h.max(1.0);
    let scale = if avail_h + 0.5 < intrinsic_h {
        (avail_h / intrinsic_h).clamp(0.01, 1.0)
    } else {
        1.0
    };
    let draw_w = intrinsic_w * scale;
    let draw_h = intrinsic_h * scale;
    let crop_w = wide_chart_crop_width(draw_w, avail_w);
    // Crop viewport matches the scaled paint; horizontal EndCrop only.
    (draw_w, draw_h, crop_w, draw_h.min(avail_h))
}

/// Clip a wider child to `width`×`height`, aligning content to the inline-end
/// (recent weeks stay visible). Uses a renderer layer so overflow is actually
/// clipped — iced's stock `Svg` ignores the parent viewport clip bounds.
struct EndCrop<'a, Message> {
    width: f32,
    height: f32,
    content: Element<'a, Message>,
}

impl<'a, Message> EndCrop<'a, Message> {
    fn new(content: Element<'a, Message>, width: f32, height: f32) -> Self {
        Self {
            width: width.max(1.0),
            height: height.max(1.0),
            content,
        }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for EndCrop<'_, Message> {
    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fixed(self.width),
            height: Length::Fixed(self.height),
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.resolve(
            Length::Fixed(self.width),
            Length::Fixed(self.height),
            Size::new(self.width, self.height),
        );
        let child_limits = layout::Limits::new(Size::ZERO, Size::INFINITE);
        let mut child =
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &child_limits);
        let child_size = child.size();
        // flex-end: overflow hangs off the inline-start so the end stays in view.
        let x = size.width - child_size.width;
        child = child.move_to(Point::new(x, 0.0));
        layout::Node::with_children(size, vec![child])
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let Some(child_layout) = layout.children().next() else {
            return;
        };
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            child_layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &adv_renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;

        let bounds = layout.bounds();
        let Some(child_layout) = layout.children().next() else {
            return;
        };
        let Some(clipped) = bounds.intersection(viewport) else {
            return;
        };
        renderer.with_layer(clipped, |renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                child_layout,
                cursor,
                &clipped,
            );
        });
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        let Some(child_layout) = layout.children().next() else {
            return;
        };
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            child_layout,
            renderer,
            operation,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let Some(child_layout) = layout.children().next() else {
            return mouse::Interaction::None;
        };
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            child_layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let child_layout = layout.children().next()?;
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            child_layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<EndCrop<'a, Message>> for Element<'a, Message> {
    fn from(value: EndCrop<'a, Message>) -> Self {
        Element::new(value)
    }
}

/// Resolve pixel size from CSS width/height/font-size, else control icon size.
pub fn resolve_icon_size(props: &WidgetProps) -> f32 {
    let from_box = match (props.layout.width, props.layout.height) {
        (Some(LengthSpec::Px(w)), Some(LengthSpec::Px(h)))
            if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 =>
        {
            Some(w.min(h))
        }
        (Some(LengthSpec::Px(w)), _) if w.is_finite() && w > 0.0 => Some(w),
        (_, Some(LengthSpec::Px(h))) if h.is_finite() && h > 0.0 => Some(h),
        _ => None,
    };
    if let Some(v) = from_box {
        return v;
    }
    if let Some(fs) = props.layout.font_size.filter(|v| v.is_finite() && *v > 0.0) {
        return fs;
    }
    props.size.icon_size()
}

/// Width×height for a chart SVG (layout px, else viewBox, else 24²).
pub fn resolve_chart_size(props: &WidgetProps) -> (f32, f32) {
    let w = match props.layout.width {
        Some(LengthSpec::Px(px)) if px.is_finite() && px > 0.0 => Some(px),
        _ => None,
    };
    let h = match props.layout.height {
        Some(LengthSpec::Px(px)) if px.is_finite() && px > 0.0 => Some(px),
        _ => None,
    };
    if let (Some(w), Some(h)) = (w, h) {
        return (w, h);
    }
    if let Some((vb_w, vb_h)) = parse_view_box_size(props) {
        return (w.unwrap_or(vb_w), h.unwrap_or(vb_h));
    }
    (w.unwrap_or(24.0), h.unwrap_or(24.0))
}

/// Try to build a cached SVG handle for `id` (self or first Lucide descendant).
pub fn try_svg_handle(snap: &SemanticSnapshot, id: WidgetId) -> Option<Handle> {
    let doc = serialize_svg_document(snap, id).or_else(|| find_descendant_svg_doc(snap, id))?;
    Some(handle_from_svg(&doc))
}

/// Try to render a structural chart `<svg>` via iced/resvg.
pub fn try_svg_chart_element<'a, Message: 'a>(
    snap: &SemanticSnapshot,
    id: WidgetId,
) -> Option<Element<'a, Message>> {
    let root = snap.get(id)?;
    if !is_chart_svg_root(root) {
        return None;
    }
    let doc = serialize_svg_document(snap, id)?;
    if std::env::var_os("NANA_DUMP_CHART_SVG").is_some() {
        let tag = root
            .props
            .class_names
            .first()
            .map(String::as_str)
            .unwrap_or("chart");
        let path = std::path::PathBuf::from(format!("/tmp/nana-chart-{tag}-{id}.svg"));
        let _ = std::fs::write(&path, &doc);
        eprintln!(
            "[nana-svg-chart] id={id} class={:?} tag={} bytes={} -> {}",
            root.props.class_names,
            root.props.element_tag,
            doc.len(),
            path.display()
        );
    }
    let (w, h) = resolve_chart_size(&root.props);
    Some(svg_chart_element(doc, w, h))
}

/// Empty placeholder when an Icon has no drawable geometry and no shell glyph.
pub fn empty_icon_placeholder<'a, Message: 'a>(size: f32) -> Element<'a, Message> {
    let size = size.max(1.0);
    space()
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

fn parse_view_box_size(props: &WidgetProps) -> Option<(f32, f32)> {
    let vb = attr_lookup(props, &["viewBox", "viewbox", "view-box"])?;
    let parts: Vec<&str> = vb.split_whitespace().collect();
    if parts.len() != 4 {
        return None;
    }
    let w: f32 = parts[2].parse().ok()?;
    let h: f32 = parts[3].parse().ok()?;
    if w > 0.0 && h > 0.0 {
        Some((w, h))
    } else {
        None
    }
}

fn find_descendant_svg_doc(snap: &SemanticSnapshot, id: WidgetId) -> Option<String> {
    let w = snap.get(id)?;
    if is_svg_icon_root(w) {
        if let Some(doc) = serialize_svg_document(snap, id) {
            return Some(doc);
        }
    }
    for &child in &w.children {
        if let Some(doc) = find_descendant_svg_doc(snap, child) {
            return Some(doc);
        }
    }
    None
}

fn serialize_svg_node(
    snap: &SemanticSnapshot,
    id: WidgetId,
    empty_cell_fallback: bool,
) -> Option<String> {
    let w = snap.get(id)?;
    let tag = w.props.element_tag.to_ascii_lowercase();
    if tag.is_empty() {
        // Path boxes often keep geometry in value without a reliable tag after map.
        if let Some(d) = path_d_from_props(&w.props) {
            let mut out = String::from("<path");
            write_paint_attrs(&mut out, &w.props, true, empty_cell_fallback);
            push_attr(&mut out, "d", &d);
            out.push_str("/>");
            return Some(out);
        }
        return None;
    }
    if !GEOMETRY_TAGS.contains(&tag.as_str()) {
        // Nested lucide roots are unusual; skip non-geometry (text labels, title…).
        return None;
    }
    if tag == "g" {
        let mut inner = String::new();
        for &child in &w.children {
            if let Some(chunk) = serialize_svg_node(snap, child, empty_cell_fallback) {
                inner.push_str(&chunk);
            }
        }
        if inner.is_empty() {
            return None;
        }
        let mut out = String::from("<g");
        write_common_shape_attrs(&mut out, &w.props);
        out.push('>');
        out.push_str(&inner);
        out.push_str("</g>");
        return Some(out);
    }

    let mut out = format!("<{tag}");
    write_common_shape_attrs(&mut out, &w.props);
    // Prefer explicit attrs; fall back to cascade-resolved layout paints.
    let prefer_fill = tag != "circle" && tag != "ellipse" && tag != "line";
    write_paint_attrs(&mut out, &w.props, prefer_fill, empty_cell_fallback);
    match tag.as_str() {
        "path" => {
            let d = path_d_from_props(&w.props)?;
            push_attr(&mut out, "d", &d);
        }
        "circle" => {
            push_opt_attr(&mut out, &w.props, "cx", &["cx"]);
            push_opt_attr(&mut out, &w.props, "cy", &["cy"]);
            push_opt_attr(&mut out, &w.props, "r", &["r"]);
            push_path_length(&mut out, &w.props);
            push_dash_attrs(&mut out, &w.props);
        }
        "ellipse" => {
            push_opt_attr(&mut out, &w.props, "cx", &["cx"]);
            push_opt_attr(&mut out, &w.props, "cy", &["cy"]);
            push_opt_attr(&mut out, &w.props, "rx", &["rx"]);
            push_opt_attr(&mut out, &w.props, "ry", &["ry"]);
            push_path_length(&mut out, &w.props);
            push_dash_attrs(&mut out, &w.props);
        }
        "line" => {
            push_opt_attr(&mut out, &w.props, "x1", &["x1"]);
            push_opt_attr(&mut out, &w.props, "y1", &["y1"]);
            push_opt_attr(&mut out, &w.props, "x2", &["x2"]);
            push_opt_attr(&mut out, &w.props, "y2", &["y2"]);
            push_dash_attrs(&mut out, &w.props);
        }
        "polyline" | "polygon" => {
            push_opt_attr(&mut out, &w.props, "points", &["points"]);
            push_dash_attrs(&mut out, &w.props);
        }
        "rect" => {
            push_opt_attr(&mut out, &w.props, "x", &["x"]);
            push_opt_attr(&mut out, &w.props, "y", &["y"]);
            push_opt_attr(&mut out, &w.props, "width", &["width"]);
            push_opt_attr(&mut out, &w.props, "height", &["height"]);
            push_opt_attr(&mut out, &w.props, "rx", &["rx"]);
            push_opt_attr(&mut out, &w.props, "ry", &["ry"]);
        }
        _ => {}
    }
    out.push_str("/>");
    Some(out)
}

fn write_common_shape_attrs(out: &mut String, props: &WidgetProps) {
    for (svg_name, keys) in [
        ("opacity", &["opacity"][..]),
        ("transform", &["transform"][..]),
        ("stroke-linecap", &["stroke-linecap", "strokeLinecap"][..]),
        (
            "stroke-linejoin",
            &["stroke-linejoin", "strokeLinejoin"][..],
        ),
    ] {
        if let Some(v) = attr_lookup(props, keys) {
            push_attr(out, svg_name, &v);
        }
    }
}

/// Emit fill / stroke / stroke-width from attrs or cascade-resolved layout.
///
/// `empty_cell_fallback` is chart-only: unresolved filled shapes get the heatmap
/// empty-cell gray. Icon trees omit fill so children inherit root `fill="none"`.
fn write_paint_attrs(
    out: &mut String,
    props: &WidgetProps,
    default_filled: bool,
    empty_cell_fallback: bool,
) {
    let fill_attr = attr_lookup(props, &["fill"]);
    let fill_none = fill_is_none(props);
    if let Some(ref fill) = fill_attr {
        if !fill.is_empty() {
            push_attr(out, "fill", fill);
        }
    } else if fill_none {
        push_attr(out, "fill", "none");
    } else if let Some(c) = props.layout.background {
        push_attr(out, "fill", &rgba_to_svg_color(c));
    } else if !default_filled {
        // Stroke-based shapes (circle rings): default fill none.
        push_attr(out, "fill", "none");
    } else if empty_cell_fallback {
        // Chart cells without a resolved paint must not omit `fill` — SVG
        // defaults to black. Match canvas empty-cell gray.
        push_attr(out, "fill", EMPTY_CELL_FILL);
    }
    // else: leave fill unset so Lucide children inherit root fill="none".

    if let Some(stroke) = attr_lookup(props, &["stroke"]) {
        if !stroke.is_empty() {
            push_attr(out, "stroke", &stroke);
        }
    } else if let Some(c) = props.layout.border_color {
        // Only emit when stroke was intentionally mapped (non-zero width or fill none).
        if props.layout.border_width.unwrap_or(0.0) > 0.0 || fill_none || !default_filled {
            push_attr(out, "stroke", &rgba_to_svg_color(c));
        }
    }

    if let Some(sw) = attr_lookup(props, &["stroke-width", "strokeWidth"]) {
        if !sw.is_empty() {
            push_attr(out, "stroke-width", &sw);
        }
    } else if let Some(w) = props.layout.border_width.filter(|v| *v > 0.0) {
        // Avoid inventing stroke-width for filled heatmap paths that only have
        // a CSS border from unrelated rules.
        if fill_none
            || !default_filled
            || attr_lookup(props, &["stroke"]).is_some()
            || props.layout.border_color.is_some()
        {
            push_attr(out, "stroke-width", &format!("{w}"));
        }
    }
}

fn fill_is_none(props: &WidgetProps) -> bool {
    if let Some(fill) = attr_lookup(props, &["fill"]) {
        return fill.eq_ignore_ascii_case("none") || fill.eq_ignore_ascii_case("transparent");
    }
    for raw in [&props.inline_style, &props.prop_style] {
        for decl in raw.split(';') {
            let decl = decl.trim();
            let Some((name, val)) = decl.split_once(':') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("fill") {
                let v = val.trim();
                if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("transparent") {
                    return true;
                }
            }
        }
    }
    false
}

fn push_path_length(out: &mut String, props: &WidgetProps) {
    if let Some(v) = attr_lookup(props, &["pathLength", "pathlength", "path-length"]) {
        push_attr(out, "pathLength", &v);
    }
}

fn push_dash_attrs(out: &mut String, props: &WidgetProps) {
    if let Some(v) = attr_lookup(props, &["stroke-dasharray", "strokeDasharray"]) {
        push_attr(out, "stroke-dasharray", &v);
    } else if looks_like_dasharray(props.value.as_str()) {
        // Legacy bridge stored dasharray in `value`.
        push_attr(out, "stroke-dasharray", props.value.trim());
    }
    if let Some(v) = attr_lookup(props, &["stroke-dashoffset", "strokeDashoffset"]) {
        push_attr(out, "stroke-dashoffset", &v);
    } else if looks_like_dashoffset(props.hint.as_str()) {
        push_attr(out, "stroke-dashoffset", props.hint.trim());
    }
}

fn looks_like_dasharray(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || looks_like_svg_path(t) {
        return false;
    }
    // "68.2 31.8" / "68,32"
    let mut saw_digit = false;
    for ch in t.chars() {
        match ch {
            '0'..='9' | '.' | '+' | '-' | ' ' | ',' => {
                if ch.is_ascii_digit() {
                    saw_digit = true;
                }
            }
            _ => return false,
        }
    }
    saw_digit
}

fn looks_like_dashoffset(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    t.parse::<f32>().is_ok()
}

fn path_d_from_props(props: &WidgetProps) -> Option<String> {
    if let Some(d) = attr_lookup(props, &["d"]) {
        if !d.is_empty() {
            return Some(d);
        }
    }
    let v = props.value.trim();
    if looks_like_svg_path(v) {
        return Some(v.to_string());
    }
    None
}

fn looks_like_svg_path(s: &str) -> bool {
    let t = s.trim();
    (t.starts_with('M') || t.starts_with('m'))
        && t.chars().any(|c| c.is_ascii_digit())
        && t.contains(|c: char| c == ' ' || c == ',' || c == '.')
}

fn attr_lookup(props: &WidgetProps, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = props.attrs.get(*key) {
            if !v.is_empty() {
                return Some(v.clone());
            }
        }
        // Normalized keys are often lowercased / kebab.
        let lower = key.to_ascii_lowercase();
        if let Some(v) = props.attrs.get(&lower) {
            if !v.is_empty() {
                return Some(v.clone());
            }
        }
    }
    None
}

fn push_opt_attr(out: &mut String, props: &WidgetProps, svg_name: &str, keys: &[&str]) {
    if let Some(v) = attr_lookup(props, keys) {
        push_attr(out, svg_name, &v);
    }
}

fn push_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    out.push_str(&xml_escape(value));
    out.push('"');
}

fn rgba_to_svg_color(c: [f32; 4]) -> String {
    let r = (c[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (c[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (c[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    let a = c[3].clamp(0.0, 1.0);
    if a >= 0.999 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("rgba({r},{g},{b},{a:.4})")
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{MessageBridge, WidgetKind, WidgetProps};
    use nana_js_engine::HostValue;

    #[test]
    fn serializes_lucide_search_from_attrs_and_children() {
        let mut bridge = MessageBridge::new();
        let mut root = WidgetProps::default();
        root.element_tag = "svg".into();
        root.class_names = vec!["lucide".into(), "lucide-search".into()];
        root.value = "search".into();
        bridge.register(1, WidgetKind::Icon, root);
        bridge.patch_prop(1, "viewBox", &HostValue::string("0 0 24 24"));
        bridge.patch_prop(1, "fill", &HostValue::string("none"));
        bridge.patch_prop(1, "stroke", &HostValue::string("currentColor"));
        bridge.patch_prop(1, "stroke-width", &HostValue::string("2"));

        let mut circle = WidgetProps::default();
        circle.element_tag = "circle".into();
        bridge.register(2, WidgetKind::Box, circle);
        bridge.patch_prop(2, "cx", &HostValue::string("11"));
        bridge.patch_prop(2, "cy", &HostValue::string("11"));
        bridge.patch_prop(2, "r", &HostValue::string("8"));
        bridge.insert_child(2, 1, None);

        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        bridge.register(3, WidgetKind::Box, path);
        bridge.patch_prop(3, "d", &HostValue::string("m21 21-4.3-4.3"));
        bridge.insert_child(3, 1, None);

        let snap = bridge.snapshot();
        let doc = serialize_svg_document(&snap, 1).expect("svg doc");
        assert!(doc.contains(r#"viewBox="0 0 24 24""#), "{doc}");
        assert!(doc.contains(r#"fill="none""#), "{doc}");
        assert!(
            !doc.contains(&format!(r#"fill="{EMPTY_CELL_FILL}""#)),
            "Lucide children must inherit fill=none, not heatmap empty-cell gray: {doc}"
        );
        assert!(doc.contains(r#"stroke="currentColor""#), "{doc}");
        assert!(
            doc.contains(r#"<circle"#) && doc.contains(r#"cx="11""#),
            "{doc}"
        );
        assert!(doc.contains(r#"d="m21 21-4.3-4.3""#), "{doc}");
        // Path must not invent a fill — only root carries fill="none".
        let path_start = doc.find("<path").expect("path");
        let path_end = doc[path_start..]
            .find("/>")
            .map(|i| path_start + i)
            .expect("path end");
        let path_tag = &doc[path_start..path_end];
        assert!(
            !path_tag.contains("fill="),
            "child path must omit fill to inherit root none: {path_tag}"
        );
        // Root value stays glyph name — path must come from child attrs.
        assert_eq!(snap.get(1).unwrap().props.value, "search");
        assert!(snap.get(3).unwrap().props.attrs.get("d").is_some());

        let handle = handle_from_svg(&doc);
        let again = handle_from_svg(&doc);
        assert_eq!(handle.id(), again.id());
    }

    #[test]
    fn lucide_root_d_stays_in_attrs_not_value() {
        let mut bridge = MessageBridge::new();
        let mut root = WidgetProps::default();
        root.class_names = vec!["lucide".into(), "lucide-x".into()];
        root.value = "x".into();
        bridge.register(1, WidgetKind::Icon, root);
        bridge.patch_prop(1, "d", &HostValue::string("M18 6 6 18"));
        let w = bridge.get(1).unwrap();
        assert_eq!(w.props.value, "x");
        assert_eq!(
            w.props.attrs.get("d").map(String::as_str),
            Some("M18 6 6 18")
        );
    }

    #[test]
    fn serializes_language_pie_circle_dasharray_ring() {
        let mut bridge = MessageBridge::new();
        let mut root = WidgetProps::default();
        root.element_tag = "svg".into();
        root.layout.width = Some(LengthSpec::Px(132.0));
        root.layout.height = Some(LengthSpec::Px(132.0));
        bridge.register(1, WidgetKind::Column, root);
        bridge.patch_prop(1, "viewBox", &HostValue::string("0 0 132 132"));

        let mut g = WidgetProps::default();
        g.element_tag = "g".into();
        bridge.register(2, WidgetKind::Column, g);
        bridge.patch_prop(2, "transform", &HostValue::string("rotate(-90 66 66)"));
        bridge.insert_child(2, 1, None);

        let mut track = WidgetProps::default();
        track.element_tag = "circle".into();
        track.layout.border_color = Some([0.2, 0.2, 0.25, 1.0]);
        track.layout.border_width = Some(28.0);
        track.inline_style = "fill:none;stroke-width:28".into();
        bridge.register(3, WidgetKind::Box, track);
        bridge.patch_prop(3, "cx", &HostValue::string("66"));
        bridge.patch_prop(3, "cy", &HostValue::string("66"));
        bridge.patch_prop(3, "r", &HostValue::string("50"));
        bridge.patch_prop(3, "pathLength", &HostValue::string("100"));
        bridge.insert_child(3, 2, None);

        let mut slice = WidgetProps::default();
        slice.element_tag = "circle".into();
        slice.inline_style = "fill:none;stroke-width:28".into();
        bridge.register(4, WidgetKind::Box, slice);
        bridge.patch_prop(4, "cx", &HostValue::string("66"));
        bridge.patch_prop(4, "cy", &HostValue::string("66"));
        bridge.patch_prop(4, "r", &HostValue::string("50"));
        bridge.patch_prop(4, "pathLength", &HostValue::string("100"));
        bridge.patch_prop(4, "stroke", &HostValue::string("#4c8bf5"));
        bridge.patch_prop(4, "stroke-dasharray", &HostValue::string("68 32"));
        bridge.patch_prop(4, "stroke-dashoffset", &HostValue::string("0"));
        bridge.insert_child(4, 2, None);

        let snap = bridge.snapshot();
        assert!(is_chart_svg_root(snap.get(1).unwrap()));
        assert!(!is_svg_icon_root(snap.get(1).unwrap()));
        let doc = serialize_svg_document(&snap, 1).expect("pie svg");
        assert!(doc.contains(r#"viewBox="0 0 132 132""#), "{doc}");
        assert!(doc.contains(r#"transform="rotate(-90 66 66)""#), "{doc}");
        assert!(doc.contains(r#"pathLength="100""#), "{doc}");
        assert!(doc.contains(r#"stroke-dasharray="68 32""#), "{doc}");
        assert!(doc.contains("stroke=\"#4c8bf5\""), "{doc}");
        assert!(doc.contains("fill=\"none\""), "{doc}");
        assert!(doc.contains("stroke-width"), "{doc}");
        let (w, h) = resolve_chart_size(&snap.get(1).unwrap().props);
        assert!((w - 132.0).abs() < f32::EPSILON && (h - 132.0).abs() < f32::EPSILON);
    }

    #[test]
    fn serializes_heatmap_filled_paths_with_resolved_fill() {
        let mut bridge = MessageBridge::new();
        let mut root = WidgetProps::default();
        root.element_tag = "svg".into();
        root.layout.width = Some(LengthSpec::Px(200.0));
        root.layout.height = Some(LengthSpec::Px(100.0));
        bridge.register(1, WidgetKind::Column, root);
        bridge.patch_prop(1, "viewBox", &HostValue::string("0 0 200 100"));

        // Rounded-rect cell path (Lilia calendarHeatmap style).
        let d = "M45 16H52Q55 16 55 19V26Q55 29 52 29H45Q42 29 42 26V19Q42 16 45 16Z";
        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        path.layout.background = Some([0.3, 0.75, 0.4, 1.0]);
        bridge.register(2, WidgetKind::Box, path);
        bridge.patch_prop(2, "d", &HostValue::string(d));
        bridge.insert_child(2, 1, None);

        let snap = bridge.snapshot();
        let doc = serialize_svg_document(&snap, 1).expect("heatmap svg");
        assert!(doc.contains(r#"viewBox="0 0 200 100""#), "{doc}");
        assert!(doc.contains(&format!(r#"d="{d}""#)), "{doc}");
        assert!(doc.contains("fill=\"#"), "{doc}");
        assert!(is_chart_svg_root(snap.get(1).unwrap()));
    }

    #[test]
    fn unresolved_path_fill_uses_empty_cell_gray_not_default_black() {
        let mut bridge = MessageBridge::new();
        let mut root = WidgetProps::default();
        root.element_tag = "svg".into();
        root.layout.width = Some(LengthSpec::Px(40.0));
        root.layout.height = Some(LengthSpec::Px(20.0));
        bridge.register(1, WidgetKind::Column, root);
        bridge.patch_prop(1, "viewBox", &HostValue::string("0 0 40 20"));

        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        // No fill attr, no layout.background — previously omitted fill → SVG black.
        bridge.register(2, WidgetKind::Box, path);
        bridge.patch_prop(
            2,
            "d",
            &HostValue::string("M2 2H10Q12 2 12 4V12Q12 14 10 14H2Q0 14 0 12V4Q0 2 2 2Z"),
        );
        bridge.insert_child(2, 1, None);

        let snap = bridge.snapshot();
        let doc = serialize_svg_document(&snap, 1).expect("svg");
        assert!(
            doc.contains(&format!(r#"fill="{EMPTY_CELL_FILL}""#)),
            "expected empty-cell fill, got {doc}"
        );
        assert!(!doc.contains("fill=\"#000\""), "{doc}");
    }

    #[test]
    fn wide_chart_crop_preserves_full_intrinsic_height() {
        // Short card body must not drop weekday rows — scale uniformly, then
        // horizontal EndCrop only. Paint uses ContentFit::Fill into draw_* so
        // the scale is not undone by iced blitting the full viewBox.
        let intrinsic_w = 905.0;
        let intrinsic_h = 125.0;
        let avail_w = 352.0;
        let avail_h = 87.0;
        assert!(avail_h < intrinsic_h);
        assert!(
            (intrinsic_w / intrinsic_h) > 2.0,
            "fixture must stay on the wide-chart path"
        );
        let (draw_w, draw_h, crop_w, crop_h) =
            wide_chart_paint_box(intrinsic_w, intrinsic_h, avail_w, avail_h);
        assert!(
            (draw_h - avail_h).abs() < 0.5,
            "draw_h={draw_h} should fit avail_h={avail_h}"
        );
        assert!(draw_w < intrinsic_w);
        assert!((crop_h - draw_h).abs() < f32::EPSILON);
        assert!(
            (crop_w - avail_w).abs() < 0.5,
            "wide charts crop width to avail, got crop_w={crop_w}"
        );
        assert!(crop_w < draw_w);
        // Scaled draw must stay inside the short track — never taller than avail.
        assert!(
            draw_h <= avail_h + 0.01,
            "draw_h={draw_h} must not exceed avail_h={avail_h}"
        );

        // Tall enough track: 1:1 pixels, height stays intrinsic.
        let (_dw2, _dh2, crop_w2, crop_h2) =
            wide_chart_paint_box(intrinsic_w, intrinsic_h, avail_w, intrinsic_h + 10.0);
        assert!((crop_h2 - intrinsic_h).abs() < f32::EPSILON);
        assert!((crop_w2 - avail_w).abs() < 0.5);
    }

    #[test]
    fn handle_cache_evicts_fifo_without_clearing_all() {
        let cache = HANDLE_CACHE.get_or_init(|| Mutex::new(HandleCache::new()));
        {
            let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
            guard.clear_for_test();
        }

        let keepers: Vec<_> = (0..8)
            .map(|i| {
                let doc = format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M{i} 2"/></svg>"#
                );
                handle_from_svg(&doc)
            })
            .collect();
        let keeper_ids: Vec<_> = keepers.iter().map(Handle::id).collect();

        // Fill to capacity without overflowing keepers (they are the oldest).
        for i in 0..(CACHE_CAP - keepers.len()) {
            let doc = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M{i} 1"/></svg>"#
            );
            let _ = handle_from_svg(&doc);
        }
        {
            let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
            assert_eq!(guard.len(), CACHE_CAP);
        }
        // Still within cap: keepers must survive (no clear-all).
        for (doc_i, id) in keeper_ids.iter().enumerate() {
            let doc = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M{doc_i} 2"/></svg>"#
            );
            assert_eq!(handle_from_svg(&doc).id(), *id, "keeper {doc_i} was wiped");
        }

        // One more insert evicts the oldest keeper only.
        let _ = handle_from_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><path d="M9 9"/></svg>"#,
        );
        {
            let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
            assert!(guard.len() <= CACHE_CAP);
        }
        // Newer keepers (1..) still present; capacity never resets to ~1.
        for doc_i in 1..keepers.len() {
            let doc = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M{doc_i} 2"/></svg>"#
            );
            assert_eq!(
                handle_from_svg(&doc).id(),
                keeper_ids[doc_i],
                "keeper {doc_i} should survive single FIFO eviction"
            );
        }
    }
}
