# 输入只剩一条路径

宿主把原生事件直接降级成 `CanonicalInputEvent`，由绑定该输入源的 `AppContext` 路由；程序在 `RoutedInput` 上看到的就是这同一个事件。中间形态 `InputEvent`、第二个路由器和 host-service 请求队列都已删除，没有兼容层。说明见 [application-api](application-api.md) 的输入一节与 [input-conformance](input-conformance.md)。

新 crate `nana-ui-input` 持有输入合同（canonical 事件、endpoint、sequencer、`HostServices`）。`nana_ui_platform` 与 `nana_ui` 继续原样重导出这些类型，普通消费者不需要新增依赖。

## 删除

| 旧 | 新 |
| --- | --- |
| `nana_ui_platform::InputEvent` | `CanonicalInputEvent { metadata, payload: InputPayload }` |
| `nana_ui_platform::ImeEvent`、`WindowEvent::Ime` | `InputPayload::Composition(CompositionInput)`，经 `RoutedInput` 送达 |
| `RuntimeInputAdapter`、`InputRouter`、`lower_input_event` | `AppContext::route_input` / `drain_input`；无头场景用 `HeadlessInput` |
| `TextInputRequest`、`TextInputPurpose`（`window.rs`） | `HostServices::set_text_input(Option<&TextInputContext>)`，由原生宿主执行 |
| HostService 请求队列、reservation、`HostServiceBackpressure` | 光标与文本输入是最新值槽位；剪贴板是同步调用 `read_clipboard` / `write_clipboard`，忙时返回 `HostServiceError::Busy` |
| `PresentationCoordinateBridge` 与 `coordinates.rs` | 无；原生宿主的物理→逻辑换算不变 |
| `input_router_counters()` | `AppContext::input_counters() -> InputCounters` |
| `nana_ui_vue::ImeEvent` | `nana_ui_vue::NativeComposition`（即 `CompositionInput`） |

## 事件形状

| 旧 `InputEvent` | 新 `InputPayload` |
| --- | --- |
| `Pointer { pointer_id: u64, .. }` | `Pointer(PointerInput { pointer_id: PointerId, .. })`，其余字段同名 |
| `Wheel { x, y, delta_x, delta_y, line_delta, modifiers }` | `Wheel(WheelInput { pointer_id, x, y, delta_x, delta_y, unit: WheelUnit::{Pixels, Lines}, modifiers })` |
| `Keyboard { pressed, key, code, text, repeat, modifiers }` | `Key(KeyInput { physical: PhysicalKey, logical: LogicalKey, state: KeyState, repeat, modifiers })`，另有一个 `Text(CommittedText { text, key: Some(按键的 sequence) })` |
| `PointerLeft` | `PointerLeave { pointer_id }`，事件自身带设备 |
| 窗口焦点 | `Focus { focused }` |

按键与文本是两个事件：先 `Key`，再 `Text`。被控件处理的按键（快捷键、焦点遍历、提交的输入框）不再插入它的文本；程序想知道一个键打出的字符时读 `Text`，不要从 `Key` 推导。

构造辅助：`PointerInput::mouse(phase, x, y)`、`KeyInput::named(physical, logical, state, modifiers)`、`CommittedText::new(text)`、`KeyInput::is_pressed()`。

## 行为变化

- **物理键名**改为 W3C `code` 字符串：`"KeyA"`、`"Digit8"`、`"F6"`、`"ControlLeft"`。此前是 winit 的 debug 形式 `"Code(KeyA)"`。存下来的旧快捷键需要在读取时去掉 `Code(` 前缀；无法识别的键是 `"Unidentified"`。
- **指针移动与滚轮**在一次事件循环内合并，程序每轮最多看到一次 move（按下、抬起、按键不合并，立即路由）。依赖每个原始 move 的代码要改读最后位置。
- **窗口失焦**只取消该窗口的指针按压与捕获，document 焦点保留；重新获得焦点时宿主重发文本输入状态。只有输入源断开且没有其他已聚焦的输入源时才清焦点。
- **触控抬起**后不再保留悬停。
- **剪贴板快捷键**回到按键链原位置：overlay、应用 `on_key`、终端先于剪贴板处理，终端里的 Ctrl+C 不再被剪贴板吞掉。
- 没有产生字节的终端按键不再报告为已处理，它的文本会正常插入。

## 迁移

程序钩子：

```rust
fn input_event(
    &mut self,
    id: WindowId,
    input: RoutedInput<'_>,
    context: &RuntimeProgramContext<Self::Message>,
) -> Result<RuntimeProgramUpdate, FrameworkError> {
    match &input.event.payload {
        InputPayload::Key(key) if key.is_pressed() && !input.disposition.prevent_default => {
            if key.physical.0 == "F6" { /* ... */ }
        }
        InputPayload::Pointer(pointer) => { /* pointer.x, pointer.y, input.pointer_hit */ }
        _ => {}
    }
    Ok(RuntimeProgramUpdate::default())
}
```

离屏或测试驱动（替代 `RuntimeInputAdapter::default().dispatch_with_shaper(..)`）：

```rust
let mut input = HeadlessInput::bind(&mut context, document);
input.pointer(&mut context, PointerPhase::Down, x, y)?;
input.press(&mut context, KeyInput::named("KeyA", "a", KeyState::Pressed, mods), Some("a"), None)?;
input.route(&mut context, InputPayload::Wheel(wheel))?;
input.advance(Duration::from_millis(16)); // 事件时间戳驱动双击、长按等
```

`HeadlessInput::services()` 记录宿主会显示的光标、文本输入状态与剪贴板内容。自建宿主实现 `HostServices`，按 source 调 `AppContext::bind_input_source`（返回 `Result`），每轮 `drain_input`，窗口关闭时 `unbind_input_source`。

## NanaLive 受影响位置

锁定 rev 61c4edc41 之后升级时需要改：

- `crates/nanalive-control/src/native_ui/hotkeys.rs`：`InputEvent::Keyboard` → `InputPayload::Key`；测试里的 `"Code(..)"` 输入改成 W3C 名，保留读取旧存档的前缀剥离。
- `crates/nanalive-control/src/native_ui/mod.rs`：约 12 处 `InputEvent::{Pointer, Wheel, Keyboard}` 匹配改为 `InputPayload`；键盘分支若读 `text`，改读随后的 `InputPayload::Text`。
- `crates/nanalive-control/src/native_ui/offscreen.rs`：`route_input` 改用 `HeadlessInput`，pointer hit 从 `InputRouteOutcome::pointer_hit` 读。
- `crates/nanalive-control/src/native_ui/presence_acceptance.rs`：键盘注入改用 `HeadlessInput::press`。
- `crates/nanalive-control/src/settings.rs`、`global_hotkeys.rs`：已兼容无前缀键名，只需更新注释与测试。
- `issue138_acceptance.rs` 注释里的 `RuntimeInputAdapter`。
