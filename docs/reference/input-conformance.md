# 输入 conformance

Issue #243。这张表列出可以在仓库里重复跑的合同，以及必须由真实宿主或真实设备补上的验收。

离屏测试不代替 native window，也不代替真实 IME、WebView、XR 或远程网络。

## 仓内合同

| 合同 | 命令 |
| --- | --- |
| 有界队列、pointer/wheel 合并、generation 重新计数 | `cargo test -p nana-ui-input --lib --locked` |
| 绑定、断连、失焦、捕获、剪贴板、文件拖放、空端点 drain 为 0。按键策略看到物理键、逻辑键、repeat 和同一组修饰键 | `cargo test -p nana-ui-runtime --all-features --lib framework::input --locked` |
| keymap 用逻辑键匹配，修饰键四位与 `InputModifiers` 一致，释放不命中 | `cargo test -p nana-ui-runtime --all-features --lib key_layers --locked` |
| 未捕获移动每次 1 次命中、捕获移动 0 次，稳态移动不分配。1000 次落在同一行的 PointerMove，以及逐行穿过的 1000 次，layout frontier 与 scene extract 都为空 | `cargo test -p nana-ui-runtime --all-features --test input_alloc --locked` |
| 单文档命中跟 presentation 的平移、缩放、投影和 clip | `cargo test -p nana-ui-runtime --all-features --lib world::hit_test --locked` |
| Host 以 240Hz 轮询、窗口保持 `FrameDemand::OnDemand`：240 次 present 为 0 | `cargo test -p nana-ui --features hosted --lib scene_host::schedule::tests::a_static_window_polled_at_240hz_presents_nothing --locked` |

Window、Vue 和 Android slot 都把原生事件降成 `CanonicalInputEvent`，再进 `AppContext::route_input`。无窗口宿主用 `HeadlessInput`。诊断计数是 `runtime.input.*`（`nana-diagnostics`，ID 20–37）。已退休的坐标桥和请求队列 ID 不复用。

1000Hz 是合成 PointerMove 流，240Hz 是宿主轮询而 Nana 仍按需出帧。上表是工作计数，不是显示器刷新率。窗口里可合并的移动和滚轮留到回合结束再路由，只有这次路由标脏了才请求帧。墙钟对比用 `nana-input-benchmark`（`crates/nana-ui-devtools/src/bin/nana-input-benchmark.rs`）。

文件拖放是 `InputPayload::FileDrag`，由路由交给已登记的放置目标。

## 仓库外的宿主

| Adapter | 仓内现状 |
| --- | --- |
| Window | `scene_host` 降级到 canonical |
| Nested NanaUI | 嵌入元数据只有范围和 generation。随 #247 再接，不另建一套变换 |
| Qt / 游戏引擎 | 无窗口路径是 `HeadlessInput`。没有 Qt 或游戏引擎绑定 |
| XR | 没有射线求交、扳机或摇杆生命周期 |
| Web / WebView | 没有 composition 或 DPR 宿主 |
| Remote | `HeadlessInput` 覆盖 sequence、generation、断连。没有认证、重放防护、限流或网络乱序 |
