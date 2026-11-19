# AppMenuBar

`AppMenuBar` draws the application's `MenuBar` model inside the application's own UI, for example in a custom title bar or at the top of the window. It follows the theme colors. The native menu bar uses the same model (see [Windows](../reference/window.md#menu-bar)). It has no template tag; put it into a view with `widget(AppMenuBar::new(model))`.

## Basic usage

```rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{AppMenuBar, MenuBarActivated};
use nana_ui::{Menu, MenuBar, MenuEntry, MenuShortcut};

let bar = MenuBar::new([
    Menu::new("File", [
        MenuEntry::item(1, "Open…").shortcut(MenuShortcut::primary("o")),
        MenuEntry::Separator,
        MenuEntry::Submenu(Menu::new("Open Recent", [MenuEntry::item(2, "a.vlive")])),
    ]),
    Menu::new("View", [MenuEntry::item(3, "Grid").checked(true)]),
]);
widget(AppMenuBar::new(bar)).on(|e: &MenuBarActivated| { /* e.id */ })
```

The `id` of a chosen item has two outlets:

- It enters the same queue the native menu uses, read with `take_menu_activations()`.
- The menu bar also emits `MenuBarActivated { id }`.

When using the unified action registry, use `MenuActionMap` (`menu_entry`, `sync_menu_bar`) to map numeric menu `id`s to `ActionId`s and keep labels, enabled state, and shortcut text in sync with `ActionRegistry` and `Keymap`, then route activations through `AppContext::dispatch_menu_action(&registry, &menu_map, menu_id, &context)`. Consume either `take_menu_activations()` or `MenuBarActivated`—not both—to avoid dispatching the same menu activation twice.

## Pointer

- Pressing on a title opens its menu. Pressing an already open title again closes the menu.
- While a menu is open, moving the pointer onto another title switches straight to that menu.
- A submenu pops out beside its row. When there is no room on the right side of the window it flips to the left and is tucked inside the window.
- After a submenu opens, it stays open while the pointer moves diagonally across other rows of the parent menu toward it. The test uses the triangle formed by the pointer's starting point and the submenu's near edge.
- Pressing on a title, dragging to an item and releasing chooses that item.
- Disabled items highlight but cannot be chosen.

While a menu is open, a transparent layer covers the whole window and receives the pointer. Pressing outside the menu only closes the menu; that press does not reach the control underneath. Pressing on the menu does not move focus.

## Keyboard

| Key | Effect |
| --- | --- |
| Press and release Alt alone, or F10 | The menu bar takes the keyboard and the first title highlights; press again to release |
| ← / → | Switch titles; inside a menu, → opens the highlighted submenu and ← closes the current submenu |
| ↓ / ↑ | Open the menu, or move within it, skipping separators and wrapping around |
| Home / End | First item / last item |
| Enter / Space | Choose the highlighted item, or open the highlighted submenu |
| Escape | Close one level at a time; after the last level closes the title stays highlighted, and pressing again releases the keyboard |

If another key was pressed while Alt was held, it counts as a key combination and releasing Alt does not open the menu. Set `.alt_key(false)` when you do not want Alt activation; F10 still works.

While a menu is open, the menu bar consumes all other keys, and they do not trigger the application's shortcuts. When a title is only highlighted and no menu is open, pressing another key releases the menu bar and that key passes through as usual.

## Display

- Each item shows, in order, a check mark, the label, and shortcut text on the right. Shortcut text follows platform conventions by default, for example `Ctrl+Shift+S` or `⇧⌘S`; the formatting function is `menu_shortcut_label`. To use the same notation on every platform, set `.shortcut_notation(ShortcutNotation::Names)` (or `Symbols`).
- A submenu shows `›` on the right.
- Shortcut text is for display only; the keys themselves are still handled by the application's keymap.

## Choosing between this and the native menu

The trade-offs of the native menu (`WindowHandle::set_menu_bar`):

- On macOS it lives in the system menu bar. That is where a Mac application's menu belongs.
- It is drawn by the system, does not follow the theme, and cannot go into a custom title bar.
- There is no native menu on Linux.

What `AppMenuBar` offers:

- The same theme and density as the rest of the UI, and it can go anywhere.
- The cost is no system menu bar integration, such as the macOS Services menu and global search.

You can use both at once: install the native menu on macOS, and put `AppMenuBar` in the title bar on other platforms.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `AppMenuBar::new(bar)` | `MenuBar` | Menu model. When the model changes, any open menu closes |
| `.alt_key(bool)` | `bool` | Whether pressing Alt alone activates the menu bar; defaults to yes |
| `set_bar(bar)` | `MenuBar` | Switch to a new model, for example labels after a language change or a moved check mark; when the model differs, any open menu closes. Call it in `update_component` |
| `.style(style)` | `NodeStyle` | The menu bar's own style; defaults to one row at small control height |
| `is_open()` / `is_active()` | `bool` | Whether a menu is open; whether it is open or holds the keyboard |
| `open_titles()` | `Vec<&str>` | Titles of open menus, outermost first |
| `highlighted()` | `Option<&str>` | Label of the highlighted item in the innermost menu |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `MenuBarActivated` | `id: u32` | An item was chosen; the same id also goes into `take_menu_activations()` |

## Slots

| Slot | Description |
| --- | --- |
| None | Titles, menu panels and rows are all assembled by the menu bar itself. When creating it by hand with `create_component`, call `assemble_app_menu_bar` |

## See also

[ContextMenu](context-menu.md) · [ActionMenu](action-menu.md) · [Windows: menu bar](../reference/window.md#menu-bar) · [Overview](index.md)
