//! Paint, border, shadow, gradient, filter, clip, and transform style specs.

use serde::{Deserialize, Serialize};

use crate::scrollbar::ScrollbarSkin;
use crate::style_model::{PaintColor, StyleModelRef};

use super::units::LengthSpec;

/// CSS `visibility` (layout placeholder vs paint/hit-test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisibilitySpec {
    #[default]
    Visible,
    Hidden,
}

/// GPU cap for comma-separated `box-shadow` layers.
pub const MAX_BOX_SHADOWS: usize = 4;

/// One `box-shadow` layer (physical px after parse).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadowSpec {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: PaintColor,
    /// CSS `inset`. Outset (false) paints outside the border box.
    pub inset: bool,
}

/// CSS `mix-blend-mode` (Compositing and Blending Level 1). Unknown values
/// fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MixBlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl MixBlendMode {
    pub fn is_normal(self) -> bool {
        matches!(self, Self::Normal)
    }

    /// Dest-group composite selector. `0` normal, `1` multiply and `2` screen
    /// blend in fixed function; `3..` read the backdrop in the shader, in
    /// declaration order.
    pub fn gpu_index(self) -> u32 {
        self as u32
    }
}

/// CSS `pointer-events` (`auto` hits, `none` skips).
///
/// Inherited. Unspecified specified-value is [`None`] on
/// [`LayoutStyle::pointer_events`]; used value is [`Self::inherit_from`].
/// Root initial used value is [`Self::Auto`]. Unknown keywords fail closed
/// and do not write a specified value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerEventsSpec {
    #[default]
    Auto,
    None,
}

/// CSS `cursor` keyword subset used by the desktop host.
///
/// The value is inherited. Custom `url(...)` cursors and unknown keywords are
/// intentionally rejected by the CSS parser.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorSpec {
    #[default]
    Default,
    Pointer,
    Text,
    Move,
    Grab,
    Grabbing,
    NotAllowed,
    Crosshair,
    /// Horizontal resize / scrub (`ew-resize`).
    EwResize,
    /// Vertical resize (`ns-resize`).
    NsResize,
    Help,
    Wait,
    Progress,
    ZoomIn,
    ZoomOut,
    None,
}

/// CSS `user-select` subset. Inherited, like [`CursorSpec`].
///
/// `auto` leaves ordinary `TextContent` unselectable (not a second TextInput).
/// `text` allows document-level drag selection on that node's text.
/// `all` selects the whole text node on press (no drag required).
/// `contain` is selectable like `text`, but a drag must not jump to a neighbor.
/// `none` keeps the node (and inherited descendants) out of the selection.
/// Unknown keywords fail closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UserSelectSpec {
    #[default]
    Auto,
    Text,
    All,
    Contain,
    None,
}

impl UserSelectSpec {
    pub fn allows_document_select(self) -> bool {
        matches!(self, Self::Text | Self::All | Self::Contain)
    }

    pub fn selects_whole_node_on_press(self) -> bool {
        matches!(self, Self::All)
    }
}

impl PointerEventsSpec {
    pub fn hittable(self) -> bool {
        matches!(self, Self::Auto)
    }

    /// Used value: specified wins, otherwise inherit `parent`.
    pub fn inherit_from(specified: Option<Self>, parent: Self) -> Self {
        specified.unwrap_or(parent)
    }
}

/// CSS `border-style` subset. Dashed/dotted stroke the existing rounded-box
/// SDF ring; `double` / 3D keywords occupy used width but do not paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Solid,
    Dashed,
    Dotted,
    /// Parsed but not stroked (`double` / groove / ridge / inset / outset).
    Unsupported,
}

impl BorderStyle {
    /// Packed into [`LayoutStyle::paint_border_style_codes`] / the quad shader.
    pub const SHADER_SOLID: u8 = 0;
    pub const SHADER_DASHED: u8 = 1;
    pub const SHADER_DOTTED: u8 = 2;

    /// `none` / `hidden` zero the used border width (CSS).
    pub fn zeros_used_width(self) -> bool {
        matches!(self, Self::None)
    }

    /// Solid / dashed / dotted (and unspecified-compat) use the rounded-box stroke.
    pub fn paints_stroke(self) -> bool {
        matches!(self, Self::Solid | Self::Dashed | Self::Dotted)
    }

    pub fn shader_code(self) -> u8 {
        match self {
            Self::Dashed => Self::SHADER_DASHED,
            Self::Dotted => Self::SHADER_DOTTED,
            Self::None | Self::Solid | Self::Unsupported => Self::SHADER_SOLID,
        }
    }
}

pub(super) fn used_border_width(width: f32, style: Option<BorderStyle>) -> f32 {
    match style {
        Some(s) if s.zeros_used_width() => 0.0,
        // Unspecified style keeps the declared width (legacy NanaUI / T-B09).
        // Explicit `none` / `hidden` still zero the used width.
        None | Some(_) => width.max(0.0),
    }
}

pub(super) fn paint_border_width(
    width: f32,
    style: Option<BorderStyle>,
    color: Option<[f32; 4]>,
) -> f32 {
    if width > 0.0 && color.is_some() && edge_paints_stroke(style) {
        width
    } else {
        0.0
    }
}

pub(super) fn edge_paints_stroke(style: Option<BorderStyle>) -> bool {
    style.map(BorderStyle::paints_stroke).unwrap_or(true)
}

/// CSS `outline-style` subset. Dashed/dotted fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutlineStyle {
    #[default]
    None,
    Solid,
}

/// Paint-only CSS `outline` (does not affect layout).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct OutlineSpec {
    pub width: f32,
    pub color: Option<[f32; 4]>,
    pub style: OutlineStyle,
}

impl OutlineSpec {
    pub fn is_active(self) -> bool {
        self.style == OutlineStyle::Solid && self.width > 0.0
    }
}

/// Single-layer `text-shadow` (physical px after parse; blur is paint-hint only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextShadowSpec {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: PaintColor,
}

/// `-webkit-text-stroke`: an outline centred on each glyph's outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStrokeSpec {
    /// Full stroke width in px. Zero draws nothing.
    pub width: f32,
    /// `None` is `currentColor`: the text's own fill.
    pub color: Option<PaintColor>,
}

/// CSS `text-decoration-line` subset Scene can stroke (underline / line-through).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextDecorationLine {
    pub underline: bool,
    pub line_through: bool,
}

impl TextDecorationLine {
    pub fn is_active(self) -> bool {
        self.underline || self.line_through
    }
}

/// One OpenType `font-feature-settings` tag (`"liga" 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FontFeatureSetting {
    pub tag: [u8; 4],
    pub value: u32,
}

impl FontFeatureSetting {
    pub const fn new(tag: [u8; 4], value: u32) -> Self {
        Self { tag, value }
    }
}

/// One stop in a CSS `linear-gradient` (position 0..=1 along the gradient line).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub position: f32,
    pub color: PaintColor,
}

/// CSS `linear-gradient` background / mask (2..=8 stops, angle in degrees).
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    /// CSS angle: `0deg` = to top, `90deg` = to right.
    pub angle_deg: f32,
    pub stops: Vec<GradientStop>,
}

impl LinearGradient {
    pub fn stop_count(&self) -> usize {
        self.stops.len().min(8)
    }
}

/// CSS `radial-gradient` background / mask (2..=8 stops).
#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradient {
    /// `true` = `circle`, `false` = `ellipse`.
    pub circle: bool,
    /// Center as specified (`%` / keywords → [`LengthSpec::Percent`], `px` →
    /// [`LengthSpec::Px`]). Resolve with [`Self::resolved_center`] at used-value;
    /// never treat px as a fraction of 100.
    pub center: [LengthSpec; 2],
    pub stops: Vec<GradientStop>,
}

impl RadialGradient {
    pub fn stop_count(&self) -> usize {
        self.stops.len().min(8)
    }

    /// Border-box fractions for the GPU sampler. Length units need a positive
    /// used box axis; missing size fails closed (`None`) instead of px/100.
    pub fn resolved_center(&self, width: f32, height: f32) -> Option<[f32; 2]> {
        Some([
            radial_center_axis_frac(self.center[0], width)?,
            radial_center_axis_frac(self.center[1], height)?,
        ])
    }
}

fn radial_center_axis_frac(spec: LengthSpec, axis: f32) -> Option<f32> {
    match spec {
        LengthSpec::Percent(p) => Some(p / 100.0),
        LengthSpec::Fill
        | LengthSpec::Shrink
        | LengthSpec::Auto
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent => None,
        _ => {
            if !axis.is_finite() || axis <= 0.0 {
                return None;
            }
            spec.resolve_px(Some(axis)).map(|px| px / axis)
        }
    }
}

/// Parsed CSS gradient fill (`linear-gradient` or `radial-gradient`).
#[derive(Debug, Clone, PartialEq)]
pub enum CssGradient {
    Linear(LinearGradient),
    Radial(RadialGradient),
}

impl CssGradient {
    pub fn stops(&self) -> &[GradientStop] {
        match self {
            Self::Linear(g) => &g.stops,
            Self::Radial(g) => &g.stops,
        }
    }

    pub fn stops_mut(&mut self) -> &mut [GradientStop] {
        match self {
            Self::Linear(g) => &mut g.stops,
            Self::Radial(g) => &mut g.stops,
        }
    }

    pub fn stop_count(&self) -> usize {
        match self {
            Self::Linear(g) => g.stop_count(),
            Self::Radial(g) => g.stop_count(),
        }
    }
}

/// Product cap for comma-separated `background-image` layers (plus `<img src>`).
pub const MAX_BACKGROUND_LAYERS: usize = 8;

/// `background-size` / `object-fit` mapping for `url()` fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundImageFit {
    Cover,
    Contain,
    Stretch,
    /// CSS initial `background-size: auto` / `object-fit: none`.
    #[default]
    Auto,
    /// Explicit `background-size` lengths (`32px`, `50%`, `auto 24px`).
    Length,
    /// CSS `object-fit: scale-down` (`min(none, contain)`).
    ScaleDown,
}

impl BackgroundImageFit {
    /// The [`crate::ContentFit`] this sizing is: every keyword but explicit
    /// `background-size` lengths ([`Self::Length`]) sizes the image as the
    /// matching `object-fit` does (`auto` is `none`, the natural size), so a
    /// `url()` layer and a host texture share one fit (Issue #202).
    pub const fn object_fit(self) -> Option<crate::ContentFit> {
        Some(match self {
            Self::Stretch => crate::ContentFit::Fill,
            Self::Contain => crate::ContentFit::Contain,
            Self::Cover => crate::ContentFit::Cover,
            Self::Auto => crate::ContentFit::None,
            Self::ScaleDown => crate::ContentFit::ScaleDown,
            Self::Length => return None,
        })
    }
}

impl From<crate::ContentFit> for BackgroundImageFit {
    fn from(fit: crate::ContentFit) -> Self {
        match fit {
            crate::ContentFit::Fill => Self::Stretch,
            crate::ContentFit::Contain => Self::Contain,
            crate::ContentFit::Cover => Self::Cover,
            crate::ContentFit::None => Self::Auto,
            crate::ContentFit::ScaleDown => Self::ScaleDown,
        }
    }
}

/// CSS `background-repeat` (shader tiling).
///
/// `space` and mixed `space`/`round`/`repeat` stay [`Self::Unsupported`] so the
/// url layer is not painted as `repeat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundRepeat {
    NoRepeat,
    /// CSS initial value for unspecified `background-repeat`.
    #[default]
    Repeat,
    RepeatX,
    RepeatY,
    /// Both axes `round` (integer tiles via dest scale).
    Round,
    /// `round` on X, `no-repeat` on Y.
    RoundX,
    /// `no-repeat` on X, `round` on Y.
    RoundY,
    /// `space` or mixed keywords we will not fake as `repeat`.
    Unsupported,
}

/// `background-position` / `object-position` (percent of free space, or px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundPosition {
    pub x: LengthSpec,
    pub y: LengthSpec,
}

impl Default for BackgroundPosition {
    fn default() -> Self {
        Self {
            x: LengthSpec::Percent(0.0),
            y: LengthSpec::Percent(0.0),
        }
    }
}

impl BackgroundPosition {
    pub fn center() -> Self {
        Self {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Percent(50.0),
        }
    }
}

/// Parsed `background-image` (`linear-gradient`, `radial-gradient`, or `url()`).
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundImage {
    Gradient(CssGradient),
    Url {
        url: String,
        fit: BackgroundImageFit,
        size_width: Option<LengthSpec>,
        size_height: Option<LengthSpec>,
        position: BackgroundPosition,
        repeat: BackgroundRepeat,
        /// Resample to the painted size (default) or keep a mip chain.
        sampling: crate::ImageSampling,
    },
}

impl BackgroundImage {
    pub fn url(url: impl Into<String>) -> Self {
        Self::url_with_fit(url, BackgroundImageFit::Auto)
    }

    pub fn url_with_fit(url: impl Into<String>, fit: BackgroundImageFit) -> Self {
        Self::Url {
            url: url.into(),
            fit,
            size_width: None,
            size_height: None,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::Repeat,
            sampling: crate::ImageSampling::Resample,
        }
    }

    /// Choose how a `url(...)` layer is prepared for sampling; gradients are
    /// unchanged. See [`crate::ImageSampling`].
    pub fn with_sampling(mut self, value: crate::ImageSampling) -> Self {
        if let Self::Url { sampling, .. } = &mut self {
            *sampling = value;
        }
        self
    }

    pub fn url_str(&self) -> Option<&str> {
        match self {
            Self::Url { url, .. } => Some(url.as_str()),
            Self::Gradient(_) => None,
        }
    }
}

/// `mask-image` source: gradient alpha or a jailed `url()` texture.
#[derive(Debug, Clone, PartialEq)]
pub enum MaskImage {
    Gradient(CssGradient),
    Url(String),
}

impl MaskImage {
    pub fn url_str(&self) -> Option<&str> {
        match self {
            Self::Url(url) => Some(url.as_str()),
            Self::Gradient(_) => None,
        }
    }
}

/// One `border-image-slice` edge. Unitless numbers are source pixels;
/// percentages are of the source image's corresponding axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BorderImageSlice {
    Number(f32),
    Percent(f32),
}

impl Default for BorderImageSlice {
    fn default() -> Self {
        Self::Percent(100.0)
    }
}

impl BorderImageSlice {
    pub fn to_px(self, image_len: f32) -> f32 {
        match self {
            Self::Number(value) => value.max(0.0),
            Self::Percent(percent) => (percent.max(0.0) / 100.0) * image_len.max(0.0),
        }
    }
}

/// One 9-slice tile: dest rect in border-box px, source UV 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderImageTile {
    pub dest_x: f32,
    pub dest_y: f32,
    pub dest_w: f32,
    pub dest_h: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

/// Minimal CSS `border-image`: `url()` or `linear-gradient` source, slice,
/// optional `fill`. Width defaults to `1`×slice; outset/repeat stay stretch.
#[derive(Debug, Clone, PartialEq)]
pub struct BorderImageSpec {
    pub source: BackgroundImage,
    /// Top, right, bottom, left.
    pub slice: [BorderImageSlice; 4],
    pub fill: bool,
}

impl BorderImageSpec {
    pub fn from_source(source: BackgroundImage) -> Self {
        Self {
            source,
            slice: [BorderImageSlice::Percent(100.0); 4],
            fill: false,
        }
    }

    pub fn paints_linear_or_url(&self) -> bool {
        match &self.source {
            BackgroundImage::Url { .. } => true,
            BackgroundImage::Gradient(CssGradient::Linear(_)) => true,
            BackgroundImage::Gradient(CssGradient::Radial(_)) => false,
        }
    }

    /// `image_w` / `image_h` are the CSS border-image intrinsic size (bitmap
    /// pixels, or the border box for a gradient).
    pub fn tiles(
        &self,
        image_w: f32,
        image_h: f32,
        box_w: f32,
        box_h: f32,
    ) -> Vec<BorderImageTile> {
        let image_w = image_w.max(1.0);
        let image_h = image_h.max(1.0);
        let mut top_px = self.slice[0].to_px(image_h);
        let mut right_px = self.slice[1].to_px(image_w);
        let mut bottom_px = self.slice[2].to_px(image_h);
        let mut left_px = self.slice[3].to_px(image_w);
        (top_px, bottom_px) = clamp_pair(top_px, bottom_px, image_h);
        (left_px, right_px) = clamp_pair(left_px, right_px, image_w);
        let u_l = left_px / image_w;
        let u_r = right_px / image_w;
        let v_t = top_px / image_h;
        let v_b = bottom_px / image_h;
        let (dest_top, dest_bottom) = clamp_pair(top_px, bottom_px, box_h.max(0.0));
        let (dest_left, dest_right) = clamp_pair(left_px, right_px, box_w.max(0.0));
        let mid_w = (box_w - dest_left - dest_right).max(0.0);
        let mid_h = (box_h - dest_top - dest_bottom).max(0.0);
        let mut tiles = vec![
            BorderImageTile {
                dest_x: 0.0,
                dest_y: 0.0,
                dest_w: dest_left,
                dest_h: dest_top,
                u0: 0.0,
                v0: 0.0,
                u1: u_l,
                v1: v_t,
            },
            BorderImageTile {
                dest_x: dest_left,
                dest_y: 0.0,
                dest_w: mid_w,
                dest_h: dest_top,
                u0: u_l,
                v0: 0.0,
                u1: 1.0 - u_r,
                v1: v_t,
            },
            BorderImageTile {
                dest_x: dest_left + mid_w,
                dest_y: 0.0,
                dest_w: dest_right,
                dest_h: dest_top,
                u0: 1.0 - u_r,
                v0: 0.0,
                u1: 1.0,
                v1: v_t,
            },
            BorderImageTile {
                dest_x: 0.0,
                dest_y: dest_top,
                dest_w: dest_left,
                dest_h: mid_h,
                u0: 0.0,
                v0: v_t,
                u1: u_l,
                v1: 1.0 - v_b,
            },
        ];
        if self.fill {
            tiles.push(BorderImageTile {
                dest_x: dest_left,
                dest_y: dest_top,
                dest_w: mid_w,
                dest_h: mid_h,
                u0: u_l,
                v0: v_t,
                u1: 1.0 - u_r,
                v1: 1.0 - v_b,
            });
        }
        tiles.extend([
            BorderImageTile {
                dest_x: dest_left + mid_w,
                dest_y: dest_top,
                dest_w: dest_right,
                dest_h: mid_h,
                u0: 1.0 - u_r,
                v0: v_t,
                u1: 1.0,
                v1: 1.0 - v_b,
            },
            BorderImageTile {
                dest_x: 0.0,
                dest_y: dest_top + mid_h,
                dest_w: dest_left,
                dest_h: dest_bottom,
                u0: 0.0,
                v0: 1.0 - v_b,
                u1: u_l,
                v1: 1.0,
            },
            BorderImageTile {
                dest_x: dest_left,
                dest_y: dest_top + mid_h,
                dest_w: mid_w,
                dest_h: dest_bottom,
                u0: u_l,
                v0: 1.0 - v_b,
                u1: 1.0 - u_r,
                v1: 1.0,
            },
            BorderImageTile {
                dest_x: dest_left + mid_w,
                dest_y: dest_top + mid_h,
                dest_w: dest_right,
                dest_h: dest_bottom,
                u0: 1.0 - u_r,
                v0: 1.0 - v_b,
                u1: 1.0,
                v1: 1.0,
            },
        ]);
        tiles.retain(|tile| {
            tile.dest_w > 0.0001
                && tile.dest_h > 0.0001
                && (tile.u1 - tile.u0) > 1.0e-6
                && (tile.v1 - tile.v0) > 1.0e-6
        });
        tiles
    }
}

fn clamp_pair(a: f32, b: f32, max: f32) -> (f32, f32) {
    let max = max.max(0.0);
    let sum = a + b;
    if sum > max && sum > 0.0 {
        let scale = max / sum;
        (a * scale, b * scale)
    } else {
        (a, b)
    }
}

/// `clip-path: inset(...)` components (lengths resolve against the border box).
#[derive(Debug, Clone, PartialEq)]
pub struct ClipInset {
    pub top: LengthSpec,
    pub right: LengthSpec,
    pub bottom: LengthSpec,
    pub left: LengthSpec,
    pub round: Option<LengthSpec>,
}

impl ClipInset {
    /// Resolve inset offsets to physical px `[top, right, bottom, left]`.
    pub fn resolve_offsets(&self, width: f32, height: f32) -> [f32; 4] {
        [
            resolve_inset_edge(self.top, width, height, true),
            resolve_inset_edge(self.right, width, height, false),
            resolve_inset_edge(self.bottom, width, height, true),
            resolve_inset_edge(self.left, width, height, false),
        ]
    }

    pub fn resolve_round(&self, width: f32, height: f32) -> f32 {
        self.round
            .map(|spec| {
                resolve_inset_edge(spec, width, height, false)
                    .min(resolve_inset_edge(spec, width, height, true))
            })
            .unwrap_or(0.0)
            .max(0.0)
    }
}

fn resolve_inset_edge(spec: LengthSpec, width: f32, height: f32, vertical: bool) -> f32 {
    let percent_base = if vertical { height } else { width };
    match spec {
        LengthSpec::Percent(p) => percent_base.max(0.0) * p / 100.0,
        LengthSpec::Px(v) => v.max(0.0),
        other => other
            .resolve_px(Some(percent_base.max(0.0)))
            .unwrap_or(0.0)
            .max(0.0),
    }
}

/// One vertex in `clip-path: polygon(...)` (percent or px against the border box).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPoint {
    pub x: LengthSpec,
    pub y: LengthSpec,
}

/// `circle()` / `ellipse()` radius: length, or CSS `closest-side` / `farthest-side`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipShapeRadius {
    ClosestSide,
    FarthestSide,
    Length(LengthSpec),
}

/// `clip-path: circle(...)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipCircle {
    pub radius: ClipShapeRadius,
    pub cx: LengthSpec,
    pub cy: LengthSpec,
}

/// `clip-path: ellipse(...)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipEllipse {
    pub rx: ClipShapeRadius,
    pub ry: ClipShapeRadius,
    pub cx: LengthSpec,
    pub cy: LengthSpec,
}

/// Parsed `clip-path` (`inset`, `polygon`, `circle`, or `ellipse`).
#[derive(Debug, Clone, PartialEq)]
pub enum ClipPath {
    Inset(ClipInset),
    Polygon(Vec<ClipPoint>),
    Circle(ClipCircle),
    Ellipse(ClipEllipse),
}

impl ClipPath {
    pub fn resolve_polygon_points(&self, width: f32, height: f32) -> Option<Vec<[f32; 2]>> {
        let ClipPath::Polygon(points) = self else {
            return None;
        };
        if points.len() < 3 {
            return None;
        }
        Some(
            points
                .iter()
                .map(|point| {
                    [
                        resolve_clip_axis(point.x, width),
                        resolve_clip_axis(point.y, height),
                    ]
                })
                .collect(),
        )
    }

    /// Local ellipse/circle bounding box `[x, y, w, h]` in the border box.
    pub fn resolve_ellipse_rect(&self, width: f32, height: f32) -> Option<[f32; 4]> {
        let (cx, cy, rx, ry) = match self {
            Self::Circle(circle) => {
                let cx = resolve_clip_axis(circle.cx, width);
                let cy = resolve_clip_axis(circle.cy, height);
                let r = circle.radius.resolve_circle(width, height, cx, cy);
                (cx, cy, r, r)
            }
            Self::Ellipse(ellipse) => {
                let cx = resolve_clip_axis(ellipse.cx, width);
                let cy = resolve_clip_axis(ellipse.cy, height);
                let rx = ellipse.radius_x(width, cx);
                let ry = ellipse.radius_y(height, cy);
                (cx, cy, rx, ry)
            }
            Self::Inset(_) | Self::Polygon(_) => return None,
        };
        if rx <= 0.0 || ry <= 0.0 {
            return None;
        }
        Some([cx - rx, cy - ry, rx * 2.0, ry * 2.0])
    }
}

impl ClipShapeRadius {
    fn resolve_circle(self, width: f32, height: f32, cx: f32, cy: f32) -> f32 {
        match self {
            Self::ClosestSide => cx.min(cy).min(width - cx).min(height - cy).max(0.0),
            Self::FarthestSide => cx.max(cy).max(width - cx).max(height - cy).max(0.0),
            Self::Length(spec) => resolve_circle_radius_len(spec, width, height),
        }
    }

    fn resolve_ellipse_axis(self, axis: f32, closest: f32, farthest: f32) -> f32 {
        match self {
            Self::ClosestSide => closest.max(0.0),
            Self::FarthestSide => farthest.max(0.0),
            Self::Length(spec) => match spec {
                LengthSpec::Percent(p) => axis.max(0.0) * p / 100.0,
                LengthSpec::Px(v) => v.max(0.0),
                other => other
                    .resolve_px(Some(axis.max(0.0)))
                    .unwrap_or(0.0)
                    .max(0.0),
            },
        }
    }
}

impl ClipEllipse {
    fn radius_x(&self, width: f32, cx: f32) -> f32 {
        self.rx
            .resolve_ellipse_axis(width, cx.min(width - cx), cx.max(width - cx))
    }

    fn radius_y(&self, height: f32, cy: f32) -> f32 {
        self.ry
            .resolve_ellipse_axis(height, cy.min(height - cy), cy.max(height - cy))
    }
}

fn resolve_circle_radius_len(spec: LengthSpec, width: f32, height: f32) -> f32 {
    let reference = ((width * width + height * height) * 0.5).sqrt().max(0.0);
    match spec {
        LengthSpec::Percent(p) => reference * p / 100.0,
        LengthSpec::Px(v) => v.max(0.0),
        other => other.resolve_px(Some(reference)).unwrap_or(0.0).max(0.0),
    }
}

fn resolve_clip_axis(spec: LengthSpec, axis: f32) -> f32 {
    match spec {
        LengthSpec::Percent(p) => axis.max(0.0) * p / 100.0,
        LengthSpec::Px(v) => v,
        other => other.resolve_px(Some(axis.max(0.0))).unwrap_or(0.0),
    }
}

/// CSS `filter: drop-shadow()` — alpha silhouette, not border-box geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterDropShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: PaintColor,
}

/// CSS `filter` brightness / saturate / contrast / grayscale / hue-rotate / invert /
/// opacity / blur / drop-shadow.
///
/// Solo `grayscale()` is encoded in [`Self::saturate`] (`grayscale(1)` → saturate 0).
/// A list that also has `saturate()` cannot share that slot and stays fail-closed.
/// `invert` / `opacity` reuse the existing color-filter uniform slots (no extra pass).
/// `sepia()` has no free slot and stays fail-closed (whole list).
/// `blur` and `drop-shadow` are the element's own filters (dest-group), distinct
/// from [`BackdropFilter`]. Exotic functions are omitted (fail closed).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorFilter {
    pub brightness: f32,
    pub saturate: f32,
    pub contrast: f32,
    /// `hue-rotate()` in degrees.
    pub hue_rotate_deg: f32,
    /// `invert()` amount in 0..=1 (packed in existing quad/layer pads).
    pub invert: f32,
    /// `opacity()` multiplier (default 1). Packed in existing quad pads;
    /// dest-group multiplies the group opacity uniform.
    pub opacity: f32,
    /// Element `blur(Npx)`, clamped at parse time. Not backdrop-filter.
    pub blur_radius: f32,
    /// Single `drop-shadow()`; extra layers and spread stay fail-closed.
    pub drop_shadow: Option<FilterDropShadow>,
}

impl Default for ColorFilter {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            saturate: 1.0,
            contrast: 1.0,
            hue_rotate_deg: 0.0,
            invert: 0.0,
            opacity: 1.0,
            blur_radius: 0.0,
            drop_shadow: None,
        }
    }
}

impl ColorFilter {
    /// Product cap for element-filter blur (tighter than backdrop frost).
    pub const MAX_BLUR_RADIUS: f32 = 16.0;
    /// Element-filter blur cap for an isolation group that opted in
    /// ([`PaintStyle::wide_filter_blur`], Issue #314). The most any filter
    /// value holds; the group it composites in applies its own cap.
    pub const MAX_WIDE_BLUR_RADIUS: f32 = 64.0;
    /// Non-identity filters one ancestor chain composites (Issue #324): a
    /// deeper one stays the identity, so a deep interface cannot keep
    /// opening filter passes. Groups with only opacity do not count.
    pub const MAX_CHAIN_DEPTH: usize = 4;

    pub fn is_identity(self) -> bool {
        (self.brightness - 1.0).abs() < 1e-5
            && (self.saturate - 1.0).abs() < 1e-5
            && (self.contrast - 1.0).abs() < 1e-5
            && self.hue_rotate_deg.abs() < 1e-5
            && self.invert.abs() < 1e-5
            && (self.opacity - 1.0).abs() < 1e-5
            && self.blur_radius <= 0.0
            && self.drop_shadow.is_none()
    }

    /// The blur cap a group compositing this filter applies: wide when it
    /// opted in ([`PaintStyle::wide_filter_blur`]).
    pub fn blur_cap(wide: bool) -> f32 {
        if wide {
            Self::MAX_WIDE_BLUR_RADIUS
        } else {
            Self::MAX_BLUR_RADIUS
        }
    }

    /// `self` with its blur and drop-shadow blur held to `cap`.
    pub fn with_blur_cap(mut self, cap: f32) -> Self {
        self.blur_radius = self.blur_radius.clamp(0.0, cap);
        if let Some(shadow) = self.drop_shadow.as_mut() {
            shadow.blur_radius = shadow.blur_radius.clamp(0.0, cap);
        }
        self
    }

    /// Dest-group composite clip pad: element blur and/or drop-shadow extent.
    pub fn dest_extent_pad(self) -> f32 {
        let drop = self
            .drop_shadow
            .map(|shadow| shadow.offset_x.abs().max(shadow.offset_y.abs()) + shadow.blur_radius)
            .unwrap_or(0.0);
        self.blur_radius.max(drop)
    }
}

/// The longest streak an open shutter draws, in logical pixels (Issue #323).
/// What lies past it is cut off.
pub const MAX_MOTION_STREAK: f32 = 64.0;

/// Per-node CSS `backdrop-filter` (dest sampling, not window material).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackdropFilter {
    /// CSS `blur(Npx)`: the gaussian's standard deviation in logical px, so
    /// the blur reaches about three times as far. Clamped at parse time.
    pub blur_radius: f32,
    /// `saturate()` multiplier (default 1).
    pub saturate: f32,
}

impl Default for BackdropFilter {
    fn default() -> Self {
        Self {
            blur_radius: 0.0,
            saturate: 1.0,
        }
    }
}

impl BackdropFilter {
    /// Product cap for frosted-glass cost control.
    pub const MAX_BLUR_RADIUS: f32 = 64.0;

    pub fn is_active(self) -> bool {
        self.blur_radius > 0.0 || (self.saturate - 1.0).abs() > 1e-5
    }
}

/// Paint-only surface properties (radii, shadow, visibility).
///
/// Layout measurement ignores this bucket except [`VisibilitySpec::Hidden`],
/// which keeps the border box but suppresses paint and hit-testing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintStyle {
    pub background_material: Option<crate::MaterialRef>,
    pub border_material: Option<crate::MaterialRef>,
    pub text_material: Option<crate::MaterialRef>,
    /// `None` inherits computed visibility from the parent.
    pub visibility: Option<VisibilitySpec>,
    /// Per-corner radii (TL, TR, BR, BL). Overrides uniform
    /// [`LayoutStyle::border_radius`] when set.
    pub border_radii: Option<[LengthSpec; 4]>,
    /// `box-shadow` layers in CSS order (first is top-most). Capped at
    /// [`MAX_BOX_SHADOWS`].
    pub box_shadows: Vec<BoxShadowSpec>,
    /// `::-webkit-scrollbar` / `::-webkit-scrollbar-thumb` skin. Geometry still
    /// comes from [`crate::scrollbar_track`]; this only overrides thickness and
    /// the default `border_strong` / `muted` / `subtle` colors.
    pub scrollbar: Option<ScrollbarSkin>,
    /// Fade a scroll container's content out toward its top and bottom
    /// edges as one composited layer ([`crate::ScrollEdgeFade`]). Read only on
    /// a box whose vertical overflow clips.
    pub scroll_edge_fade: Option<crate::ScrollEdgeFade>,
    /// Every `text-shadow` layer in CSS order (the first is drawn on top),
    /// at most [`crate::MAX_TEXT_SHADOWS`].
    pub text_shadows: Vec<TextShadowSpec>,
    /// `-webkit-text-stroke`.
    pub text_stroke: Option<TextStrokeSpec>,
    /// `paint-order` puts the stroke before the fill: the outline grows
    /// around each glyph instead of eating into it.
    pub paint_order_stroke_first: bool,
    /// Extra stroke outside the border box. Does not affect layout.
    pub outline: OutlineSpec,
    /// GPU-capable `mix-blend-mode`. Other keywords stay [`MixBlendMode::Normal`].
    pub mix_blend: MixBlendMode,
    /// This node's isolation group opts into the wide element-blur cap
    /// ([`ColorFilter::MAX_WIDE_BLUR_RADIUS`], 64 px) for its own `filter`
    /// (Issue #314). Off, the cap is [`ColorFilter::MAX_BLUR_RADIUS`]
    /// (16 px), whatever any other group chose. The one group pass is what
    /// costs more; `backdrop-filter` keeps its own cap.
    pub wide_filter_blur: bool,
    /// The shutter this node's isolation group exposes for, as a fraction of
    /// the document's frame interval (`0.0`, the default, is closed; values
    /// clamp to `0.0..=1.0`). Open, the group is streaked along the velocity
    /// its own `transform` track has at `now`, at most
    /// [`MAX_MOTION_STREAK`] long (Issue #323). Children's motion inside the
    /// group does not count, hit testing keeps the unstreaked box, and a cut
    /// has no velocity, so it leaves no streak.
    pub motion_shutter: f32,
    /// `url()` / `linear-gradient` + slice 9-slice. Other sources/repeat/width
    /// stay fail-closed and sticky; source/slice longhands must not clear it.
    pub unsupported_border_image: bool,
    /// Painted 9-slice when [`Self::unsupported_border_image`] is false.
    pub border_image: Option<BorderImageSpec>,
    /// All `background-image` layers in CSS order (index `0` is top-most).
    pub background_layers: Vec<BackgroundImage>,
    /// Last parsed `background-size` list, zipped onto a later `background-image`.
    /// Cleared by the `background` shorthand so leftover longhands do not leak.
    pub background_size_list: Vec<BackgroundImageFit>,
    /// Explicit width/height for [`BackgroundImageFit::Length`] entries, parallel
    /// to [`Self::background_size_list`].
    pub background_size_lengths: Vec<(Option<LengthSpec>, Option<LengthSpec>)>,
    /// Last parsed `background-position` list.
    pub background_position_list: Vec<BackgroundPosition>,
    /// Last parsed `background-repeat` list.
    pub background_repeat_list: Vec<BackgroundRepeat>,
    /// `<img src>` replaced content, painted above background layers.
    pub content_image: Option<BackgroundImage>,
    /// Replaced media that the CSS path will not pretend to execute (`iframe`, `video`
    /// without a poster, `<canvas>` without a HostTexture slot). Empty means
    /// the node is not an explicit skip.
    pub skipped_replaced: Option<String>,
    /// `object-fit` for [`Self::content_image`] (`None` = CSS `fill` / stretch).
    pub object_fit: Option<BackgroundImageFit>,
    /// `object-position` for [`Self::content_image`] (`None` = `50% 50%`).
    pub object_position: Option<BackgroundPosition>,
    /// `mask-image` / `-webkit-mask-image` (gradient alpha or `url()` texture).
    ///
    /// Evaluated over the node's layout box, it applies to what the node
    /// itself draws, not to its subtree:
    ///
    /// - the node's own quad (background fill; not its border or
    ///   `box-shadow`) and a HostTexture's content;
    /// - a gradient mask also to everything the node's `Painter` records —
    ///   fills, strokes, shadows, text, icons and images — in the same
    ///   node-local space. Past the box (a painted shadow or glow) it holds
    ///   its value at the box's edge instead of cutting the paint off. A
    ///   `url()` mask is not applied to painter output (counted in
    ///   `runtime.paint.mask_unsupported`).
    ///
    /// Children and the node's own text are not masked: CSS masking of a
    /// whole subtree needs it isolated into a layer, which NanaUI does not do.
    pub mask: Option<MaskImage>,
    pub clip_path: Option<ClipPath>,
    pub filter: Option<ColorFilter>,
    pub backdrop_filter: Option<BackdropFilter>,
}

impl PaintStyle {
    pub fn is_visible(&self) -> bool {
        self.visibility != Some(VisibilitySpec::Hidden)
    }

    pub fn has_advanced_paint(&self) -> bool {
        self.background_material.is_some()
            || self.border_material.is_some()
            || !self.background_layers.is_empty()
            || self.content_image.is_some()
            || self.mask.is_some()
            || self.clip_path.is_some()
            || self.filter.is_some_and(|filter| !filter.is_identity())
            || self.backdrop_filter.is_some_and(BackdropFilter::is_active)
            || !self.box_shadows.is_empty()
            || self.outline.is_active()
            || !self.mix_blend.is_normal()
            || self.border_image.is_some()
    }

    pub fn primary_box_shadow(&self) -> Option<BoxShadowSpec> {
        self.box_shadows.first().copied()
    }

    pub fn primary_text_shadow(&self) -> Option<TextShadowSpec> {
        self.text_shadows.first().copied()
    }

    pub fn primary_background_image(&self) -> Option<&BackgroundImage> {
        self.background_layers.first()
    }

    /// Background layers in CSS paint order (bottom → top), then `<img src>`.
    pub fn paint_image_layers(&self) -> impl Iterator<Item = &BackgroundImage> {
        self.background_layers
            .iter()
            .rev()
            .chain(self.content_image.as_ref())
    }

    pub fn has_dynamic_theme(&self) -> bool {
        let gradient_has_dynamic =
            |g: &CssGradient| g.stops().iter().any(|s| s.color.is_theme_dynamic());
        let bg_has_dynamic = |img: &BackgroundImage| matches!(img, BackgroundImage::Gradient(g) if gradient_has_dynamic(g));

        self.box_shadows.iter().any(|s| s.color.is_theme_dynamic())
            || self.text_shadows.iter().any(|s| s.color.is_theme_dynamic())
            || self
                .text_stroke
                .is_some_and(|s| s.color.is_some_and(PaintColor::is_theme_dynamic))
            || self
                .filter
                .and_then(|f| f.drop_shadow)
                .is_some_and(|s| s.color.is_theme_dynamic())
            || self.background_layers.iter().any(bg_has_dynamic)
            || matches!(&self.mask, Some(MaskImage::Gradient(g)) if gradient_has_dynamic(g))
    }

    pub fn resolve_theme(&mut self, model: StyleModelRef) {
        let resolve_color = |color: &mut PaintColor| {
            if color.is_theme_dynamic() {
                *color = color.resolve(model);
            }
        };
        let resolve_stops = |g: &mut CssGradient| {
            for stop in g.stops_mut() {
                resolve_color(&mut stop.color);
            }
        };
        for shadow in &mut self.box_shadows {
            resolve_color(&mut shadow.color);
        }
        for shadow in &mut self.text_shadows {
            resolve_color(&mut shadow.color);
        }
        if let Some(stroke) = &mut self.text_stroke
            && let Some(color) = &mut stroke.color
        {
            resolve_color(color);
        }
        if let Some(filter) = &mut self.filter
            && let Some(drop_shadow) = &mut filter.drop_shadow
        {
            resolve_color(&mut drop_shadow.color);
        }
        for layer in &mut self.background_layers {
            if let BackgroundImage::Gradient(g) = layer {
                resolve_stops(g);
            }
        }
        if let Some(MaskImage::Gradient(g)) = &mut self.mask {
            resolve_stops(g);
        }
    }
}

/// CSS 2D affine paint transform applied without changing layout.
///
/// The six fields use the Canvas/CSS `matrix(a, b, c, d, e, f)` convention.
/// NanaUI applies the matrix around [`LayoutStyle::transform_origin`] (default
/// CSS `50% 50%` = box center) via [`Self::around_center`] /
/// [`Self::around_origin`]. Translation is expressed in logical pixels.
///
/// 2D lists stay on this 2×3. Planar CSS 3D (`perspective()` + `rotateY`, true
/// `matrix3d`) is stored separately as [`PaintMat4`] and painted as the z=0
/// homography in the existing Scene quad pass. Parent `perspective` /
/// `preserve-3d` stay fail-closed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintTransform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

pub(super) const PAINT_MAT4_EPS: f32 = 1e-5;

/// CSS `matrix3d` 4×4 in argument/column-major order (16 floats).
///
/// Scene paints the z=0 plane as a 3×3 homography in the existing quad pass
/// (same 4 vertices, perspective divide). Not a second engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintMat4 {
    pub m: [f32; 16],
}

impl Default for PaintTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl PaintTransform {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Scales by `x` across and `y` down, about the box's transform origin.
    pub const fn scaling(x: f32, y: f32) -> Self {
        Self {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

    pub fn is_identity(self) -> bool {
        self == Self::default()
    }

    /// Concatenates a transform function on the right, preserving CSS list order.
    pub fn then(self, rhs: Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            e: self.a * rhs.e + self.c * rhs.f + self.e,
            f: self.b * rhs.e + self.d * rhs.f + self.f,
        }
    }

    /// World-space 2×3 using the CSS default origin (`50% 50%` = box center).
    pub fn around_center(self, x: f32, y: f32, width: f32, height: f32) -> [f32; 6] {
        self.around_origin(x, y, width * 0.5, height * 0.5)
    }

    /// World-space 2×3 pivoted at box-local `(origin_x, origin_y)` pixels.
    ///
    /// Same six `Copy` coefficients as [`Self::around_center`]; only the pivot
    /// changes. `origin_x` / `origin_y` are offsets from the box origin `(x, y)`.
    pub fn around_origin(self, x: f32, y: f32, origin_x: f32, origin_y: f32) -> [f32; 6] {
        let ox = x + origin_x;
        let oy = y + origin_y;
        [
            self.a,
            self.b,
            self.c,
            self.d,
            ox + self.e - self.a * ox - self.c * oy,
            oy + self.f - self.b * ox - self.d * oy,
        ]
    }
}

impl Default for PaintMat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl PaintMat4 {
    pub const IDENTITY: Self = Self {
        m: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    };

    pub fn from_affine(t: PaintTransform) -> Self {
        Self {
            m: [
                t.a, t.b, 0.0, 0.0, t.c, t.d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, t.e, t.f, 0.0, 1.0,
            ],
        }
    }

    pub fn from_matrix3d(m: [f32; 16]) -> Option<Self> {
        m.iter().all(|v| v.is_finite()).then_some(Self { m })
    }

    pub fn translation(x: f32, y: f32, z: f32) -> Self {
        let mut m = Self::IDENTITY;
        m.m[12] = x;
        m.m[13] = y;
        m.m[14] = z;
        m
    }

    pub fn scaling(x: f32, y: f32, z: f32) -> Self {
        let mut m = Self::IDENTITY;
        m.m[0] = x;
        m.m[5] = y;
        m.m[10] = z;
        m
    }

    pub fn rotate_x(angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self {
            m: [
                1.0, 0.0, 0.0, 0.0, 0.0, cos, sin, 0.0, 0.0, -sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn rotate_y(angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self {
            m: [
                cos, 0.0, -sin, 0.0, 0.0, 1.0, 0.0, 0.0, sin, 0.0, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn rotate_z(angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self {
            m: [
                cos, sin, 0.0, 0.0, -sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    /// CSS `rotate3d(x, y, z, angle)` around a (possibly unnormalized) axis.
    pub fn rotate3d(x: f32, y: f32, z: f32, angle: f32) -> Option<Self> {
        let len = (x * x + y * y + z * z).sqrt();
        if !len.is_finite() || len < PAINT_MAT4_EPS {
            return None;
        }
        let x = x / len;
        let y = y / len;
        let z = z / len;
        let (sin, cos) = angle.sin_cos();
        let c = 1.0 - cos;
        Some(Self {
            m: [
                cos + x * x * c,
                y * x * c + z * sin,
                z * x * c - y * sin,
                0.0,
                x * y * c - z * sin,
                cos + y * y * c,
                z * y * c + x * sin,
                0.0,
                x * z * c + y * sin,
                y * z * c - x * sin,
                cos + z * z * c,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ],
        })
    }

    /// CSS `perspective(d)`: `matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,-1/d, 0,0,0,1)`.
    pub fn perspective(d: f32) -> Option<Self> {
        if !d.is_finite() || d.abs() < PAINT_MAT4_EPS {
            return None;
        }
        let mut m = Self::IDENTITY;
        m.m[11] = -1.0 / d;
        Some(m)
    }

    pub fn is_identity(self) -> bool {
        self.m
            .iter()
            .zip(Self::IDENTITY.m.iter())
            .all(|(a, b)| (*a - *b).abs() <= PAINT_MAT4_EPS)
    }

    /// Concatenates `rhs` on the right (CSS list order: `self` then `rhs`).
    pub fn then(self, rhs: Self) -> Self {
        let a = self.m;
        let b = rhs.m;
        let mut m = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                m[col * 4 + row] = a[row] * b[col * 4]
                    + a[4 + row] * b[col * 4 + 1]
                    + a[8 + row] * b[col * 4 + 2]
                    + a[12 + row] * b[col * 4 + 3];
            }
        }
        Self { m }
    }

    /// `T(pivot) * self * T(-pivot)` with a 2D origin (`z = 0`).
    pub fn around_origin(self, x: f32, y: f32, origin_x: f32, origin_y: f32) -> Self {
        let ox = x + origin_x;
        let oy = y + origin_y;
        Self::translation(ox, oy, 0.0)
            .then(self)
            .then(Self::translation(-ox, -oy, 0.0))
    }

    /// Map `(x, y, 0, 1)` through this matrix and perspective-divide.
    pub fn project_xy(self, x: f32, y: f32) -> Option<[f32; 2]> {
        let m = self.m;
        let xp = m[0] * x + m[4] * y + m[12];
        let yp = m[1] * x + m[5] * y + m[13];
        let wp = m[3] * x + m[7] * y + m[15];
        if !wp.is_finite() || wp.abs() < 1e-8 {
            return None;
        }
        let sx = xp / wp;
        let sy = yp / wp;
        (sx.is_finite() && sy.is_finite()).then_some([sx, sy])
    }

    /// z=0 homography `(a,b,c,d,e,f)` plus `(g,h)` with `w = g x + h y + 1`.
    pub fn planar_homography(self) -> Option<([f32; 6], [f32; 2])> {
        let m = self.m;
        let mut a = m[0];
        let mut b = m[1];
        let mut c = m[4];
        let mut d = m[5];
        let mut e = m[12];
        let mut f = m[13];
        let mut g = m[3];
        let mut h = m[7];
        let i = m[15];
        if !i.is_finite() || i.abs() < 1e-8 {
            return None;
        }
        let inv = 1.0 / i;
        a *= inv;
        b *= inv;
        c *= inv;
        d *= inv;
        e *= inv;
        f *= inv;
        g *= inv;
        h *= inv;
        [a, b, c, d, e, f, g, h]
            .into_iter()
            .all(f32::is_finite)
            .then_some(([a, b, c, d, e, f], [g, h]))
    }

    pub fn is_planar_affine(self) -> bool {
        let Some((_, [g, h])) = self.planar_homography() else {
            return false;
        };
        g.abs() <= PAINT_MAT4_EPS && h.abs() <= PAINT_MAT4_EPS
    }

    pub fn as_affine(self) -> Option<PaintTransform> {
        if !self.is_planar_affine() {
            return None;
        }
        let ([a, b, c, d, e, f], _) = self.planar_homography()?;
        Some(PaintTransform { a, b, c, d, e, f })
    }

    /// Projected corners of a border box (TL, TR, BR, BL).
    pub fn projected_corners(
        self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<[[f32; 2]; 4]> {
        Some([
            self.project_xy(x, y)?,
            self.project_xy(x + width, y)?,
            self.project_xy(x + width, y + height)?,
            self.project_xy(x, y + height)?,
        ])
    }
}

/// CSS `transform-box` (2D origin reference).
///
/// CSS initial is `view-box`. For CSS layout boxes (HTML), `view-box` uses the
/// border box and `fill-box` uses the content box — there is no SVG viewport
/// origin space. Percent [`TransformOrigin`] is relative to this box; the
/// resulting pivot is still applied via [`PaintTransform::around_origin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransformBox {
    /// CSS initial. Used as the border box for CSS layout boxes.
    #[default]
    ViewBox,
    BorderBox,
    FillBox,
    ContentBox,
}

impl TransformBox {
    /// `content-box` / `fill-box` for HTML; `view-box` / `border-box` stay on
    /// the layout (border) box.
    pub fn uses_content_box(self) -> bool {
        matches!(self, Self::ContentBox | Self::FillBox)
    }
}

/// CSS `backface-visibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BackfaceVisibility {
    #[default]
    Visible,
    Hidden,
}

/// Parent `perspective` + `perspective-origin` resolved in ancestor layout
/// space for a direct child's 3D transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParentPerspective {
    pub distance: f32,
    pub origin: [f32; 2],
}

/// True when the z=0 projective homography `(affine, persp)` maps the center
/// `(center_x, center_y)` behind the camera (`w <= 0`) or reverses orientation
/// (`det <= 0`), i.e. the element's back face is turned toward the viewer.
pub fn is_projective_backface(
    affine: [f32; 6],
    persp: [f32; 2],
    center_x: f32,
    center_y: f32,
) -> bool {
    let [a, b, c, d, e, f] = affine;
    let [g, h] = persp;
    let w = g * center_x + h * center_y + 1.0;
    let det = (a * d - b * c) + g * (c * f - d * e) + h * (b * e - a * f);
    !w.is_finite() || !det.is_finite() || w <= 1e-6 || det <= 1e-6
}

/// CSS `transform-origin` (2D). Percent is relative to [`LayoutStyle::transform_box`].
/// A third z length is accepted by the parser and dropped on this 2×3 path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TransformOrigin {
    pub x: LengthSpec,
    pub y: LengthSpec,
}

impl Default for TransformOrigin {
    fn default() -> Self {
        Self {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Percent(50.0),
        }
    }
}

impl TransformOrigin {
    /// Resolve to box-local pixels (`50%` of a 40×20 box is `(20, 10)`).
    pub fn resolve(self, width: f32, height: f32) -> [f32; 2] {
        [
            self.x.resolve_px(Some(width)).unwrap_or(width * 0.5),
            self.y.resolve_px(Some(height)).unwrap_or(height * 0.5),
        ]
    }
}
