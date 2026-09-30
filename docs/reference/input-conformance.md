# 输入验收矩阵

Issue 243。这张矩阵告诉你两件事。哪些合同测试可以在仓库里重复跑。哪些验收必须由真实宿主或真实设备补上。

离屏测试不能代替 native window。也不能代替真实 IME、WebView、XR 或远程网络。

## 已覆盖的仓内合同

| Fixture | 合同 | 证据 |
| --- | --- | --- |
| canonical endpoint | 有界队列：相邻 pointer move 与 wheel 合并，队列满时把事件交还调用方；`InputSequencer` 按 generation 重新计数。generation、顺序、断连的校验只在路由层做一次（见下一行） | `cargo test -p nana-ui-input --lib --locked` |
| host services | 光标与文本输入是最新值槽位；headless 服务记下宿主会显示的状态；共享剪贴板忙时回答 `Busy` 而不等待 | `cargo test -p nana-ui-input --lib --locked`、`cargo test -p nana-ui-platform --lib --locked` |
| runtime router | 绑定与校验（stale、乱序、时间回退、断连、同 generation 换 document）；多 source/device 指针身份；窗口失焦只取消指针、保留 document 焦点、重新获得焦点时重发文本输入；source 断开时才清焦点；触控抬起不留悬停；首个事件不读其他 source 的捕获；被处理按键的文本被丢弃；剪贴板在按键链原位置；两个 source 各自的文本输入与光标；文件拖放交给放置目标、目标即 `pointer_hit`、失焦与断开结束悬停 | `cargo test -p nana-ui-runtime --all-features --lib framework::input --locked` |
| 组件行为（原 adapter 测试） | 指针、滚轮、键盘、文本、组字进入各类组件的原有行为，经新路由逐条保留（83 条） | 同上，`framework::input::tests::dispatch` |
| 分配与命中门禁 | 未捕获移动每次恰好 1 次命中查询、捕获移动 0 次；同一行、跨行（每次都切换悬停）、滚动视口内、捕获中的稳态移动都零分配 | `cargo test -p nana-ui-runtime --all-features --test input_alloc --locked`（计数分配器只统计测试自己的线程） |
| native scene host | winit 事件直接降级为 canonical（W3C 物理键名、滚轮与鼠标同一指针、指针离开带自己的设备；拖放降级为 `FileDrag`，含延迟到达的路径与取路径失败时的取消）；IME 请求在 host services 中合成；光标由 Runtime 意图、窗口边框缩放与程序覆盖合成 | `cargo test -p nana-ui --features hosted --lib scene_host --locked`（仅仓内证据） |
| Vue | scene host 已路由的事件只进入 Vue 观察层；没有原生窗口时（`VueHost` 的 `dispatch_*` / `commit_text` / `dispatch_key` / 组字与 IME、`VueHostedRuntime::runtime_input`）由每个 `VueHost` 自己的输入源先路由、再观察，Vue 不再自己改 Runtime。滚轮的 overflow、RTL 负向横滚、嵌套冒泡、`pointer-events: none` 穿透经路由验证；被处理或被页面阻止的按键不再发出 `insertText`；失焦取消组字 | `cargo test -p nana-ui-vue --all-features --lib --locked` |
| devtools headless | `RuntimeAgentSession` 经 `HeadlessInput` 走同一条路由；1000 次指针事件后路由计数与空闲 flush 正确 | `cargo test -p nana-ui-devtools --features runtime-agent --all-targets --locked` |

## 性能

- 确定性门禁用计数，不用时间。见上表「分配与命中门禁」。
- 墙钟对比用 `nana-input-benchmark`（`crates/nana-ui-devtools/src/bin/nana-input-benchmark.rs`）。它只走 `RuntimeAgentSession::pointer_event`，不含 flush。两份二进制交替运行，读每轮 p50 的最小值。2026-09-28 本机（负载 3–5，交替 4 轮）：

  | 场景 | 重写前 | 重写后 | 每事件分配 |
  | --- | --- | --- | --- |
  | 同一行内移动 | 364 ns | 248 ns（−31.8%） | 1 → 0 |
  | 跨行移动 | 362 ns | 249 ns（−31.2%） | 1 → 0 |
  | 滚动视口内移动 | 631 ns | 415 ns（−34.2%） | 5 → 0 |
  | 捕获中拖动 range | 1331 ns | 1056 ns（−20.7%） | 7.86 → 2.79 |

  拖动 range 剩下的分配，是组件每次发出的 `RangeInput` 事件。它不在路由路径上。
- 空闲 drain 不做任何工作：0 次路由、0 次命中查询，pending work revision 不变（`an_idle_drain_does_no_work`）。
- 1000 次捕获中的指针移动不改变 `pending_work_revision`（`hit_queries_are_budgeted_and_a_disconnect_revokes_capture`）。
- 真实 1000Hz/240Hz 宿主的墙钟和原生帧计数，仍然需要宿主基准。本文件不用单元测试冒充这些证据。

## 尚未具备生产证据的 adapter

按 #253 的六类 adapter 逐项核对：

| Adapter | 当前仓内证据 | #253 结论 |
| --- | --- | --- |
| Window/winit | `crates/nana-ui/src/scene_host` 的原生降级、每窗口 endpoint、host services | 有参考路径；真实 OS window 兼容性仍需 Windows/macOS/Linux 验收 |
| Nested NanaUI | 无；坐标桥已删除，没有真实嵌入生产者 | #251 回到 OPEN，随 #247 的嵌入生产者再加 |
| Native host | `examples/runtime-host-fixture` 是 Runtime host 示例，没有 Qt/游戏引擎输入适配 | 不满足 native reference adapter 门禁 |
| XR | 无 | 缺 OpenXR/OpenVR 射线求交、扳机、摇杆生命周期 |
| Web/WebView | Vue 独立模式注入输入与 JS 观察测试 | 缺真实 WebView composition、DPR/resize、剪贴板/光标能力 fixture |
| Remote/headless | `HeadlessInput`、sequence/generation/stale/disconnect 单测 | 缺认证、重放防护、限流、网络乱序与延迟证据 |

真实 Windows IME（CJK 候选框）、macOS 候选框、Android 软键盘，以及窗口边框缩放和程序光标覆盖的组合，都要你逐平台人工验收。headless 测试不覆盖这些。
