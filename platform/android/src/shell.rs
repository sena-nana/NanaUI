//! Android MVP **Nana shell** stub — workspace geometry + chrome band plan.
//!
//! NanaUI owns shell layout and optional generic Iced controls. Vue apps keep CSS, custom
//! components, and renderer freedom via [`nana_ui_vue::VueHost`] — this stub does not restrict
//! Vue widget types. Desktop [`nana_ui::DesktopShell`] composes the same regions with Iced;
//! this stub keeps [`WorkspaceLayout`] / [`WorkspaceGeometry`] from `nana-ui-core` so Primary
//! viewport sizing matches desktop. Full Iced shell paint is still open; hosts can already
//! present [`ShellChromeBand`] fills via scissor + solid-color pipeline
//! (title / resources / primary) without Iced.

use nana_ui_core::{PhysicalRect, RegionId, TITLE_BAR_HEIGHT, WorkspaceGeometry, WorkspaceLayout};

/// Android mdpi baseline: `densityDpi == 160` → scale factor `1.0`.
pub const ANDROID_MDPI_DPI: u32 = 160;

/// Fallback when [`scale_factor_from_density_dpi`] has no usable density.
///
/// Matches the historical stub default (xhdpi / 320dpi). Host CI without an
/// Android configuration keeps this value until `resize` injects a density.
pub const DEFAULT_ANDROID_SCALE_FACTOR: f32 = 2.0;

/// Derive window scale from Android `Configuration` densityDpi.
///
/// `Some(dpi)` → `dpi / 160` (clamped to ≥ 0.25). `None`, `0`, or non-finite
/// results fall back to [`DEFAULT_ANDROID_SCALE_FACTOR`].
pub fn scale_factor_from_density_dpi(density_dpi: Option<u32>) -> f32 {
    match density_dpi {
        Some(dpi) if dpi > 0 => {
            let scale = dpi as f32 / ANDROID_MDPI_DPI as f32;
            if scale.is_finite() && scale > 0.0 {
                scale.max(0.25)
            } else {
                DEFAULT_ANDROID_SCALE_FACTOR
            }
        }
        _ => DEFAULT_ANDROID_SCALE_FACTOR,
    }
}

/// One shell chrome band for host wgpu scissor clears (pre-Iced paint).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShellChromeBand {
    pub rect: PhysicalRect,
    pub color: [f64; 4],
}

/// Non-Iced shell placeholder aligned with the DesktopShell workspace contract.
#[derive(Debug, Clone)]
pub struct AndroidShellStub {
    layout: WorkspaceLayout,
    geometry: WorkspaceGeometry,
    scale_factor: f32,
}

impl Default for AndroidShellStub {
    fn default() -> Self {
        Self::new()
    }
}

impl AndroidShellStub {
    pub fn new() -> Self {
        let layout = WorkspaceLayout::default();
        let scale = DEFAULT_ANDROID_SCALE_FACTOR;
        let geometry = WorkspaceGeometry::new(&layout, 720.0, 1280.0, scale);
        Self {
            layout,
            geometry,
            scale_factor: scale,
        }
    }

    pub fn layout(&self) -> &WorkspaceLayout {
        &self.layout
    }

    pub fn geometry(&self) -> &WorkspaceGeometry {
        &self.geometry
    }

    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Recompute region rects from the native window size (**physical** px).
    ///
    /// NativeWindow / wgpu framebuffer / MotionEvent share this pixel space.
    /// `WorkspaceGeometry` lays out in logical px (`physical / scale`); the
    /// resulting [`WorkspaceGeometry::physical_size`] matches the window again
    /// (callers must not treat NativeWindow pixels as logical — that double-
    /// scales chrome / hit rects off-screen when `scale > 1`).
    pub fn resize(&mut self, physical_width: u32, physical_height: u32, scale_factor: f32) {
        self.scale_factor = scale_factor.max(0.25);
        let logical_w = physical_width.max(1) as f32 / self.scale_factor;
        let logical_h = physical_height.max(1) as f32 / self.scale_factor;
        self.geometry =
            WorkspaceGeometry::new(&self.layout, logical_w, logical_h, self.scale_factor);
    }

    /// Primary region physical size — feed [`VueHost`] viewport and future Nana Primary slot.
    pub fn primary_physical_size(&self) -> (u32, u32) {
        self.geometry
            .region(&RegionId::Primary)
            .map(|region| (region.physical.width, region.physical.height))
            .unwrap_or((720, 1280))
    }

    /// Title bar height in logical px (same constant as desktop shell).
    pub fn title_bar_height(&self) -> f32 {
        TITLE_BAR_HEIGHT
    }

    /// Whether Nana Iced [`DesktopShell`] rendering is wired on this target.
    pub const fn iced_shell_available() -> bool {
        false
    }

    /// Whether host can paint stub chrome bands (title / sidebar / primary).
    pub const fn shell_chrome_bands_available() -> bool {
        true
    }

    /// Whether host solid-color scissor fill pipeline is wired (not Iced).
    pub const fn shell_chrome_fill_available() -> bool {
        true
    }

    /// Whether Nana Iced controls can paint into the Primary slot
    /// (Icon + Text + Input + Switch + Button via `iced_wgpu`) — still not full DesktopShell.
    pub const fn iced_control_widget_available() -> bool {
        true
    }

    /// Whether NativeActivity pointer events route into the slot Button.
    pub const fn iced_control_input_available() -> bool {
        true
    }

    /// Physical chrome bands for pre-Iced Surface present (title bar, Resources, Primary).
    pub fn chrome_bands(&self) -> Vec<ShellChromeBand> {
        let (fw, fh) = self.geometry.physical_size;
        let scale = self.scale_factor.max(0.25);
        let title_h = ((TITLE_BAR_HEIGHT * scale).round() as u32).min(fh);
        let mut bands = Vec::with_capacity(3);
        if title_h > 0 && fw > 0 {
            bands.push(ShellChromeBand {
                rect: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: fw,
                    height: title_h,
                },
                // Title bar — slightly lighter than body.
                color: [0.14, 0.16, 0.20, 1.0],
            });
        }
        if let Some(resources) = self.geometry.region(&RegionId::Resources) {
            if resources.visible && resources.physical.width > 0 && resources.physical.height > 0 {
                bands.push(ShellChromeBand {
                    rect: resources.physical,
                    color: [0.11, 0.13, 0.17, 1.0],
                });
            }
        }
        if let Some(primary) = self.geometry.region(&RegionId::Primary) {
            if primary.visible && primary.physical.width > 0 && primary.physical.height > 0 {
                bands.push(ShellChromeBand {
                    rect: primary.physical,
                    color: [0.10, 0.12, 0.16, 1.0],
                });
            }
        }
        if bands.is_empty() && fw > 0 && fh > 0 {
            bands.push(ShellChromeBand {
                rect: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: fw,
                    height: fh,
                },
                color: [0.10, 0.12, 0.16, 1.0],
            });
        }
        bands
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iced_slot::{ICED_CONTROL_SLOT_INSET, iced_control_slot_paint_bounds};
    use crate::iced_slot_input::pointer_in_slot;

    #[test]
    fn default_layout_has_primary_region() {
        let shell = AndroidShellStub::new();
        assert!(shell.geometry().region(&RegionId::Primary).is_some());
        assert!(shell.geometry().region(&RegionId::Resources).is_some());
        assert!((shell.scale_factor() - DEFAULT_ANDROID_SCALE_FACTOR).abs() < f32::EPSILON);
    }

    #[test]
    fn scale_factor_from_density_dpi_uses_mdpi_baseline() {
        assert!((scale_factor_from_density_dpi(Some(160)) - 1.0).abs() < f32::EPSILON);
        assert!((scale_factor_from_density_dpi(Some(320)) - 2.0).abs() < f32::EPSILON);
        assert!((scale_factor_from_density_dpi(Some(480)) - 3.0).abs() < f32::EPSILON);
        assert!((scale_factor_from_density_dpi(Some(213)) - (213.0 / 160.0)).abs() < 0.001);
    }

    #[test]
    fn scale_factor_from_density_dpi_falls_back_when_missing() {
        assert!(
            (scale_factor_from_density_dpi(None) - DEFAULT_ANDROID_SCALE_FACTOR).abs()
                < f32::EPSILON
        );
        assert!(
            (scale_factor_from_density_dpi(Some(0)) - DEFAULT_ANDROID_SCALE_FACTOR).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn resize_updates_primary_viewport() {
        let mut shell = AndroidShellStub::new();
        shell.resize(1080, 1920, 2.0);
        let (w, h) = shell.primary_physical_size();
        assert!(w > 0);
        assert!(h > 0);
        assert!(!AndroidShellStub::iced_shell_available());
        assert!(AndroidShellStub::shell_chrome_bands_available());
    }

    #[test]
    fn resize_physical_window_matches_geometry_physical_size() {
        let mut shell = AndroidShellStub::new();
        // NativeWindow pixels @ density 2 — must not become 2160×3840.
        shell.resize(1080, 1920, 2.0);
        assert_eq!(shell.geometry().physical_size, (1080, 1920));
        assert!((shell.geometry().logical_size.0 - 540.0).abs() < 0.01);
        assert!((shell.geometry().logical_size.1 - 960.0).abs() < 0.01);
    }

    #[test]
    fn injected_density_keeps_physical_window_and_slot_hit() {
        // Same NativeWindow pixels; mdpi vs xxhdpi only change logical size / slot px.
        let window = (1080u32, 1920u32);
        for dpi in [160u32, 320, 480] {
            let scale = scale_factor_from_density_dpi(Some(dpi));
            let mut shell = AndroidShellStub::new();
            shell.resize(window.0, window.1, scale);
            assert_eq!(shell.geometry().physical_size, window);
            assert!((shell.scale_factor() - scale).abs() < f32::EPSILON);
            assert!(
                (shell.geometry().logical_size.0 - window.0 as f32 / scale).abs() < 0.01,
                "dpi={dpi}: logical width must be physical/scale"
            );

            let slot = shell.iced_control_slot().expect("slot").rect;
            let paint = iced_control_slot_paint_bounds(window, scale).expect("paint");
            assert_eq!(slot, paint);
            let inset = (ICED_CONTROL_SLOT_INSET * scale).round() as u32;
            assert_eq!(slot.x, inset);
            assert_eq!(slot.y + slot.height + inset, window.1);

            // MotionEvent physical coords inside the slot must hit.
            let cx = slot.x as f32 + slot.width as f32 * 0.5;
            let cy = slot.y as f32 + slot.height as f32 * 0.5;
            assert!(
                pointer_in_slot(slot, cx, cy),
                "dpi={dpi}: center of slot must hit at physical ({cx}, {cy})"
            );
            // Just above the slot (still in chrome) must miss.
            if slot.y > 0 {
                assert!(
                    !pointer_in_slot(slot, cx, slot.y as f32 - 1.0),
                    "dpi={dpi}: above-slot physical y must miss"
                );
            }
        }
    }

    #[test]
    fn chrome_bands_cover_title_resources_primary() {
        let mut shell = AndroidShellStub::new();
        shell.resize(1080, 1920, 2.0);
        let bands = shell.chrome_bands();
        assert!(
            bands.len() >= 2,
            "expected title + region bands, got {}",
            bands.len()
        );
        assert_eq!(bands[0].rect.y, 0);
        assert!(bands[0].rect.height > 0);
        let total: u64 = bands
            .iter()
            .map(|b| u64::from(b.rect.width) * u64::from(b.rect.height))
            .sum();
        assert!(total > 0);
    }
}
