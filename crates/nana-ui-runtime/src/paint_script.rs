//! 数据驱动的节点自绘（Issue #217）：把一份 JSON 命令列表当作 [`Painter`]。
//!
//! 给拿不到 Rust trait 的消费方用：命令与 [`PaintContext`] 的方法
//! 一一对应，录制时按节点当前尺寸和交互状态回放。格式见
//! [`PaintScript::from_json`]。

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use nana_ui_core::{
    ElevationRole, RadiusTier, SemanticColorMix, SemanticColorRole, TextDecorationStyle,
    TextStrokeJoin, TextStrokePlacement,
};
use serde_json::Value;

use crate::{
    BlendMode, BoxPaint, ColorStop, CornerRadii, FillRule, Gradient, GradientExtend, ImageFit,
    LayoutBox, LineCap, LineJoin, Paint, PaintColor, PaintContext, PaintPath, PaintPhase,
    PaintShadow, PaintState, PaintText, PaintTextDecoration, PaintTextShadow, PaintTextStroke,
    Painter, PathBooleanOp, Radius, StrokeStyle, TextHorizontalAlignment, TextVerticalAlignment,
};

/// 一份解析好的绘制脚本。相同的 JSON 得到相同的 `paint_key`。
#[derive(Debug, Clone)]
pub struct PaintScript {
    commands: Arc<[Command]>,
    hit: HitMode,
    key: u64,
}

/// Which points of the node a script's painter hits.
#[derive(Debug, Clone)]
enum HitMode {
    /// The whole layout box.
    Bounds,
    /// What the recording paints.
    Painted,
    /// This path, in the node's coordinates.
    Path(ScriptPath),
}

impl PaintScript {
    /// 解析脚本。顶层是命令数组，或 `{ "commands": [...], "hit": ... }`。`hit`
    /// 缺省为 `"bounds"`（整个布局矩形可命中），`"painted"` 为录下的内容，
    /// `{ "path": 路径 }` 为这条路径围成的区域。
    ///
    /// 每条命令是带 `"op"` 的对象，可选 `"phase": "behind" | "over"`（缺省
    /// `behind` 画在子节点下面，`over` 画在上面）和 `"when": { "hovered": true,
    /// ... }`（只在这些交互状态下执行，键为 `hovered` / `pressed` / `focused` /
    /// `disabled` / `selected`）。命令里出现它不认识的键、或布尔字段不是
    /// `true` / `false` 时整份脚本解析失败，不会悄悄按缺省值画。
    ///
    /// - 长度：数字为像素；字符串 `"50%"`、`"100% - 12"`、`"50% + 4"` 相对节点
    ///   宽（x、宽）或高（y、高）。半径类长度（圆角、`arc` / `arcTo` 半径、
    ///   径向渐变半径）的百分比相对节点较短的一边，`"50%"` 即内切圆。
    /// - 颜色：语义角色名（`"accent"`、`"surface"` …）、`[r, g, b]` 或
    ///   `[r, g, b, a]`（`0..=1`）；主题相关的混色写 `{ "mix": [角色, 角色, 比例] }`
    ///   （比例是第一个角色的份量）或 `{ "alpha": [角色, 不透明度] }`。CSS
    ///   字符串由拥有 CSS 引擎的上游适配器通过 [`Self::from_json_with_color_parser`]
    ///   解析；Runtime 本身不解析 CSS。
    /// - 上色：颜色，或渐变 `{ "linear": [x0, y0, x1, y1] | "radial": [cx, cy, r]
    ///   | "conic": [cx, cy, 起始弧度], "stops": [[偏移, 颜色], …], "extend":
    ///   "pad" | "repeat" | "reflect" }`，色标进入 Scene 后在 linear scRGB 中按
    ///   premultiplied 方式插值。
    /// - 路径：SVG path 字符串（`M L H V C S Q T A Z`，大小写均可），或分段数组
    ///   `[["M", x, y], ["L", x, y], ["Q", cx, cy, x, y], ["C", …6], ["arcTo",
    ///   拐角x, 拐角y, 终点x, 终点y, r], ["arc", cx, cy, r, 起始, 扫过], ["rect",
    ///   x, y, w, h], ["roundedRect", x, y, w, h, 圆角或四个圆角], ["ellipse", x, y,
    ///   w, h], ["Z"]]`，分段里的长度可以写相对值；要指定填充规则时写成
    ///   `{ "d": 上面任一种, "fillRule": "nonzero" | "evenodd" }`。
    /// - 圆角：长度，档位名 `"xs" | "sm" | "md" | "lg" | "xl" | "xxl" | "full"`（`"xxl"` 是
    ///   2xl 档），或四个这样的值（左上、右上、右下、左下）。
    /// - 混合方式：CSS `mix-blend-mode` 的全部取值（`"normal"`、`"multiply"`、
    ///   `"screen"`、`"overlay"`、`"darken"`、`"lighten"`、`"color-dodge"`、
    ///   `"color-burn"`、`"hard-light"`、`"soft-light"`、`"difference"`、
    ///   `"exclusion"`、`"hue"`、`"saturation"`、`"color"`、`"luminosity"`）。
    ///
    /// 命令：`fill`（`path`、`paint`）、`stroke`（`path`、`paint`、`width` 缺省
    /// 1、`join: "miter" | "round" | "bevel"` 缺省 `miter`、`cap: "butt" |
    /// "round" | "square"` 缺省 `butt`、`dash`、`dashOffset`、`miterLimit` 缺省
    /// 10）、`shadow`（`path`，以及 `elevation: "surface" | "overlay"` 或
    /// `color`、`offset: [x, y]`、`blur`、`spread`、`inset`）、`roundedRect`
    /// （`rect: [x, y, w, h]`、`radii`、`fill`、`border: [颜色, 宽度]`、
    /// `shadow`：同 `shadow` 命令的字段）、`text`（`rect`、`text`、`paint`、
    /// `size`、`weight`、`italic`、`lineHeight`、`wrap`、`maxLines`、`align:
    /// ["start"|"center"|"end", "top"|"center"|"bottom"]`）、`image`（`rect`、
    /// `src`、`fit`、`radii`）、`icon`（`rect`、`name`：内建图标名，`paint`）、
    /// `fillImage`（`path`、`rect`、`src`、`fit`）、`save`、`restore`、
    /// `translate`（`x`、`y`，可写相对长度）、`scale`（`x`，`y` 缺省同 `x`）、
    /// `rotate`（`angle` 弧度）、`transform`（`matrix: [a, b, c, d, e, f]`，乘到
    /// 当前变换上）、`setTransform`（`matrix`，替换当前变换）、`opacity`
    /// （`value`）、`blend`（`mode`）、`pushLayer`（`opacity`、`blend`）、
    /// `popLayer`、`pushClip`（`path`）、`popClip`、`drawDefault`。`fit` 为
    /// `"fill" | "contain" | "cover" | "none" | "scale-down"`，缺省 `contain`。
    ///
    /// 脚本是静态的：需要读回结果的 `measure_text`、自定义 `hit_test` 逻辑只在
    /// Rust 的 [`Painter`] 里有。
    pub fn from_json(value: &Value) -> Result<Self, String> {
        Self::from_json_impl(value, None)
    }

    /// Parse a script while delegating CSS color strings to the caller.
    ///
    /// The runtime deliberately has no dependency on `nana-ui-css`. Hosts
    /// that own CSS input can pass that crate's canonical parser here and map
    /// its typed result to [`nana_ui_core::PaintColor`].
    pub fn from_json_with_color_parser(
        value: &Value,
        color_parser: &dyn Fn(&str) -> Option<nana_ui_core::PaintColor>,
    ) -> Result<Self, String> {
        Self::from_json_impl(value, Some(color_parser))
    }

    /// Parse a JSON script while delegating CSS color strings to the caller.
    pub fn from_json_str_with_color_parser(
        text: &str,
        color_parser: &dyn Fn(&str) -> Option<nana_ui_core::PaintColor>,
    ) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        Self::from_json_with_color_parser(&value, color_parser)
    }

    fn from_json_impl(value: &Value, color_parser: ColorParser<'_>) -> Result<Self, String> {
        if !numbers_fit_f32(value) {
            return Err("a number does not fit a 32-bit float".into());
        }
        let (list, hit) = match value {
            Value::Array(list) => (list.as_slice(), None),
            Value::Object(map) => {
                only_keys(map, &["commands", "hit"], "a paint script")?;
                (
                    map.get("commands")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .ok_or("a paint script object needs `commands`")?,
                    map.get("hit"),
                )
            }
            _ => return Err("a paint script is an array or an object".into()),
        };
        let hit = match hit {
            None | Some(Value::Null) => HitMode::Bounds,
            Some(Value::String(mode)) if mode == "bounds" => HitMode::Bounds,
            Some(Value::String(mode)) if mode == "painted" => HitMode::Painted,
            Some(Value::Object(shape)) if shape.len() == 1 && shape.contains_key("path") => {
                HitMode::Path(
                    parse_path(&shape["path"], color_parser)
                        .map_err(|error| format!("hit: {error}"))?,
                )
            }
            Some(_) => {
                return Err("`hit` is \"bounds\", \"painted\" or { \"path\": 路径 }".into());
            }
        };
        let commands = list
            .iter()
            .enumerate()
            .map(|(index, command)| {
                Command::parse(command, color_parser)
                    .map_err(|error| format!("command {index}: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.to_string().hash(&mut hasher);
        Ok(Self {
            commands: commands.into(),
            hit,
            key: hasher.finish(),
        })
    }

    /// 从 JSON 文本解析，格式同 [`Self::from_json`]。
    pub fn from_json_str(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        Self::from_json(&value)
    }

    fn replay(&self, cx: &mut PaintContext<'_>, phase: PaintPhase) {
        let size = cx.size();
        let state = cx.state();
        for command in self
            .commands
            .iter()
            .filter(|command| command.phase == phase && command.when.admits(state))
        {
            command.op.apply(cx, size);
        }
    }
}

impl Painter for PaintScript {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        self.replay(cx, PaintPhase::BehindChildren);
    }

    fn paint_over_children(&self, cx: &mut PaintContext<'_>) {
        self.replay(cx, PaintPhase::OverChildren);
    }

    fn hit_test(&self, local: [f32; 2], size: [f32; 2]) -> Option<bool> {
        match &self.hit {
            HitMode::Path(path) => Some(path.build(None, size).contains(local)),
            HitMode::Bounds | HitMode::Painted => None,
        }
    }

    fn hit_painted_outline(&self) -> bool {
        matches!(self.hit, HitMode::Painted)
    }

    fn paint_key(&self) -> u64 {
        self.key
    }
}

#[derive(Debug, Clone)]
struct Command {
    phase: PaintPhase,
    when: StateFilter,
    op: Op,
}

#[derive(Debug, Clone, Copy, Default)]
struct StateFilter {
    hovered: Option<bool>,
    pressed: Option<bool>,
    focused: Option<bool>,
    disabled: Option<bool>,
    selected: Option<bool>,
}

impl StateFilter {
    fn admits(self, state: PaintState) -> bool {
        [
            (self.hovered, state.hovered),
            (self.pressed, state.pressed),
            (self.focused, state.focused),
            (self.disabled, state.disabled),
            (self.selected, state.selected),
        ]
        .into_iter()
        .all(|(want, is)| want.is_none_or(|want| want == is))
    }
}

/// A radius: a length against the node's shorter side (`"50%"` of it makes a
/// circle of a square), or a theme tier.
#[derive(Debug, Clone, Copy)]
enum ScriptRadius {
    Len(Len),
    Tier(RadiusTier),
}

impl ScriptRadius {
    fn resolve(self, size: [f32; 2]) -> Radius {
        match self {
            Self::Len(len) => Radius::Px(len.resolve(size[0].min(size[1]))),
            Self::Tier(tier) => Radius::Tier(tier),
        }
    }
}

fn radii_of(radii: &[ScriptRadius; 4], size: [f32; 2]) -> CornerRadii {
    CornerRadii(radii.map(|radius| radius.resolve(size)))
}

/// A length against the node's shorter side, for a radius a path needs in
/// pixels.
fn short_side(len: Len, size: [f32; 2]) -> f32 {
    len.resolve(size[0].min(size[1]))
}

/// A length: px, or a fraction of the node's size on its axis plus px.
#[derive(Debug, Clone, Copy)]
struct Len {
    fraction: f32,
    px: f32,
}

impl Len {
    fn resolve(self, extent: f32) -> f32 {
        self.fraction * extent + self.px
    }
}

#[derive(Debug, Clone)]
enum Op {
    Fill {
        path: ScriptPath,
        paint: ScriptPaint,
    },
    Stroke {
        path: ScriptPath,
        paint: ScriptPaint,
        stroke: StrokeStyle,
    },
    Shadow {
        path: ScriptPath,
        shadow: PaintShadow,
    },
    RoundedRect {
        rect: [Len; 4],
        radii: [ScriptRadius; 4],
        fill: Option<ScriptPaint>,
        border: Option<(PaintColor, f32)>,
        shadow: Option<PaintShadow>,
    },
    Text {
        rect: [Len; 4],
        text: PaintTextSpec,
    },
    Image {
        rect: [Len; 4],
        src: Arc<str>,
        fit: ImageFit,
        radii: [ScriptRadius; 4],
    },
    Icon {
        rect: [Len; 4],
        icon: nana_ui_core::Icon,
        paint: ScriptPaint,
    },
    FillImage {
        path: ScriptPath,
        rect: [Len; 4],
        src: Arc<str>,
        fit: ImageFit,
    },
    Save,
    Restore,
    Translate(Len, Len),
    Scale(f32, f32),
    Rotate(f32),
    Transform([f32; 6]),
    SetTransform([f32; 6]),
    Opacity(f32),
    Blend(BlendMode),
    PushLayer(f32, BlendMode),
    PopLayer,
    PushClip(ScriptPath),
    PopClip,
    DrawDefault,
}

#[derive(Debug, Clone)]
struct PaintTextStrokeSpec {
    width: f32,
    paint: ScriptPaint,
    join: TextStrokeJoin,
    placement: TextStrokePlacement,
}

#[derive(Debug, Clone)]
struct PaintTextSpec {
    content: Arc<str>,
    paint: ScriptPaint,
    size: Option<f32>,
    weight: Option<u16>,
    family: Option<Arc<str>>,
    italic: bool,
    line_height: Option<f32>,
    letter_spacing: f32,
    wrap: bool,
    max_lines: Option<u16>,
    horizontal: TextHorizontalAlignment,
    vertical: TextVerticalAlignment,
    decoration: Option<PaintTextDecoration>,
    stroke: Option<PaintTextStrokeSpec>,
    shadows: Vec<PaintTextShadow>,
}

impl PaintTextSpec {
    fn to_paint_text(&self, size: [f32; 2]) -> PaintText {
        let mut paint_text = PaintText::new(Arc::clone(&self.content))
            .paint(self.paint.resolve(size))
            .italic(self.italic)
            .letter_spacing(self.letter_spacing)
            .wrap(self.wrap)
            .align(self.horizontal, self.vertical);
        paint_text.size = self.size;
        paint_text.weight = self.weight;
        paint_text.family = self.family.clone();
        paint_text.line_height = self.line_height;
        paint_text.max_lines = self.max_lines;
        paint_text.decoration = self.decoration;
        paint_text.stroke = self.stroke.as_ref().map(|s| {
            PaintTextStroke::new(s.width, s.paint.resolve(size))
                .join(s.join)
                .placement(s.placement)
        });
        paint_text.shadows = self.shadows.clone();
        paint_text
    }
}

impl Op {
    fn apply(&self, cx: &mut PaintContext<'_>, size: [f32; 2]) {
        match self {
            Self::Fill { path, paint } => {
                let path = path.build(Some(cx), size);
                cx.fill_path(&path, paint.resolve(size));
            }
            Self::Stroke {
                path,
                paint,
                stroke,
            } => {
                let path = path.build(Some(cx), size);
                cx.stroke_path(&path, stroke.clone(), paint.resolve(size));
            }
            Self::Shadow { path, shadow } => {
                let path = path.build(Some(cx), size);
                cx.shadow(&path, *shadow);
            }
            Self::RoundedRect {
                rect,
                radii,
                fill,
                border,
                shadow,
            } => cx.rounded_rect(
                rect_of(rect, size),
                radii_of(radii, size),
                BoxPaint {
                    fill: fill.as_ref().map(|fill| fill.resolve(size)),
                    border: *border,
                    shadow: *shadow,
                },
            ),
            Self::Text { rect, text } => {
                cx.text(rect_of(rect, size), text.to_paint_text(size));
            }
            Self::Image {
                rect,
                src,
                fit,
                radii,
            } => cx.image(
                rect_of(rect, size),
                Arc::clone(src),
                *fit,
                radii_of(radii, size),
            ),
            Self::Icon { rect, icon, paint } => {
                cx.icon(rect_of(rect, size), *icon, paint.resolve(size));
            }
            Self::FillImage {
                path,
                rect,
                src,
                fit,
            } => {
                let path = path.build(Some(cx), size);
                cx.fill_path_with_image(&path, rect_of(rect, size), Arc::clone(src), *fit);
            }
            Self::Save => cx.save(),
            Self::Restore => cx.restore(),
            Self::Translate(x, y) => cx.translate(x.resolve(size[0]), y.resolve(size[1])),
            Self::Scale(x, y) => cx.scale(*x, *y),
            Self::Rotate(angle) => cx.rotate(*angle),
            Self::Transform(matrix) => cx.concat(*matrix),
            Self::SetTransform(matrix) => cx.set_transform(*matrix),
            Self::Opacity(value) => cx.set_opacity(*value),
            Self::Blend(mode) => cx.set_blend(*mode),
            Self::PushLayer(opacity, blend) => cx.push_layer(*opacity, *blend),
            Self::PopLayer => cx.pop_layer(),
            Self::PushClip(path) => {
                let path = path.build(Some(cx), size);
                cx.push_clip(&path);
            }
            Self::PopClip => cx.pop_clip(),
            Self::DrawDefault => cx.draw_default(),
        }
    }
}

fn rect_of(rect: &[Len; 4], size: [f32; 2]) -> LayoutBox {
    LayoutBox {
        x: rect[0].resolve(size[0]),
        y: rect[1].resolve(size[1]),
        width: rect[2].resolve(size[0]),
        height: rect[3].resolve(size[1]),
    }
}

#[derive(Debug, Clone)]
enum ScriptPaint {
    Color(PaintColor),
    Gradient {
        shape: GradientSpec,
        stops: Vec<(f32, PaintColor)>,
        extend: GradientExtend,
    },
}

#[derive(Debug, Clone, Copy)]
enum GradientSpec {
    Linear([Len; 4]),
    Radial([Len; 2], Len),
    Conic([Len; 2], f32),
}

impl ScriptPaint {
    fn resolve(&self, size: [f32; 2]) -> Paint {
        match self {
            Self::Color(color) => Paint::Color(*color),
            Self::Gradient {
                shape,
                stops,
                extend,
            } => {
                let point = |x: Len, y: Len| [x.resolve(size[0]), y.resolve(size[1])];
                let mut gradient = match *shape {
                    GradientSpec::Linear([x0, y0, x1, y1]) => {
                        Gradient::linear(point(x0, y0), point(x1, y1))
                    }
                    GradientSpec::Radial([x, y], radius) => {
                        Gradient::radial(point(x, y), short_side(radius, size))
                    }
                    GradientSpec::Conic([x, y], angle) => Gradient::conic(point(x, y), angle),
                };
                gradient.stops = stops
                    .iter()
                    .map(|(offset, color)| ColorStop {
                        offset: *offset,
                        color: *color,
                    })
                    .collect();
                Paint::Gradient(gradient.extend(*extend))
            }
        }
    }
}

#[derive(Debug, Clone)]
enum ScriptPath {
    Segments {
        segments: Arc<[Segment]>,
        fill_rule: FillRule,
    },
    Boolean {
        op: PathBooleanOp,
        paths: Arc<[ScriptPath]>,
        fill_rule: Option<FillRule>,
    },
    TextPath {
        text: Box<PaintTextSpec>,
        origin: [Len; 2],
        max_width: Option<Len>,
        fill_rule: Option<FillRule>,
    },
}

#[derive(Debug, Clone)]
enum Segment {
    Move(Len, Len),
    Line(Len, Len),
    Quad([Len; 4]),
    Cubic([Len; 6]),
    ArcTo([Len; 4], Len),
    Arc([Len; 2], Len, f32, f32),
    Rect([Len; 4]),
    RoundedRect([Len; 4], [Len; 4]),
    Ellipse([Len; 4]),
    Close,
}

impl ScriptPath {
    fn build(&self, cx: Option<&PaintContext<'_>>, size: [f32; 2]) -> PaintPath {
        match self {
            Self::Segments {
                segments,
                fill_rule,
            } => {
                let x = |len: Len| len.resolve(size[0]);
                let y = |len: Len| len.resolve(size[1]);
                let rect = |r: &[Len; 4]| LayoutBox {
                    x: x(r[0]),
                    y: y(r[1]),
                    width: x(r[2]),
                    height: y(r[3]),
                };
                let mut path = PaintPath::new().with_fill_rule(*fill_rule);
                for segment in segments.iter() {
                    match segment {
                        Segment::Move(a, b) => {
                            path.move_to(x(*a), y(*b));
                        }
                        Segment::Line(a, b) => {
                            path.line_to(x(*a), y(*b));
                        }
                        Segment::Quad(v) => {
                            path.quad_to(x(v[0]), y(v[1]), x(v[2]), y(v[3]));
                        }
                        Segment::Cubic(v) => {
                            path.cubic_to(x(v[0]), y(v[1]), x(v[2]), y(v[3]), x(v[4]), y(v[5]));
                        }
                        Segment::ArcTo(v, radius) => {
                            path.arc_to(
                                x(v[0]),
                                y(v[1]),
                                x(v[2]),
                                y(v[3]),
                                short_side(*radius, size),
                            );
                        }
                        Segment::Arc(center, radius, start, sweep) => {
                            path.arc(
                                x(center[0]),
                                y(center[1]),
                                short_side(*radius, size),
                                *start,
                                *sweep,
                            );
                        }
                        Segment::Rect(r) => {
                            path.rect(rect(r));
                        }
                        Segment::RoundedRect(r, radii) => {
                            path.rounded_rect(
                                rect(r),
                                radii.map(|radius| short_side(radius, size)),
                            );
                        }
                        Segment::Ellipse(r) => {
                            path.ellipse(rect(r));
                        }
                        Segment::Close => {
                            path.close();
                        }
                    }
                }
                path
            }
            Self::Boolean {
                op,
                paths,
                fill_rule,
            } => {
                let mut iter = paths.iter();
                let Some(first) = iter.next() else {
                    return PaintPath::new();
                };
                let mut acc = first.build(cx, size);
                let mut any = false;
                for next in iter {
                    acc = acc.boolean(&next.build(cx, size), *op);
                    any = true;
                }
                if !any {
                    // Simplify a single-entry boolean path through self-union.
                    acc = acc.union(&PaintPath::new());
                }
                if let Some(rule) = fill_rule {
                    acc.set_fill_rule(*rule);
                }
                acc
            }
            Self::TextPath {
                text,
                origin,
                max_width,
                fill_rule,
            } => {
                let Some(cx) = cx else {
                    return PaintPath::new();
                };
                let mut path = cx.text_path(
                    &text.to_paint_text(size),
                    [origin[0].resolve(size[0]), origin[1].resolve(size[1])],
                    max_width.map(|w| w.resolve(size[0])),
                );
                if let Some(rule) = fill_rule {
                    path.set_fill_rule(*rule);
                }
                path
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

type Map = serde_json::Map<String, Value>;
type ColorParser<'a> = Option<&'a dyn Fn(&str) -> Option<nana_ui_core::PaintColor>>;

impl Command {
    fn parse(value: &Value, color_parser: ColorParser<'_>) -> Result<Self, String> {
        let map = value.as_object().ok_or("a command is an object")?;
        let op = map
            .get("op")
            .and_then(Value::as_str)
            .ok_or("a command needs `op`")?;
        let phase = match map.get("phase").and_then(Value::as_str) {
            None | Some("behind") => PaintPhase::BehindChildren,
            Some("over") => PaintPhase::OverChildren,
            Some(other) => return Err(format!("unknown phase `{other}`")),
        };
        let mut when = StateFilter::default();
        if let Some(filter) = map.get("when") {
            let filter = filter.as_object().ok_or("`when` is an object")?;
            for (key, value) in filter {
                let value = value.as_bool().ok_or("`when` values are booleans")?;
                let slot = match key.as_str() {
                    "hovered" => &mut when.hovered,
                    "pressed" => &mut when.pressed,
                    "focused" => &mut when.focused,
                    "disabled" => &mut when.disabled,
                    "selected" => &mut when.selected,
                    other => return Err(format!("unknown state `{other}`")),
                };
                *slot = Some(value);
            }
        }
        Ok(Self {
            phase,
            when,
            op: parse_op(op, map, color_parser)?,
        })
    }
}

/// The keys each op reads, past `op`, `phase` and `when`.
fn op_keys(op: &str) -> Option<&'static [&'static str]> {
    Some(match op {
        "fill" => &["path", "paint"],
        "stroke" => &[
            "path",
            "paint",
            "width",
            "join",
            "cap",
            "dash",
            "dashOffset",
            "miterLimit",
        ],
        "shadow" => &[
            "path",
            "elevation",
            "color",
            "offset",
            "blur",
            "spread",
            "inset",
        ],
        "roundedRect" => &["rect", "radii", "fill", "border", "shadow"],
        "text" => &[
            "rect",
            "text",
            "paint",
            "size",
            "weight",
            "family",
            "italic",
            "lineHeight",
            "letterSpacing",
            "wrap",
            "maxLines",
            "align",
            "decoration",
            "stroke",
            "shadows",
        ],
        "image" => &["rect", "src", "fit", "radii"],
        "icon" => &["rect", "name", "paint"],
        "fillImage" => &["path", "rect", "src", "fit"],
        "translate" | "scale" => &["x", "y"],
        "rotate" => &["angle"],
        "transform" | "setTransform" => &["matrix"],
        "opacity" => &["value"],
        "blend" => &["mode"],
        "pushLayer" => &["opacity", "blend"],
        "pushClip" => &["path"],
        "save" | "restore" | "popLayer" | "popClip" | "drawDefault" => &[],
        _ => return None,
    })
}

/// Reject a key nothing reads: a misspelt one would otherwise paint with a
/// default and say nothing.
fn only_keys(map: &Map, allowed: &[&str], what: &str) -> Result<(), String> {
    match map.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(format!("unknown key `{key}` in {what}")),
        None => Ok(()),
    }
}

/// A boolean field, `default` when absent.
fn flag(map: &Map, key: &str, default: bool) -> Result<bool, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("`{key}` is true or false")),
    }
}

fn matrix(map: &Map) -> Result<[f32; 6], String> {
    let values = required(map, "matrix")?
        .as_array()
        .ok_or("`matrix` is six numbers")?
        .iter()
        .map(|v| {
            v.as_f64()
                .map(|v| v as f32)
                .ok_or("`matrix` is six numbers")
        })
        .collect::<Result<Vec<_>, _>>()?;
    values
        .try_into()
        .map_err(|_| "`matrix` is six numbers".to_string())
}

fn parse_op(op: &str, map: &Map, color_parser: ColorParser<'_>) -> Result<Op, String> {
    let Some(keys) = op_keys(op) else {
        return Err(format!("unknown op `{op}`"));
    };
    if let Some(key) = map.keys().find(|key| {
        !matches!(key.as_str(), "op" | "phase" | "when") && !keys.contains(&key.as_str())
    }) {
        return Err(format!("unknown key `{key}` for `{op}`"));
    }
    let path = || required(map, "path").and_then(|v| parse_path(v, color_parser));
    let paint = |key: &str| required(map, key).and_then(|value| parse_paint(value, color_parser));
    let number = |key: &str, default: f32| optional_number(map, key).map(|v| v.unwrap_or(default));
    let rect = || required(map, "rect").and_then(|v| lens::<4>(v, "rect"));
    Ok(match op {
        "fill" => Op::Fill {
            path: path()?,
            paint: paint("paint")?,
        },
        "stroke" => {
            let mut stroke = StrokeStyle::new(number("width", 1.0)?)
                .join(match map.get("join").and_then(Value::as_str) {
                    None | Some("miter") => LineJoin::Miter,
                    Some("round") => LineJoin::Round,
                    Some("bevel") => LineJoin::Bevel,
                    Some(other) => return Err(format!("unknown join `{other}`")),
                })
                .cap(match map.get("cap").and_then(Value::as_str) {
                    None | Some("butt") => LineCap::Butt,
                    Some("round") => LineCap::Round,
                    Some("square") => LineCap::Square,
                    Some(other) => return Err(format!("unknown cap `{other}`")),
                })
                .miter_limit(number("miterLimit", 10.0)?);
            if let Some(dash) = map.get("dash") {
                let pattern = dash
                    .as_array()
                    .ok_or("`dash` is an array of numbers")?
                    .iter()
                    .map(|v| v.as_f64().map(|v| v as f32).ok_or("`dash` holds numbers"))
                    .collect::<Result<Vec<_>, _>>()?;
                stroke = stroke.dash(pattern, number("dashOffset", 0.0)?);
            }
            Op::Stroke {
                path: path()?,
                paint: paint("paint")?,
                stroke,
            }
        }
        "shadow" => Op::Shadow {
            path: path()?,
            shadow: parse_shadow(map, color_parser)?,
        },
        "roundedRect" => Op::RoundedRect {
            rect: rect()?,
            radii: map
                .get("radii")
                .map_or(Ok([ScriptRadius::Len(px(0.0)); 4]), parse_radii)?,
            fill: map
                .get("fill")
                .map(|value| parse_paint(value, color_parser))
                .transpose()?,
            border: map
                .get("border")
                .map(|border| {
                    let pair = border.as_array().ok_or("`border` is [color, width]")?;
                    let [color, width] = pair.as_slice() else {
                        return Err("`border` is [color, width]".to_string());
                    };
                    Ok((
                        parse_color(color, color_parser)?,
                        width.as_f64().ok_or("border width is a number")? as f32,
                    ))
                })
                .transpose()?,
            shadow: map
                .get("shadow")
                .map(|shadow| {
                    let shadow = shadow.as_object().ok_or("`shadow` is an object")?;
                    only_keys(
                        shadow,
                        &["elevation", "color", "offset", "blur", "spread", "inset"],
                        "`shadow`",
                    )?;
                    parse_shadow(shadow, color_parser)
                })
                .transpose()?,
        },
        "text" => Op::Text {
            rect: rect()?,
            text: parse_text_spec(map, color_parser)?,
        },
        "image" => Op::Image {
            rect: rect()?,
            src: source(map)?,
            fit: parse_fit(map.get("fit"))?,
            radii: map
                .get("radii")
                .map_or(Ok([ScriptRadius::Len(px(0.0)); 4]), parse_radii)?,
        },
        "icon" => {
            let name = required(map, "name")?
                .as_str()
                .ok_or("`name` is a string")?;
            Op::Icon {
                rect: rect()?,
                icon: nana_ui_core::Icon::parse_name(name)
                    .ok_or_else(|| format!("unknown icon `{name}`"))?,
                paint: map.get("paint").map_or(
                    Ok(ScriptPaint::Color(PaintColor::Role(
                        SemanticColorRole::Text,
                    ))),
                    |value| parse_paint(value, color_parser),
                )?,
            }
        }
        "fillImage" => Op::FillImage {
            path: path()?,
            rect: rect()?,
            src: source(map)?,
            fit: parse_fit(map.get("fit"))?,
        },
        "save" => Op::Save,
        "restore" => Op::Restore,
        "translate" => Op::Translate(
            map.get("x").map_or(Ok(px(0.0)), parse_len)?,
            map.get("y").map_or(Ok(px(0.0)), parse_len)?,
        ),
        "scale" => {
            let x = number("x", 1.0)?;
            Op::Scale(x, number("y", x)?)
        }
        "rotate" => Op::Rotate(number("angle", 0.0)?),
        "transform" => Op::Transform(matrix(map)?),
        "setTransform" => Op::SetTransform(matrix(map)?),
        "opacity" => Op::Opacity(number("value", 1.0)?),
        "blend" => Op::Blend(parse_blend(map.get("mode"))?),
        "pushLayer" => Op::PushLayer(number("opacity", 1.0)?, parse_blend(map.get("blend"))?),
        "popLayer" => Op::PopLayer,
        "pushClip" => Op::PushClip(path()?),
        "popClip" => Op::PopClip,
        "drawDefault" => Op::DrawDefault,
        other => unreachable!("`{other}` has keys but no parser"),
    })
}

fn parse_align_pair(
    align: Option<&Value>,
) -> Result<(TextHorizontalAlignment, TextVerticalAlignment), String> {
    let Some(align) = align else {
        return Ok((
            TextHorizontalAlignment::Start,
            TextVerticalAlignment::Center,
        ));
    };
    let pair = align
        .as_array()
        .ok_or("`align` is [horizontal, vertical]")?;
    let word = |index: usize| pair.get(index).and_then(Value::as_str);
    let horizontal = match word(0) {
        None | Some("start") => TextHorizontalAlignment::Start,
        Some("center") => TextHorizontalAlignment::Center,
        Some("end") => TextHorizontalAlignment::End,
        Some(other) => return Err(format!("unknown horizontal alignment `{other}`")),
    };
    let vertical = match word(1) {
        None | Some("center") => TextVerticalAlignment::Center,
        Some("top") => TextVerticalAlignment::Top,
        Some("bottom") => TextVerticalAlignment::Bottom,
        Some(other) => return Err(format!("unknown vertical alignment `{other}`")),
    };
    Ok((horizontal, vertical))
}

fn parse_text_decoration(
    value: &Value,
    color_parser: ColorParser<'_>,
) -> Result<PaintTextDecoration, String> {
    if let Some(word) = value.as_str() {
        return match word {
            "underline" => Ok(PaintTextDecoration::underline()),
            "line-through" => Ok(PaintTextDecoration::line_through()),
            "overline" => Ok(PaintTextDecoration::overline()),
            other => Err(format!("unknown decoration `{other}`")),
        };
    }
    let map = value
        .as_object()
        .ok_or("`decoration` is a string or an object")?;
    only_keys(
        map,
        &[
            "underline",
            "lineThrough",
            "overline",
            "style",
            "color",
            "thickness",
        ],
        "`decoration`",
    )?;
    let style = match map.get("style").and_then(Value::as_str) {
        None | Some("solid") => TextDecorationStyle::Solid,
        Some("double") => TextDecorationStyle::Double,
        Some("dotted") => TextDecorationStyle::Dotted,
        Some("dashed") => TextDecorationStyle::Dashed,
        Some("wavy") => TextDecorationStyle::Wavy,
        Some(other) => return Err(format!("unknown decoration style `{other}`")),
    };
    let color = map
        .get("color")
        .map(|v| parse_color(v, color_parser))
        .transpose()?;
    Ok(PaintTextDecoration {
        underline: flag(map, "underline", false)?,
        line_through: flag(map, "lineThrough", false)?,
        overline: flag(map, "overline", false)?,
        style,
        color,
        thickness: optional_number(map, "thickness")?,
    })
}

fn parse_text_stroke(
    value: &Value,
    color_parser: ColorParser<'_>,
) -> Result<PaintTextStrokeSpec, String> {
    let map = value.as_object().ok_or("`stroke` is an object")?;
    only_keys(
        map,
        &["width", "paint", "join", "placement"],
        "`text.stroke`",
    )?;
    let width = optional_number(map, "width")?.unwrap_or(1.0);
    let paint = parse_paint(required(map, "paint")?, color_parser)?;
    let join = match map.get("join").and_then(Value::as_str) {
        None | Some("round") => TextStrokeJoin::Round,
        Some("miter") => TextStrokeJoin::Miter,
        Some("bevel") => TextStrokeJoin::Bevel,
        Some(other) => return Err(format!("unknown stroke join `{other}`")),
    };
    let placement = match map.get("placement").and_then(Value::as_str) {
        None | Some("under") => TextStrokePlacement::Under,
        Some("over") => TextStrokePlacement::Over,
        Some(other) => return Err(format!("unknown stroke placement `{other}`")),
    };
    Ok(PaintTextStrokeSpec {
        width,
        paint,
        join,
        placement,
    })
}

fn parse_text_shadows(
    value: &Value,
    color_parser: ColorParser<'_>,
) -> Result<Vec<PaintTextShadow>, String> {
    let list = value
        .as_array()
        .ok_or("`shadows` is an array of shadow objects")?;
    list.iter()
        .map(|entry| {
            let map = entry.as_object().ok_or("a text shadow is an object")?;
            only_keys(
                map,
                &["offset", "blur", "spread", "color"],
                "`text.shadows[]`",
            )?;
            let offset = match map.get("offset") {
                None => [0.0, 0.0],
                Some(value) => {
                    let pair = value
                        .as_array()
                        .filter(|l| l.len() == 2)
                        .ok_or("`offset` is [x, y]")?;
                    [
                        pair[0].as_f64().ok_or("`offset` holds numbers")? as f32,
                        pair[1].as_f64().ok_or("`offset` holds numbers")? as f32,
                    ]
                }
            };
            let color = map
                .get("color")
                .map_or(Ok(PaintColor::Rgba([0.0, 0.0, 0.0, 0.25])), |v| {
                    parse_color(v, color_parser)
                })?;
            Ok(PaintTextShadow {
                offset,
                blur: optional_number(map, "blur")?.unwrap_or(0.0),
                spread: optional_number(map, "spread")?.unwrap_or(0.0),
                color,
            })
        })
        .collect()
}

fn parse_text_spec(map: &Map, color_parser: ColorParser<'_>) -> Result<PaintTextSpec, String> {
    let (horizontal, vertical) = parse_align_pair(map.get("align"))?;
    let family = match map.get("family") {
        None | Some(Value::Null) => None,
        Some(value) => Some(Arc::from(value.as_str().ok_or("`family` is a string")?)),
    };
    Ok(PaintTextSpec {
        content: Arc::from(
            required(map, "text")?
                .as_str()
                .ok_or("`text` is a string")?,
        ),
        paint: map.get("paint").map_or(
            Ok(ScriptPaint::Color(PaintColor::Role(
                SemanticColorRole::Text,
            ))),
            |value| parse_paint(value, color_parser),
        )?,
        size: optional_number(map, "size")?,
        weight: optional_number(map, "weight")?.map(|w| w.clamp(1.0, 1000.0) as u16),
        family,
        italic: flag(map, "italic", false)?,
        line_height: optional_number(map, "lineHeight")?,
        letter_spacing: optional_number(map, "letterSpacing")?.unwrap_or(0.0),
        wrap: flag(map, "wrap", false)?,
        max_lines: optional_number(map, "maxLines")?.map(|n| n.clamp(1.0, 65535.0) as u16),
        horizontal,
        vertical,
        decoration: map
            .get("decoration")
            .map(|v| parse_text_decoration(v, color_parser))
            .transpose()?,
        stroke: map
            .get("stroke")
            .map(|v| parse_text_stroke(v, color_parser))
            .transpose()?,
        shadows: map
            .get("shadows")
            .map(|v| parse_text_shadows(v, color_parser))
            .transpose()?
            .unwrap_or_default(),
    })
}

fn required<'a>(map: &'a Map, key: &str) -> Result<&'a Value, String> {
    map.get(key).ok_or_else(|| format!("missing `{key}`"))
}

fn optional_number(map: &Map, key: &str) -> Result<Option<f32>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .map(|value| Some(value as f32))
            .ok_or_else(|| format!("`{key}` is a number")),
    }
}

fn source(map: &Map) -> Result<Arc<str>, String> {
    required(map, "src")?
        .as_str()
        .map(Arc::from)
        .ok_or_else(|| "`src` is a string".to_string())
}

fn px(value: f32) -> Len {
    Len {
        fraction: 0.0,
        px: value,
    }
}

/// A number spelled in text, if it is finite: `NaN` and `inf` would reach
/// the tessellator.
fn finite(text: &str) -> Option<f32> {
    text.parse::<f32>().ok().filter(|value| value.is_finite())
}

/// Whether every JSON number stays finite as an `f32`.
fn numbers_fit_f32(value: &Value) -> bool {
    match value {
        Value::Number(number) => number
            .as_f64()
            .is_some_and(|number| (number as f32).is_finite()),
        Value::Array(list) => list.iter().all(numbers_fit_f32),
        Value::Object(map) => map.values().all(numbers_fit_f32),
        _ => true,
    }
}

/// `12`, `"12"`, `"50%"`, `"100% - 12"`, `"50% + 4"`.
fn parse_len(value: &Value) -> Result<Len, String> {
    if let Some(number) = value.as_f64() {
        return Ok(px(number as f32));
    }
    let text = value
        .as_str()
        .ok_or("a length is a number or a string like \"50% + 4\"")?
        .replace(' ', "");
    let bad = || format!("bad length `{text}`");
    let Some(percent_at) = text.find('%') else {
        return finite(&text).map(px).ok_or_else(bad);
    };
    let fraction = finite(&text[..percent_at]).ok_or_else(bad)? / 100.0;
    let rest = &text[percent_at + 1..];
    let offset = if rest.is_empty() {
        0.0
    } else {
        finite(rest).ok_or_else(bad)?
    };
    Ok(Len {
        fraction,
        px: offset,
    })
}

fn lens<const N: usize>(value: &Value, what: &str) -> Result<[Len; N], String> {
    let list = value
        .as_array()
        .filter(|list| list.len() == N)
        .ok_or_else(|| format!("`{what}` holds {N} lengths"))?;
    let parsed = list.iter().map(parse_len).collect::<Result<Vec<_>, _>>()?;
    parsed
        .try_into()
        .map_err(|_| format!("`{what}` holds {N} lengths"))
}

fn parse_color(value: &Value, color_parser: ColorParser<'_>) -> Result<PaintColor, String> {
    if let Some(map) = value.as_object() {
        let role = |value: &Value| {
            value
                .as_str()
                .and_then(SemanticColorRole::from_css_token_name)
                .ok_or_else(|| "a colour mix names semantic roles".to_string())
        };
        let ratio = |value: &Value| {
            value
                .as_f64()
                .map(|v| v as f32)
                .ok_or_else(|| "a colour mix ratio is a number".to_string())
        };
        return match (map.get("mix"), map.get("alpha"), map.len()) {
            (Some(Value::Array(list)), None, 1) if list.len() == 3 => Ok(PaintColor::Mix(
                SemanticColorMix::new(role(&list[0])?, role(&list[1])?, ratio(&list[2])?),
            )),
            (None, Some(Value::Array(list)), 1) if list.len() == 2 => Ok(PaintColor::Mix(
                SemanticColorMix::alpha(role(&list[0])?, ratio(&list[1])?),
            )),
            _ => Err(
                "a colour object is {\"mix\": [role, role, ratio]} or {\"alpha\": [role, alpha]}"
                    .into(),
            ),
        };
    }
    if let Some(list) = value.as_array() {
        let channels = list
            .iter()
            .map(|v| {
                v.as_f64()
                    .map(|v| v as f32)
                    .ok_or("a colour array holds numbers")
            })
            .collect::<Result<Vec<_>, _>>()?;
        return match channels.as_slice() {
            [r, g, b] => Ok(PaintColor::Rgba([*r, *g, *b, 1.0])),
            [r, g, b, a] => Ok(PaintColor::Rgba([*r, *g, *b, *a])),
            _ => Err("a colour array is [r, g, b] or [r, g, b, a]".into()),
        };
    }
    let text = value.as_str().ok_or("a colour is a string or an array")?;
    if let Some(color) = color_parser.and_then(|parse| parse(text)) {
        return Ok(PaintColor::Authoring(color));
    }
    SemanticColorRole::from_css_token_name(text)
        .map(PaintColor::Role)
        .ok_or_else(|| format!("unknown colour `{text}`"))
}

fn parse_paint(value: &Value, color_parser: ColorParser<'_>) -> Result<ScriptPaint, String> {
    let Some(map) = value.as_object() else {
        return parse_color(value, color_parser).map(ScriptPaint::Color);
    };
    if map.contains_key("mix") || map.contains_key("alpha") {
        return parse_color(value, color_parser).map(ScriptPaint::Color);
    }
    only_keys(
        map,
        &["linear", "radial", "conic", "stops", "extend"],
        "a gradient",
    )?;
    let shape = if let Some(v) = map.get("linear") {
        GradientSpec::Linear(lens::<4>(v, "linear")?)
    } else if let Some(v) = map.get("radial") {
        let list = v
            .as_array()
            .filter(|l| l.len() == 3)
            .ok_or("`radial` is [cx, cy, r]")?;
        GradientSpec::Radial(
            [parse_len(&list[0])?, parse_len(&list[1])?],
            parse_len(&list[2])?,
        )
    } else if let Some(v) = map.get("conic") {
        let list = v
            .as_array()
            .filter(|l| l.len() == 3)
            .ok_or("`conic` is [cx, cy, angle]")?;
        GradientSpec::Conic(
            [parse_len(&list[0])?, parse_len(&list[1])?],
            list[2].as_f64().ok_or("the angle is a number")? as f32,
        )
    } else {
        return Err("a gradient has `linear`, `radial` or `conic`".into());
    };
    let stops = required(map, "stops")?
        .as_array()
        .ok_or("`stops` is an array")?
        .iter()
        .map(|stop| {
            let pair = stop.as_array().ok_or("a stop is [offset, colour]")?;
            let [offset, color] = pair.as_slice() else {
                return Err("a stop is [offset, colour]".to_string());
            };
            Ok((
                offset.as_f64().ok_or("a stop offset is a number")? as f32,
                parse_color(color, color_parser)?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let extend = match map.get("extend").and_then(Value::as_str) {
        None | Some("pad") => GradientExtend::Pad,
        Some("repeat") => GradientExtend::Repeat,
        Some("reflect") => GradientExtend::Reflect,
        Some(other) => return Err(format!("unknown extend `{other}`")),
    };
    Ok(ScriptPaint::Gradient {
        shape,
        stops,
        extend,
    })
}

fn parse_radius(value: &Value) -> Result<ScriptRadius, String> {
    match value.as_str() {
        Some("xs") => Ok(ScriptRadius::Tier(RadiusTier::Xs)),
        Some("sm") => Ok(ScriptRadius::Tier(RadiusTier::Sm)),
        Some("md") => Ok(ScriptRadius::Tier(RadiusTier::Md)),
        Some("lg") => Ok(ScriptRadius::Tier(RadiusTier::Lg)),
        Some("xl") => Ok(ScriptRadius::Tier(RadiusTier::Xl)),
        Some("xxl") => Ok(ScriptRadius::Tier(RadiusTier::Xxl)),
        Some("full") => Ok(ScriptRadius::Tier(RadiusTier::Full)),
        _ => parse_len(value)
            .map(ScriptRadius::Len)
            .map_err(|_| "a radius is a length or xs / sm / md / lg / xl / xxl".into()),
    }
}

fn parse_radii(value: &Value) -> Result<[ScriptRadius; 4], String> {
    match value.as_array() {
        Some(list) if list.len() == 4 => {
            let radii = list
                .iter()
                .map(parse_radius)
                .collect::<Result<Vec<_>, _>>()?;
            Ok([radii[0], radii[1], radii[2], radii[3]])
        }
        Some(_) => Err("`radii` is one radius or four".into()),
        None => parse_radius(value).map(|radius| [radius; 4]),
    }
}

fn parse_shadow(map: &Map, color_parser: ColorParser<'_>) -> Result<PaintShadow, String> {
    if let Some(role) = map.get("elevation") {
        if map.contains_key("color") || map.contains_key("blur") {
            return Err("a shadow is an `elevation` or its own `color` / `blur` …".into());
        }
        return match role.as_str() {
            Some("surface") => Ok(PaintShadow::Elevation(ElevationRole::Surface)),
            Some("overlay") => Ok(PaintShadow::Elevation(ElevationRole::Overlay)),
            _ => Err("`elevation` is surface or overlay".into()),
        };
    }
    let offset = match map.get("offset") {
        None => [0.0, 0.0],
        Some(value) => {
            let list = value
                .as_array()
                .filter(|l| l.len() == 2)
                .ok_or("`offset` is [x, y]")?;
            [
                list[0].as_f64().ok_or("`offset` holds numbers")? as f32,
                list[1].as_f64().ok_or("`offset` holds numbers")? as f32,
            ]
        }
    };
    Ok(PaintShadow::Custom {
        color: map
            .get("color")
            .map_or(Ok(PaintColor::Rgba([0.0, 0.0, 0.0, 0.25])), |value| {
                parse_color(value, color_parser)
            })?,
        offset,
        blur: optional_number(map, "blur")?.unwrap_or(0.0),
        spread: optional_number(map, "spread")?.unwrap_or(0.0),
        inset: flag(map, "inset", false)?,
    })
}

fn parse_fit(value: Option<&Value>) -> Result<ImageFit, String> {
    Ok(match value.and_then(Value::as_str) {
        None | Some("contain") => ImageFit::Contain,
        Some("fill") => ImageFit::Fill,
        Some("cover") => ImageFit::Cover,
        Some("none") => ImageFit::None,
        Some("scale-down") => ImageFit::ScaleDown,
        Some(other) => return Err(format!("unknown fit `{other}`")),
    })
}

/// A CSS `mix-blend-mode` keyword.
fn parse_blend(value: Option<&Value>) -> Result<BlendMode, String> {
    match value {
        None | Some(Value::Null) => Ok(BlendMode::Normal),
        Some(value) => {
            let name = value.as_str().ok_or("a blend mode is a string")?;
            parse_blend_mode(name).ok_or_else(|| format!("unknown blend `{name}`"))
        }
    }
}

fn parse_blend_mode(input: &str) -> Option<BlendMode> {
    Some(match input.trim().to_ascii_lowercase().as_str() {
        "normal" => BlendMode::Normal,
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color-dodge" => BlendMode::ColorDodge,
        "color-burn" => BlendMode::ColorBurn,
        "hard-light" => BlendMode::HardLight,
        "soft-light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        _ => return None,
    })
}

fn parse_fill_rule(value: Option<&Value>) -> Result<Option<FillRule>, String> {
    match value.and_then(Value::as_str) {
        None => Ok(None),
        Some("nonzero") => Ok(Some(FillRule::NonZero)),
        Some("evenodd") => Ok(Some(FillRule::EvenOdd)),
        Some(other) => Err(format!("unknown fill rule `{other}`")),
    }
}

/// A path may be an SVG string, a segment list, a `{ d, fillRule }` wrapper,
/// a boolean object `{ op, paths }`, or a `{ textPath: { ... } }` object.
fn parse_path(mut value: &Value, color_parser: ColorParser<'_>) -> Result<ScriptPath, String> {
    let mut fill_rule = None;
    let segments = loop {
        match value {
            Value::String(text) => break parse_svg_path(text)?,
            Value::Array(list) => break parse_segments(list)?,
            Value::Object(map) if map.contains_key("op") => {
                only_keys(map, &["op", "paths", "fillRule"], "a boolean path object")?;
                let op = match required(map, "op")?.as_str() {
                    Some("union") => PathBooleanOp::Union,
                    Some("intersect") => PathBooleanOp::Intersect,
                    Some("difference") => PathBooleanOp::Difference,
                    Some("xor") => PathBooleanOp::Xor,
                    Some(other) => return Err(format!("unknown path boolean op `{other}`")),
                    None => return Err("`op` is a string".into()),
                };
                let raw_paths = required(map, "paths")?
                    .as_array()
                    .ok_or("`paths` is a non-empty array of paths")?;
                if raw_paths.is_empty() {
                    return Err("`paths` must not be empty".into());
                }
                let paths = raw_paths
                    .iter()
                    .map(|p| parse_path(p, color_parser))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(rule) = parse_fill_rule(map.get("fillRule"))? {
                    fill_rule.get_or_insert(rule);
                }
                return Ok(ScriptPath::Boolean {
                    op,
                    paths: paths.into(),
                    fill_rule,
                });
            }
            Value::Object(map) if map.contains_key("textPath") => {
                only_keys(map, &["textPath", "fillRule"], "a textPath object")?;
                if let Some(rule) = parse_fill_rule(map.get("fillRule"))? {
                    fill_rule.get_or_insert(rule);
                }
                let spec = required(map, "textPath")?
                    .as_object()
                    .ok_or("`textPath` is an object")?;
                only_keys(
                    spec,
                    &[
                        "text",
                        "origin",
                        "maxWidth",
                        "size",
                        "weight",
                        "family",
                        "italic",
                        "lineHeight",
                        "letterSpacing",
                        "wrap",
                        "maxLines",
                        "align",
                    ],
                    "`textPath`",
                )?;
                let origin = match spec.get("origin") {
                    None => [px(0.0), px(0.0)],
                    Some(v) => lens::<2>(v, "origin")?,
                };
                let max_width = spec.get("maxWidth").map(parse_len).transpose()?;
                let text = parse_text_spec(spec, color_parser)?;
                return Ok(ScriptPath::TextPath {
                    text: Box::new(text),
                    origin,
                    max_width,
                    fill_rule,
                });
            }
            Value::Object(map) => {
                only_keys(map, &["d", "fillRule"], "a path object")?;
                if let Some(rule) = parse_fill_rule(map.get("fillRule"))? {
                    fill_rule.get_or_insert(rule);
                }
                value = required(map, "d")?;
            }
            _ => return Err("a path is an SVG path string or an array of segments".into()),
        }
    };
    Ok(ScriptPath::Segments {
        segments: segments.into(),
        fill_rule: fill_rule.unwrap_or(FillRule::NonZero),
    })
}

fn parse_segments(list: &[Value]) -> Result<Vec<Segment>, String> {
    list.iter()
        .map(|segment| {
            let parts = segment.as_array().ok_or("a segment is an array")?;
            let (name, args) = parts.split_first().ok_or("an empty segment")?;
            let name = name.as_str().ok_or("a segment starts with its name")?;
            let lengths = |n: usize| -> Result<Vec<Len>, String> {
                if args.len() < n {
                    return Err(format!("`{name}` needs {n} values"));
                }
                args[..n].iter().map(parse_len).collect()
            };
            let number = |i: usize| -> Result<f32, String> {
                args.get(i)
                    .and_then(Value::as_f64)
                    .map(|v| v as f32)
                    .ok_or_else(|| format!("`{name}` needs a number at {i}"))
            };
            let four = |v: Vec<Len>| [v[0], v[1], v[2], v[3]];
            Ok(match name {
                "M" => {
                    let v = lengths(2)?;
                    Segment::Move(v[0], v[1])
                }
                "L" => {
                    let v = lengths(2)?;
                    Segment::Line(v[0], v[1])
                }
                "Q" => Segment::Quad(four(lengths(4)?)),
                "C" => {
                    let v = lengths(6)?;
                    Segment::Cubic([v[0], v[1], v[2], v[3], v[4], v[5]])
                }
                "arcTo" => {
                    let v = lengths(5)?;
                    Segment::ArcTo([v[0], v[1], v[2], v[3]], v[4])
                }
                "arc" => {
                    let v = lengths(3)?;
                    Segment::Arc([v[0], v[1]], v[2], number(3)?, number(4)?)
                }
                "rect" => Segment::Rect(four(lengths(4)?)),
                "roundedRect" => {
                    let radii = match args.get(4) {
                        None => [px(0.0); 4],
                        Some(Value::Array(list)) if list.len() == 4 => {
                            let mut radii = [px(0.0); 4];
                            for (slot, value) in radii.iter_mut().zip(list) {
                                *slot = parse_len(value)?;
                            }
                            radii
                        }
                        Some(value) => [parse_len(value)?; 4],
                    };
                    Segment::RoundedRect(four(lengths(4)?), radii)
                }
                "ellipse" => Segment::Ellipse(four(lengths(4)?)),
                "Z" | "z" => Segment::Close,
                other => return Err(format!("unknown segment `{other}`")),
            })
        })
        .collect()
}

/// SVG path data, absolute and relative, with arcs converted to cubics.
fn parse_svg_path(data: &str) -> Result<Vec<Segment>, String> {
    let path = PaintPath::from_svg(data)?;
    Ok(path
        .verbs()
        .iter()
        .map(|verb| match *verb {
            crate::PathVerb::MoveTo([x, y]) => Segment::Move(px(x), px(y)),
            crate::PathVerb::LineTo([x, y]) => Segment::Line(px(x), px(y)),
            crate::PathVerb::QuadTo([cx, cy], [x, y]) => {
                Segment::Quad([px(cx), px(cy), px(x), px(y)])
            }
            crate::PathVerb::CubicTo([c1x, c1y], [c2x, c2y], [x, y]) => {
                Segment::Cubic([px(c1x), px(c1y), px(c2x), px(c2y), px(x), px(y)])
            }
            crate::PathVerb::Close => Segment::Close,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PaintOp, PathVerb, ResolvedPaint};

    fn record(script: &str, size: [f32; 2], state: PaintState) -> crate::PaintRecording {
        let script = PaintScript::from_json_str(script).expect("a valid script");
        crate::PaintRecording::record(
            &script,
            nana_ui_core::builtin_theme_arc(nana_ui_core::ThemeAppearance::Dark).as_ref(),
            size,
            state,
        )
    }

    #[test]
    fn a_script_replays_onto_the_context_at_the_node_size() {
        let recording = record(
            r##"[
                {"op": "fill", "path": [["rect", 0, 0, "100%", "50% - 2"]], "paint": "accent"},
                {"op": "stroke", "path": "M0 0 h10 v10 z", "paint": [1, 0, 0, 0.5019608], "width": 2,
                 "dash": [3, 1], "phase": "over"},
                {"op": "fill", "path": "M0 0 L1 1 Z", "paint": "surface", "when": {"hovered": true}}
            ]"##,
            [80.0, 40.0],
            PaintState::default(),
        );
        let [PaintOp::FillPath { path, paint }] = &recording.behind_children[..] else {
            panic!("{recording:?}");
        };
        assert!(path.verbs().contains(&PathVerb::LineTo([80.0, 18.0])));
        assert_eq!(
            *paint,
            ResolvedPaint::Solid(nana_ui_core::SemanticPalette::dark().accent.as_rgba_array())
        );
        let [PaintOp::StrokePath { stroke, paint, .. }] = &recording.over_children[..] else {
            panic!("{recording:?}");
        };
        assert_eq!(stroke.dash.as_ref(), &[3.0, 1.0]);
        assert_eq!(*paint, ResolvedPaint::Solid([1.0, 0.0, 0.0, 128.0 / 255.0]));
        let hovered = record(
            r##"[{"op": "fill", "path": "M0 0 L1 1 Z", "paint": "surface", "when": {"hovered": true}}]"##,
            [10.0, 10.0],
            PaintState {
                hovered: true,
                ..PaintState::default()
            },
        );
        assert_eq!(hovered.behind_children.len(), 1);
    }

    #[test]
    fn malformed_input_fails_closed_instead_of_hanging_or_panicking() {
        // A number after closepath has no command to repeat.
        assert!(PaintScript::from_json_str(r#"[{"op":"fill","path":"M0 0 Z 5 5"}]"#).is_err());
        assert!(
            PaintScript::from_json_str(r##"[{"op":"fill","path":"M0 0","paint":"#ff0000"}]"##)
                .is_err()
        );
        assert!(parse_len(&Value::from("NaN")).is_err());
        assert!(parse_len(&Value::from("inf% + 1")).is_err());
        assert!(parse_svg_path("M0 0 L1e99 0").is_err());
        assert!(PaintScript::from_json_str(r#"[{"op":"opacity","value":1e39}]"#).is_err());
    }

    /// Every theme step has a script name; the 2xl one is `"xxl"`, after the
    /// Rust step it names.
    #[test]
    fn a_rounded_rect_names_every_radius_step() {
        let recording = record(
            r##"[{"op": "roundedRect", "rect": [0, 0, 100, 100],
                  "radii": ["xs", "lg", "xl", "xxl"], "fill": "accent"}]"##,
            [100.0, 100.0],
            PaintState::default(),
        );
        let [PaintOp::RoundedRect { radii, .. }] = &recording.behind_children[..] else {
            panic!("{recording:?}");
        };
        let metrics = nana_ui_core::UI_METRICS;
        assert_eq!(
            *radii,
            [
                metrics.radius_xs,
                metrics.radius_lg,
                metrics.radius_xl,
                metrics.radius_xxl
            ]
        );
        assert!(
            PaintScript::from_json_str(r#"[{"op":"roundedRect","rect":[0,0,1,1],"radii":"xxxl"}]"#)
                .is_err()
        );
    }

    #[test]
    fn a_runaway_arc_sweep_and_a_sub_pixel_dash_stay_bounded() {
        let recording = record(
            r##"{"hit": "painted", "commands": [
                {"op": "fill", "path": [["arc", 50, 50, 10, 0, 1e30]], "paint": "accent"},
                {"op": "stroke", "path": "M0 90 H1000", "paint": "text", "width": 4,
                 "dash": [0.00001, 0.00001]}
            ]}"##,
            [1000.0, 100.0],
            PaintState::default(),
        );
        let PaintOp::FillPath { path, .. } = &recording.behind_children[0] else {
            panic!("{recording:?}");
        };
        // A sweep past a full turn draws one turn, as Canvas does.
        assert!(path.verbs().len() <= 6, "{}", path.verbs().len());
        // Too many dashes to cut: hit as the solid stroke it draws as.
        assert!(recording.contains([500.0, 90.0], [1000.0, 100.0]));
        assert!(!recording.contains([500.0, 80.0], [1000.0, 100.0]));
    }

    #[test]
    fn restore_pops_the_clips_pushed_since_save_and_default_follows_the_transform() {
        let recording = record(
            r##"{"hit": "painted", "commands": [
                {"op": "save"},
                {"op": "pushClip", "path": [["rect", 0, 0, 10, 10]]},
                {"op": "restore"},
                {"op": "translate", "x": 50},
                {"op": "drawDefault"}
            ]}"##,
            [40.0, 40.0],
            PaintState::default(),
        );
        assert!(matches!(
            recording.behind_children[..],
            [PaintOp::PushClip { .. }, PaintOp::PopClip, ..]
        ));
        assert!(recording.contains([60.0, 20.0], [40.0, 40.0]));
        assert!(!recording.contains([20.0, 20.0], [40.0, 40.0]));

        // What an invisible layer holds is not there to hit either.
        let hidden = record(
            r##"{"hit": "painted", "commands": [
                {"op": "pushLayer", "opacity": 0},
                {"op": "fill", "path": [["rect", 0, 0, 10, 10]], "paint": "text"},
                {"op": "popLayer"}
            ]}"##,
            [40.0, 40.0],
            PaintState::default(),
        );
        assert!(!hidden.contains([5.0, 5.0], [40.0, 40.0]));
    }

    #[test]
    fn every_css_blend_mode_parses_and_typos_are_reported() {
        for mode in [
            "normal",
            "multiply",
            "screen",
            "overlay",
            "darken",
            "lighten",
            "color-dodge",
            "color-burn",
            "hard-light",
            "soft-light",
            "difference",
            "exclusion",
            "hue",
            "saturation",
            "color",
            "luminosity",
        ] {
            let script = format!(r#"[{{"op": "blend", "mode": "{mode}"}}]"#);
            assert!(PaintScript::from_json_str(&script).is_ok(), "{mode}");
        }
        let err = |script: &str| PaintScript::from_json_str(script).unwrap_err();
        assert!(err(r#"[{"op": "fill", "path": "M0 0", "colour": "red"}]"#).contains("colour"));
        assert!(
            err(r#"[{"op": "text", "rect": [0,0,1,1], "text": "a", "wrap": "yes"}]"#)
                .contains("wrap")
        );
        assert!(err(r#"{"commands": [], "hitt": "painted"}"#).contains("hitt"));
        assert!(
            err(r#"[{"op": "fill", "path": {"d": "M0 0", "rule": "evenodd"}, "paint": "text"}]"#)
                .contains("rule")
        );
    }

    #[test]
    fn a_script_reaches_set_transform_colour_mixes_relative_radii_and_a_hit_path() {
        let recording = record(
            r##"[
                {"op": "setTransform", "matrix": [1, 0, 0, 1, 5, 0]},
                {"op": "fill", "path": [["roundedRect", 0, 0, "100%", "100%", "50%"]],
                 "paint": {"mix": ["accent", "surface", 0.5]}},
                {"op": "fill", "path": [["arc", "50%", "50%", "25%", 0, 6.3]],
                 "paint": {"alpha": ["text", 0.25]}}
            ]"##,
            [40.0, 20.0],
            PaintState::default(),
        );
        let [
            PaintOp::SetTransform(t),
            PaintOp::FillPath { path: pill, paint },
            PaintOp::FillPath { path: disc, .. },
        ] = &recording.behind_children[..]
        else {
            panic!("{recording:?}");
        };
        assert_eq!(*t, [1.0, 0.0, 0.0, 1.0, 5.0, 0.0]);
        let palette = nana_ui_core::SemanticPalette::dark();
        let (a, b) = (
            palette.accent.as_rgba_array(),
            palette.surface.as_rgba_array(),
        );
        let ResolvedPaint::Solid(mixed) = paint else {
            panic!("{paint:?}");
        };
        assert!((mixed[0] - (a[0] + b[0]) / 2.0).abs() < 0.01, "{mixed:?}");
        // "50%" of the 20px short side: a pill, round at both ends.
        assert!(!pill.contains([0.5, 0.5]) && pill.contains([20.0, 10.0]));
        // A 5px (25% of 20) circle about the centre.
        let bounds = disc.bounds().unwrap();
        assert!((bounds.width - 10.0).abs() < 0.5, "{bounds:?}");

        let script = PaintScript::from_json_str(
            r#"{"commands": [], "hit": {"path": [["ellipse", 0, 0, "100%", "100%"]]}}"#,
        )
        .unwrap();
        assert_eq!(script.hit_test([20.0, 10.0], [40.0, 20.0]), Some(true));
        assert_eq!(script.hit_test([1.0, 1.0], [40.0, 20.0]), Some(false));
    }

    #[test]
    fn svg_path_data_parses_relative_smooth_and_arc_commands() {
        let segments =
            parse_svg_path("m10 10 l5 0 h5 v5 s5 5 10 0 t5 5 a5 5 0 0 1 10 0 z").unwrap();
        let path = ScriptPath::Segments {
            segments: segments.into(),
            fill_rule: FillRule::NonZero,
        }
        .build(None, [0.0, 0.0]);
        let verbs = path.verbs();
        assert_eq!(verbs[0], PathVerb::MoveTo([10.0, 10.0]));
        assert_eq!(verbs[1], PathVerb::LineTo([15.0, 10.0]));
        assert_eq!(verbs[2], PathVerb::LineTo([20.0, 10.0]));
        assert_eq!(verbs[3], PathVerb::LineTo([20.0, 15.0]));
        // The arc ends where it says it does.
        let arc_end = verbs
            .iter()
            .rev()
            .find_map(|verb| match verb {
                PathVerb::CubicTo(_, _, to) => Some(*to),
                _ => None,
            })
            .unwrap();
        assert!(
            (arc_end[0] - 45.0).abs() < 1e-3 && (arc_end[1] - 20.0).abs() < 1e-3,
            "{arc_end:?}"
        );
        assert_eq!(verbs.last(), Some(&PathVerb::Close));
    }

    #[test]
    fn the_same_script_is_the_same_painter_and_bad_input_is_reported() {
        let a = crate::NodePainter::new(
            PaintScript::from_json_str(r##"[{"op": "drawDefault"}]"##).unwrap(),
        );
        let b = crate::NodePainter::new(
            PaintScript::from_json_str(r##"[{"op": "drawDefault"}]"##).unwrap(),
        );
        let c = crate::NodePainter::new(
            PaintScript::from_json_str(r##"[{"op": "popClip"}]"##).unwrap(),
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
        let error = PaintScript::from_json_str(
            r##"[{"op": "fill", "path": "M0 0", "paint": "no-such-role"}]"##,
        )
        .unwrap_err();
        assert!(
            error.contains("command 0") && error.contains("no-such-role"),
            "{error}"
        );
        assert!(
            PaintScript::from_json_str(
                r##"[{"op": "fill", "path": "M0 0", "paint": [1, 0, 0, 1]}]"##
            )
            .is_ok()
        );
    }

    #[test]
    fn an_icon_is_named_and_an_unknown_name_is_reported() {
        let recording = record(
            r##"[{"op": "icon", "rect": [0, 0, 16, 16], "name": "file", "paint": "accent"}]"##,
            [16.0, 16.0],
            PaintState::default(),
        );
        assert!(matches!(
            &recording.behind_children[..],
            [PaintOp::Icon { icon, .. }] if *icon == nana_ui_core::Icon::File
        ));
        let error = PaintScript::from_json_str(
            r##"[{"op": "icon", "rect": [0, 0, 1, 1], "name": "zzz"}]"##,
        )
        .unwrap_err();
        assert!(error.contains("zzz"), "{error}");
    }

    #[test]
    fn nested_path_objects_take_the_outermost_fill_rule_on_a_small_stack() {
        // As deep as the JSON reader admits: its own limit is 128.
        let depth = 120;
        let json = format!(
            "{}\"M0 0 L1 1 Z\"{}",
            r#"{"fillRule": "nonzero", "d": "#.repeat(depth),
            "}".repeat(depth)
        );
        let json = json.replacen("nonzero", "evenodd", 1);
        let path = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || parse_path(&serde_json::from_str(&json).unwrap(), None))
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        let ScriptPath::Segments {
            segments,
            fill_rule,
        } = path
        else {
            panic!("expected segment path");
        };
        assert_eq!(fill_rule, FillRule::EvenOdd);
        assert_eq!(segments.len(), 3);
        let plain = parse_path(&serde_json::json!({"d": "M0 0"}), None).unwrap();
        let ScriptPath::Segments { fill_rule, .. } = plain else {
            panic!("expected segment path");
        };
        assert_eq!(fill_rule, FillRule::NonZero);
    }

    #[test]
    fn script_supports_path_booleans_and_styled_text() {
        let recording = record(
            r##"[
                {
                    "op": "fill",
                    "path": {
                        "op": "difference",
                        "paths": [
                            [["rect", 0, 0, 40, 40]],
                            [["rect", 10, 10, 20, 20]]
                        ]
                    },
                    "paint": "accent"
                },
                {
                    "op": "text",
                    "rect": [0, 0, 100, 40],
                    "text": "Styled",
                    "family": "monospace",
                    "letterSpacing": 2.0,
                    "decoration": {
                        "underline": true,
                        "overline": true,
                        "style": "dashed",
                        "color": [1, 0, 0, 1],
                        "thickness": 2.0
                    },
                    "stroke": {
                        "width": 1.5,
                        "paint": [0, 0, 0, 1],
                        "join": "bevel",
                        "placement": "over"
                    },
                    "shadows": [
                        {"offset": [1, 2], "blur": 3, "color": [0, 0, 0, 0.5]}
                    ]
                }
            ]"##,
            [100.0, 40.0],
            PaintState::default(),
        );
        let PaintOp::FillPath { path, .. } = &recording.behind_children[0] else {
            panic!("expected FillPath: {recording:?}");
        };
        assert!(path.contains([5.0, 5.0]));
        assert!(!path.contains([20.0, 20.0]), "boolean hole carved");

        let PaintOp::Text {
            family,
            letter_spacing,
            effects: Some(effects),
            ..
        } = &recording.behind_children[1]
        else {
            panic!("expected styled Text: {recording:?}");
        };
        assert_eq!(family.as_deref(), Some("monospace"));
        assert_eq!(*letter_spacing, 2.0);
        let deco = effects.decoration.as_ref().expect("decoration");
        assert!(deco.line.underline && deco.line.overline);
        assert_eq!(deco.style, TextDecorationStyle::Dashed);
        assert_eq!(deco.thickness, Some(2.0));
        let stroke = effects.stroke.as_ref().expect("stroke");
        assert_eq!(stroke.width, 1.5);
        assert_eq!(stroke.join, TextStrokeJoin::Bevel);
        assert_eq!(stroke.placement, TextStrokePlacement::Over);
        assert_eq!(effects.shadows.len(), 1);

        // textPath parses cleanly and rejects unknown keys.
        assert!(
            PaintScript::from_json_str(
                r#"[{"op":"fill","path":{"textPath":{"text":"Hi","origin":[4,8],"size":20}},"paint":"accent"}]"#
            )
            .is_ok()
        );
        assert!(
            PaintScript::from_json_str(
                r#"[{"op":"fill","path":{"op":"union","paths":[]},"paint":"accent"}]"#
            )
            .is_err()
        );
        assert!(
            PaintScript::from_json_str(
                r#"[{"op":"fill","path":{"op":"bogus","paths":["M0 0"]},"paint":"accent"}]"#
            )
            .is_err()
        );
    }
}
