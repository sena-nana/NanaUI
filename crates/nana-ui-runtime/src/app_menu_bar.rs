//! In-app menu bar (#339): the application's [`MenuBar`] model drawn inside
//! its own window — a custom title bar, the top of a pane — in the theme's
//! colors, where the platform's native menu cannot go.
//!
//! A press on a title opens its menu; while one is open, moving onto another
//! title switches to it. Submenus open to the side of their row and flip to
//! the other side when the window is too narrow; a submenu stays open while
//! the pointer heads for it across its parent's other rows, until the pointer
//! rests there (see `crate::submenu_aim`). A press-drag from
//! a title onto an item chooses it on release. Alt pressed and released alone
//! or F10 give the bar the keyboard: the arrows walk the titles and the rows,
//! Enter chooses, Escape closes one level at a time.
//!
//! A chosen item's id goes to the same queue as the native menu's
//! ([`nana_ui_core::take_menu_activations`]), and the bar also emits
//! [`MenuBarActivated`]. The shortcut text beside an item is display only:
//! the application's keymap still owns the key.
//!
//! While a menu is open a transparent layer over the window takes the
//! pointer, so a press outside closes the menu without reaching what is
//! underneath.

use std::sync::Arc;
use std::time::Duration;

use nana_ui_core::{
    AlignSpec, BoxShadowSpec, ControlSize, DisplaySpec, FlexDirection, LengthSpec, Menu, MenuBar,
    MenuEntry, MenuShortcut, PositionSpec, SemanticColorRole, ViewportAxis,
};

use crate::popover::{
    MENU_ITEM_GAP, MENU_MIN_WIDTH, MENU_OVERLAY_Z_INDEX, MENU_SURFACE_PADDING, menu_surface_style,
};
use crate::submenu_aim::SubmenuAim;
use crate::text_width::estimated_text_width;
use crate::view_components::{Text, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, DocumentId, Entity,
    FrameworkError, InteractionState, LayoutBox, MutationQueue, NodeKind, NodePointer,
    NodePointerHooks, NodeStyle, NodeWheel, StableNodeId, TextContent, TextHorizontalAlignment,
    UiWorld,
};

/// The check mark column, px.
const MARK_WIDTH: f32 = 16.0;
const CHECK: &str = "✓";
const SUBMENU: &str = "›";
const SEPARATOR_HEIGHT: f32 = nana_ui_core::HAIRLINE;
const MAX_MENU_WIDTH: f32 = 480.0;

/// An item was chosen. Its id also goes to
/// [`nana_ui_core::take_menu_activations`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuBarActivated {
    pub id: u32,
}

/// One open menu: the highlighted entry and the submenu open from it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MenuLevel {
    pub(crate) hot: Option<usize>,
    pub(crate) sub: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct MenuBarState {
    /// The highlighted title: its menu is open, or the pointer is over it, or
    /// the keyboard holds the bar.
    pub(crate) title: Option<usize>,
    /// The open menus, the title's first.
    pub(crate) levels: Vec<MenuLevel>,
    /// The keyboard holds the bar.
    pub(crate) keyboard: bool,
    /// Where the pointer was at its last move or press.
    last: Option<(f32, f32)>,
    /// The pointer heading for an open submenu across its parent's rows.
    pub(crate) aim: Option<MenuAim>,
    /// The press that opened the menu: releasing it on the title keeps the
    /// menu open.
    opening: bool,
    /// The press that closed the menu: the layer stays up and takes its
    /// release, so nothing underneath sees half a click.
    swallow: bool,
}

/// The pointer heading for the submenu open from `level`'s menu, and the
/// row of that menu it is over meanwhile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MenuAim {
    level: usize,
    row: Option<usize>,
    pub(crate) aim: SubmenuAim,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MenuRowParts {
    pub(crate) row: StableNodeId,
    /// Mark, label and shortcut; a separator has none.
    pub(crate) texts: Option<[StableNodeId; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MenuPanelParts {
    /// The title's index, then the entry of each submenu on the way.
    key: Vec<usize>,
    pub(crate) panel: StableNodeId,
    pub(crate) rows: Vec<MenuRowParts>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MenuBarParts {
    /// Each title's box and text.
    pub(crate) titles: Vec<(StableNodeId, StableNodeId)>,
    pub(crate) panels: Vec<MenuPanelParts>,
    pub(crate) scrim: StableNodeId,
}

/// The menu bar drawn by the runtime; see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct AppMenuBar {
    pub bar: MenuBar,
    /// Alt pressed and released alone gives the bar the keyboard. F10 always
    /// does.
    pub alt_key: bool,
    /// How the shortcut text beside an item is spelled; the host's
    /// convention unless set.
    pub shortcut_notation: ShortcutNotation,
    pub style: NodeStyle,
    pub(crate) state: MenuBarState,
    pub(crate) parts: Option<MenuBarParts>,
}

impl AppMenuBar {
    pub fn new(bar: MenuBar) -> Self {
        Self {
            bar,
            alt_key: true,
            shortcut_notation: ShortcutNotation::HOST,
            style: bar_style(),
            state: MenuBarState::default(),
            parts: None,
        }
    }

    /// Replaces the node style wholesale.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn alt_key(mut self, alt_key: bool) -> Self {
        self.alt_key = alt_key;
        self
    }

    pub fn shortcut_notation(mut self, notation: ShortcutNotation) -> Self {
        self.shortcut_notation = notation;
        self
    }

    /// Show `bar` from now on: relabelled menus after a language switch, a
    /// check mark that moved. Whatever was open closes when the menus differ.
    pub fn set_bar(&mut self, bar: MenuBar) {
        if self.bar != bar {
            self.bar = bar;
            self.state = MenuBarState::default();
        }
    }

    /// A menu is open.
    pub fn is_open(&self) -> bool {
        !self.state.levels.is_empty()
    }

    /// The bar is open or holds the keyboard.
    pub fn is_active(&self) -> bool {
        self.is_open() || (self.state.keyboard && self.state.title.is_some())
    }

    /// Titles of the open menus, the bar's first: `["File", "Recent"]`.
    pub fn open_titles(&self) -> Vec<&str> {
        self.open_menus()
            .into_iter()
            .map(|menu| menu.title.as_str())
            .collect()
    }

    /// Label of the highlighted entry of the innermost open menu.
    pub fn highlighted(&self) -> Option<&str> {
        let menus = self.open_menus();
        let level = self.state.levels.len().checked_sub(1)?;
        let hot = self.state.levels[level].hot?;
        match menus.get(level)?.entries.get(hot)? {
            MenuEntry::Item { label, .. } => Some(label),
            MenuEntry::Submenu(menu) => Some(&menu.title),
            MenuEntry::Separator => None,
        }
    }

    /// The open menus, one per level.
    fn open_menus(&self) -> Vec<&Menu> {
        let Some(mut menu) = self
            .state
            .title
            .filter(|_| self.is_open())
            .and_then(|title| self.bar.menus.get(title))
        else {
            return Vec::new();
        };
        let mut menus = vec![menu];
        for level in &self.state.levels[..self.state.levels.len() - 1] {
            match level.sub.and_then(|sub| menu.entries.get(sub)) {
                Some(MenuEntry::Submenu(sub)) => {
                    menu = sub;
                    menus.push(menu);
                }
                _ => break,
            }
        }
        menus
    }

    fn entry(&self, level: usize, index: usize) -> Option<&MenuEntry> {
        self.open_menus().get(level)?.entries.get(index)
    }

    fn open_menu(&mut self, title: usize, keyboard: bool, from_end: bool) {
        let Some(menu) = self.bar.menus.get(title) else {
            return;
        };
        let hot = keyboard
            .then(|| step(&menu.entries, None, if from_end { -1 } else { 1 }))
            .flatten();
        self.state.title = Some(title);
        self.state.levels = vec![MenuLevel { hot, sub: None }];
        self.state.keyboard = keyboard;
        self.state.aim = None;
    }

    /// Open the submenu at `index` of `level`, its first row highlighted when
    /// the keyboard opened it.
    fn open_submenu(&mut self, level: usize, index: usize, keyboard: bool) {
        let hot = match self.entry(level, index) {
            Some(MenuEntry::Submenu(menu)) => keyboard.then(|| step(&menu.entries, None, 1)),
            _ => return,
        };
        self.state.levels.truncate(level + 1);
        self.state.levels[level].hot = Some(index);
        self.state.levels[level].sub = Some(index);
        self.state.levels.push(MenuLevel {
            hot: hot.flatten(),
            sub: None,
        });
        self.state.aim = None;
    }

    /// The pointer takes `index` of `level`: it highlights, and opens when
    /// it is a submenu.
    fn hover_row(&mut self, level: usize, index: usize) {
        if self.state.levels[level].sub != Some(index) {
            self.state.levels.truncate(level + 1);
            self.state.levels[level].sub = None;
            self.state.levels[level].hot = Some(index);
            if matches!(self.entry(level, index), Some(MenuEntry::Submenu(_))) {
                self.open_submenu(level, index, false);
            }
        }
        self.state.keyboard = false;
    }

    /// Whether the pointer at `at`, over `row` of `level`'s menu (`None`:
    /// between its rows or off the menus), still heads for the submenu open
    /// from that menu. An aim starts from the pointer's last place when
    /// `begin` is set.
    fn aiming(
        &mut self,
        level: usize,
        row: Option<usize>,
        at: (f32, f32),
        now: Duration,
        panels: &[Option<LayoutBox>],
        begin: bool,
    ) -> bool {
        let aim = match self.state.aim.filter(|aim| aim.level == level) {
            Some(aim) => Some(aim.aim),
            None if begin => match (self.state.last, panels.get(level + 1)) {
                (Some(from), Some(Some(target))) => Some(SubmenuAim::begin(from, *target, now)),
                _ => None,
            },
            None => None,
        };
        self.state.aim =
            aim.and_then(|mut aim| aim.track(at, now).then_some(MenuAim { level, row, aim }));
        self.state.aim.is_some()
    }

    /// The pointer left the rows of `level`'s menu (any menu when `None`):
    /// an aim from it goes on while it holds, with no row waiting.
    fn keep_aiming(
        &mut self,
        level: Option<usize>,
        at: (f32, f32),
        now: Duration,
        panels: &[Option<LayoutBox>],
    ) {
        match self.state.aim {
            Some(aim) if level.is_none_or(|level| level == aim.level) => {
                self.aiming(aim.level, None, at, now, panels, false);
            }
            _ => self.state.aim = None,
        }
    }

    /// The pointer rested inside the triangle until `now`: the row it is
    /// over takes the highlight.
    pub(crate) fn settle_aim(&mut self, now: Duration) {
        let Some(aim) = self.state.aim.filter(|aim| now >= aim.aim.until()) else {
            return;
        };
        self.state.aim = None;
        if let Some(row) = aim.row
            && aim.level < self.state.levels.len()
        {
            self.hover_row(aim.level, row);
        }
    }

    fn close(&mut self) {
        self.state = MenuBarState::default();
    }

    /// The pointer moved to `hit` at `at`. `panels` are the open panels'
    /// boxes, outermost first. `aim` is false for a finger, which has no
    /// path between rows to follow.
    fn hover(
        &mut self,
        hit: MenuHit,
        at: (f32, f32),
        now: Duration,
        aim: bool,
        panels: &[Option<LayoutBox>],
    ) {
        if !self.is_open() {
            if let MenuHit::Title(title) = hit {
                self.state.title = Some(title);
                self.state.keyboard = false;
            } else if !self.state.keyboard {
                self.state.title = None;
            }
            return;
        }
        match hit {
            MenuHit::Title(title) if self.state.title != Some(title) => {
                self.open_menu(title, false, false);
            }
            MenuHit::Row(level, index) => {
                let open = self.state.levels[level].sub;
                // a sibling row crossed on the way to the open submenu waits
                if !(open.is_some_and(|sub| sub != index)
                    && self.aiming(level, Some(index), at, now, panels, aim))
                {
                    self.state.aim = None;
                    self.hover_row(level, index);
                }
            }
            MenuHit::Outside => {
                if let Some(last) = self.state.levels.last_mut()
                    && last.sub.is_none()
                {
                    last.hot = None;
                }
                self.keep_aiming(None, at, now, panels);
            }
            MenuHit::Panel(level) => self.keep_aiming(Some(level), at, now, panels),
            MenuHit::Title(_) => self.state.aim = None,
        }
        self.state.last = Some(at);
    }

    /// A press at `hit`; answers whether the bar takes it.
    fn press(&mut self, hit: MenuHit) -> bool {
        let open = self.is_open();
        match hit {
            MenuHit::Title(title) if open && self.state.title == Some(title) => {
                self.close();
                self.state.title = Some(title);
                self.state.swallow = true;
            }
            MenuHit::Title(title) => {
                self.open_menu(title, false, false);
                self.state.opening = true;
            }
            MenuHit::Row(..) | MenuHit::Panel(_) => {}
            MenuHit::Outside if self.is_active() => {
                self.close();
                self.state.swallow = open;
            }
            MenuHit::Outside => return false,
        }
        true
    }

    /// The release of a press the bar took: the id of the item it chose.
    fn release(&mut self, hit: MenuHit) -> Option<u32> {
        self.state.opening = false;
        self.state.swallow = false;
        let MenuHit::Row(level, index) = hit else {
            return None;
        };
        match self.entry(level, index)? {
            MenuEntry::Item {
                id, enabled: true, ..
            } => {
                let id = *id;
                self.close();
                Some(id)
            }
            MenuEntry::Submenu(_) if self.state.levels[level].sub != Some(index) => {
                self.open_submenu(level, index, false);
                None
            }
            _ => None,
        }
    }

    /// A key while the bar is active.
    fn key(&mut self, key: &str) -> MenuKey {
        let Some(title) = self.state.title else {
            return MenuKey::Ignored;
        };
        let count = self.bar.menus.len().max(1);
        let previous = (title + count - 1) % count;
        let next = (title + 1) % count;
        let open = self.is_open();
        if !open {
            match key {
                "ArrowLeft" => self.state.title = Some(previous),
                "ArrowRight" => self.state.title = Some(next),
                "ArrowDown" | "Enter" | " " | "Space" => self.open_menu(title, true, false),
                "ArrowUp" => self.open_menu(title, true, true),
                "Escape" => self.close(),
                _ => {
                    self.close();
                    return MenuKey::Ignored;
                }
            }
            return MenuKey::Consumed;
        }
        self.state.aim = None;
        let depth = self.state.levels.len() - 1;
        let entries = self
            .open_menus()
            .get(depth)
            .map(|menu| menu.entries.clone())
            .unwrap_or_default();
        let hot = self.state.levels[depth].hot;
        match key {
            "Escape" | "ArrowLeft" if depth > 0 => {
                self.state.levels.pop();
                self.state.levels[depth - 1].sub = None;
            }
            "Escape" => {
                self.state.levels.clear();
            }
            "ArrowLeft" => self.open_menu(previous, true, false),
            "ArrowRight" => match hot.and_then(|hot| entries.get(hot)) {
                Some(MenuEntry::Submenu(_)) => self.open_submenu(depth, hot.unwrap(), true),
                _ => self.open_menu(next, true, false),
            },
            "ArrowDown" => self.state.levels[depth].hot = step(&entries, hot, 1),
            "ArrowUp" => self.state.levels[depth].hot = step(&entries, hot, -1),
            "Home" => self.state.levels[depth].hot = step(&entries, None, 1),
            "End" => self.state.levels[depth].hot = step(&entries, None, -1),
            "Enter" | " " | "Space" => match hot.and_then(|hot| entries.get(hot)) {
                Some(MenuEntry::Submenu(_)) => self.open_submenu(depth, hot.unwrap(), true),
                Some(MenuEntry::Item {
                    id, enabled: true, ..
                }) => {
                    let id = *id;
                    self.close();
                    return MenuKey::Activated(id);
                }
                _ => {}
            },
            _ => {}
        }
        self.state.keyboard = true;
        MenuKey::Consumed
    }

    fn effective_style(&self) -> NodeStyle {
        self.style.clone()
    }
}

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuHit {
    Title(usize),
    /// An entry of the open menu at a level.
    Row(usize, usize),
    /// A menu's padding or a separator.
    Panel(usize),
    Outside,
}

pub(crate) enum MenuKey {
    Ignored,
    Consumed,
    Activated(u32),
}

/// The entry `direction` steps from `from` (from an end when `None`),
/// over separators, wrapping.
fn step(entries: &[MenuEntry], from: Option<usize>, direction: isize) -> Option<usize> {
    let count = entries.len() as isize;
    if count == 0 {
        return None;
    }
    let mut index = match from {
        Some(from) => from as isize,
        None if direction > 0 => -1,
        None => count,
    };
    for _ in 0..count {
        index = (index + direction).rem_euclid(count);
        if !matches!(entries[index as usize], MenuEntry::Separator) {
            return Some(index as usize);
        }
    }
    None
}

/// How shortcut text is spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutNotation {
    /// The macOS symbols: `⇧⌘S`.
    Symbols,
    /// Modifier names joined by `+`: `Ctrl+Shift+S`.
    Names,
}

impl ShortcutNotation {
    /// The convention of the platform this was built for.
    pub const HOST: Self = if cfg!(target_os = "macos") {
        Self::Symbols
    } else {
        Self::Names
    };

    pub fn label(self, shortcut: &MenuShortcut) -> String {
        let key = if shortcut.key.chars().count() == 1 {
            shortcut.key.to_uppercase()
        } else {
            shortcut.key.clone()
        };
        if self == Self::Symbols {
            let mut text = String::new();
            for (on, mark) in [
                (shortcut.alt, "⌥"),
                (shortcut.shift, "⇧"),
                (shortcut.primary, "⌘"),
            ] {
                if on {
                    text.push_str(mark);
                }
            }
            return text + &key;
        }
        let mut parts = Vec::new();
        for (on, name) in [
            (shortcut.primary, "Ctrl"),
            (shortcut.alt, "Alt"),
            (shortcut.shift, "Shift"),
        ] {
            if on {
                parts.push(name);
            }
        }
        parts.push(&key);
        parts.join("+")
    }
}

/// Platform-conventional text for a shortcut: `Ctrl+Shift+S`, `⇧⌘S`.
pub fn menu_shortcut_label(shortcut: &MenuShortcut) -> String {
    ShortcutNotation::HOST.label(shortcut)
}

impl ComponentView for AppMenuBar {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(AppContext::assemble_app_menu_bar),
        // a press elsewhere lets go of a bar the keyboard holds; an open menu
        // closes on its own layer
        close_options: Some(|cx, bar| {
            cx.drive_menu_bar(bar, |bar| {
                if bar.state.keyboard && !bar.is_open() {
                    bar.close();
                    true
                } else {
                    false
                }
            })
        }),
        pointer: Some(&BAR_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        // what is open is the bar's own, until the menus themselves change
        if next.bar == self.bar {
            next.state = std::mem::take(&mut self.state);
        }
        next.parts = self.parts.take();
        *self = next;
    }

    fn keep_framework_state(&mut self, previous: &Self) {
        if self.parts.is_none() {
            self.parts = previous.parts.clone();
        }
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "app-menu-bar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Menu,
                role_description: Some(Arc::from("menu bar")),
                orientation: Some(crate::SelectionOrientation::Horizontal),
                ..AccessibilityState::default()
            },
        );
    }
}

/// A title, a menu, a row or a separator: a styled box the bar keeps.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MenuBarBox {
    tag: &'static str,
    style: NodeStyle,
    accessibility: AccessibilityState,
}

impl MenuBarBox {
    fn new(tag: &'static str) -> Self {
        Self {
            tag,
            style: NodeStyle::default(),
            accessibility: AccessibilityState::default(),
        }
    }
}

impl ComponentView for MenuBarBox {
    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: self.tag.into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        // the bar finds what the pointer is over itself
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            self.accessibility.clone(),
        );
    }
}

/// The transparent layer over the window while a menu is open.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MenuBarScrim {
    bar: StableNodeId,
    open: bool,
}

impl ComponentView for MenuBarScrim {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        pointer: Some(&SCRIM_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "menu-bar-scrim".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &scrim_style(self.open),
            InteractionState {
                pointer_events: self.open,
                focusable: false,
            },
            AccessibilityState {
                hidden: true,
                ..AccessibilityState::default()
            },
        );
    }
}

fn bar_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Stretch;
        layout.width = Some(LengthSpec::Fill);
        layout.padding_left = Some(LengthSpec::Px(nana_ui_core::space::XXS));
        layout.padding_right = Some(LengthSpec::Px(nana_ui_core::space::XXS));
    }
    style.control_height = Some(nana_ui_core::ControlHeight::Exact(ControlSize::Small));
    style
}

fn title_style(hot: bool, open: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.padding_left = Some(LengthSpec::Px(nana_ui_core::space::SM));
        layout.padding_right = Some(LengthSpec::Px(nana_ui_core::space::SM));
    }
    style.radius = Some(nana_ui_core::RadiusTier::Sm);
    style.background = if open {
        Some(SemanticColorRole::Active)
    } else if hot {
        Some(SemanticColorRole::Hover)
    } else {
        None
    };
    style
}

fn row_style(hot: bool) -> NodeStyle {
    let size = ControlSize::Small;
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.gap = Some(LengthSpec::Px(nana_ui_core::space::XS));
        layout.width = Some(LengthSpec::Fill);
        layout.flex_shrink = Some(0.0);
    }
    style.radius = Some(nana_ui_core::RadiusTier::Sm);
    style.control_height = Some(nana_ui_core::ControlHeight::Exact(size));
    style.control_padding_x = Some(size.into());
    style.background = hot.then_some(SemanticColorRole::Hover);
    style
}

fn separator_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(SEPARATOR_HEIGHT));
        layout.flex_shrink = Some(0.0);
    }
    style.background = Some(SemanticColorRole::Border);
    style
}

fn panel_style(rect: LayoutBox) -> NodeStyle {
    let mut style = menu_surface_style(rect.width, MENU_SURFACE_PADDING);
    let layout = Arc::make_mut(&mut style.layout);
    layout.offset_left = Some(LengthSpec::Px(rect.x));
    layout.offset_top = Some(LengthSpec::Px(rect.y));
    layout.height = Some(LengthSpec::Px(rect.height));
    layout.paint.box_shadows = vec![BoxShadowSpec {
        paint_color: None,
        offset_x: 0.0,
        offset_y: 4.0,
        blur_radius: 16.0,
        spread_radius: 0.0,
        color: [0.0, 0.0, 0.0, 0.4],
        inset: false,
    }];
    style
}

fn scrim_style(open: bool) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Fixed;
    layout.offset_left = Some(LengthSpec::Px(0.0));
    layout.offset_top = Some(LengthSpec::Px(0.0));
    layout.width = Some(LengthSpec::Viewport {
        axis: ViewportAxis::Width,
        value: 100.0,
    });
    layout.height = Some(LengthSpec::Viewport {
        axis: ViewportAxis::Height,
        value: 100.0,
    });
    layout.z_index = Some(MENU_OVERLAY_Z_INDEX + 1);
    layout.display = (!open).then_some(DisplaySpec::None);
    style
}

fn text_style(size: f32, width: Option<LengthSpec>, color: SemanticColorRole) -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.font_size = Some(size);
        layout.width = width;
        layout.flex_shrink = Some(0.0);
    }
    style.foreground = Some(color);
    style
}

/// Each entry's height, px.
fn entry_height(entry: &MenuEntry, row: f32) -> f32 {
    match entry {
        MenuEntry::Separator => SEPARATOR_HEIGHT,
        _ => row,
    }
}

/// Distance from a menu's top to the top of the entry at `index`.
fn entry_top(entries: &[MenuEntry], index: usize, row: f32) -> f32 {
    let inset = MENU_SURFACE_PADDING + nana_ui_core::HAIRLINE;
    inset
        + entries[..index]
            .iter()
            .map(|entry| entry_height(entry, row) + MENU_ITEM_GAP)
            .sum::<f32>()
}

fn menu_height(entries: &[MenuEntry], row: f32) -> f32 {
    let inset = MENU_SURFACE_PADDING + nana_ui_core::HAIRLINE;
    let rows = entries
        .iter()
        .map(|entry| entry_height(entry, row))
        .sum::<f32>();
    2.0 * inset + rows + MENU_ITEM_GAP * entries.len().saturating_sub(1) as f32
}

fn menu_width(
    entries: &[MenuEntry],
    notation: ShortcutNotation,
    metrics: nana_ui_core::ThemeMetrics,
) -> f32 {
    let size = ControlSize::Small;
    let mut label = 0.0f32;
    let mut hint = estimated_text_width(SUBMENU, nana_ui_core::type_scale::HINT);
    for entry in entries {
        match entry {
            MenuEntry::Item {
                label: text,
                shortcut,
                ..
            } => {
                label = label.max(estimated_text_width(text, size.text_size()));
                if let Some(shortcut) = shortcut {
                    hint = hint.max(estimated_text_width(
                        &notation.label(shortcut),
                        nana_ui_core::type_scale::HINT,
                    ));
                }
            }
            MenuEntry::Submenu(menu) => {
                label = label.max(estimated_text_width(&menu.title, size.text_size()));
            }
            MenuEntry::Separator => {}
        }
    }
    let gap = nana_ui_core::space::XS;
    let row = 2.0 * size.padding_x_in(metrics)
        + MARK_WIDTH
        + gap
        + label
        + nana_ui_core::space::LG
        + hint;
    let inset = MENU_SURFACE_PADDING + nana_ui_core::HAIRLINE;
    (row + 2.0 * inset).clamp(MENU_MIN_WIDTH, MAX_MENU_WIDTH)
}

const BAR_POINTER: NodePointerHooks<AppMenuBar> = NodePointerHooks {
    moved: |cx, bar, pointer| cx.menu_bar_pointer(bar, bar.stable_id(), pointer, Phase::Move),
    left: |cx, bar| {
        cx.drive_menu_bar(bar, |bar| {
            if !bar.is_open() && !bar.state.keyboard {
                bar.state.title = None;
            }
        })
    },
    pressed: |cx, bar, pointer| cx.menu_bar_pointer(bar, bar.stable_id(), pointer, Phase::Press),
    released: |cx, bar, pointer| cx.menu_bar_pointer(bar, bar.stable_id(), pointer, Phase::Release),
    wheel: no_wheel,
};

const SCRIM_POINTER: NodePointerHooks<MenuBarScrim> = NodePointerHooks {
    moved: |cx, scrim, pointer| scrim_pointer(cx, scrim, pointer, Phase::Move),
    left: |_, _| Ok(()),
    pressed: |cx, scrim, pointer| scrim_pointer(cx, scrim, pointer, Phase::Press),
    released: |cx, scrim, pointer| scrim_pointer(cx, scrim, pointer, Phase::Release),
    wheel: |_, _, _: NodeWheel| Ok(true),
};

fn no_wheel<C: crate::View>(
    _: &mut AppContext,
    _: Entity<C>,
    _: NodeWheel,
) -> Result<bool, FrameworkError> {
    Ok(false)
}

fn scrim_pointer(
    cx: &mut AppContext,
    scrim: Entity<MenuBarScrim>,
    pointer: NodePointer,
    phase: Phase,
) -> Result<bool, FrameworkError> {
    let bar = cx.read(scrim, |scrim| scrim.bar)?;
    let Some(bar) = cx.view_entity::<AppMenuBar>(bar) else {
        return Ok(false);
    };
    cx.menu_bar_pointer(bar, scrim.stable_id(), pointer, phase)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Move,
    Press,
    Release,
}

impl AppContext {
    /// Change `bar`'s state through `change`; writes the bar only when the
    /// state moved, so a pointer that keeps still over a row costs nothing.
    pub(crate) fn drive_menu_bar<R>(
        &mut self,
        bar: Entity<AppMenuBar>,
        change: impl FnOnce(&mut AppMenuBar) -> R,
    ) -> Result<R, FrameworkError> {
        let mut copy = self.read(bar, Clone::clone)?;
        let before = copy.state.clone();
        let out = change(&mut copy);
        if copy.state != before {
            let active = copy.is_active();
            self.note_menu_aim(bar.stable_id(), copy.state.aim.map(|aim| aim.aim.until()));
            let state = copy.state;
            self.update_component(bar, |bar, _| bar.state = state)?;
            if let Some(document) = self.world().document_of(bar.stable_id()) {
                self.note_menu_bar(document, bar.stable_id(), active);
            }
        }
        Ok(out)
    }

    /// Choose `id`: the queue the native menu reports to, and the bar's event.
    fn activate_menu_bar_item(
        &mut self,
        bar: Entity<AppMenuBar>,
        id: u32,
    ) -> Result<(), FrameworkError> {
        nana_ui_core::push_menu_activation(id);
        self.update_component(bar, |_, view| view.emit(MenuBarActivated { id }))
    }

    fn menu_bar_pointer(
        &mut self,
        bar: Entity<AppMenuBar>,
        node: StableNodeId,
        pointer: NodePointer,
        phase: Phase,
    ) -> Result<bool, FrameworkError> {
        if phase == Phase::Press && pointer.button != 0 {
            // another button outside an open menu still closes it
            let open = self.read(bar, AppMenuBar::is_open)?;
            if open {
                self.drive_menu_bar(bar, |bar| {
                    bar.close();
                    bar.state.swallow = true;
                })?;
            }
            return Ok(open);
        }
        let Some(origin) = self.world().component_layout_box(node) else {
            return Ok(false);
        };
        let at = (pointer.x + origin.x, pointer.y + origin.y);
        let Some(parts) = self.read(bar, |bar| bar.parts.clone())? else {
            return Ok(false);
        };
        let hit = self.menu_bar_hit(&parts, at);
        match phase {
            Phase::Move => {
                let panels = parts
                    .panels
                    .iter()
                    .map(|panel| self.world().component_layout_box(panel.panel))
                    .collect::<Vec<_>>();
                let now = self.menu_aim_clock();
                let aim = pointer.pointer_type != nana_ui_input::PointerType::Touch;
                self.drive_menu_bar(bar, |bar| bar.hover(hit, at, now, aim, &panels))?;
                Ok(true)
            }
            Phase::Press => self.drive_menu_bar(bar, |bar| {
                let taken = bar.press(hit);
                bar.state.last = Some(at);
                taken
            }),
            Phase::Release => {
                if let Some(id) = self.drive_menu_bar(bar, |bar| bar.release(hit))? {
                    self.activate_menu_bar_item(bar, id)?;
                }
                Ok(true)
            }
        }
    }

    fn menu_bar_hit(&self, parts: &MenuBarParts, (x, y): (f32, f32)) -> MenuHit {
        let world = self.world();
        let contains = |id: StableNodeId| {
            world
                .component_layout_box(id)
                .is_some_and(|b| b.contains(x, y))
        };
        for (level, panel) in parts.panels.iter().enumerate().rev() {
            if !contains(panel.panel) {
                continue;
            }
            return panel
                .rows
                .iter()
                .position(|row| row.texts.is_some() && contains(row.row))
                .map_or(MenuHit::Panel(level), |index| MenuHit::Row(level, index));
        }
        parts
            .titles
            .iter()
            .position(|(title, _)| contains(*title))
            .map_or(MenuHit::Outside, MenuHit::Title)
    }

    /// A key for `bar`, which is active. Answers whether the bar took it.
    pub(crate) fn menu_bar_key(
        &mut self,
        bar: Entity<AppMenuBar>,
        key: &str,
    ) -> Result<bool, FrameworkError> {
        match self.drive_menu_bar(bar, |bar| bar.key(key))? {
            MenuKey::Ignored => Ok(false),
            MenuKey::Consumed => Ok(true),
            MenuKey::Activated(id) => {
                self.activate_menu_bar_item(bar, id)?;
                Ok(true)
            }
        }
    }

    /// Give `bar` the keyboard, or take it back: Alt or F10.
    pub(crate) fn toggle_menu_bar_keyboard(
        &mut self,
        bar: Entity<AppMenuBar>,
    ) -> Result<bool, FrameworkError> {
        self.drive_menu_bar(bar, |bar| {
            if bar.is_active() {
                bar.close();
            } else if !bar.bar.menus.is_empty() {
                bar.state.title = Some(0);
                bar.state.keyboard = true;
            }
        })?;
        Ok(true)
    }

    /// Build or update the bar's titles, its open menus and the layer that
    /// takes the pointer while one is open.
    pub fn assemble_app_menu_bar(
        &mut self,
        bar: Entity<AppMenuBar>,
    ) -> Result<bool, FrameworkError> {
        let document = self.document_of_node(bar.stable_id())?;
        let snapshot = self.read(bar, Clone::clone)?;
        let metrics = self.world().theme_metrics();
        let row = ControlSize::Small.height_in(metrics);
        let alive = snapshot
            .parts
            .clone()
            .filter(|parts| self.world().contains(parts.scrim));
        let created = alive.is_none();
        let mut parts = match alive {
            Some(parts) => parts,
            None => {
                let scrim = self.create_detached_component(
                    document,
                    MenuBarScrim {
                        bar: bar.stable_id(),
                        open: false,
                    },
                )?;
                MenuBarParts {
                    titles: Vec::new(),
                    panels: Vec::new(),
                    scrim: scrim.stable_id(),
                }
            }
        };
        let mut gone = Vec::new();

        // titles
        let menus = &snapshot.bar.menus;
        if parts.titles.len() != menus.len() {
            gone.extend(parts.titles.drain(..).map(|(title, _)| title));
            for _ in menus {
                let text = self.create_detached_component(document, Text::new(""))?;
                let title =
                    self.create_detached_component(document, MenuBarBox::new("menu-title"))?;
                self.append_children(title.stable_id(), &[text.stable_id()])?;
                parts.titles.push((title.stable_id(), text.stable_id()));
            }
        }
        let state = &snapshot.state;
        for (index, (menu, &(title, text))) in menus.iter().zip(&parts.titles).enumerate() {
            let hot = state.title == Some(index);
            let open = hot && !state.levels.is_empty();
            let label = menu.title.clone();
            self.update_component(Entity::<Text>::from_stable_id(text), |text, _| {
                text.value = label;
                text.decorative = true;
                text.style = text_style(
                    ControlSize::Small.text_size(),
                    None,
                    SemanticColorRole::Text,
                );
            })?;
            let name = Arc::from(menu.title.as_str());
            self.update_component(Entity::<MenuBarBox>::from_stable_id(title), |title, _| {
                title.style = title_style(hot, open);
                title.accessibility = AccessibilityState {
                    role: AccessibilityRole::MenuItem,
                    label: Some(name),
                    has_popup: true,
                    selected: Some(open),
                    ..AccessibilityState::default()
                };
            })?;
        }

        // the open menus: keep each panel whose path did not change
        let open = snapshot.open_menus();
        let mut keys = Vec::with_capacity(open.len());
        if let Some(title) = state.title {
            let mut key = vec![title];
            for level in 0..open.len() {
                keys.push(key.clone());
                if let Some(sub) = state.levels[level].sub {
                    key.push(sub);
                }
            }
        }
        let keep = parts
            .panels
            .iter()
            .zip(&keys)
            .take_while(|(panel, key)| panel.key == **key)
            .count();
        gone.extend(parts.panels.drain(keep..).map(|panel| panel.panel));
        for (level, menu) in open.iter().enumerate().skip(keep) {
            let panel = self.create_detached_component(document, MenuBarBox::new("menu"))?;
            let mut rows = Vec::with_capacity(menu.entries.len());
            for entry in &menu.entries {
                if matches!(entry, MenuEntry::Separator) {
                    let mut separator = MenuBarBox::new("menu-separator");
                    separator.style = separator_style();
                    separator.accessibility.role = AccessibilityRole::Separator;
                    let separator = self.create_detached_component(document, separator)?;
                    rows.push(MenuRowParts {
                        row: separator.stable_id(),
                        texts: None,
                    });
                    continue;
                }
                let row = self.create_detached_component(document, MenuBarBox::new("menu-row"))?;
                let mut texts = [row.stable_id(); 3];
                for slot in &mut texts {
                    *slot = self
                        .create_detached_component(document, Text::new(""))?
                        .stable_id();
                }
                self.append_children(row.stable_id(), &texts)?;
                rows.push(MenuRowParts {
                    row: row.stable_id(),
                    texts: Some(texts),
                });
            }
            let ids = rows.iter().map(|row| row.row).collect::<Vec<_>>();
            self.append_children(panel.stable_id(), &ids)?;
            parts.panels.push(MenuPanelParts {
                key: keys[level].clone(),
                panel: panel.stable_id(),
                rows,
            });
        }

        // where each menu goes: under its title, beside its row, inside the window
        let viewport = self.world().document_viewport(document);
        let bar_box = self.world().component_layout_box(bar.stable_id());
        let mut rects: Vec<LayoutBox> = Vec::with_capacity(open.len());
        for (level, menu) in open.iter().enumerate() {
            let width = menu_width(&menu.entries, snapshot.shortcut_notation, metrics);
            let height = menu_height(&menu.entries, row);
            let (mut x, mut y) = match level.checked_sub(1) {
                None => {
                    let title = state
                        .title
                        .and_then(|title| parts.titles.get(title))
                        .and_then(|(title, _)| self.world().component_layout_box(*title))
                        .unwrap_or_default();
                    let bottom = bar_box.map_or(title.y + title.height, |b| b.y + b.height);
                    (title.x, bottom)
                }
                Some(parent) => {
                    let outer = rects[parent];
                    let sub = state.levels[parent].sub.unwrap_or(0);
                    let inset = MENU_SURFACE_PADDING + nana_ui_core::HAIRLINE;
                    let top = outer.y + entry_top(&open[parent].entries, sub, row) - inset;
                    let right = outer.x + outer.width - MENU_SURFACE_PADDING;
                    let flip = viewport.is_some_and(|v| right + width > v.width);
                    let x = if flip {
                        outer.x - width + MENU_SURFACE_PADDING
                    } else {
                        right
                    };
                    (x, top)
                }
            };
            if let Some(viewport) = viewport {
                x = x.min(viewport.width - width).max(0.0);
                y = y.min(viewport.height - height).max(0.0);
            }
            rects.push(LayoutBox {
                x,
                y,
                width,
                height,
            });
        }

        for (level, (menu, panel)) in open.iter().zip(&parts.panels).enumerate() {
            let rect = rects[level];
            let name = Arc::from(menu.title.as_str());
            self.update_component(
                Entity::<MenuBarBox>::from_stable_id(panel.panel),
                |panel, _| {
                    panel.style = panel_style(rect);
                    panel.accessibility = AccessibilityState {
                        role: AccessibilityRole::Menu,
                        label: Some(name),
                        ..AccessibilityState::default()
                    };
                },
            )?;
            let levels = &state.levels[level];
            for (index, (entry, parts)) in menu.entries.iter().zip(&panel.rows).enumerate() {
                let Some([mark, label, hint]) = parts.texts else {
                    continue;
                };
                let hot = levels.hot == Some(index) || levels.sub == Some(index);
                let (text, shortcut, enabled, checked, submenu) = match entry {
                    MenuEntry::Item {
                        label,
                        shortcut,
                        enabled,
                        checked,
                        ..
                    } => (
                        label.clone(),
                        shortcut
                            .as_ref()
                            .map(|shortcut| snapshot.shortcut_notation.label(shortcut))
                            .unwrap_or_default(),
                        *enabled,
                        *checked,
                        false,
                    ),
                    MenuEntry::Submenu(menu) => {
                        (menu.title.clone(), SUBMENU.to_owned(), true, false, true)
                    }
                    MenuEntry::Separator => continue,
                };
                let ink = if enabled {
                    SemanticColorRole::Text
                } else {
                    SemanticColorRole::Faint
                };
                let quiet = if enabled {
                    SemanticColorRole::Muted
                } else {
                    SemanticColorRole::Faint
                };
                let small = ControlSize::Small.text_size();
                self.update_component(Entity::<Text>::from_stable_id(mark), |mark, _| {
                    mark.value = if checked { CHECK } else { "" }.to_owned();
                    mark.decorative = true;
                    mark.style = text_style(small, Some(LengthSpec::Px(MARK_WIDTH)), ink);
                    mark.style.text_horizontal_alignment = TextHorizontalAlignment::Center;
                })?;
                let spoken = Arc::from(text.as_str());
                self.update_component(Entity::<Text>::from_stable_id(label), |label, _| {
                    label.value = text;
                    label.decorative = true;
                    label.style = text_style(small, Some(LengthSpec::Fill), ink);
                    Arc::make_mut(&mut label.style.layout).flex_shrink = Some(1.0);
                })?;
                let description =
                    (!shortcut.is_empty() && !submenu).then(|| Arc::from(shortcut.as_str()));
                self.update_component(Entity::<Text>::from_stable_id(hint), |hint, _| {
                    hint.value = shortcut;
                    hint.decorative = true;
                    hint.style = text_style(nana_ui_core::type_scale::HINT, None, quiet);
                })?;
                self.update_component(
                    Entity::<MenuBarBox>::from_stable_id(parts.row),
                    |row, _| {
                        row.style = row_style(hot);
                        row.accessibility = AccessibilityState {
                            role: if checked {
                                AccessibilityRole::Checkbox
                            } else {
                                AccessibilityRole::MenuItem
                            },
                            label: Some(spoken),
                            description,
                            disabled: !enabled,
                            checked: checked.then_some(true),
                            selected: Some(hot),
                            has_popup: submenu,
                            ..AccessibilityState::default()
                        };
                    },
                )?;
            }
        }

        let scrim_open = !open.is_empty() || state.swallow;
        self.update_component(
            Entity::<MenuBarScrim>::from_stable_id(parts.scrim),
            |scrim, _| scrim.open = scrim_open,
        )?;

        if !gone.is_empty() {
            let mut mutations = MutationQueue::new();
            for id in gone {
                mutations.despawn_subtree(id);
            }
            self.commit_mutations(mutations)?;
        }
        if snapshot.parts.as_ref() != Some(&parts) {
            let stored = parts.clone();
            self.update_component(bar, |bar, _| bar.parts = Some(stored))?;
        }
        let children = parts
            .titles
            .iter()
            .map(|(title, _)| *title)
            .chain(parts.panels.iter().map(|panel| panel.panel))
            .chain(std::iter::once(parts.scrim))
            .collect::<Vec<_>>();
        self.append_children(bar.stable_id(), &children)?;
        Ok(created)
    }

    /// The id of `document`'s active bar, if it still is one.
    pub(crate) fn active_menu_bar(&self, id: StableNodeId) -> Option<Entity<AppMenuBar>> {
        self.view_entity::<AppMenuBar>(id)
            .filter(|bar| self.read(*bar, AppMenuBar::is_active).unwrap_or(false))
    }

    /// The menu bar Alt or F10 give the keyboard to: the active one, or the
    /// first in `document`.
    pub(crate) fn menu_bar_for_keys(
        &self,
        document: DocumentId,
        active: Option<StableNodeId>,
        candidates: impl Iterator<Item = StableNodeId>,
    ) -> Option<Entity<AppMenuBar>> {
        active.and_then(|id| self.active_menu_bar(id)).or_else(|| {
            candidates
                .filter(|id| {
                    self.world().document_of(*id) == Some(document) && self.world().is_mounted(*id)
                })
                .min()
                .and_then(|id| self.view_entity::<AppMenuBar>(id))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> AppMenuBar {
        AppMenuBar::new(MenuBar::new([
            Menu::new(
                "File",
                [
                    MenuEntry::item(1, "Open").shortcut(MenuShortcut::primary("o")),
                    MenuEntry::Separator,
                    MenuEntry::Submenu(Menu::new(
                        "Recent",
                        [MenuEntry::item(2, "a.vlive"), MenuEntry::item(3, "b.vlive")],
                    )),
                    MenuEntry::item(4, "Quit").enabled(false),
                ],
            ),
            Menu::new("Edit", [MenuEntry::item(5, "Undo")]),
        ]))
    }

    #[test]
    fn keys_walk_titles_rows_and_submenus() {
        let mut bar = bar();
        bar.state.title = Some(0);
        bar.state.keyboard = true;
        assert!(matches!(bar.key("ArrowDown"), MenuKey::Consumed));
        assert_eq!(bar.open_titles(), ["File"]);
        assert_eq!(bar.highlighted(), Some("Open"));
        bar.key("ArrowDown");
        assert_eq!(bar.highlighted(), Some("Recent"), "skips the separator");
        bar.key("ArrowRight");
        assert_eq!(bar.open_titles(), ["File", "Recent"]);
        assert_eq!(bar.highlighted(), Some("a.vlive"));
        bar.key("ArrowUp");
        assert_eq!(bar.highlighted(), Some("b.vlive"), "wraps");
        bar.key("Escape");
        assert_eq!(bar.open_titles(), ["File"]);
        bar.key("ArrowRight");
        bar.key("ArrowLeft");
        assert_eq!(bar.open_titles(), ["File"]);
        bar.key("End");
        assert_eq!(bar.highlighted(), Some("Quit"));
        assert!(
            matches!(bar.key("Enter"), MenuKey::Consumed),
            "a disabled item does nothing"
        );
        bar.key("ArrowRight");
        assert_eq!(bar.open_titles(), ["Edit"]);
        assert!(matches!(bar.key("Enter"), MenuKey::Activated(5)));
        assert!(!bar.is_active());
    }

    #[test]
    fn a_new_model_closes_what_was_open() {
        let mut bar = bar();
        bar.open_menu(0, false, false);
        bar.set_bar(bar.bar.clone());
        assert!(bar.is_open(), "the same menus keep what is open");
        let mut fewer = bar.bar.clone();
        fewer.menus.pop();
        bar.set_bar(fewer);
        assert!(!bar.is_open());
    }

    #[test]
    fn a_row_crossed_toward_a_submenu_waits_until_the_pointer_rests() {
        let panels = [
            Some(LayoutBox {
                x: 0.0,
                y: 20.0,
                width: 160.0,
                height: 120.0,
            }),
            Some(LayoutBox {
                x: 160.0,
                y: 60.0,
                width: 140.0,
                height: 70.0,
            }),
        ];
        let ms = Duration::from_millis;
        let (recent, toward) = ((80.0, 70.0), (150.0, 95.0));
        let opened = || {
            let mut bar = bar();
            bar.open_menu(0, false, false);
            bar.hover(MenuHit::Row(0, 2), recent, ms(0), true, &panels);
            assert_eq!(bar.open_titles(), ["File", "Recent"]);
            bar
        };

        let mut bar = opened();
        bar.hover(MenuHit::Row(0, 3), toward, ms(16), true, &panels);
        assert_eq!(bar.open_titles(), ["File", "Recent"], "on the way");
        bar.settle_aim(ms(200));
        assert_eq!(
            bar.open_titles(),
            ["File", "Recent"],
            "not rested long enough"
        );
        bar.settle_aim(ms(16) + crate::submenu_aim::AIM_GRACE);
        assert_eq!(bar.open_titles(), ["File"]);
        assert_eq!(bar.highlighted(), Some("Quit"), "the row it rested on");

        let mut bar = opened();
        bar.hover(MenuHit::Row(0, 3), toward, ms(16), false, &panels);
        assert_eq!(
            bar.highlighted(),
            Some("Quit"),
            "a finger takes the row at once"
        );

        let mut bar = opened();
        bar.hover(MenuHit::Row(0, 3), toward, ms(16), true, &panels);
        bar.hover(MenuHit::Row(1, 0), (170.0, 70.0), ms(32), true, &panels);
        assert_eq!(bar.highlighted(), Some("a.vlive"));
        assert_eq!(bar.state.aim, None, "reaching the submenu ends the aim");
    }

    #[test]
    fn shortcut_text_follows_the_platform() {
        let shortcut = MenuShortcut::primary("s").shift();
        assert_eq!(ShortcutNotation::Symbols.label(&shortcut), "⇧⌘S");
        assert_eq!(ShortcutNotation::Names.label(&shortcut), "Ctrl+Shift+S");
        let host = if cfg!(target_os = "macos") {
            "⇧⌘S"
        } else {
            "Ctrl+Shift+S"
        };
        assert_eq!(menu_shortcut_label(&shortcut), host);
    }
}
