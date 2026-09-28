//! Desktop decoration semantics. These never change client geometry or input.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum WindowShadow {
    #[default]
    Auto,
    None,
    Custom(WindowShadowStyle),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowShadowSource {
    #[default]
    WindowShape,
    /// Explicit, potentially per-frame work. Unsupported backends report it.
    ContentAlpha,
}

/// Logical pixels; color is straight sRGB RGBA in 0..=1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowShadowStyle {
    pub source: WindowShadowSource,
    pub color: [f32; 4],
    pub offset: [f32; 2],
    pub blur: f32,
    pub spread: f32,
}
impl Default for WindowShadowStyle {
    fn default() -> Self {
        Self {
            source: WindowShadowSource::WindowShape,
            color: [0.0, 0.0, 0.0, 0.25],
            offset: [0.0, 4.0],
            blur: 16.0,
            spread: 0.0,
        }
    }
}
impl WindowShadowStyle {
    pub fn is_valid(self) -> bool {
        self.color
            .into_iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
            && self.offset.into_iter().all(f32::is_finite)
            && self.blur.is_finite()
            && self.blur >= 0.0
            && self.spread.is_finite()
    }
}

/// The visible body of a window the shadow follows: a rounded rectangle in
/// logical client coordinates. It comes from what the window draws (its
/// root card), never from its input, drag or resize regions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowVisualShape {
    /// `[x, y, width, height]`.
    pub rect: [f32; 4],
    /// Corner radii: top-left, top-right, bottom-right, bottom-left.
    pub radii: [f32; 4],
}

impl WindowVisualShape {
    /// The whole client area with square corners.
    pub const fn client(width: f32, height: f32) -> Self {
        Self {
            rect: [0.0, 0.0, width, height],
            radii: [0.0; 4],
        }
    }

    pub fn is_valid(self) -> bool {
        self.rect.iter().chain(&self.radii).all(|v| v.is_finite())
            && self.rect[2] > 0.0
            && self.rect[3] > 0.0
            && self.radii.iter().all(|r| *r >= 0.0)
    }
}

/// Work a window's shadow did, for the "no change, no work" gates. Only a
/// shape, style, scale or size change may raise the first four; a move only
/// raises `companion_moves`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowShadowWork {
    /// Visual shapes derived from the scene that differed from the last one.
    pub shape_rebuilds: u64,
    /// Retained companion updates (geometry, offsets, paths) without raster.
    pub effect_updates: u64,
    /// Shadow images rasterized on the CPU (style or scale changes).
    pub rasterizations: u64,
    /// Companion resources (window, surfaces, layers) created.
    pub resource_recreates: u64,
    pub companion_moves: u64,
    pub companion_resizes: u64,
    /// Companions alive for this window: 0 or 1.
    pub companion_count: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowShadowBackend {
    #[default]
    None,
    Native,
    Companion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowShadowFallback {
    /// Resolution waits for the native window and its visual geometry.
    Pending,
    Unsupported,
    ContentAlphaUnavailable,
    VisualShapeUnavailable,
    InvalidStyle,
    BackendUnavailable,
    /// A native shadow is shown, but it cannot express the custom style.
    CustomStyleApproximated,
    /// The platform's compositor decides; nothing observable says whether it
    /// draws a shadow (Linux server-side decorations).
    CompositorManaged,
    /// `None` was asked for, but the platform's frame shadow cannot be
    /// removed without giving up the frame it belongs to (Windows DWM).
    DisableUnsupported,
}

/// Actual platform outcome, never a promise inferred from the request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowShadowOutcome {
    pub backend: WindowShadowBackend,
    pub fallback: Option<WindowShadowFallback>,
}
impl WindowShadowOutcome {
    pub const fn disabled() -> Self {
        Self {
            backend: WindowShadowBackend::None,
            fallback: None,
        }
    }
    pub const fn unavailable(reason: WindowShadowFallback) -> Self {
        Self {
            backend: WindowShadowBackend::None,
            fallback: Some(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn style_roundtrips_and_missing_style_fields_use_defaults() {
        let style: WindowShadowStyle = serde_json::from_str("{}").unwrap();
        assert_eq!(style, WindowShadowStyle::default());
        let request = WindowShadow::Custom(style);
        assert_eq!(
            serde_json::from_str::<WindowShadow>(&serde_json::to_string(&request).unwrap())
                .unwrap(),
            request
        );
        assert_eq!(
            crate::WindowDescriptor::default().shadow,
            WindowShadow::Auto
        );
    }
    #[test]
    fn invalid_styles_are_rejected_without_clamping() {
        for style in [
            WindowShadowStyle {
                blur: -1.0,
                ..Default::default()
            },
            WindowShadowStyle {
                spread: f32::INFINITY,
                ..Default::default()
            },
            WindowShadowStyle {
                offset: [f32::NAN, 0.0],
                ..Default::default()
            },
            WindowShadowStyle {
                color: [0.0, 0.0, 0.0, 2.0],
                ..Default::default()
            },
        ] {
            assert!(!style.is_valid());
        }
        assert!(WindowShadowStyle::default().is_valid());
    }

    #[test]
    fn visual_shapes_reject_empty_or_non_finite_geometry() {
        assert!(WindowVisualShape::client(10.0, 10.0).is_valid());
        assert!(
            !WindowVisualShape {
                rect: [0.0, 0.0, 0.0, 10.0],
                radii: [0.0; 4],
            }
            .is_valid()
        );
        assert!(
            !WindowVisualShape {
                rect: [0.0, 0.0, 10.0, 10.0],
                radii: [f32::NAN, 0.0, 0.0, 0.0],
            }
            .is_valid()
        );
    }
}
