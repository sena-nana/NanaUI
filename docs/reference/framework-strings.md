# 框架文案

框架自己的控件会说话：标题栏按钮的名称、确认框的「确认」「取消」、媒体条的「播放」「暂停」、内置外观设置页的每一行。这些话都放在一张表里，就是 `nana_ui_core::FrameworkStrings`。默认是中文，应用可以整张换掉，也可以按键换其中几条。

应用自己给控件的文案总是优先。比如 `ConfirmDialog::confirm_label("删除")` 不会被表盖掉。表只管应用没给的那部分。

## 换掉

Rust：

```rust
let mut strings = nana_ui::runtime::FrameworkStrings::default();
strings.set("window.close", "Close");
strings.set("dialog.confirm", "OK");
context.set_framework_strings(strings)?;
```

字段也可以直接写，例如 `strings.window_close = "Close".into()`。

Vue 页面调用宿主接口 `setFrameworkStrings`，传一个「键 → 文字」的对象：

```js
const unknown = __nanaHost.call("setFrameworkStrings", [{ "window.close": "Close" }]);
```

返回值是表里没有的键。

最好在建界面之前设置。已经建好的控件会重新装配、重新投影，改说新的话。

## 模板

带参数的条目在花括号里写参数名，例如 `diff.reject_hunk` 默认是 `拒绝块{hunk}`，`calendar.month` 默认是 `{month}月`。换的时候保留这些参数名：`"Reject hunk {hunk}"`。

## 键

全部键见 `FrameworkStrings::KEYS`，分组如下：

| 前缀 | 控件 |
|---|---|
| `window.*` | 标题栏的最小化、最大化、还原、关闭 |
| `dialog.*` | 确认框的确认、取消 |
| `image_viewer.*` | 图片查看器的关闭、上一张、下一张 |
| `media.*` | 媒体条的播放、暂停、音量、全屏、进度、设置 |
| `find.*` | 查找替换条 |
| `diff.*` | 差异视图 |
| `calendar.*`、`date_picker.*` | 日历热力图、日期选择 |
| `command_palette.*`、`menu.search` | 命令面板、可搜索的菜单 |
| `theme.*`、`material.*`、`settings.*` | 内置外观设置页 |
| 其余 | `chip.remove`、`file_tab.close`、`color_field.label`、`path_field.*`、`browser_view.label`、`dynamic_form.auto` |

`scripts/audit-framework-strings.py` 在 CI 里检查：框架源码里新写的中文界面文字要放进这张表。

## 开关的读法

开关（`Switch`）不再默认写 `role_description`。平台无障碍层会用用户的语言说出“开关”或 “switch”。只有应用想给控件起一个平台没有的类型名时，才自己设 `AccessibilityState::role_description`。
