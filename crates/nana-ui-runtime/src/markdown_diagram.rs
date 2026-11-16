//! Native Mermaid diagrams in Markdown.
//!
//! A fence is parsed and laid out by `nana-ui-diagram` once per source,
//! measurement and size: both results sit in bounded process-wide caches,
//! so layout, measure and input passes over the same document reuse one
//! [`DiagramScene`]. The scene is theme-neutral; [`record`] turns it into a
//! [`PaintRecording`] against the installed theme, which the scene
//! tessellates into vector meshes. A theme switch re-records without laying
//! the diagram out again.

use std::sync::{Arc, Mutex, OnceLock};

use nana_ui_core::{CompiledTheme, SemanticPalette};
use nana_ui_diagram::geom::Verb;
use nana_ui_diagram::{
    Align, Cap, Corner, Dash, DiagramError, DiagramMetrics, DiagramScene, FixedMeasure, FontSpec,
    Geom, Ink, Item, Join, Limits, Parsed, StrokeWidth, TextMeasure, Tone,
};

use crate::custom_paint::{
    LineCap, LineJoin, PaintColor, PaintContext, PaintPath, PaintRecording, PaintState, PaintText,
    Painter, StrokeStyle,
};
use crate::{LayoutBox, TextHorizontalAlignment, TextVerticalAlignment};

/// What a mermaid fence becomes.
#[derive(Debug, Clone)]
pub(crate) enum Outcome {
    /// A native drawing.
    Native(Arc<DiagramScene>),
    /// Source the engine cannot read: shown as a code block.
    Fallback,
}

/// The advance closure Markdown measures runs with, plus its identity.
pub(crate) struct Measurer<'a> {
    pub advances: Option<&'a crate::rich_text::MarkdownMeasure<'a>>,
    pub key: u64,
}

/// Body text size diagrams set their labels in, matching Markdown body text.
const FONT_SIZE: f32 = nana_ui_core::type_scale::BODY;

pub(crate) fn metrics() -> DiagramMetrics {
    DiagramMetrics::from_font_size(FONT_SIZE)
}

/// What a fence says about itself for assistive technology: its title,
/// description and element counts, from the cached parse.
pub(crate) fn describe(source: &str) -> Option<Description> {
    let parsed = parsed(source)?;
    let parsed = parsed.as_ref().as_ref().ok()?;
    Some(Description {
        title: parsed
            .meta
            .acc_title
            .clone()
            .or_else(|| parsed.meta.title.clone()),
        description: parsed.meta.acc_descr.clone(),
        summary: parsed.summary(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Description {
    pub title: Option<String>,
    pub description: Option<String>,
    pub summary: nana_ui_diagram::Summary,
}

fn parsed(source: &str) -> Option<Arc<Result<Parsed, DiagramError>>> {
    let limits = Limits::DEFAULT;
    if source.len() > limits.source_bytes {
        return None;
    }
    let hash = hash_str(source);
    let mut cache = parse_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(hit) = cache.get(source, hash, 0) {
        return Some(hit);
    }
    let parsed = Arc::new(nana_ui_diagram::parse(source, &limits));
    cache.insert(source, hash, 0, Arc::clone(&parsed));
    Some(parsed)
}

/// Lay out a fence's source, reusing earlier work for the same source and
/// measurement.
pub(crate) fn layout(source: &str, measurer: &Measurer<'_>) -> Outcome {
    let limits = Limits::DEFAULT;
    let Some(parsed) = parsed(source) else {
        return Outcome::Fallback;
    };
    let hash = hash_str(source);
    let parsed: &Parsed = match parsed.as_ref() {
        Ok(parsed) => parsed,
        Err(_) => return Outcome::Fallback,
    };
    let metrics = metrics();
    let key = measurer.key ^ metrics.key().rotate_left(17);
    {
        let mut cache = layout_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(hit) = cache.get(source, hash, key) {
            return hit;
        }
    }
    let adapter = Adapter(measurer);
    let outcome = match nana_ui_diagram::layout(parsed, &adapter, &metrics, &limits) {
        Ok(scene) => Outcome::Native(Arc::new(scene)),
        Err(_) => Outcome::Fallback,
    };
    layout_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(source, hash, key, outcome.clone());
    outcome
}

struct Adapter<'a, 'b>(&'a Measurer<'b>);

impl TextMeasure for Adapter<'_, '_> {
    fn key(&self) -> u64 {
        self.0.key
    }

    fn advances(&self, text: &str, font: FontSpec) -> Vec<f32> {
        if let Some(measure) = self.0.advances {
            let advances = measure(text, font.size, font.weight, font.italic);
            use unicode_segmentation::UnicodeSegmentation as _;
            if advances.len() == text.graphemes(true).count() {
                return advances;
            }
        }
        FixedMeasure.advances(text, font)
    }
}

/// A small least-recently-used map keyed by source text and a secondary key.
struct Lru<T> {
    entries: Vec<(Arc<str>, u64, u64, T, u64)>,
    clock: u64,
    cap: usize,
}

impl<T: Clone> Lru<T> {
    fn new(cap: usize) -> Self {
        Self {
            entries: Vec::new(),
            clock: 0,
            cap,
        }
    }

    fn get(&mut self, source: &str, hash: u64, key: u64) -> Option<T> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self
            .entries
            .iter_mut()
            .find(|(s, h, k, ..)| *h == hash && *k == key && s.as_ref() == source)?;
        entry.4 = clock;
        Some(entry.3.clone())
    }

    fn insert(&mut self, source: &str, hash: u64, key: u64, value: T) {
        self.clock += 1;
        if self.entries.len() >= self.cap
            && let Some(oldest) = (0..self.entries.len()).min_by_key(|&i| self.entries[i].4)
        {
            self.entries.swap_remove(oldest);
        }
        self.entries
            .push((Arc::from(source), hash, key, value, self.clock));
    }
}

fn parse_cache() -> &'static Mutex<Lru<Arc<Result<Parsed, DiagramError>>>> {
    static CACHE: OnceLock<Mutex<Lru<Arc<Result<Parsed, DiagramError>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Lru::new(256)))
}

fn layout_cache() -> &'static Mutex<Lru<Outcome>> {
    static CACHE: OnceLock<Mutex<Lru<Outcome>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Lru::new(128)))
}

fn hash_str(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for b in text.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A laid-out diagram recorded against a theme at the size it is shown.
#[derive(Clone, Debug)]
pub struct DiagramDrawing {
    pub(crate) scene: Arc<DiagramScene>,
    pub recording: Arc<PaintRecording>,
    /// Size of the recording: the natural size times `scale`.
    pub size: [f32; 2],
    /// How far the diagram was scaled down to fit its column.
    pub scale: f32,
}

impl PartialEq for DiagramDrawing {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.scene, &other.scene) && Arc::ptr_eq(&self.recording, &other.recording)
    }
}

/// Record `scene` against `theme`, scaled by `scale` (at most 1). Geometry
/// and type shrink together; hairlines keep their width so a narrow column
/// does not wash the lines out.
pub(crate) fn record(
    scene: &Arc<DiagramScene>,
    theme: &CompiledTheme,
    scale: f32,
) -> DiagramDrawing {
    let scale = if scale.is_finite() {
        scale.clamp(0.05, 1.0)
    } else {
        1.0
    };
    let painter = DiagramPainter {
        scene: Arc::clone(scene),
        series: theme.palette().series_palette().into(),
        palette: theme.palette(),
        hairline: theme.border().hairline,
        scale,
    };
    let size = [scene.size[0] * scale, scene.size[1] * scale];
    let recording = PaintRecording::record(&painter, theme, size, PaintState::default());
    DiagramDrawing {
        scene: Arc::clone(scene),
        recording: Arc::new(recording),
        size,
        scale,
    }
}

struct DiagramPainter {
    scene: Arc<DiagramScene>,
    series: Arc<[[f32; 4]]>,
    palette: SemanticPalette,
    hairline: f32,
    scale: f32,
}

impl DiagramPainter {
    fn color(&self, cx: &PaintContext<'_>, ink: Ink) -> PaintColor {
        match ink {
            Ink::Role(role) => PaintColor::Role(role),
            Ink::Mix(mix) => PaintColor::Mix(mix),
            Ink::Rgba(rgba) => PaintColor::Rgba(rgba),
            Ink::Series { index, tone } => {
                let base = if self.series.is_empty() {
                    self.palette.text.as_rgba_array()
                } else {
                    self.series[usize::from(index) % self.series.len()]
                };
                PaintColor::Rgba(match tone {
                    Tone::Solid => base,
                    Tone::Soft => {
                        let surface = cx.color(nana_ui_core::SemanticColorRole::Surface);
                        let t = 0.22;
                        [
                            surface[0] + (base[0] - surface[0]) * t,
                            surface[1] + (base[1] - surface[1]) * t,
                            surface[2] + (base[2] - surface[2]) * t,
                            1.0,
                        ]
                    }
                    Tone::On => {
                        let l = 0.2126 * base[0] + 0.7152 * base[1] + 0.0722 * base[2];
                        if l > 0.55 {
                            [0.1, 0.1, 0.12, 1.0]
                        } else {
                            [1.0, 1.0, 1.0, 1.0]
                        }
                    }
                })
            }
        }
    }

    fn width(&self, width: StrokeWidth) -> f32 {
        match width {
            StrokeWidth::Hairline => self.hairline,
            StrokeWidth::Thick => self.hairline * 2.0,
            StrokeWidth::Px(px) => (px * self.scale).max(self.hairline.min(px)),
        }
    }

    fn rect(&self, rect: &nana_ui_diagram::Rect) -> nana_ui_diagram::Rect {
        let s = self.scale;
        nana_ui_diagram::Rect::new(rect.x * s, rect.y * s, rect.w * s, rect.h * s)
    }

    fn path(&self, cx: &PaintContext<'_>, geom: &Geom) -> PaintPath {
        let s = self.scale;
        match geom {
            Geom::Rect { rect, corner } => {
                let mut path = PaintPath::new();
                let r = self.rect(rect);
                let radius = radius(cx, *corner, r.w, r.h);
                path.rounded_rect(layout_box(&r), [radius; 4]);
                path
            }
            Geom::Ellipse(rect) => {
                let mut path = PaintPath::new();
                path.ellipse(layout_box(&self.rect(rect)));
                path
            }
            Geom::Path(_) if s != 1.0 => path_of(cx, geom).transformed([s, 0.0, 0.0, s, 0.0, 0.0]),
            Geom::Path(_) => path_of(cx, geom),
        }
    }
}

fn path_of(cx: &PaintContext<'_>, geom: &Geom) -> PaintPath {
    let mut path = PaintPath::new();
    match geom {
        Geom::Rect { rect, corner } => {
            let r = radius(cx, *corner, rect.w, rect.h);
            path.rounded_rect(layout_box(rect), [r; 4]);
        }
        Geom::Ellipse(rect) => {
            path.ellipse(layout_box(rect));
        }
        Geom::Path(data) => {
            for verb in &data.verbs {
                match *verb {
                    Verb::Move(p) => {
                        path.move_to(p[0], p[1]);
                    }
                    Verb::Line(p) => {
                        path.line_to(p[0], p[1]);
                    }
                    Verb::Quad(c, p) => {
                        path.quad_to(c[0], c[1], p[0], p[1]);
                    }
                    Verb::Cubic(a, b, p) => {
                        path.cubic_to(a[0], a[1], b[0], b[1], p[0], p[1]);
                    }
                    Verb::Close => {
                        path.close();
                    }
                }
            }
        }
    }
    path
}

fn radius(cx: &PaintContext<'_>, corner: Corner, w: f32, h: f32) -> f32 {
    let r = match corner {
        Corner::None => 0.0,
        Corner::Px(px) => px,
        Corner::Tier(tier) => cx.radius(tier),
    };
    r.min(w / 2.0).min(h / 2.0).max(0.0)
}

fn layout_box(rect: &nana_ui_diagram::Rect) -> LayoutBox {
    LayoutBox {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
    }
}

impl Painter for DiagramPainter {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        for item in &self.scene.items {
            match item {
                Item::Shape { geom, fill, stroke } => {
                    let solid_rect = matches!(geom, Geom::Rect { .. })
                        && stroke.is_none_or(|s| s.dash == Dash::Solid);
                    if let (true, Geom::Rect { rect, corner }) = (solid_rect, geom) {
                        let rect = &self.rect(rect);
                        let r = radius(cx, *corner, rect.w, rect.h);
                        let mut paint = crate::custom_paint::BoxPaint {
                            fill: fill.map(|ink| self.color(cx, ink).into()),
                            ..Default::default()
                        };
                        if let Some(s) = stroke {
                            paint = paint.border(self.color(cx, s.ink), self.width(s.width));
                        }
                        cx.rounded_rect(layout_box(rect), r, paint);
                        continue;
                    }
                    let path = self.path(cx, geom);
                    if let Some(ink) = fill {
                        let color = self.color(cx, *ink);
                        cx.fill_path(&path, color);
                    }
                    if let Some(s) = stroke {
                        let width = self.width(s.width);
                        let mut style = StrokeStyle::new(width)
                            .cap(match s.cap {
                                Cap::Butt => LineCap::Butt,
                                Cap::Round => LineCap::Round,
                                Cap::Square => LineCap::Square,
                            })
                            .join(match s.join {
                                Join::Miter => LineJoin::Miter,
                                Join::Round => LineJoin::Round,
                                Join::Bevel => LineJoin::Bevel,
                            });
                        match s.dash {
                            Dash::Solid => {}
                            Dash::Dashed => style = style.dash(vec![width * 5.0, width * 3.5], 0.0),
                            Dash::Dotted => {
                                style = style
                                    .dash(vec![width.max(1.0) * 0.01, width.max(1.0) * 3.0], 0.0)
                            }
                        }
                        let color = self.color(cx, s.ink);
                        cx.stroke_path(&path, style, color);
                    }
                }
                Item::Text {
                    rect,
                    text,
                    font,
                    ink,
                    align,
                } => {
                    // A little slack keeps rounding from eliding a run the
                    // layout measured to fit.
                    let rect = &self.rect(rect);
                    let size = font.size * self.scale;
                    let slack = size * 0.5;
                    let horizontal = match align {
                        Align::Start => TextHorizontalAlignment::Start,
                        Align::Center => TextHorizontalAlignment::Center,
                        Align::End => TextHorizontalAlignment::End,
                    };
                    let x = match align {
                        Align::Start => rect.x,
                        Align::Center => rect.x - slack / 2.0,
                        Align::End => rect.x - slack,
                    };
                    let color = self.color(cx, *ink);
                    cx.text(
                        LayoutBox {
                            x,
                            y: rect.y,
                            width: rect.w + slack,
                            height: rect.h,
                        },
                        PaintText::new(text.clone())
                            .color(color)
                            .size(size)
                            .weight(font.weight)
                            .italic(font.italic)
                            .line_height(rect.h)
                            .align(horizontal, TextVerticalAlignment::Center),
                    );
                }
                Item::ClipBegin(geom) => {
                    let path = self.path(cx, geom);
                    cx.push_clip(&path);
                }
                Item::ClipEnd => cx.pop_clip(),
            }
        }
    }

    fn paint_key(&self) -> u64 {
        Arc::as_ptr(&self.scene) as u64 ^ u64::from(self.scale.to_bits()).rotate_left(32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed<'a>() -> Measurer<'a> {
        Measurer {
            advances: None,
            key: 0,
        }
    }

    #[test]
    fn layouts_are_reused_and_errors_pick_their_fallback() {
        let source = "flowchart LR\n  A[cached] --> B";
        let Outcome::Native(a) = layout(source, &fixed()) else {
            panic!("native")
        };
        let Outcome::Native(b) = layout(source, &fixed()) else {
            panic!("native")
        };
        assert!(Arc::ptr_eq(&a, &b));
        assert!(matches!(
            layout("flowchart LR\n  A --> B[unclosed", &fixed()),
            Outcome::Native(_) | Outcome::Fallback
        ));
        assert!(matches!(
            layout("notADiagram\n  x", &fixed()),
            Outcome::Fallback
        ));
        assert!(matches!(
            layout(
                "sequenceDiagram
  A->>B: hi",
                &fixed()
            ),
            Outcome::Native(_)
        ));
    }

    #[test]
    fn recording_resolves_roles_and_rerecords_per_theme() {
        let Outcome::Native(scene) = layout("flowchart TD\n  A[Start] -->|go| B{Check}", &fixed())
        else {
            panic!("native")
        };
        let light = record(
            &scene,
            nana_ui_core::builtin_theme(nana_ui_core::ThemeAppearance::Light),
            1.0,
        );
        let dark = record(
            &scene,
            nana_ui_core::builtin_theme(nana_ui_core::ThemeAppearance::Dark),
            1.0,
        );
        let half = record(
            &scene,
            nana_ui_core::builtin_theme(nana_ui_core::ThemeAppearance::Light),
            0.5,
        );
        assert_eq!(half.size, [scene.size[0] * 0.5, scene.size[1] * 0.5]);
        assert!(!light.recording.behind_children.is_empty());
        assert_ne!(*light.recording, *dark.recording);
        assert!(Arc::ptr_eq(&light.scene, &dark.scene));
        let texts = light
            .recording
            .behind_children
            .iter()
            .filter(|op| matches!(op, crate::custom_paint::PaintOp::Text { .. }))
            .count();
        assert_eq!(texts, 3);
    }
}
