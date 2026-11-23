# 框架内置文案表（`FrameworkStrings`）

框架内置控件自带一批默认界面与无障碍文案：例如标题栏窗口按钮的无障碍名称、确认对话框的「确认 / 取消」、媒体控制条的「播放 / 暂停 / 全屏」、查找替换条的占位符，以及内置外观设置页的各项标签。这些文案集中定义在 `nana_ui_core::FrameworkStrings` 一张表中（默认中文），应用可以整表替换，也可以按键名覆盖其中的任意几条。

应用在具体控件上显式传入的文案始终优先（例如 `ConfirmDialog::confirm_label("删除")` 不会被表中的默认值覆盖）；`FrameworkStrings` 仅作为应用未显式指定文案时的回退来源。

## 如何替换文案

### Rust 路径

```rust
let mut strings = nana_ui::runtime::FrameworkStrings::default();
strings.set("window.close", "Close");
strings.set("dialog.confirm", "OK");
context.set_framework_strings(strings)?;
```

除按键名调用 `.set(key, value)` 外，也可以直接读写结构体字段（如 `strings.window_close = "Close".into()`）。建议在挂载视图前设置；若在运行期动态切换语言并调用 `set_framework_strings`，已挂载的内置复合控件会自动重新装配并重新投影为新文案。

## 带参数的模板键与键名前缀

部分条目包含花括号占位符（如 `diff.reject_hunk` 默认为 `"拒绝块{hunk}"`，`calendar.month` 默认为 `"{month}月"`，`toast.percent` 默认为 `"{percent}%"`）。替换多语言译文时请保留同名花括号参数（例如 `"Reject hunk {hunk}"`）。

全量键名清单可通过 `FrameworkStrings::KEYS` 枚举查看，按前缀分组如下：

| 键名前缀 | 对应的内置控件与表面 |
| --- | --- |
| `window.*` | 自绘标题栏的最小化、最大化、还原、关闭按钮名称 |
| `dialog.*` | 确认对话框（`ConfirmDialog`）的默认确认与取消按钮文案 |
| `toast.*` | 通知提示（`Toast`）的关闭按钮（`toast.close`）、进度百分比（`toast.percent`）与通知栈区域名（`toast.region`） |
| `image_viewer.*` | 图片查看器（`ImageViewer`）的关闭、上一张、下一张按钮名称 |
| `media.*` | 媒体传输条（`MediaTransportBar`）的播放、暂停、音量、全屏、进度、设置按钮名称 |
| `find.*` | 查找与替换工具条（`FindReplaceBar`）的标题、输入框占位符与操作按钮文案 |
| `diff.*` | 差异对比视图的操作按钮文案 |
| `calendar.*`、`date_picker.*` | 日历热力图（`CalendarHeatmap`）与日期选择器（`DatePicker`）的月份/星期格式 |
| `command_palette.*`、`menu.search` | 命令面板（`CommandPalette`）与可搜索菜单的搜索框占位符 |
| `diagram.*`、`markdown.alert.*` | `NativeMarkdown` 里图表的无障碍名称与摘要，以及 GFM 提示块（`> [!NOTE]` 等五种）的标题 |
| `theme.*`、`material.*`、`settings.*` | 内置外观设置页的主题、材质与配置项标签 |
| 其余独立键 | `chip.remove`、`range_field.reset`、`file_tab.close`、`color_field.label`、`path_field.*`、`browser_view.label`、`dynamic_form.auto` |

CI 中的 `scripts/audit-framework-strings.py` 会扫描框架源码：任何新增在内置控件代码里的中文界面字面量都必须收入 `FrameworkStrings` 表，不得散落在组件实现中。

## 开关的读法

开关组件（`Switch`）默认不再写入硬编码的 `role_description`——操作系统的平台无障碍层会自动按用户当前的系统语言朗读“开关”或 “switch”。只有当应用希望为某个自定义控件赋予平台标准角色之外的特定类型读音时，才需要显式设置 `AccessibilityState::role_description`。
