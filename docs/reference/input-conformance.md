# 输入验收矩阵（Input Conformance）

本文列出输入合同（Issue #243）在仓库内可重复执行的自动化回归命令，以及必须由真实平台窗口或物理设备完成的边界验收。离屏与无头测试不能替代原生窗口、真实系统输入法（IME）、WebView、XR 或远程网络环境的实机证据。

## 仓内自动化合同测试

窗口宿主与 Android 控制槽均在边界处将平台原生事件降维为 `CanonicalInputEvent`，统一送入 `AppContext::route_input`（无窗口测试环境使用 `HeadlessInput`）。诊断计数登记在 `nana-diagnostics` 的 `runtime.input.*`（ID 20–37，已退役的旧 ID 永不复用）。

| 验收合同 | 验证命令 |
| --- | --- |
| 有界队列容量、`PointerMove` / `Wheel` 相邻事件合并、端点 `generation` 推进与重计数 | `cargo test -p nana-ui-input --lib --locked` |
| 源绑定、断连清理、失焦、指针捕获、剪贴板、文件拖放（`InputPayload::FileDrag`）、空端点 drain 为 0；按键策略同时观测物理键、逻辑键、repeat 与统一修饰键 | `cargo test -p nana-ui-runtime --all-features --lib framework::input --locked` |
| `Keymap` 按逻辑键匹配（四位修饰键与 `InputModifiers` 严格一致，按键释放不误触发） | `cargo test -p nana-ui-runtime --all-features --lib key_layers --locked` |
| 未捕获指针移动每次仅 1 次命中查询、捕获期间 0 次查询，稳态移动零堆分配；连续 1,000 次同位置或跨行 `PointerMove` 下 layout frontier 与 scene extract 均为空 | `cargo test -p nana-ui-runtime --all-features --test perf_contracts input_alloc --locked` |
| 单文档命中测试跟随呈现层（presentation）的平移、缩放、3D 投影与裁剪链 | `cargo test -p nana-ui-runtime --all-features --lib world::hit_test --locked` |
| 宿主以 240 Hz 轮询而静态窗口保持 `FrameDemand::OnDemand` 时，240 次轮询期间 `present` 次数为 0 | `cargo test -p nana-ui --features hosted --lib scene_host::schedule::tests::a_static_window_polled_at_240hz_presents_nothing --locked` |

注：测试中的 `1000 Hz` 指合成 `PointerMove` 事件流频率，`240 Hz` 指宿主事件循环轮询频率（验证的是工作计数而非物理显示器刷新率）。窗口中可合并的指针移动与滚轮事件会留到当前事件回合末尾再统一路由，仅当本次路由实际标脏了文档工作时才请求重绘帧。墙钟耗时基准可使用 `nana-input-benchmark`（`crates/nana-ui-devtools/src/bin/nana-input-benchmark.rs`）测量。

## 跨宿主与外部环境边界

| 宿主适配器 | 仓内支持现状与外部边界 |
| --- | --- |
| **桌面原生窗口（Window）** | `scene_host` 将 winit 事件降维为 `CanonicalInputEvent` 并完成端到端路由 |
| **嵌套 NanaUI（Nested）** | `EmbeddedSurfaceNode` 当前仅传递尺寸与代际元数据；嵌套输入变换随 #247 接入，不另造第二套坐标变换 |
| **Qt / 游戏引擎嵌入** | 无窗口路径统一走 `HeadlessInput`；仓库内不包含特定 Qt 或游戏引擎胶水层 |
| **XR 空间输入** | 未包含射线求交（raycast）、扳机或摇杆生命周期合同 |
| **Web / WebView** | 未包含浏览器 DOM composition 或动态 DPR 宿主桥接 |
| **远程流式输入（Remote）** | `HeadlessInput` 覆盖序列号、代际与断连语义；网络层的鉴权、防重放、限流与乱序重排归外部传输层负责 |

## Windows 实机验收（Issue #271 / #276）

真实窗口里的 IME 与手写笔用 `input-acceptance-probe` 示例和 `scripts/validate-input-acceptance.ps1` 验收。探针窗口只有一个聚焦的 `TextInput`（无障碍名 `IME editor`）和一块空白书写区；窗口路由器交付的每个规范输入事件都以一行 JSON（`"event":"input"`）写到 stdout，编辑器的值与焦点变化写成 `"event":"editor"`。脚本只操作探针窗口：IME 部分用 `SendInput` 敲键，经系统里真实的微软拼音（0804，须处于中文模式）与微软日语输入法（0411）产生预编辑、候选与提交，再用 UI Automation 核对编辑器的 `ControlType.Edit` 与 `ValuePattern` 值；笔部分用 `InjectSyntheticPointerInput` 注入 `PT_PEN` 帧，走的是数位板同一条 `WM_POINTER` 路径，但不是物理设备。运行时不要动键盘和鼠标：注入的按键发给前台窗口，脚本一旦发现探针失去前台就停止敲键。

```powershell
cargo build --release -p nana-ui --features hosted,bundled-fonts --example input-acceptance-probe
powershell -NoProfile -ExecutionPolicy Bypass -Mta -File scripts/validate-input-acceptance.ps1 -ExePath target/release/examples/input-acceptance-probe.exe
```

产物在 `target/acceptance/input/`：`trace.jsonl`（事件轨迹）、`summary.json`（每项检查与环境）、预编辑 / 候选 / 提交时的截图。系统丢弃注入按键时（某些终端策略下 `SendInput` 返回成功但最后输入时间不变），IME 部分记为 not executed，不算通过。

2026-11-23 记录（Windows 11 企业版 25H2 build 26200.9974，RTX 5060 Ti / DX12，96 DPI）：

| 项目 | 结果 |
| --- | --- |
| 笔悬停：在范围内移动送达，`buttons = 0`、`pressure = 0` | 通过 |
| 接触与抬起各一次；压力 1/1024 → 1.0 → 64/1024 逐帧跟随设备值，接触时无跳变，范围 0–1 | 通过 |
| 倾角 `(10, -5)` 原样送达 | 通过 |
| 按住笔杆键接触映射为 `button = 2`（副键），抬起清空 `buttons` | 通过 |
| 笔画中途失焦：送达 `Focus { focused: false }`，笔画结束且无残留按键 | 通过 |
| 笔之后的鼠标移动：`pointer_type = Mouse`、`buttons = 0`、`pressure = 0`，无串扰 | 通过 |
| 取消（`POINTER_FLAG_CANCELED`） | 注入无法验证：系统把注入帧上的取消标志去掉，抬笔以 `UP \| PRIMARY \| CONFIDENCE` 到达窗口，所以记为 not executed。宿主已在窗口过程里读消息 `wParam` 与笔帧上的取消标志，带标志的抬起送达 `Cancel`（`scene_host` 单元测试覆盖）；要用真实笔的取消（防误触、别的窗口夺走指针）确认 |
| 离开感应范围 | 通过：笔移出范围时送达 `PointerLeave`。宿主忽略 winit 在每次抬笔后发的笔 `PointerLeft`（那时笔仍在悬停），改由窗口过程里的 `WM_POINTERLEAVE` 报告离开。抬起的笔下一次悬停仍是新的指针 id，这是路由器的规则（抬起的触控与笔不留悬停），不是离开事件造成的 |
| 首帧压力为 0 的接触 | 通过：设备报 0、40/1024、80/1024，送达 0、0.039、0.078。宿主从窗口过程读到“报了压力且为 0”，不再补 `0.5`；不报压力的设备按下时仍是 `0.5` |
| 微软拼音 `nihao` + 空格：预编辑 `ni'hao` 逐键送达，候选窗贴在 caret 下方、不挤动布局，提交 `你好`，焦点不变 | 通过 |
| 微软日语输入法 `konnnichiha` + 空格 + 回车：预编辑与提交 `こんにちは`，焦点不变 | 通过 |
| UIA：编辑器为 `ControlType.Edit`，`ValuePattern` 值与提交后的值一致（两种输入法） | 通过 |
| 组字中途失焦：送达 `Focus { focused: false }` 与 `Composition::End`，再得焦恢复编辑焦点 | 通过 |
| 组字中途失焦后的提交 | 通过：宿主发 `End` 之后，微软拼音仍发 `Commit("ceshi")`；路由器在失焦取消预编辑后、焦点回来之前丢掉这一来源的组字编辑，编辑器保持空值（`ime.focus_loss.no_late_commit`）。与窗口参考（`docs/reference/window.md`）IME 段的“原生窗口失焦时 Runtime 取消预编辑”一致 |
| 物理 XP-Pen 数位板 | 未执行，需要人手 |

winit 的 Win32 指针处理不读 `POINTER_FLAG_CANCELED`、不处理 `WM_POINTERLEAVE`、不分压力 0 与“无压力”。宿主不改 winit：`nana-window` 的 `PenInput` 在 winit 之前看同一条 `WM_POINTER*` 消息，记下这一帧的取消与压力，宿主把它们用到 winit 为这一帧交付的事件上；笔离开由 `WM_POINTERLEAVE` 唤醒宿主送达。其他平台不安装，行为不变。物理笔的验收用同一个探针手动进行：启动探针、把 stdout 存为轨迹，然后用真实笔完成悬停、带压力的笔画、按笔杆键、中途切走窗口、移出范围，并触发一次取消。
