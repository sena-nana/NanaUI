# Issue 243 输入 conformance

这份矩阵记录可以在仓内重复执行的合同测试，以及必须由真实宿主或设备补齐的验收。离屏测试不能替代 native window、真实 IME、WebView、XR 或远程网络。

## 已覆盖的仓内合同

| Fixture | 合同 | 证据 |
| --- | --- | --- |
| canonical endpoint | source/device/pointer identity、generation、source-local sequence、断连、stale、pointer/wheel 合并和不可丢失 transition | `cargo test -p nana-ui-platform --lib --locked` |
| canonical wire envelope | explicit contract version、JSON round-trip、未知版本拒绝；不把 Rust enum layout 当作 ABI | `cargo test -p nana-ui-platform --lib canonical::tests::versioned_wire_round_trip_and_rejects_unknown_version --locked` |
| coordinate bridge / Runtime clip | DPI、logical/presentation/host extent、旋转、非均匀缩放、clip、non-invertible、revision-bound inverse cache、parent-local/viewport/content host-supplied transforms；Runtime 命中复用 inset/circle/ellipse/polygon clip | `cargo test -p nana-ui-platform --lib coordinates::tests --locked`、`cargo test -p nana-ui-runtime --lib world::hit_test::presentation_tests --locked` |
| runtime router | 多 source/device/pointer、capture/focus revoke、IME 生命周期、pending work revision、HostService outcome | `cargo test -p nana-ui --features hosted --lib --locked` |
| native scene host | pointer/key/wheel winit lowering 进入 per-window `InputEndpoint` 后再由 Router drain；physical cursor bridge、native window lifecycle、capture observation 不重复全局 hit-test；Router 已生产 Cursor intent，native scene host 已执行 Cursor request；NativeTextInput intent 在 host 边界复用既有 IME request state machine | `cargo test -p nana-ui --features hosted --lib scene_host::tests --locked`：62 passed（仅仓内证据） |
| Vue observation | Vue injected canonical input now enters a per-window `InputEndpoint`; native already-routed input only enters Vue observation layer and does not repeat Runtime text/IME mutation | `cargo test -p nana-ui-vue --features hosted --lib --locked`：883 passed |

## 性能门禁

- canonical 高频 pointer lowering 使用 inline iterator；pointer payload 不分配 heap object，adjacent move/wheel 可合并。
- unchanged transform revision 重复映射只计算一次 inverse；测试检查 1000 次查询只有一次 recompute。
- 坐标桥累计记录 mapping latency，并通过 `runtime.input.mapping` 诊断 histogram 暴露。
- Vue hosted adapter 保留 bounded HostService outcomes，调用方可观察 headless capability denial，不再静默丢弃结果。
- IME composition 在 Runtime mutation 前同时检查 request slot 与 payload budget，队列背压不会再消费该输入事件；surrounding request carries the focused caret `cursor_area`. Pointer/key 导致焦点切换时，当前只能预检旧焦点 payload；新 editor 的 surrounding-text 大小要在 Runtime mutation 后才能得知，因此接近 payload 上限时仍需事务式 reservation 或 retry intent 的后续合同。
- native Focused transitions drain IME lifecycle intents at the same event-loop boundary even when no pointer/key event follows.
- focus lifecycle emits a bounded `NativeTextInput` capability intent while text content and caret state remain owned by the existing IME request path.
- focused IME anchor regression verifies the logical caret rectangle survives into `ImeEnable` without host-side coordinate guessing.
- endpoint drain 在 HostService 背压时保留队首 canonical event；释放 request slot 后可按原 sequence 重试，不丢失输入或改变顺序。
- native scene host 的 endpoint capacity rejection 会把拥有权移入 per-window bounded pending queue；窗口销毁时一并清理，避免 `RejectedInput.event` 被静默丢弃。
- Vue hosted endpoint 在 Runtime 路由后始终 drain `UnsupportedHostServices` outcome，即使本轮遇到 HostService backpressure，也不会把 capability queue 永久锁住。
- HostService drain 以 `UiWorld` 的 live document roots 作为生命周期权威；未挂载 document 的 request 会被丢弃并计入 stale counter。
- Clipboard response validation distinguishes document-selection Copy (live selected node/document is sufficient) from Cut/Paste (focused editable owner is required), preserving async capability semantics without rejecting a valid unfocused document selection.
- captured pointer move 复用 `InputRouter` 的 capture owner，不调用 scene host 的全局 hit-test。
- native `PointerLeft` 现在 lowering 为 canonical `PointerLeave`；它清理 hover 但不撤销 pointer capture，后续 move/up 仍可由 capture owner 接收；frame-move 仍保留显式 cancel 路径。
- canonical router regression drives 1000 captured PointerMove events: only the initial Down increments `hit_tests`; all moves increment the routing-cache hit counter.
- The same 1000-event regression snapshots `UiWorld::pending_work_revision`; captured moves leave the Runtime work revision unchanged, so the fixture rejects accidental layout/render invalidation.
- empty endpoint drain is an explicit idle fixture: zero routed events, hit-tests, cache hits, or pending Runtime work.
- Runtime 路由通过单调 `pending_work_revision` 报告 invalidation，不 drain `SystemWork`。
- presentation transform overlay 的命中文档集合按 PresentationStore/world revision 缓存，PointerMove 不扫描所有节点或所有文档。
- idle、静态 hover、真实 1000Hz/240Hz host 的完整 wall-clock 和 native frame counters 仍需要宿主 benchmark；本文件不以单元测试冒充这些证据。

## 尚未具备生产证据的 adapter

按 #253 的六类 adapter 逐项核对：

| Adapter | 当前仓内证据 | #253 结论 |
| --- | --- | --- |
| Window/winit | `crates/nana-ui/src/scene_host` 的 native lowering、per-window endpoint、scene-host tests | 有 reference path；真实 OS window 兼容性仍需 Windows/macOS/Linux 验收 |
| Nested NanaUI | Coordinate bridge 的 ParentLocal/Viewport/Content 数学测试，没有 embedded child host producer | 缺少真实 nested endpoint、focus/capture/IME delegation fixture |
| Native host | `examples/runtime-host-fixture` 是 Runtime host 示例，但没有 Qt/game-engine input adapter 和 HostServices mapping | 不满足 native reference adapter 门禁 |
| XR | 只有 `XrSurfaceUv` 数据空间和映射单测 | 缺 OpenXR/OpenVR ray intersection、trigger、thumbstick lifecycle |
| Web/WebView | Vue hosted injected input 与 JS observation tests | 缺真实 WebView composition、DPR/resize、clipboard/cursor capability fixture |
| Remote/headless | canonical wire/sequence/generation/stale/disconnect 单测 | 缺认证、重放防护、限流、网络 reorder/latency 和远程帧率解耦证据 |

Window/winit 与 Vue hosted 的 injected pointer/key/wheel/Focus/IME 已经过 per-window endpoint；native already-routed observation 不重复路由。native 动态 presentation transform metadata、XR ray intersection、Web/WebView DPR/composition、Android IME、OpenXR、remote authentication/replay/rate limit、跨线程背压，以及真实 accessibility provider 仍需独立 adapter fixture 和平台验收。

远程 payload、拖放文件权限、FFI/serialization ABI、multi-window/document/presentation-target 持久绑定也不能由当前 headless fixture 推断完成；这些保持为后续 Issue 的明确验收项。

当前生产链路仍有两个已知边界：Canonical Router 的 Ctrl+C/X/V 已使用可返回结果的
`HostServiceRequest`，native Scene host 已在输入路由外执行 OS clipboard request；但
`RuntimeInputAdapter` 兼容入口仍直接使用现有 `ClipboardHost` 的 try-lock 快速探测，
无焦点 IME update 已在 host drain 处丢弃；native candidate UI、selection lifecycle 和完整焦点撤销仍需真实宿主接线后验收。`CompositionInput::End` 已映射为取消并丢弃 preedit，
`Disabled` 才保留既有的失焦提交语义。
