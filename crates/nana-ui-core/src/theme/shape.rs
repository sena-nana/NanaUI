//! [`ShapeRecipe`]: which radius step a built-in component's corners take.
//!
//! A component used to name its own [`RadiusTier`] (a button said `Sm`), so
//! the theme could move how round `Sm` is but never which step a button
//! sits on: a design system whose buttons and tags are pills had to restyle
//! every one of them by hand, and lost that the moment a theme swap rebuilt
//! the view. The recipe moves the choice of step to the theme. A component
//! names its [`ShapeRole`] on the node (`NodeStyle::shape`); the installed
//! theme's recipe picks the step and its metrics the pixels, both at style
//! resolution, so a theme installed later reaches buttons already on screen.
//! Rows a scene painter draws without a node of their own (dropdown option
//! rows, a menu's search field) read the same recipe through
//! [`super::ChromeRadii`], resolved at every extract.
//!
//! A step a node names itself (`NodeStyle::radius`, `corner_radii`) or a
//! number it writes over the component's style still wins: the recipe only
//! answers for corners nobody else decided.

use super::{RadiusTier, ThemeMetrics};

use serde::{Deserialize, Serialize};

/// Which built-in component's corner the [`ShapeRecipe`] decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShapeRole {
    /// `Button`, every kind and size.
    Button,
    /// `IconButton`, compact heading tools included.
    IconButton,
    /// The quiet label `tag` / `tag_frame` draws.
    Tag,
    /// Text fields: `TextInput`, `NumberInput`, `TextArea`, and the frames
    /// of the composite fields built on them (colour, size, scrub, path),
    /// plus the search field a menu surface paints.
    Field,
    /// A `Select`'s trigger.
    Select,
    /// A `ListItem` row: its hover, press and selected fill.
    ListItem,
    /// A segmented control's track. The selected option's pill is cut from
    /// the track's resolved corner, so it stays concentric with it.
    Segmented,
    /// An option row in a dropdown: a `Select`'s list, the rows of a menu
    /// surface (action, context and popover menus) and a command palette's
    /// result rows.
    MenuRow,
    /// A tab in a `Tabs` strip.
    Tab,
    /// A row of a radio group: its hover and press fill.
    RadioRow,
    /// A `TreeView` row's hover and selected fill, and the outline of the
    /// row a drag would drop into.
    TreeRow,
}

impl ShapeRole {
    pub const ALL: [Self; 11] = [
        Self::Button,
        Self::IconButton,
        Self::Tag,
        Self::Field,
        Self::Select,
        Self::ListItem,
        Self::Segmented,
        Self::MenuRow,
        Self::Tab,
        Self::RadioRow,
        Self::TreeRow,
    ];
}

/// The radius step each [`ShapeRole`] takes. The built-in value is what the
/// components named themselves before the recipe existed, so a theme that
/// does not set one looks exactly as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ShapeRecipe {
    pub button: RadiusTier,
    pub icon_button: RadiusTier,
    pub tag: RadiusTier,
    pub field: RadiusTier,
    pub select: RadiusTier,
    pub list_item: RadiusTier,
    pub segmented: RadiusTier,
    pub menu_row: RadiusTier,
    pub tab: RadiusTier,
    pub radio_row: RadiusTier,
    pub tree_row: RadiusTier,
}

impl ShapeRecipe {
    pub const DEFAULT: Self = Self {
        button: RadiusTier::Sm,
        icon_button: RadiusTier::Sm,
        tag: RadiusTier::Sm,
        field: RadiusTier::Sm,
        select: RadiusTier::Sm,
        list_item: RadiusTier::Sm,
        segmented: RadiusTier::Sm,
        menu_row: RadiusTier::Sm,
        tab: RadiusTier::Sm,
        radio_row: RadiusTier::Sm,
        tree_row: RadiusTier::Sm,
    };

    /// Every role on `tier`: `ShapeRecipe::uniform(RadiusTier::Full)` makes
    /// every shaped component a pill — fields, list rows and menu rows too.
    /// To move only some, start from [`Self::DEFAULT`] and [`Self::with`].
    pub const fn uniform(tier: RadiusTier) -> Self {
        Self {
            button: tier,
            icon_button: tier,
            tag: tier,
            field: tier,
            select: tier,
            list_item: tier,
            segmented: tier,
            menu_row: tier,
            tab: tier,
            radio_row: tier,
            tree_row: tier,
        }
    }

    /// This recipe with `role` on `tier`.
    pub const fn with(mut self, role: ShapeRole, tier: RadiusTier) -> Self {
        match role {
            ShapeRole::Button => self.button = tier,
            ShapeRole::IconButton => self.icon_button = tier,
            ShapeRole::Tag => self.tag = tier,
            ShapeRole::Field => self.field = tier,
            ShapeRole::Select => self.select = tier,
            ShapeRole::ListItem => self.list_item = tier,
            ShapeRole::Segmented => self.segmented = tier,
            ShapeRole::MenuRow => self.menu_row = tier,
            ShapeRole::Tab => self.tab = tier,
            ShapeRole::RadioRow => self.radio_row = tier,
            ShapeRole::TreeRow => self.tree_row = tier,
        }
        self
    }

    /// The step `role` takes.
    pub const fn tier(&self, role: ShapeRole) -> RadiusTier {
        match role {
            ShapeRole::Button => self.button,
            ShapeRole::IconButton => self.icon_button,
            ShapeRole::Tag => self.tag,
            ShapeRole::Field => self.field,
            ShapeRole::Select => self.select,
            ShapeRole::ListItem => self.list_item,
            ShapeRole::Segmented => self.segmented,
            ShapeRole::MenuRow => self.menu_row,
            ShapeRole::Tab => self.tab,
            ShapeRole::RadioRow => self.radio_row,
            ShapeRole::TreeRow => self.tree_row,
        }
    }

    /// `role`'s corner in logical pixels under `metrics`.
    pub const fn resolve(&self, role: ShapeRole, metrics: ThemeMetrics) -> f32 {
        self.tier(role).resolve(metrics)
    }
}

impl Default for ShapeRecipe {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_shape_is_the_step_each_component_named_before() {
        for role in ShapeRole::ALL {
            assert_eq!(ShapeRecipe::DEFAULT.tier(role), RadiusTier::Sm);
        }
    }

    #[test]
    fn with_moves_one_role_and_leaves_the_rest() {
        let recipe = ShapeRecipe::DEFAULT.with(ShapeRole::Field, RadiusTier::Full);
        for role in ShapeRole::ALL {
            let expected = if role == ShapeRole::Field {
                RadiusTier::Full
            } else {
                RadiusTier::Sm
            };
            assert_eq!(recipe.tier(role), expected, "{role:?}");
        }
    }

    #[test]
    fn a_uniform_recipe_moves_every_role() {
        let pill = ShapeRecipe::uniform(RadiusTier::Full);
        for role in ShapeRole::ALL {
            assert_eq!(pill.tier(role), RadiusTier::Full);
            assert_eq!(
                pill.resolve(role, super::super::UI_METRICS),
                super::super::FULL_RADIUS
            );
        }
    }
}
