//! Icon coverage masks, drawn from an icon's own geometry.
//!
//! An [`Icon`] carries its outline twice: as typed shapes on a 24 × 24 grid
//! and as the SVG those shapes were generated from. The atlas used to
//! rasterize the SVG, which put an XML parser, an SVG tree builder and a
//! second path rasterizer (`usvg`, `resvg`, `tiny-skia`: about 0.8 MiB) into
//! every application that shows an icon. The shapes are drawn here instead,
//! by the rasterizer glyphs are scaled with (`nana_text::raster`).
//!
//! The drawing rule is Tabler's, which is what the catalog is generated
//! from: every shape is an unfilled outline two grid units wide with round
//! caps and joins, bar a rectangle or a path that asks to be filled.

use nana_text::raster::{PathPaint, PathStep, path_coverage};
use nana_ui_core::{Icon, IconPathCommand, IconShape};

/// The grid an icon's geometry is drawn on.
const GRID: f32 = 24.0;
/// Outline width, in grid units.
const STROKE: f32 = 2.0;
/// A line shorter than this, in grid units, is a dot.
const DOT: f32 = 0.1;
/// How far a cubic's control points sit along a quarter circle's tangents,
/// as a fraction of its radius.
const QUARTER_ARC: f32 = 0.552_284_8;

/// `icon` as a `pixel_size` square of white texels whose alpha is its
/// coverage (RGBA, row-major). `None` when there is nothing to draw it from.
pub(super) fn white_mask(icon: Icon, pixel_size: u32) -> Option<Vec<u8>> {
    if pixel_size == 0 {
        return None;
    }
    if icon.shapes().is_empty() {
        return from_svg(icon, pixel_size);
    }
    let coverage = coverage(icon.shapes(), pixel_size);
    let mut rgba = vec![255; coverage.len() * 4];
    for (texel, alpha) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(coverage) {
        texel[3] = alpha;
    }
    Some(rgba)
}

/// Rasterize `shapes` on the 24x24 icon grid into straight-alpha RGBA8 pixels
/// tinted with `color` at `(width, height)`.
#[cfg(feature = "hosted")]
pub(crate) fn rasterize_shapes_rgba(
    shapes: &[IconShape],
    color: [u8; 4],
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || shapes.is_empty() {
        return None;
    }
    let edge = width.max(height);
    let cov = coverage(shapes, edge);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let alpha = cov[y * edge as usize + x];
            let out_alpha = ((u32::from(alpha) * u32::from(color[3]) + 127) / 255) as u8;
            let idx = (y * width as usize + x) * 4;
            rgba[idx] = color[0];
            rgba[idx + 1] = color[1];
            rgba[idx + 2] = color[2];
            rgba[idx + 3] = out_alpha;
        }
    }
    Some(rgba)
}

/// Rasterize an SVG cursor document into straight-alpha RGBA8 pixels at
/// `(width, height)`, or emit `IMAGE_CODEC_UNAVAILABLE` once when `svg` is off.
#[cfg(feature = "hosted")]
pub(crate) fn rasterize_svg_rgba(
    cache_key: u64,
    bytes: &[u8],
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || bytes.is_empty() {
        return None;
    }
    #[cfg(feature = "svg")]
    {
        let _ = cache_key;
        let raster = nana_svg_raster::rasterize_document_capped(bytes, width.max(height))?;
        if raster.width == width && raster.height == height {
            return Some(raster.rgba.to_vec());
        }
        let img = image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba.to_vec())?;
        let resized =
            image::imageops::resize(&img, width, height, image::imageops::FilterType::Triangle);
        Some(resized.into_raw())
    }
    #[cfg(not(feature = "svg"))]
    {
        let _ = (bytes, width, height);
        emit_svg_hash_unavailable_once(cache_key);
        None
    }
}

#[cfg(all(feature = "hosted", not(feature = "svg")))]
fn emit_svg_hash_unavailable_once(key: u64) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static REPORTED: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    let set = REPORTED.get_or_init(|| Mutex::new(HashSet::new()));
    let Ok(mut guard) = set.lock() else {
        return;
    };
    if guard.len() < 1024 && guard.insert(key) {
        nana_diagnostics::event!(nana_diagnostics::framework::resource::IMAGE_CODEC_UNAVAILABLE);
    }
}

#[cfg(not(feature = "svg"))]
fn emit_svg_codec_unavailable_once(icon: Icon) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static REPORTED: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
    let set = REPORTED.get_or_init(|| Mutex::new(HashSet::new()));
    let Ok(mut guard) = set.lock() else {
        return;
    };
    if guard.len() < 1024 && guard.insert(icon.as_ptr() as usize) {
        nana_diagnostics::event!(nana_diagnostics::framework::resource::IMAGE_CODEC_UNAVAILABLE);
    }
}

/// An icon a host wrapped with no shapes is drawn from its SVG, when the
/// application reads SVG at all.
fn from_svg(icon: Icon, pixel_size: u32) -> Option<Vec<u8>> {
    #[cfg(feature = "svg")]
    if !icon.svg().is_empty() {
        return nana_svg_raster::rasterize_white_mask(icon.svg(), pixel_size, pixel_size);
    }
    #[cfg(not(feature = "svg"))]
    if !icon.svg().is_empty() {
        emit_svg_codec_unavailable_once(icon);
    }
    let _ = (icon, pixel_size);
    None
}

/// A path as the rasterizer takes it: points, and the steps that use them.
#[derive(Default)]
struct Path {
    points: Vec<[f32; 2]>,
    steps: Vec<PathStep>,
}

impl Path {
    fn move_to(&mut self, to: [f32; 2]) {
        self.points.push(to);
        self.steps.push(PathStep::MoveTo);
    }

    fn line_to(&mut self, to: [f32; 2]) {
        self.points.push(to);
        self.steps.push(PathStep::LineTo);
    }

    fn curve_to(&mut self, control_a: [f32; 2], control_b: [f32; 2], to: [f32; 2]) {
        self.points.extend([control_a, control_b, to]);
        self.steps.push(PathStep::CurveTo);
    }

    fn close(&mut self) {
        self.steps.push(PathStep::Close);
    }

    fn commands(&mut self, commands: &[IconPathCommand]) {
        for command in commands {
            match *command {
                IconPathCommand::MoveTo(to) => self.move_to(to),
                IconPathCommand::LineTo(to) => self.line_to(to),
                IconPathCommand::CubicTo {
                    control_a,
                    control_b,
                    to,
                } => self.curve_to(control_a, control_b, to),
                IconPathCommand::Close => self.close(),
            }
        }
    }

    /// A quarter of an ellipse from `from`, where the path is, to `to`,
    /// bulging toward `corner`: the corner of the box the two points sit on
    /// the sides of.
    fn quarter(&mut self, from: [f32; 2], corner: [f32; 2], to: [f32; 2]) {
        let toward = |a: [f32; 2], b: [f32; 2]| {
            [
                a[0] + (b[0] - a[0]) * QUARTER_ARC,
                a[1] + (b[1] - a[1]) * QUARTER_ARC,
            ]
        };
        self.curve_to(toward(from, corner), toward(to, corner), to);
    }

    fn circle(&mut self, [cx, cy]: [f32; 2], radius: f32) {
        let (left, right) = (cx - radius, cx + radius);
        let (top, bottom) = (cy - radius, cy + radius);
        self.move_to([right, cy]);
        self.quarter([right, cy], [right, bottom], [cx, bottom]);
        self.quarter([cx, bottom], [left, bottom], [left, cy]);
        self.quarter([left, cy], [left, top], [cx, top]);
        self.quarter([cx, top], [right, top], [right, cy]);
        self.close();
    }

    fn rect(&mut self, [x, y]: [f32; 2], [width, height]: [f32; 2], radius: f32) {
        let radius = radius.clamp(0.0, width.min(height) * 0.5);
        let (right, bottom) = (x + width, y + height);
        if radius == 0.0 {
            self.move_to([x, y]);
            self.line_to([right, y]);
            self.line_to([right, bottom]);
            self.line_to([x, bottom]);
            self.close();
            return;
        }
        self.move_to([x + radius, y]);
        self.line_to([right - radius, y]);
        self.quarter([right - radius, y], [right, y], [right, y + radius]);
        self.line_to([right, bottom - radius]);
        self.quarter(
            [right, bottom - radius],
            [right, bottom],
            [right - radius, bottom],
        );
        self.line_to([x + radius, bottom]);
        self.quarter([x + radius, bottom], [x, bottom], [x, bottom - radius]);
        self.line_to([x, y + radius]);
        self.quarter([x, y + radius], [x, y], [x + radius, y]);
        self.close();
    }

    /// `commands` as outline subpaths, broken wherever the path turns
    /// straight back on itself.
    ///
    /// The stroker joins two segments by the angle between them, and at a
    /// cusp (the points of `sparkles`, where one arc ends heading the way
    /// the next one came from) it has no angle to work with and draws no
    /// join: the tip comes out a stroke's half width short. Ending the
    /// subpath there gives the tip two round caps instead, which together
    /// are the disc a round join is.
    ///
    /// A segment too short to have a direction is a dot (Tabler writes one
    /// as `M12 12v.01`): the stroker drops it, where a round cap makes it a
    /// disc as wide as the stroke. It goes into `dots` as that disc.
    fn outline(&mut self, commands: &[IconPathCommand], dots: &mut Path) {
        let (mut start, mut at) = ([0.0f32; 2], [0.0f32; 2]);
        // The direction the last segment arrived in, while a subpath is open.
        let mut arrived: Option<[f32; 2]> = None;
        for command in commands {
            match *command {
                IconPathCommand::MoveTo(to) => {
                    start = to;
                    at = to;
                    arrived = None;
                }
                IconPathCommand::LineTo(to) => {
                    let along = direction(at, to);
                    if along[0].hypot(along[1]) < DOT {
                        let middle = [(at[0] + to[0]) * 0.5, (at[1] + to[1]) * 0.5];
                        dots.circle(middle, STROKE * 0.5);
                        arrived = None;
                    } else {
                        if arrived.is_none_or(|before| turns_back(before, along)) {
                            self.move_to(at);
                        }
                        self.line_to(to);
                        arrived = Some(along);
                    }
                    at = to;
                }
                IconPathCommand::CubicTo {
                    control_a,
                    control_b,
                    to,
                } => {
                    // A control point on its end point says nothing of the
                    // direction there; the next point along does.
                    let leaves = [control_a, control_b, to]
                        .into_iter()
                        .map(|point| direction(at, point))
                        .find(|toward| *toward != [0.0; 2])
                        .unwrap_or([0.0; 2]);
                    let arrives = [control_b, control_a, at]
                        .into_iter()
                        .map(|point| direction(point, to))
                        .find(|toward| *toward != [0.0; 2])
                        .unwrap_or([0.0; 2]);
                    if arrived.is_none_or(|before| turns_back(before, leaves)) {
                        self.move_to(at);
                    }
                    self.curve_to(control_a, control_b, to);
                    arrived = Some(arrives);
                    at = to;
                }
                IconPathCommand::Close => {
                    if arrived.is_some() {
                        self.close();
                    }
                    at = start;
                    arrived = None;
                }
            }
        }
    }
}

fn direction(from: [f32; 2], to: [f32; 2]) -> [f32; 2] {
    [to[0] - from[0], to[1] - from[1]]
}

/// Whether `after` heads back the way `before` came, to within a couple of
/// degrees.
fn turns_back(before: [f32; 2], after: [f32; 2]) -> bool {
    let dot = before[0] * after[0] + before[1] * after[1];
    let lengths = (before[0].hypot(before[1])) * (after[0].hypot(after[1]));
    lengths > 0.0 && dot < -0.999 * lengths
}

/// One byte of coverage per texel.
fn coverage(shapes: &[IconShape], pixel_size: u32) -> Vec<u8> {
    let (mut outlined, mut filled) = (Path::default(), Path::default());
    for shape in shapes {
        match *shape {
            IconShape::Path(commands) => outlined.outline(commands, &mut filled),
            IconShape::Fill(commands) => filled.commands(commands),
            IconShape::Circle { center, radius } => outlined.circle(center, radius),
            IconShape::Rect {
                origin,
                size,
                filled: fill,
            } => {
                let path = if fill { &mut filled } else { &mut outlined };
                path.rect(origin, size, 0.0);
            }
            IconShape::RoundedRect {
                origin,
                size,
                radius,
            } => outlined.rect(origin, size, radius),
        }
    }
    let scale = pixel_size as f32 / GRID;
    let mut coverage = path_coverage(
        &outlined.points,
        &outlined.steps,
        PathPaint::Stroke { width: STROKE },
        scale,
        pixel_size,
    );
    if !filled.steps.is_empty() {
        let inside = path_coverage(
            &filled.points,
            &filled.steps,
            PathPaint::Fill,
            scale,
            pixel_size,
        );
        // Painted over the outlines, as the SVG paints one shape over another.
        for (under, over) in coverage.iter_mut().zip(inside) {
            let (under_alpha, over_alpha) = (u32::from(*under), u32::from(over));
            *under = (over_alpha + under_alpha * (255 - over_alpha) / 255) as u8;
        }
    }
    coverage
}

#[cfg(all(test, feature = "svg"))]
mod tests {
    use super::*;

    /// The built-in catalog.
    const CATALOG: &[Icon] = &[
        Icon::About,
        Icon::Add,
        Icon::Appearance,
        Icon::ArrowLeft,
        Icon::ArrowRight,
        Icon::ArrowUp,
        Icon::Bot,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::ChevronUp,
        Icon::Chart,
        Icon::Close,
        Icon::Eye,
        Icon::File,
        Icon::Folder,
        Icon::GitBranch,
        Icon::Maximize,
        Icon::MessageSquarePlus,
        Icon::Minimize,
        Icon::Moon,
        Icon::More,
        Icon::Nodes,
        Icon::Paperclip,
        Icon::Restore,
        Icon::Search,
        Icon::ColorPicker,
        Icon::Settings,
        Icon::ShieldCheck,
        Icon::Sidebar,
        Icon::Sparkles,
        Icon::Workspace,
        Icon::Activity,
        Icon::Atom,
        Icon::Blend,
        Icon::Bug,
        Icon::Clapperboard,
        Icon::Cpu,
        Icon::Gamepad2,
        Icon::MonitorPlay,
        Icon::Package,
        Icon::Palette,
        Icon::Puzzle,
        Icon::Sun,
        Icon::Webcam,
        Icon::Wind,
        Icon::Reset,
        Icon::HelpCircle,
        Icon::AlertTriangle,
        Icon::CircleX,
    ];

    /// The SVG rasterizer the atlas used before, as the reference.
    fn through_svg(icon: Icon, pixel_size: u32) -> Vec<u8> {
        nana_svg_raster::rasterize_white_mask(icon.svg(), pixel_size, pixel_size)
            .expect("a catalog icon is valid SVG")
    }

    /// Mean per-texel difference in coverage, out of 255.
    fn apart(ours: &[u8], reference: &[u8]) -> f32 {
        assert_eq!(ours.len(), reference.len());
        let total: u64 = ours
            .as_chunks::<4>()
            .0
            .iter()
            .zip(reference.as_chunks::<4>().0)
            .map(|(a, b)| u64::from(a[3].abs_diff(b[3])))
            .sum();
        total as f32 / (ours.len() / 4) as f32
    }

    /// The texels with ink, as `(left, top, right, bottom)`.
    fn ink(mask: &[u8], pixel_size: u32) -> (u32, u32, u32, u32) {
        let (mut left, mut top, mut right, mut bottom) = (pixel_size, pixel_size, 0, 0);
        for y in 0..pixel_size {
            for x in 0..pixel_size {
                if mask[((y * pixel_size + x) * 4 + 3) as usize] > 32 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x);
                    bottom = bottom.max(y);
                }
            }
        }
        (left, top, right, bottom)
    }

    /// The shapes written back out as SVG: what separates a difference in
    /// the geometry from a difference between the two rasterizers.
    fn shapes_as_svg(icon: Icon) -> String {
        let mut body = String::new();
        for shape in icon.shapes() {
            match *shape {
                IconShape::Path(commands) => {
                    let mut d = String::new();
                    for command in commands {
                        match *command {
                            IconPathCommand::MoveTo([x, y]) => d += &format!("M{x} {y}"),
                            IconPathCommand::LineTo([x, y]) => d += &format!("L{x} {y}"),
                            IconPathCommand::CubicTo {
                                control_a: [ax, ay],
                                control_b: [bx, by],
                                to: [x, y],
                            } => d += &format!("C{ax} {ay} {bx} {by} {x} {y}"),
                            IconPathCommand::Close => d += "Z",
                        }
                    }
                    body += &format!(r#"<path d="{d}"/>"#);
                }
                IconShape::Fill(_) => unreachable!("the built-in catalog fills nothing"),
                IconShape::Circle {
                    center: [cx, cy],
                    radius,
                } => body += &format!(r#"<circle cx="{cx}" cy="{cy}" r="{radius}"/>"#),
                IconShape::Rect {
                    origin: [x, y],
                    size: [w, h],
                    filled,
                } => {
                    let fill = if filled {
                        r#" fill="currentColor""#
                    } else {
                        ""
                    };
                    body += &format!(r#"<rect x="{x}" y="{y}" width="{w}" height="{h}"{fill}/>"#);
                }
                IconShape::RoundedRect {
                    origin: [x, y],
                    size: [w, h],
                    radius,
                } => {
                    body += &format!(
                        r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{radius}"/>"#
                    );
                }
            }
        }
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"#
        )
    }

    /// The shapes are the SVG's geometry: written back out as SVG and
    /// drawn by the same rasterizer, they give the same texels. This is what
    /// lets the atlas draw the shapes and leave the SVG alone.
    #[test]
    fn the_shapes_of_a_catalog_icon_are_its_svg() {
        for &icon in CATALOG {
            let rebuilt = nana_svg_raster::rasterize_white_mask(&shapes_as_svg(icon), 48, 48)
                .expect("the shapes write out as SVG");
            let mean = apart(&rebuilt, &through_svg(icon, 48));
            assert!(mean < 0.05, "{}: {mean}", icon.name());
        }
    }

    /// Drawn from its shapes, an icon covers what its SVG covered. The two
    /// rasterizers antialias an edge differently (measured over the catalog:
    /// a mean of 1 in 255 at 48 px, 3 for the busiest icon, 5 at 16 px), so
    /// the bound is on the mean and on where the ink reaches, not per texel.
    #[test]
    fn an_icon_drawn_from_its_shapes_covers_what_its_svg_covered() {
        for (pixel_size, bound) in [(16u32, 16.0f32), (32, 8.0), (48, 4.0), (96, 3.0)] {
            for &icon in CATALOG {
                let ours = white_mask(icon, pixel_size).expect("a catalog icon has shapes");
                let reference = through_svg(icon, pixel_size);
                assert!(
                    ours.as_chunks::<4>()
                        .0
                        .iter()
                        .all(|texel| texel[..3] == [255; 3]),
                    "the mask is white"
                );
                let mean = apart(&ours, &reference);
                assert!(
                    mean < bound,
                    "{} at {pixel_size} px: mean difference {mean}",
                    icon.name()
                );
                let (ours, reference) = (ink(&ours, pixel_size), ink(&reference, pixel_size));
                let reach = [
                    ours.0.abs_diff(reference.0),
                    ours.1.abs_diff(reference.1),
                    ours.2.abs_diff(reference.2),
                    ours.3.abs_diff(reference.3),
                ];
                assert!(
                    reach.iter().all(|edge| *edge <= 1),
                    "{} at {pixel_size} px: ink {ours:?}, the SVG's {reference:?}",
                    icon.name()
                );
            }
        }
    }

    /// The points of `sparkles` are cusps: each arc ends heading back the
    /// way the next one starts. The outline reaches a half stroke width
    /// past each tip there as it does past any other corner.
    #[test]
    fn a_cusp_is_capped_like_any_other_corner() {
        let ours = white_mask(Icon::Sparkles, 48).expect("sparkles has shapes");
        assert_eq!(ink(&ours, 48), ink(&through_svg(Icon::Sparkles, 48), 48));
    }

    #[test]
    fn a_filled_rectangle_is_filled_and_an_empty_icon_draws_nothing() {
        static SHAPES: &[IconShape] = &[IconShape::Rect {
            origin: [6.0, 6.0],
            size: [12.0, 12.0],
            filled: true,
        }];
        static BLOCK: nana_ui_core::IconData = nana_ui_core::IconData {
            name: "block",
            shapes: SHAPES,
            svg: "",
        };
        static EMPTY: nana_ui_core::IconData = nana_ui_core::IconData {
            name: "empty",
            shapes: &[],
            svg: "",
        };
        let mask = white_mask(Icon::from_data(&BLOCK), 24).expect("a filled block");
        assert_eq!(mask[(12 * 24 + 12) * 4 + 3], 255, "the middle is inside");
        assert_eq!(mask[(2 * 24 + 2) * 4 + 3], 0, "the corner is outside");
        assert_eq!(ink(&mask, 24), (6, 6, 17, 17));
        assert!(white_mask(Icon::from_data(&EMPTY), 24).is_none());
    }
}

#[cfg(all(test, not(feature = "svg")))]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn svg_only_icon_without_feature_emits_codec_unavailable_once_per_key() {
        const CHILD: &str = "NANA_UI_TEST_ICON_RASTER_SVG_CHILD";
        if std::env::var_os(CHILD).is_none() {
            crate::test_gpu::run_child_with_timeout(
                "scene_paint::icon_raster::diagnostic_tests::svg_only_icon_without_feature_emits_codec_unavailable_once_per_key",
                CHILD,
                "1",
            );
            return;
        }

        static NEXT_DIR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nana-icon-raster-diag-{}-{seq}",
            std::process::id()
        ));
        let guard = nana_diagnostics::install(
            nana_diagnostics::DiagnosticsConfig {
                min_severity: nana_diagnostics::Severity::Debug,
                persist: nana_diagnostics::PersistMode::All,
                poll_interval: std::time::Duration::from_millis(5),
                panic_hook: false,
                ..nana_diagnostics::DiagnosticsConfig::default()
            },
            nana_diagnostics::SessionMetadata::new(
                "dev.nana.icon-raster-test",
                "Icon raster diagnostics",
                "0",
            ),
            nana_diagnostics::DiagnosticsPaths::new(dir.join("logs"), dir.join("crash")),
        )
        .expect("isolated child owns global diagnostics");
        nana_diagnostics::register_thread();

        static SVG_ONLY: nana_ui_core::IconData = nana_ui_core::IconData {
            name: "svg-only",
            shapes: &[],
            svg: r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"/>"#,
        };
        static EMPTY: nana_ui_core::IconData = nana_ui_core::IconData {
            name: "empty",
            shapes: &[],
            svg: "",
        };

        assert!(white_mask(Icon::from_data(&SVG_ONLY), 24).is_none());
        assert!(white_mask(Icon::from_data(&SVG_ONLY), 32).is_none());
        assert!(white_mask(Icon::from_data(&EMPTY), 24).is_none());

        assert!(
            guard
                .diagnostics()
                .flush(false, std::time::Duration::from_secs(5))
        );
        let log = guard.diagnostics().current_log();
        drop(guard);

        let file = nana_diagnostics::nlog::read_file(log.expect("session log was opened")).unwrap();
        let count = file
            .entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    nana_diagnostics::nlog::Entry::Event { key, .. }
                        if file.event_name(*key) == Some("resource.image_codec_unavailable")
                )
            })
            .count();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            count, 1,
            "expected one image_codec_unavailable per svg icon key"
        );
    }
}
