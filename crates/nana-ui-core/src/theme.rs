use serde::{Deserialize, Serialize};

/// NanaUI's standard body and medium-control text size.
pub const UI_BASE_TEXT_SIZE: f32 = 13.0;

/// 1px rule. Not a [`space`] step.
pub const HAIRLINE: f32 = 1.0;

/// Spacing scale shared by every NanaUI component family.
///
/// A step of 2 logical px up to 16, then 20 and 24. The steps are not invented:
/// they are the distribution the component tree already uses — 8 and 6 dominate,
/// then 10, 4, 12, 14 and 24 — which a 4-based scale could not express without
/// moving existing geometry.
///
/// Use these instead of a bare literal for gaps between siblings and for
/// container padding. Control-internal metrics stay in [`ThemeMetrics`].
/// Off-grid leftovers (1, 3, 5, 7) map onto [`XXS`] / [`XS`] / [`SM`].
pub mod space {
    /// Hairline separation; adjacent rows in a dense list.
    pub const XXS: f32 = 2.0;
    /// Menu and popover inner padding; tight icon groups.
    pub const XS: f32 = 4.0;
    /// Icon-to-label inside one control.
    pub const SM: f32 = 6.0;
    /// The default gap between related controls in a row.
    pub const MD: f32 = 8.0;
    /// Row inner padding; looser inline groups.
    pub const LG: f32 = 10.0;
    /// Between grouped blocks inside a panel.
    pub const XL: f32 = 12.0;
    /// Panel vertical padding.
    pub const XXL: f32 = 14.0;
    /// Panel horizontal padding; between sections.
    pub const XXXL: f32 = 16.0;
    /// Page top inset.
    pub const PAGE_TIGHT: f32 = 20.0;
    /// Page horizontal inset; between top-level page sections.
    pub const PAGE: f32 = 24.0;
}

/// Product type scale: font size and weight steps shared by desktop shells.
///
/// [`BODY`] is [`UI_BASE_TEXT_SIZE`]. Card-title and metric display sizes
/// stay with the application when they are product-specific.
pub mod type_scale {
    use super::UI_BASE_TEXT_SIZE;

    /// Body line box (small/medium controls).
    pub const LINE: f32 = 16.0;
    /// Tall line box (large controls, card titles).
    pub const LINE_TALL: f32 = 18.0;
    /// Compact chrome caption (hints, section titles, toast copy).
    pub const HINT: f32 = 11.0;
    /// Caption / meta line (timestamps, counts, badges).
    pub const META: f32 = 12.0;
    /// Body copy. Same value as [`UI_BASE_TEXT_SIZE`].
    pub const BODY: f32 = UI_BASE_TEXT_SIZE;
    /// Section title inside a page or card.
    pub const SECTION: f32 = 14.0;
    /// In-page heading (profile name, dialog title).
    pub const HEADING: f32 = 16.0;
    /// Page title (settings).
    pub const TITLE: f32 = 18.0;
    /// Display / hero line.
    pub const DISPLAY: f32 = 20.0;

    /// Regular body weight.
    pub const REGULAR: u16 = 400;
    /// Medium emphasis (labels, card titles).
    pub const MEDIUM: u16 = 500;
    /// Semibold headings.
    pub const SEMIBOLD: u16 = 600;
    /// Bold chrome (sidebar section titles).
    pub const BOLD: u16 = 700;
}

/// Non-color design tokens shared by layout and interaction primitives.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ThemeMetrics {
    pub radius_xs: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub compact_control_height: f32,
    pub control_height: f32,
    /// Horizontal inset of a small control. Matches
    /// [`crate::ControlSize::padding_x_in`] for `Small`: one concept, one value.
    pub compact_control_padding_x: f32,
    pub control_padding_x: f32,
    pub selection_height: f32,
    pub icon_button_size: f32,
    pub panel_padding_x: f32,
    pub panel_padding_y: f32,
    /// Horizontal inset of a text field. Matches `control_padding_x` so a
    /// field and the `Select` beside it start their text on the same line.
    pub field_padding_x: f32,
    /// Horizontal inset of a list row. Matches the sidebar's own row padding:
    /// list rows and sidebar rows are one visual family.
    pub list_item_padding_x: f32,
    /// Horizontal inset of a large control. `serde(default)` so a metrics
    /// blob written before this field still loads as [`space::XXL`], which is
    /// the value the Large step used when it was a spacing constant.
    #[serde(default = "default_large_control_padding_x")]
    pub large_control_padding_x: f32,
    /// **Nothing reads this.** Every real duration is a `const` in
    /// [`crate::motion`] — `HOVER_COLOR` 120, `OVERLAY_FADE` 140,
    /// `MENU_OPACITY` 160, `MENU_POP` 180, `SIDEBAR_COLLAPSE` 260,
    /// `SPINNER_ROTATION` 900, `LOADING_SPIN` 800, `SKELETON_PULSE` 1400 —
    /// so changing this field changes nothing a user can see.
    ///
    /// Kept rather than deleted because the two are not equivalent choices:
    /// wiring them up means deciding which of those eight durations each token
    /// owns, and `SIDEBAR_COLLAPSE` (260) already disagrees with
    /// `motion_standard_ms` (240). That decision is Issue #100's Motion token
    /// layer, not a rename. Until then, treat this as reserved.
    /// See `docs/theme.md` §1.4 F2.
    pub motion_fast_ms: u16,
    /// Reserved, like [`Self::motion_fast_ms`]. Nothing reads it.
    pub motion_standard_ms: u16,
    /// Scrollbar chrome geometry.
    ///
    /// Composed rather than flattened: five scrollbar-shaped numbers do not
    /// belong beside `control_height`, but they do belong to the *installed*
    /// theme. Before this they lived only as the
    /// [`crate::SCROLLBAR_METRICS`] constant, which meant a density or theme
    /// change could not reach a scrollbar at all.
    ///
    /// `serde(default)` so a settings blob written before this field still
    /// loads; the default is that same constant.
    #[serde(default)]
    pub scrollbar: crate::ScrollbarMetrics,
}

/// Which radius step a control wants, rather than how many pixels that is.
///
/// This is the same shape as [`crate::SemanticColorRole`]: the component names
/// the design step, the theme decides the value. A component that writes
/// `UI_METRICS.radius_sm` into its style has already spent the token — the
/// number is baked at construction, and installing a theme afterwards can
/// never move it. Naming the tier keeps the decision open until the installed
/// [`ThemeMetrics`] is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RadiusTier {
    /// Hairline rounding: swatches, tags, inline marks.
    Xs,
    /// Controls: buttons, fields, chips, menu rows.
    Sm,
    /// Cards, panels and popovers.
    Md,
    /// Page-level surfaces and workspace corners.
    Lg,
}

impl RadiusTier {
    pub const fn resolve(self, metrics: ThemeMetrics) -> f32 {
        match self {
            Self::Xs => metrics.radius_xs,
            Self::Sm => metrics.radius_sm,
            Self::Md => metrics.radius_md,
            Self::Lg => metrics.radius_lg,
        }
    }
}

/// Which control-size step a node's box follows, and how.
///
/// `ControlSize` was already the intent — 58 of the runtime's `.height()`
/// calls go through it. What was missing is *when* it resolves:
/// `ControlSize::height()` resolves against the [`UI_METRICS`] constant at
/// construction time, so an installed theme could never move a control's
/// height. Naming the step on the node defers that to
/// [`ControlSize::height_in`] against the installed metrics.
///
/// The two variants are not decoration: some controls floor their height and
/// let content grow them, others are fixed. Collapsing them would change
/// layout for anything that overflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlHeight {
    /// Floor. The control is at least this tall; content may grow it.
    Min(crate::ControlSize),
    /// Fixed. The control is exactly this tall.
    Exact(crate::ControlSize),
}

impl ControlHeight {
    pub const fn size(self) -> crate::ControlSize {
        match self {
            Self::Min(size) | Self::Exact(size) => size,
        }
    }

    pub const fn resolve(self, metrics: ThemeMetrics) -> f32 {
        self.size().height_in(metrics)
    }
}

/// A control's horizontal inset, named as the design step rather than spent
/// as pixels at construction.
///
/// The counterpart to [`ControlHeight`] on the other axis, but **not** a
/// wrapper around [`crate::ControlSize`]: the insets a theme actually carries
/// do not line up one-to-one with the size steps. A text field and a list row
/// each have their own metrics field and are not "a medium control with
/// different padding", so naming them as steps here is what lets those two
/// stop reading [`UI_METRICS`] directly. Same shape as [`RadiusTier`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPadding {
    /// Small / compact controls.
    Compact,
    /// The default control inset.
    Standard,
    /// Large controls.
    Roomy,
    /// Text fields and areas.
    Field,
    /// List and menu rows.
    ListItem,
}

impl ControlPadding {
    pub const fn resolve(self, metrics: ThemeMetrics) -> f32 {
        match self {
            Self::Compact => metrics.small_control_padding_x(),
            Self::Standard => metrics.medium_control_padding_x(),
            Self::Roomy => metrics.large_control_padding_x(),
            Self::Field => metrics.field_padding_x,
            Self::ListItem => metrics.list_item_padding_x,
        }
    }
}

impl From<crate::ControlSize> for ControlPadding {
    fn from(size: crate::ControlSize) -> Self {
        match size {
            crate::ControlSize::Small => Self::Compact,
            crate::ControlSize::Medium => Self::Standard,
            crate::ControlSize::Large => Self::Roomy,
        }
    }
}

/// Panel / card inset, named as the design step rather than spent as pixels.
///
/// Not a [`ControlPadding`]: a card is not a large button, and the theme
/// carries a dedicated `panel_padding_*` pair. `Panel` writes both axes;
/// `PanelX` is the tab-strip case that only insets horizontally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfacePadding {
    /// Cards, settings groups, form surfaces.
    Panel,
    /// Horizontal panel inset only (pane tab strips).
    PanelX,
}

impl SurfacePadding {
    pub const fn resolve_x(self, metrics: ThemeMetrics) -> f32 {
        metrics.panel_padding_x
    }

    pub const fn resolve_y(self, metrics: ThemeMetrics) -> Option<f32> {
        match self {
            Self::Panel => Some(metrics.panel_padding_y),
            Self::PanelX => None,
        }
    }
}

/// A square box owned by the theme, rather than a spent `min_width`/`min_height`.
///
/// Icon buttons default to [`ThemeMetrics::icon_button_size`]. Calling
/// [`crate::ControlSize`] on them is a different step — a Small icon button is
/// not "the icon-button metric", it is the compact control height on both
/// axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SquareSize {
    IconButton,
    Control(crate::ControlSize),
}

impl SquareSize {
    pub const fn resolve(self, metrics: ThemeMetrics) -> f32 {
        match self {
            Self::IconButton => metrics.icon_button_size,
            Self::Control(size) => size.height_in(metrics),
        }
    }
}

fn default_large_control_padding_x() -> f32 {
    space::XXL
}

/// The four radius steps, already resolved against the installed theme.
///
/// This travels on the extracted node so the renderer never resolves a tier.
/// Framework chrome — menu surfaces, modal frames, palette rows, focus plates
/// — is painted by the Scene for visuals the node does not author a radius
/// for, and the Scene used to read the [`UI_METRICS`] constant to get one.
/// That made those corners the one part of the UI a theme could not reach.
///
/// Picking a step at paint time is still a design decision living in the
/// renderer; Issue #100 §3 moves it into Component Recipes. Carrying resolved
/// values is the part that has to be true first — the renderer consumes, it
/// does not resolve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromeRadii {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
}

impl From<ThemeMetrics> for ChromeRadii {
    fn from(metrics: ThemeMetrics) -> Self {
        Self {
            xs: metrics.radius_xs,
            sm: metrics.radius_sm,
            md: metrics.radius_md,
            lg: metrics.radius_lg,
        }
    }
}

impl Default for ChromeRadii {
    fn default() -> Self {
        Self::from(UI_METRICS)
    }
}

/// Shared Lilia-style geometry used by every NanaUI component family.
pub const UI_METRICS: ThemeMetrics = ThemeMetrics {
    radius_xs: space::XXS,
    radius_sm: space::SM,
    radius_md: space::LG,
    radius_lg: space::XXL,
    compact_control_height: 28.0,
    control_height: 32.0,
    compact_control_padding_x: space::MD,
    control_padding_x: space::LG,
    selection_height: 36.0,
    icon_button_size: 28.0,
    panel_padding_x: space::XXXL,
    panel_padding_y: space::XXL,
    field_padding_x: space::LG,
    list_item_padding_x: space::MD,
    large_control_padding_x: space::XXL,
    motion_fast_ms: 120,
    motion_standard_ms: 240,
    scrollbar: crate::scrollbar::SCROLLBAR_METRICS,
};

impl Default for ThemeMetrics {
    fn default() -> Self {
        UI_METRICS
    }
}

impl ThemeMetrics {
    /// Small single-line control height.
    pub const fn small_control_height(self) -> f32 {
        self.compact_control_height
    }

    /// Medium single-line control height.
    pub const fn medium_control_height(self) -> f32 {
        self.control_height
    }

    /// Large single-line control height.
    ///
    /// `selection_height` remains the serialized backing field for public
    /// compatibility, while components consume this semantic accessor.
    pub const fn large_control_height(self) -> f32 {
        self.selection_height
    }

    /// Small control horizontal inset.
    pub const fn small_control_padding_x(self) -> f32 {
        self.compact_control_padding_x
    }

    /// Medium control horizontal inset.
    pub const fn medium_control_padding_x(self) -> f32 {
        self.control_padding_x
    }

    /// Large control horizontal inset.
    pub const fn large_control_padding_x(self) -> f32 {
        self.large_control_padding_x
    }
}

/// The two application themes currently supported by the design system.
///
/// Part of the Style Model **Tokens** slice ([`crate::style_model`]).
/// Palette RGBA values live on [`crate::SemanticPalette`]; `nana-ui::theme::Colors`
/// is the paint adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    pub fn metrics(self) -> ThemeMetrics {
        let _ = self;
        UI_METRICS
    }

    pub const fn palette(self) -> crate::SemanticPalette {
        crate::SemanticPalette::for_mode(self)
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeMode;

    #[test]
    fn theme_mode_round_trips_for_host_persistence() {
        let encoded = serde_json::to_string(&ThemeMode::Light).expect("theme serializes");
        let restored: ThemeMode = serde_json::from_str(&encoded).expect("theme restores");
        assert_eq!(restored, ThemeMode::Light);
    }

    /// Issue #101 §1.4 F2: these two fields are reserved, and the audit says
    /// so because nothing reads them. If that stops being true, the audit is
    /// stale and this test is where it says so.
    #[test]
    fn the_motion_metrics_are_still_reserved_rather_than_wired() {
        let metrics = super::UI_METRICS;
        assert_eq!(metrics.motion_fast_ms, 120);
        assert_eq!(metrics.motion_standard_ms, 240);
        // The real durations live in `crate::motion` and do not agree with the
        // reserved tokens, which is precisely why wiring them is a decision
        // rather than a rename.
        assert_eq!(
            crate::motion::HOVER_COLOR,
            std::time::Duration::from_millis(u64::from(metrics.motion_fast_ms)),
        );
        assert_ne!(
            crate::motion::SIDEBAR_COLLAPSE,
            std::time::Duration::from_millis(u64::from(metrics.motion_standard_ms)),
        );
    }

    #[test]
    fn type_scale_body_matches_base_text_size() {
        assert_eq!(super::type_scale::BODY, super::UI_BASE_TEXT_SIZE);
        assert_eq!(super::type_scale::HINT, 11.0);
        assert_eq!(super::type_scale::META, 12.0);
        assert_eq!(super::type_scale::HEADING, 16.0);
        assert_eq!(super::type_scale::TITLE, 18.0);
        assert_eq!(super::type_scale::DISPLAY, 20.0);
        assert_eq!(super::type_scale::LINE, 16.0);
        assert_eq!(super::type_scale::LINE_TALL, 18.0);
        assert_eq!(super::type_scale::REGULAR, 400);
        assert_eq!(super::type_scale::SEMIBOLD, 600);
        assert_eq!(super::type_scale::BOLD, 700);
        assert_eq!(super::HAIRLINE, 1.0);
        assert_eq!(
            crate::ControlSize::Small.caption_size(),
            super::type_scale::HINT
        );
        assert_eq!(
            crate::ControlSize::Medium.caption_size(),
            super::type_scale::META
        );
        assert_eq!(
            crate::ControlSize::Large.caption_size(),
            super::type_scale::BODY
        );
    }

    #[test]
    fn a_metrics_blob_without_large_padding_still_loads_the_old_step() {
        let encoded = serde_json::to_string(&super::UI_METRICS).expect("metrics serializes");
        let mut value: serde_json::Value = serde_json::from_str(&encoded).expect("json");
        value
            .as_object_mut()
            .expect("object")
            .remove("large_control_padding_x");
        let restored: super::ThemeMetrics =
            serde_json::from_value(value).expect("legacy metrics restore");
        assert_eq!(restored.large_control_padding_x, super::space::XXL);
    }
}
