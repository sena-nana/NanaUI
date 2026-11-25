//! Emit Rust source expressions for compiled [`LayoutStyle`] patches.
//!
//! Used at build time (`nana-ui-view-codegen`) so compiled stylesheets
//! emit direct [`nana_ui_core::StylePatch`] closures without runtime
//! `serde_json` deserialization.

use std::fmt::Write as _;
use std::sync::Arc;

use nana_ui_core::dynamic_layout::{
    AdaptationProfile, AxisElasticity, DiscreteAdaptation, DiscreteCandidate, ElasticFloor,
    ElasticLength, KeepWith, LayoutCost,
};
use nana_ui_core::shared::Shared;
use nana_ui_core::{
    AlignSpec, AnchorSpec, BackdropFilter, BackfaceVisibility, BackgroundImage, BackgroundImageFit,
    BackgroundPosition, BackgroundRepeat, BorderImageSlice, BorderImageSpec, BorderStyle,
    BorderWidth, BoxShadowSpec, BoxSizing, CalcBinOp, CalcExpr, ClipCircle, ClipEllipse, ClipInset,
    ClipPath, ClipPoint, ClipShapeRadius, ColorFilter, ContainerType, ControlPadding, ControlSize,
    CssGradient, CursorSpec, CustomCursorSpec, CustomCursorVariant, CustomCursorVector, DirSpec,
    DisplaySpec, DominantBaselineSpec, FilterDropShadow, FlexDirection, FlexWrap,
    FontFeatureSetting, FontKerningSpec, FontVariationSetting, GradientStop, GridAutoFlow,
    GridLine, GridRepeatAuto, GridTemplateAreas, GridTrack, GridTrackListUnsupported,
    HitTestBehavior, HyphensSpec, ImageSampling, JustifySpec, LayoutStyle, LengthSpec,
    LineBreakSpec, LineHeightSpec, LineRole, LinearGradient, MaskImage, MaterialRef,
    MaterialTexture, MixBlendMode, OutlineStyle, OverflowSpec, OverflowWrapSpec, PaintColor,
    PaintMat4, PaintTransform, PointerEventsSpec, PopoverAlignment, PopoverPlacement, PositionSpec,
    RadialGradient, RadiusTier, ScrollEdgeFade, ScrollbarSkin, SemanticColorMix, SemanticColorRole,
    SpacingStep, SystemCursor, TextAlignSpec, TextAutospaceSpec, TextDecorationLine,
    TextEmphasisColor, TextEmphasisPosition, TextEmphasisShape, TextEmphasisStyle, TextJustifySpec,
    TextOrientationSpec, TextShadowSpec, TextSpacingTrimSpec, TextStrokeSpec, TextWeight,
    TextWrapStyleSpec, ThemeExpr, ThemeToken, TransformBox, TransformOrigin, TypeRole,
    UserSelectSpec, VerticalAlignSpec, ViewportAxis, VisibilitySpec, WhiteSpaceSpec,
    WindowResizeEdge, WordBreakSpec, WritingModeSpec,
};

pub(crate) trait EmitRust {
    fn emit_rust(&self, m: &str, out: &mut String);
}

impl EmitRust for bool {
    fn emit_rust(&self, _m: &str, out: &mut String) {
        out.push_str(if *self { "true" } else { "false" });
    }
}

impl EmitRust for u8 {
    fn emit_rust(&self, _m: &str, out: &mut String) {
        let _ = write!(out, "{self}_u8");
    }
}

impl EmitRust for u16 {
    fn emit_rust(&self, _m: &str, out: &mut String) {
        let _ = write!(out, "{self}_u16");
    }
}

impl EmitRust for u32 {
    fn emit_rust(&self, _m: &str, out: &mut String) {
        let _ = write!(out, "{self}_u32");
    }
}

impl EmitRust for i16 {
    fn emit_rust(&self, _m: &str, out: &mut String) {
        let _ = write!(out, "{self}_i16");
    }
}

impl EmitRust for i32 {
    fn emit_rust(&self, _m: &str, out: &mut String) {
        let _ = write!(out, "{self}_i32");
    }
}

impl EmitRust for f32 {
    fn emit_rust(&self, _m: &str, out: &mut String) {
        let v = if *self == 0.0 { 0.0 } else { *self };
        if v.is_nan() {
            out.push_str("f32::NAN");
        } else if v == f32::INFINITY {
            out.push_str("f32::INFINITY");
        } else if v == f32::NEG_INFINITY {
            out.push_str("f32::NEG_INFINITY");
        } else {
            let _ = write!(out, "{v:?}_f32");
        }
    }
}

impl EmitRust for String {
    fn emit_rust(&self, _m: &str, out: &mut String) {
        let _ = write!(out, " ::std::string::String::from({self:?})");
    }
}

impl EmitRust for Arc<str> {
    fn emit_rust(&self, _m: &str, out: &mut String) {
        let _ = write!(out, " ::std::sync::Arc::<str>::from({:?})", self.as_ref());
    }
}

impl EmitRust for Arc<[u8]> {
    fn emit_rust(&self, m: &str, out: &mut String) {
        if self.is_empty() {
            out.push_str(" ::std::sync::Arc::<[u8]>::from(::std::vec::Vec::new())");
        } else {
            out.push_str(" ::std::sync::Arc::<[u8]>::from(&[");
            for (i, item) in self.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                item.emit_rust(m, out);
            }
            out.push_str("][..])");
        }
    }
}

impl<T: EmitRust> EmitRust for Box<T> {
    fn emit_rust(&self, m: &str, out: &mut String) {
        out.push_str(" ::std::boxed::Box::new(");
        (**self).emit_rust(m, out);
        out.push(')');
    }
}

impl<T: EmitRust> EmitRust for Arc<T> {
    fn emit_rust(&self, m: &str, out: &mut String) {
        out.push_str(" ::std::sync::Arc::new(");
        (**self).emit_rust(m, out);
        out.push(')');
    }
}

impl<T: EmitRust> EmitRust for Option<T> {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            None => out.push_str("None"),
            Some(v) => {
                out.push_str("Some(");
                v.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl<T: EmitRust> EmitRust for Vec<T> {
    fn emit_rust(&self, m: &str, out: &mut String) {
        if self.is_empty() {
            out.push_str(" ::std::vec::Vec::new()");
            return;
        }
        out.push_str(" ::std::vec![");
        for (i, item) in self.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            item.emit_rust(m, out);
        }
        out.push(']');
    }
}

impl<T: EmitRust, const N: usize> EmitRust for [T; N] {
    fn emit_rust(&self, m: &str, out: &mut String) {
        out.push('[');
        for (i, item) in self.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            item.emit_rust(m, out);
        }
        out.push(']');
    }
}

impl<A: EmitRust, B: EmitRust> EmitRust for (A, B) {
    fn emit_rust(&self, m: &str, out: &mut String) {
        out.push('(');
        self.0.emit_rust(m, out);
        out.push(',');
        self.1.emit_rust(m, out);
        out.push(')');
    }
}

impl<T: EmitRust + nana_ui_core::shared::SharedDefault> EmitRust for Shared<T> {
    fn emit_rust(&self, m: &str, out: &mut String) {
        if self.is_shared_default() {
            let _ = write!(out, "{m}::Shared::empty()");
        } else {
            let _ = write!(out, "{m}::Shared::new(");
            (**self).emit_rust(m, out);
            out.push(')');
        }
    }
}

macro_rules! impl_unit_enum {
    ($ty:ident, $name:literal => { $($variant:ident),* $(,)? }) => {
        impl EmitRust for $ty {
            fn emit_rust(&self, m: &str, out: &mut String) {
                let v = match self {
                    $(Self::$variant => stringify!($variant),)*
                };
                let _ = write!(out, "{m}::{}::{v}", $name);
            }
        }
    };
}

impl_unit_enum!(FlexDirection, "FlexDirection" => { Column, Row });
impl_unit_enum!(DirSpec, "DirSpec" => { Ltr, Rtl });
impl_unit_enum!(WritingModeSpec, "WritingModeSpec" => { HorizontalTb, VerticalRl, VerticalLr });
impl_unit_enum!(TextOrientationSpec, "TextOrientationSpec" => { Mixed, Upright, Sideways });
impl_unit_enum!(FlexWrap, "FlexWrap" => { NoWrap, Wrap, WrapReverse });
impl_unit_enum!(DisplaySpec, "DisplaySpec" => {
    None, Flex, InlineFlex, Grid, InlineGrid, Inline, Layers, Free
});
impl_unit_enum!(BoxSizing, "BoxSizing" => { BorderBox, ContentBox });
impl_unit_enum!(PositionSpec, "PositionSpec" => { Static, Relative, Absolute, Fixed, Sticky });
impl_unit_enum!(PopoverPlacement, "PopoverPlacement" => { Top, Bottom, Left, Right });
impl_unit_enum!(PopoverAlignment, "PopoverAlignment" => { Start, Center, End });
impl_unit_enum!(TransformBox, "TransformBox" => { ViewBox, BorderBox, FillBox, ContentBox });
impl_unit_enum!(BackfaceVisibility, "BackfaceVisibility" => { Visible, Hidden });
impl_unit_enum!(AlignSpec, "AlignSpec" => { Start, Center, End, Stretch, Baseline });
impl_unit_enum!(JustifySpec, "JustifySpec" => {
    Start, Center, End, SpaceBetween, SpaceAround, SpaceEvenly, Stretch
});
impl_unit_enum!(OverflowSpec, "OverflowSpec" => { Visible, Hidden, Auto, Scroll });
impl_unit_enum!(PointerEventsSpec, "PointerEventsSpec" => { Auto, None });
impl_unit_enum!(SystemCursor, "SystemCursor" => {
    Default, Pointer, Text, Move, Grab, Grabbing, NotAllowed, Crosshair,
    EwResize, NsResize, Help, Wait, Progress, ZoomIn, ZoomOut, None
});
impl_unit_enum!(WindowResizeEdge, "WindowResizeEdge" => {
    North, South, East, West, NorthEast, NorthWest, SouthEast, SouthWest
});
impl EmitRust for HitTestBehavior {
    fn emit_rust(&self, m: &str, out: &mut String) {
        out.push_str(m);
        match self {
            Self::Auto => out.push_str("::HitTestBehavior::Auto"),
            Self::Client => out.push_str("::HitTestBehavior::Client"),
            Self::PassThrough => out.push_str("::HitTestBehavior::PassThrough"),
            Self::Drag => out.push_str("::HitTestBehavior::Drag"),
            Self::Resize(edge) => {
                out.push_str("::HitTestBehavior::Resize(");
                edge.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}
fn emit_rust_slice<T: EmitRust>(items: &[T], m: &str, out: &mut String) {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        item.emit_rust(m, out);
    }
    out.push(']');
}

impl EmitRust for CustomCursorVariant {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(
            out,
            "{m}::CustomCursorVariant {{ scale_milli: {}_u32, width: {}_u16, height: {}_u16, hotspot: [{}_u16, {}_u16], rgba:",
            self.scale_milli, self.width, self.height, self.hotspot[0], self.hotspot[1]
        );
        self.rgba.emit_rust(m, out);
        out.push_str(" }");
    }
}
impl EmitRust for nana_ui_core::IconPathCommand {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::MoveTo(p) => {
                out.push_str(m);
                out.push_str("::IconPathCommand::MoveTo([");
                p[0].emit_rust(m, out);
                out.push_str(", ");
                p[1].emit_rust(m, out);
                out.push_str("])");
            }
            Self::LineTo(p) => {
                out.push_str(m);
                out.push_str("::IconPathCommand::LineTo([");
                p[0].emit_rust(m, out);
                out.push_str(", ");
                p[1].emit_rust(m, out);
                out.push_str("])");
            }
            Self::CubicTo {
                control_a,
                control_b,
                to,
            } => {
                out.push_str(m);
                out.push_str("::IconPathCommand::CubicTo { control_a: [");
                control_a[0].emit_rust(m, out);
                out.push_str(", ");
                control_a[1].emit_rust(m, out);
                out.push_str("], control_b: [");
                control_b[0].emit_rust(m, out);
                out.push_str(", ");
                control_b[1].emit_rust(m, out);
                out.push_str("], to: [");
                to[0].emit_rust(m, out);
                out.push_str(", ");
                to[1].emit_rust(m, out);
                out.push_str("] }");
            }
            Self::Close => {
                out.push_str(m);
                out.push_str("::IconPathCommand::Close");
            }
        }
    }
}
impl EmitRust for nana_ui_core::IconShape {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Path(cmds) => {
                out.push_str(m);
                out.push_str("::IconShape::Path(&");
                emit_rust_slice(cmds, m, out);
                out.push(')');
            }
            Self::Fill(cmds) => {
                out.push_str(m);
                out.push_str("::IconShape::Fill(&");
                emit_rust_slice(cmds, m, out);
                out.push(')');
            }
            Self::Circle { center, radius } => {
                out.push_str(m);
                out.push_str("::IconShape::Circle { center: [");
                center[0].emit_rust(m, out);
                out.push_str(", ");
                center[1].emit_rust(m, out);
                out.push_str("], radius: ");
                radius.emit_rust(m, out);
                out.push_str(" }");
            }
            Self::Rect {
                origin,
                size,
                filled,
            } => {
                out.push_str(m);
                out.push_str("::IconShape::Rect { origin: [");
                origin[0].emit_rust(m, out);
                out.push_str(", ");
                origin[1].emit_rust(m, out);
                out.push_str("], size: [");
                size[0].emit_rust(m, out);
                out.push_str(", ");
                size[1].emit_rust(m, out);
                out.push_str("], filled: ");
                filled.emit_rust(m, out);
                out.push_str(" }");
            }
            Self::RoundedRect {
                origin,
                size,
                radius,
            } => {
                out.push_str(m);
                out.push_str("::IconShape::RoundedRect { origin: [");
                origin[0].emit_rust(m, out);
                out.push_str(", ");
                origin[1].emit_rust(m, out);
                out.push_str("], size: [");
                size[0].emit_rust(m, out);
                out.push_str(", ");
                size[1].emit_rust(m, out);
                out.push_str("], radius: ");
                radius.emit_rust(m, out);
                out.push_str(" }");
            }
        }
    }
}
impl EmitRust for CustomCursorVector {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Shapes { shapes, color } => {
                let _ = write!(
                    out,
                    "{m}::CustomCursorVector::Shapes {{ shapes: ::std::sync::Arc::from("
                );
                emit_rust_slice(shapes, m, out);
                let _ = write!(
                    out,
                    "), color: [{}_u8, {}_u8, {}_u8, {}_u8] }}",
                    color[0], color[1], color[2], color[3]
                );
            }
            Self::Svg(bytes) => {
                let _ = write!(out, "{m}::CustomCursorVector::Svg(");
                bytes.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}
impl EmitRust for CustomCursorSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Raster { variants, fallback } => {
                let _ = write!(
                    out,
                    "{m}::CustomCursorSpec::Raster {{ variants: ::std::sync::Arc::from("
                );
                emit_rust_slice(variants, m, out);
                out.push_str("), fallback: ");
                fallback.emit_rust(m, out);
                out.push_str(" }");
            }
            Self::Vector {
                source,
                logical_size,
                hotspot,
                fallback,
            } => {
                let _ = write!(out, "{m}::CustomCursorSpec::Vector {{ source: ");
                source.emit_rust(m, out);
                let _ = write!(
                    out,
                    ", logical_size: [{}_u16, {}_u16], hotspot: [{}_u16, {}_u16], fallback: ",
                    logical_size[0], logical_size[1], hotspot[0], hotspot[1]
                );
                fallback.emit_rust(m, out);
                out.push_str(" }");
            }
        }
    }
}
impl EmitRust for CursorSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Default => {
                let _ = write!(out, "{m}::CursorSpec::Default");
            }
            Self::Pointer => {
                let _ = write!(out, "{m}::CursorSpec::Pointer");
            }
            Self::Text => {
                let _ = write!(out, "{m}::CursorSpec::Text");
            }
            Self::Move => {
                let _ = write!(out, "{m}::CursorSpec::Move");
            }
            Self::Grab => {
                let _ = write!(out, "{m}::CursorSpec::Grab");
            }
            Self::Grabbing => {
                let _ = write!(out, "{m}::CursorSpec::Grabbing");
            }
            Self::NotAllowed => {
                let _ = write!(out, "{m}::CursorSpec::NotAllowed");
            }
            Self::Crosshair => {
                let _ = write!(out, "{m}::CursorSpec::Crosshair");
            }
            Self::EwResize => {
                let _ = write!(out, "{m}::CursorSpec::EwResize");
            }
            Self::NsResize => {
                let _ = write!(out, "{m}::CursorSpec::NsResize");
            }
            Self::Help => {
                let _ = write!(out, "{m}::CursorSpec::Help");
            }
            Self::Wait => {
                let _ = write!(out, "{m}::CursorSpec::Wait");
            }
            Self::Progress => {
                let _ = write!(out, "{m}::CursorSpec::Progress");
            }
            Self::ZoomIn => {
                let _ = write!(out, "{m}::CursorSpec::ZoomIn");
            }
            Self::ZoomOut => {
                let _ = write!(out, "{m}::CursorSpec::ZoomOut");
            }
            Self::None => {
                let _ = write!(out, "{m}::CursorSpec::None");
            }
            Self::Custom(custom) => {
                let _ = write!(out, "{m}::CursorSpec::Custom(");
                custom.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}
impl_unit_enum!(UserSelectSpec, "UserSelectSpec" => { Auto, Text, All, Contain, None });
impl_unit_enum!(WhiteSpaceSpec, "WhiteSpaceSpec" => { Normal, Nowrap, Pre, PreWrap, PreLine });
impl_unit_enum!(WordBreakSpec, "WordBreakSpec" => { Normal, BreakAll, BreakWord });
impl_unit_enum!(OverflowWrapSpec, "OverflowWrapSpec" => { Normal, Anywhere });
impl_unit_enum!(TextAlignSpec, "TextAlignSpec" => { Start, Center, End, Left, Right, Justify });
impl EmitRust for VerticalAlignSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Baseline => {
                let _ = write!(out, "{m}::VerticalAlignSpec::Baseline");
            }
            Self::Sub => {
                let _ = write!(out, "{m}::VerticalAlignSpec::Sub");
            }
            Self::Super => {
                let _ = write!(out, "{m}::VerticalAlignSpec::Super");
            }
            Self::TextTop => {
                let _ = write!(out, "{m}::VerticalAlignSpec::TextTop");
            }
            Self::TextBottom => {
                let _ = write!(out, "{m}::VerticalAlignSpec::TextBottom");
            }
            Self::Middle => {
                let _ = write!(out, "{m}::VerticalAlignSpec::Middle");
            }
            Self::Top => {
                let _ = write!(out, "{m}::VerticalAlignSpec::Top");
            }
            Self::Bottom => {
                let _ = write!(out, "{m}::VerticalAlignSpec::Bottom");
            }
            Self::Offset(len) => {
                let _ = write!(out, "{m}::VerticalAlignSpec::Offset(");
                len.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}
impl_unit_enum!(FontKerningSpec, "FontKerningSpec" => { Auto, Normal, None });
impl_unit_enum!(LineBreakSpec, "LineBreakSpec" => { Auto, Normal, Anywhere, Strict, Loose });
impl_unit_enum!(HyphensSpec, "HyphensSpec" => { None, Manual, Auto });
impl_unit_enum!(DominantBaselineSpec, "DominantBaselineSpec" => { Auto, Central, Alphabetic, Ideographic });
impl_unit_enum!(TextSpacingTrimSpec, "TextSpacingTrimSpec" => { SpaceAll, Normal, TrimBoth, Auto });
impl_unit_enum!(TextAutospaceSpec, "TextAutospaceSpec" => { NoAutospace, Normal });
impl_unit_enum!(TextJustifySpec, "TextJustifySpec" => { Auto, None, InterWord, InterCharacter });
impl_unit_enum!(TextWrapStyleSpec, "TextWrapStyleSpec" => { Auto, Pretty });
impl_unit_enum!(TextEmphasisShape, "TextEmphasisShape" => {
    Dot, Circle, DoubleCircle, Triangle, Sesame
});
impl_unit_enum!(GridTrackListUnsupported, "GridTrackListUnsupported" => {
    RepeatAutoFit, RepeatAutoFill, NestedRepeat, Subgrid
});
impl_unit_enum!(GridAutoFlow, "GridAutoFlow" => { Row, Column, RowDense, ColumnDense });
impl_unit_enum!(BorderStyle, "BorderStyle" => { None, Solid, Dashed, Dotted, Unsupported });
impl_unit_enum!(OutlineStyle, "OutlineStyle" => { None, Solid });
impl_unit_enum!(ContainerType, "ContainerType" => { Normal, InlineSize, Size });
impl_unit_enum!(VisibilitySpec, "VisibilitySpec" => { Visible, Hidden });
impl_unit_enum!(MixBlendMode, "MixBlendMode" => {
    Normal, Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn,
    HardLight, SoftLight, Difference, Exclusion, Hue, Saturation, Color, Luminosity,
    SourceIn, SourceOut, SourceAtop, DestinationOver, DestinationIn, DestinationOut,
    DestinationAtop, Xor, Copy, Lighter
});
impl_unit_enum!(BackgroundImageFit, "BackgroundImageFit" => {
    Cover, Contain, Stretch, Auto, Length, ScaleDown
});
impl_unit_enum!(BackgroundRepeat, "BackgroundRepeat" => {
    NoRepeat, Repeat, RepeatX, RepeatY, Round, RoundX, RoundY, Unsupported
});
impl_unit_enum!(ImageSampling, "ImageSampling" => { Resample, Mipmap, Nearest });
impl_unit_enum!(ViewportAxis, "ViewportAxis" => { Width, Height, Min, Max });
impl_unit_enum!(CalcBinOp, "CalcBinOp" => { Add, Sub, Mul, Div });
impl_unit_enum!(KeepWith, "KeepWith" => { None, Previous, Next });
impl_unit_enum!(SpacingStep, "SpacingStep" => {
    Xxs, Xs, Sm, Md, Lg, Xl, Xxl, Xxxl, PageTight, Page
});
impl_unit_enum!(TypeRole, "TypeRole" => {
    Hint, Meta, Body, Section, CardTitle, Heading, Title, Display
});
impl_unit_enum!(LineRole, "LineRole" => { Line, Tall });
impl_unit_enum!(TextWeight, "TextWeight" => { Regular, Medium, Semibold, Bold });
impl_unit_enum!(BorderWidth, "BorderWidth" => { Hairline });
impl_unit_enum!(RadiusTier, "RadiusTier" => { Xs, Sm, Md, Lg, Xl, Xxl, Full });
impl_unit_enum!(ControlSize, "ControlSize" => { Small, Medium, Large });
impl_unit_enum!(ControlPadding, "ControlPadding" => {
    Compact, Standard, Roomy, Field, ListItem
});

impl EmitRust for CalcExpr {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Number(v) => {
                let _ = write!(out, "{m}::CalcExpr::Number(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Px(v) => {
                let _ = write!(out, "{m}::CalcExpr::Px(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Percent(v) => {
                let _ = write!(out, "{m}::CalcExpr::Percent(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Em(v) => {
                let _ = write!(out, "{m}::CalcExpr::Em(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Rem(v) => {
                let _ = write!(out, "{m}::CalcExpr::Rem(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Viewport { axis, value } => {
                let _ = write!(out, "{m}::CalcExpr::Viewport{{axis:");
                axis.emit_rust(m, out);
                out.push_str(",value:");
                value.emit_rust(m, out);
                out.push('}');
            }
            Self::Binary { op, left, right } => {
                let _ = write!(out, "{m}::CalcExpr::Binary{{op:");
                op.emit_rust(m, out);
                out.push_str(",left:");
                left.emit_rust(m, out);
                out.push_str(",right:");
                right.emit_rust(m, out);
                out.push('}');
            }
            Self::Neg(inner) => {
                let _ = write!(out, "{m}::CalcExpr::Neg(");
                inner.emit_rust(m, out);
                out.push(')');
            }
            Self::Min(a, b) => {
                let _ = write!(out, "{m}::CalcExpr::Min(");
                a.emit_rust(m, out);
                out.push(',');
                b.emit_rust(m, out);
                out.push(')');
            }
            Self::Max(a, b) => {
                let _ = write!(out, "{m}::CalcExpr::Max(");
                a.emit_rust(m, out);
                out.push(',');
                b.emit_rust(m, out);
                out.push(')');
            }
            Self::Clamp { min, val, max } => {
                let _ = write!(out, "{m}::CalcExpr::Clamp{{min:");
                min.emit_rust(m, out);
                out.push_str(",val:");
                val.emit_rust(m, out);
                out.push_str(",max:");
                max.emit_rust(m, out);
                out.push('}');
            }
        }
    }
}

impl EmitRust for LengthSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::Px(v) => {
                let _ = write!(out, "{m}::LengthSpec::Px(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Percent(v) => {
                let _ = write!(out, "{m}::LengthSpec::Percent(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Em(v) => {
                let _ = write!(out, "{m}::LengthSpec::Em(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Rem(v) => {
                let _ = write!(out, "{m}::LengthSpec::Rem(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Viewport { axis, value } => {
                let _ = write!(out, "{m}::LengthSpec::Viewport{{axis:");
                axis.emit_rust(m, out);
                out.push_str(",value:");
                value.emit_rust(m, out);
                out.push('}');
            }
            Self::PercentOffset { offset_q7, percent } => {
                let _ = write!(out, "{m}::LengthSpec::PercentOffset{{offset_q7:");
                offset_q7.emit_rust(m, out);
                out.push_str(",percent:");
                percent.emit_rust(m, out);
                out.push('}');
            }
            Self::Calc(r) => {
                let _ = write!(
                    out,
                    "{m}::LengthSpec::Calc({m}::intern_calc(::std::boxed::Box::new("
                );
                r.inner().emit_rust(m, out);
                out.push_str(")))");
            }
            Self::Fill => {
                let _ = write!(out, "{m}::LengthSpec::Fill");
            }
            Self::Shrink => {
                let _ = write!(out, "{m}::LengthSpec::Shrink");
            }
            Self::Auto => {
                let _ = write!(out, "{m}::LengthSpec::Auto");
            }
            Self::MinContent => {
                let _ = write!(out, "{m}::LengthSpec::MinContent");
            }
            Self::MaxContent => {
                let _ = write!(out, "{m}::LengthSpec::MaxContent");
            }
            Self::FitContent => {
                let _ = write!(out, "{m}::LengthSpec::FitContent");
            }
        }
    }
}

impl EmitRust for LineHeightSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::Relative(v) => {
                let _ = write!(out, "{m}::LineHeightSpec::Relative(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Absolute(v) => {
                let _ = write!(out, "{m}::LineHeightSpec::Absolute(");
                v.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for AnchorSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::AnchorSpec{{rect:");
        self.rect.emit_rust(m, out);
        out.push_str(",placement:");
        self.placement.emit_rust(m, out);
        out.push_str(",alignment:");
        self.alignment.emit_rust(m, out);
        out.push_str(",gap:");
        self.gap.emit_rust(m, out);
        out.push_str(",room:");
        self.room.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for PaintTransform {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::PaintTransform{{a:");
        self.a.emit_rust(m, out);
        out.push_str(",b:");
        self.b.emit_rust(m, out);
        out.push_str(",c:");
        self.c.emit_rust(m, out);
        out.push_str(",d:");
        self.d.emit_rust(m, out);
        out.push_str(",e:");
        self.e.emit_rust(m, out);
        out.push_str(",f:");
        self.f.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for PaintMat4 {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::PaintMat4{{m:");
        self.m.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for TransformOrigin {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::TransformOrigin{{x:");
        self.x.emit_rust(m, out);
        out.push_str(",y:");
        self.y.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for LayoutCost {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::Finite(v) => {
                let _ = write!(out, "{m}::LayoutCost::Finite(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Forbidden => {
                let _ = write!(out, "{m}::LayoutCost::Forbidden");
            }
        }
    }
}

impl EmitRust for ElasticFloor {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::Px(v) => {
                let _ = write!(out, "{m}::ElasticFloor::Px(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Fraction(p) => {
                let _ = write!(out, "{m}::ElasticFloor::Fraction(");
                p.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for ElasticLength {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::ElasticLength{{floor:");
        self.floor.emit_rust(m, out);
        out.push_str(",cost:");
        self.cost.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for AxisElasticity {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::AxisElasticity{{padding:");
        self.padding.emit_rust(m, out);
        out.push_str(",gap:");
        self.gap.emit_rust(m, out);
        out.push_str(",content_gap:");
        self.content_gap.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for DiscreteCandidate {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::DiscreteCandidate{{saves_px:");
        self.saves_px.emit_rust(m, out);
        out.push_str(",cost:");
        self.cost.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for DiscreteAdaptation {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::DiscreteAdaptation{{candidates:");
        self.candidates.emit_rust(m, out);
        out.push_str(",hysteresis_px:");
        self.hysteresis_px.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for AdaptationProfile {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::AdaptationProfile{{inline:");
        self.inline.emit_rust(m, out);
        out.push_str(",block:");
        self.block.emit_rust(m, out);
        out.push_str(",aggregate_children:");
        self.aggregate_children.emit_rust(m, out);
        out.push_str(",solve_overflow:");
        self.solve_overflow.emit_rust(m, out);
        out.push_str(",break_before:");
        self.break_before.emit_rust(m, out);
        out.push_str(",break_after:");
        self.break_after.emit_rust(m, out);
        out.push_str(",keep_with:");
        self.keep_with.emit_rust(m, out);
        out.push_str(",discrete:");
        self.discrete.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for SemanticColorRole {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let variant = match self {
            Self::Background => "Background",
            Self::Surface => "Surface",
            Self::Subtle => "Subtle",
            Self::Raised => "Raised",
            Self::Header => "Header",
            Self::Hover => "Hover",
            Self::Active => "Active",
            Self::Selected => "Selected",
            Self::SelectedHover => "SelectedHover",
            Self::SelectedPressed => "SelectedPressed",
            Self::Border => "Border",
            Self::BorderSoft => "BorderSoft",
            Self::BorderStrong => "BorderStrong",
            Self::Text => "Text",
            Self::Muted => "Muted",
            Self::Faint => "Faint",
            Self::Accent => "Accent",
            Self::AccentStrong => "AccentStrong",
            Self::AccentSoft => "AccentSoft",
            Self::AccentSoftHover => "AccentSoftHover",
            Self::AccentSoftPressed => "AccentSoftPressed",
            Self::AccentOnSoft => "AccentOnSoft",
            Self::AccentText => "AccentText",
            Self::FocusSurface => "FocusSurface",
            Self::FocusBorder => "FocusBorder",
            Self::FocusText => "FocusText",
            Self::Highlight => "Highlight",
            Self::HighlightHover => "HighlightHover",
            Self::HighlightPressed => "HighlightPressed",
            Self::HighlightText => "HighlightText",
            Self::Success => "Success",
            Self::Warning => "Warning",
            Self::WarningSoft => "WarningSoft",
            Self::WarningSoftHover => "WarningSoftHover",
            Self::WarningSoftPressed => "WarningSoftPressed",
            Self::Danger => "Danger",
            Self::DangerSoftHover => "DangerSoftHover",
            Self::DangerSoftPressed => "DangerSoftPressed",
            Self::Titlebar => "Titlebar",
            Self::OnMedia => "OnMedia",
            Self::Function => "Function",
            Self::Builtin => "Builtin",
            Self::Type => "Type",
            Self::Variable => "Variable",
            Self::Parameter => "Parameter",
            Self::Const => "Const",
            Self::Texture => "Texture",
            Self::Property => "Property",
            Self::Keyword => "Keyword",
        };
        let _ = write!(out, "{m}::SemanticColorRole::{variant}");
    }
}

impl EmitRust for SemanticColorMix {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let (first, second, weight) = self.parts();
        let _ = write!(out, "{m}::SemanticColorMix::from_parts(");
        first.emit_rust(m, out);
        out.push(',');
        second.emit_rust(m, out);
        out.push(',');
        let _ = write!(out, "{weight}u16)");
    }
}

impl EmitRust for PaintColor {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::Srgb { rgba } => {
                let _ = write!(out, "{m}::PaintColor::Srgb{{rgba:");
                rgba.emit_rust(m, out);
                out.push('}');
            }
            Self::Oklch { l, c, h, alpha } => {
                let _ = write!(out, "{m}::PaintColor::Oklch{{l:");
                l.emit_rust(m, out);
                out.push_str(",c:");
                c.emit_rust(m, out);
                out.push_str(",h:");
                h.emit_rust(m, out);
                out.push_str(",alpha:");
                alpha.emit_rust(m, out);
                out.push('}');
            }
            Self::Hsv { h, s, v, alpha } => {
                let _ = write!(out, "{m}::PaintColor::Hsv{{h:");
                h.emit_rust(m, out);
                out.push_str(",s:");
                s.emit_rust(m, out);
                out.push_str(",v:");
                v.emit_rust(m, out);
                out.push_str(",alpha:");
                alpha.emit_rust(m, out);
                out.push('}');
            }
            Self::LinearScRgb { channels, alpha } => {
                let _ = write!(out, "{m}::PaintColor::LinearScRgb{{channels:");
                channels.emit_rust(m, out);
                out.push_str(",alpha:");
                alpha.emit_rust(m, out);
                out.push('}');
            }
            Self::ThemeRole { role } => {
                let _ = write!(out, "{m}::PaintColor::ThemeRole{{role:");
                role.emit_rust(m, out);
                out.push('}');
            }
            Self::ThemeMix(mix) => {
                let _ = write!(out, "{m}::PaintColor::ThemeMix(");
                mix.emit_rust(m, out);
                out.push(')');
            }
            Self::ThemePair(pair) => {
                let (light, dark) = pair.pair();
                let _ = write!(out, "{m}::PaintColor::light_dark(");
                light.emit_rust(m, out);
                out.push(',');
                dark.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for TextDecorationLine {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::TextDecorationLine{{underline:");
        self.underline.emit_rust(m, out);
        out.push_str(",line_through:");
        self.line_through.emit_rust(m, out);
        out.push_str(",overline:");
        self.overline.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for TextEmphasisStyle {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::None => {
                let _ = write!(out, "{m}::TextEmphasisStyle::None");
            }
            Self::Mark { shape, open } => {
                let _ = write!(out, "{m}::TextEmphasisStyle::Mark{{shape:");
                shape.emit_rust(m, out);
                out.push_str(",open:");
                open.emit_rust(m, out);
                out.push('}');
            }
            Self::String(s) => {
                let _ = write!(out, "{m}::TextEmphasisStyle::String(");
                s.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for TextEmphasisPosition {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::TextEmphasisPosition{{under:");
        self.under.emit_rust(m, out);
        out.push_str(",left:");
        self.left.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for TextEmphasisColor {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::CurrentColor => {
                let _ = write!(out, "{m}::TextEmphasisColor::CurrentColor");
            }
            Self::Color(c) => {
                let _ = write!(out, "{m}::TextEmphasisColor::Color(");
                c.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for FontFeatureSetting {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::FontFeatureSetting{{tag:");
        self.tag.emit_rust(m, out);
        out.push_str(",value:");
        self.value.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for FontVariationSetting {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::FontVariationSetting{{tag:");
        self.tag.emit_rust(m, out);
        out.push_str(",value:");
        self.value.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for GridTrack {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::Px(v) => {
                let _ = write!(out, "{m}::GridTrack::Px(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Percent(v) => {
                let _ = write!(out, "{m}::GridTrack::Percent(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Fr(v) => {
                let _ = write!(out, "{m}::GridTrack::Fr(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::MinMax { min_px, fr, max_px } => {
                let _ = write!(out, "{m}::GridTrack::MinMax{{min_px:");
                min_px.emit_rust(m, out);
                out.push_str(",fr:");
                fr.emit_rust(m, out);
                out.push_str(",max_px:");
                max_px.emit_rust(m, out);
                out.push('}');
            }
            Self::Auto => {
                let _ = write!(out, "{m}::GridTrack::Auto");
            }
        }
    }
}

impl EmitRust for GridRepeatAuto {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::GridRepeatAuto{{kind:");
        self.kind.emit_rust(m, out);
        out.push_str(",tracks:");
        self.tracks.emit_rust(m, out);
        out.push_str(",prefix:");
        self.prefix.emit_rust(m, out);
        out.push_str(",suffix:");
        self.suffix.emit_rust(m, out);
        out.push_str(",prefix_line_names:");
        self.prefix_line_names.emit_rust(m, out);
        out.push_str(",pattern_line_names:");
        self.pattern_line_names.emit_rust(m, out);
        out.push_str(",suffix_line_names:");
        self.suffix_line_names.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for GridTemplateAreas {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::GridTemplateAreas{{cells:");
        self.cells.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for GridLine {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Auto => {
                let _ = write!(out, "{m}::GridLine::Auto");
            }
            Self::Index(i) => {
                let _ = write!(out, "{m}::GridLine::Index(");
                i.emit_rust(m, out);
                out.push(')');
            }
            Self::Span(s) => {
                let _ = write!(out, "{m}::GridLine::Span(");
                s.emit_rust(m, out);
                out.push(')');
            }
            Self::Name(n) => {
                let _ = write!(out, "{m}::GridLine::Name(");
                n.emit_rust(m, out);
                out.push(')');
            }
            Self::NthName(n, k) => {
                let _ = write!(out, "{m}::GridLine::NthName(");
                n.emit_rust(m, out);
                out.push(',');
                k.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for ThemeToken {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::Space(v) => {
                let _ = write!(out, "{m}::ThemeToken::Space(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Type(v) => {
                let _ = write!(out, "{m}::ThemeToken::Type(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Weight(v) => {
                let _ = write!(out, "{m}::ThemeToken::Weight(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Line(v) => {
                let _ = write!(out, "{m}::ThemeToken::Line(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Border(v) => {
                let _ = write!(out, "{m}::ThemeToken::Border(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Radius(v) => {
                let _ = write!(out, "{m}::ThemeToken::Radius(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Control(v) => {
                let _ = write!(out, "{m}::ThemeToken::Control(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::ControlPadding(v) => {
                let _ = write!(out, "{m}::ThemeToken::ControlPadding(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::PanelPaddingX => {
                let _ = write!(out, "{m}::ThemeToken::PanelPaddingX");
            }
            _ => unreachable!("unhandled ThemeToken variant in rust_expr"),
        }
    }
}

impl EmitRust for ThemeExpr {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::ThemeExpr{{constant:");
        self.constant.emit_rust(m, out);
        out.push_str(",terms:");
        self.terms.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for MaterialTexture {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::MaterialTexture{{size:");
        self.size.emit_rust(m, out);
        out.push_str(",rgba:");
        self.rgba.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for MaterialRef {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(
            out,
            "{m}::MaterialRef::new({m}::MaterialProgram::new({:?}))",
            self.program().source.as_ref()
        );
        let params = self.parameters();
        if params != [[0.0_f32; 4]] {
            out.push_str(".with_parameters(::std::sync::Arc::<[[f32;4]]>::from(&[");
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                p.emit_rust(m, out);
            }
            out.push_str("][..]))");
        }
        let textures = self.textures();
        if !textures.is_empty() {
            out.push_str(".with_textures(::std::sync::Arc::from(&[");
            for (i, t) in textures.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                t.emit_rust(m, out);
            }
            out.push_str("][..]))");
        }
    }
}

impl EmitRust for BoxShadowSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::BoxShadowSpec{{offset_x:");
        self.offset_x.emit_rust(m, out);
        out.push_str(",offset_y:");
        self.offset_y.emit_rust(m, out);
        out.push_str(",blur_radius:");
        self.blur_radius.emit_rust(m, out);
        out.push_str(",spread_radius:");
        self.spread_radius.emit_rust(m, out);
        out.push_str(",color:");
        self.color.emit_rust(m, out);
        out.push_str(",inset:");
        self.inset.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for ScrollbarSkin {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::ScrollbarSkin{{thickness:");
        self.thickness.emit_rust(m, out);
        out.push_str(",thumb_thickness:");
        self.thumb_thickness.emit_rust(m, out);
        out.push_str(",thumb_active_thickness:");
        self.thumb_active_thickness.emit_rust(m, out);
        out.push_str(",track_color:");
        self.track_color.emit_rust(m, out);
        out.push_str(",track_hot_color:");
        self.track_hot_color.emit_rust(m, out);
        out.push_str(",thumb_color:");
        self.thumb_color.emit_rust(m, out);
        out.push_str(",thumb_hot_color:");
        self.thumb_hot_color.emit_rust(m, out);
        out.push_str(",track_border_color:");
        self.track_border_color.emit_rust(m, out);
        out.push_str(",thumb_border_color:");
        self.thumb_border_color.emit_rust(m, out);
        out.push_str(",over_media:");
        self.over_media.emit_rust(m, out);
        out.push_str(",room:");
        self.room.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for ScrollEdgeFade {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::ScrollEdgeFade{{top:");
        self.top.emit_rust(m, out);
        out.push_str(",bottom:");
        self.bottom.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for TextShadowSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::TextShadowSpec{{offset_x:");
        self.offset_x.emit_rust(m, out);
        out.push_str(",offset_y:");
        self.offset_y.emit_rust(m, out);
        out.push_str(",blur_radius:");
        self.blur_radius.emit_rust(m, out);
        out.push_str(",color:");
        self.color.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for TextStrokeSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::TextStrokeSpec{{width:");
        self.width.emit_rust(m, out);
        out.push_str(",color:");
        self.color.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for GradientStop {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::GradientStop{{position:");
        self.position.emit_rust(m, out);
        out.push_str(",color:");
        self.color.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for LinearGradient {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::LinearGradient{{angle_deg:");
        self.angle_deg.emit_rust(m, out);
        out.push_str(",stops:");
        self.stops.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for RadialGradient {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::RadialGradient{{circle:");
        self.circle.emit_rust(m, out);
        out.push_str(",center:");
        self.center.emit_rust(m, out);
        out.push_str(",stops:");
        self.stops.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for CssGradient {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Linear(g) => {
                let _ = write!(out, "{m}::CssGradient::Linear(");
                g.emit_rust(m, out);
                out.push(')');
            }
            Self::Radial(g) => {
                let _ = write!(out, "{m}::CssGradient::Radial(");
                g.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for BackgroundPosition {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::BackgroundPosition{{x:");
        self.x.emit_rust(m, out);
        out.push_str(",y:");
        self.y.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for BackgroundImage {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Gradient(g) => {
                let _ = write!(out, "{m}::BackgroundImage::Gradient(");
                g.emit_rust(m, out);
                out.push(')');
            }
            Self::Url {
                url,
                fit,
                size_width,
                size_height,
                position,
                repeat,
                sampling,
            } => {
                let _ = write!(out, "{m}::BackgroundImage::Url{{url:");
                url.emit_rust(m, out);
                out.push_str(",fit:");
                fit.emit_rust(m, out);
                out.push_str(",size_width:");
                size_width.emit_rust(m, out);
                out.push_str(",size_height:");
                size_height.emit_rust(m, out);
                out.push_str(",position:");
                position.emit_rust(m, out);
                out.push_str(",repeat:");
                repeat.emit_rust(m, out);
                out.push_str(",sampling:");
                sampling.emit_rust(m, out);
                out.push('}');
            }
        }
    }
}

impl EmitRust for BorderImageSlice {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::Number(v) => {
                let _ = write!(out, "{m}::BorderImageSlice::Number(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Percent(v) => {
                let _ = write!(out, "{m}::BorderImageSlice::Percent(");
                v.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for BorderImageSpec {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::BorderImageSpec{{source:");
        self.source.emit_rust(m, out);
        out.push_str(",slice:");
        self.slice.emit_rust(m, out);
        out.push_str(",fill:");
        self.fill.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for MaskImage {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Gradient(g) => {
                let _ = write!(out, "{m}::MaskImage::Gradient(");
                g.emit_rust(m, out);
                out.push(')');
            }
            Self::Url(u) => {
                let _ = write!(out, "{m}::MaskImage::Url(");
                u.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for ClipInset {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::ClipInset{{top:");
        self.top.emit_rust(m, out);
        out.push_str(",right:");
        self.right.emit_rust(m, out);
        out.push_str(",bottom:");
        self.bottom.emit_rust(m, out);
        out.push_str(",left:");
        self.left.emit_rust(m, out);
        out.push_str(",round:");
        self.round.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for ClipPoint {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::ClipPoint{{x:");
        self.x.emit_rust(m, out);
        out.push_str(",y:");
        self.y.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for ClipShapeRadius {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match *self {
            Self::ClosestSide => {
                let _ = write!(out, "{m}::ClipShapeRadius::ClosestSide");
            }
            Self::FarthestSide => {
                let _ = write!(out, "{m}::ClipShapeRadius::FarthestSide");
            }
            Self::Length(len) => {
                let _ = write!(out, "{m}::ClipShapeRadius::Length(");
                len.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for ClipCircle {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::ClipCircle{{radius:");
        self.radius.emit_rust(m, out);
        out.push_str(",cx:");
        self.cx.emit_rust(m, out);
        out.push_str(",cy:");
        self.cy.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for ClipEllipse {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::ClipEllipse{{rx:");
        self.rx.emit_rust(m, out);
        out.push_str(",ry:");
        self.ry.emit_rust(m, out);
        out.push_str(",cx:");
        self.cx.emit_rust(m, out);
        out.push_str(",cy:");
        self.cy.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for ClipPath {
    fn emit_rust(&self, m: &str, out: &mut String) {
        match self {
            Self::Inset(v) => {
                let _ = write!(out, "{m}::ClipPath::Inset(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Polygon(pts) => {
                let _ = write!(out, "{m}::ClipPath::Polygon(");
                pts.emit_rust(m, out);
                out.push(')');
            }
            Self::Circle(v) => {
                let _ = write!(out, "{m}::ClipPath::Circle(");
                v.emit_rust(m, out);
                out.push(')');
            }
            Self::Ellipse(v) => {
                let _ = write!(out, "{m}::ClipPath::Ellipse(");
                v.emit_rust(m, out);
                out.push(')');
            }
        }
    }
}

impl EmitRust for FilterDropShadow {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::FilterDropShadow{{offset_x:");
        self.offset_x.emit_rust(m, out);
        out.push_str(",offset_y:");
        self.offset_y.emit_rust(m, out);
        out.push_str(",blur_radius:");
        self.blur_radius.emit_rust(m, out);
        out.push_str(",color:");
        self.color.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for ColorFilter {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::ColorFilter{{brightness:");
        self.brightness.emit_rust(m, out);
        out.push_str(",saturate:");
        self.saturate.emit_rust(m, out);
        out.push_str(",contrast:");
        self.contrast.emit_rust(m, out);
        out.push_str(",hue_rotate_deg:");
        self.hue_rotate_deg.emit_rust(m, out);
        out.push_str(",invert:");
        self.invert.emit_rust(m, out);
        out.push_str(",opacity:");
        self.opacity.emit_rust(m, out);
        out.push_str(",blur_radius:");
        self.blur_radius.emit_rust(m, out);
        out.push_str(",drop_shadow:");
        self.drop_shadow.emit_rust(m, out);
        out.push('}');
    }
}

impl EmitRust for BackdropFilter {
    fn emit_rust(&self, m: &str, out: &mut String) {
        let _ = write!(out, "{m}::BackdropFilter{{blur_radius:");
        self.blur_radius.emit_rust(m, out);
        out.push_str(",saturate:");
        self.saturate.emit_rust(m, out);
        out.push('}');
    }
}

fn emit_set(setter: &str, value: &impl EmitRust, m: &str, out: &mut String) {
    let _ = write!(out, "{m}::set::{setter}(__nana_layout,");
    value.emit_rust(m, out);
    out.push_str(");");
}

/// Emit `{m}::set::<path>(__nana_layout, <expr>);` for any leaf in [`LayoutStyle`].
pub(crate) fn emit_patch_stmt(layout: &LayoutStyle, path: &str, m: &str, out: &mut String) {
    if let Some(rest) = path.strip_prefix("theme.") {
        let val = match rest {
            "gap" => &layout.theme.gap,
            "row_gap" => &layout.theme.row_gap,
            "column_gap" => &layout.theme.column_gap,
            "padding" => &layout.theme.padding,
            "padding_top" => &layout.theme.padding_top,
            "padding_right" => &layout.theme.padding_right,
            "padding_bottom" => &layout.theme.padding_bottom,
            "padding_left" => &layout.theme.padding_left,
            "margin" => &layout.theme.margin,
            "margin_top" => &layout.theme.margin_top,
            "margin_right" => &layout.theme.margin_right,
            "margin_bottom" => &layout.theme.margin_bottom,
            "margin_left" => &layout.theme.margin_left,
            "offset_top" => &layout.theme.offset_top,
            "offset_right" => &layout.theme.offset_right,
            "offset_bottom" => &layout.theme.offset_bottom,
            "offset_left" => &layout.theme.offset_left,
            "width" => &layout.theme.width,
            "height" => &layout.theme.height,
            "min_width" => &layout.theme.min_width,
            "max_width" => &layout.theme.max_width,
            "min_height" => &layout.theme.min_height,
            "max_height" => &layout.theme.max_height,
            "flex_basis" => &layout.theme.flex_basis,
            "font_size" => &layout.theme.font_size,
            "font_weight" => &layout.theme.font_weight,
            "line_height" => &layout.theme.line_height,
            "letter_spacing" => &layout.theme.letter_spacing,
            "border_width" => &layout.theme.border_width,
            "border_top_width" => &layout.theme.border_top_width,
            "border_right_width" => &layout.theme.border_right_width,
            "border_bottom_width" => &layout.theme.border_bottom_width,
            "border_left_width" => &layout.theme.border_left_width,
            "border_radius" => &layout.theme.border_radius,
            "scrollbar_room" => &layout.theme.scrollbar_room,
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        };
        let setter = format!("theme::{rest}");
        emit_set(&setter, val, m, out);
        return;
    }

    if let Some(rest) = path.strip_prefix("paint_colors.") {
        let val = match rest {
            "color" => &layout.paint_colors.color,
            "background" => &layout.paint_colors.background,
            "border" => &layout.paint_colors.border,
            "border_top" => &layout.paint_colors.border_top,
            "border_right" => &layout.paint_colors.border_right,
            "border_bottom" => &layout.paint_colors.border_bottom,
            "border_left" => &layout.paint_colors.border_left,
            "outline" => &layout.paint_colors.outline,
            "selection_background" => &layout.paint_colors.selection_background,
            "selection_color" => &layout.paint_colors.selection_color,
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        };
        let setter = format!("paint_colors::{rest}");
        emit_set(&setter, val, m, out);
        return;
    }

    if let Some(rest) = path.strip_prefix("transforms.") {
        let setter = format!("transforms::{rest}");
        let t = &layout.transforms;
        match rest {
            "transform" => emit_set(&setter, &t.transform, m, out),
            "transform_3d" => emit_set(&setter, &t.transform_3d, m, out),
            "unsupported_transform" => emit_set(&setter, &t.unsupported_transform, m, out),
            "origin" => emit_set(&setter, &t.origin, m, out),
            "css_perspective" => emit_set(&setter, &t.css_perspective, m, out),
            "perspective_origin" => emit_set(&setter, &t.perspective_origin, m, out),
            "preserve_3d" => emit_set(&setter, &t.preserve_3d, m, out),
            "backface_visibility" => emit_set(&setter, &t.backface_visibility, m, out),
            "transform_box" => emit_set(&setter, &t.transform_box, m, out),
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        }
        return;
    }

    if let Some(subpath) = path.strip_prefix("logical_edges.") {
        for (group_name, edges) in [
            ("padding", &layout.logical_edges.padding),
            ("margin", &layout.logical_edges.margin),
            ("inset", &layout.logical_edges.inset),
        ] {
            if let Some(rest) = subpath
                .strip_prefix(group_name)
                .and_then(|s| s.strip_prefix('.'))
            {
                let setter = format!("logical_edges::{group_name}::{rest}");
                match rest {
                    "inline_start" => emit_set(&setter, &edges.inline_start, m, out),
                    "inline_end" => emit_set(&setter, &edges.inline_end, m, out),
                    "block_start" => emit_set(&setter, &edges.block_start, m, out),
                    "block_end" => emit_set(&setter, &edges.block_end, m, out),
                    "phys_top" => emit_set(&setter, &edges.phys_top, m, out),
                    "phys_right" => emit_set(&setter, &edges.phys_right, m, out),
                    "phys_bottom" => emit_set(&setter, &edges.phys_bottom, m, out),
                    "phys_left" => emit_set(&setter, &edges.phys_left, m, out),
                    "generations" => emit_set(&setter, &edges.generations, m, out),
                    "next_gen" => emit_set(&setter, &edges.next_gen, m, out),
                    "derived" => emit_set(&setter, &edges.derived, m, out),
                    _ => panic!("unknown LayoutStyle leaf path: {path}"),
                }
                return;
            }
        }
        panic!("unknown LayoutStyle leaf path: {path}");
    }

    if let Some(rest) = path.strip_prefix("text_ext.text_typography.") {
        let setter = format!("text_ext::text_typography::{rest}");
        let tt = &layout.text_ext.text_typography;
        match rest {
            "spacing_trim" => emit_set(&setter, &tt.spacing_trim, m, out),
            "autospace" => emit_set(&setter, &tt.autospace, m, out),
            "justify" => emit_set(&setter, &tt.justify, m, out),
            "wrap_style" => emit_set(&setter, &tt.wrap_style, m, out),
            "justify_lines" => emit_set(&setter, &tt.justify_lines, m, out),
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        }
        return;
    }

    if let Some(rest) = path.strip_prefix("text_ext.") {
        let setter = format!("text_ext::{rest}");
        let te = &layout.text_ext;
        match rest {
            "font_family" => emit_set(&setter, &te.font_family, m, out),
            "letter_spacing" => emit_set(&setter, &te.letter_spacing, m, out),
            "line_clamp" => emit_set(&setter, &te.line_clamp, m, out),
            "word_break" => emit_set(&setter, &te.word_break, m, out),
            "overflow_wrap" => emit_set(&setter, &te.overflow_wrap, m, out),
            "text_decoration" => emit_set(&setter, &te.text_decoration, m, out),
            "text_emphasis_style" => emit_set(&setter, &te.text_emphasis_style, m, out),
            "text_emphasis_position" => emit_set(&setter, &te.text_emphasis_position, m, out),
            "text_emphasis_color" => emit_set(&setter, &te.text_emphasis_color, m, out),
            "font_features" => emit_set(&setter, &te.font_features, m, out),
            "font_variation_settings" => emit_set(&setter, &te.font_variation_settings, m, out),
            "font_kerning" => emit_set(&setter, &te.font_kerning, m, out),
            "line_break" => emit_set(&setter, &te.line_break, m, out),
            "hyphens" => emit_set(&setter, &te.hyphens, m, out),
            "unsupported_font_variation" => {
                emit_set(&setter, &te.unsupported_font_variation, m, out)
            }
            "unsupported_writing_mode" => emit_set(&setter, &te.unsupported_writing_mode, m, out),
            "text_orientation" => emit_set(&setter, &te.text_orientation, m, out),
            "dominant_baseline" => emit_set(&setter, &te.dominant_baseline, m, out),
            "vertical_align" => emit_set(&setter, &te.vertical_align, m, out),
            "placeholder_color" => emit_set(&setter, &te.placeholder_color, m, out),
            "placeholder_opacity" => emit_set(&setter, &te.placeholder_opacity, m, out),
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        }
        return;
    }

    if let Some(rest) = path.strip_prefix("grid.placement.") {
        let setter = format!("grid::placement::{rest}");
        let gp = &layout.grid.placement;
        match rest {
            "column_start" => emit_set(&setter, &gp.column_start, m, out),
            "column_end" => emit_set(&setter, &gp.column_end, m, out),
            "row_start" => emit_set(&setter, &gp.row_start, m, out),
            "row_end" => emit_set(&setter, &gp.row_end, m, out),
            "area" => emit_set(&setter, &gp.area, m, out),
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        }
        return;
    }

    if let Some(rest) = path.strip_prefix("grid.") {
        let setter = format!("grid::{rest}");
        let g = &layout.grid;
        match rest {
            "columns" => emit_set(&setter, &g.columns, m, out),
            "rows" => emit_set(&setter, &g.rows, m, out),
            "columns_unsupported" => emit_set(&setter, &g.columns_unsupported, m, out),
            "rows_unsupported" => emit_set(&setter, &g.rows_unsupported, m, out),
            "auto_columns" => emit_set(&setter, &g.auto_columns, m, out),
            "auto_rows" => emit_set(&setter, &g.auto_rows, m, out),
            "auto_flow" => emit_set(&setter, &g.auto_flow, m, out),
            "columns_repeat" => emit_set(&setter, &g.columns_repeat, m, out),
            "rows_repeat" => emit_set(&setter, &g.rows_repeat, m, out),
            "columns_subgrid" => emit_set(&setter, &g.columns_subgrid, m, out),
            "rows_subgrid" => emit_set(&setter, &g.rows_subgrid, m, out),
            "template_areas" => emit_set(&setter, &g.template_areas, m, out),
            "column_line_names" => emit_set(&setter, &g.column_line_names, m, out),
            "row_line_names" => emit_set(&setter, &g.row_line_names, m, out),
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        }
        return;
    }

    if let Some(rest) = path.strip_prefix("border_sides.") {
        let setter = format!("border_sides::{rest}");
        let bs = &layout.border_sides;
        match rest {
            "top_width" => emit_set(&setter, &bs.top_width, m, out),
            "right_width" => emit_set(&setter, &bs.right_width, m, out),
            "bottom_width" => emit_set(&setter, &bs.bottom_width, m, out),
            "left_width" => emit_set(&setter, &bs.left_width, m, out),
            "top_style" => emit_set(&setter, &bs.top_style, m, out),
            "right_style" => emit_set(&setter, &bs.right_style, m, out),
            "bottom_style" => emit_set(&setter, &bs.bottom_style, m, out),
            "left_style" => emit_set(&setter, &bs.left_style, m, out),
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        }
        return;
    }

    if let Some(rest) = path.strip_prefix("container.") {
        let setter = format!("container::{rest}");
        let c = &layout.container;
        match rest {
            "container_type" => emit_set(&setter, &c.container_type, m, out),
            "container_name" => emit_set(&setter, &c.container_name, m, out),
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        }
        return;
    }

    if let Some(rest) = path.strip_prefix("paint.") {
        let setter = format!("paint::{}", rest.replace('.', "::"));
        match rest {
            "background_material" => emit_set(&setter, &layout.paint.background_material, m, out),
            "border_material" => emit_set(&setter, &layout.paint.border_material, m, out),
            "text_material" => emit_set(&setter, &layout.paint.text_material, m, out),
            "visibility" => emit_set(&setter, &layout.paint.visibility, m, out),
            "border_radii" => emit_set(&setter, &layout.paint.border_radii, m, out),
            "box_shadows" => emit_set(&setter, &layout.paint.box_shadows, m, out),
            "scrollbar" => emit_set(&setter, &layout.paint.scrollbar, m, out),
            "scroll_edge_fade" => emit_set(&setter, &layout.paint.scroll_edge_fade, m, out),
            "text_shadows" => emit_set(&setter, &layout.paint.text_shadows, m, out),
            "text_stroke" => emit_set(&setter, &layout.paint.text_stroke, m, out),
            "paint_order_stroke_first" => {
                emit_set(&setter, &layout.paint.paint_order_stroke_first, m, out)
            }
            "mix_blend" => emit_set(&setter, &layout.paint.mix_blend, m, out),
            "wide_filter_blur" => emit_set(&setter, &layout.paint.wide_filter_blur, m, out),
            "motion_shutter" => emit_set(&setter, &layout.paint.motion_shutter, m, out),
            "unsupported_border_image" => {
                emit_set(&setter, &layout.paint.unsupported_border_image, m, out)
            }
            "border_image" => emit_set(&setter, &layout.paint.border_image, m, out),
            "background_layers" => emit_set(&setter, &layout.paint.background_layers, m, out),
            "background_size_list" => emit_set(&setter, &layout.paint.background_size_list, m, out),
            "background_size_lengths" => {
                emit_set(&setter, &layout.paint.background_size_lengths, m, out)
            }
            "background_position_list" => {
                emit_set(&setter, &layout.paint.background_position_list, m, out)
            }
            "background_repeat_list" => {
                emit_set(&setter, &layout.paint.background_repeat_list, m, out)
            }
            "content_image" => emit_set(&setter, &layout.paint.content_image, m, out),
            "skipped_replaced" => emit_set(&setter, &layout.paint.skipped_replaced, m, out),
            "object_fit" => emit_set(&setter, &layout.paint.object_fit, m, out),
            "object_position" => emit_set(&setter, &layout.paint.object_position, m, out),
            "mask" => emit_set(&setter, &layout.paint.mask, m, out),
            "clip_path" => emit_set(&setter, &layout.paint.clip_path, m, out),
            "filter" => emit_set(&setter, &layout.paint.filter, m, out),
            "backdrop_filter" => emit_set(&setter, &layout.paint.backdrop_filter, m, out),
            "outline.width" => emit_set(&setter, &layout.paint.outline.width, m, out),
            "outline.style" => emit_set(&setter, &layout.paint.outline.style, m, out),
            _ => panic!("unknown LayoutStyle leaf path: {path}"),
        }
        return;
    }

    match path {
        "layout_isolation" => emit_set(path, &layout.layout_isolation, m, out),
        "direction" => emit_set(path, &layout.direction, m, out),
        "dir" => emit_set(path, &layout.dir, m, out),
        "writing_mode" => emit_set(path, &layout.writing_mode, m, out),
        "flex_reverse" => emit_set(path, &layout.flex_reverse, m, out),
        "order" => emit_set(path, &layout.order, m, out),
        "flex_wrap" => emit_set(path, &layout.flex_wrap, m, out),
        "equal_wrap" => emit_set(path, &layout.equal_wrap, m, out),
        "display" => emit_set(path, &layout.display, m, out),
        "box_sizing" => emit_set(path, &layout.box_sizing, m, out),
        "position" => emit_set(path, &layout.position, m, out),
        "anchor" => emit_set(path, &layout.anchor, m, out),
        "z_index" => emit_set(path, &layout.z_index, m, out),
        "isolation" => emit_set(path, &layout.isolation, m, out),
        "gap" => emit_set(path, &layout.gap, m, out),
        "row_gap" => emit_set(path, &layout.row_gap, m, out),
        "column_gap" => emit_set(path, &layout.column_gap, m, out),
        "padding" => emit_set(path, &layout.padding, m, out),
        "padding_top" => emit_set(path, &layout.padding_top, m, out),
        "padding_right" => emit_set(path, &layout.padding_right, m, out),
        "padding_bottom" => emit_set(path, &layout.padding_bottom, m, out),
        "padding_left" => emit_set(path, &layout.padding_left, m, out),
        "margin" => emit_set(path, &layout.margin, m, out),
        "margin_top" => emit_set(path, &layout.margin_top, m, out),
        "margin_right" => emit_set(path, &layout.margin_right, m, out),
        "margin_bottom" => emit_set(path, &layout.margin_bottom, m, out),
        "margin_left" => emit_set(path, &layout.margin_left, m, out),
        "offset_top" => emit_set(path, &layout.offset_top, m, out),
        "offset_right" => emit_set(path, &layout.offset_right, m, out),
        "offset_bottom" => emit_set(path, &layout.offset_bottom, m, out),
        "offset_left" => emit_set(path, &layout.offset_left, m, out),
        "width" => emit_set(path, &layout.width, m, out),
        "height" => emit_set(path, &layout.height, m, out),
        "min_width" => emit_set(path, &layout.min_width, m, out),
        "max_width" => emit_set(path, &layout.max_width, m, out),
        "min_height" => emit_set(path, &layout.min_height, m, out),
        "max_height" => emit_set(path, &layout.max_height, m, out),
        "allow_shrink" => emit_set(path, &layout.allow_shrink, m, out),
        "align_items" => emit_set(path, &layout.align_items, m, out),
        "align_self" => emit_set(path, &layout.align_self, m, out),
        "align_content" => emit_set(path, &layout.align_content, m, out),
        "justify_content" => emit_set(path, &layout.justify_content, m, out),
        "justify_items" => emit_set(path, &layout.justify_items, m, out),
        "justify_self" => emit_set(path, &layout.justify_self, m, out),
        "flex_grow" => emit_set(path, &layout.flex_grow, m, out),
        "flex_shrink" => emit_set(path, &layout.flex_shrink, m, out),
        "flex_basis" => emit_set(path, &layout.flex_basis, m, out),
        "adaptation" => emit_set(path, &layout.adaptation, m, out),
        "overflow_x" => emit_set(path, &layout.overflow_x, m, out),
        "overflow_y" => emit_set(path, &layout.overflow_y, m, out),
        "text_overflow_ellipsis" => emit_set(path, &layout.text_overflow_ellipsis, m, out),
        "pointer_events" => emit_set(path, &layout.pointer_events, m, out),
        "hit_test" => emit_set(path, &layout.hit_test, m, out),
        "inert" => emit_set(path, &layout.inert, m, out),
        "cursor" => emit_set(path, &layout.cursor, m, out),
        "user_select" => emit_set(path, &layout.user_select, m, out),
        "white_space" => emit_set(path, &layout.white_space, m, out),
        "aspect_ratio" => emit_set(path, &layout.aspect_ratio, m, out),
        "text_align" => emit_set(path, &layout.text_align, m, out),
        "font_size" => emit_set(path, &layout.font_size, m, out),
        "font_weight" => emit_set(path, &layout.font_weight, m, out),
        "font_italic" => emit_set(path, &layout.font_italic, m, out),
        "line_height" => emit_set(path, &layout.line_height, m, out),
        "hidden" => emit_set(path, &layout.hidden, m, out),
        "opacity" => emit_set(path, &layout.opacity, m, out),
        "border_radius" => emit_set(path, &layout.border_radius, m, out),
        "border_width" => emit_set(path, &layout.border_width, m, out),
        "border_style" => emit_set(path, &layout.border_style, m, out),
        _ => panic!("unknown LayoutStyle leaf path: {path}"),
    }
}

/// Emit Rust tokens for one [`PaintColor`] referencing the caller's `__nana_patch` module alias.
pub fn emit_paint_color_rust(color: &PaintColor, patch_mod: &str) -> String {
    let mut out = String::new();
    color.emit_rust(patch_mod, &mut out);
    out
}

/// Emit Rust tokens for one [`ColorFilter`] referencing the caller's `__nana_patch` module alias.
pub fn emit_color_filter_rust(filter: &ColorFilter, patch_mod: &str) -> String {
    let mut out = String::new();
    filter.emit_rust(patch_mod, &mut out);
    out
}
