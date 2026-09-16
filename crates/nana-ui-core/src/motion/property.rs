//! Framework-owned animation class for each animatable property.
//!
//! Components do not choose the execution class. FLIP / presentation layout
//! is an explicit strategy ([`FlipRect`]), not a per-widget override
//! that rewrites width as scale.

/// Where a property's animation may run. Classification is a framework
/// contract: compositor-safe tracks must not be implemented as per-frame
/// `UiWorld` property writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationClass {
    /// Transform / opacity / clip / shader parameter: presentation overlay.
    Compositor,
    /// Color, filter, shadow: paint without layout.
    Paint,
    /// Width / height / padding / font-size: retained layout.
    Layout,
    /// `display` and other topology: no interpolation.
    Discrete,
}

/// Properties the Motion IR can address. CSS names resolve through
/// [`AnimatableProperty::from_css_name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimatableProperty {
    Transform,
    Opacity,
    Clip,
    Color,
    Background,
    Blur,
    Filter,
    Shadow,
    ShaderParameter,
    Width,
    Height,
    Padding,
    Margin,
    FontSize,
    FontAxis,
    Display,
    /// Unit progress `0..=1`. Runtime `AnimationSpec` samples this until a
    /// property-classified track is compiled.
    Progress,
}

impl AnimatableProperty {
    pub fn css_name(self) -> &'static str {
        match self {
            Self::Transform => "transform",
            Self::Opacity => "opacity",
            Self::Clip => "clip-path",
            Self::Color => "color",
            Self::Background => "background-color",
            Self::Blur => "blur",
            Self::Filter => "filter",
            Self::Shadow => "box-shadow",
            Self::ShaderParameter => "shader-parameter",
            Self::Width => "width",
            Self::Height => "height",
            Self::Padding => "padding",
            Self::Margin => "margin",
            Self::FontSize => "font-size",
            Self::FontAxis => "font-variation-settings",
            Self::Display => "display",
            Self::Progress => "progress",
        }
    }

    /// Default execution class. Color / filter / shadow default to Paint;
    /// compositor promotion is Workstream C/D.
    pub fn animation_class(self) -> AnimationClass {
        match self {
            Self::Transform | Self::Opacity | Self::Clip | Self::ShaderParameter => {
                AnimationClass::Compositor
            }
            Self::Color
            | Self::Background
            | Self::Blur
            | Self::Filter
            | Self::Shadow
            | Self::Progress => AnimationClass::Paint,
            Self::Width
            | Self::Height
            | Self::Padding
            | Self::Margin
            | Self::FontSize
            | Self::FontAxis => AnimationClass::Layout,
            Self::Display => AnimationClass::Discrete,
        }
    }

    pub fn from_css_name(name: &str) -> Option<Self> {
        let n = name.trim().to_ascii_lowercase();
        Some(match n.as_str() {
            "transform" => Self::Transform,
            "opacity" => Self::Opacity,
            "clip" | "clip-path" => Self::Clip,
            "color" => Self::Color,
            "background" | "background-color" => Self::Background,
            "blur" => Self::Blur,
            "filter" => Self::Filter,
            "box-shadow" | "text-shadow" | "shadow" => Self::Shadow,
            "shader-parameter" | "shader_parameter" => Self::ShaderParameter,
            "width" => Self::Width,
            "height" => Self::Height,
            "padding" | "padding-top" | "padding-right" | "padding-bottom" | "padding-left"
            | "padding-inline" | "padding-block" => Self::Padding,
            "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left"
            | "margin-inline" | "margin-block" => Self::Margin,
            "font-size" => Self::FontSize,
            "font-variation-settings" | "font-axis" => Self::FontAxis,
            "display" => Self::Display,
            "progress" => Self::Progress,
            _ => return None,
        })
    }

    /// Developer-facing hint. Tests should match [`Self::animation_class`],
    /// not the wording of this string.
    pub fn diagnostic_hint(self) -> String {
        let name = self.css_name();
        match self.animation_class() {
            AnimationClass::Layout => format!(
                "Animating `{name}` requires layout on every sample. Consider a presentation transform / FLIP transition if visual scaling is sufficient."
            ),
            AnimationClass::Discrete => {
                format!("`{name}` is discrete and does not interpolate.")
            }
            AnimationClass::Compositor => format!(
                "`{name}` is compositor-safe; presentation can evaluate without UiWorld mutation."
            ),
            AnimationClass::Paint => {
                format!("Animating `{name}` requires paint on sampled frames.")
            }
        }
    }
}

/// Registry lookup: CSS name → property + class.
pub fn classify_animatable_property(name: &str) -> Option<(AnimatableProperty, AnimationClass)> {
    let property = AnimatableProperty::from_css_name(name)?;
    Some((property, property.animation_class()))
}

/// Captured First / Last box. Origin and size match Runtime layout boxes.
/// Layout-class width/height stay on the CPU layout path; FLIP never rewrites
/// width as a scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlipRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl FlipRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Inverse translate that visually restores `self` after layout has
    /// already committed `last`. Size is not encoded as scale.
    pub fn invert_translate(self, last: Self) -> crate::PaintTransform {
        invert_flip_translate(self, last)
    }

    pub fn size_differs(self, last: Self) -> bool {
        (self.width - last.width).abs() > 1e-4 || (self.height - last.height).abs() > 1e-4
    }
}

/// Invert step: `translate(first - last)`. Play animates this to identity.
pub fn invert_flip_translate(first: FlipRect, last: FlipRect) -> crate::PaintTransform {
    crate::PaintTransform {
        e: first.x - last.x,
        f: first.y - last.y,
        ..crate::PaintTransform::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_table_classes_match_framework_defaults() {
        let cases = [
            ("transform", AnimationClass::Compositor),
            ("opacity", AnimationClass::Compositor),
            ("clip-path", AnimationClass::Compositor),
            ("color", AnimationClass::Paint),
            ("background", AnimationClass::Paint),
            ("blur", AnimationClass::Paint),
            ("filter", AnimationClass::Paint),
            ("box-shadow", AnimationClass::Paint),
            ("shader-parameter", AnimationClass::Compositor),
            ("width", AnimationClass::Layout),
            ("height", AnimationClass::Layout),
            ("padding", AnimationClass::Layout),
            ("margin", AnimationClass::Layout),
            ("font-size", AnimationClass::Layout),
            ("font-variation-settings", AnimationClass::Layout),
            ("display", AnimationClass::Discrete),
        ];
        for (name, class) in cases {
            let (property, got) = classify_animatable_property(name).expect(name);
            assert_eq!(got, class, "{name}");
            assert_eq!(property.animation_class(), class);
        }
    }

    #[test]
    fn unknown_css_name_is_not_classified() {
        assert_eq!(classify_animatable_property("animation-name"), None);
    }

    #[test]
    fn invert_is_first_minus_last_translate_not_scale() {
        let first = FlipRect::new(10.0, 20.0, 40.0, 16.0);
        let last = FlipRect::new(40.0, 20.0, 40.0, 16.0);
        let invert = invert_flip_translate(first, last);
        assert_eq!(invert.a, 1.0);
        assert_eq!(invert.d, 1.0);
        assert_eq!(invert.e, -30.0);
        assert_eq!(invert.f, 0.0);
        assert!(!first.size_differs(last));
    }

    #[test]
    fn size_mismatch_is_not_encoded_as_scale() {
        let first = FlipRect::new(0.0, 0.0, 20.0, 10.0);
        let last = FlipRect::new(0.0, 0.0, 40.0, 20.0);
        let invert = first.invert_translate(last);
        assert_eq!(invert.a, 1.0);
        assert_eq!(invert.d, 1.0);
        assert_eq!(invert.e, 0.0);
        assert!(first.size_differs(last));
    }
}
