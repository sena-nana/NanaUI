# AppMenuBar

`AppMenuBar` 把应用的 `MenuBar` 模型画在应用自己的界面里，例如自定义标题栏或窗口顶部。它跟随主题配色。原生菜单栏用的是同一份模型（见 [窗口](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/window.md#菜单栏)）。它没有模板标签，用 `widget(AppMenuBar::new(模型))` 放进视图。

## 基本用法

```rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{AppMenuBar, MenuBarActivated};
use nana_ui::{Menu, MenuBar, MenuEntry, MenuShortcut};

let bar = MenuBar::new([
    Menu::new("文件", [
        MenuEntry::item(1, "打开…").shortcut(MenuShortcut::primary("o")),
        MenuEntry::Separator,
        MenuEntry::Submenu(Menu::new("最近打开", [MenuEntry::item(2, "a.vlive")])),
    ]),
    Menu::new("视图", [MenuEntry::item(3, "网格").checked(true)]),
]);
widget(AppMenuBar::new(bar)).on(|e: &MenuBarActivated| { /* e.id */ })
```

选中项的 `id` 有两条出口：

- 进入原生菜单使用的同一个队列，用 `take_menu_activations()` 取出。
- 菜单栏同时发出 `MenuBarActivated { id }`。

若使用单一动作注册表，可用 `MenuActionMap`（`menu_entry`、`sync_menu_bar`）把数字菜单项 `id` 映射到 `ActionId`，并把启用状态、动态标签与快捷键提示与 `ActionRegistry` / `Keymap` 保持同步，激活时经 `AppContext::dispatch_menu_action(&registry, &menu_map, menu_id, &context)` 分发。应用消费 `take_menu_activations()` 或监听 `MenuBarActivated` 二选一即可，不要同时分发以免同一菜单点击触发两次。

## 指针

- 在标题上按下，打开它的菜单。再按一次已打开的标题，菜单关闭。
- 有菜单打开时，指针移到别的标题上，菜单直接切换过去。
- 子菜单在所在行的侧边弹出。窗口右侧放不下时翻到左侧，并收进窗口。
- 子菜单打开后，指针斜着穿过父菜单的其他行移向它时，子菜单保持打开。判定用的是指针起点和子菜单近边构成的三角区。
- 在标题上按下后拖到某一项再松开，选中该项。
- 禁用项会高亮，但选不中。

菜单打开期间，一层透明层盖住整个窗口并接收指针。在菜单外按下只关闭菜单，这次按下不会落到下面的控件上。在菜单上按下不会移动焦点。

## 键盘

| 按键 | 效果 |
| --- | --- |
| 单独按下再松开 Alt，或 F10 | 菜单栏获得键盘，第一个标题高亮；再按一次则放开 |
| ← / → | 切换标题；在菜单里，→ 打开高亮的子菜单，← 关闭当前子菜单 |
| ↓ / ↑ | 打开菜单，或在菜单里移动，跳过分隔线，首尾相接 |
| Home / End | 第一项 / 最后一项 |
| Enter / Space | 选中高亮项，或打开高亮的子菜单 |
| Escape | 逐级关闭；最后一级关闭后标题仍高亮，再按一次放开键盘 |

Alt 按下期间按过其他键，算组合键，松开时不打开菜单。不需要 Alt 激活时设 `.alt_key(false)`，F10 照常可用。

菜单打开时，其他按键都由菜单栏吃掉，不会触发应用的快捷键。只是高亮标题、没有打开菜单时，按其他键会放开菜单栏，这个键照常传下去。

## 显示

- 每一项依次显示勾选标记、标签和右侧的快捷键文字。快捷键文字默认按平台习惯显示，例如 `Ctrl+Shift+S`、`⇧⌘S`，格式化函数是 `menu_shortcut_label`。要在所有平台用同一种写法时设 `.shortcut_notation(ShortcutNotation::Names)`（或 `Symbols`）。
- 子菜单在右侧显示 `›`。
- 快捷键文字只用于显示，按键本身仍由应用的键位表处理。

## 和原生菜单的取舍

原生菜单（`WindowHandle::set_menu_bar`）的优缺点：

- macOS 上它在系统菜单条里。这是 Mac 应用应有的位置。
- 它由系统绘制，不跟随主题，也进不了自绘标题栏。
- Linux 上没有原生菜单。

`AppMenuBar` 的特点：

- 和界面其他部分同主题、同密度，可以放进任何位置。
- 代价是没有系统菜单条的集成，例如 macOS 的服务菜单和全局搜索。

两者可以同时使用：macOS 上安装原生菜单，其他平台把 `AppMenuBar` 放进标题栏。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `AppMenuBar::new(bar)` | `MenuBar` | 菜单模型。模型改变时，已打开的菜单会关闭 |
| `.alt_key(bool)` | `bool` | 单独按 Alt 是否激活菜单栏，默认是 |
| `set_bar(bar)` | `MenuBar` | 换成新模型，例如切换语言后的标签、移动了的勾选；模型不同时已打开的菜单关闭。在 `update_component` 里调用 |
| `.style(style)` | `NodeStyle` | 菜单栏自身的样式，默认是一行小号控件高度 |
| `is_open()` / `is_active()` | `bool` | 是否有菜单打开；是否打开或持有键盘 |
| `open_titles()` | `Vec<&str>` | 已打开菜单的标题，从外到内 |
| `highlighted()` | `Option<&str>` | 最内层菜单高亮项的标签 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `MenuBarActivated` | `id: u32` | 选中了一项；同一个 id 也进 `take_menu_activations()` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 标题、菜单面板和行都由菜单栏自己组装。手工用 `create_component` 创建时，要调用 `assemble_app_menu_bar` |

## 参见

[ContextMenu](https://github.com/sena-nana/NanaUI/blob/main/docs/components/context-menu.md) · [ActionMenu](https://github.com/sena-nana/NanaUI/blob/main/docs/components/action-menu.md) · [窗口：菜单栏](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/window.md#菜单栏) · [总览](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md)
