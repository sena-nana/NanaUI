# NanaUI 的 `accesskit_windows` fork

[English](README.md)

`third_party/accesskit/windows` 存放 NanaUI 基于 `accesskit_windows` 0.35.1 打补丁的版本，发布到 crates.io 时包名为 `nana-accesskit-windows`（`0.35.1-nana.1`）；`accesskit`、`accesskit_macos`、`accesskit_unix` 和 `accesskit_android` 均直接使用 crates.io 官方版本。

## 相对上游 `accesskit_windows` 0.35.1 的改动

### 上游 PR #816
- 在 `src/node.rs` 中补充本地化 `Switch` 控件类型（未设置 `role_description` 时 `UIA_LocalizedControlTypePropertyId` 默认返回 `"switch"`）与 `ScrollPattern`（`IScrollProvider`）支持。

### 稳定性与无 Panic 加固
- `src/adapter.rs`：`QueuedEvents::raise()` 忽略 `UiaRaiseAutomationEvent` 与 `UiaRaiseAutomationPropertyChangedEvent` 返回的瞬态、断连或窗口销毁期 COM 错误，避免在 `panic = "abort"` 发布构建中崩溃。
- `src/util.rs`、`src/context.rs`、`src/node.rs`、`src/text.rs`：`client_top_left()` 改为返回 `Result<Point>`，将 `ClientToScreen` 失败映射为 `UIA_E_ELEMENTNOTAVAILABLE` 并在 `BoundingRectangle`、`ElementProviderFromPoint`、`RangeFromPoint` 和 `GetBoundingRectangles` 中用 `?` 传播；同时消除 `IValueProvider::SetValue` UTF-16 解码、`UiaGetReservedNotSupportedValue`、`UiaGetReservedMixedAttributeValue` 及 `ITextRangeProvider` 端点/节点查询中的可失败 `.unwrap()`。
- `src/subclass.rs`：将原生 `SetWindowLongPtrW(GWLP_WNDPROC)` + `SetPropW`/`GetPropW`/`RemovePropW` 替换为 ComCtl32 的 `SetWindowSubclass` / `GetWindowSubclass` / `RemoveWindowSubclass` / `DefSubclassProc`（启用 `windows` 的 `Win32_UI_Shell` 特性）及基于 `Rc` 的防重入生命周期管理，保证与 `nana-window` 子类化链安全叠加，并在 `WM_NCDESTROY` / 卸载时安全断开 UIA 提供程序（`UiaDisconnectProvider` + `UiaReturnRawElementProvider(hwnd, 0, 0, None)`）。
- `src/adapter.rs`、`src/context.rs`：在 `Placeholder -> Active` 树替换时清空过期的 `platform_nodes`。

### Windows UIA 热路径性能优化
- `src/adapter.rs`：在 `update_if_active` 与 `update_window_focus_state` 中检查 `UiaClientsAreListening()`。当没有 UIA 客户端监听事件时，仍会更新树并从 `context.platform_nodes` 中清理被移除节点 ID，但跳过属性 diff、UTF-16/BSTR/VARIANT 转换、`PlatformNode` 创建和 `QueuedEvent` 生成。
- `src/adapter.rs`：使用独立的 `text_changed_events` 缓冲区在末尾一次性前置 `UIA_Text_TextChangedEventId`，替代 `self.queue.insert(0, ...)` 的 $O(N^2)$ 挪动；并将 `node_updated` 中 live-region 的 `name()` 重算限定在 `live != Live::Off` 分支内。
- `src/node.rs`、`src/util.rs`：对比 `UIA_ControllerForPropertyId` 时先比较 `FullNodeId` 迭代器，仅在列表确实变化时才创建 `PlatformNode` 与 `Vec<IUnknown>`（同时修复新增尾部受控节点被遗漏的问题）；`description` 直接借用为 `StrWrapper`；对 `UIA_NamePropertyId` 和 `UIA_AriaPropertiesPropertyId` 的 diff 增加底层字段快速短路检查；`AriaProperties` 写入整数属性时避免中间 `String` 堆分配。

### 补齐 UIA 语义映射
- `aria-invalid`：暴露 `UIA_IsDataValidForFormPropertyId`（支持属性变更事件），并在 `UIA_AriaPropertiesPropertyId` 中输出 `invalid=true|grammar|spelling`。
- 地标（Landmark）：按 W3C Core-AAM / Chromium UIA 映射，为 `Main`、`Navigation`、`Search`、`Form`、`Banner`/`Header`、`ContentInfo`/`Footer`、`Complementary` 和 `Region` 暴露 `UIA_LandmarkTypePropertyId` 与 `UIA_LocalizedLandmarkTypePropertyId`。
- `aria-level`：修复 `aria_properties()` 中 `level` 少 1 的问题，改为 1-based，与 `UIA_LevelPropertyId` 和 WAI-ARIA 保持一致。
- 菜单与工具提示生命周期事件：在节点新增、可见性/角色切换和移除时，为 `Role::Menu` / `Role::MenuListPopup` 发送 `UIA_MenuOpenedEventId` / `UIA_MenuClosedEventId`，为 `Role::Tooltip` 发送 `UIA_ToolTipOpenedEventId` / `UIA_ToolTipClosedEventId`。

待上述改动合入上游并发布新版本后，即可切回上游 `accesskit_windows` 并移除此目录。
