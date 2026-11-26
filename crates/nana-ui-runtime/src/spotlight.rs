//! Guided spotlight scrim (EvenOdd multi-cutout backdrop mask) and
//! auto-flipping anchored callout primitives.
//!
//! All colors, corner radii, border strokes, and shadows are resolved from the
//! active [`CompiledTheme`](nana_ui_core::CompiledTheme) via [`PaintContext`];
//! no hardcoded RGBA values or design dimensions are baked into the runtime.

use std::hash::{Hash, Hasher};

use nana_ui_core::{ElevationRole, RadiusTier, SemanticColorRole};

use crate::{FillRule, LayoutBox, PaintContext, PaintPath, Painter, StrokeStyle};

/// Geometric cutout shape for a [`SpotlightHole`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpotlightHoleShape {
    /// Sharp-cornered rectangle.
    Rect,
    /// Rounded rectangle with uniform corner radius in logical px.
    RoundedRect(f32),
    /// Capsule / stadium shape (`radius = min(width, height) * 0.5`).
    Pill,
    /// Inscribed ellipse / circle.
    Ellipse,
}

impl Default for SpotlightHoleShape {
    fn default() -> Self {
        Self::Rect
    }
}

/// One illuminated target region carved out of a [`SpotlightScrim`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpotlightHole {
    /// Target cutout bounds in the scrim's local coordinates (logical px).
    pub rect: LayoutBox,
    /// Cutout shape.
    pub shape: SpotlightHoleShape,
    /// Optional highlight ring width around the cutout (`None` disables the ring).
    pub ring_width: Option<f32>,
}

impl SpotlightHole {
    pub const fn rect(rect: LayoutBox) -> Self {
        Self {
            rect,
            shape: SpotlightHoleShape::Rect,
            ring_width: None,
        }
    }

    pub const fn rounded(rect: LayoutBox, radius: f32) -> Self {
        Self {
            rect,
            shape: SpotlightHoleShape::RoundedRect(radius),
            ring_width: None,
        }
    }

    pub const fn pill(rect: LayoutBox) -> Self {
        Self {
            rect,
            shape: SpotlightHoleShape::Pill,
            ring_width: None,
        }
    }

    pub const fn ellipse(rect: LayoutBox) -> Self {
        Self {
            rect,
            shape: SpotlightHoleShape::Ellipse,
            ring_width: None,
        }
    }

    /// Expand the hole uniformly on all four sides by `padding` logical px.
    pub fn inflate(mut self, padding: f32) -> Self {
        let p = if padding.is_finite() {
            padding.max(0.0)
        } else {
            0.0
        };
        self.rect = LayoutBox {
            x: self.rect.x - p,
            y: self.rect.y - p,
            width: (self.rect.width + p * 2.0).max(0.0),
            height: (self.rect.height + p * 2.0).max(0.0),
        };
        self
    }

    /// Add an accent highlight stroke around the cutout perimeter.
    pub fn ring_width(mut self, width: f32) -> Self {
        self.ring_width = (width.is_finite() && width > 0.0).then_some(width);
        self
    }

    fn append_to_path(&self, path: &mut PaintPath) {
        if self.rect.width <= 0.0 || self.rect.height <= 0.0 {
            return;
        }
        match self.shape {
            SpotlightHoleShape::Rect => {
                path.rect(self.rect);
            }
            SpotlightHoleShape::RoundedRect(radius) => {
                let r = if radius.is_finite() {
                    radius.max(0.0)
                } else {
                    0.0
                };
                path.rounded_rect(self.rect, [r; 4]);
            }
            SpotlightHoleShape::Pill => {
                let r = self.rect.width.min(self.rect.height) * 0.5;
                path.rounded_rect(self.rect, [r; 4]);
            }
            SpotlightHoleShape::Ellipse => {
                path.ellipse(self.rect);
            }
        }
    }

    fn hash_into(&self, hasher: &mut impl Hasher) {
        for v in [
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.ring_width.unwrap_or_default(),
        ] {
            v.to_bits().hash(hasher);
        }
        match self.shape {
            SpotlightHoleShape::Rect => 0u8.hash(hasher),
            SpotlightHoleShape::RoundedRect(r) => {
                1u8.hash(hasher);
                r.to_bits().hash(hasher);
            }
            SpotlightHoleShape::Pill => 2u8.hash(hasher),
            SpotlightHoleShape::Ellipse => 3u8.hash(hasher),
        }
    }
}

/// Full-viewport backdrop scrim [`Painter`] using [`SemanticColorRole::ModalScrim`]
/// and [`FillRule::EvenOdd`] to carve zero or more [`SpotlightHole`] cutouts.
#[derive(Debug, Clone, PartialEq)]
pub struct SpotlightScrim {
    pub holes: Vec<SpotlightHole>,
    /// Semantic color role for the backdrop mask (defaults to [`SemanticColorRole::ModalScrim`]).
    pub scrim_role: SemanticColorRole,
    /// Semantic color role for optional hole highlight rings (defaults to [`SemanticColorRole::Accent`]).
    pub ring_role: SemanticColorRole,
    /// Optional opacity multiplier (`0.0..=1.0`, `None` = full theme scrim alpha).
    pub opacity: Option<f32>,
    /// Whether pointer hits inside carved holes pass through to underlying UI.
    pub pass_through_holes: bool,
}

impl Default for SpotlightScrim {
    fn default() -> Self {
        Self {
            holes: Vec::new(),
            scrim_role: SemanticColorRole::ModalScrim,
            ring_role: SemanticColorRole::Accent,
            opacity: None,
            pass_through_holes: false,
        }
    }
}

impl SpotlightScrim {
    pub fn new(holes: impl Into<Vec<SpotlightHole>>) -> Self {
        Self {
            holes: holes.into(),
            ..Self::default()
        }
    }

    pub fn hole(mut self, hole: SpotlightHole) -> Self {
        self.holes.push(hole);
        self
    }

    pub const fn scrim_role(mut self, role: SemanticColorRole) -> Self {
        self.scrim_role = role;
        self
    }

    pub const fn ring_role(mut self, role: SemanticColorRole) -> Self {
        self.ring_role = role;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.is_finite().then_some(opacity.clamp(0.0, 1.0));
        self
    }

    pub const fn pass_through_holes(mut self, pass_through: bool) -> Self {
        self.pass_through_holes = pass_through;
        self
    }

    /// Build the EvenOdd mask path covering `[0, 0, width, height]` with every
    /// configured [`SpotlightHole`] punched out.
    pub fn build_scrim_path(&self, width: f32, height: f32) -> PaintPath {
        let mut path = PaintPath::new().with_fill_rule(FillRule::EvenOdd);
        if width <= 0.0 || height <= 0.0 {
            return path;
        }
        path.rect(LayoutBox {
            x: 0.0,
            y: 0.0,
            width,
            height,
        });
        for hole in &self.holes {
            hole.append_to_path(&mut path);
        }
        path
    }
}

impl Painter for SpotlightScrim {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        let [w, h] = cx.size();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let opacity = self.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        let mut color = cx.color(self.scrim_role);
        color[3] *= opacity;
        let scrim = self.build_scrim_path(w, h);
        cx.fill_path(&scrim, color);

        let mut ring_color = cx.color(self.ring_role);
        ring_color[3] *= opacity;
        for hole in &self.holes {
            if let Some(width) = hole.ring_width.filter(|w| w.is_finite() && *w > 0.0) {
                let mut ring = PaintPath::new();
                hole.append_to_path(&mut ring);
                cx.stroke_path(&ring, StrokeStyle::new(width), ring_color);
            }
        }
    }

    fn hit_test(&self, local: [f32; 2], size: [f32; 2]) -> Option<bool> {
        if !self.pass_through_holes {
            return None;
        }
        let scrim = self.build_scrim_path(size[0], size[1]);
        Some(scrim.contains(local))
    }

    fn paint_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.scrim_role.hash(&mut hasher);
        self.ring_role.hash(&mut hasher);
        self.opacity.unwrap_or(1.0).to_bits().hash(&mut hasher);
        self.pass_through_holes.hash(&mut hasher);
        self.holes.len().hash(&mut hasher);
        for hole in &self.holes {
            hole.hash_into(&mut hasher);
        }
        hasher.finish()
    }
}

/// Which side of the anchor rectangle an [`AnchoredCallout`] is placed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CalloutSide {
    #[default]
    Bottom,
    Top,
    Right,
    Left,
    /// Fallback when no anchor is present; centered inside the viewport.
    None,
}

impl CalloutSide {
    /// Opposite side used when auto-flipping away from a cramped viewport edge.
    pub const fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::None => Self::None,
        }
    }
}

/// Solved placement of an [`AnchoredCallout`] in viewport coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalloutPlacement {
    /// Final bounding box of the callout plate.
    pub bounds: LayoutBox,
    /// Chosen side relative to the anchor (may be flipped from preferred side).
    pub side: CalloutSide,
    /// Notch / pointer tip center along the facing edge of `bounds`, measured
    /// in local coordinates relative to `bounds` (`x` for Top/Bottom, `y` for
    /// Left/Right).
    pub arrow_offset: f32,
}

/// Auto-flipping anchored pointer callout solver.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnchoredCallout {
    /// Preferred side relative to the anchor.
    pub preferred_side: CalloutSide,
    /// Gap between the anchor edge and the callout plate (including notch depth).
    pub gap: f32,
    /// Minimum inset from each viewport edge in logical px.
    pub viewport_inset: f32,
    /// Minimum distance from callout corners to the pointer notch center.
    pub arrow_corner_inset: f32,
}

impl AnchoredCallout {
    pub fn new(preferred_side: CalloutSide) -> Self {
        Self {
            preferred_side,
            ..Self::default()
        }
    }

    pub const fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub const fn viewport_inset(mut self, inset: f32) -> Self {
        self.viewport_inset = inset;
        self
    }

    pub const fn arrow_corner_inset(mut self, inset: f32) -> Self {
        self.arrow_corner_inset = inset;
        self
    }

    /// Solve callout placement around `anchor` (or centered when `anchor` is
    /// `None`) for a callout plate of `callout_size = (w, h)` inside
    /// `viewport = (vw, vh)`.
    pub fn resolve(
        &self,
        anchor: Option<LayoutBox>,
        callout_size: (f32, f32),
        viewport: (f32, f32),
    ) -> CalloutPlacement {
        let cw = callout_size.0.max(0.0);
        let ch = callout_size.1.max(0.0);
        let vw = viewport.0.max(0.0);
        let vh = viewport.1.max(0.0);
        let inset = self.viewport_inset.max(0.0);
        let gap = self.gap.max(0.0);
        let max_x = (vw - inset - cw).max(inset);
        let max_y = (vh - inset - ch).max(inset);

        let Some(anchor) = anchor else {
            return CalloutPlacement {
                bounds: LayoutBox {
                    x: ((vw - cw) * 0.5).clamp(inset, max_x),
                    y: ((vh - ch) * 0.5).clamp(inset, max_y),
                    width: cw,
                    height: ch,
                },
                side: CalloutSide::None,
                arrow_offset: 0.0,
            };
        };

        let candidates = Self::side_order(self.preferred_side);
        let fits = |side: CalloutSide| match side {
            CalloutSide::Bottom => anchor.y + anchor.height + gap + ch <= vh - inset,
            CalloutSide::Top => anchor.y - gap - ch >= inset,
            CalloutSide::Right => anchor.x + anchor.width + gap + cw <= vw - inset,
            CalloutSide::Left => anchor.x - gap - cw >= inset,
            CalloutSide::None => true,
        };
        let available_space = |side: CalloutSide| match side {
            CalloutSide::Bottom => (vh - inset) - (anchor.y + anchor.height + gap),
            CalloutSide::Top => (anchor.y - gap) - inset,
            CalloutSide::Right => (vw - inset) - (anchor.x + anchor.width + gap),
            CalloutSide::Left => (anchor.x - gap) - inset,
            CalloutSide::None => 0.0,
        };

        let chosen = candidates
            .iter()
            .copied()
            .find(|&side| fits(side))
            .unwrap_or_else(|| {
                candidates
                    .iter()
                    .copied()
                    .max_by(|a, b| available_space(*a).total_cmp(&available_space(*b)))
                    .unwrap_or(self.preferred_side)
            });

        let anchor_cx = anchor.x + anchor.width * 0.5;
        let anchor_cy = anchor.y + anchor.height * 0.5;

        let (raw_x, raw_y) = match chosen {
            CalloutSide::Bottom => (anchor_cx - cw * 0.5, anchor.y + anchor.height + gap),
            CalloutSide::Top => (anchor_cx - cw * 0.5, anchor.y - gap - ch),
            CalloutSide::Right => (anchor.x + anchor.width + gap, anchor_cy - ch * 0.5),
            CalloutSide::Left => (anchor.x - gap - cw, anchor_cy - ch * 0.5),
            CalloutSide::None => ((vw - cw) * 0.5, (vh - ch) * 0.5),
        };

        let x = raw_x.clamp(inset, max_x);
        let y = raw_y.clamp(inset, max_y);
        let arrow_inset = self.arrow_corner_inset.max(0.0);
        let arrow_offset = match chosen {
            CalloutSide::Top | CalloutSide::Bottom => {
                let lo = arrow_inset.min(cw * 0.5);
                let hi = (cw - lo).max(lo);
                (anchor_cx - x).clamp(lo, hi)
            }
            CalloutSide::Left | CalloutSide::Right => {
                let lo = arrow_inset.min(ch * 0.5);
                let hi = (ch - lo).max(lo);
                (anchor_cy - y).clamp(lo, hi)
            }
            CalloutSide::None => 0.0,
        };

        CalloutPlacement {
            bounds: LayoutBox {
                x,
                y,
                width: cw,
                height: ch,
            },
            side: chosen,
            arrow_offset,
        }
    }

    fn side_order(preferred: CalloutSide) -> [CalloutSide; 4] {
        match preferred {
            CalloutSide::Bottom => [
                CalloutSide::Bottom,
                CalloutSide::Top,
                CalloutSide::Right,
                CalloutSide::Left,
            ],
            CalloutSide::Top => [
                CalloutSide::Top,
                CalloutSide::Bottom,
                CalloutSide::Right,
                CalloutSide::Left,
            ],
            CalloutSide::Right => [
                CalloutSide::Right,
                CalloutSide::Left,
                CalloutSide::Bottom,
                CalloutSide::Top,
            ],
            CalloutSide::Left => [
                CalloutSide::Left,
                CalloutSide::Right,
                CalloutSide::Bottom,
                CalloutSide::Top,
            ],
            CalloutSide::None => [
                CalloutSide::Bottom,
                CalloutSide::Top,
                CalloutSide::Right,
                CalloutSide::Left,
            ],
        }
    }
}

/// Themed [`Painter`] for an [`AnchoredCallout`] surface with a directional
/// pointer notch, elevation shadow, and hairline border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalloutPlatePainter {
    /// Side the callout sits on relative to its anchor (notch points toward anchor).
    pub side: CalloutSide,
    /// Notch center offset along the anchor-facing edge in local px.
    pub arrow_offset: f32,
    /// Half-width of the pointer notch base in logical px (`0.0` omits the notch).
    pub arrow_half_width: f32,
    /// Height/protrusion of the pointer notch in logical px.
    pub arrow_depth: f32,
    /// Corner radius tier resolved from the active theme.
    pub radius: RadiusTier,
    /// Surface fill semantic color role.
    pub surface_role: SemanticColorRole,
    /// Border stroke semantic color role.
    pub border_role: SemanticColorRole,
    /// Elevation role for the callout shadow.
    pub elevation: ElevationRole,
}

impl Default for CalloutPlatePainter {
    fn default() -> Self {
        Self {
            side: CalloutSide::None,
            arrow_offset: f32::default(),
            arrow_half_width: f32::default(),
            arrow_depth: f32::default(),
            radius: RadiusTier::Lg,
            surface_role: SemanticColorRole::Surface,
            border_role: SemanticColorRole::BorderSoft,
            elevation: ElevationRole::Overlay,
        }
    }
}

impl CalloutPlatePainter {
    pub fn from_placement(
        placement: CalloutPlacement,
        arrow_half_width: f32,
        arrow_depth: f32,
    ) -> Self {
        Self {
            side: placement.side,
            arrow_offset: placement.arrow_offset,
            arrow_half_width,
            arrow_depth,
            ..Self::default()
        }
    }

    fn build_plate_path(&self, width: f32, height: f32, radius: f32) -> PaintPath {
        let mut plate = PaintPath::new();
        let rect = LayoutBox {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        plate.rounded_rect(rect, [radius; 4]);
        let hw = self.arrow_half_width.max(0.0);
        let depth = self.arrow_depth.max(0.0);
        if hw <= 0.0 || depth <= 0.0 || self.side == CalloutSide::None {
            return plate;
        }
        let mut notch = PaintPath::new();
        match self.side {
            CalloutSide::Bottom => {
                // Anchor is above the callout; notch points upward from top edge.
                let cx = self.arrow_offset.clamp(hw, (width - hw).max(hw));
                notch
                    .move_to(cx - hw, 0.0)
                    .line_to(cx, -depth)
                    .line_to(cx + hw, 0.0)
                    .close();
            }
            CalloutSide::Top => {
                // Anchor is below the callout; notch points downward from bottom edge.
                let cx = self.arrow_offset.clamp(hw, (width - hw).max(hw));
                notch
                    .move_to(cx - hw, height)
                    .line_to(cx, height + depth)
                    .line_to(cx + hw, height)
                    .close();
            }
            CalloutSide::Right => {
                // Anchor is to the left; notch points left from left edge.
                let cy = self.arrow_offset.clamp(hw, (height - hw).max(hw));
                notch
                    .move_to(0.0, cy - hw)
                    .line_to(-depth, cy)
                    .line_to(0.0, cy + hw)
                    .close();
            }
            CalloutSide::Left => {
                // Anchor is to the right; notch points right from right edge.
                let cy = self.arrow_offset.clamp(hw, (height - hw).max(hw));
                notch
                    .move_to(width, cy - hw)
                    .line_to(width + depth, cy)
                    .line_to(width, cy + hw)
                    .close();
            }
            CalloutSide::None => {}
        }
        plate.boolean(&notch, crate::PathBooleanOp::Union)
    }
}

impl Painter for CalloutPlatePainter {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        let [w, h] = cx.size();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let radius = cx.radius(self.radius);
        let path = self.build_plate_path(w, h, radius);
        cx.shadow(&path, self.elevation);
        cx.fill_path(&path, self.surface_role);
        let hairline = cx.theme().border().hairline;
        if hairline > 0.0 {
            cx.stroke_path(&path, StrokeStyle::new(hairline), self.border_role);
        }
    }

    fn paint_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.side.hash(&mut hasher);
        self.arrow_offset.to_bits().hash(&mut hasher);
        self.arrow_half_width.to_bits().hash(&mut hasher);
        self.arrow_depth.to_bits().hash(&mut hasher);
        self.radius.hash(&mut hasher);
        self.surface_role.hash(&mut hasher);
        self.border_role.hash(&mut hasher);
        self.elevation.hash(&mut hasher);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotlight_scrim_carves_multiple_evenodd_holes() {
        let hole_a = SpotlightHole::rounded(
            LayoutBox {
                x: 40.0,
                y: 40.0,
                width: 80.0,
                height: 40.0,
            },
            8.0,
        );
        let hole_b = SpotlightHole::ellipse(LayoutBox {
            x: 200.0,
            y: 120.0,
            width: 60.0,
            height: 60.0,
        });
        let scrim = SpotlightScrim::new([hole_a, hole_b]).pass_through_holes(true);
        let path = scrim.build_scrim_path(400.0, 300.0);
        assert_eq!(path.fill_rule(), FillRule::EvenOdd);

        // Outside holes: covered by scrim.
        assert!(path.contains([10.0, 10.0]));
        assert_eq!(scrim.hit_test([10.0, 10.0], [400.0, 300.0]), Some(true));
        // Inside hole A and hole B: carved out (passes through).
        assert!(!path.contains([80.0, 60.0]));
        assert_eq!(scrim.hit_test([80.0, 60.0], [400.0, 300.0]), Some(false));
        assert!(!path.contains([230.0, 150.0]));
        assert_eq!(scrim.hit_test([230.0, 150.0], [400.0, 300.0]), Some(false));
    }

    #[test]
    fn anchored_callout_flips_away_from_tight_edges_and_computes_arrow_offset() {
        let solver = AnchoredCallout::new(CalloutSide::Bottom)
            .gap(12.0)
            .viewport_inset(10.0)
            .arrow_corner_inset(16.0);

        // Anchor near bottom of viewport: preferred Bottom does not fit -> flips to Top.
        let near_bottom = LayoutBox {
            x: 160.0,
            y: 250.0,
            width: 80.0,
            height: 30.0,
        };
        let placed = solver.resolve(Some(near_bottom), (200.0, 80.0), (400.0, 300.0));
        assert_eq!(placed.side, CalloutSide::Top);
        assert_eq!(placed.bounds.y, 250.0 - 12.0 - 80.0);
        assert_eq!(placed.bounds.x, 100.0);
        assert_eq!(placed.arrow_offset, 100.0);

        // Anchor near left edge: callout clamps to viewport_inset, arrow tracks anchor center.
        let near_left = LayoutBox {
            x: 12.0,
            y: 40.0,
            width: 24.0,
            height: 24.0,
        };
        let clamped = solver.resolve(Some(near_left), (200.0, 80.0), (400.0, 300.0));
        assert_eq!(clamped.side, CalloutSide::Bottom);
        assert_eq!(clamped.bounds.x, 10.0);
        assert_eq!(clamped.arrow_offset, 16.0);
    }
}
