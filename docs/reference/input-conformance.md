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
