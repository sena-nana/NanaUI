//! Typed design-token categories owned by a [`ThemeDefinition`].
//!
//! [`ThemeDefinition`]: super::ThemeDefinition
//!
//! Every category here follows one shape, the one Issue #101 §1.5 proved out
//! on [`RadiusTier`](super::RadiusTier): **the component names a step, the
//! theme owns the value**. A category is a plain struct of values plus a role
//! enum that indexes it, so a lookup is a `match` on an enum rather than a
//! string hash — Issue #100 forbids `theme.get("radius.medium")` on the
//! runtime path.
//!
//! ## Why the bare `const`s below still exist
//!
//! [`space`](super::space), [`type_scale`](super::type_scale),
//! [`HAIRLINE`](super::HAIRLINE) and [`crate::motion`]'s durations are not a
//! second authority: each one is **derived from** the matching field of a
//! `DEFAULT` in this module. They are the default theme's values spelled as
//! constants for the call sites that cannot reach an installed theme yet.
//! Changing a number here changes both. Adding a number *there* is what would
//! start a second authority, and the audit gate in
//! `scripts/audit-theme-hardcoding.py` is what notices.

use std::time::Duration;

use crate::motion::Easing;
use crate::style_model::{SemanticColor, SemanticColorRole};

/// The accent family, stated once instead of six times.
///
/// This is the [foundation layer](super::FoundationTokens) Issue #100 §2 asks
/// for, in the one place the palette is genuinely derived rather than picked:
/// `accent_soft` / `accent_soft_hover` / `accent_soft_pressed` are the base
/// accent at three alphas, and they used to be three more literal RGBA
/// constants. That is why setting `palette.accent` on its own left the soft
/// fills on the previous hue — the derivation existed only in whoever chose
/// the constants.
///
/// The four colours are *not* derived from `base`: a readable
/// on-accent foreground is a design decision, not a formula, and light mode
/// proves it (`on_soft` is a darker blue than `base`, `text` is white).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccentRamp {
    /// The accent itself.
    pub base: SemanticColor,
    /// A heavier accent for pressed fills and keywords.
    pub strong: SemanticColor,
    /// Foreground that reads on top of a soft accent fill.
    pub on_soft: SemanticColor,
    /// Foreground that reads on top of a solid accent fill.
    pub text: SemanticColor,
    /// Alpha of the resting soft fill.
    pub soft_alpha: f32,
    /// Alpha of the hovered soft fill.
    pub soft_hover_alpha: f32,
    /// Alpha of the pressed soft fill.
    pub soft_pressed_alpha: f32,
    /// The fill a keyboard-focused control takes, alpha included.
    ///
    /// Written out rather than derived from [`Self::base`], because the two
    /// modes do not agree and the disagreement is arithmetic, not taste.
    /// WCAG 2.4.11 wants 3:1 between a control's focused and unfocused
    /// appearance, and a wash has to move *away* from the page: lighter than a
    /// dark one, darker than a light one. Over `#181818`, `base` at 0.50
    /// reaches 3.16:1. Over white, no alpha of `base` ever does — it tops out
    /// around 2:1 — so light nearly fills with `strong`, which needs
    /// [`SemanticPalette::focus_text`] to carry the label onto it.
    /// `focus_surface_clears_the_wcag_step_in_both_modes` holds both numbers.
    pub focus: SemanticColor,
}

impl AccentRamp {
    pub const DARK: Self = Self {
        base: SemanticColor::rgb8(123, 185, 240),
        strong: SemanticColor::rgb8(73, 145, 215),
        on_soft: SemanticColor::rgb8(123, 185, 240),
        text: SemanticColor::rgb8(13, 22, 34),
        soft_alpha: 0.14,
        soft_hover_alpha: 0.20,
        soft_pressed_alpha: 0.23,
        focus: SemanticColor::rgba8(123, 185, 240, 0.50),
    };

    pub const LIGHT: Self = Self {
        base: SemanticColor::rgb8(73, 145, 215),
        strong: SemanticColor::rgb8(44, 126, 214),
        on_soft: SemanticColor::rgb8(0, 85, 159),
        text: SemanticColor::rgba(1.0, 1.0, 1.0, 1.0),
        soft_alpha: 0.10,
        soft_hover_alpha: 0.20,
        soft_pressed_alpha: 0.23,
        focus: SemanticColor::rgba8(44, 126, 214, 0.90),
    };

    pub const fn soft(self) -> SemanticColor {
        SemanticColor {
            a: self.soft_alpha,
            ..self.base
        }
    }

    pub const fn soft_hover(self) -> SemanticColor {
        SemanticColor {
            a: self.soft_hover_alpha,
            ..self.base
        }
    }

    pub const fn soft_pressed(self) -> SemanticColor {
        SemanticColor {
            a: self.soft_pressed_alpha,
            ..self.base
        }
    }

    pub const fn for_mode(mode: super::ThemeMode) -> Self {
        match mode {
            super::ThemeMode::Dark => Self::DARK,
            super::ThemeMode::Light => Self::LIGHT,
        }
    }

    /// The ramp a palette already exhibits.
    ///
    /// Read, not repaired: a palette whose `accent_soft` is a different hue
    /// from `accent` produces a ramp that says so. This is how a definition
    /// built from loose palette fields keeps its foundation layer truthful
    /// instead of quietly claiming a ramp the palette does not have.
    pub const fn from_palette(palette: &crate::style_model::SemanticPalette) -> Self {
        Self {
            base: palette.accent,
            strong: palette.accent_strong,
            on_soft: palette.accent_on_soft,
            text: palette.accent_text,
            soft_alpha: palette.accent_soft.a,
            soft_hover_alpha: palette.accent_soft_hover.a,
            soft_pressed_alpha: palette.accent_soft_pressed.a,
            focus: palette.focus_surface,
        }
    }

    pub(super) const fn alphas(self) -> [f32; 3] {
        [
            self.soft_alpha,
            self.soft_hover_alpha,
            self.soft_pressed_alpha,
        ]
    }
}

/// Spacing scale: gaps between siblings and container padding.
///
/// Control-internal insets are [`ThemeMetrics`](super::ThemeMetrics) instead —
/// a field's horizontal inset is a control metric, not a gap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacingTokens {
    pub xxs: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
    pub xxxl: f32,
    pub page_tight: f32,
    pub page: f32,
}

/// Which spacing step a node wants, rather than how many pixels that is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpacingStep {
    Xxs,
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
    Xxl,
    Xxxl,
    PageTight,
    Page,
}

impl SpacingTokens {
    /// The scale the whole component tree is built on.
    ///
    /// A step of 2 logical px up to 16, then 20 and 24. The steps are not
    /// invented: they are the distribution the tree already used.
    pub const DEFAULT: Self = Self {
        xxs: 2.0,
        xs: 4.0,
        sm: 6.0,
        md: 8.0,
        lg: 10.0,
        xl: 12.0,
        xxl: 14.0,
        xxxl: 16.0,
        page_tight: 20.0,
        page: 24.0,
    };

    pub const fn resolve(self, step: SpacingStep) -> f32 {
        match step {
            SpacingStep::Xxs => self.xxs,
            SpacingStep::Xs => self.xs,
            SpacingStep::Sm => self.sm,
            SpacingStep::Md => self.md,
            SpacingStep::Lg => self.lg,
            SpacingStep::Xl => self.xl,
            SpacingStep::Xxl => self.xxl,
            SpacingStep::Xxxl => self.xxxl,
            SpacingStep::PageTight => self.page_tight,
            SpacingStep::Page => self.page,
        }
    }

    pub(super) const fn steps(self) -> [f32; 10] {
        [
            self.xxs,
            self.xs,
            self.sm,
            self.md,
            self.lg,
            self.xl,
            self.xxl,
            self.xxxl,
            self.page_tight,
            self.page,
        ]
    }
}

impl Default for SpacingTokens {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Font size steps. Weights live on [`TypographyTokens`] beside them because a
/// theme that moves the scale usually moves the weights with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeRole {
    /// Compact chrome caption (hints, section titles, toast copy).
    Hint,
    /// Caption / meta line (timestamps, counts, badges).
    Meta,
    /// Body copy.
    Body,
    /// Section title inside a page or card.
    Section,
    /// In-page heading (profile name, dialog title).
    Heading,
    /// Page title.
    Title,
    /// Display / hero line.
    Display,
}

/// Line box steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineRole {
    /// Body line box (small / medium controls).
    Line,
    /// Tall line box (large controls, card titles).
    Tall,
}

/// Weight steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextWeight {
    Regular,
    Medium,
    Semibold,
    Bold,
}

/// Typography tokens: the product type scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypographyTokens {
    pub hint: f32,
    pub meta: f32,
    pub body: f32,
    pub section: f32,
    pub heading: f32,
    pub title: f32,
    pub display: f32,
    pub line: f32,
    pub line_tall: f32,
    pub regular: u16,
    pub medium: u16,
    pub semibold: u16,
    pub bold: u16,
}

impl TypographyTokens {
    pub const DEFAULT: Self = Self {
        hint: 11.0,
        meta: 12.0,
        body: 13.0,
        section: 14.0,
        heading: 16.0,
        title: 18.0,
        display: 20.0,
        line: 16.0,
        line_tall: 18.0,
        regular: 400,
        medium: 500,
        semibold: 600,
        bold: 700,
    };

    pub const fn size(self, role: TypeRole) -> f32 {
        match role {
            TypeRole::Hint => self.hint,
            TypeRole::Meta => self.meta,
            TypeRole::Body => self.body,
            TypeRole::Section => self.section,
            TypeRole::Heading => self.heading,
            TypeRole::Title => self.title,
            TypeRole::Display => self.display,
        }
    }

    pub const fn line_height(self, role: LineRole) -> f32 {
        match role {
            LineRole::Line => self.line,
            LineRole::Tall => self.line_tall,
        }
    }

    pub const fn weight(self, weight: TextWeight) -> u16 {
        match weight {
            TextWeight::Regular => self.regular,
            TextWeight::Medium => self.medium,
            TextWeight::Semibold => self.semibold,
            TextWeight::Bold => self.bold,
        }
    }

    pub(super) const fn sizes(self) -> [f32; 9] {
        [
            self.hint,
            self.meta,
            self.body,
            self.section,
            self.heading,
            self.title,
            self.display,
            self.line,
            self.line_tall,
        ]
    }

    pub(super) const fn weights(self) -> [u16; 4] {
        [self.regular, self.medium, self.semibold, self.bold]
    }
}

impl Default for TypographyTokens {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Stroke widths.
///
/// One step today. A 1px rule is a **stroke**, never a spacing step, which is
/// why it is a category of its own rather than [`SpacingTokens::xxs`] wearing
/// two hats. Focus-ring stroke and outset are still literals inside
/// `nana-ui-scene`; see `docs/theme.md`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderTokens {
    /// 1px rule: separators, control outlines, table grid lines.
    pub hairline: f32,
}

/// Which stroke width a node wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BorderWidth {
    Hairline,
}

impl BorderTokens {
    pub const DEFAULT: Self = Self { hairline: 1.0 };

    pub const fn resolve(self, width: BorderWidth) -> f32 {
        match width {
            BorderWidth::Hairline => self.hairline,
        }
    }
}

impl Default for BorderTokens {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// State-layer alphas: how strongly a tinted overlay reads at rest, on hover
/// and while pressed.
///
/// These used to be literals inside `SemanticPalette::get`, where the light /
/// dark difference was decided by sniffing `background.r > 0.5`. A theme that
/// happened to have a mid-grey background got the wrong branch, and no theme
/// could move the numbers at all. They belong to the theme, so each mode's
/// definition states them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpacityTokens {
    pub warning_soft: f32,
    pub warning_soft_hover: f32,
    pub warning_soft_pressed: f32,
    pub danger_soft_hover: f32,
    pub danger_soft_pressed: f32,
}

/// Which state-layer alpha a derived role uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateLayer {
    WarningSoft,
    WarningSoftHover,
    WarningSoftPressed,
    DangerSoftHover,
    DangerSoftPressed,
}

impl OpacityTokens {
    /// Dark-mode state layers.
    pub const DARK: Self = Self {
        warning_soft: 0.16,
        warning_soft_hover: 0.20,
        warning_soft_pressed: 0.24,
        danger_soft_hover: 0.18,
        danger_soft_pressed: 0.22,
    };

    /// Light-mode state layers. Only `warning_soft` differs from [`Self::DARK`];
    /// that single difference is what the old brightness sniff encoded.
    pub const LIGHT: Self = Self {
        warning_soft: 0.12,
        ..Self::DARK
    };

    pub const fn resolve(self, layer: StateLayer) -> f32 {
        match layer {
            StateLayer::WarningSoft => self.warning_soft,
            StateLayer::WarningSoftHover => self.warning_soft_hover,
            StateLayer::WarningSoftPressed => self.warning_soft_pressed,
            StateLayer::DangerSoftHover => self.danger_soft_hover,
            StateLayer::DangerSoftPressed => self.danger_soft_pressed,
        }
    }

    pub const fn for_mode(mode: super::ThemeMode) -> Self {
        match mode {
            super::ThemeMode::Dark => Self::DARK,
            super::ThemeMode::Light => Self::LIGHT,
        }
    }

    pub(super) const fn alphas(self) -> [f32; 5] {
        [
            self.warning_soft,
            self.warning_soft_hover,
            self.warning_soft_pressed,
            self.danger_soft_hover,
            self.danger_soft_pressed,
        ]
    }
}

impl Default for OpacityTokens {
    fn default() -> Self {
        Self::DARK
    }
}

/// Which named transition a surface plays.
///
/// Issue #101 §1.4 F2 recorded `ThemeMetrics::motion_fast_ms` /
/// `motion_standard_ms` as dead fields: nothing read them, and the eight real
/// durations were `const`s a theme could not move. Wiring them was never a
/// rename — it was deciding which duration each token owns. This enum is that
/// decision: the roles *are* the eight durations, named by what they animate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MotionRole {
    /// Hover / pressed colour cross-fade on a control.
    HoverColor,
    /// Overlay fade-in / fade-out; switch thumb travel.
    OverlayFade,
    /// Menu opacity transition.
    MenuOpacity,
    /// Menu pop-in scale / translate.
    MenuPop,
    /// Sidebar and workspace region collapse / expand.
    SidebarCollapse,
    /// Skeleton pulse cycle.
    SkeletonPulse,
    /// One full turn of an indeterminate busy indicator.
    SpinnerRotation,
    /// Button / switch / card loading indicator cycle.
    LoadingSpin,
}

/// Which easing a transition follows.
///
/// Three roles because the tree animates on three curves, not because three
/// is a nice number: `EaseOutCubic` for anything settling into a new state,
/// `EaseInOutCubic` for the symmetric open/close of a collapsing region, and
/// the LiliaUI `cubic-bezier(0.2, 0.8, 0.2, 1)` for a menu popping in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EasingRole {
    /// State transitions that settle into place: hover colour, overlay fade.
    Standard,
    /// Symmetric open / close: sidebar and workspace region collapse.
    Expand,
    /// Entrances that should read as deliberate: menu pop-in.
    Emphasized,
}

/// Motion tokens: duration scale plus easing policy.
///
/// Durations are milliseconds so the struct stays `Copy` and serialisable;
/// [`Self::duration`] hands back a [`Duration`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionTokens {
    pub hover_color_ms: u16,
    pub overlay_fade_ms: u16,
    pub menu_opacity_ms: u16,
    pub menu_pop_ms: u16,
    pub sidebar_collapse_ms: u16,
    pub skeleton_pulse_ms: u16,
    pub spinner_rotation_ms: u16,
    pub loading_spin_ms: u16,
    pub standard_easing: Easing,
    pub expand_easing: Easing,
    pub emphasized_easing: Easing,
}

impl MotionTokens {
    /// The LiliaUI motion spec the component tree already animates on.
    pub const DEFAULT: Self = Self {
        hover_color_ms: 120,
        overlay_fade_ms: 140,
        menu_opacity_ms: 160,
        menu_pop_ms: 180,
        sidebar_collapse_ms: 260,
        skeleton_pulse_ms: 1400,
        spinner_rotation_ms: 900,
        loading_spin_ms: 800,
        standard_easing: Easing::EaseOutCubic,
        expand_easing: Easing::EaseInOutCubic,
        emphasized_easing: Easing::MENU_POP,
    };

    pub const fn millis(self, role: MotionRole) -> u16 {
        match role {
            MotionRole::HoverColor => self.hover_color_ms,
            MotionRole::OverlayFade => self.overlay_fade_ms,
            MotionRole::MenuOpacity => self.menu_opacity_ms,
            MotionRole::MenuPop => self.menu_pop_ms,
            MotionRole::SidebarCollapse => self.sidebar_collapse_ms,
            MotionRole::SkeletonPulse => self.skeleton_pulse_ms,
            MotionRole::SpinnerRotation => self.spinner_rotation_ms,
            MotionRole::LoadingSpin => self.loading_spin_ms,
        }
    }

    pub const fn duration(self, role: MotionRole) -> Duration {
        Duration::from_millis(self.millis(role) as u64)
    }

    pub const fn easing(self, role: EasingRole) -> Easing {
        match role {
            EasingRole::Standard => self.standard_easing,
            EasingRole::Expand => self.expand_easing,
            EasingRole::Emphasized => self.emphasized_easing,
        }
    }

    pub(super) const fn durations(self) -> [u16; 8] {
        [
            self.hover_color_ms,
            self.overlay_fade_ms,
            self.menu_opacity_ms,
            self.menu_pop_ms,
            self.sidebar_collapse_ms,
            self.skeleton_pulse_ms,
            self.spinner_rotation_ms,
            self.loading_spin_ms,
        ]
    }
}

impl Default for MotionTokens {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Which elevation step a surface sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElevationRole {
    /// Menus, popovers, hover cards — a surface lifted off the page.
    Surface,
    /// Modal frames — a surface lifted off everything, over a scrim.
    Overlay,
}

/// One drop shadow, backend-neutral. Mirrors CSS `box-shadow`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowToken {
    pub color: SemanticColor,
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub inset: bool,
}

/// Effect tokens: the elevation ramp.
///
/// The two shadows used to live as a `match theme_mode` inside
/// `ComponentElevation::surface_shadow` and as a `background.r > 0.5`
/// brightness sniff inside the modal-frame geometry. Both were the theme
/// deciding something in a place the theme could not reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectTokens {
    pub surface: ShadowToken,
    pub overlay: ShadowToken,
}

impl EffectTokens {
    /// Lilia `--shadow-surface` dark: `0 10px 30px -24px rgba(0,0,0,.62)`.
    pub const DARK: Self = Self {
        surface: ShadowToken {
            color: SemanticColor::rgba(0.0, 0.0, 0.0, 0.62),
            offset_x: 0.0,
            offset_y: 10.0,
            blur_radius: 30.0,
            spread_radius: -24.0,
            inset: false,
        },
        overlay: ShadowToken {
            color: SemanticColor::rgba(0.0, 0.0, 0.0, 0.45),
            offset_x: 0.0,
            offset_y: 14.0,
            blur_radius: 30.0,
            spread_radius: 0.0,
            inset: false,
        },
    };

    /// Lilia `--shadow-surface` light: `0 10px 26px -24px rgba(17,24,39,.24)`.
    pub const LIGHT: Self = Self {
        surface: ShadowToken {
            color: SemanticColor::rgba8(17, 24, 39, 0.24),
            offset_x: 0.0,
            offset_y: 10.0,
            blur_radius: 26.0,
            spread_radius: -24.0,
            inset: false,
        },
        overlay: ShadowToken {
            color: SemanticColor::rgba(0.0, 0.0, 0.0, 0.28),
            offset_x: 0.0,
            offset_y: 14.0,
            blur_radius: 30.0,
            spread_radius: 0.0,
            inset: false,
        },
    };

    pub const fn shadow(self, role: ElevationRole) -> ShadowToken {
        match role {
            ElevationRole::Surface => self.surface,
            ElevationRole::Overlay => self.overlay,
        }
    }

    pub const fn for_mode(mode: super::ThemeMode) -> Self {
        match mode {
            super::ThemeMode::Dark => Self::DARK,
            super::ThemeMode::Light => Self::LIGHT,
        }
    }
}

impl Default for EffectTokens {
    fn default() -> Self {
        Self::DARK
    }
}

/// A window surface named by what it is, not by what the platform paints it
/// with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfaceRole {
    /// Primary content area.
    Window,
    /// Sidebar / chrome strip.
    Chrome,
}

/// Whether a surface is allowed to let the desktop through.
///
/// Semantic only. The theme says "this surface may be translucent"; whether it
/// *is* translucent is [`crate::WindowMaterialMode`] policy, and the blur
/// itself is the platform's. No platform object is held here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfaceMaterial {
    Opaque,
    Translucent,
}

/// One surface's contract: which palette role paints it, and whether a
/// backdrop is allowed to thin it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceSpec {
    pub paint: SemanticColorRole,
    pub material: SurfaceMaterial,
}

/// Surface / material roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceTokens {
    pub window: SurfaceSpec,
    pub chrome: SurfaceSpec,
}

impl SurfaceTokens {
    pub const DEFAULT: Self = Self {
        window: SurfaceSpec {
            paint: SemanticColorRole::Background,
            material: SurfaceMaterial::Translucent,
        },
        chrome: SurfaceSpec {
            paint: SemanticColorRole::Surface,
            material: SurfaceMaterial::Translucent,
        },
    };

    pub const fn spec(self, role: SurfaceRole) -> SurfaceSpec {
        match role {
            SurfaceRole::Window => self.window,
            SurfaceRole::Chrome => self.chrome,
        }
    }
}

impl Default for SurfaceTokens {
    fn default() -> Self {
        Self::DEFAULT
    }
}
