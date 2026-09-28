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
| `InputEndpoint::new(source, generation, max_events, max_bytes)` 及其校验、计数器（`InputRejection`、`RejectedInput`、`InputEnqueueOutcome`、`InputEndpointCounters`、`InputDeviceCounters`）、`reset` / `len` / `front` | `InputEndpoint::new(max_events, max_bytes)` 只是有界队列：`push` 返回 `Result<(), CanonicalInputEvent>`，满了就把事件交还；generation、顺序、断连由路由校验 |
| `HeadlessHostServices::counters()` | `AppContext::input_counters()` 的 `cursor_updates` / `text_input_updates` |
| 输入类型的 serde 派生 | 无；canonical 负载不是 wire 格式 |
| `nana_ui_vue::ImeEvent` | `nana_ui_vue::NativeComposition`（即 `CompositionInput`） |
| `WindowEvent::{FileHovered, FileDropped, FileHoverCancelled}` | `InputPayload::FileDrag(FileDragInput { kind: FileDragKind::{Hover, Drop, Cancel}, paths, position, modifiers })`，经 `RoutedInput` 送达 |
| `AppContext::dispatch_file_drag`（公开） | 无；路由调用它，程序不再调用 |
| `nana_ui_vue::FileDragEventKind`、`VueHost::dispatch_file_drag` | `nana_ui_vue::FileDragKind`、`VueHost::emit_file_drag_from_runtime`（只发页面事件，不再改 Runtime） |

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
- **文件拖放**由路由交给放置目标，程序不再自己转交；自己实现 `RuntimeProgram` 却没转交的程序，此前放置目标收不到文件，现在收得到。窗口失焦或关闭会结束拖放悬停（放置目标收到 `FileDropEvent::Left`）。
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

## Vue

`VueHost` 的输入方法签名不变（`dispatch_pointer` / `dispatch_pointer_result`、`dispatch_wheel`、`dispatch_keyboard`、`commit_text`、`dispatch_key`、`dispatch_composition`、`dispatch_native_ime`），但它们不再在 Vue 里自己改 Runtime：每个 `VueHost` 有一个输入源，事件先经 Runtime 路由，页面再观察，与原生窗口托管 Vue 时完全一致。由此带来的变化：

- **`preventDefault` 拦不住 Runtime 的默认动作。** 页面在路由之后才收到 `wheel` / `keydown`，滚动与插入文字已经发生；结果里的 `default_prevented` 照样报告。原生窗口里一直如此。
- **焦点与 Tab 顺序是 Runtime 的**：只有注册成部件的元素（Button、Input 等）可聚焦，`tabindex` 与按标签推断的可聚焦性不再参与。焦点因任何按键或指针移动时，页面都会收到 `blur` / `focus`（此前只有 Tab 与按下），`:focus-within` 随之重算。
- **文字只写进 Runtime 认得的文本输入**：注册成 Input / Textarea / NumberInput 部件、或带 `TextInputState` 的节点。原生窗口里给 Vue 输入框打字此前不会改 Runtime 的值（只有 IME 提交会），现在会。
- **组字随焦点离开而取消**：输入法关闭时只有当前聚焦字段里的剩余组字会提交；焦点已经移走时，页面收到 `data` 为空的 `compositionend`，不再把组字补提交到原字段。
- **键盘调 Range 由页面完成**：Runtime 里 Vue 的 Range 只有投影，方向键、PageUp/PageDown、Home/End 走页面的默认动作；此前原生窗口里这一步被关掉了。
- `emit_native_ime_from_runtime(engine, event, applied)` 多了 `applied`：路由是否应用了这次 IME 事件。

## NanaLive 受影响位置

锁定 rev 61c4edc41 之后升级时需要改：

- `crates/nanalive-control/src/native_ui/hotkeys.rs`：`InputEvent::Keyboard` → `InputPayload::Key`；测试里的 `"Code(..)"` 输入改成 W3C 名，保留读取旧存档的前缀剥离。
- `crates/nanalive-control/src/native_ui/mod.rs`：约 12 处 `InputEvent::{Pointer, Wheel, Keyboard}` 匹配改为 `InputPayload`；键盘分支若读 `text`，改读随后的 `InputPayload::Text`。
- `crates/nanalive-control/src/native_ui/mod.rs:4553`、`:4669`：`WindowEvent::FileDropped { paths, .. }` 移到 `input_event`，匹配 `InputPayload::FileDrag(FileDragInput { kind: FileDragKind::Drop, paths, .. })`。
- `crates/nanalive-control/src/native_ui/offscreen.rs`：`route_input` 改用 `HeadlessInput`，pointer hit 从 `InputRouteOutcome::pointer_hit` 读。
- `crates/nanalive-control/src/native_ui/presence_acceptance.rs`：键盘注入改用 `HeadlessInput::press`。
- `crates/nanalive-control/src/settings.rs`、`global_hotkeys.rs`：已兼容无前缀键名，只需更新注释与测试。
- `issue138_acceptance.rs` 注释里的 `RuntimeInputAdapter`。
