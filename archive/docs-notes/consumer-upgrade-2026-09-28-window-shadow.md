# Issue #215：WindowShadow 接入平台

此前 `WindowDescriptor::shadow` 只有合同：`Auto` / `Custom` 永远报 `Pending`，`None` 报 `disabled` 但什么也没做，macOS 上 `Custom` 反而去掉了原生阴影，透明窗口没有阴影。现在由宿主真正应用，结果写回 `presentation().shadow()`。说明见 [窗口](../../docs/reference/window.md) 的 WindowShadow 一节。

## 行为变化

- **透明窗口默认有阴影。** `Auto` 在透明窗口上创建 companion，阴影跟随窗口画的根卡片（找不到时跟随整个客户区）。卡片自己画了 UiScene 阴影、或是点击穿透覆盖层的窗口，设 `WindowShadow::None`。
- `Custom(WindowShape)` 在所有窗口上都是 companion（平台阴影不接受自定义样式）；macOS 不再因为 `Custom` 去掉阴影。
- 有 DWM 边框的 Windows 窗口请求 `None` 时报 `Native` + `DisableUnsupported`（DWM 边框阴影无法单独去掉）。
- Linux 等报 `Unsupported` / `CompositorManaged`。

## API

- 新增 `WindowCommand::SetShadow { id, shadow }`、`WindowVisualShape`、`WindowShadowWork`，`WindowShadowFallback` 新增 `CustomStyleApproximated`、`CompositorManaged`、`DisableUnsupported` 并标为 `#[non_exhaustive]`（穷举 match 需加通配分支）。
- 删除 `WindowShadowCapabilities`：它与宿主的决策是两套 authority，且没有消费者。
- `nana_window::shadow` 提供平台层 `WindowShadowState`（宿主内部使用）。
- Vue：`Nana.windows.create({ shadow })`，见 [Vue](../../docs/reference/vue.md)。

## NanaLive

`nanalive-control` 的工具窗口（`native_ui/tools/mod.rs`）与使用准备窗口（`native_ui/launch/onboarding.rs`）是透明窗口，升级到此版本后默认获得 companion 阴影；若它们的卡片已有自己的阴影，改设 `WindowShadow::None`。

## 未验证

Windows companion（DComp 九宫格、`WS_EX_LAYERED` 与 `WS_EX_NOREDIRECTIONBITMAP` 组合、跨进程点击穿透、拖动时的跟随延迟）只经过交叉编译检查，需要 Windows 真机验收。
