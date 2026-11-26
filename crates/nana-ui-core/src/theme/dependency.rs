//! Theme dependency classification, category generations, and theme diffs (#106, #104).

use serde::{Deserialize, Serialize};

use super::{CompiledTheme, ThemeMetrics};
use crate::style_model::{SemanticColorMix, SemanticColorRole, SemanticPalette};

/// Stable identity of a [`ThemeScope`](crate::theme::ThemePatch) node in `UiWorld`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct ThemeScopeId(pub u32);

impl ThemeScopeId {
    /// Document root scope (no subtree patch).
    pub const ROOT: Self = Self(0);

    pub const fn is_root(self) -> bool {
        self.0 == 0
    }
}

/// Monotonic generation of a resolved [`ThemeScopeId`] overlay.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct ScopeGeneration(pub u32);

impl ScopeGeneration {
    pub const INITIAL: Self = Self(1);

    pub const fn next(self) -> Self {
        let next = self.0.wrapping_add(1);
        if next == 0 { Self(1) } else { Self(next) }
    }
}

/// Which component recipe families can be invalidated independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(usize)]
pub enum ThemeRecipeFamily {
    Button = 0,
    Input = 1,
    Select = 2,
    Tabs = 3,
    TreeView = 4,
    Menu = 5,
    Dialog = 6,
    Scrollbar = 7,
    Toast = 8,
    Shape = 9,
    Status = 10,
    Family = 11,
}

impl ThemeRecipeFamily {
    pub const COUNT: usize = 12;

    pub const ALL: [Self; Self::COUNT] = [
        Self::Button,
        Self::Input,
        Self::Select,
        Self::Tabs,
        Self::TreeView,
        Self::Menu,
        Self::Dialog,
        Self::Scrollbar,
        Self::Toast,
        Self::Shape,
        Self::Status,
        Self::Family,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn to_mask(self) -> ThemeDependencyMask {
        match self {
            Self::Button => ThemeDependencyMask::RECIPE_BUTTON,
            Self::Input => ThemeDependencyMask::RECIPE_INPUT,
            Self::Select => ThemeDependencyMask::RECIPE_SELECT,
            Self::Tabs => ThemeDependencyMask::RECIPE_TABS,
            Self::TreeView => ThemeDependencyMask::RECIPE_TREE_VIEW,
            Self::Menu => ThemeDependencyMask::RECIPE_MENU,
            Self::Dialog => ThemeDependencyMask::RECIPE_DIALOG,
            Self::Scrollbar => ThemeDependencyMask::RECIPE_SCROLLBAR,
            Self::Toast => ThemeDependencyMask::RECIPE_TOAST,
            Self::Shape => ThemeDependencyMask::RECIPE_SHAPE,
            Self::Status => ThemeDependencyMask::RECIPE_STATUS,
            Self::Family => ThemeDependencyMask::RECIPE_FAMILY,
        }
    }
}

/// Bitmask describing which theme token/recipe categories a node or change touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ThemeDependencyMask(pub u32);

impl ThemeDependencyMask {
    pub const NONE: Self = Self(0);
    pub const PALETTE: Self = Self(1 << 0);
    pub const ACCENT: Self = Self(1 << 1);
    pub const METRICS: Self = Self(1 << 2);
    pub const TYPOGRAPHY_SHAPE: Self = Self(1 << 3);
    pub const TYPOGRAPHY_LAYOUT: Self = Self(1 << 4);
    pub const EFFECT: Self = Self(1 << 5);
    pub const MOTION_POLICY: Self = Self(1 << 6);
    pub const RECIPE_BUTTON: Self = Self(1 << 7);
    pub const RECIPE_INPUT: Self = Self(1 << 8);
    pub const RECIPE_SELECT: Self = Self(1 << 9);
    pub const RECIPE_TABS: Self = Self(1 << 10);
    pub const RECIPE_TREE_VIEW: Self = Self(1 << 11);
    pub const RECIPE_MENU: Self = Self(1 << 12);
    pub const RECIPE_DIALOG: Self = Self(1 << 13);
    pub const RECIPE_SCROLLBAR: Self = Self(1 << 14);
    pub const RECIPE_TOAST: Self = Self(1 << 15);
    pub const RECIPE_SHAPE: Self = Self(1 << 16);
    pub const RECIPE_STATUS: Self = Self(1 << 17);
    pub const RECIPE_FAMILY: Self = Self(1 << 18);
    pub const CUSTOM_PAINT: Self = Self(1 << 19);
    pub const CHART: Self = Self(1 << 20);
    pub const INHERITED_TYPOGRAPHY: Self = Self(1 << 21);

    pub const COLOR_ANY: Self = Self(Self::PALETTE.0 | Self::ACCENT.0);
    pub const TYPOGRAPHY_ANY: Self = Self(Self::TYPOGRAPHY_SHAPE.0 | Self::TYPOGRAPHY_LAYOUT.0);
    pub const RECIPE_ANY: Self = Self(
        Self::RECIPE_BUTTON.0
            | Self::RECIPE_INPUT.0
            | Self::RECIPE_SELECT.0
            | Self::RECIPE_TABS.0
            | Self::RECIPE_TREE_VIEW.0
            | Self::RECIPE_MENU.0
            | Self::RECIPE_DIALOG.0
            | Self::RECIPE_SCROLLBAR.0
            | Self::RECIPE_TOAST.0
            | Self::RECIPE_SHAPE.0
            | Self::RECIPE_STATUS.0
            | Self::RECIPE_FAMILY.0,
    );
    pub const VISUAL_ANY: Self = Self(
        Self::COLOR_ANY.0
            | Self::METRICS.0
            | Self::TYPOGRAPHY_ANY.0
            | Self::EFFECT.0
            | Self::RECIPE_ANY.0
            | Self::CUSTOM_PAINT.0
            | Self::CHART.0
            | Self::INHERITED_TYPOGRAPHY.0,
    );
    pub const ALL: Self = Self(Self::VISUAL_ANY.0 | Self::MOTION_POLICY.0);

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Dependency class for a semantic colour role.
    ///
    /// Accent, focus, and highlight roles follow [`AccentRamp`] changes, while
    /// background/surface/text/status roles stay in [`Self::PALETTE`].
    pub const fn for_color_role(role: SemanticColorRole) -> Self {
        match role {
            SemanticColorRole::Accent
            | SemanticColorRole::AccentStrong
            | SemanticColorRole::AccentSoft
            | SemanticColorRole::AccentSoftHover
            | SemanticColorRole::AccentSoftPressed
            | SemanticColorRole::AccentOnSoft
            | SemanticColorRole::AccentText
            | SemanticColorRole::FocusSurface
            | SemanticColorRole::FocusBorder
            | SemanticColorRole::FocusText
            | SemanticColorRole::Highlight
            | SemanticColorRole::HighlightHover
            | SemanticColorRole::HighlightPressed
            | SemanticColorRole::HighlightText => Self::ACCENT,
            _ => Self::PALETTE,
        }
    }

    /// Dependency class for a semantic colour mix.
    pub const fn for_color_mix(mix: SemanticColorMix) -> Self {
        let (first, second, _) = mix.parts();
        let first_mask = Self::for_color_role(first);
        match second {
            Some(role) => first_mask.union(Self::for_color_role(role)),
            None => first_mask,
        }
    }

    /// Human-readable class names present in this mask, in stable order.
    pub fn names(self) -> Vec<&'static str> {
        const LABELS: [(ThemeDependencyMask, &str); 23] = [
            (ThemeDependencyMask::PALETTE, "PALETTE"),
            (ThemeDependencyMask::ACCENT, "ACCENT"),
            (ThemeDependencyMask::METRICS, "METRICS"),
            (ThemeDependencyMask::TYPOGRAPHY_SHAPE, "TYPOGRAPHY_SHAPE"),
            (ThemeDependencyMask::TYPOGRAPHY_LAYOUT, "TYPOGRAPHY_LAYOUT"),
            (ThemeDependencyMask::EFFECT, "EFFECT"),
            (ThemeDependencyMask::MOTION_POLICY, "MOTION_POLICY"),
            (ThemeDependencyMask::RECIPE_BUTTON, "RECIPE_BUTTON"),
            (ThemeDependencyMask::RECIPE_INPUT, "RECIPE_INPUT"),
            (ThemeDependencyMask::RECIPE_SELECT, "RECIPE_SELECT"),
            (ThemeDependencyMask::RECIPE_TABS, "RECIPE_TABS"),
            (ThemeDependencyMask::RECIPE_TREE_VIEW, "RECIPE_TREE_VIEW"),
            (ThemeDependencyMask::RECIPE_MENU, "RECIPE_MENU"),
            (ThemeDependencyMask::RECIPE_DIALOG, "RECIPE_DIALOG"),
            (ThemeDependencyMask::RECIPE_SCROLLBAR, "RECIPE_SCROLLBAR"),
            (ThemeDependencyMask::RECIPE_TOAST, "RECIPE_TOAST"),
            (ThemeDependencyMask::RECIPE_SHAPE, "RECIPE_SHAPE"),
            (ThemeDependencyMask::RECIPE_STATUS, "RECIPE_STATUS"),
            (ThemeDependencyMask::RECIPE_FAMILY, "RECIPE_FAMILY"),
            (ThemeDependencyMask::CUSTOM_PAINT, "CUSTOM_PAINT"),
            (ThemeDependencyMask::CHART, "CHART"),
            (
                ThemeDependencyMask::INHERITED_TYPOGRAPHY,
                "INHERITED_TYPOGRAPHY",
            ),
            (ThemeDependencyMask::NONE, "NONE"),
        ];
        if self.is_empty() {
            return Vec::new();
        }
        LABELS
            .iter()
            .filter(|(bit, _)| !bit.is_empty() && self.intersects(*bit))
            .map(|(_, name)| *name)
            .collect()
    }
}

/// Per-category revision counters replacing a single global invalidation epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThemeCategoryGenerations {
    pub palette: u32,
    pub accent: u32,
    pub metrics: u32,
    pub typography_shape: u32,
    pub typography_layout: u32,
    pub effect: u32,
    pub motion: u32,
    pub recipes: [u32; ThemeRecipeFamily::COUNT],
}

impl ThemeCategoryGenerations {
    pub const INITIAL: Self = Self {
        palette: 1,
        accent: 1,
        metrics: 1,
        typography_shape: 1,
        typography_layout: 1,
        effect: 1,
        motion: 1,
        recipes: [1; ThemeRecipeFamily::COUNT],
    };

    pub fn advance(&mut self, changed: ThemeDependencyMask) {
        let bump = |v: &mut u32| {
            let next = v.wrapping_add(1);
            *v = if next == 0 { 1 } else { next };
        };
        if changed.intersects(ThemeDependencyMask::PALETTE) {
            bump(&mut self.palette);
        }
        if changed.intersects(ThemeDependencyMask::ACCENT) {
            bump(&mut self.accent);
        }
        if changed.intersects(ThemeDependencyMask::METRICS) {
            bump(&mut self.metrics);
        }
        if changed.intersects(ThemeDependencyMask::TYPOGRAPHY_SHAPE) {
            bump(&mut self.typography_shape);
        }
        if changed.intersects(ThemeDependencyMask::TYPOGRAPHY_LAYOUT) {
            bump(&mut self.typography_layout);
        }
        if changed.intersects(ThemeDependencyMask::EFFECT) {
            bump(&mut self.effect);
        }
        if changed.intersects(ThemeDependencyMask::MOTION_POLICY) {
            bump(&mut self.motion);
        }
        for family in ThemeRecipeFamily::ALL {
            if changed.intersects(family.to_mask()) {
                bump(&mut self.recipes[family.index()]);
            }
        }
    }

    pub const fn recipe(self, family: ThemeRecipeFamily) -> u32 {
        self.recipes[family.index()]
    }

    /// Compute which dependency classes actually changed between `old` and `new`.
    pub fn diff_themes(old: &CompiledTheme, new: &CompiledTheme) -> ThemeDependencyMask {
        let mut mask = ThemeDependencyMask::NONE;

        let old_sm = old.style_model();
        let new_sm = new.style_model();
        let (non_accent_changed, accent_changed) = diff_palettes(&old_sm.palette, &new_sm.palette);
        if non_accent_changed
            || old.appearance() != new.appearance()
            || old_sm.theme_appearance != new_sm.theme_appearance
            || old_sm.titlebar != new_sm.titlebar
            || old_sm.on_media != new_sm.on_media
            || old_sm.modal_scrim != new_sm.modal_scrim
            || old_sm.opacity.warning_soft != new_sm.opacity.warning_soft
            || old_sm.opacity.warning_soft_hover != new_sm.opacity.warning_soft_hover
            || old_sm.opacity.warning_soft_pressed != new_sm.opacity.warning_soft_pressed
            || old_sm.opacity.danger_soft_hover != new_sm.opacity.danger_soft_hover
            || old_sm.opacity.danger_soft_pressed != new_sm.opacity.danger_soft_pressed
        {
            mask = mask.union(ThemeDependencyMask::PALETTE);
        }
        if accent_changed {
            mask = mask.union(ThemeDependencyMask::ACCENT);
        }

        let old_metrics: ThemeMetrics = old.metrics();
        let new_metrics: ThemeMetrics = new.metrics();
        if !old_metrics.same_layout(&new_metrics)
            || old.spacing() != new.spacing()
            || old.border() != new.border()
        {
            mask = mask.union(ThemeDependencyMask::METRICS);
        }

        let old_type = old.typography();
        let new_type = new.typography();
        if old_type.regular != new_type.regular
            || old_type.medium != new_type.medium
            || old_type.semibold != new_type.semibold
            || old_type.bold != new_type.bold
        {
            mask = mask.union(ThemeDependencyMask::TYPOGRAPHY_SHAPE);
        }
        if old_type.hint != new_type.hint
            || old_type.meta != new_type.meta
            || old_type.body != new_type.body
            || old_type.section != new_type.section
            || old_type.card_title != new_type.card_title
            || old_type.heading != new_type.heading
            || old_type.title != new_type.title
            || old_type.display != new_type.display
            || old_type.line != new_type.line
            || old_type.line_tall != new_type.line_tall
        {
            mask = mask.union(ThemeDependencyMask::TYPOGRAPHY_LAYOUT);
        }

        if old.effects() != new.effects()
            || old.surfaces() != new.surfaces()
            || old_metrics.corner_shape != new_metrics.corner_shape
            || old_metrics.focus_ring != new_metrics.focus_ring
            || old_sm.opacity.secondary_text != new_sm.opacity.secondary_text
            || old_sm.opacity.disabled != new_sm.opacity.disabled
            || old_sm.opacity.notice_tint != new_sm.opacity.notice_tint
        {
            mask = mask.union(ThemeDependencyMask::EFFECT);
        }

        if old.motion() != new.motion() {
            mask = mask.union(ThemeDependencyMask::MOTION_POLICY);
        }

        let old_r = old.recipes();
        let new_r = new.recipes();
        if old_r.button() != new_r.button() {
            mask = mask.union(ThemeDependencyMask::RECIPE_BUTTON);
        }
        if old_r.input() != new_r.input() {
            mask = mask.union(ThemeDependencyMask::RECIPE_INPUT);
        }
        if old_r.select() != new_r.select() {
            mask = mask.union(ThemeDependencyMask::RECIPE_SELECT);
        }
        if old_r.tabs() != new_r.tabs() {
            mask = mask.union(ThemeDependencyMask::RECIPE_TABS);
        }
        if old_r.tree_view() != new_r.tree_view() {
            mask = mask.union(ThemeDependencyMask::RECIPE_TREE_VIEW);
        }
        if old_r.menu() != new_r.menu() {
            mask = mask.union(ThemeDependencyMask::RECIPE_MENU);
        }
        if old_r.dialog() != new_r.dialog() {
            mask = mask.union(ThemeDependencyMask::RECIPE_DIALOG);
        }
        if old_r.scrollbar() != new_r.scrollbar() || old_metrics.scrollbar != new_metrics.scrollbar
        {
            mask = mask.union(ThemeDependencyMask::RECIPE_SCROLLBAR);
        }
        if old_r.toast() != new_r.toast() {
            mask = mask.union(ThemeDependencyMask::RECIPE_TOAST);
        }
        if old_r.shape() != new_r.shape() {
            mask = mask.union(ThemeDependencyMask::RECIPE_SHAPE);
        }
        if old_r.status() != new_r.status() {
            mask = mask.union(ThemeDependencyMask::RECIPE_STATUS);
        }
        if old_r.families_slice() != new_r.families_slice() {
            mask = mask.union(ThemeDependencyMask::RECIPE_FAMILY);
        }

        mask
    }
}

impl Default for ThemeCategoryGenerations {
    fn default() -> Self {
        Self::INITIAL
    }
}

fn diff_palettes(old: &SemanticPalette, new: &SemanticPalette) -> (bool, bool) {
    let non_accent = old.background != new.background
        || old.surface != new.surface
        || old.subtle != new.subtle
        || old.raised != new.raised
        || old.header != new.header
        || old.hover != new.hover
        || old.active != new.active
        || old.selected != new.selected
        || old.selected_hover != new.selected_hover
        || old.selected_pressed != new.selected_pressed
        || old.border != new.border
        || old.border_soft != new.border_soft
        || old.border_strong != new.border_strong
        || old.text != new.text
        || old.muted != new.muted
        || old.faint != new.faint
        || old.success != new.success
        || old.warning != new.warning
        || old.danger != new.danger;
    let accent = old.accent != new.accent
        || old.accent_strong != new.accent_strong
        || old.accent_soft != new.accent_soft
        || old.accent_soft_hover != new.accent_soft_hover
        || old.accent_soft_pressed != new.accent_soft_pressed
        || old.accent_on_soft != new.accent_on_soft
        || old.accent_text != new.accent_text
        || old.focus_surface != new.focus_surface
        || old.focus_border != new.focus_border
        || old.focus_text != new.focus_text
        || old.highlight != new.highlight
        || old.highlight_hover != new.highlight_hover
        || old.highlight_pressed != new.highlight_pressed
        || old.highlight_text != new.highlight_text;
    (non_accent, accent)
}
