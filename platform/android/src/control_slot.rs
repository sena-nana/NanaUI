//! Viewport-bottom **NanaUI control slot** geometry (pre-DesktopShell).
//!
//! Reserves a host-testable inset band for the Runtime control strip
//! (Text + Input + Switch + Button). Paint + pointer/key routing live in
//! `slot_paint` / `slot_input`. This is not DesktopShell.
//!
//! Hit-testing and chrome fill **must** use [`control_slot_bounds`] — the same
//! window-bottom rect that the Scene painter uses (full-window viewport,
//! strip bottom-aligned). Anchoring to Primary alone drifts ~Diagnostics height
//! above the visible widgets.

use nana_ui_core::PhysicalRect;

use crate::shell::{AndroidShellStub, ShellChromeBand};

/// Logical height of the reserved control strip (Input row).
pub const CONTROL_SLOT_LOGICAL_HEIGHT: f32 = 64.0;

/// Logical inset from parent edges (matches strip padding).
pub const CONTROL_SLOT_INSET: f32 = 12.0;

/// Distinct fill so the slot is visible against the chrome band.
pub const CONTROL_SLOT_COLOR: [f64; 4] = [0.18, 0.22, 0.30, 1.0];

/// Physical rect of the control strip at the bottom of `parent`.
///
/// Shared by hit-testing, chrome fill, and paint-bounds regression tests so all
/// three stay on one geometry.
pub fn control_slot_bounds(parent: PhysicalRect, scale: f32) -> Option<PhysicalRect> {
    let scale = scale.max(0.25);
    let inset = (CONTROL_SLOT_INSET * scale).round() as u32;
    let slot_h = (CONTROL_SLOT_LOGICAL_HEIGHT * scale).round() as u32;
    if parent.width <= inset.saturating_mul(2) || parent.height <= inset.saturating_mul(2) {
        return None;
    }
    let width = parent.width.saturating_sub(inset.saturating_mul(2));
    let height = slot_h.min(parent.height.saturating_sub(inset.saturating_mul(2)));
    if width == 0 || height == 0 {
        return None;
    }
    let x = parent.x.saturating_add(inset);
    let y = parent
        .y
        .saturating_add(parent.height.saturating_sub(inset.saturating_add(height)));
    Some(PhysicalRect {
        x,
        y,
        width,
        height,
    })
}

/// Paint / hit bounds for a full-window NanaUI viewport (same parent the painter uses).
pub fn control_slot_paint_bounds(physical_size: (u32, u32), scale: f32) -> Option<PhysicalRect> {
    let (width, height) = physical_size;
    if width == 0 || height == 0 {
        return None;
    }
    control_slot_bounds(
        PhysicalRect {
            x: 0,
            y: 0,
            width,
            height,
        },
        scale,
    )
}

impl AndroidShellStub {
    /// Geometry + capability only — not a DesktopShell claim.
    pub const fn control_slot_available() -> bool {
        true
    }

    /// Bottom inset of the **window viewport** reserved for the control row.
    ///
    /// Matches [`control_slot_paint_bounds`] / Scene paint (not Primary
    /// alone — Diagnostics would otherwise leave a hit/paint gap).
    ///
    /// Uses [`WorkspaceGeometry::physical_size`], which equals the NativeWindow /
    /// MotionEvent / painter viewport after [`Self::resize`] (physical px in).
    pub fn control_slot(&self) -> Option<ShellChromeBand> {
        control_slot_paint_bounds(self.geometry().physical_size, self.scale_factor()).map(|rect| {
            ShellChromeBand {
                rect,
                color: CONTROL_SLOT_COLOR,
            }
        })
    }

    /// Chrome bands plus the NanaUI control slot (when available).
    pub fn chrome_present_bands(&self) -> Vec<ShellChromeBand> {
        let mut bands = self.chrome_bands();
        if let Some(slot) = self.control_slot() {
            bands.push(slot);
        }
        bands
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome_fill::{band_draw_list, clamp_scissor};
    use nana_ui_core::RegionId;

    #[test]
    fn control_slot_sits_at_viewport_bottom() {
        let mut shell = AndroidShellStub::new();
        shell.resize(1080, 1920, 2.0);
        assert!(AndroidShellStub::control_slot_available());
        assert!(AndroidShellStub::control_widget_available());
        assert!(AndroidShellStub::control_input_available());
        assert!(!AndroidShellStub::desktop_shell_available());
        let (fw, fh) = shell.geometry().physical_size;
        let inset_px = (CONTROL_SLOT_INSET * shell.scale_factor()).round() as u32;
        let slot = shell.control_slot().expect("slot");
        assert_eq!(slot.rect.x, inset_px);
        assert_eq!(slot.rect.x + slot.rect.width + inset_px, fw);
        assert_eq!(slot.rect.y + slot.rect.height + inset_px, fh);
        assert_eq!(slot.color, CONTROL_SLOT_COLOR);
    }

    #[test]
    fn hit_rect_equals_paint_bounds() {
        let mut shell = AndroidShellStub::new();
        shell.resize(1080, 1920, 2.0);
        let hit = shell.control_slot().expect("hit").rect;
        let paint = control_slot_paint_bounds(shell.geometry().physical_size, shell.scale_factor())
            .expect("paint");
        assert_eq!(hit, paint, "SlotInputGate hit must share paint geometry");

        // Default layout: Primary ends above Diagnostics — Primary-bottom hit would miss
        // the window-bottom strip by ~Diagnostics height.
        let primary = shell
            .geometry()
            .region(&RegionId::Primary)
            .expect("primary")
            .physical;
        let diagnostics = shell
            .geometry()
            .region(&RegionId::Diagnostics)
            .expect("diagnostics")
            .physical;
        assert!(
            primary.y + primary.height <= diagnostics.y + 1,
            "precondition: Primary sits above Diagnostics"
        );
        assert!(
            paint.y >= diagnostics.y,
            "paint strip must reach the viewport bottom (over Diagnostics), not Primary alone"
        );
        let primary_anchored =
            control_slot_bounds(primary, shell.scale_factor()).expect("primary-anchored");
        assert_ne!(
            paint, primary_anchored,
            "regression: window paint bounds must not equal Primary-only bounds"
        );
    }

    #[test]
    fn slot_hit_stays_in_window_pixels_when_scale_gt_1() {
        // NativeWindow / GPU / MotionEvent space — not layout×scale doubling.
        let window = (1080u32, 1920u32);
        let scale = 2.0_f32;
        let mut shell = AndroidShellStub::new();
        shell.resize(window.0, window.1, scale);
        assert_eq!(
            shell.geometry().physical_size,
            window,
            "geometry.physical_size must equal NativeWindow pixels"
        );

        let hit = shell.control_slot().expect("hit").rect;
        let paint = control_slot_paint_bounds(window, scale).expect("paint");
        assert_eq!(hit, paint, "hit must match painter window bounds");
        assert!(hit.x + hit.width <= window.0);
        assert!(hit.y + hit.height <= window.1);
        assert!(
            clamp_scissor(hit, window.0, window.1).is_some(),
            "slot must survive framebuffer clamp (regression: doubled y was off-screen)"
        );

        // Chrome present list must keep the slot band against the real frame.
        let draws = band_draw_list(&shell.chrome_present_bands(), window.0, window.1);
        assert!(
            draws.iter().any(|d| d.4 == CONTROL_SLOT_COLOR),
            "present_chrome_bands must not drop the slot via clamp_scissor"
        );
    }

    #[test]
    fn chrome_present_bands_include_slot_draw() {
        let mut shell = AndroidShellStub::new();
        shell.resize(1080, 1920, 2.0);
        let bands = shell.chrome_present_bands();
        assert!(bands.len() > shell.chrome_bands().len());
        let (fw, fh) = shell.geometry().physical_size;
        assert_eq!((fw, fh), (1080, 1920));
        let draws = band_draw_list(&bands, fw, fh);
        assert!(
            draws.iter().any(|d| d.4 == CONTROL_SLOT_COLOR),
            "present list must include control-slot fill"
        );
    }
}
