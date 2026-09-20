//! Component recipes: which semantic role a component family paints with.
//!
//! A recipe is the theme answering *"what does a Button look like"* instead of
//! the button answering it about itself. Issue #101 §1.4 F5 counted 448 role
//! decisions spread across component bodies and the extraction pass; this
//! module is where they move to.
//!
//! **Phase 1 scope.** Two tables land here, chosen because both already
//! resolve somewhere the installed theme is in hand:
//!
//! - [`ComponentRecipe`] — the default foreground of each component family,
//!   which `world/extraction.rs` used to decide in a 25-arm `match` over
//!   `StandardVisual`. It resolves per extract, so an installed recipe reaches
//!   live nodes with no reprojection.
//! - [`ButtonRecipe`] — the variant × state colour table `Button::project`
//!   used to carry inline. It resolves at **projection** time, so a recipe
//!   change reaches a button when that button is next projected, not when the
//!   theme is installed. That gap is the same shape as Issue #101 §1.4 F1 and
//!   is named by a test rather than left to be discovered; closing it is the
//!   Component Recipe phase (#100 §3), which moves the slots onto the node the
//!   way `RadiusTier` moved radius.
//!
//! Slots are authored as `Option` and **compile fail-closed**: a registry that
//! leaves a required slot unset is rejected, never silently defaulted. That is
//! the whole reason the authoring and compiled forms are different types.

use crate::semantics::{ButtonKind, StatusTone};
use crate::style_model::SemanticColorRole;

/// Which component family a recipe describes.
///
/// The families are the ones the paint path actually distinguishes today,
/// which is also the first batch Issue #100 §3 names. A family whose visual is
/// only ever "the default text colour" still gets its own id: the point of a
/// registry is that a theme can move one family without moving the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentRecipeId {
    /// Buttons and button-shaped chips.
    Button,
    /// Standalone icon glyphs.
    Icon,
    /// Text fields and areas.
    TextInput,
    /// Checkbox and radio.
    Checkbox,
    /// Switch.
    Switch,
    /// Slider / range.
    Range,
    /// Cards and surfaces.
    Card,
    /// List rows.
    ListItem,
    /// Selectable option rows: segmented, tabs, dropdown options.
    Selection,
    /// Menus, popovers, select surfaces, command palette, tree views.
    Menu,
    /// Modal frames and dialogs.
    Overlay,
    /// Scrollbars.
    Scrollbar,
    /// Accent-driven indicators: progress, spinner, badges, form-field chrome.
    Indicator,
    /// Read-only content surfaces: charts, markdown, viewers, canvases.
    Content,
}

impl ComponentRecipeId {
    /// Every family, in declaration order. The compiled registry is an array
    /// indexed by [`Self::index`], so this is also the storage order.
    pub const ALL: [Self; 14] = [
        Self::Button,
        Self::Icon,
        Self::TextInput,
        Self::Checkbox,
        Self::Switch,
        Self::Range,
        Self::Card,
        Self::ListItem,
        Self::Selection,
        Self::Menu,
        Self::Overlay,
        Self::Scrollbar,
        Self::Indicator,
        Self::Content,
    ];

    pub const COUNT: usize = Self::ALL.len();

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Button => "Button",
            Self::Icon => "Icon",
            Self::TextInput => "TextInput",
            Self::Checkbox => "Checkbox",
            Self::Switch => "Switch",
            Self::Range => "Range",
            Self::Card => "Card",
            Self::ListItem => "ListItem",
            Self::Selection => "Selection",
            Self::Menu => "Menu",
            Self::Overlay => "Overlay",
            Self::Scrollbar => "Scrollbar",
            Self::Indicator => "Indicator",
            Self::Content => "Content",
        }
    }
}

/// One family's resolved foreground contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentRecipe {
    /// Foreground at rest.
    pub foreground: SemanticColorRole,
    /// Foreground while the family's primary indicator is on (a checked box, a
    /// switch thrown on). `None` for families with no on/off indicator —
    /// asking for one is then a programming error, not a missing token.
    pub foreground_checked: Option<SemanticColorRole>,
}

/// Authoring form of [`ComponentRecipe`]: slots may be unset, and compile
/// rejects a registry that leaves a required one unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ComponentRecipeDraft {
    pub foreground: Option<SemanticColorRole>,
    pub foreground_checked: Option<SemanticColorRole>,
}

impl ComponentRecipeDraft {
    pub const fn plain(foreground: SemanticColorRole) -> Self {
        Self {
            foreground: Some(foreground),
            foreground_checked: None,
        }
    }

    pub const fn checkable(
        foreground: SemanticColorRole,
        foreground_checked: SemanticColorRole,
    ) -> Self {
        Self {
            foreground: Some(foreground),
            foreground_checked: Some(foreground_checked),
        }
    }
}

/// One button variant's colour slots.
///
/// `background` and `border` are `Option` because "no fill" and "no stroke"
/// are real design answers for a ghost button — unlike [`Self::foreground`],
/// which every variant must state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ButtonVariantRecipe {
    pub foreground: SemanticColorRole,
    pub background: Option<SemanticColorRole>,
    pub border: Option<SemanticColorRole>,
    pub hovered_background: SemanticColorRole,
    pub pressed_background: SemanticColorRole,
}

/// Authoring form of [`ButtonVariantRecipe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ButtonVariantDraft {
    pub foreground: Option<SemanticColorRole>,
    /// Outer `None` means "unset, reject at compile"; `Some(None)` means "this
    /// variant deliberately has no fill".
    pub background: Option<Option<SemanticColorRole>>,
    pub border: Option<Option<SemanticColorRole>>,
    pub hovered_background: Option<SemanticColorRole>,
    pub pressed_background: Option<SemanticColorRole>,
}

/// Which button variant a slot belongs to. Mirrors [`ButtonKind`].
pub(super) const fn button_index(kind: ButtonKind) -> usize {
    match kind {
        ButtonKind::Ghost => 0,
        ButtonKind::Subtle => 1,
        ButtonKind::Selected => 2,
        ButtonKind::Primary => 3,
        ButtonKind::Warning => 4,
        ButtonKind::Danger => 5,
        ButtonKind::Text => 6,
        ButtonKind::Menu => 7,
    }
}

pub(super) const BUTTON_KINDS: [ButtonKind; 8] = [
    ButtonKind::Ghost,
    ButtonKind::Subtle,
    ButtonKind::Selected,
    ButtonKind::Primary,
    ButtonKind::Warning,
    ButtonKind::Danger,
    ButtonKind::Text,
    ButtonKind::Menu,
];

pub(super) const fn button_kind_name(kind: ButtonKind) -> &'static str {
    match kind {
        ButtonKind::Ghost => "Ghost",
        ButtonKind::Subtle => "Subtle",
        ButtonKind::Selected => "Selected",
        ButtonKind::Primary => "Primary",
        ButtonKind::Warning => "Warning",
        ButtonKind::Danger => "Danger",
        ButtonKind::Text => "Text",
        ButtonKind::Menu => "Menu",
    }
}

/// The compiled button table: one entry per [`ButtonKind`], plus the overlay
/// an invalid button paints on top of whichever variant it is.
///
/// `invalid_border` is an **overlay** state, not an exclusive one: an invalid
/// primary button keeps its primary fill and gains a danger stroke. Issue #100
/// §4 asks recipes to say which states are exclusive and which stack; this is
/// the first one that stacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ButtonRecipe {
    variants: [ButtonVariantRecipe; 8],
    pub invalid_border: SemanticColorRole,
}

impl ButtonRecipe {
    pub(super) const fn from_variants(
        variants: [ButtonVariantRecipe; 8],
        invalid_border: SemanticColorRole,
    ) -> Self {
        Self {
            variants,
            invalid_border,
        }
    }

    pub const fn variant(&self, kind: ButtonKind) -> ButtonVariantRecipe {
        self.variants[button_index(kind)]
    }
}

/// Authoring form of [`ButtonRecipe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ButtonRecipeDraft {
    pub variants: [ButtonVariantDraft; 8],
    pub invalid_border: Option<SemanticColorRole>,
}

impl ButtonRecipeDraft {
    pub const fn empty() -> Self {
        Self {
            variants: [ButtonVariantDraft {
                foreground: None,
                background: None,
                border: None,
                hovered_background: None,
                pressed_background: None,
            }; 8],
            invalid_border: None,
        }
    }

    pub const fn with(mut self, kind: ButtonKind, draft: ButtonVariantDraft) -> Self {
        self.variants[button_index(kind)] = draft;
        self
    }

    pub const fn variant(&self, kind: ButtonKind) -> ButtonVariantDraft {
        self.variants[button_index(kind)]
    }
}

/// Which semantic role each status tone paints with.
///
/// This replaces the free-standing `status_tone_role` helper in the runtime:
/// "info is the accent colour" is a design-system decision, not a runtime one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatusRecipe {
    pub neutral: SemanticColorRole,
    pub info: SemanticColorRole,
    pub success: SemanticColorRole,
    pub warning: SemanticColorRole,
    pub danger: SemanticColorRole,
}

impl StatusRecipe {
    /// The mapping both built-ins use. Light and dark disagree about what
    /// "warning" looks like, not about which role a warning paints with.
    pub const DEFAULT: Self = Self {
        neutral: SemanticColorRole::Muted,
        info: SemanticColorRole::Accent,
        success: SemanticColorRole::Success,
        warning: SemanticColorRole::Warning,
        danger: SemanticColorRole::Danger,
    };

    pub const fn role(self, tone: StatusTone) -> SemanticColorRole {
        match tone {
            StatusTone::Neutral => self.neutral,
            StatusTone::Info => self.info,
            StatusTone::Success => self.success,
            StatusTone::Warning => self.warning,
            StatusTone::Danger => self.danger,
        }
    }
}

/// Authoring form of the whole registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentThemeRegistry {
    pub families: [ComponentRecipeDraft; ComponentRecipeId::COUNT],
    pub button: ButtonRecipeDraft,
    pub status: Option<StatusRecipe>,
}

impl ComponentThemeRegistry {
    /// An empty registry. Compiling it fails on the first missing slot, which
    /// is the point: a theme that forgets a family is rejected, not patched.
    pub const fn empty() -> Self {
        Self {
            families: [ComponentRecipeDraft {
                foreground: None,
                foreground_checked: None,
            }; ComponentRecipeId::COUNT],
            button: ButtonRecipeDraft::empty(),
            status: None,
        }
    }

    pub const fn with(mut self, id: ComponentRecipeId, draft: ComponentRecipeDraft) -> Self {
        self.families[id.index()] = draft;
        self
    }

    pub const fn draft(&self, id: ComponentRecipeId) -> ComponentRecipeDraft {
        self.families[id.index()]
    }
}

/// The validated registry the runtime reads. Every slot is present; lookups
/// are array indexing by a typed enum, never a string hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompiledRecipes {
    families: [ComponentRecipe; ComponentRecipeId::COUNT],
    button: ButtonRecipe,
    status: StatusRecipe,
}

impl CompiledRecipes {
    pub(super) const fn new(
        families: [ComponentRecipe; ComponentRecipeId::COUNT],
        button: ButtonRecipe,
        status: StatusRecipe,
    ) -> Self {
        Self {
            families,
            button,
            status,
        }
    }

    pub const fn family(&self, id: ComponentRecipeId) -> ComponentRecipe {
        self.families[id.index()]
    }

    /// Foreground for a family whose indicator is in the given state.
    ///
    /// `checked` on a family with no indicator falls back to the rest
    /// foreground rather than erroring: the caller asking is a paint path, and
    /// a paint path has to produce a colour.
    pub const fn foreground(&self, id: ComponentRecipeId, checked: bool) -> SemanticColorRole {
        let recipe = self.families[id.index()];
        match (checked, recipe.foreground_checked) {
            (true, Some(role)) => role,
            _ => recipe.foreground,
        }
    }

    pub const fn button(&self) -> &ButtonRecipe {
        &self.button
    }

    pub const fn status(&self) -> StatusRecipe {
        self.status
    }
}
