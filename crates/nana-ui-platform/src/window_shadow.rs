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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowShadowCapabilities {
    pub native_frame_shadow: bool,
    pub native_visual_shape_shadow: bool,
    pub companion_shadow: bool,
    pub content_alpha_shadow: bool,
}

impl WindowShadowCapabilities {
    /// Resolves a request without touching a native window. Platform hosts use
    /// this after probing their actual native capabilities; a missing
    /// capability is reported explicitly instead of being treated as a
    /// successful no-op.
    pub fn resolve(self, request: WindowShadow) -> WindowShadowOutcome {
        let style = match request {
            WindowShadow::None => return WindowShadowOutcome::disabled(),
            WindowShadow::Auto => return self.resolve_auto(),
            WindowShadow::Custom(style) => style,
        };
        if !style.is_valid() {
            return WindowShadowOutcome::unavailable(WindowShadowFallback::InvalidStyle);
        }
        match style.source {
            WindowShadowSource::WindowShape if self.native_visual_shape_shadow => {
                WindowShadowOutcome {
                    backend: WindowShadowBackend::Native,
                    fallback: None,
                }
            }
            WindowShadowSource::WindowShape if self.companion_shadow => WindowShadowOutcome {
                backend: WindowShadowBackend::Companion,
                fallback: None,
            },
            WindowShadowSource::WindowShape => {
                WindowShadowOutcome::unavailable(WindowShadowFallback::VisualShapeUnavailable)
            }
            WindowShadowSource::ContentAlpha if self.content_alpha_shadow => WindowShadowOutcome {
                backend: WindowShadowBackend::Companion,
                fallback: None,
            },
            WindowShadowSource::ContentAlpha => {
                WindowShadowOutcome::unavailable(WindowShadowFallback::ContentAlphaUnavailable)
            }
        }
    }

    const fn resolve_auto(self) -> WindowShadowOutcome {
        if self.native_frame_shadow {
            return WindowShadowOutcome {
                backend: WindowShadowBackend::Native,
                fallback: None,
            };
        }
        if self.native_visual_shape_shadow || self.companion_shadow {
            return WindowShadowOutcome {
                backend: if self.native_visual_shape_shadow {
                    WindowShadowBackend::Native
                } else {
                    WindowShadowBackend::Companion
                },
                fallback: None,
            };
        }
        WindowShadowOutcome::unavailable(WindowShadowFallback::Unsupported)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowShadowBackend {
    #[default]
    None,
    Native,
    Companion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowShadowFallback {
    /// Resolution waits for the native window and its visual geometry.
    Pending,
    Unsupported,
    ContentAlphaUnavailable,
    VisualShapeUnavailable,
    InvalidStyle,
    BackendUnavailable,
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
    fn capabilities_resolve_none_and_supported_sources_without_claiming_more() {
        let capabilities = WindowShadowCapabilities {
            native_frame_shadow: true,
            native_visual_shape_shadow: false,
            companion_shadow: true,
            content_alpha_shadow: true,
        };
        assert_eq!(
            capabilities.resolve(WindowShadow::None),
            WindowShadowOutcome::disabled()
        );
        assert_eq!(
            capabilities.resolve(WindowShadow::Auto).backend,
            WindowShadowBackend::Native
        );
        assert_eq!(
            capabilities
                .resolve(WindowShadow::Custom(WindowShadowStyle {
                    source: WindowShadowSource::ContentAlpha,
                    ..Default::default()
                }))
                .backend,
            WindowShadowBackend::Companion
        );
    }
}
