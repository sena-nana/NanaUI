//! 节点自绘（Issue #217）：业务侧像 Qt `paintEvent` 一样重写一个节点的外观。
//!
//! [`Painter`] 挂在 [`NodeStyle::painter`](crate::NodeStyle) 上。Runtime 在抽取
//! 节点时调用它，把 [`PaintContext`] 录下的命令解析成 [`PaintRecording`]，随
//! [`ExtractedNode::custom_paint`](crate::ExtractedNode) 进 UiScene。录制不是
//! 立即模式：同一节点在（`paint_key`、布局尺寸、主题代数）不变时直接复用上一次
//! 的录制，不再调用 `paint`。
//!
//! 颜色写 [`SemanticColorRole`]、圆角写 [`RadiusTier`]、阴影写 [`ElevationRole`]，
//! 由 `cx` 按当前主题解析，所以切换主题会自动重录。这里不暴露 GPU。

use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use nana_text::TextEngine as _;
use nana_ui_core::sort::CompactSort as _;
use std::any::TypeId;
use std::fmt;
use std::sync::Arc;

use nana_ui_core::{
    CompiledTheme, ElevationRole, Icon, RadiusTier, SemanticColorMix, SemanticColorRole,
    ThemeMetrics,
};

#[cfg(test)]
use nana_ui_core::ThemeAppearance;

use crate::{ComponentElevation, LayoutBox, TextHorizontalAlignment, TextVerticalAlignment};

/// 节点的自绘逻辑。
///
/// Rust 没有继承，这里用组合实现「重写」：`paint` 就是这个节点的 `paintEvent`。
/// 它只接管节点自己的外观（背景、边框、阴影、装饰），子节点照常布局和绘制。
///
/// 节点的渐变 `mask-image`（`LayoutStyle::paint.mask`）也作用在录下的每条
/// 命令上，和它作用在节点自己背景上一样：在节点布局盒上求值，坐标是节点内
/// 坐标。超出布局盒的部分（阴影、外发光）取盒边上的值，不会被裁掉。文字、
/// 图标和图片为此各画进一个图层。`url()` mask 不作用在自绘上；子节点也不
/// 受 mask 影响。
pub trait Painter: Send + Sync + 'static {
    /// 画在子节点下面。不调用 [`PaintContext::draw_default`] 就等于完全替换内建外观。
    fn paint(&self, cx: &mut PaintContext<'_>);

    /// 画在子节点上面（前景）。默认不画。
    fn paint_over_children(&self, _cx: &mut PaintContext<'_>) {}

    /// 自定义命中形状：`local` 是节点内坐标（原点在节点左上角），`size` 是
    /// 布局尺寸。返回 `Some(false)` 时这一点点穿到下面；返回 `None` 交给
    /// [`Self::hit_painted_outline`] 决定。只在布局矩形以内被问到。
    fn hit_test(&self, _local: [f32; 2], _size: [f32; 2]) -> Option<bool> {
        None
    }

    /// 命中区域是否等于录下的内容（填充、描边、圆角矩形、图片、文字框和
    /// `draw_default()` 所占的节点矩形，按局部变换和 `push_clip` 计算）。
    /// 默认 `false`：整个布局矩形都能命中。
    fn hit_painted_outline(&self) -> bool {
        false
    }

    /// 绘制输入的摘要：两次返回相同的值，就保证 `paint` 录出同样的命令。
    ///
    /// 布局尺寸和主题不用算进来，Runtime 已经把它们放进缓存键。
    fn paint_key(&self) -> u64;
}

/// 挂在 [`NodeStyle`](crate::NodeStyle) 上的 [`Painter`]。
struct NodePainterInner {
    type_id: TypeId,
    painter: Box<dyn Painter>,
}

///
/// 相等按（painter 类型，`paint_key`）比较：视图每帧重建出一个同键的新
/// painter 不算样式变化，不会让节点重新抽取；换了键才重录。
#[derive(Clone)]
pub struct NodePainter(Arc<NodePainterInner>);

const _: () = assert!(std::mem::size_of::<Option<NodePainter>>() == 8);

impl NodePainter {
    pub fn new<P: Painter>(painter: P) -> Self {
        Self(Arc::new(NodePainterInner {
            type_id: TypeId::of::<P>(),
            painter: Box::new(painter),
        }))
    }

    pub fn painter(&self) -> &dyn Painter {
        self.0.painter.as_ref()
    }

    pub(crate) fn cache_key(
        &self,
        size: [f32; 2],
        theme_epoch: u64,
        state: PaintState,
    ) -> PaintCacheKey {
        PaintCacheKey {
            painter: self.0.type_id,
            paint_key: self.0.painter.paint_key(),
            size: [size[0].to_bits(), size[1].to_bits()],
            theme_epoch,
            state,
            sections: 0,
        }
    }

    /// The key of a recording of this painter as a modal card's surface:
    /// the card's sections join the size, theme and state, so a header that
    /// grows re-records a surface shaped around it.
    pub(crate) fn surface_cache_key(
        &self,
        size: [f32; 2],
        theme_epoch: u64,
        sections: &crate::ModalSurfaceSections,
    ) -> PaintCacheKey {
        PaintCacheKey {
            sections: sections.digest(),
            ..self.cache_key(size, theme_epoch, PaintState::default())
        }
    }
}

impl<P: Painter> From<P> for NodePainter {
    fn from(painter: P) -> Self {
        Self::new(painter)
    }
}

impl PartialEq for NodePainter {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.type_id == other.0.type_id
                && self.0.painter.paint_key() == other.0.painter.paint_key())
    }
}

impl fmt::Debug for NodePainter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodePainter")
            .field("paint_key", &self.0.painter.paint_key())
            .finish_non_exhaustive()
    }
}

/// 录制缓存键：节点之外的那一半（节点由缓存表本身区分）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaintCacheKey {
    painter: TypeId,
    paint_key: u64,
    size: [u32; 2],
    theme_epoch: u64,
    state: PaintState,
    /// A modal surface's sections ([`crate::ModalSurfaceSections::digest`]);
    /// 0 for a node's own painter.
    sections: u64,
}

/// 节点当前的交互状态。状态变化会让节点重录，painter 不必把它算进
/// `paint_key`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct PaintState {
    /// 有指针悬停在节点上。
    pub hovered: bool,
    /// 有指针在节点上按下未抬起。
    pub pressed: bool,
    /// 节点持有可见焦点（键盘导航得到的焦点；鼠标点击得到的焦点不算）。
    pub focused: bool,
    /// 无障碍状态为禁用。
    pub disabled: bool,
    /// 无障碍状态为选中或勾选（含半选）。
    pub selected: bool,
}

/// Curve flattening tolerance in logical px. Shared with `nana-ui-scene` so
/// `PaintPath` booleans, hit-testing, and GPU tessellation agree on geometry.
pub const PATH_FLATTEN_TOLERANCE: f32 = 0.05;

/// Maximum coordinate magnitude from the node origin before a path is treated
/// as non-drawable.
pub const MAX_PATH_COORDINATE: f32 = 1.0e6;

/// Single-line metrics of a laid-out [`PaintText`], in logical px.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PaintTextLineMetrics {
    pub width: f32,
    /// Distance from the text box top to this line's baseline.
    pub baseline: f32,
    pub line_height: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// Measurement of a [`PaintText`] block, including per-line metrics and Canvas
/// `TextMetrics` ink bounding box extents, in logical px.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintTextMetrics {
    pub width: f32,
    pub height: f32,
    /// Distance from the text box top to the first line's baseline.
    pub baseline: f32,
    /// First-line font ascent above the alphabetic baseline.
    pub ascent: f32,
    /// First-line font descent below the alphabetic baseline.
    pub descent: f32,
    /// Distance left from the alignment origin to the leftmost glyph ink.
    pub actual_bounding_box_left: f32,
    /// Distance right from the alignment origin to the rightmost glyph ink.
    pub actual_bounding_box_right: f32,
    /// Distance up from the first line's baseline to the highest glyph ink.
    pub actual_bounding_box_ascent: f32,
    /// Distance down from the first line's baseline to the lowest glyph ink.
    pub actual_bounding_box_descent: f32,
    /// Per-line layout metrics, in visual line order.
    pub lines: Vec<PaintTextLineMetrics>,
}

/// 与下层内容的混合方式，同 CSS `mix-blend-mode` 与 Canvas Porter-Duff 合成模式。
pub type BlendMode = nana_ui_core::MixBlendMode;
/// Mask sampling mode for [`PaintContext::push_mask`].
pub type MaskKind = nana_ui_core::MaskKind;

/// Host text services invoked during custom paint recording.
pub(crate) trait PaintTextHost {
    fn measure_text(&self, text: &PaintText, size: f32, max_width: Option<f32>)
    -> PaintTextMetrics;
    fn text_path(
        &self,
        text: &PaintText,
        size: f32,
        origin: [f32; 2],
        max_width: Option<f32>,
    ) -> PaintPath;
}

/// Fallback text host when no `UiWorld` / `nana-text` engine is attached.
pub(crate) struct EmPaintTextHost;

impl PaintTextHost for EmPaintTextHost {
    fn measure_text(
        &self,
        text: &PaintText,
        size: f32,
        max_width: Option<f32>,
    ) -> PaintTextMetrics {
        let base = crate::ComputedStyle::default();
        let (style, constraints, content) = paint_text_request(&base, text, size, max_width);
        let metrics = crate::components::measure_em_text(&content, &style, constraints);
        paint_text_metrics_fallback(&metrics, size)
    }

    fn text_path(
        &self,
        _text: &PaintText,
        _size: f32,
        _origin: [f32; 2],
        _max_width: Option<f32>,
    ) -> PaintPath {
        PaintPath::new()
    }
}

/// 颜色：语义角色、角色混色，或者直接给定的 RGBA。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaintColor {
    Role(SemanticColorRole),
    Mix(SemanticColorMix),
    Rgba([f32; 4]),
    /// An explicit authoring-space value shared with the CSS paint resolver.
    /// It remains tagged through custom paint resolution until the scene
    /// painter uploads the final primitive.
    Authoring(nana_ui_core::PaintColor),
}

impl From<SemanticColorRole> for PaintColor {
    fn from(role: SemanticColorRole) -> Self {
        Self::Role(role)
    }
}

impl From<SemanticColorMix> for PaintColor {
    fn from(mix: SemanticColorMix) -> Self {
        Self::Mix(mix)
    }
}

impl From<[f32; 4]> for PaintColor {
    fn from(rgba: [f32; 4]) -> Self {
        Self::Rgba(rgba)
    }
}

impl From<nana_ui_core::PaintColor> for PaintColor {
    fn from(color: nana_ui_core::PaintColor) -> Self {
        Self::Authoring(color)
    }
}

/// 长度：像素，或主题圆角档位。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Radius {
    Px(f32),
    Tier(RadiusTier),
}

impl From<f32> for Radius {
    fn from(px: f32) -> Self {
        Self::Px(px)
    }
}

impl From<RadiusTier> for Radius {
    fn from(tier: RadiusTier) -> Self {
        Self::Tier(tier)
    }
}

/// 四个角的圆角，顺序为左上、右上、右下、左下。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerRadii(pub [Radius; 4]);

impl<R: Into<Radius>> From<R> for CornerRadii {
    fn from(radius: R) -> Self {
        let radius = radius.into();
        Self([radius; 4])
    }
}

impl<R: Into<Radius> + Copy> From<[R; 4]> for CornerRadii {
    fn from(radii: [R; 4]) -> Self {
        Self(radii.map(Into::into))
    }
}

/// 阴影：主题的高度档位，或者自定义参数（CSS `box-shadow` 语义，逻辑像素）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaintShadow {
    Elevation(ElevationRole),
    Custom {
        color: PaintColor,
        offset: [f32; 2],
        blur: f32,
        spread: f32,
        /// CSS `box-shadow: inset`：画在路径以内。
        inset: bool,
    },
}

impl From<ElevationRole> for PaintShadow {
    fn from(role: ElevationRole) -> Self {
        Self::Elevation(role)
    }
}

/// 填充规则，同 SVG `fill-rule` / Canvas `fill(rule)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    /// 环绕数不为 0 的区域（缺省）。
    #[default]
    NonZero,
    /// 环绕数为奇数的区域：自交处和内环挖空。
    EvenOdd,
}

/// 描边拐角的连接方式，同 Canvas `lineJoin`，缺省与 Canvas、SVG 一致为尖角。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineJoin {
    /// 尖角，超过 [`StrokeStyle::miter_limit`] 时改为斜切。
    #[default]
    Miter,
    /// 圆角。
    Round,
    /// 斜切。
    Bevel,
}

/// 开放路径端点的线帽，同 Canvas `lineCap`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    /// 平头，止于端点（缺省）。
    #[default]
    Butt,
    /// 圆头，伸出半个线宽。
    Round,
    /// 方头，伸出半个线宽。
    Square,
}

/// 描边参数，长度都在录制时的局部坐标系（见 [`PaintContext::transform`]）里。
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub join: LineJoin,
    pub cap: LineCap,
    /// Canvas `setLineDash`：实段、空段交替的长度。空表示实线；奇数个时重复
    /// 一遍凑成偶数；含负数或非有限值、或总长为 0 时按实线处理。
    pub dash: Arc<[f32]>,
    /// Canvas `lineDashOffset`：虚线从图案的这个位置开始。
    pub dash_offset: f32,
    /// Canvas `miterLimit`：尖角连接的斜接长度超过线宽的这个倍数就改为斜切。
    /// 只对 [`LineJoin::Miter`] 生效，默认 10。
    pub miter_limit: f32,
}

impl StrokeStyle {
    pub fn new(width: f32) -> Self {
        Self {
            width,
            join: LineJoin::default(),
            cap: LineCap::default(),
            dash: Arc::from(Vec::new()),
            dash_offset: 0.0,
            miter_limit: 10.0,
        }
    }

    pub fn miter_limit(mut self, limit: f32) -> Self {
        self.miter_limit = limit;
        self
    }

    /// 虚线，见 [`Self::dash`]。每一段都按 [`Self::cap`] 收尾。
    pub fn dash(mut self, pattern: impl Into<Arc<[f32]>>, offset: f32) -> Self {
        self.dash = pattern.into();
        self.dash_offset = offset;
        self
    }

    pub fn join(mut self, join: LineJoin) -> Self {
        self.join = join;
        self
    }

    pub fn cap(mut self, cap: LineCap) -> Self {
        self.cap = cap;
        self
    }
}

impl From<f32> for StrokeStyle {
    fn from(width: f32) -> Self {
        Self::new(width)
    }
}

/// 渐变的一个色标，`offset` 在 `0..=1`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorStop {
    pub offset: f32,
    pub color: PaintColor,
}

/// 渐变几何，坐标在录制时的局部坐标系里，随 [`PaintContext::transform`] 一起变换。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientShape {
    /// 沿 `start → end`，`t = 0` 在 `start`。
    Linear { start: [f32; 2], end: [f32; 2] },
    /// 以 `center` 为圆心，`t = 0` 在圆心、`t = 1` 在半径处。
    Radial { center: [f32; 2], radius: f32 },
    /// Two-circle focal radial gradient (`(x0, y0, r0)` -> `(x1, y1, r1)`).
    RadialTwoCircle {
        start: [f32; 2],
        start_radius: f32,
        end: [f32; 2],
        end_radius: f32,
    },
    /// 绕 `center` 一周，`t = 0` 在 `start_angle`（弧度，0 指向 +x，顺时针为正，y 轴向下）。
    Conic { center: [f32; 2], start_angle: f32 },
}

/// Color space in which gradient stops interpolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GradientColorSpace {
    #[default]
    PremulSrgb,
    LinearScRgb,
    Oklab,
}

/// `t` 落在 `0..=1` 之外时怎么取色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GradientExtend {
    /// 取端点色标。
    #[default]
    Pad,
    Repeat,
    Reflect,
}

/// 线性 / 径向 / 锥形渐变。
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub shape: GradientShape,
    pub stops: Vec<ColorStop>,
    pub extend: GradientExtend,
    pub transform: Affine,
    pub color_space: Option<GradientColorSpace>,
}

impl Gradient {
    pub fn linear(start: [f32; 2], end: [f32; 2]) -> Self {
        Self::new(GradientShape::Linear { start, end })
    }

    pub fn radial(center: [f32; 2], radius: f32) -> Self {
        Self::new(GradientShape::Radial { center, radius })
    }

    /// Two-circle focal radial gradient (`(x0, y0, r0)` to `(x1, y1, r1)`).
    pub fn radial_two_circle(
        start: [f32; 2],
        start_radius: f32,
        end: [f32; 2],
        end_radius: f32,
    ) -> Self {
        Self::new(GradientShape::RadialTwoCircle {
            start,
            start_radius: start_radius.max(0.0),
            end,
            end_radius: end_radius.max(0.0),
        })
    }

    /// Elliptical radial gradient centered at `center` with semi-axes `(rx, ry)`
    /// and clockwise `rotation` (radians).
    pub fn elliptical_radial(center: [f32; 2], rx: f32, ry: f32, rotation: f32) -> Self {
        let (sin, cos) = rotation.sin_cos();
        let rx = rx.max(1e-4);
        let ry = ry.max(1e-4);
        let transform = [
            cos * rx,
            sin * rx,
            -sin * ry,
            cos * ry,
            center[0],
            center[1],
        ];
        Self::radial([0.0, 0.0], 1.0).transform(transform)
    }

    pub fn conic(center: [f32; 2], start_angle: f32) -> Self {
        Self::new(GradientShape::Conic {
            center,
            start_angle,
        })
    }

    fn new(shape: GradientShape) -> Self {
        Self {
            shape,
            stops: Vec::new(),
            extend: GradientExtend::default(),
            transform: AFFINE_IDENTITY,
            color_space: None,
        }
    }

    pub fn stop(mut self, offset: f32, color: impl Into<PaintColor>) -> Self {
        self.stops.push(ColorStop {
            offset,
            color: color.into(),
        });
        self
    }

    pub fn extend(mut self, extend: GradientExtend) -> Self {
        self.extend = extend;
        self
    }

    /// Applies an affine transform in gradient space (used for elliptical radials).
    pub fn transform(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }

    /// Selects the stop interpolation color space (`PremulSrgb`, `LinearScRgb`, `Oklab`).
    pub fn color_space(mut self, color_space: GradientColorSpace) -> Self {
        self.color_space = Some(color_space);
        self
    }
}

/// Tiling repetition mode for [`PaintPattern`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PatternRepeat {
    #[default]
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

/// Image pattern paint usable for fills, strokes, and text.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintPattern {
    pub source: Arc<str>,
    pub src_rect: Option<LayoutBox>,
    pub repeat: PatternRepeat,
    pub transform: Affine,
    pub sampling: Option<nana_ui_core::ImageSampling>,
}

impl PaintPattern {
    pub fn new(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            src_rect: None,
            repeat: PatternRepeat::Repeat,
            transform: AFFINE_IDENTITY,
            sampling: None,
        }
    }

    pub fn with_src_rect(mut self, rect: LayoutBox) -> Self {
        self.src_rect = Some(rect);
        self
    }

    pub fn with_repeat(mut self, repeat: PatternRepeat) -> Self {
        self.repeat = repeat;
        self
    }

    pub fn with_transform(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }

    pub fn with_smoothing(mut self, smoothing: bool) -> Self {
        self.sampling = Some(if smoothing {
            nana_ui_core::ImageSampling::Resample
        } else {
            nana_ui_core::ImageSampling::Nearest
        });
        self
    }

    pub fn with_sampling(mut self, sampling: nana_ui_core::ImageSampling) -> Self {
        self.sampling = Some(sampling);
        self
    }
}

/// Options for drawing an image with optional source crop, nine-slice, and sampling.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintImage {
    pub source: Arc<str>,
    pub src_rect: Option<LayoutBox>,
    pub fit: ImageFit,
    pub radii: [f32; 4],
    pub sampling: Option<nana_ui_core::ImageSampling>,
    /// Optional 9-slice insets `[top, right, bottom, left]` in source pixels.
    pub nine_slice: Option<[f32; 4]>,
    pub nine_slice_fill: bool,
}

impl PaintImage {
    pub fn new(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            src_rect: None,
            fit: ImageFit::Fill,
            radii: [0.0; 4],
            sampling: None,
            nine_slice: None,
            nine_slice_fill: true,
        }
    }

    pub fn with_src_rect(mut self, rect: LayoutBox) -> Self {
        self.src_rect = Some(rect);
        self
    }

    pub fn with_fit(mut self, fit: ImageFit) -> Self {
        self.fit = fit;
        self
    }

    pub fn with_radii(mut self, radii: [f32; 4]) -> Self {
        self.radii = radii;
        self
    }

    pub fn with_smoothing(mut self, smoothing: bool) -> Self {
        self.sampling = Some(if smoothing {
            nana_ui_core::ImageSampling::Resample
        } else {
            nana_ui_core::ImageSampling::Nearest
        });
        self
    }

    pub fn with_sampling(mut self, sampling: nana_ui_core::ImageSampling) -> Self {
        self.sampling = Some(sampling);
        self
    }

    pub fn with_nine_slice(mut self, insets: [f32; 4], fill: bool) -> Self {
        self.nine_slice = Some(insets);
        self.nine_slice_fill = fill;
        self
    }
}

/// Lightweight atlas sub-rectangle handle for batched sprite draws without
/// creating retained UI nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureRegion {
    pub source: Arc<str>,
    pub rect: LayoutBox,
}

impl TextureRegion {
    pub fn new(source: impl Into<Arc<str>>, rect: LayoutBox) -> Self {
        Self {
            source: source.into(),
            rect,
        }
    }
}

/// 填充或描边用什么上色：纯色、渐变或纹理图案。
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Color(PaintColor),
    Gradient(Gradient),
    Pattern(PaintPattern),
}

impl<C: Into<PaintColor>> From<C> for Paint {
    fn from(color: C) -> Self {
        Self::Color(color.into())
    }
}

impl From<Gradient> for Paint {
    fn from(gradient: Gradient) -> Self {
        Self::Gradient(gradient)
    }
}

impl From<PaintPattern> for Paint {
    fn from(pattern: PaintPattern) -> Self {
        Self::Pattern(pattern)
    }
}

/// 按主题解析后的渐变：色标已排序并夹到 `0..=1`，颜色为 sRGB RGBA。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGradient {
    pub shape: GradientShape,
    pub stops: Vec<(f32, [f32; 4])>,
    /// Linear scRGB stops when at least one stop was supplied in an explicit
    /// authoring space or when `color_space` is `LinearScRgb` / `Oklab`.
    pub linear_stops: Option<Vec<(f32, [f32; 4])>>,
    pub extend: GradientExtend,
    pub transform: Affine,
    pub color_space: GradientColorSpace,
}

/// Resolved pattern paint for scene and GPU rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPattern {
    pub source: Arc<str>,
    pub src_rect: Option<LayoutBox>,
    pub repeat: PatternRepeat,
    pub transform: Affine,
    pub sampling: nana_ui_core::ImageSampling,
}

/// 按主题解析后的上色。
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedPaint {
    Solid([f32; 4]),
    /// An explicit authoring-space solid retained until the scene painter.
    Authoring(nana_ui_core::PaintColor),
    Gradient(Arc<ResolvedGradient>),
    Pattern(Arc<ResolvedPattern>),
}

impl ResolvedPaint {
    /// 画出来是否完全透明。
    pub fn is_invisible(&self) -> bool {
        match self {
            Self::Solid(color) => color[3] <= 0.0,
            Self::Authoring(color) => color.to_linear_sc_rgb().1 <= 0.0,
            Self::Gradient(gradient) => gradient.stops.iter().all(|(_, color)| color[3] <= 0.0),
            Self::Pattern(pattern) => pattern.source.is_empty(),
        }
    }
}

/// 图片在目标矩形里怎么摆，同 CSS `object-fit`，居中对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFit {
    /// 拉伸铺满，不保持宽高比。
    Fill,
    /// 完整放进矩形，保持宽高比。
    #[default]
    Contain,
    /// 铺满矩形，保持宽高比，超出部分裁掉。
    Cover,
    /// 原始尺寸。
    None,
    /// `None` 与 `Contain` 中较小的一个。
    ScaleDown,
}

/// 2D 仿射变换 `[a, b, c, d, e, f]`，同 Canvas `setTransform`：
/// `x' = a·x + c·y + e`，`y' = b·x + d·y + f`。
pub type Affine = [f32; 6];

/// 恒等变换。
pub const AFFINE_IDENTITY: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `outer ∘ inner`：先 `inner` 再 `outer`。
pub fn affine_multiply(outer: Affine, inner: Affine) -> Affine {
    let [a, b, c, d, e, f] = outer;
    let [ia, ib, ic, id, ie, iff] = inner;
    [
        a * ia + c * ib,
        b * ia + d * ib,
        a * ic + c * id,
        b * ic + d * id,
        a * ie + c * iff + e,
        b * ie + d * iff + f,
    ]
}

/// 路径命令。圆弧在构建时已经换成三次贝塞尔。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathVerb {
    MoveTo([f32; 2]),
    LineTo([f32; 2]),
    QuadTo([f32; 2], [f32; 2]),
    CubicTo([f32; 2], [f32; 2], [f32; 2]),
    Close,
}

/// Boolean operation between two [`PaintPath`] shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathBooleanOp {
    Union,
    Intersect,
    Difference,
    Xor,
}

/// 节点内坐标系下的路径（原点是节点左上角，单位逻辑像素）。
///
/// 构建方式与 Canvas / `QPainterPath` 相同。[`Self::arc_to`] 用来给任意拐角倒圆，
/// 凹角也一样。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintPath {
    verbs: Vec<PathVerb>,
    fill_rule: FillRule,
    start: Option<[f32; 2]>,
    current: Option<[f32; 2]>,
    /// 整条路径只是一个 [`Self::rect`] / [`Self::rounded_rect`] 时的矩形与四角半径。
    shape_hint: Option<(LayoutBox, [f32; 4])>,
}

impl PaintPath {
    /// 空路径，填充规则为 [`FillRule::NonZero`]。
    pub fn new() -> Self {
        Self::default()
    }

    /// 录下的路径命令。
    pub fn verbs(&self) -> &[PathVerb] {
        &self.verbs
    }

    /// 填充规则。
    pub fn fill_rule(&self) -> FillRule {
        self.fill_rule
    }

    /// 是否一条命令也没有。
    pub fn is_empty(&self) -> bool {
        self.verbs.is_empty()
    }

    /// 换一种填充规则，按值链式使用：`PaintPath::new().with_fill_rule(..)`。
    pub fn with_fill_rule(mut self, rule: FillRule) -> Self {
        self.fill_rule = rule;
        self
    }

    /// 换一种填充规则，与其他构建方法一样可在 `&mut` 链上调用。
    pub fn set_fill_rule(&mut self, rule: FillRule) -> &mut Self {
        self.fill_rule = rule;
        self
    }

    /// 点是否落在路径按填充规则围成的区域里，同 Canvas `isPointInPath`。
    pub fn contains(&self, point: [f32; 2]) -> bool {
        path_contains(self, AFFINE_IDENTITY, point)
    }

    /// 点是否落在按 `stroke` 描出的线上（含虚线的空段判断），同 Canvas
    /// `isPointInStroke`；线帽和连接按圆形算。
    pub fn stroke_contains(&self, point: [f32; 2], stroke: &StrokeStyle) -> bool {
        stroke_contains(self, stroke, AFFINE_IDENTITY, point)
    }

    /// Boolean operation (`union` / `intersect` / `difference` / `xor`) between
    /// `self` and `other`, interpreting each input according to its own
    /// [`FillRule`]. Curves are flattened at [`PATH_FLATTEN_TOLERANCE`] before
    /// the boolean, returning a polyline `PaintPath` usable for fill, stroke,
    /// clip, hit-testing, and further booleans.
    pub fn boolean(&self, other: &Self, op: PathBooleanOp) -> Self {
        let a = path_to_overlay_shapes(self);
        let b = path_to_overlay_shapes(other);
        let shapes = match op {
            PathBooleanOp::Union => {
                if a.is_empty() {
                    b
                } else if b.is_empty() {
                    a
                } else {
                    a.overlay(&b, OverlayRule::Union, OverlayFill::NonZero)
                }
            }
            PathBooleanOp::Intersect => {
                if a.is_empty() || b.is_empty() {
                    Vec::new()
                } else {
                    a.overlay(&b, OverlayRule::Intersect, OverlayFill::NonZero)
                }
            }
            PathBooleanOp::Difference => {
                if a.is_empty() {
                    Vec::new()
                } else if b.is_empty() {
                    a
                } else {
                    a.overlay(&b, OverlayRule::Difference, OverlayFill::NonZero)
                }
            }
            PathBooleanOp::Xor => {
                if a.is_empty() {
                    b
                } else if b.is_empty() {
                    a
                } else {
                    a.overlay(&b, OverlayRule::Xor, OverlayFill::NonZero)
                }
            }
        };
        overlay_shapes_to_path(&shapes)
    }

    /// Boolean union (`self ∪ other`).
    pub fn union(&self, other: &Self) -> Self {
        self.boolean(other, PathBooleanOp::Union)
    }

    /// Boolean intersection (`self ∩ other`).
    pub fn intersect(&self, other: &Self) -> Self {
        self.boolean(other, PathBooleanOp::Intersect)
    }

    /// Boolean difference (`self \ other`).
    pub fn difference(&self, other: &Self) -> Self {
        self.boolean(other, PathBooleanOp::Difference)
    }

    /// Boolean symmetric difference (`self ⊕ other`).
    pub fn xor(&self, other: &Self) -> Self {
        self.boolean(other, PathBooleanOp::Xor)
    }

    /// Converts a [`PaintText`] to glyph outline paths via `engine`, placed at
    /// `origin` with optional wrapping width `max_width`.
    pub fn from_text(
        engine: &nana_text::SharedTextEngine,
        text: &PaintText,
        origin: [f32; 2],
        max_width: Option<f32>,
    ) -> Self {
        let base = crate::ComputedStyle::default();
        let size = text.size.unwrap_or(nana_ui_core::UI_BASE_TEXT_SIZE);
        paint_text_to_path(engine, &base, text, size, origin, max_width)
    }

    /// 所有点（含曲线控制点）的外接矩形；空路径为 `None`。曲线本身不会超出它。
    pub fn bounds(&self) -> Option<LayoutBox> {
        let mut points = self.verbs.iter().flat_map(|verb| match *verb {
            PathVerb::MoveTo(p) | PathVerb::LineTo(p) => vec![p],
            PathVerb::QuadTo(c, p) => vec![c, p],
            PathVerb::CubicTo(a, b, p) => vec![a, b, p],
            PathVerb::Close => Vec::new(),
        });
        let first = points.next()?;
        let (min, max) = points.fold((first, first), |(min, max), p| {
            (
                [min[0].min(p[0]), min[1].min(p[1])],
                [max[0].max(p[0]), max[1].max(p[1])],
            )
        });
        Some(LayoutBox {
            x: min[0],
            y: min[1],
            width: max[0] - min[0],
            height: max[1] - min[1],
        })
    }

    /// 经 `transform` 变换后的新路径，填充规则不变。
    pub fn transformed(&self, transform: Affine) -> Self {
        let [a, b, c, d, e, f] = transform;
        let map = |p: [f32; 2]| [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f];
        Self {
            verbs: self
                .verbs
                .iter()
                .map(|verb| match *verb {
                    PathVerb::MoveTo(p) => PathVerb::MoveTo(map(p)),
                    PathVerb::LineTo(p) => PathVerb::LineTo(map(p)),
                    PathVerb::QuadTo(c, p) => PathVerb::QuadTo(map(c), map(p)),
                    PathVerb::CubicTo(x, y, p) => PathVerb::CubicTo(map(x), map(y), map(p)),
                    PathVerb::Close => PathVerb::Close,
                })
                .collect(),
            fill_rule: self.fill_rule,
            start: self.start.map(map),
            current: self.current.map(map),
            shape_hint: None,
        }
    }

    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.shape_hint = None;
        self.verbs.push(PathVerb::MoveTo([x, y]));
        self.start = Some([x, y]);
        self.current = Some([x, y]);
        self
    }

    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        if self.current.is_none() {
            return self.move_to(x, y);
        }
        self.shape_hint = None;
        self.verbs.push(PathVerb::LineTo([x, y]));
        self.current = Some([x, y]);
        self
    }

    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Self {
        if self.current.is_none() {
            self.move_to(cx, cy);
        }
        self.shape_hint = None;
        self.verbs.push(PathVerb::QuadTo([cx, cy], [x, y]));
        self.current = Some([x, y]);
        self
    }

    /// 三次贝塞尔曲线到 `(x, y)`，控制点 `(c1x, c1y)`、`(c2x, c2y)`，同 Canvas
    /// `bezierCurveTo`。
    pub fn cubic_to(
        &mut self,
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    ) -> &mut Self {
        self.cubic([c1x, c1y], [c2x, c2y], [x, y])
    }

    fn cubic(&mut self, c1: [f32; 2], c2: [f32; 2], to: [f32; 2]) -> &mut Self {
        if self.current.is_none() {
            self.move_to(c1[0], c1[1]);
        }
        self.shape_hint = None;
        self.verbs.push(PathVerb::CubicTo(c1, c2, to));
        self.current = Some(to);
        self
    }

    pub fn close(&mut self) -> &mut Self {
        if self.current.is_some() {
            self.shape_hint = None;
            self.verbs.push(PathVerb::Close);
            self.current = self.start;
        }
        self
    }

    /// Canvas `arcTo(x1, y1, x2, y2, radius)`：从当前点朝拐角 `(x1, y1)` 走，
    /// 在拐角处以 `radius` 倒圆后转向 `(x2, y2)`，停在第二条边的切点上。凸角、
    /// 凹角一视同仁。
    ///
    /// 半径超过两条边能容下的长度时自动缩小；三点共线时退化成直线到拐角。
    pub fn arc_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, radius: f32) -> &mut Self {
        self.corner_arc([x1, y1], [x2, y2], radius)
    }

    fn corner_arc(&mut self, corner: [f32; 2], to: [f32; 2], radius: f32) -> &mut Self {
        let Some(from) = self.current else {
            return self.move_to(corner[0], corner[1]);
        };
        let v1 = sub(from, corner);
        let v2 = sub(to, corner);
        let (l1, l2) = (len(v1), len(v2));
        if radius <= 0.0 || l1 < 1e-6 || l2 < 1e-6 {
            return self.line_to(corner[0], corner[1]);
        }
        let (u1, u2) = (scale(v1, 1.0 / l1), scale(v2, 1.0 / l2));
        let cos = (u1[0] * u2[0] + u1[1] * u2[1]).clamp(-1.0, 1.0);
        let theta = cos.acos();
        if theta < 1e-4 || (std::f32::consts::PI - theta) < 1e-4 {
            return self.line_to(corner[0], corner[1]);
        }
        let half = theta * 0.5;
        let mut distance = radius / half.tan();
        let mut radius = radius;
        let limit = l1.min(l2);
        if distance > limit {
            distance = limit;
            radius = distance * half.tan();
        }
        let t1 = add(corner, scale(u1, distance));
        let t2 = add(corner, scale(u2, distance));
        let bisector = add(u1, u2);
        let center = add(
            corner,
            scale(bisector, (radius / half.sin()) / len(bisector)),
        );
        let start = (t1[1] - center[1]).atan2(t1[0] - center[0]);
        let end = (t2[1] - center[1]).atan2(t2[0] - center[0]);
        let mut sweep = end - start;
        if sweep > std::f32::consts::PI {
            sweep -= std::f32::consts::TAU;
        } else if sweep < -std::f32::consts::PI {
            sweep += std::f32::consts::TAU;
        }
        self.line_to(t1[0], t1[1]);
        self.arc_segments(center, radius, start, sweep);
        self
    }

    /// 以 `(cx, cy)` 为圆心的圆弧，角度为弧度，`sweep` 是扫过的角度（与
    /// `QPainterPath::arcTo` 一样，不是 Canvas 的终止角），为正时顺时针（y 轴
    /// 向下），超过一整圈按一整圈画。已有当前点时先连一条直线到弧的起点。
    pub fn arc(&mut self, cx: f32, cy: f32, radius: f32, start: f32, sweep: f32) -> &mut Self {
        let center = [cx, cy];
        let begin = [
            center[0] + radius * start.cos(),
            center[1] + radius * start.sin(),
        ];
        if self.current.is_some() {
            self.line_to(begin[0], begin[1]);
        } else {
            self.move_to(begin[0], begin[1]);
        }
        self.arc_segments(center, radius, start, sweep);
        self
    }

    fn arc_segments(&mut self, center: [f32; 2], radius: f32, start: f32, sweep: f32) {
        // Canvas clamps a sweep past a full turn; a non-finite one draws nothing.
        if !sweep.is_finite() || !start.is_finite() || !radius.is_finite() {
            return;
        }
        let sweep = sweep.clamp(-std::f32::consts::TAU, std::f32::consts::TAU);
        let count = (sweep.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
        let step = sweep / count as f32;
        let k = 4.0 / 3.0 * (step / 4.0).tan() * radius;
        let mut angle = start;
        for _ in 0..count {
            let next = angle + step;
            let (s0, c0) = angle.sin_cos();
            let (s1, c1) = next.sin_cos();
            let p0 = [center[0] + radius * c0, center[1] + radius * s0];
            let p3 = [center[0] + radius * c1, center[1] + radius * s1];
            self.cubic(
                [p0[0] - k * s0, p0[1] + k * c0],
                [p3[0] + k * s1, p3[1] - k * c1],
                p3,
            );
            angle = next;
        }
    }

    /// 矩形子路径。
    pub fn rect(&mut self, rect: LayoutBox) -> &mut Self {
        let alone = self.verbs.is_empty();
        self.move_to(rect.x, rect.y)
            .line_to(rect.x + rect.width, rect.y)
            .line_to(rect.x + rect.width, rect.y + rect.height)
            .line_to(rect.x, rect.y + rect.height)
            .close();
        self.shape_hint = alone.then_some((rect, [0.0; 4]));
        self
    }

    /// 四角独立的圆角矩形子路径，半径按左上、右上、右下、左下，超出时按 CSS
    /// 规则等比缩小。
    pub fn rounded_rect(&mut self, rect: LayoutBox, radii: [f32; 4]) -> &mut Self {
        let alone = self.verbs.is_empty();
        let fitted = fit_radii(rect, radii);
        let [tl, tr, br, bl] = fitted;
        let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);
        self.move_to(x + tl, y).line_to(x + w - tr, y);
        if tr > 0.0 {
            self.corner_arc([x + w, y], [x + w, y + h], tr);
        }
        self.line_to(x + w, y + h - br);
        if br > 0.0 {
            self.corner_arc([x + w, y + h], [x, y + h], br);
        }
        self.line_to(x + bl, y + h);
        if bl > 0.0 {
            self.corner_arc([x, y + h], [x, y], bl);
        }
        self.line_to(x, y + tl);
        if tl > 0.0 {
            self.corner_arc([x, y], [x + w, y], tl);
        }
        self.close();
        self.shape_hint = alone.then_some((rect, fitted));
        self
    }

    /// 内切于 `rect` 的椭圆子路径。
    pub fn ellipse(&mut self, rect: LayoutBox) -> &mut Self {
        let (rx, ry) = (rect.width * 0.5, rect.height * 0.5);
        let (cx, cy) = (rect.x + rx, rect.y + ry);
        let k = 0.552_284_8;
        self.move_to(cx + rx, cy)
            .cubic(
                [cx + rx, cy + ry * k],
                [cx + rx * k, cy + ry],
                [cx, cy + ry],
            )
            .cubic(
                [cx - rx * k, cy + ry],
                [cx - rx, cy + ry * k],
                [cx - rx, cy],
            )
            .cubic(
                [cx - rx, cy - ry * k],
                [cx - rx * k, cy - ry],
                [cx, cy - ry],
            )
            .cubic(
                [cx + rx * k, cy - ry],
                [cx + rx, cy - ry * k],
                [cx + rx, cy],
            )
            .close()
    }

    /// 这条路径若只是一个轴对齐的（圆角）矩形，返回它和四角半径（左上、右上、
    /// 右下、左下，已按 CSS 规则缩到放得下）。
    ///
    /// Scene 用它把这样的填充和描边交给矩形图元画，不必三角化。
    pub fn as_rounded_rect_corners(&self) -> Option<(LayoutBox, [f32; 4])> {
        self.shape_hint
    }

    /// 这条路径若只是一个轴对齐的（圆角）矩形，返回它和统一的圆角半径。
    ///
    /// Scene 用它把裁剪精确地交给 GPU 裁剪区，而不是退回外接矩形。
    pub fn as_rounded_rect(&self) -> Option<(LayoutBox, f32)> {
        self.shape_hint
            .as_ref()
            .copied()
            .filter(|(_, radii)| radii.iter().all(|r| (r - radii[0]).abs() < 1e-4))
            .map(|(rect, radii)| (rect, radii[0]))
    }

    /// 按 SVG path 数据（`M L H V C S Q T A Z`，大小写均可）建路径，弧转成
    /// 三次曲线；数据不合法时返回原因。
    pub fn from_svg(data: &str) -> Result<Self, String> {
        let mut tokens = SvgTokens::new(data);
        let mut path = Self::new();
        let mut current = [0.0f32; 2];
        let mut start = [0.0f32; 2];
        // The last control point, for S and T reflections.
        let mut last_cubic: Option<[f32; 2]> = None;
        let mut last_quad: Option<[f32; 2]> = None;
        let mut command = None;
        while let Some(next) = tokens.next_command_or_number()? {
            let letter = match next {
                SvgToken::Command(letter) => {
                    command = Some(letter);
                    letter
                }
                SvgToken::Number => {
                    tokens.unread();
                    match command {
                        // Coordinates after a moveto are implicit linetos.
                        Some('M') => 'L',
                        Some('m') => 'l',
                        Some('Z' | 'z') => return Err("number after closepath".into()),
                        Some(letter) => letter,
                        None => return Err("path data starts with a number".into()),
                    }
                }
            };
            let relative = letter.is_ascii_lowercase();
            let base = if relative { current } else { [0.0, 0.0] };
            let read_point = |tokens: &mut SvgTokens| -> Result<[f32; 2], String> {
                Ok([tokens.number()? + base[0], tokens.number()? + base[1]])
            };
            match letter.to_ascii_uppercase() {
                'M' => {
                    let p = read_point(&mut tokens)?;
                    path.move_to(p[0], p[1]);
                    current = p;
                    start = p;
                    last_cubic = None;
                    last_quad = None;
                }
                'L' => {
                    let p = read_point(&mut tokens)?;
                    path.line_to(p[0], p[1]);
                    current = p;
                    last_cubic = None;
                    last_quad = None;
                }
                'H' => {
                    let x = tokens.number()? + if relative { current[0] } else { 0.0 };
                    current = [x, current[1]];
                    path.line_to(current[0], current[1]);
                    last_cubic = None;
                    last_quad = None;
                }
                'V' => {
                    let y = tokens.number()? + if relative { current[1] } else { 0.0 };
                    current = [current[0], y];
                    path.line_to(current[0], current[1]);
                    last_cubic = None;
                    last_quad = None;
                }
                'C' | 'S' => {
                    let c1 = if letter.eq_ignore_ascii_case(&'S') {
                        last_cubic.map_or(current, |c| {
                            [2.0 * current[0] - c[0], 2.0 * current[1] - c[1]]
                        })
                    } else {
                        read_point(&mut tokens)?
                    };
                    let c2 = read_point(&mut tokens)?;
                    let p = read_point(&mut tokens)?;
                    path.cubic_to(c1[0], c1[1], c2[0], c2[1], p[0], p[1]);
                    current = p;
                    last_cubic = Some(c2);
                    last_quad = None;
                }
                'Q' | 'T' => {
                    let c = if letter.eq_ignore_ascii_case(&'T') {
                        last_quad.map_or(current, |c| {
                            [2.0 * current[0] - c[0], 2.0 * current[1] - c[1]]
                        })
                    } else {
                        read_point(&mut tokens)?
                    };
                    let p = read_point(&mut tokens)?;
                    path.quad_to(c[0], c[1], p[0], p[1]);
                    current = p;
                    last_quad = Some(c);
                    last_cubic = None;
                }
                'A' => {
                    let rx = tokens.number()?.abs();
                    let ry = tokens.number()?.abs();
                    let rotation = tokens.number()?.to_radians();
                    let large = tokens.flag()?;
                    let sweep = tokens.flag()?;
                    let p = read_point(&mut tokens)?;
                    for cubic in svg_arc(current, p, rx, ry, rotation, large, sweep) {
                        path.cubic_to(cubic[0], cubic[1], cubic[2], cubic[3], cubic[4], cubic[5]);
                    }
                    current = p;
                    last_cubic = None;
                    last_quad = None;
                }
                'Z' => {
                    path.close();
                    current = start;
                    last_cubic = None;
                    last_quad = None;
                }
                other => return Err(format!("unknown path command `{other}`")),
            }
        }
        Ok(path)
    }
}

enum SvgToken {
    Command(char),
    /// A number where a command letter could have been: the previous
    /// command repeats. Read again as a coordinate after `unread`.
    Number,
}

struct SvgTokens<'a> {
    text: &'a [u8],
    at: usize,
    last: usize,
}

impl<'a> SvgTokens<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text: text.as_bytes(),
            at: 0,
            last: 0,
        }
    }

    fn skip_separators(&mut self) {
        while self.at < self.text.len()
            && (self.text[self.at].is_ascii_whitespace() || self.text[self.at] == b',')
        {
            self.at += 1;
        }
    }

    fn next_command_or_number(&mut self) -> Result<Option<SvgToken>, String> {
        self.skip_separators();
        self.last = self.at;
        let Some(&byte) = self.text.get(self.at) else {
            return Ok(None);
        };
        if byte.is_ascii_alphabetic() && byte != b'e' && byte != b'E' {
            self.at += 1;
            return Ok(Some(SvgToken::Command(byte as char)));
        }
        self.number().map(|_| Some(SvgToken::Number))
    }

    fn unread(&mut self) {
        self.at = self.last;
    }

    fn number(&mut self) -> Result<f32, String> {
        self.skip_separators();
        let begin = self.at;
        let mut end = self.at;
        let bytes = self.text;
        if matches!(bytes.get(end), Some(b'+' | b'-')) {
            end += 1;
        }
        let mut seen_dot = false;
        while let Some(&byte) = bytes.get(end) {
            if byte.is_ascii_digit() {
                end += 1;
            } else if byte == b'.' && !seen_dot {
                seen_dot = true;
                end += 1;
            } else if (byte == b'e' || byte == b'E')
                && bytes
                    .get(end + 1)
                    .is_some_and(|next| next.is_ascii_digit() || *next == b'-' || *next == b'+')
            {
                end += 2;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
                break;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&bytes[begin..end]).unwrap_or_default();
        let value = text
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("expected a finite number at byte {begin}"))?;
        self.at = end;
        Ok(value)
    }

    /// An arc flag: a single `0` or `1`, which may run into the next number.
    fn flag(&mut self) -> Result<bool, String> {
        self.skip_separators();
        match self.text.get(self.at) {
            Some(b'0') => {
                self.at += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.at += 1;
                Ok(true)
            }
            _ => Err(format!("expected an arc flag at byte {}", self.at)),
        }
    }
}

/// An SVG elliptical arc from `from` to `to`, as cubic segments
/// `[c1x, c1y, c2x, c2y, x, y]` (SVG 1.1 appendix F.6).
fn svg_arc(
    from: [f32; 2],
    to: [f32; 2],
    mut rx: f32,
    mut ry: f32,
    rotation: f32,
    large: bool,
    sweep: bool,
) -> Vec<[f32; 6]> {
    if rx < 1e-6 || ry < 1e-6 || (from[0] - to[0]).abs() + (from[1] - to[1]).abs() < 1e-6 {
        return vec![[from[0], from[1], to[0], to[1], to[0], to[1]]];
    }
    let (sin, cos) = rotation.sin_cos();
    let dx = (from[0] - to[0]) / 2.0;
    let dy = (from[1] - to[1]) / 2.0;
    let x1 = cos * dx + sin * dy;
    let y1 = -sin * dx + cos * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }
    let numerator = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = (numerator / denominator.max(1e-12)).sqrt();
    if large == sweep {
        k = -k;
    }
    let cx1 = k * rx * y1 / ry;
    let cy1 = -k * ry * x1 / rx;
    let cx = cos * cx1 - sin * cy1 + (from[0] + to[0]) / 2.0;
    let cy = sin * cx1 + cos * cy1 + (from[1] + to[1]) / 2.0;
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let mut a = (dot / len.max(1e-12)).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let start = angle(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut delta = angle(
        (x1 - cx1) / rx,
        (y1 - cy1) / ry,
        (-x1 - cx1) / rx,
        (-y1 - cy1) / ry,
    );
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let count = (delta.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = delta / count as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let map = |x: f32, y: f32| {
        [
            cos * rx * x - sin * ry * y + cx,
            sin * rx * x + cos * ry * y + cy,
        ]
    };
    let mut out = Vec::with_capacity(count);
    let mut a = start;
    for _ in 0..count {
        let b = a + step;
        let (s0, c0) = a.sin_cos();
        let (s1, c1) = b.sin_cos();
        let p1 = map(c0 - k * s0, s0 + k * c0);
        let p2 = map(c1 + k * s1, s1 - k * c1);
        let p3 = map(c1, s1);
        out.push([p1[0], p1[1], p2[0], p2[1], p3[0], p3[1]]);
        a = b;
    }
    // Land exactly on the endpoint.
    if let Some(last) = out.last_mut() {
        last[4] = to[0];
        last[5] = to[1];
    }
    out
}

fn fit_radii(rect: LayoutBox, radii: [f32; 4]) -> [f32; 4] {
    let radii = radii.map(|r| if r.is_finite() { r.max(0.0) } else { 0.0 });
    let [tl, tr, br, bl] = radii;
    let mut factor = 1.0f32;
    for (sum, side) in [
        (tl + tr, rect.width),
        (tr + br, rect.height),
        (br + bl, rect.width),
        (bl + tl, rect.height),
    ] {
        if sum > side.max(0.0) && sum > 0.0 {
            factor = factor.min(side.max(0.0) / sum);
        }
    }
    radii.map(|r| r * factor)
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn scale(a: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] * s, a[1] * s]
}

fn len(a: [f32; 2]) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

/// 圆角矩形的外观：填充、描边、阴影都可选。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoxPaint {
    pub fill: Option<Paint>,
    /// 颜色与宽度（逻辑像素），描在矩形内侧。
    pub border: Option<(PaintColor, f32)>,
    pub shadow: Option<PaintShadow>,
}

impl BoxPaint {
    pub fn fill(paint: impl Into<Paint>) -> Self {
        Self {
            fill: Some(paint.into()),
            ..Self::default()
        }
    }

    pub fn border(mut self, color: impl Into<PaintColor>, width: f32) -> Self {
        self.border = Some((color.into(), width));
        self
    }

    pub fn shadow(mut self, shadow: impl Into<PaintShadow>) -> Self {
        self.shadow = Some(shadow.into());
        self
    }
}

/// Decoration lines for [`PaintText`] (underline, line-through, overline).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PaintTextDecoration {
    pub underline: bool,
    pub line_through: bool,
    pub overline: bool,
    pub style: nana_ui_core::TextDecorationStyle,
    pub color: Option<PaintColor>,
    pub thickness: Option<f32>,
}

impl PaintTextDecoration {
    pub fn underline() -> Self {
        Self {
            underline: true,
            ..Self::default()
        }
    }

    pub fn line_through() -> Self {
        Self {
            line_through: true,
            ..Self::default()
        }
    }

    pub fn overline() -> Self {
        Self {
            overline: true,
            ..Self::default()
        }
    }

    pub fn with_underline(mut self, underline: bool) -> Self {
        self.underline = underline;
        self
    }

    pub fn with_line_through(mut self, line_through: bool) -> Self {
        self.line_through = line_through;
        self
    }

    pub fn with_overline(mut self, overline: bool) -> Self {
        self.overline = overline;
        self
    }

    pub fn style(mut self, style: nana_ui_core::TextDecorationStyle) -> Self {
        self.style = style;
        self
    }

    pub fn color(mut self, color: impl Into<PaintColor>) -> Self {
        self.color = Some(color.into());
        self
    }

    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = Some(thickness);
        self
    }

    pub fn is_active(&self) -> bool {
        self.underline || self.line_through || self.overline
    }
}

/// Outline stroke for [`PaintText`].
#[derive(Debug, Clone, PartialEq)]
pub struct PaintTextStroke {
    pub width: f32,
    pub paint: Paint,
    pub join: nana_ui_core::TextStrokeJoin,
    pub placement: nana_ui_core::TextStrokePlacement,
}

impl PaintTextStroke {
    pub fn new(width: f32, paint: impl Into<Paint>) -> Self {
        Self {
            width,
            paint: paint.into(),
            join: nana_ui_core::TextStrokeJoin::Round,
            placement: nana_ui_core::TextStrokePlacement::Under,
        }
    }

    pub fn join(mut self, join: nana_ui_core::TextStrokeJoin) -> Self {
        self.join = join;
        self
    }

    pub fn placement(mut self, placement: nana_ui_core::TextStrokePlacement) -> Self {
        self.placement = placement;
        self
    }
}

/// Shadow layer for [`PaintText`] (CSS `text-shadow` semantics).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintTextShadow {
    pub offset: [f32; 2],
    pub blur: f32,
    pub spread: f32,
    pub color: PaintColor,
}

impl PaintTextShadow {
    pub fn new(offset: [f32; 2], blur: f32, color: impl Into<PaintColor>) -> Self {
        Self {
            offset,
            blur,
            spread: 0.0,
            color: color.into(),
        }
    }

    pub fn spread(mut self, spread: f32) -> Self {
        self.spread = spread;
        self
    }
}

/// 一段文字。字号缺省为主题正文字号。
#[derive(Debug, Clone, PartialEq)]
pub struct PaintText {
    pub content: Arc<str>,
    /// 纯色或渐变。
    pub paint: Paint,
    pub size: Option<f32>,
    pub weight: Option<u16>,
    /// 字体族（CSS `font-family` 列表）；`None` 沿用节点自己的字体。
    pub family: Option<Arc<str>>,
    pub italic: bool,
    /// 行高，逻辑像素；`None` 用字体默认行高。
    pub line_height: Option<f32>,
    /// Additional tracking between glyphs in logical px (default `0.0`).
    pub letter_spacing: f32,
    /// 超出宽度时折行；否则单行，超长省略。
    pub wrap: bool,
    /// 最多几行，超出省略。
    pub max_lines: Option<u16>,
    pub horizontal: TextHorizontalAlignment,
    pub vertical: TextVerticalAlignment,
    pub decoration: Option<PaintTextDecoration>,
    pub stroke: Option<PaintTextStroke>,
    pub shadows: Vec<PaintTextShadow>,
}

impl PaintText {
    pub fn new(content: impl Into<Arc<str>>) -> Self {
        Self {
            content: content.into(),
            paint: Paint::Color(PaintColor::Role(SemanticColorRole::Text)),
            size: None,
            weight: None,
            family: None,
            italic: false,
            line_height: None,
            letter_spacing: 0.0,
            wrap: false,
            max_lines: None,
            horizontal: TextHorizontalAlignment::Start,
            vertical: TextVerticalAlignment::Center,
            decoration: None,
            stroke: None,
            shadows: Vec::new(),
        }
    }

    /// 纯色或渐变。
    pub fn paint(mut self, paint: impl Into<Paint>) -> Self {
        self.paint = paint.into();
        self
    }

    pub fn color(self, color: impl Into<PaintColor>) -> Self {
        self.paint(color.into())
    }

    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    pub fn line_height(mut self, line_height: f32) -> Self {
        self.line_height = Some(line_height);
        self
    }

    pub fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.letter_spacing = letter_spacing;
        self
    }

    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    pub fn max_lines(mut self, lines: u16) -> Self {
        self.max_lines = Some(lines);
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = Some(weight);
        self
    }

    /// 字体族，与 CSS `font-family` 同语法（逗号分隔的回退列表）。测量
    /// （`cx.measure_text`）和绘制读的是同一个字体族。
    pub fn family(mut self, family: impl Into<Arc<str>>) -> Self {
        self.family = Some(family.into());
        self
    }

    pub fn align(
        mut self,
        horizontal: TextHorizontalAlignment,
        vertical: TextVerticalAlignment,
    ) -> Self {
        self.horizontal = horizontal;
        self.vertical = vertical;
        self
    }

    pub fn decoration(mut self, decoration: PaintTextDecoration) -> Self {
        self.decoration = Some(decoration);
        self
    }

    pub fn stroke(mut self, stroke: PaintTextStroke) -> Self {
        self.stroke = Some(stroke);
        self
    }

    pub fn shadow(mut self, shadow: PaintTextShadow) -> Self {
        self.shadows.push(shadow);
        self
    }

    pub fn shadows(mut self, shadows: impl IntoIterator<Item = PaintTextShadow>) -> Self {
        self.shadows.extend(shadows);
        self
    }
}

/// Theme-resolved decoration for a recorded [`PaintOp::Text`].
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPaintTextDecoration {
    pub line: nana_ui_core::TextDecorationLine,
    pub style: nana_ui_core::TextDecorationStyle,
    pub color: Option<nana_ui_core::PaintColor>,
    pub thickness: Option<f32>,
}

/// Theme-resolved outline stroke for a recorded [`PaintOp::Text`].
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPaintTextStroke {
    pub width: f32,
    pub paint: ResolvedPaint,
    pub join: nana_ui_core::TextStrokeJoin,
    pub placement: nana_ui_core::TextStrokePlacement,
}

/// Theme-resolved decorations, stroke, and shadow layers for [`PaintOp::Text`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResolvedPaintTextEffects {
    pub decoration: Option<ResolvedPaintTextDecoration>,
    pub stroke: Option<ResolvedPaintTextStroke>,
    pub shadows: Arc<[nana_ui_core::RichTextShadow]>,
}

/// 绘制阶段：子节点下面（背景）或上面（前景）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintPhase {
    BehindChildren,
    OverChildren,
}

/// 录下的一条命令，颜色和尺寸已按主题解析，坐标在节点内坐标系。
#[derive(Debug, Clone, PartialEq)]
pub enum PaintOp {
    /// 该节点原本的内建外观（WidgetVisual、背景、边框、文字）。
    DrawDefault,
    /// 之后的命令都在这个局部变换下（相对节点左上角），直到下一条
    /// `SetTransform`。录制开始时是恒等变换。
    SetTransform(Affine),
    /// 之后每条命令各自乘上这个不透明度（Canvas `globalAlpha`），直到下一条
    /// `SetOpacity`。录制开始时是 1。
    SetOpacity(f32),
    /// 之后的命令先画进一个图层，到配对的 [`Self::PopLayer`] 时按
    /// `opacity` 和 `blend` 整体合成到下面。
    PushLayer {
        opacity: f32,
        blend: BlendMode,
    },
    PopLayer,
    /// Opens an arbitrary custom-paint mask layer (`MaskKind::Alpha` or
    /// `MaskKind::Luminance`) applied to the enclosing layer on [`Self::PopMask`].
    PushMask {
        kind: MaskKind,
    },
    PopMask,
    FillPath {
        path: Arc<PaintPath>,
        paint: ResolvedPaint,
    },
    StrokePath {
        path: Arc<PaintPath>,
        paint: ResolvedPaint,
        stroke: StrokeStyle,
    },
    Shadow {
        path: Arc<PaintPath>,
        shadow: ComponentElevation,
    },
    RoundedRect {
        rect: LayoutBox,
        radii: [f32; 4],
        fill: Option<ResolvedPaint>,
        border: Option<([f32; 4], f32)>,
        border_paint: Option<(PaintColor, f32)>,
        shadow: Option<ComponentElevation>,
    },
    Image {
        rect: LayoutBox,
        /// 与 CSS `url()` 相同的来源：`data:`、文件路径或经宿主 fetch 的 URL。
        source: Arc<str>,
        fit: ImageFit,
        /// 录制时的 [`PaintContext::set_image_sampling`]。
        sampling: nana_ui_core::ImageSampling,
        radii: [f32; 4],
        src_rect: Option<LayoutBox>,
        nine_slice: Option<[f32; 4]>,
        nine_slice_fill: bool,
    },
    /// Lightweight batch of atlas sub-rectangle sprite draws sharing one source texture.
    TextureRegionBatch {
        source: Arc<str>,
        sampling: nana_ui_core::ImageSampling,
        /// `(src_rect, dest_rect)` pairs in source and local layout coordinates.
        items: Vec<(LayoutBox, LayoutBox)>,
    },
    /// 图片摆进 `rect`，只露出 `path` 以内的部分。
    FillImage {
        path: Arc<PaintPath>,
        rect: LayoutBox,
        source: Arc<str>,
        fit: ImageFit,
        sampling: nana_ui_core::ImageSampling,
    },
    Text {
        rect: LayoutBox,
        content: Arc<str>,
        paint: ResolvedPaint,
        size: f32,
        weight: Option<u16>,
        family: Option<Arc<str>>,
        italic: bool,
        line_height: Option<f32>,
        letter_spacing: f32,
        wrap: bool,
        max_lines: Option<u16>,
        horizontal: TextHorizontalAlignment,
        vertical: TextVerticalAlignment,
        effects: Option<Arc<ResolvedPaintTextEffects>>,
    },
    Icon {
        rect: LayoutBox,
        icon: Icon,
        paint: ResolvedPaint,
    },
    /// 之后的命令裁剪到这条路径以内，直到配对的 [`Self::PopClip`]。
    PushClip {
        path: Arc<PaintPath>,
    },
    PopClip,
}

/// 一个节点的录制结果。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintRecording {
    pub behind_children: Vec<PaintOp>,
    pub over_children: Vec<PaintOp>,
}

impl PaintRecording {
    /// 节点内坐标 `local` 是否落在录下的内容里：填充、描边、圆角矩形、图片、
    /// 文字和图标的框，以及 `draw_default()` 所占的节点矩形；按当时的局部变换
    /// 和 `push_clip` 计算。阴影不算。
    pub fn contains(&self, local: [f32; 2], size: [f32; 2]) -> bool {
        phase_contains(&self.behind_children, local, size)
            || phase_contains(&self.over_children, local, size)
    }

    /// `draw_default()` 录在哪个阶段；没调用则为 `None`。
    pub fn default_phase(&self) -> Option<PaintPhase> {
        // Matched by variant: comparing every op for equality costs as much as
        // the op is large, and a canvas records thousands.
        let drawn = |ops: &[PaintOp]| ops.iter().any(|op| matches!(op, PaintOp::DrawDefault));
        if drawn(&self.behind_children) {
            Some(PaintPhase::BehindChildren)
        } else if drawn(&self.over_children) {
            Some(PaintPhase::OverChildren)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy)]
struct HitLayer {
    blend: BlendMode,
    visible: bool,
    mask_kind: Option<MaskKind>,
    hit: bool,
}

fn compose_hit(dst: bool, src: bool, blend: BlendMode) -> bool {
    match blend {
        BlendMode::Copy | BlendMode::DestinationAtop => src,
        BlendMode::SourceIn | BlendMode::DestinationIn | BlendMode::SourceAtop => dst && src,
        BlendMode::SourceOut => !dst && src,
        BlendMode::DestinationOut => dst && !src,
        BlendMode::Xor => dst ^ src,
        _ => dst || src,
    }
}

fn paint_has_luminance(paint: &ResolvedPaint) -> bool {
    match paint {
        ResolvedPaint::Solid([r, g, b, a]) => {
            *a > 0.0 && (0.2126 * r + 0.7152 * g + 0.0722 * b) > 1e-4
        }
        ResolvedPaint::Authoring(color) => {
            let ([r, g, b], a) = color.to_linear_sc_rgb();
            a > 0.0 && (0.2126 * r + 0.7152 * g + 0.0722 * b) > 1e-4
        }
        ResolvedPaint::Gradient(_) | ResolvedPaint::Pattern(_) => true,
    }
}

fn phase_contains(ops: &[PaintOp], p: [f32; 2], size: [f32; 2]) -> bool {
    let mut t = AFFINE_IDENTITY;
    let mut clips: Vec<(&PaintPath, Affine)> = Vec::new();
    let mut visible = true;
    let mut layers = vec![HitLayer {
        blend: BlendMode::Normal,
        visible: true,
        mask_kind: None,
        hit: false,
    }];
    let close_layer = |layers: &mut Vec<HitLayer>| {
        if layers.len() <= 1 {
            return;
        }
        let layer = layers.pop().expect("len > 1 checked above");
        let parent = layers.last_mut().expect("root layer present");
        let src = layer.hit && layer.visible;
        if layer.mask_kind.is_some() {
            parent.hit = parent.hit && src;
        } else {
            parent.hit = compose_hit(parent.hit, src, layer.blend);
        }
    };
    for op in ops {
        let luminance_mask = layers
            .last()
            .is_some_and(|l| l.mask_kind == Some(MaskKind::Luminance));
        let hit = match op {
            PaintOp::SetTransform(transform) => {
                t = *transform;
                false
            }
            PaintOp::PushClip { path } => {
                clips.push((path, t));
                false
            }
            PaintOp::PopClip => {
                clips.pop();
                false
            }
            PaintOp::FillPath { path, paint } => {
                (!luminance_mask || paint_has_luminance(paint)) && path_contains(path, t, p)
            }
            PaintOp::FillImage { path, .. } => path_contains(path, t, p),
            PaintOp::StrokePath {
                path,
                paint,
                stroke,
            } => {
                (!luminance_mask || paint_has_luminance(paint))
                    && stroke_contains(path, stroke, t, p)
            }
            PaintOp::RoundedRect {
                rect,
                radii,
                fill,
                border,
                ..
            } => {
                let luma_ok = !luminance_mask
                    || fill.as_ref().is_some_and(paint_has_luminance)
                    || border.is_some_and(|([r, g, b, a], _)| {
                        a > 0.0 && (0.2126 * r + 0.7152 * g + 0.0722 * b) > 1e-4
                    });
                luma_ok
                    && inverse_apply(t, p).is_some_and(|q| rounded_rect_contains(*rect, *radii, q))
            }
            PaintOp::Image { rect, radii, .. } => {
                inverse_apply(t, p).is_some_and(|q| rounded_rect_contains(*rect, *radii, q))
            }
            PaintOp::TextureRegionBatch { items, .. } => inverse_apply(t, p).is_some_and(|q| {
                items
                    .iter()
                    .any(|(_, dest)| rounded_rect_contains(*dest, [0.0; 4], q))
            }),
            PaintOp::Text { rect, paint, .. } | PaintOp::Icon { rect, paint, .. } => {
                (!luminance_mask || paint_has_luminance(paint))
                    && inverse_apply(t, p)
                        .is_some_and(|q| rounded_rect_contains(*rect, [0.0; 4], q))
            }
            PaintOp::DrawDefault => inverse_apply(t, p)
                .is_some_and(|q| q[0] >= 0.0 && q[1] >= 0.0 && q[0] <= size[0] && q[1] <= size[1]),
            PaintOp::SetOpacity(opacity) => {
                // What draws fully transparent is not there to hit.
                visible = *opacity > 0.0;
                false
            }
            PaintOp::PushLayer { opacity, blend } => {
                layers.push(HitLayer {
                    blend: *blend,
                    visible: *opacity > 0.0,
                    mask_kind: None,
                    hit: false,
                });
                false
            }
            PaintOp::PopLayer => {
                close_layer(&mut layers);
                false
            }
            PaintOp::PushMask { kind } => {
                layers.push(HitLayer {
                    blend: BlendMode::Normal,
                    visible: true,
                    mask_kind: Some(*kind),
                    hit: false,
                });
                false
            }
            PaintOp::PopMask => {
                close_layer(&mut layers);
                false
            }
            PaintOp::Shadow { .. } => false,
        };
        if hit
            && visible
            && clips.iter().all(|(clip, ct)| path_contains(clip, *ct, p))
            && let Some(top) = layers.last_mut()
        {
            top.hit = true;
        }
    }
    while layers.len() > 1 {
        close_layer(&mut layers);
    }
    layers[0].hit
}

fn inverse_apply(t: Affine, p: [f32; 2]) -> Option<[f32; 2]> {
    let [a, b, c, d, e, f] = t;
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let (x, y) = (p[0] - e, p[1] - f);
    Some([(d * x - c * y) / det, (a * y - b * x) / det])
}

const MAX_CURVE_SUBDIVISIONS: u8 = 12;

fn point_sane(p: [f32; 2]) -> bool {
    p.iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_PATH_COORDINATE)
}

fn path_is_drawable(path: &PaintPath) -> bool {
    path.verbs().iter().all(|verb| match *verb {
        PathVerb::MoveTo(p) | PathVerb::LineTo(p) => point_sane(p),
        PathVerb::QuadTo(c, p) => point_sane(c) && point_sane(p),
        PathVerb::CubicTo(a, b, p) => point_sane(a) && point_sane(b) && point_sane(p),
        PathVerb::Close => true,
    })
}

fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    dx * dx + dy * dy
}

fn midpoint(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn flatten_quad(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    tol_sq: f32,
    depth: u8,
    out: &mut Vec<[f32; 2]>,
) {
    // Max deviation of a quadratic Bezier from its chord is 0.5 * |p1 - mid(p0, p2)|.
    let mid = midpoint(p0, p2);
    if depth >= MAX_CURVE_SUBDIVISIONS || 0.25 * dist2(p1, mid) <= tol_sq {
        out.push(p2);
        return;
    }
    let a = midpoint(p0, p1);
    let b = midpoint(p1, p2);
    let c = midpoint(a, b);
    flatten_quad(p0, a, c, tol_sq, depth + 1, out);
    flatten_quad(c, b, p2, tol_sq, depth + 1, out);
}

fn flatten_cubic(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    p3: [f32; 2],
    tol_sq: f32,
    depth: u8,
    out: &mut Vec<[f32; 2]>,
) {
    // Max deviation of a cubic Bezier from chord p0..p3 is bounded by 0.75 * max(d1, d2).
    let d1 = dist2(p1, lerp2(p0, p3, 1.0 / 3.0));
    let d2 = dist2(p2, lerp2(p0, p3, 2.0 / 3.0));
    if depth >= MAX_CURVE_SUBDIVISIONS || 0.5625 * d1.max(d2) <= tol_sq {
        out.push(p3);
        return;
    }
    let p01 = midpoint(p0, p1);
    let p12 = midpoint(p1, p2);
    let p23 = midpoint(p2, p3);
    let p012 = midpoint(p01, p12);
    let p123 = midpoint(p12, p23);
    let p0123 = midpoint(p012, p123);
    flatten_cubic(p0, p01, p012, p0123, tol_sq, depth + 1, out);
    flatten_cubic(p0123, p123, p23, p3, tol_sq, depth + 1, out);
}

/// Sub-paths as polylines in the path's own coordinates, flattened at
/// [`PATH_FLATTEN_TOLERANCE`], with whether each was closed.
fn polylines(path: &PaintPath) -> Vec<(Vec<[f32; 2]>, bool)> {
    if !path_is_drawable(path) {
        return Vec::new();
    }
    let tol_sq = PATH_FLATTEN_TOLERANCE * PATH_FLATTEN_TOLERANCE;
    let mut out: Vec<(Vec<[f32; 2]>, bool)> = Vec::new();
    let mut current: Vec<[f32; 2]> = Vec::new();
    let mut start = [0.0f32; 2];
    let finish = |current: &mut Vec<[f32; 2]>, out: &mut Vec<(Vec<[f32; 2]>, bool)>, closed| {
        if current.len() > 1 {
            out.push((std::mem::take(current), closed));
        } else {
            current.clear();
        }
    };
    for verb in path.verbs() {
        let from = current.last().copied().unwrap_or(start);
        match *verb {
            PathVerb::MoveTo(p) => {
                finish(&mut current, &mut out, false);
                current.push(p);
                start = p;
            }
            PathVerb::LineTo(p) => {
                if current.is_empty() {
                    current.push(from);
                }
                current.push(p);
            }
            PathVerb::QuadTo(c, p) => {
                if current.is_empty() {
                    current.push(from);
                }
                flatten_quad(from, c, p, tol_sq, 0, &mut current);
            }
            PathVerb::CubicTo(c1, c2, p) => {
                if current.is_empty() {
                    current.push(from);
                }
                flatten_cubic(from, c1, c2, p, tol_sq, 0, &mut current);
            }
            PathVerb::Close => {
                finish(&mut current, &mut out, true);
                current.push(start);
            }
        }
    }
    finish(&mut current, &mut out, false);
    out
}

fn path_to_overlay_shapes(path: &PaintPath) -> Vec<Vec<Vec<[f64; 2]>>> {
    if path.is_empty() {
        return Vec::new();
    }
    let contours: Vec<Vec<[f64; 2]>> = polylines(path)
        .into_iter()
        .filter_map(|(pts, _)| {
            let mut contour: Vec<[f64; 2]> = Vec::with_capacity(pts.len());
            for p in pts {
                let pt = [f64::from(p[0]), f64::from(p[1])];
                let same_last = contour
                    .last()
                    .is_some_and(|l| (l[0] - pt[0]).abs() < 1e-4 && (l[1] - pt[1]).abs() < 1e-4);
                if !same_last {
                    contour.push(pt);
                }
            }
            if contour.len() > 1 {
                let first = contour[0];
                let last = contour[contour.len() - 1];
                if (first[0] - last[0]).abs() < 1e-4 && (first[1] - last[1]).abs() < 1e-4 {
                    contour.pop();
                }
            }
            (contour.len() >= 3).then_some(contour)
        })
        .collect();
    if contours.is_empty() {
        return Vec::new();
    }
    let rule = match path.fill_rule() {
        FillRule::NonZero => OverlayFill::NonZero,
        FillRule::EvenOdd => OverlayFill::EvenOdd,
    };
    contours.simplify_shape(rule)
}

fn overlay_shapes_to_path(shapes: &[Vec<Vec<[f64; 2]>>]) -> PaintPath {
    let mut out = PaintPath::new();
    for shape in shapes {
        for contour in shape {
            if contour.len() < 3 {
                continue;
            }
            out.move_to(contour[0][0] as f32, contour[0][1] as f32);
            for pt in &contour[1..] {
                out.line_to(pt[0] as f32, pt[1] as f32);
            }
            out.close();
        }
    }
    out
}

fn path_contains(path: &PaintPath, t: Affine, p: [f32; 2]) -> bool {
    let Some(q) = inverse_apply(t, p) else {
        return false;
    };
    let mut winding = 0i32;
    for (points, _) in polylines(path) {
        let n = points.len();
        for i in 0..n {
            let (a, b) = (points[i], points[(i + 1) % n]);
            if (a[1] <= q[1]) != (b[1] <= q[1]) {
                let side = (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
                if b[1] > a[1] && side > 0.0 {
                    winding += 1;
                } else if b[1] <= a[1] && side < 0.0 {
                    winding -= 1;
                }
            }
        }
    }
    match path.fill_rule() {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding % 2 != 0,
    }
}

fn stroke_contains(path: &PaintPath, stroke: &StrokeStyle, t: Affine, p: [f32; 2]) -> bool {
    let Some(q) = inverse_apply(t, p) else {
        return false;
    };
    let reach = stroke.width * 0.5;
    let pattern = dash_pattern(&stroke.dash);
    polylines(path).into_iter().any(|(mut points, closed)| {
        if closed {
            points.push(points[0]);
        }
        let pieces = match &pattern {
            Some(pattern) if !too_many_dashes(&points, pattern) => {
                dash_pieces(&points, pattern, stroke.dash_offset)
            }
            _ => vec![points],
        };
        pieces.iter().any(|piece| {
            piece
                .windows(2)
                .any(|segment| segment_distance(q, segment[0], segment[1]) <= reach)
        })
    })
}

/// The dash pattern a stroke draws with, `None` for a solid one. Mirrors the
/// scene's reading of Canvas `setLineDash`.
fn dash_pattern(dash: &[f32]) -> Option<Vec<f32>> {
    if dash.is_empty()
        || dash
            .iter()
            .any(|length| !length.is_finite() || *length < 0.0)
    {
        return None;
    }
    let mut pattern = dash.to_vec();
    if pattern.len() % 2 == 1 {
        pattern.extend_from_within(..);
    }
    (pattern.iter().sum::<f32>() > 1e-6).then_some(pattern)
}

/// Past this many dashes a stroke is drawn solid, as the scene does: the
/// pieces would be sub-pixel and walking them would stall the frame.
const MAX_DASHES: f32 = 10_000.0;

fn too_many_dashes(points: &[[f32; 2]], pattern: &[f32]) -> bool {
    let length: f32 = points
        .windows(2)
        .map(|s| ((s[1][0] - s[0][0]).powi(2) + (s[1][1] - s[0][1]).powi(2)).sqrt())
        .sum();
    let period: f32 = pattern.iter().sum();
    let dashes = length / period;
    dashes.is_nan() || dashes > MAX_DASHES
}

/// The "on" pieces of a polyline under a dash pattern.
fn dash_pieces(points: &[[f32; 2]], pattern: &[f32], offset: f32) -> Vec<Vec<[f32; 2]>> {
    let period: f32 = pattern.iter().sum();
    let mut phase = offset.rem_euclid(period);
    let mut index = 0;
    while phase >= pattern[index] {
        phase -= pattern[index];
        index = (index + 1) % pattern.len();
    }
    let mut left = pattern[index] - phase;
    let mut on = index % 2 == 0;
    let mut pieces = Vec::new();
    let mut current = if on { vec![points[0]] } else { Vec::new() };
    for segment in points.windows(2) {
        let (mut from, to) = (segment[0], segment[1]);
        let mut length = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
        while length > left {
            let k = left / length;
            let cut = [
                from[0] + (to[0] - from[0]) * k,
                from[1] + (to[1] - from[1]) * k,
            ];
            if on {
                current.push(cut);
                pieces.push(std::mem::take(&mut current));
            } else {
                current = vec![cut];
            }
            on = !on;
            length -= left;
            from = cut;
            index = (index + 1) % pattern.len();
            left = pattern[index];
        }
        left -= length;
        if on {
            current.push(to);
        }
    }
    if on && current.len() >= 2 {
        pieces.push(current);
    }
    pieces
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let length = ab[0] * ab[0] + ab[1] * ab[1];
    let k = if length > 0.0 {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let d = [ap[0] - ab[0] * k, ap[1] - ab[1] * k];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

fn rounded_rect_contains(rect: LayoutBox, radii: [f32; 4], q: [f32; 2]) -> bool {
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
    if q[0] < x0 || q[1] < y0 || q[0] > x1 || q[1] > y1 {
        return false;
    }
    let [tl, tr, br, bl] = fit_radii(rect, radii);
    for (radius, cx, cy, left, top) in [
        (tl, x0 + tl, y0 + tl, true, true),
        (tr, x1 - tr, y0 + tr, false, true),
        (br, x1 - br, y1 - br, false, false),
        (bl, x0 + bl, y1 - bl, true, false),
    ] {
        let in_corner =
            (if left { q[0] < cx } else { q[0] > cx }) && (if top { q[1] < cy } else { q[1] > cy });
        if radius > 0.0 && in_corner {
            let (dx, dy) = (q[0] - cx, q[1] - cy);
            return dx * dx + dy * dy <= radius * radius;
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenScope {
    Layer,
    Mask,
    Clip,
}

/// 录制绘制命令的上下文，相当于 `QPainter`。
///
/// 坐标原点是节点布局盒的左上角，单位逻辑像素。节点的 transform、裁剪、
/// 不透明度、层叠顺序和合成器动画由 Scene 统一套上，这里不用管。
pub struct PaintContext<'a> {
    theme: &'a CompiledTheme,
    size: [f32; 2],
    state: PaintState,
    text_host: &'a dyn PaintTextHost,
    ops: Vec<PaintOp>,
    default_drawn: bool,
    clip_depth: usize,
    layer_depth: usize,
    /// Open scopes, innermost last.
    open: Vec<OpenScope>,
    transform: Affine,
    opacity: f32,
    blend: BlendMode,
    image_sampling: nana_ui_core::ImageSampling,
    /// The transform and opacity the ops recorded so far are under.
    recorded_transform: Affine,
    recorded_opacity: f32,
    saved: Vec<SavedState>,
    /// Where a modal card's sections lie, when this records the card's
    /// surface rather than a node.
    sections: Option<crate::ModalSurfaceSections>,
}

type SavedState = (Affine, f32, BlendMode, nana_ui_core::ImageSampling, usize);

impl<'a> PaintContext<'a> {
    fn new(
        theme: &'a CompiledTheme,
        size: [f32; 2],
        state: PaintState,
        text_host: &'a dyn PaintTextHost,
        default_drawn: bool,
    ) -> Self {
        Self {
            theme,
            size,
            state,
            text_host,
            ops: Vec::new(),
            default_drawn,
            clip_depth: 0,
            layer_depth: 0,
            open: Vec::new(),
            transform: AFFINE_IDENTITY,
            opacity: 1.0,
            blend: BlendMode::Normal,
            image_sampling: nana_ui_core::ImageSampling::Resample,
            recorded_transform: AFFINE_IDENTITY,
            recorded_opacity: 1.0,
            saved: Vec::new(),
            sections: None,
        }
    }

    /// Record a state op the scene tracks, if it moved.
    fn sync_state(&mut self) {
        if self.transform != self.recorded_transform {
            self.recorded_transform = self.transform;
            self.ops.push(PaintOp::SetTransform(self.transform));
        }
        if self.opacity != self.recorded_opacity {
            self.recorded_opacity = self.opacity;
            self.ops.push(PaintOp::SetOpacity(self.opacity));
        }
    }

    /// Record a drawing op under the current transform, opacity and blend.
    fn push(&mut self, op: PaintOp) {
        self.sync_state();
        if self.blend == BlendMode::Normal {
            self.ops.push(op);
        } else {
            self.ops.push(PaintOp::PushLayer {
                opacity: 1.0,
                blend: self.blend,
            });
            self.ops.push(op);
            self.ops.push(PaintOp::PopLayer);
        }
    }

    /// 节点当前的交互状态。
    pub fn state(&self) -> PaintState {
        self.state
    }

    /// 录的是模态对话框卡面（[`crate::Dialog::surface_painter`]）时，卡面
    /// 各段在卡面坐标里的位置：标题行、标题块、图标位、关闭位、正文、底栏。
    /// 录普通节点时是 `None`。
    pub fn modal_sections(&self) -> Option<crate::ModalSurfaceSections> {
        self.sections
    }

    /// 按节点字体和当前字体集测量一段文字；`max_width` 为 `None` 时不折行。
    /// 字号缺省为主题正文字号。与 [`Self::text`] 画出来的尺寸一致。
    pub fn measure_text(&self, text: &PaintText, max_width: Option<f32>) -> PaintTextMetrics {
        let size = text
            .size
            .unwrap_or_else(|| self.theme.style_model().base_text_size());
        self.text_host.measure_text(text, size, max_width)
    }

    /// Converts `text` into glyph outline paths using the host's `nana-text`
    /// engine, laid out at `origin` with optional wrapping width `max_width`.
    /// The returned [`PaintPath`] can be filled, stroked, clipped, hit-tested,
    /// or combined with [`PaintPath`] boolean operations.
    pub fn text_path(
        &self,
        text: &PaintText,
        origin: [f32; 2],
        max_width: Option<f32>,
    ) -> PaintPath {
        if text.content.is_empty() {
            return PaintPath::new();
        }
        let size = text
            .size
            .unwrap_or_else(|| self.theme.style_model().base_text_size());
        self.text_host.text_path(text, size, origin, max_width)
    }

    /// 之后每条命令的不透明度（Canvas `globalAlpha`），`0..=1`。
    pub fn set_opacity(&mut self, opacity: f32) {
        if opacity.is_finite() {
            self.opacity = opacity.clamp(0.0, 1.0);
        }
    }

    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    /// 之后每条命令与下层内容的混合方式；非 `Normal` 时每条命令单独成层。
    pub fn set_blend(&mut self, blend: BlendMode) {
        self.blend = blend;
    }

    /// 之后的 [`Self::image`] / [`Self::fill_image`] 怎样准备采样（类似 Canvas
    /// `imageSmoothingQuality`）。默认按实际绘制的设备像素重采样一层；
    /// 同一张图同时以多种尺寸出现、或尺寸持续变化（缩放动画）时用
    /// [`nana_ui_core::ImageSampling::Mipmap`]。
    pub fn set_image_sampling(&mut self, sampling: nana_ui_core::ImageSampling) {
        self.image_sampling = sampling;
    }

    pub fn image_sampling(&self) -> nana_ui_core::ImageSampling {
        self.image_sampling
    }

    /// Enables bilinear/resampled smoothing (`true`) or nearest-neighbor sampling
    /// (`false`, matching Canvas `imageSmoothingEnabled`) for subsequent images and patterns.
    pub fn set_image_smoothing(&mut self, smoothing: bool) {
        if smoothing {
            if self.image_sampling == nana_ui_core::ImageSampling::Nearest {
                self.image_sampling = nana_ui_core::ImageSampling::Resample;
            }
        } else {
            self.image_sampling = nana_ui_core::ImageSampling::Nearest;
        }
    }

    pub fn image_smoothing(&self) -> bool {
        self.image_sampling != nana_ui_core::ImageSampling::Nearest
    }

    /// 开一个图层：之后的命令先画进图层，[`Self::pop_layer`] 时按 `opacity`
    /// 和 `blend` 整体合成，重叠部分不会叠加透明度。
    pub fn push_layer(&mut self, opacity: f32, blend: BlendMode) {
        self.sync_state();
        self.layer_depth += 1;
        self.open.push(OpenScope::Layer);
        self.ops.push(PaintOp::PushLayer {
            opacity: if opacity.is_finite() {
                opacity.clamp(0.0, 1.0)
            } else {
                1.0
            },
            blend,
        });
    }

    /// 合成最近一个 [`Self::push_layer`] 打开的图层；没有打开的图层时什么都
    /// 不做。
    pub fn pop_layer(&mut self) {
        if let Some(at) = self.open.iter().rposition(|s| *s == OpenScope::Layer) {
            self.open.remove(at);
            self.layer_depth -= 1;
            self.ops.push(PaintOp::PopLayer);
        }
    }

    /// Opens an arbitrary custom-paint mask layer (`MaskKind::Alpha` or
    /// `MaskKind::Luminance`). Commands drawn until [`Self::pop_mask`] define
    /// the mask applied to the enclosing layer.
    pub fn push_mask(&mut self, kind: MaskKind) {
        self.sync_state();
        self.layer_depth += 1;
        self.open.push(OpenScope::Mask);
        self.ops.push(PaintOp::PushMask { kind });
    }

    /// Applies and closes the mask opened by [`Self::push_mask`].
    pub fn pop_mask(&mut self) {
        if let Some(at) = self.open.iter().rposition(|s| *s == OpenScope::Mask) {
            self.open.remove(at);
            self.layer_depth -= 1;
            self.ops.push(PaintOp::PopMask);
        }
    }

    /// 当前局部变换。
    pub fn transform(&self) -> Affine {
        self.transform
    }

    /// 替换当前局部变换（Canvas `setTransform`）。
    pub fn set_transform(&mut self, transform: Affine) {
        if transform.iter().all(|value| value.is_finite()) {
            self.transform = transform;
        }
    }

    /// 在当前变换之后再叠一层（Canvas `transform`）：之后的坐标先经 `transform`。
    pub fn concat(&mut self, transform: Affine) {
        self.set_transform(affine_multiply(self.transform, transform));
    }

    pub fn translate(&mut self, x: f32, y: f32) {
        self.concat([1.0, 0.0, 0.0, 1.0, x, y]);
    }

    pub fn scale(&mut self, x: f32, y: f32) {
        self.concat([x, 0.0, 0.0, y, 0.0, 0.0]);
    }

    /// 旋转 `angle` 弧度，y 轴向下时正值顺时针。
    pub fn rotate(&mut self, angle: f32) {
        let (sin, cos) = angle.sin_cos();
        self.concat([cos, sin, -sin, cos, 0.0, 0.0]);
    }

    /// 保存当前变换、不透明度、混合方式、图片采样方式和裁剪层数，与
    /// [`Self::restore`] 配对。图层另由 `push_layer` / `pop_layer` 管。
    pub fn save(&mut self) {
        self.saved.push((
            self.transform,
            self.opacity,
            self.blend,
            self.image_sampling,
            self.clip_depth,
        ));
    }

    /// 恢复最近一次 [`Self::save`] 时的状态，并像 Canvas 的 `restore()` 一样
    /// 弹出其后压入的裁剪；没有可恢复的就什么都不做。
    pub fn restore(&mut self) {
        if let Some((transform, opacity, blend, image_sampling, clip_depth)) = self.saved.pop() {
            self.transform = transform;
            self.opacity = opacity;
            self.blend = blend;
            self.image_sampling = image_sampling;
            while self.clip_depth > clip_depth {
                self.pop_clip();
            }
        }
    }

    /// 按当前主题解析上色。
    pub fn paint(&self, paint: impl Into<Paint>) -> ResolvedPaint {
        match paint.into() {
            Paint::Color(color) => match color {
                PaintColor::Authoring(color) => ResolvedPaint::Authoring(color),
                color => ResolvedPaint::Solid(self.color(color)),
            },
            Paint::Gradient(gradient) => {
                let mut stops: Vec<(f32, [f32; 4])> = gradient
                    .stops
                    .iter()
                    .filter(|stop| stop.offset.is_finite())
                    .map(|stop| (stop.offset.clamp(0.0, 1.0), self.color(stop.color)))
                    .collect();
                // Stable, so equal offsets keep their order: a hard edge.
                stops.compact_sort_by(|a, b| a.0.total_cmp(&b.0));
                if stops.is_empty() {
                    stops.push((0.0, [0.0; 4]));
                }
                let has_authoring = gradient
                    .stops
                    .iter()
                    .any(|stop| matches!(stop.color, PaintColor::Authoring(_)));
                let color_space = gradient.color_space.unwrap_or(if has_authoring {
                    GradientColorSpace::LinearScRgb
                } else {
                    GradientColorSpace::PremulSrgb
                });
                let needs_linear = has_authoring
                    || matches!(
                        color_space,
                        GradientColorSpace::LinearScRgb | GradientColorSpace::Oklab
                    );
                let linear_stops = needs_linear.then(|| {
                    let mut stops = gradient
                        .stops
                        .iter()
                        .filter(|stop| stop.offset.is_finite())
                        .map(|stop| {
                            let offset = stop.offset.clamp(0.0, 1.0);
                            let resolved = match stop.color {
                                PaintColor::Authoring(color) => color,
                                color => nana_ui_core::PaintColor::Srgb {
                                    rgba: self.color(color),
                                },
                            };
                            let (channels, alpha) = resolved.to_linear_sc_rgb();
                            (offset, [channels[0], channels[1], channels[2], alpha])
                        })
                        .collect::<Vec<_>>();
                    stops.compact_sort_by(|a, b| a.0.total_cmp(&b.0));
                    if stops.is_empty() {
                        stops.push((0.0, [0.0; 4]));
                    }
                    stops
                });
                let transform = if gradient.transform.iter().all(|v| v.is_finite()) {
                    gradient.transform
                } else {
                    AFFINE_IDENTITY
                };
                ResolvedPaint::Gradient(Arc::new(ResolvedGradient {
                    shape: gradient.shape,
                    stops,
                    linear_stops,
                    extend: gradient.extend,
                    transform,
                    color_space,
                }))
            }
            Paint::Pattern(pattern) => {
                let transform = if pattern.transform.iter().all(|v| v.is_finite()) {
                    pattern.transform
                } else {
                    AFFINE_IDENTITY
                };
                ResolvedPaint::Pattern(Arc::new(ResolvedPattern {
                    source: pattern.source,
                    src_rect: pattern.src_rect,
                    repeat: pattern.repeat,
                    transform,
                    sampling: pattern.sampling.unwrap_or(self.image_sampling),
                }))
            }
        }
    }

    /// 节点布局尺寸 `[宽, 高]`。
    pub fn size(&self) -> [f32; 2] {
        self.size
    }

    /// 节点自身的矩形，原点为 `(0, 0)`。
    pub fn bounds(&self) -> LayoutBox {
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: self.size[0],
            height: self.size[1],
        }
    }

    /// The installed theme for product painters deriving their visual geometry.
    pub fn theme(&self) -> &nana_ui_core::CompiledTheme {
        self.theme
    }

    pub fn theme_appearance(&self) -> nana_ui_core::ThemeAppearance {
        self.theme.appearance()
    }

    pub fn metrics(&self) -> ThemeMetrics {
        self.theme.metrics()
    }

    /// The installed effect tokens: the media scrim a built-in painter lays
    /// under chrome over media.
    pub(crate) fn effects(&self) -> nana_ui_core::EffectTokens {
        self.theme.effects()
    }

    /// 按当前主题解析颜色。
    pub fn color(&self, color: impl Into<PaintColor>) -> [f32; 4] {
        let model = self.theme.style_model();
        match color.into() {
            PaintColor::Role(role) => model.color(role).as_rgba_array(),
            PaintColor::Mix(mix) => mix.resolve(model).as_rgba_array(),
            PaintColor::Rgba(rgba) => rgba,
            PaintColor::Authoring(color) => color.to_srgb(),
        }
    }

    /// The installed theme's modal backdrop scrim color (`SemanticColorRole::ModalScrim`).
    pub fn modal_scrim(&self) -> [f32; 4] {
        self.color(SemanticColorRole::ModalScrim)
    }

    /// 按当前主题解析圆角档位。
    pub fn radius(&self, radius: impl Into<Radius>) -> f32 {
        match radius.into() {
            Radius::Px(px) => px,
            Radius::Tier(tier) => tier.resolve(self.theme.metrics()),
        }
    }

    /// 按当前主题解析阴影。
    pub fn elevation(&self, shadow: impl Into<PaintShadow>) -> ComponentElevation {
        match shadow.into() {
            PaintShadow::Elevation(role) => {
                ComponentElevation::from_shadow(self.theme.shadow(role))
            }
            PaintShadow::Custom {
                color,
                offset,
                blur,
                spread,
                inset,
            } => ComponentElevation {
                color: match color {
                    PaintColor::Authoring(color) => color,
                    color => nana_ui_core::PaintColor::srgb(self.color(color)),
                },
                offset_x: offset[0],
                offset_y: offset[1],
                blur_radius: blur.max(0.0),
                spread_radius: spread,
                inset,
            },
        }
    }

    /// 画出该节点原本的内建外观，相当于在重写里调用基类的 `paintEvent`。
    /// 每个节点只生效一次。
    pub fn draw_default(&mut self) {
        if !self.default_drawn {
            self.default_drawn = true;
            self.push(PaintOp::DrawDefault);
        }
    }

    /// 填充路径，纯色或渐变。
    pub fn fill_path(&mut self, path: &PaintPath, paint: impl Into<Paint>) {
        if path.is_empty() {
            return;
        }
        let paint = self.paint(paint);
        self.push(PaintOp::FillPath {
            path: Arc::new(path.clone()),
            paint,
        });
    }

    /// 描边路径，纯色或渐变；虚线见 [`StrokeStyle::dash`]。
    pub fn stroke_path(
        &mut self,
        path: &PaintPath,
        stroke: impl Into<StrokeStyle>,
        paint: impl Into<Paint>,
    ) {
        let stroke = stroke.into();
        if path.is_empty() || stroke.width.is_nan() || stroke.width <= 0.0 {
            return;
        }
        let paint = self.paint(paint);
        self.push(PaintOp::StrokePath {
            path: Arc::new(path.clone()),
            paint,
            stroke,
        });
    }

    /// 沿路径轮廓的投影（CSS `box-shadow` 语义）。外阴影画在路径以外，
    /// `inset` 阴影画在路径以内。
    pub fn shadow(&mut self, path: &PaintPath, shadow: impl Into<PaintShadow>) {
        if path.is_empty() {
            return;
        }
        let shadow = self.elevation(shadow);
        self.push(PaintOp::Shadow {
            path: Arc::new(path.clone()),
            shadow,
        });
    }

    /// 四角独立的圆角矩形。
    pub fn rounded_rect(
        &mut self,
        rect: LayoutBox,
        radii: impl Into<CornerRadii>,
        paint: BoxPaint,
    ) {
        let CornerRadii(radii) = radii.into();
        let radii = fit_radii(rect, radii.map(|radius| self.radius(radius)));
        let fill = paint.fill.map(|fill| self.paint(fill));
        let border = paint
            .border
            .filter(|(_, width)| *width > 0.0)
            .map(|(color, width)| (self.color(color), width));
        let border_paint = paint
            .border
            .filter(|(color, width)| *width > 0.0 && matches!(color, PaintColor::Authoring(_)));
        let shadow = paint.shadow.map(|shadow| self.elevation(shadow));
        if fill.is_none() && border.is_none() && shadow.is_none() {
            return;
        }
        self.push(PaintOp::RoundedRect {
            rect,
            radii,
            fill,
            border,
            border_paint,
            shadow,
        });
    }

    /// resolves a [`PaintColor`] into a [`nana_ui_core::PaintColor`], preserving authoring spaces.
    fn resolve_core_paint_color(&self, color: PaintColor) -> nana_ui_core::PaintColor {
        match color {
            PaintColor::Authoring(color) => color,
            color => nana_ui_core::PaintColor::srgb(self.color(color)),
        }
    }

    /// 一段文字，排在 `rect` 里。纯色或渐变；`wrap` / `max_lines` 控制折行。
    pub fn text(&mut self, rect: LayoutBox, text: PaintText) {
        if text.content.is_empty() {
            return;
        }
        let paint = self.paint(text.paint.clone());
        let size = text
            .size
            .unwrap_or_else(|| self.theme.style_model().base_text_size());
        let letter_spacing = if text.letter_spacing.is_finite() {
            text.letter_spacing
        } else {
            0.0
        };
        let decoration =
            text.decoration
                .filter(|d| d.is_active())
                .map(|d| ResolvedPaintTextDecoration {
                    line: nana_ui_core::TextDecorationLine {
                        underline: d.underline,
                        line_through: d.line_through,
                        overline: d.overline,
                    },
                    style: d.style,
                    color: d.color.map(|c| self.resolve_core_paint_color(c)),
                    thickness: d.thickness.filter(|t| t.is_finite() && *t > 0.0),
                });
        let stroke = text
            .stroke
            .filter(|s| s.width.is_finite() && s.width > 0.0)
            .map(|s| ResolvedPaintTextStroke {
                width: s.width.min(1.0e4),
                paint: self.paint(s.paint),
                join: s.join,
                placement: s.placement,
            });
        let shadows: Vec<nana_ui_core::RichTextShadow> = text
            .shadows
            .iter()
            .filter(|s| s.offset[0].is_finite() && s.offset[1].is_finite() && s.blur.is_finite())
            .map(|s| nana_ui_core::RichTextShadow {
                offset: s.offset,
                blur_px: s.blur.max(0.0),
                spread_px: if s.spread.is_finite() {
                    s.spread.max(0.0)
                } else {
                    0.0
                },
                color: self.resolve_core_paint_color(s.color),
            })
            .collect();
        let effects = if decoration.is_some() || stroke.is_some() || !shadows.is_empty() {
            Some(Arc::new(ResolvedPaintTextEffects {
                decoration,
                stroke,
                shadows: shadows.into(),
            }))
        } else {
            None
        };
        self.push(PaintOp::Text {
            rect,
            content: text.content,
            paint,
            size,
            weight: text.weight,
            family: text.family,
            italic: text.italic,
            line_height: text.line_height,
            letter_spacing,
            wrap: text.wrap,
            max_lines: text.max_lines,
            horizontal: text.horizontal,
            vertical: text.vertical,
            effects,
        });
    }

    /// 一个图标，纯色或渐变。
    pub fn icon(&mut self, rect: LayoutBox, icon: Icon, paint: impl Into<Paint>) {
        let paint = self.paint(paint);
        self.push(PaintOp::Icon { rect, icon, paint });
    }

    /// 用图片填充路径：图片按 `fit` 摆进 `rect`，只露出 `path` 以内的部分。
    pub fn fill_path_with_image(
        &mut self,
        path: &PaintPath,
        rect: LayoutBox,
        source: impl Into<Arc<str>>,
        fit: ImageFit,
    ) {
        let source = source.into();
        if path.is_empty() || source.is_empty() {
            return;
        }
        self.push(PaintOp::FillImage {
            path: Arc::new(path.clone()),
            rect,
            source,
            fit,
            sampling: self.image_sampling,
        });
    }

    /// 一张图片，按 `fit` 摆进 `rect`，裁到四角圆角以内。`source` 与 CSS
    /// `url()` 同源：`data:`、文件路径或经宿主 fetch 的 URL，异步加载，加载完成前
    /// 不画。
    pub fn image(
        &mut self,
        rect: LayoutBox,
        source: impl Into<Arc<str>>,
        fit: ImageFit,
        radii: impl Into<CornerRadii>,
    ) {
        let source = source.into();
        if source.is_empty() {
            return;
        }
        let CornerRadii(radii) = radii.into();
        let radii = fit_radii(rect, radii.map(|radius| self.radius(radius)));
        self.push(PaintOp::Image {
            rect,
            source,
            fit,
            sampling: self.image_sampling,
            radii,
            src_rect: None,
            nine_slice: None,
            nine_slice_fill: true,
        });
    }

    /// Draws an image described by [`PaintImage`] (source crop, 9-slice, fit,
    /// corner radii, and smoothing).
    pub fn draw_image(&mut self, rect: LayoutBox, image: PaintImage) {
        if image.source.is_empty() {
            return;
        }
        let radii = fit_radii(rect, image.radii);
        self.push(PaintOp::Image {
            rect,
            source: image.source,
            fit: image.fit,
            sampling: image.sampling.unwrap_or(self.image_sampling),
            radii,
            src_rect: image.src_rect,
            nine_slice: image.nine_slice,
            nine_slice_fill: image.nine_slice_fill,
        });
    }

    /// Draws one [`TextureRegion`] sub-rectangle into `dest`.
    pub fn draw_texture_region(&mut self, region: &TextureRegion, dest: LayoutBox) {
        if region.source.is_empty() || dest.width <= 0.0 || dest.height <= 0.0 {
            return;
        }
        self.push(PaintOp::Image {
            rect: dest,
            source: Arc::clone(&region.source),
            fit: ImageFit::Fill,
            sampling: self.image_sampling,
            radii: [0.0; 4],
            src_rect: Some(region.rect),
            nine_slice: None,
            nine_slice_fill: true,
        });
    }

    /// Draws a batch of atlas [`TextureRegion`] sprites without creating UI
    /// nodes. Contiguous regions sharing the same source texture emit a single
    /// batched draw call.
    pub fn draw_texture_regions(&mut self, items: &[(&TextureRegion, LayoutBox)]) {
        let mut index = 0;
        while index < items.len() {
            let (first_region, first_dest) = items[index];
            index += 1;
            if first_region.source.is_empty() || first_dest.width <= 0.0 || first_dest.height <= 0.0
            {
                continue;
            }
            let source = Arc::clone(&first_region.source);
            let mut batch = vec![(first_region.rect, first_dest)];
            while index < items.len() {
                let (region, dest) = items[index];
                if region.source.as_ref() != source.as_ref() {
                    break;
                }
                if dest.width > 0.0 && dest.height > 0.0 {
                    batch.push((region.rect, dest));
                }
                index += 1;
            }
            self.push(PaintOp::TextureRegionBatch {
                source,
                sampling: self.image_sampling,
                items: batch,
            });
        }
    }

    /// 之后的绘制裁剪到 `path` 以内（与已有裁剪取交集），直到 [`Self::pop_clip`]。
    ///
    /// 只作用于这个 painter 录下的内容：节点自己的阴影若画在 `push_clip` 之前，
    /// 不会被裁掉；子节点也不受影响。
    pub fn push_clip(&mut self, path: &PaintPath) {
        self.clip_depth += 1;
        self.open.push(OpenScope::Clip);
        self.sync_state();
        self.ops.push(PaintOp::PushClip {
            path: Arc::new(path.clone()),
        });
    }

    /// 撤掉最近一个 [`Self::push_clip`]；没有裁剪时什么都不做。
    pub fn pop_clip(&mut self) {
        if let Some(at) = self.open.iter().rposition(|s| *s == OpenScope::Clip) {
            self.open.remove(at);
            self.clip_depth -= 1;
            self.ops.push(PaintOp::PopClip);
        }
    }

    fn finish(mut self) -> (Vec<PaintOp>, bool) {
        // What was left open closes innermost first.
        while let Some(scope) = self.open.pop() {
            self.ops.push(match scope {
                OpenScope::Layer => PaintOp::PopLayer,
                OpenScope::Mask => PaintOp::PopMask,
                OpenScope::Clip => PaintOp::PopClip,
            });
        }
        (self.ops, self.default_drawn)
    }
}

/// What a painter's text is shaped as: the node's resolved style with the
/// text's own size, weight, slant, letter spacing, and line height.
pub(crate) fn paint_text_request(
    base: &crate::ComputedStyle,
    text: &PaintText,
    size: f32,
    max_width: Option<f32>,
) -> (
    crate::ComputedStyle,
    crate::TextShapeConstraints,
    crate::TextContent,
) {
    let mut style = base.clone();
    style.font_size = size;
    style.font_weight = text.weight.or(style.font_weight);
    if let Some(family) = &text.family {
        style.font_family = Some(Arc::clone(family));
    }
    style.italic = text.italic;
    style.letter_spacing = if text.letter_spacing.is_finite() {
        text.letter_spacing
    } else {
        0.0
    };
    style.line_height = text.line_height.map(nana_ui_core::LineHeightSpec::Absolute);
    let constraints = crate::TextShapeConstraints {
        max_width,
        wrap: text.wrap && max_width.is_some(),
        max_lines: text.max_lines,
        preserve_lines: text.wrap || text.max_lines.is_some(),
        ..crate::TextShapeConstraints::default()
    };
    let content = crate::TextContent {
        value: text.content.to_string().into(),
    };
    (style, constraints, content)
}

pub(crate) fn paint_text_layout(
    engine: &nana_text::SharedTextEngine,
    base: &crate::ComputedStyle,
    text: &PaintText,
    size: f32,
    max_width: Option<f32>,
) -> Arc<nana_text::TextLayout> {
    let (style, constraints, content) = paint_text_request(base, text, size, max_width);
    let nana_style = crate::text_node::nana_text_style(&style);
    let nana_constraints =
        crate::text_node::nana_text_constraints(&style, &constraints, text.horizontal);
    let kind = crate::text_node::text_kind(&constraints);
    let source = nana_text::TextSource::new(content.value.as_str());
    let mut counters = nana_text::TextWorkCounters::default();
    let mut engine = nana_text::lock_text_engine(engine);
    engine.layout(kind, &source, &nana_style, &nana_constraints, &mut counters)
}

pub(crate) fn paint_text_metrics_from_engine(
    engine: &nana_text::SharedTextEngine,
    base: &crate::ComputedStyle,
    text: &PaintText,
    size: f32,
    max_width: Option<f32>,
) -> PaintTextMetrics {
    let layout = paint_text_layout(engine, base, text, size, max_width);
    let (width, height) = layout.physical_size();
    let lines: Vec<PaintTextLineMetrics> = layout
        .lines
        .iter()
        .map(|line| PaintTextLineMetrics {
            width: line.metrics.width_px,
            baseline: line.metrics.baseline_y_px,
            line_height: line.metrics.height_px,
            ascent: line.metrics.ascent_px,
            descent: line.metrics.descent_px,
        })
        .collect();
    let first = lines.first().copied();
    let ascent = first.map_or(size * nana_ui_core::TEXT_APPROX_ASCENT_EM, |l| l.ascent);
    let descent = first.map_or((size * 1.2 - ascent).max(0.0), |l| l.descent);
    let baseline = first.map_or(ascent, |l| l.baseline);
    let ink = nana_text::lock_text_engine(engine).layout_ink_bounds(&layout);
    let (actual_left, actual_right, actual_ascent, actual_descent) = match ink {
        Some(ink) => (
            -ink.x,
            ink.right(),
            baseline - ink.y,
            ink.bottom() - baseline,
        ),
        None => (0.0, 0.0, 0.0, 0.0),
    };
    PaintTextMetrics {
        width,
        height,
        baseline,
        ascent,
        descent,
        actual_bounding_box_left: actual_left,
        actual_bounding_box_right: actual_right,
        actual_bounding_box_ascent: actual_ascent,
        actual_bounding_box_descent: actual_descent,
        lines,
    }
}

pub(crate) fn paint_text_to_path(
    engine: &nana_text::SharedTextEngine,
    base: &crate::ComputedStyle,
    text: &PaintText,
    size: f32,
    origin: [f32; 2],
    max_width: Option<f32>,
) -> PaintPath {
    if text.content.is_empty() {
        return PaintPath::new();
    }
    struct PathSink(PaintPath);
    impl nana_text::OutlineSink for PathSink {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.move_to(x, y);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.line_to(x, y);
        }
        fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
            self.0.quad_to(cx, cy, x, y);
        }
        fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
            self.0.cubic_to(cx0, cy0, cx1, cy1, x, y);
        }
        fn close(&mut self) {
            self.0.close();
        }
    }
    let layout = paint_text_layout(engine, base, text, size, max_width);
    let mut sink = PathSink(PaintPath::new());
    nana_text::lock_text_engine(engine).emit_layout_outlines(&layout, origin, &mut sink);
    sink.0
}

pub(crate) fn paint_text_metrics_fallback(
    metrics: &crate::TextMetrics,
    size: f32,
) -> PaintTextMetrics {
    let ascent = metrics
        .ascent
        .unwrap_or(size * nana_ui_core::TEXT_APPROX_ASCENT_EM);
    let line_height = metrics.height.max(ascent);
    let descent = (line_height - ascent).max(0.0);
    PaintTextMetrics {
        width: metrics.width,
        height: metrics.height,
        baseline: ascent,
        ascent,
        descent,
        actual_bounding_box_left: 0.0,
        actual_bounding_box_right: metrics.width,
        actual_bounding_box_ascent: ascent,
        actual_bounding_box_descent: descent,
        lines: if metrics.width > 0.0 || metrics.height > 0.0 {
            vec![PaintTextLineMetrics {
                width: metrics.width,
                baseline: ascent,
                line_height,
                ascent,
                descent,
            }]
        } else {
            Vec::new()
        },
    }
}

impl PaintRecording {
    /// 在给定主题、尺寸和交互状态下录一次 `painter`，文字按 em 估算测量。
    ///
    /// 用于在没有 `UiWorld` 的地方检查一个 painter 录下了什么，比如它的单元
    /// 测试；挂在节点上时由框架录制，文字用宿主的排版引擎测量。
    pub fn record(
        painter: &dyn Painter,
        theme: &CompiledTheme,
        size: [f32; 2],
        state: PaintState,
    ) -> Self {
        record(painter, theme, size, state, &EmPaintTextHost)
    }

    /// 把 `painter` 当作一张 `size` 大、分段为 `sections` 的模态对话框卡面
    /// 录一次，文字按 em 估算测量。
    ///
    /// 用于在没有 `UiWorld` 的地方检查一个卡面 painter 录下了什么；挂在
    /// [`crate::Dialog::surface_painter`] 上时由框架录制。
    pub fn record_modal_surface(
        painter: &dyn Painter,
        theme: &CompiledTheme,
        size: [f32; 2],
        sections: crate::ModalSurfaceSections,
    ) -> Self {
        record_surface(painter, theme, size, sections, &EmPaintTextHost)
    }
}

/// 录一次：先背景阶段，再前景阶段。
pub(crate) fn record(
    painter: &dyn Painter,
    theme: &CompiledTheme,
    size: [f32; 2],
    state: PaintState,
    text_host: &dyn PaintTextHost,
) -> PaintRecording {
    let mut behind = PaintContext::new(theme, size, state, text_host, false);
    painter.paint(&mut behind);
    let (behind_children, default_drawn) = behind.finish();
    let mut over = PaintContext::new(theme, size, state, text_host, default_drawn);
    painter.paint_over_children(&mut over);
    let (over_children, _) = over.finish();
    PaintRecording {
        behind_children,
        over_children,
    }
}

/// 录一张模态对话框卡面：只有背景阶段。卡面画在遮罩之上、卡片的文字与
/// 插槽之下，没有「子节点之上」可言，所以 `paint_over_children` 不录。
pub(crate) fn record_surface(
    painter: &dyn Painter,
    theme: &CompiledTheme,
    size: [f32; 2],
    sections: crate::ModalSurfaceSections,
    text_host: &dyn PaintTextHost,
) -> PaintRecording {
    let mut surface = PaintContext::new(theme, size, PaintState::default(), text_host, false);
    surface.sections = Some(sections);
    painter.paint(&mut surface);
    let (behind_children, _) = surface.finish();
    PaintRecording {
        behind_children,
        over_children: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentId, MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld};
    use nana_ui_core::{SemanticPalette, ThemeDefinition, UI_METRICS};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fills its box with a role and a tier, and counts how often it is asked.
    struct Counting {
        key: u64,
        role: SemanticColorRole,
        calls: Arc<AtomicUsize>,
    }

    impl Painter for Counting {
        fn paint(&self, cx: &mut PaintContext<'_>) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let radius = cx.radius(RadiusTier::Md);
            let mut path = PaintPath::new();
            path.rounded_rect(cx.bounds(), [radius; 4]);
            cx.fill_path(&path, self.role);
        }

        fn paint_key(&self) -> u64 {
            self.key
        }
    }

    fn record_plain(painter: &dyn Painter, size: [f32; 2]) -> PaintRecording {
        record(
            painter,
            nana_ui_core::builtin_theme_arc(ThemeAppearance::Dark).as_ref(),
            size,
            PaintState::default(),
            &EmPaintTextHost,
        )
    }

    fn node() -> StableNodeId {
        StableNodeId::new(1).unwrap()
    }

    fn painted_world(painter: Counting) -> UiWorld {
        painted_world_with(painter)
    }

    fn painted_world_with(painter: impl Painter) -> UiWorld {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(),
            DocumentId::new(1).unwrap(),
            NodeKind::Element { tag: "div".into() },
        );
        queue.set_style(node(), NodeStyle::default().painter(painter));
        queue.write_layout(
            node(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 40.0,
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world
    }

    fn filled(world: &UiWorld) -> ([f32; 4], Arc<PaintPath>) {
        let extracted = world.extract_nodes(&[node()]);
        let recording = extracted[0]
            .custom_paint
            .clone()
            .expect("a painted node carries its recording");
        match &recording.behind_children[..] {
            [
                PaintOp::FillPath {
                    path,
                    paint: ResolvedPaint::Solid(color),
                },
            ] => (*color, Arc::clone(path)),
            other => panic!("unexpected recording {other:?}"),
        }
    }

    fn commit(world: &mut UiWorld, queue: MutationQueue) -> crate::SystemWork {
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        work
    }

    #[test]
    fn a_path_answers_what_it_covers_and_moves_and_parses_from_svg() {
        let path = PaintPath::from_svg("M0 0 H20 V10 H0 Z").unwrap();
        assert!(path.contains([10.0, 5.0]) && !path.contains([25.0, 5.0]));
        let stroke = StrokeStyle::new(2.0);
        assert!(path.stroke_contains([20.5, 5.0], &stroke));
        assert!(!path.stroke_contains([10.0, 5.0], &stroke));
        let moved = path.transformed([2.0, 0.0, 0.0, 1.0, 5.0, 0.0]);
        let bounds = moved.bounds().unwrap();
        assert_eq!(
            (bounds.x, bounds.y, bounds.width, bounds.height),
            (5.0, 0.0, 40.0, 10.0)
        );
        assert!(PaintPath::from_svg("M0 0 L").is_err());
        assert_eq!(PaintPath::new().bounds(), None);
    }

    #[test]
    fn a_painter_records_on_its_own_and_what_it_leaves_open_closes_innermost_first() {
        struct Open;
        impl Painter for Open {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                cx.push_layer(0.5, BlendMode::Normal);
                cx.push_clip(&PaintPath::from_svg("M0 0 H10 V10 Z").unwrap());
                cx.push_layer(1.0, BlendMode::Normal);
            }
            fn paint_key(&self) -> u64 {
                0
            }
        }
        let theme = nana_ui_core::builtin_theme_arc(ThemeAppearance::Dark);
        let recording = PaintRecording::record(&Open, &theme, [10.0, 10.0], PaintState::default());
        let closes: Vec<_> = recording.behind_children[3..]
            .iter()
            .map(|op| matches!(op, PaintOp::PopLayer))
            .collect();
        assert_eq!(closes, [true, false, true]);
    }

    #[test]
    fn an_unchanged_node_is_never_re_recorded() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut world = painted_world(Counting {
            key: 1,
            role: SemanticColorRole::Surface,
            calls: Arc::clone(&calls),
        });
        let (color, _) = filled(&world);
        assert_eq!(color, SemanticPalette::dark().surface.as_rgba_array());
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        world.extract_nodes(&[node()]);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "extraction alone");

        // A view rebuilt every frame hands in a fresh painter with the same key.
        let mut queue = MutationQueue::new();
        queue.set_style(
            node(),
            NodeStyle::default().painter(Counting {
                key: 1,
                role: SemanticColorRole::Surface,
                calls: Arc::clone(&calls),
            }),
        );
        let work = commit(&mut world, queue);
        assert!(
            work.render_extraction.is_empty(),
            "same key is the same style"
        );
        world.extract_nodes(&[node()]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_new_key_re_records_without_relayout() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut world = painted_world(Counting {
            key: 1,
            role: SemanticColorRole::Surface,
            calls: Arc::clone(&calls),
        });
        filled(&world);
        let mut queue = MutationQueue::new();
        queue.set_style(
            node(),
            NodeStyle::default().painter(Counting {
                key: 2,
                role: SemanticColorRole::Accent,
                calls: Arc::clone(&calls),
            }),
        );
        let work = commit(&mut world, queue);
        assert!(
            work.layout_frontier_seeds.is_empty(),
            "a colour role is paint, not layout"
        );
        assert_eq!(work.render_extraction, vec![node()]);
        let (color, _) = filled(&world);
        assert_eq!(color, SemanticPalette::dark().accent.as_rgba_array());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_resize_re_records_at_the_new_size() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut world = painted_world(Counting {
            key: 1,
            role: SemanticColorRole::Surface,
            calls: Arc::clone(&calls),
        });
        filled(&world);
        let mut queue = MutationQueue::new();
        queue.write_layout(
            node(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 160.0,
                height: 40.0,
            },
        );
        commit(&mut world, queue);
        let (_, path) = filled(&world);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(
            path.verbs()
                .iter()
                .any(|verb| matches!(verb, PathVerb::LineTo([x, _]) if (*x - 160.0).abs() < 1e-3)),
            "the outline reaches the new right edge"
        );
    }

    #[test]
    fn a_theme_switch_follows_the_new_palette_and_radius_tier() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut world = painted_world(Counting {
            key: 1,
            role: SemanticColorRole::Surface,
            calls: Arc::clone(&calls),
        });
        let (_, dark_path) = filled(&world);

        let mut metrics = UI_METRICS;
        metrics.radius_md += 6.0;
        let theme = ThemeDefinition::for_appearance(ThemeAppearance::Light)
            .with_metrics(metrics)
            .with_palette(SemanticPalette::light())
            .compile()
            .expect("a test theme compiles");
        let mut queue = MutationQueue::new();
        queue.set_theme_tokens(Arc::new(theme));
        commit(&mut world, queue);

        let (color, light_path) = filled(&world);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(color, SemanticPalette::light().surface.as_rgba_array());
        // The first straight edge starts where the top-left corner ends.
        let start = |path: &PaintPath| match path.verbs()[0] {
            PathVerb::MoveTo([x, _]) => x,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(start(&dark_path), UI_METRICS.radius_md);
        assert_eq!(start(&light_path), metrics.radius_md);
    }

    #[test]
    fn draw_default_is_recorded_once_and_clips_are_balanced() {
        struct Layered;
        impl Painter for Layered {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                let mut clip = PaintPath::new();
                clip.rect(cx.bounds());
                cx.push_clip(&clip);
                cx.draw_default();
                cx.draw_default();
            }
            fn paint_over_children(&self, cx: &mut PaintContext<'_>) {
                cx.draw_default();
                cx.pop_clip();
            }
            fn paint_key(&self) -> u64 {
                0
            }
        }
        let recording = record_plain(&Layered, [10.0, 10.0]);
        assert_eq!(recording.default_phase(), Some(PaintPhase::BehindChildren));
        assert!(matches!(
            &recording.behind_children[..],
            [
                PaintOp::PushClip { .. },
                PaintOp::DrawDefault,
                PaintOp::PopClip
            ]
        ));
        assert!(recording.over_children.is_empty());
    }

    #[test]
    fn arc_to_rounds_a_concave_corner_tangent_to_both_edges() {
        // Down the left edge of a cut-out, then along its floor to the right:
        // the corner at (0, 20) turns the other way from the outline's
        // convex corners.
        let mut path = PaintPath::new();
        path.move_to(0.0, 0.0).arc_to(0.0, 20.0, 40.0, 20.0, 8.0);
        let verbs = path.verbs();
        assert_eq!(verbs[1], PathVerb::LineTo([0.0, 12.0]));
        let PathVerb::CubicTo(c1, c2, end) = verbs[2] else {
            panic!("an arc follows the tangent point");
        };
        assert!((end[0] - 8.0).abs() < 1e-4 && (end[1] - 20.0).abs() < 1e-4);
        // Tangent continuity: the handles leave along the incoming edge and
        // arrive along the outgoing one.
        assert!(c1[0].abs() < 1e-4 && c1[1] > 12.0);
        assert!((c2[1] - 20.0).abs() < 1e-4 && c2[0] < 8.0);
    }

    #[test]
    fn a_path_that_is_one_rounded_rect_says_so() {
        let rect = LayoutBox {
            x: 1.0,
            y: 2.0,
            width: 30.0,
            height: 20.0,
        };
        let mut path = PaintPath::new();
        path.rounded_rect(rect, [4.0; 4]);
        assert_eq!(path.as_rounded_rect(), Some((rect, 4.0)));
        path.line_to(0.0, 0.0);
        assert_eq!(path.as_rounded_rect(), None);
    }

    #[test]
    fn the_transform_stack_is_recorded_ahead_of_the_ops_it_governs() {
        struct Turned;
        impl Painter for Turned {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                let mut dot = PaintPath::new();
                dot.rect(LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                });
                cx.save();
                cx.translate(10.0, 20.0);
                cx.scale(2.0, 2.0);
                cx.translate(1.0, 0.0);
                cx.fill_path(&dot, [1.0; 4]);
                cx.fill_path(&dot, [1.0; 4]);
                cx.restore();
                cx.fill_path(&dot, [1.0; 4]);
                // Changing the transform and drawing nothing records nothing.
                cx.rotate(1.0);
            }
            fn paint_key(&self) -> u64 {
                0
            }
        }
        let recording = record_plain(&Turned, [10.0, 10.0]);
        let ops = &recording.behind_children;
        assert!(
            matches!(
                ops[..],
                [
                    PaintOp::SetTransform([2.0, 0.0, 0.0, 2.0, 12.0, 20.0]),
                    PaintOp::FillPath { .. },
                    PaintOp::FillPath { .. },
                    PaintOp::SetTransform(AFFINE_IDENTITY),
                    PaintOp::FillPath { .. },
                ]
            ),
            "{ops:?}"
        );
    }

    #[test]
    fn a_gradient_resolves_its_roles_and_sorts_its_stops() {
        let theme = nana_ui_core::builtin_theme_arc(ThemeAppearance::Dark);
        let cx = PaintContext::new(
            theme.as_ref(),
            [10.0, 10.0],
            PaintState::default(),
            &EmPaintTextHost,
            false,
        );
        let ResolvedPaint::Gradient(gradient) = cx.paint(
            Gradient::linear([0.0, 0.0], [10.0, 0.0])
                .stop(1.4, SemanticColorRole::Accent)
                .stop(0.25, [1.0, 0.0, 0.0, 1.0])
                .stop(f32::NAN, [0.0; 4])
                .extend(GradientExtend::Reflect),
        ) else {
            panic!("a gradient");
        };
        assert_eq!(
            gradient.stops,
            vec![
                (0.25, [1.0, 0.0, 0.0, 1.0]),
                (1.0, SemanticPalette::dark().accent.as_rgba_array()),
            ]
        );
        assert_eq!(gradient.extend, GradientExtend::Reflect);
    }

    #[test]
    fn explicit_authoring_paint_keeps_linear_sc_rgb_until_scene_upload() {
        let theme = nana_ui_core::builtin_theme_arc(ThemeAppearance::Dark);
        let cx = PaintContext::new(
            theme.as_ref(),
            [10.0, 10.0],
            PaintState::default(),
            &EmPaintTextHost,
            false,
        );
        let color = nana_ui_core::PaintColor::LinearScRgb {
            channels: [1.25, -0.125, 0.25],
            alpha: 0.7,
        };
        assert_eq!(cx.paint(color), ResolvedPaint::Authoring(color));
        let ResolvedPaint::Gradient(gradient) = cx.paint(
            Gradient::linear([0.0, 0.0], [10.0, 0.0])
                .stop(0.0, color)
                .stop(1.0, [0.0, 0.0, 0.0, 1.0]),
        ) else {
            panic!("explicit gradient");
        };
        let stops = gradient.linear_stops.as_ref().expect("linear stops");
        assert!((stops[0].1[0] - 1.25).abs() <= 1.0e-6);
        assert!((stops[0].1[1] + 0.125).abs() <= 1.0e-6);
        assert!((stops[0].1[3] - 0.7).abs() <= 1.0e-6);
    }

    /// Records the state it was painted in.
    struct Stateful(Arc<std::sync::Mutex<Vec<PaintState>>>);

    impl Painter for Stateful {
        fn paint(&self, cx: &mut PaintContext<'_>) {
            self.0.lock().unwrap().push(cx.state());
        }
        fn paint_key(&self) -> u64 {
            0
        }
    }

    #[test]
    fn a_state_change_re_records_with_the_new_state() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let document = DocumentId::new(1).unwrap();
        queue.create(node(), document, NodeKind::Element { tag: "div".into() });
        queue.set_style(
            node(),
            NodeStyle::default().painter(Stateful(Arc::clone(&seen))),
        );
        queue.set_interaction(
            node(),
            crate::InteractionState {
                pointer_events: true,
                focusable: false,
            },
        );
        queue.write_layout(
            node(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
        );
        commit(&mut world, queue);
        let recording = |world: &UiWorld| {
            world.extract_nodes(&[node()])[0]
                .custom_paint
                .clone()
                .expect("painted")
        };
        let before = recording(&world);
        world.set_pointer_hover(document, 1, Some(node())).unwrap();
        let work = world.take_system_work();
        assert!(
            work.render_extraction.contains(&node()),
            "hover repaints a painter"
        );
        let after = recording(&world);
        world.extract_nodes(&[node()]);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "once per state, not per extraction");
        assert!(!seen[0].hovered && seen[1].hovered, "{seen:?}");
        // It paints the same whatever the state: the scene keeps its
        // triangles, which it reuses by identity.
        assert!(Arc::ptr_eq(&before, &after));
    }

    #[test]
    fn a_painter_that_measured_text_re_records_on_a_font_change_only() {
        struct Measures(Arc<AtomicUsize>);
        impl Painter for Measures {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                self.0.fetch_add(1, Ordering::SeqCst);
                cx.measure_text(&PaintText::new("label"), None);
            }
            fn paint_key(&self) -> u64 {
                0
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mut world = painted_world_with(Measures(Arc::clone(&calls)));
        world.extract_nodes(&[node()]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let restyle = |world: &mut UiWorld, edit: &dyn Fn(&mut nana_ui_core::LayoutStyle)| {
            let mut style = NodeStyle::default().painter(Measures(Arc::clone(&calls)));
            edit(Arc::make_mut(&mut style.layout));
            let mut queue = MutationQueue::new();
            queue.set_style(node(), style);
            commit(world, queue);
            world.extract_nodes(&[node()]);
        };
        // A new resolved style that measures the same: no re-record.
        restyle(&mut world, &|layout| {
            layout.paint_colors.color = Some(nana_ui_core::PaintColor::srgb([1.0, 0.0, 0.0, 1.0]))
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Another family measures differently.
        restyle(&mut world, &|layout| {
            layout.paint_colors.color = Some(nana_ui_core::PaintColor::srgb([1.0, 0.0, 0.0, 1.0]));
            layout.text_ext.font_family = Some("Serif".into());
        });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn text_is_measured_and_wraps_when_narrowed() {
        struct Measures(Arc<std::sync::Mutex<Vec<PaintTextMetrics>>>);
        impl Painter for Measures {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                let text = PaintText::new("a label long enough to wrap")
                    .size(12.0)
                    .wrap(true);
                let mut sizes = self.0.lock().unwrap();
                sizes.push(cx.measure_text(&text, None));
                sizes.push(cx.measure_text(&text, Some(40.0)));
            }
            fn paint_key(&self) -> u64 {
                0
            }
        }
        let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let world = painted_world_with(Measures(Arc::clone(&sizes)));
        world.extract_nodes(&[node()]);
        let sizes = sizes.lock().unwrap();
        let (line, wrapped) = (&sizes[0], &sizes[1]);
        assert!(line.width > 40.0 && line.height > 0.0, "{line:?}");
        assert!(wrapped.width <= 40.0 + 1e-3, "{wrapped:?}");
        assert!(
            wrapped.height > line.height * 1.5,
            "{wrapped:?} vs {line:?}"
        );
        assert!(line.baseline > 0.0 && line.baseline < line.height);
    }

    #[test]
    fn opacity_layers_and_blends_are_recorded_around_the_ops_they_govern() {
        struct Layered;
        impl Painter for Layered {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                let mut dot = PaintPath::new();
                dot.rect(cx.bounds());
                cx.set_opacity(0.5);
                cx.fill_path(&dot, [1.0; 4]);
                cx.push_layer(0.25, BlendMode::Normal);
                cx.set_blend(BlendMode::Multiply);
                cx.fill_path(&dot, [1.0; 4]);
                // An unbalanced layer is closed at the end of the phase.
            }
            fn paint_key(&self) -> u64 {
                0
            }
        }
        let ops = record_plain(&Layered, [10.0, 10.0]).behind_children;
        assert!(
            matches!(
                ops[..],
                [
                    PaintOp::SetOpacity(0.5),
                    PaintOp::FillPath { .. },
                    PaintOp::PushLayer {
                        opacity: 0.25,
                        blend: BlendMode::Normal
                    },
                    PaintOp::PushLayer {
                        opacity: 1.0,
                        blend: BlendMode::Multiply
                    },
                    PaintOp::FillPath { .. },
                    PaintOp::PopLayer,
                    PaintOp::PopLayer,
                ]
            ),
            "{ops:?}"
        );
    }

    #[test]
    fn a_painter_decides_which_points_of_its_box_hit() {
        /// Hits only in a disc, and paints only its left half.
        struct Disc {
            painted: bool,
        }
        impl Painter for Disc {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                let mut half = PaintPath::new();
                half.rect(LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 50.0,
                    height: 100.0,
                });
                cx.fill_path(&half, [1.0; 4]);
            }
            fn hit_test(&self, local: [f32; 2], size: [f32; 2]) -> Option<bool> {
                (!self.painted).then(|| {
                    let (dx, dy) = (local[0] - size[0] / 2.0, local[1] - size[1] / 2.0);
                    dx * dx + dy * dy <= 50.0 * 50.0
                })
            }
            fn hit_painted_outline(&self) -> bool {
                self.painted
            }
            fn paint_key(&self) -> u64 {
                u64::from(self.painted)
            }
        }
        for painted in [false, true] {
            let mut world = UiWorld::new();
            let document = DocumentId::new(1).unwrap();
            let mut queue = MutationQueue::new();
            queue.create(node(), document, NodeKind::Element { tag: "div".into() });
            queue.set_style(node(), NodeStyle::default().painter(Disc { painted }));
            queue.set_interaction(
                node(),
                crate::InteractionState {
                    pointer_events: true,
                    focusable: false,
                },
            );
            queue.write_layout(
                node(),
                LayoutBox {
                    x: 10.0,
                    y: 10.0,
                    width: 100.0,
                    height: 100.0,
                },
            );
            commit(&mut world, queue);
            world.rebuild_hit_test(document);
            let hit = |x, y| world.hit_test(document, x, y) == Some(node());
            if painted {
                assert!(
                    hit(30.0, 100.0) && !hit(90.0, 30.0),
                    "only the painted half"
                );
            } else {
                assert!(hit(60.0, 60.0), "the disc's centre");
                assert!(!hit(13.0, 13.0), "a corner of the box, outside the disc");
            }
        }
    }

    #[test]
    fn a_painted_outline_hit_follows_a_re_record_without_an_index_rebuild() {
        /// Paints its right half only while selected.
        struct Grows;
        impl Painter for Grows {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                let mut half = PaintPath::new();
                half.rect(LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: if cx.state().selected { 100.0 } else { 50.0 },
                    height: 100.0,
                });
                cx.fill_path(&half, [1.0; 4]);
            }
            fn hit_painted_outline(&self) -> bool {
                true
            }
            fn paint_key(&self) -> u64 {
                0
            }
        }
        let mut world = UiWorld::new();
        let document = DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(node(), document, NodeKind::Element { tag: "div".into() });
        queue.set_style(node(), NodeStyle::default().painter(Grows));
        queue.set_interaction(
            node(),
            crate::InteractionState {
                pointer_events: true,
                focusable: false,
            },
        );
        queue.write_layout(
            node(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        );
        commit(&mut world, queue);
        world.rebuild_hit_test(document);
        assert_ne!(world.hit_test(document, 80.0, 50.0), Some(node()));
        let mut queue = MutationQueue::new();
        queue.set_accessibility(
            node(),
            crate::AccessibilityState {
                selected: Some(true),
                ..Default::default()
            },
        );
        commit(&mut world, queue);
        // The frame extracts (and re-records) the node; the hit index keeps
        // the entry it built, which reads the recording through its slot.
        world.extract_nodes(&[node()]);
        assert_eq!(world.hit_test(document, 80.0, 50.0), Some(node()));
    }

    #[test]
    fn a_dashed_stroke_hits_only_on_its_dashes() {
        let mut line = PaintPath::new();
        line.move_to(0.0, 5.0).line_to(40.0, 5.0);
        let recording = PaintRecording {
            behind_children: vec![PaintOp::StrokePath {
                path: Arc::new(line),
                paint: ResolvedPaint::Solid([1.0; 4]),
                stroke: StrokeStyle::new(4.0).dash(vec![10.0, 10.0], 0.0),
            }],
            over_children: Vec::new(),
        };
        assert!(recording.contains([5.0, 5.0], [40.0, 10.0]));
        assert!(!recording.contains([15.0, 5.0], [40.0, 10.0]), "a gap");
        assert!(recording.contains([25.0, 6.5], [40.0, 10.0]));
    }

    #[test]
    fn a_painter_set_on_the_node_outlives_style_rewrites() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut world = painted_world(Counting {
            key: 1,
            role: SemanticColorRole::Surface,
            calls: Arc::clone(&calls),
        });
        let mut queue = MutationQueue::new();
        queue.set_painter(
            node(),
            Some(NodePainter::new(Counting {
                key: 7,
                role: SemanticColorRole::Accent,
                calls: Arc::clone(&calls),
            })),
        );
        commit(&mut world, queue);
        let (color, _) = filled(&world);
        assert_eq!(
            color,
            SemanticPalette::dark().accent.as_rgba_array(),
            "the node's painter wins"
        );
        // A component projecting a style without a painter leaves it on.
        let mut queue = MutationQueue::new();
        queue.set_style(node(), NodeStyle::default());
        commit(&mut world, queue);
        let (color, _) = filled(&world);
        assert_eq!(color, SemanticPalette::dark().accent.as_rgba_array());
        let mut queue = MutationQueue::new();
        queue.set_painter(node(), None);
        let work = commit(&mut world, queue);
        assert_eq!(work.render_extraction, vec![node()]);
        assert!(world.extract_nodes(&[node()])[0].custom_paint.is_none());
    }

    #[test]
    fn path_boolean_ops_cover_all_four_ops_self_intersection_evenodd_and_empty() {
        let mut left = PaintPath::new();
        left.rect(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        });
        let mut right = PaintPath::new();
        right.rect(LayoutBox {
            x: 10.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        });

        let u = left.union(&right);
        assert!(u.contains([5.0, 10.0]));
        assert!(u.contains([15.0, 10.0]));
        assert!(u.contains([25.0, 10.0]));
        assert!(!u.contains([35.0, 10.0]));

        let i = left.intersect(&right);
        assert!(!i.contains([5.0, 10.0]));
        assert!(i.contains([15.0, 10.0]));
        assert!(!i.contains([25.0, 10.0]));

        let d = left.difference(&right);
        assert!(d.contains([5.0, 10.0]));
        assert!(!d.contains([15.0, 10.0]));
        assert!(!d.contains([25.0, 10.0]));

        let x = left.xor(&right);
        assert!(x.contains([5.0, 10.0]));
        assert!(!x.contains([15.0, 10.0]));
        assert!(x.contains([25.0, 10.0]));

        // Disjoint intersection and self-contained difference return empty paths.
        let mut far = PaintPath::new();
        far.rect(LayoutBox {
            x: 100.0,
            y: 100.0,
            width: 10.0,
            height: 10.0,
        });
        assert!(left.intersect(&far).is_empty());
        assert!(left.difference(&left).is_empty());

        // Self-intersecting bowtie: NonZero fills the center cross, EvenOdd still fills two lobes.
        let mut bowtie = PaintPath::new();
        bowtie
            .move_to(0.0, 0.0)
            .line_to(20.0, 20.0)
            .line_to(20.0, 0.0)
            .line_to(0.0, 20.0)
            .close();
        let bowtie_u = bowtie.union(&PaintPath::new());
        assert!(bowtie_u.contains([3.0, 10.0]));
        assert!(bowtie_u.contains([17.0, 10.0]));

        // EvenOdd concentric square cuts out the inner hole before boolean ops.
        let mut holed = PaintPath::new().with_fill_rule(FillRule::EvenOdd);
        holed
            .rect(LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 40.0,
            })
            .rect(LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 20.0,
                height: 20.0,
            });
        let mut strip = PaintPath::new();
        strip.rect(LayoutBox {
            x: 15.0,
            y: 0.0,
            width: 10.0,
            height: 40.0,
        });
        let holed_intersect = holed.intersect(&strip);
        assert!(holed_intersect.contains([20.0, 5.0]));
        assert!(!holed_intersect.contains([20.0, 20.0]), "hole preserved");
        assert!(holed_intersect.contains([20.0, 35.0]));

        // Curved inputs flatten at PATH_FLATTEN_TOLERANCE into a polyline usable for stroke_contains.
        let mut circle = PaintPath::new();
        circle.ellipse(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        });
        let clipped_circle = circle.difference(&right);
        assert!(clipped_circle.verbs().iter().all(|v| matches!(
            v,
            PathVerb::MoveTo(_) | PathVerb::LineTo(_) | PathVerb::Close
        )));
        assert!(clipped_circle.contains([5.0, 10.0]));
        assert!(!clipped_circle.contains([15.0, 10.0]));
        assert!(clipped_circle.stroke_contains([10.0, 5.0], &StrokeStyle::new(2.0)));
    }

    fn bundled_test_text_engine() -> nana_text::SharedTextEngine {
        let mut policy = nana_text::font::FallbackPolicy::empty();
        policy.set_generic(nana_text::font::GenericFamily::SansSerif, ["Noto Sans SC"]);
        let mut fonts = nana_text::font::FontSystem::with_policy(policy);
        fonts
            .register_file(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../nana-ui-core/assets/fonts/NotoSansSC-Variable.ttf"
                ),
                &nana_text::font::FaceDescriptor::default(),
            )
            .expect("the bundled UI face registers");
        Arc::new(std::sync::Mutex::new(nana_text::NativeTextEngine::new(
            fonts,
        )))
    }

    #[test]
    fn text_path_and_metrics_unify_with_nana_text_and_path_booleans() {
        let engine = bundled_test_text_engine();

        let base_text = PaintText::new("Ág\nxy").size(24.0).wrap(true);
        let spaced = PaintText::new("Ág\nxy")
            .size(24.0)
            .wrap(true)
            .letter_spacing(6.0);
        let m1 = paint_text_metrics_from_engine(
            &engine,
            &crate::ComputedStyle::default(),
            &base_text,
            24.0,
            Some(200.0),
        );
        let m2 = paint_text_metrics_from_engine(
            &engine,
            &crate::ComputedStyle::default(),
            &spaced,
            24.0,
            Some(200.0),
        );
        assert_eq!(m1.lines.len(), 2);
        assert!(m1.lines[1].baseline > m1.lines[0].baseline);
        assert!(m1.ascent > 0.0 && m1.descent > 0.0);
        assert!(m1.actual_bounding_box_ascent > 0.0);
        assert!(m1.actual_bounding_box_descent > 0.0);
        assert!(m1.actual_bounding_box_right > 0.0);
        assert!(m2.width > m1.width, "letter_spacing widens text");

        // PaintPath::from_text produces outlines usable in booleans.
        let text_outline = PaintPath::from_text(
            &engine,
            &PaintText::new("HI").size(32.0),
            [10.0, 10.0],
            None,
        );
        assert!(!text_outline.is_empty());
        let bounds = text_outline.bounds().unwrap();
        let mut plate = PaintPath::new();
        plate.rect(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: bounds.x + bounds.width + 20.0,
            height: bounds.y + bounds.height + 20.0,
        });
        let punched = plate.difference(&text_outline);
        assert!(!punched.is_empty());
        assert!(punched.contains([2.0, 2.0]));
    }

    #[test]
    fn paint_text_records_letter_spacing_decorations_stroke_and_shadows() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(),
            DocumentId::new(1).unwrap(),
            NodeKind::Element { tag: "div".into() },
        );
        struct StyledText;
        impl Painter for StyledText {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                let text = PaintText::new("Hi")
                    .size(16.0)
                    .letter_spacing(2.5)
                    .decoration(
                        PaintTextDecoration::underline()
                            .with_overline(true)
                            .style(nana_ui_core::TextDecorationStyle::Wavy)
                            .color([1.0, 0.0, 0.0, 1.0])
                            .thickness(2.0),
                    )
                    .stroke(PaintTextStroke::new(1.5, [0.0, 0.0, 0.0, 1.0]))
                    .shadow(PaintTextShadow::new([1.0, 2.0], 3.0, [0.0, 0.0, 0.0, 0.5]))
                    .shadow(PaintTextShadow::new([3.0, 4.0], 6.0, [0.0, 0.0, 1.0, 0.4]));
                cx.text(cx.bounds(), text);
            }
            fn paint_key(&self) -> u64 {
                1
            }
        }
        queue.set_style(node(), NodeStyle::default().painter(StyledText));
        queue.write_layout(
            node(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 40.0,
            },
        );
        commit(&mut world, queue);
        let extracted = world.extract_nodes(&[node()]);
        let recording = extracted[0].custom_paint.as_ref().unwrap();
        let [
            PaintOp::Text {
                letter_spacing,
                effects: Some(effects),
                ..
            },
        ] = &recording.behind_children[..]
        else {
            panic!("expected styled PaintOp::Text: {recording:?}");
        };
        assert_eq!(*letter_spacing, 2.5);
        assert!(
            effects
                .decoration
                .as_ref()
                .is_some_and(|d| d.line.underline && d.line.overline)
        );
        assert!(effects.stroke.is_some());
        assert_eq!(effects.shadows.len(), 2);
    }

    #[test]
    fn custom_paint_text_path_and_metrics_invalidate_without_costing_unused_nodes() {
        let mut world = UiWorld::new();
        let mut shaper = crate::NanaTextEngineShaper::new(bundled_test_text_engine());
        let doc = DocumentId::new(1).unwrap();
        let text_node = StableNodeId::new(1).unwrap();
        let plain_node = StableNodeId::new(2).unwrap();
        let text_calls = Arc::new(AtomicUsize::new(0));
        let plain_calls = Arc::new(AtomicUsize::new(0));

        struct OutlinePainter(Arc<AtomicUsize>);
        impl Painter for OutlinePainter {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                self.0.fetch_add(1, Ordering::SeqCst);
                let spec = PaintText::new("HI").letter_spacing(1.5);
                let metrics = cx.measure_text(&spec, None);
                assert!(metrics.width > 0.0 && metrics.ascent > 0.0);
                let outline = cx.text_path(&spec, [4.0, 4.0], None);
                assert!(!outline.is_empty());
                cx.fill_path(&outline, [0.2, 0.4, 0.8, 1.0]);
            }
            fn paint_key(&self) -> u64 {
                77
            }
        }
        struct PlainPainter(Arc<AtomicUsize>);
        impl Painter for PlainPainter {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                self.0.fetch_add(1, Ordering::SeqCst);
                cx.rounded_rect(cx.bounds(), 0.0, BoxPaint::fill([0.1, 0.1, 0.1, 1.0]));
            }
            fn paint_key(&self) -> u64 {
                88
            }
        }

        let mut queue = MutationQueue::new();
        for id in [text_node, plain_node] {
            queue.create(id, doc, NodeKind::Element { tag: "div".into() });
            queue.write_layout(
                id,
                LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 80.0,
                },
            );
        }
        queue.set_style(
            text_node,
            NodeStyle::default().painter(OutlinePainter(Arc::clone(&text_calls))),
        );
        queue.set_style(
            plain_node,
            NodeStyle::default().painter(PlainPainter(Arc::clone(&plain_calls))),
        );
        commit(&mut world, queue);

        let work = world.take_system_work();
        world.shape_text(&work.text, &mut shaper).unwrap();
        let initial_extracted = world.extract_nodes(&[text_node, plain_node]);
        assert!(matches!(
            &initial_extracted[0].custom_paint.as_ref().unwrap().behind_children[0],
            PaintOp::FillPath { path, .. } if !path.is_empty()
        ));
        assert_eq!(text_calls.load(Ordering::SeqCst), 1);
        assert_eq!(plain_calls.load(Ordering::SeqCst), 1);

        // Font change on plain_node does not re-record it (zero text dependency overhead),
        // whereas font change on text_node triggers re-recording because it used text_path/measure_text.
        let mut queue = MutationQueue::new();
        queue.set_style(
            text_node,
            NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    font_size: Some(28.0),
                    ..nana_ui_core::LayoutStyle::default()
                }),
                ..NodeStyle::default().painter(OutlinePainter(Arc::clone(&text_calls)))
            },
        );
        queue.set_style(
            plain_node,
            NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    font_size: Some(28.0),
                    ..nana_ui_core::LayoutStyle::default()
                }),
                ..NodeStyle::default().painter(PlainPainter(Arc::clone(&plain_calls)))
            },
        );
        commit(&mut world, queue);
        world.extract_nodes(&[text_node, plain_node]);
        assert_eq!(text_calls.load(Ordering::SeqCst), 2);
        assert_eq!(plain_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn porter_duff_and_mask_hit_testing_matches_compositing() {
        let make_rect = |x, y, width, height| {
            let mut p = PaintPath::new();
            p.rect(LayoutBox {
                x,
                y,
                width,
                height,
            });
            p
        };
        let full = make_rect(0.0, 0.0, 40.0, 40.0);
        let hole = make_rect(10.0, 10.0, 20.0, 20.0);

        struct CutoutPainter {
            full: PaintPath,
            hole: PaintPath,
        }
        impl Painter for CutoutPainter {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                cx.fill_path(&self.full, [1.0, 0.0, 0.0, 1.0]);
                cx.set_blend(BlendMode::DestinationOut);
                cx.fill_path(&self.hole, [0.0, 0.0, 0.0, 1.0]);
            }
            fn paint_key(&self) -> u64 {
                901
            }
        }
        let rec_cut = record_plain(
            &CutoutPainter {
                full: full.clone(),
                hole: hole.clone(),
            },
            [40.0, 40.0],
        );
        assert!(rec_cut.contains([5.0, 5.0], [40.0, 40.0]));
        assert!(!rec_cut.contains([20.0, 20.0], [40.0, 40.0]));

        struct LumMaskPainter {
            full: PaintPath,
            bright: PaintPath,
            dark: PaintPath,
        }
        impl Painter for LumMaskPainter {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                cx.fill_path(&self.full, [0.0, 1.0, 0.0, 1.0]);
                cx.push_mask(MaskKind::Luminance);
                cx.fill_path(&self.bright, [1.0, 1.0, 1.0, 1.0]);
                cx.fill_path(&self.dark, [0.0, 0.0, 0.0, 1.0]);
                cx.pop_mask();
            }
            fn paint_key(&self) -> u64 {
                902
            }
        }
        let rec_mask = record_plain(
            &LumMaskPainter {
                full,
                bright: make_rect(0.0, 0.0, 20.0, 40.0),
                dark: make_rect(20.0, 0.0, 20.0, 40.0),
            },
            [40.0, 40.0],
        );
        assert!(rec_mask.contains([10.0, 20.0], [40.0, 40.0]));
        assert!(!rec_mask.contains([30.0, 20.0], [40.0, 40.0]));
    }

    #[test]
    fn gradient_pattern_image_and_smoothing_builders_record_p3_fields() {
        struct P3Painter;
        impl Painter for P3Painter {
            fn paint(&self, cx: &mut PaintContext<'_>) {
                let make_rect = |x, y, width, height| {
                    let mut p = PaintPath::new();
                    p.rect(LayoutBox {
                        x,
                        y,
                        width,
                        height,
                    });
                    p
                };
                let grad = Gradient::radial_two_circle([10.0, 10.0], 2.0, [20.0, 20.0], 18.0)
                    .stop(0.0, [1.0, 0.0, 0.0, 1.0])
                    .stop(1.0, [0.0, 0.0, 1.0, 1.0])
                    .color_space(GradientColorSpace::Oklab)
                    .transform([2.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                cx.fill_path(&make_rect(0.0, 0.0, 20.0, 20.0), grad);

                let pattern = PaintPattern::new("atlas.png")
                    .with_src_rect(LayoutBox {
                        x: 4.0,
                        y: 4.0,
                        width: 8.0,
                        height: 8.0,
                    })
                    .with_repeat(PatternRepeat::RepeatX)
                    .with_smoothing(false);
                cx.fill_path(&make_rect(0.0, 20.0, 20.0, 20.0), pattern);

                cx.save();
                cx.set_image_smoothing(false);
                assert!(!cx.image_smoothing());
                assert_eq!(cx.image_sampling(), nana_ui_core::ImageSampling::Nearest);
                cx.draw_image(
                    LayoutBox {
                        x: 20.0,
                        y: 0.0,
                        width: 20.0,
                        height: 20.0,
                    },
                    PaintImage::new("frame.png")
                        .with_src_rect(LayoutBox {
                            x: 0.0,
                            y: 0.0,
                            width: 16.0,
                            height: 16.0,
                        })
                        .with_nine_slice([4.0, 4.0, 4.0, 4.0], true),
                );
                cx.restore();
                assert!(cx.image_smoothing());

                let r1 = TextureRegion::new(
                    "atlas.png",
                    LayoutBox {
                        x: 0.0,
                        y: 0.0,
                        width: 8.0,
                        height: 8.0,
                    },
                );
                let r2 = TextureRegion::new(
                    "atlas.png",
                    LayoutBox {
                        x: 8.0,
                        y: 0.0,
                        width: 8.0,
                        height: 8.0,
                    },
                );
                cx.draw_texture_regions(&[
                    (
                        &r1,
                        LayoutBox {
                            x: 20.0,
                            y: 20.0,
                            width: 10.0,
                            height: 10.0,
                        },
                    ),
                    (
                        &r2,
                        LayoutBox {
                            x: 30.0,
                            y: 20.0,
                            width: 10.0,
                            height: 10.0,
                        },
                    ),
                ]);
            }
            fn paint_key(&self) -> u64 {
                903
            }
        }

        let rec = record_plain(&P3Painter, [40.0, 40.0]);
        assert!(matches!(
            &rec.behind_children[0],
            PaintOp::FillPath {
                paint: ResolvedPaint::Gradient(g),
                ..
            } if g.color_space == GradientColorSpace::Oklab
                && matches!(g.shape, GradientShape::RadialTwoCircle { .. })
                && g.transform == [2.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        ));
        assert!(matches!(
            &rec.behind_children[1],
            PaintOp::FillPath {
                paint: ResolvedPaint::Pattern(p),
                ..
            } if p.repeat == PatternRepeat::RepeatX
                && p.sampling == nana_ui_core::ImageSampling::Nearest
        ));
        assert!(matches!(
            &rec.behind_children[2],
            PaintOp::Image {
                sampling: nana_ui_core::ImageSampling::Nearest,
                src_rect: Some(_),
                nine_slice: Some([4.0, 4.0, 4.0, 4.0]),
                nine_slice_fill: true,
                ..
            }
        ));
        assert!(matches!(
            &rec.behind_children[3],
            PaintOp::TextureRegionBatch { items, .. } if items.len() == 2
        ));
    }
}
