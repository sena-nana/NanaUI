//! Framework-owned animation class for each animatable property.
//!
//! Components do not choose the execution class. FLIP / presentation layout
//! is an explicit strategy ([`FlipRect`]), not a per-widget override
//! that rewrites width as scale.

use std::{borrow::Cow, fmt, sync::Arc, time::Duration};

use super::{Easing, OVERLAY_FADE};

/// Where a property's animation may run. Classification is a framework
/// contract: compositor-safe tracks must not be implemented as per-frame
/// `UiWorld` property writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationClass {
    /// Transform / opacity / clip / shader parameter / `filter`:
    /// presentation overlay.
    Compositor,
    /// Color, background, shadow: paint without layout.
    Paint,
    /// Width / height / padding / font-size: retained layout.
    Layout,
    /// `display` and other topology: no interpolation.
    Discrete,
}

/// Properties the Motion IR can address. CSS names resolve through
/// [`AnimatableProperty::from_css_name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    /// One `font-variation-settings` axis, by its OpenType tag. A CSS
    /// `font-variation-settings` value is a list, so it compiles to one track
    /// per axis; there is no track for "the axes" as a whole, and an axis is
    /// never folded onto `wght`.
    FontAxis([u8; 4]),
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
            Self::FontAxis(_) => "font-variation-settings",
            Self::Display => "display",
            Self::Progress => "progress",
        }
    }

    /// Default execution class. A node's `filter` is evaluated by the
    /// compositor into its isolation group's uniform at the document's `now`
    /// (Issue #312). Text colour, background and the control's own shadow
    /// stay Paint: a button's hover does not move onto the compositor.
    pub fn animation_class(self) -> AnimationClass {
        match self {
            Self::Transform | Self::Opacity | Self::Clip | Self::ShaderParameter | Self::Filter => {
                AnimationClass::Compositor
            }
            Self::Color | Self::Background | Self::Blur | Self::Shadow | Self::Progress => {
                AnimationClass::Paint
            }
            Self::Width
            | Self::Height
            | Self::Padding
            | Self::Margin
            | Self::FontSize
            | Self::FontAxis(_) => AnimationClass::Layout,
            Self::Display => AnimationClass::Discrete,
        }
    }

    /// `font-variation-settings` resolves to nothing here: the name does not
    /// say which axis, and a track is always one axis
    /// ([`Self::FontAxis`]). Its class is [`AnimationClass::Layout`] all the
    /// same; see [`classify_animatable_property`].
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
            "display" => Self::Display,
            "progress" => Self::Progress,
            _ => return None,
        })
    }

    /// Developer-facing hint. Tests should match [`Self::animation_class`],
    /// not the wording of this string.
    pub fn diagnostic_hint(self) -> String {
        let name = self.css_name();
        if let Self::FontAxis(tag) = self {
            // A glyph variation has no compositor equivalent: scaling the old
            // outlines is not the new instance, so there is no transform to
            // suggest.
            return format!(
                "Animating `{name}` axis `{}` reshapes, re-lays-out and re-rasterizes the text on every sample.",
                String::from_utf8_lossy(&tag)
            );
        }
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

impl std::fmt::Display for AnimatableProperty {
    /// The CSS name, and for a font axis the axis as well:
    /// `font-variation-settings "wdth"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FontAxis(tag) => write!(
                f,
                "{} \"{}\"",
                self.css_name(),
                String::from_utf8_lossy(tag)
            ),
            _ => f.write_str(self.css_name()),
        }
    }
}

/// Registry lookup: CSS name → property + class.
pub fn classify_animatable_property(name: &str) -> Option<(AnimatableProperty, AnimationClass)> {
    let property = AnimatableProperty::from_css_name(name)?;
    Some((property, property.animation_class()))
}

/// Whether `name` is `font-variation-settings`, which compiles to one
/// [`AnimatableProperty::FontAxis`] track per axis.
pub fn is_font_variation_settings(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("font-variation-settings")
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

/// Explicit cross-node shared-element / hero transition key (`<TransitionGroup>`).
///
/// Reuses the shared [`FlipRect`] + `MotionDescriptor` IR instead of creating a
/// second timeline or rewriting size mismatches into non-uniform scale.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SharedElementKey(Arc<str>);

impl SharedElementKey {
    pub fn new(key: impl AsRef<str>) -> Self {
        Self(Arc::from(key.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SharedElementKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for SharedElementKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SharedElementKey {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

impl From<Cow<'static, str>> for SharedElementKey {
    fn from(value: Cow<'static, str>) -> Self {
        Self::new(value.as_ref())
    }
}

/// How a cross-node shared-element hero morph reconciles size differences
/// between `First` and `Last` without turning `width`/`height` into scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SharedElementSizeMode {
    /// Compositor translation invert (`invert_flip_translate`) only; `Last`
    /// logical dimensions stand immediately.
    #[default]
    TranslateOnly,
    /// Compositor translation invert paired with `Layout`-class `Width` and
    /// `Height` tracks (`PresentationLengthOverlay`) — never `Transform` scale.
    LayoutSize,
    /// Compositor translation invert paired with compositor `Clip`
    /// presentation progress (`0.0 -> 1.0`) while keeping logical layout `Last`.
    ClipPresentation,
    /// Compositor translation invert paired with compositor `Opacity`
    /// cross-fade (`0.0 -> target_opacity`) while keeping logical layout `Last`.
    CrossFade,
}

/// Policy for a cross-node shared-element hero transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharedElementTransition {
    pub duration: Duration,
    /// `None` uses the installed theme's standard easing.
    pub easing: Option<Easing>,
    pub size_mode: SharedElementSizeMode,
    /// Optional opacity cross-fade alongside translation/size/clip morphing.
    pub cross_fade: bool,
}

impl Default for SharedElementTransition {
    fn default() -> Self {
        Self::new(OVERLAY_FADE)
    }
}

impl SharedElementTransition {
    pub const fn new(duration: Duration) -> Self {
        Self {
            duration,
            easing: None,
            size_mode: SharedElementSizeMode::TranslateOnly,
            cross_fade: false,
        }
    }

    pub const fn ease(mut self, easing: Easing) -> Self {
        self.easing = Some(easing);
        self
    }

    pub const fn size_mode(mut self, size_mode: SharedElementSizeMode) -> Self {
        self.size_mode = size_mode;
        self
    }

    pub const fn cross_fade(mut self, cross_fade: bool) -> Self {
        self.cross_fade = cross_fade;
        self
    }
}

/// Captured `First` presentation state from a leaving/unmounting source node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharedElementSnapshot {
    pub source: u64,
    pub first: FlipRect,
    pub opacity: f32,
    pub captured_at: Duration,
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
            ("filter", AnimationClass::Compositor),
            ("box-shadow", AnimationClass::Paint),
            ("shader-parameter", AnimationClass::Compositor),
            ("width", AnimationClass::Layout),
            ("height", AnimationClass::Layout),
            ("padding", AnimationClass::Layout),
            ("margin", AnimationClass::Layout),
            ("font-size", AnimationClass::Layout),
            ("display", AnimationClass::Discrete),
        ];
        for (name, class) in cases {
            let (property, got) = classify_animatable_property(name).expect(name);
            assert_eq!(got, class, "{name}");
            assert_eq!(property.animation_class(), class);
        }
    }

    /// A font axis is text work (shape, layout, raster), never a compositor
    /// property, whatever the axis — and the property name alone names no
    /// axis, so it is not one track.
    #[test]
    fn font_axes_are_layout_class_per_axis_tracks() {
        assert_eq!(
            classify_animatable_property("font-variation-settings"),
            None
        );
        for tag in [*b"wght", *b"wdth", *b"BEVL", *b"opsz"] {
            let axis = AnimatableProperty::FontAxis(tag);
            assert_eq!(axis.animation_class(), AnimationClass::Layout);
            assert_eq!(axis.css_name(), "font-variation-settings");
        }
        assert_ne!(
            AnimatableProperty::FontAxis(*b"wght"),
            AnimatableProperty::FontAxis(*b"BEVL")
        );
        assert_eq!(
            AnimatableProperty::FontAxis(*b"BEVL").to_string(),
            "font-variation-settings \"BEVL\""
        );
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

    #[test]
    fn shared_element_policy_preserves_no_scale_flip_contract() {
        let key_a: SharedElementKey = "hero:card".into();
        let key_b = SharedElementKey::new(String::from("hero:card"));
        assert_eq!(key_a, key_b);
        let policy = SharedElementTransition::new(Duration::from_millis(240))
            .ease(Easing::EaseOutCubic)
            .size_mode(SharedElementSizeMode::LayoutSize)
            .cross_fade(true);
        assert_eq!(policy.size_mode, SharedElementSizeMode::LayoutSize);
        assert!(policy.cross_fade);

        let first = FlipRect::new(16.0, 32.0, 48.0, 24.0);
        let last = FlipRect::new(120.0, 64.0, 180.0, 96.0);
        let invert = first.invert_translate(last);
        assert_eq!(
            (invert.a, invert.b, invert.c, invert.d),
            (1.0, 0.0, 0.0, 1.0)
        );
        assert_eq!((invert.e, invert.f), (-104.0, -32.0));
    }
}
