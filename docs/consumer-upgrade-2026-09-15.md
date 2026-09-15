# 消费方升级记录（2026-09-15）

本轮补齐 Issue 51：叠加工具窗可以不出现在任务栏。

## API 变化

### 窗口

- `WindowDescriptor` 新增 `skip_taskbar: bool`，默认 `false`。
- 新增 `WindowCommand::SetSkipTaskbar { id, skip_taskbar }` 与 `WindowHandle::set_skip_taskbar`。
- 新增 `WindowEvent::SkipTaskbarChanged { id, skip_taskbar, result }`：每次请求都回报，`skip_taskbar` 为实际状态。
- Windows 生效；macOS、Linux 返回 `Unsupported`。
- `nana-window` 新增 `set_skip_taskbar` 与 `SkipTaskbarError`，供宿主适配器使用；普通控件仍拿不到窗口句柄。
- Vue JS 接口不变。

## 各应用

| 应用 | 适配 |
| --- | --- |
| 直接构造 `WindowDescriptor` 字面量的应用 | 补 `skip_taskbar: false`，或改用 `..Default::default()` |
| 穷尽匹配 `WindowEvent` 的应用 | 增加 `SkipTaskbarChanged` 分支（或继续用 `_`） |

公开合同见 [窗口](window.md)。
