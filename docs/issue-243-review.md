# Issue 243 review matrix

这份矩阵按 249 → 250 → 251 → 252 → 253 的顺序记录当前工作树证据。`PASS` 只表示仓内合同通过；真实宿主、设备、网络和安全边界没有对应证据时保持 `OPEN`。

| 阶段 | 当前结论 | 已证明的合同 | 仍需补齐 |
| --- | --- | --- | --- |
| 249 Canonical InputEvent | `PARTIAL` | 强类型 source/device/pointer/generation/sequence/timestamp；key 与 committed text/IME 分离；pointer/wheel 合并；断连、stale、source-local ordering；bounded queue 与 per-device counters；versioned JSON wire envelope 与未知版本拒绝 | Runtime 内部仍有 canonical → 兼容 `InputEvent` 的转换层；跨线程 endpoint、远程认证/重放/限流；正式 C ABI 仍需独立协议 |
| 250 InputRouter | `PARTIAL` | document binding；per source/device/pointer identity；capture/focus/hover；disconnect/focus revoke cancel；capture owner 复用；native PointerLeft 现在先进入 canonical PointerLeave，capture 不再因离开 surface 被误取消；统一 route outcome；InputRouter counters/diagnostics 覆盖 hit-test、successful dispatch、focus/capture/hover 变化和 routing cache hit/miss；1000 个 captured PointerMove 回归确认只做一次初始 hit-test；Vue/native 共享 Runtime 路径 | `hit_candidates` 与真实 routed-node path 仍由 Runtime/传播层补充；真实 native/WebView/remote adapter；完整 HostService producer/result apply；宿主高频 wall-clock/frame-counter 证据 |
| 251 Presentation Coordinate Bridge | `PARTIAL` | logical/physical/normalized/remote/XR-UV/parent-local/viewport/content 映射；DPI、extent、rotate、non-uniform scale、clip、non-invertible；revision-bound inverse cache；metadata 原子更新和冲突拒绝；Runtime 与 Scene 的 inset/polygon/ellipse clip-path 命中测试；mapping latency metric | parent-local/viewport/content 的真实宿主 producer；scene host 动态 presentation metadata；XR surface intersection；真实 resize/DPR/dynamic resolution |
| 252 HostServices | `PARTIAL` | capability enum/request/outcome；带原始 request/context 的 response API；generation/document/node 检查；bounded payload queue；focus/切换/撤销会生成 IME enable/disable 与 NativeTextInput intent，焦点切换在 Runtime mutation 前预留三个 intent 槽位并在 native Focused 边界 drain；无焦点 IME update 在 host drain 处丢弃；IME surrounding request 已携带 focused caret `cursor_area`；Canonical Router 的 Ctrl+C/X/V 已改为异步 ClipboardWrite/Read，cut 仅在 Success 后删除；native Scene host 在路由外执行 OS clipboard response；pointer hover 的 Cursor intent 由 Router 生产并去重，native scene host 已接通 Cursor executor；Vue outcome queue；兼容 `RuntimeInputAdapter` 的同步快捷键路径使用 `try_lock`，不会阻塞输入线程 | 兼容 `RuntimeInputAdapter` 入口仍保留本地 clipboard 快捷键路径；焦点撤销后的真实 IME 生命周期与 native executor；Clipboard selection/权限的完整异步生命周期；native IME candidate UI/selection；DnD、Accessibility producer/executor |
| 253 Adapter Matrix / gates | `OPEN` | Window/winit lowering、Vue observation、headless canonical fixtures；idle endpoint、1000 captured PointerMove、Runtime/Platform/Scene tests；Clippy/fmt/diff/engine boundary | 1000Hz/240Hz native counters；真实 Windows、Android、WebView、XR、remote、accessibility；workspace full build（当前受 V8 symlink 权限阻断） |

## 固定证据

- `cargo test -p nana-ui-platform --lib --locked`：canonical、coordinates、HostServices、clipboard 共 95 项。
- `cargo test -p nana-ui-runtime --lib --locked`：Runtime 1286 项，包含极小可逆 affine 命中回归。
- `cargo test -p nana-ui --features hosted --lib --locked`：NanaUI 790 项，包含 endpoint generation 重绑后重新发出 IME enable、无编辑器焦点的 document-selection Copy、native PointerLeave 保持 capture 回归。
- `cargo test -p nana-ui --features hosted --lib scene_host::tests --locked`：native scene host 62 项通过。
- `cargo test -p nana-ui-vue --features hosted --lib --locked`：Vue hosted 883 项通过。
- `cargo test -p nana-ui --features hosted --lib canonical_router_tests --locked`：InputRouter 生命周期、idle、capture、pointer identity 和 HostService 语义 29 项通过。
- `cargo test -p nana-ui-platform --lib host_services --locked --quiet`：HostService capability、payload budget、stale response 和 DnD/A11y contract 7 项通过。
- `cargo test -p nana-ui-vue --features hosted --lib --locked`：Vue 883 项。
- `cargo test -p nana-ui-scene --lib --locked`：Scene 194 项。
- affected crates 的严格 Clippy、`cargo fmt --all -- --check`、`git diff --check`、`python scripts/check-engine-boundary.py` 已通过。

这些命令证明仓内行为和边界合同；它们不替代真实 OS window、IME、WebView、XR、Android、远程网络或安全审计。Issue 243 只有在 OPEN 项获得对应平台证据后才能关闭。

## 按子 Issue 验收条目逐项审计

### #249

| 条目 | 当前证据 | 结论 |
| --- | --- | --- |
| Window/winit lowering | native pointer/key/wheel/Focus/IME lowering 现在进入 per-window `InputEndpoint`，再由 Router drain；native IME request state machine 仍在 drain 后执行 | 仓内通过 |
| endpoint capacity/backpressure | `RejectedInput.event` 在 native adapter 进入 per-window bounded pending queue，窗口销毁时清理；HostService backpressure 保留 endpoint 队首 | 仓内通过；pending overflow 只能报告 host failure，真实 OS backpressure 仍需平台验收 |
| keyboard/text/IME separation | canonical tests 覆盖 physical/logical key、committed text、composition；没有从 KeyDown 推导文本 | 通过 |
| Pointer/Touch/Pen/Wheel compact model | `PointerInput` 通过 `PointerType` 表达 mouse/touch/pen，共用一份紧凑 payload；`WheelInput` 独立 | 语义可表达；若要求 Touch/Pen 为独立 enum variant，仍是 API follow-up |
| multiple source/device/pointer | typed identity、per-device counters、pointer remap tests | 通过 |
| disconnect/stale generation | endpoint 与 router tests 覆盖 reconnect、source/device tombstone、generation rejection | 通过 |
| no heap-heavy high-frequency pointer | inline lowering、coalescing、payload allocation test | 仓内通过；未有 allocator/native wall-clock profile |
| upper layers consume canonical semantics | Router 是 canonical authority，但 Runtime adapter 内部仍转换到兼容 `InputEvent`；Vue hosted adapter 现在也经 per-window `InputEndpoint` drain，already-routed observation 不重复路由 | 兼容迁移层仍存在；endpoint 接线已覆盖 native/Vue |

静态消费者审计结果：仓内剩余 `InputEvent` 使用集中在 lowering 实现、兼容 `RuntimeInputAdapter::dispatch`/测试与窗口 chrome 的 host 手势处理；Vue/native 业务输入先进入 `InputRouter`，未发现绕过 Router 直接调用组件业务回调的生产路径。

当前消费者边界进一步核对：

| 消费者 | 旧 `InputEvent` 是否仍存在 | 是否绕过 Canonical/InputRouter | 处理结论 |
| --- | --- | --- | --- |
| `scene_host::InputTracker` | 是，作为 winit lowering 中间值 | 否，随后通过 `lower_input_event`、`InputEndpoint` 和 `InputRouter` | 可保留至 direct-replacement 迁移完成 |
| `RuntimeInputAdapter` | 是，内部组件编辑/导航实现 | 否，Canonical Router 只在 Runtime 边界调用兼容实现 | 兼容层残留，需后续逐步替换内部签名 |
| `window_chrome` | 是，标题栏 host gesture | 不属于 UiWorld 业务路由；使用 host chrome geometry | 明确的宿主 chrome 特例 |
| Vue `observe_runtime_canonical` | 是，浏览器事件观察投影 | 不再二次路由；canonical 已先完成 Runtime route | 观察 API，不是第二路由 authority |
| tests/examples/devtools | 是 | 不代表生产 host path | 迁移时按 fixture/consumer 分批更新 |

### #250

| 条目 | 当前证据 | 结论 |
| --- | --- | --- |
| Nana owns focus/capture | router + UiWorld tests | 通过 |
| captured pointer outside surface | canonical PointerLeave native lowering、capture owner path and cancel tests | 仓内通过；真实 OS pointer capture/release 仍需 native acceptance |
| endpoint destroy/disconnect revoke | detach/disconnect cancel tests | 通过 |
| unchanged move avoids structural work | pending-work revision and existing Runtime tests | 仓内部分通过；未完成 1000Hz native counter |
| geometry equals rendered hit | Runtime presentation and inset/polygon/ellipse clip-path tests | 仓内通过；#236 high-density geometry fixture 未接入 |
| host does not call widget callbacks | native/Vue path enters router | 仓内通过 |
| required diagnostics | route latency、InputRouter counters、Runtime WorkCounters 和 diagnostics metrics | `hit_candidates` 与真实 routed-node path 尚未独立暴露；宿主 wall-clock 仍未测 |

### #251 Presentation Coordinate Bridge

| Issue 验收条目 | 当前证据 | 结论 |
| --- | --- | --- |
| 视觉位置与命中位置一致 | `coordinates` 与 Runtime presentation/矩形 overflow clip hit-test tests 共用 logical extent、transform revision 和 clip 语义 | 仓内部分通过；Scene 的 clip-path 尚未进入 Runtime 命中，真实渲染 surface 仍需验收 |
| nested NanaUI 复用坐标桥 | `ParentLocal`/`Viewport`/`Content` space 与 host-provided application transform API 存在 | 接口和单测通过；没有 nested native/reference host fixture |
| XR ray/UV 进入普通 pointer pipeline | `XrSurfaceUv` space 可归一为 logical point | 仅数据模型；没有 XR surface intersection/trigger fixture |
| render resolution 与 logical input 解耦 | normalized/physical tests 在 presentation extent 改变后保持 logical position | 仓内通过；没有动态 host resize/dynamic-resolution 运行证据 |
| 无第二套 transform authority | bridge 只保存 host metadata/inverse cache，UiWorld 仍拥有 geometry/hit index | 仓内通过 |
| transform cache work counter | revision-bound inverse counter、non-invertible、clip rejection、mapping latency tests | 仓内通过 |

结论：`PARTIAL`。parent-local 的真实嵌套 producer、XR intersection、WebView DPR/resize 和动态 presentation metadata 仍未验收。

### #252 HostServices

| Issue 验收条目 | 当前证据 | 结论 |
| --- | --- | --- |
| Window backend 能迁入 HostServices | Clipboard/Cursor/IME/NativeTextInput 已由 Router 产生 request，native scene host 在路由外 drain | 仓内部分通过；DnD/A11y 仍走既有原生 action/window 链路 |
| nested NanaUI 有 service delegation/override 规则 | request 绑定 source/generation/document/node，`HostServiceBroker` 支持 capability gate | 合同存在；没有 nested delegation/reference host fixture |
| IME 不依赖 fake key event | committed text、composition 独立 canonical payload；NativeTextInput 复用既有 IME request state machine | 仓内通过；真实 OS candidate/selection/focus revoke 未验收 |
| clipboard/drag-drop 支持 async/capability denial | Clipboard request/response、Denied/Unsupported、bounded queue 已测试 | Clipboard 仓内通过；DnD producer/executor 尚未实现 |
| cursor/a11y 无 presentation-frame polling | cursor intent 去重并在 event-loop boundary 执行；既有 a11y action queue 为事件驱动 | Cursor 仓内通过；统一 A11y HostService bridge 尚未实现 |
| service 生命周期与 focus/generation 对齐 | request drain 和 async response 均检查 generation/document/node/focus，stale request 计数 | 仓内通过；真实 native focus revoke 仍需平台验收 |

结论：`PARTIAL`。DnD、Accessibility、native IME candidate/selection 以及 clipboard permission lifecycle 仍是明确 follow-up。

### #253 Adapter Matrix 与性能门禁

| Issue 验收条目 | 当前证据 | 结论 |
| --- | --- | --- |
| 六类 input 场景有 reference fixture | Window/winit、Vue observation、headless canonical fixtures；未有 Nested、Native Qt/game、XR、WebView、Remote 完整 fixture | 未完成 |
| Window 只是 InputProvider adapter | native scene host lower 到 canonical，再由 InputRouter 路由；旧兼容 adapter 仍保留 | 仓内部分通过，长期双 semantic path 尚未消除 |
| nested/XR/web/remote 不伪造 OS window message | canonical API 和 coordinate spaces 可注入；缺少真实 adapters | 仅接口证据，未完成 |
| source/frame rate 与 Nana render rate 解耦 | endpoint idle、capture move、pending-work revision 单测 | 仓内部分通过；无 Host 240Hz + Nana static wall-clock/frame evidence |
| idle 输入路径真正零工作 | empty endpoint fixture：0 canonical route、0 hit-test、0 routing cache、0 pending work | 仓内通过 |
| 高频事件不触发全局 layout/render work | 1000 captured PointerMove 只做一次初始 hit-test；Runtime work revision 不增长于无关路径 | 仓内部分通过；无真实 1000Hz/native frame counter |
| CI 识别事件风暴/stale/fallback/polling 回归 | canonical/router counters、diagnostics metrics、stale/queue tests | 仓内部分通过；尚未有独立 CI performance threshold/job |

结论：`OPEN`。需要 reference adapter 矩阵、真实宿主性能测量和 CI 性能门禁。

这些条目的外部缺口包括 parent-local/viewport/content 的真实 producer、XR ray intersection、WebView DPR/composition、native IME、Qt/game-host service mapping、remote authentication/replay/rate-limit，以及 240/1000Hz 与 Nana render-rate 解耦的 wall-clock/frame-counter evidence。当前 headless 单测不替代这些证据。

## 本轮 review 新发现的阻断项

- **#250 native leave 语义（本轮已修复）**：scene host 原先把 `PointerLeft` 降成 `PointerPhase::Cancel`，会错误撤销 captured pointer。现在 native leave 直接进入 canonical `PointerLeave`，只清理 hover；frame-move 仍走原有 cancel 路径，并有 Runtime capture 保留回归测试。
- **#251 命中性能**：当前 presentation transform dirty 判定已经按 Document 缓存和限定；同一 Document 存在 compositor transform 时仍会走 presentation hit walk，尚未证明 PointerMove 的工作量严格受 transform depth 约束。需要以真实 Runtime work counter 覆盖大树、动态 transform 和空白区域移动。
- **#251 生产坐标接线**：scene host 仍默认 identity presentation transform，动态 presentation metadata、nested producer、XR surface intersection、WebView DPR/resize 尚未接入；坐标桥数学单测不能替代这些宿主证据。
- **#252 路由语义**：HostService 队列容量现在在主要 dispatch 前预留，避免常见 IME/focus 背压在 mutation 后才失败；但预检只能知道当前焦点 editor 的 composition payload，无法在 pointer/key 导致焦点切换前知道新 editor 的 surrounding-text 大小。若队列接近 payload 上限，Runtime mutation 后生成 `ImeEnable` 仍可能因新 surrounding payload 被拒绝；这需要事务式 payload reservation 或可重试的 IME intent，不能用当前 headless 证据宣称已闭合。legacy adapter 的 clipboard shortcut 仍是独立兼容路径，DnD/A11y producer/executor 仍未统一到 HostServices。

上述问题不是由离屏单测覆盖的“已通过”项；在修复并重新运行功能、性能和边界验收前，Phase 3/4 继续保持 `PARTIAL`，Phase 5 保持 `OPEN`。

## Phase 1 语义复核

- PointerMove 只有在 source/device/generation、pointer identity、pointer type、button/buttons、modifiers、primary 和 activation 状态全部一致时才合并；button transition、key、focus、IME 不进入合并分支。
- Wheel 只有在 pointer、坐标、unit、modifiers 一致且累计 delta 有限时合并；sequence/timestamp 仍在合并前按 source endpoint 检查，因此合并不会跨越乱序输入。
- generation reset 会清空旧队列、payload 预算、device tombstone 和 ordering cursor；disconnect 后只允许对应 reconnect/control event，旧事件保留所有权并返回 rejection。
- 这些语义已有 canonical endpoint 单测；跨线程同步、远程认证/重放/限流和真实 OS backpressure 仍属于 Phase 5/platform follow-up。

## 关联 Issue 合同审计

| 关联 Issue | 当前取证 | 对 #243 的影响 |
| --- | --- | --- |
| #203 LayoutResult / Fragment / Geometry authority（OPEN） | `UiWorld` 的基础 hit index 消费 `LayoutBox`，但 `ComponentGeometry` 仍由 `world::geometry` 派生，并被 Runtime 组件、输入和 A11y 多处读取；未发现已统一的公开 `LayoutResult/Fragment` authority | #250/#251 只能证明当前 Runtime projection 一致，不能宣称已完成 #203 的唯一几何合同 |
| #234 ViewportTransform（OPEN） | 当前仓内未检索到 `ViewportTransform` canonical 类型；`PresentationCoordinateBridge` 的 `Viewport/Content` 是 host-provided affine slot，不是 #234 的 viewport capability/visible-bounds/revision 合同 | #251 的 viewport/content 目前是兼容接口和单测，不等于复用 #234 |
| #247 EmbeddedApplication / ExternalSurfaceNode（OPEN） | 当前输入 bridge 有 endpoint/document/generation 字段，但未发现 `EmbeddedApplication` 输入 seam、child presentation metadata producer 或 parent/child delegation fixture | nested NanaUI 的 focus/capture/IME 只能列为未验收 |
| #8 Performance Contract（CLOSED，现行合同仍要求 counter/CI 门禁） | Runtime/Scene 已有 WorkCounters，InputRouter 有 counters/diagnostics；但本 Issue 的 240/1000Hz native frame counters 与独立 CI threshold 尚不存在 | #253 仍 OPEN；不能用单元 counter 代替 #8 的宿主性能门禁 |
| #242 Presentation Output Epic（OPEN） | 输入侧没有依赖第二棵渲染树，但真实 VisualEndpoint/RenderTarget/Presenter 生命周期尚未作为 adapter 证据接入 | #251/#253 的真实 presentation transform 和多输出验收仍缺 |

### 额外开放 Issue 审计

| Issue | 与 #243 的关系 | 当前决定 |
| --- | --- | --- |
| #174 Platform Input / UI Action / application input 分层（OPEN） | Canonical input 只覆盖 platform event；不能把连续轴、玩家映射或 universal ActionMap 塞进 InputRouter | 已遵守边界；gamepad/连续业务输入不纳入本 Epic |
| #180 inert 子树（OPEN） | focus revoke、pointer hit 和 A11y action 的生命周期会影响 fade-out/disabled subtree | 当前 Router 只处理 endpoint/source 生命周期；inert 语义需作为独立 Runtime follow-up，不能假设已由 capture/focus 清理覆盖 |
| #201 Viewport / Clip behavior（OPEN） | #251 的 Viewport/Content 映射与 scroll-only hit-test、clip chain authority 直接相关 | 当前 bridge 只消费 host transform；scroll metrics/clip authority 继续依赖 #201/#203 |
| #206 Layout Foundation counters（OPEN） | #253 的高频输入门禁必须与 layout/measure/reflow counters 共用 #8 catalog | 当前已有 Runtime work counters，但没有完整 #206 structural gate 接入 |
| #214 Dynamic Layout performance（OPEN） | 输入事件风暴不能掩盖 dynamic solver/layout work；需要 cheap path、budget 和 resize-storm 证据 | 作为性能依赖记录，不在 #243 内复制 solver/profiler |
| #271 Windows CJK IME + AccessKit（OPEN） | 直接验证 #252 IME candidate/focus 与 Accessibility producer 的真实窗口行为 | 明确列为真实 Windows 验收阻断，headless IME 单测不能替代 |
| #276 Windows stylus（OPEN） | 验证 #249 Pen/pressure/hover/barrel-button 的真实设备 lowering | Canonical 数据模型可表达，但没有真实设备证据，保持 OPEN |
| #277 high-refresh acceptance（OPEN） | 覆盖 #253 的 120Hz、复杂 UI、多窗口、device loss、Windows/Android 平台性能矩阵 | 作为最终性能验收 Issue，不用当前 crate 单测宣称完成 |

这组依赖说明为什么本 Epic 当前保持 PARTIAL/OPEN：输入实现可以复用现有 `UiWorld`/Scene 路径，但关联 Issue 尚未提供所有目标 authority 或真实宿主 fixture。
