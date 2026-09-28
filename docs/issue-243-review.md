# Issue 243 review matrix

这份矩阵按 249 → 250 → 251 → 252 → 253 的顺序记录当前工作树证据。`PASS` 只表示仓内合同通过；真实宿主、设备、网络和安全边界没有对应证据时保持 `OPEN`。

2026-09-28 重写后只剩一条输入路径：宿主把原生事件降级成 `CanonicalInputEvent`（`nana-ui-input`），盖戳后进入每个 source 的 `InputEndpoint`，由它绑定的 `AppContext` 用 `drain_input` / `route_input` 路由，程序从 `RoutedInput` 观察同一个 canonical 事件。旧的 `InputEvent` / `ImeEvent`、`RuntimeInputAdapter`、`InputRouter`、HostService 请求队列与 broker、wire envelope 与坐标桥都已删除。

| 阶段 | 当前结论 | 已证明的合同 | 仍需补齐 |
| --- | --- | --- | --- |
| 249 Canonical InputEvent | `PASS`（仓内） | 强类型 source/device/pointer/generation/sequence/timestamp；按键、提交文本、组字是独立负载，文本带着产生它的按键 sequence；pointer/wheel 合并；断连、stale、source-local ordering；有界队列与 per-device 计数；`InputSequencer`；所有宿主直接降级，不再有中间形态 | 跨线程 endpoint、远程认证/重放/限流；正式 FFI/序列化版本需独立协议 |
| 250 InputRouter | `PASS`（仓内） | 路由状态挂在每个 `AppContext` 上、按 source 分；绑定返回 `Result`；指针身份先解析再读捕获与悬停；窗口失焦只取消指针、不清 document 焦点；触控抬起清悬停；被处理按键的文本被丢弃；未捕获事件 1 次命中查询、捕获 0 次；稳态移动零分配；时间取自事件时间戳；唯一的 drain 先出队再路由 | 真实 native/WebView/remote adapter；宿主高频墙钟与帧计数 |
| 251 Presentation Coordinate Bridge | `OPEN` | 原实现只在恒等变换下被使用，已删除；原生宿主的物理→逻辑换算是 `position.to_logical(scale)` | parent-local/viewport/content 的真实生产者（#247）、XR surface intersection、WebView DPR/resize、动态 presentation metadata |
| 252 HostServices | `PARTIAL` | 光标与文本输入是最新值槽位（只在变化时、窗口重新获得焦点时下发）；IME 周围文本恒为 ≤4000 字节的窗口；剪贴板同步调用、在按键链原位置、忙时不等待；原生宿主 IME 执行器（winit）、光标合成（程序覆盖 > 窗口边框 > Runtime 意图）、进程唯一剪贴板；headless 与 Android 各有实现 | 真实 Windows/macOS 候选框与焦点撤销验收；剪贴板权限生命周期；Accessibility action 仍走既有通道（见下） |
| 253 Adapter Matrix / gates | `OPEN` | Window/winit、Vue、devtools、Android 共用同一路由；空闲 drain 零工作；分配与命中计数门禁；A/B 基准（`nana-input-benchmark`） | 1000Hz/240Hz 原生计数；真实 Windows、Android、WebView、XR、remote、accessibility |

## 固定证据

- `cargo test -p nana-ui-input --lib --locked`：canonical endpoint、sequencer 与 host services。
- `cargo test -p nana-ui-runtime --all-features --lib framework::input --locked`：路由语义（26 条重写）、原组件行为（83 条移植）、终端 3 条、hover card 15 条、IME 状态生产者与周围文本窗口。
- `cargo test -p nana-ui-runtime --all-features --test input_alloc --locked`：稳态移动零分配（同一行、跨行、滚动视口、捕获中）与命中查询预算。
- `cargo test -p nana-ui --features hosted --lib --locked`：原生降级、IME 执行器、光标合成、scene host。
- `cargo test -p nana-ui-vue --all-features --lib --locked`、`cargo test -p nana-ui-devtools --features runtime-agent --all-targets --locked`、`cargo test -p nana-android-host --locked`。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check`、`python scripts/check-engine-boundary.py`、`python scripts/check-api-convergence.py`。

这些命令证明仓内行为和边界合同；它们不替代真实 OS window、IME、WebView、XR、Android、远程网络或安全审计。

## 文件拖放与 Accessibility action

- **文件拖放**走输入路由：宿主降级为 `InputPayload::FileDrag`，路由交给放置目标并把目标作为 `pointer_hit`；失焦与断开结束悬停。此前由各程序（`Application` 包装、gallery、Vue）各自调用 `dispatch_file_drag`，自己实现 `RuntimeProgram` 的程序若不调用，放置目标就收不到文件。
- **Accessibility action** 仍不走输入路由：AccessKit 动作经 `RuntimeProgram::accessibility_action` 交给 `apply_accessibility_action`。它不是设备输入：没有 source、指针或键盘语义，已有自己的类型化入口；并入路由需要把 runtime 的动作类型搬进平台合同，本轮不做。

## 关联 Issue 合同审计

| 关联 Issue | 当前取证 | 对 #243 的影响 |
| --- | --- | --- |
| #203 LayoutResult / Fragment / Geometry authority（OPEN） | `UiWorld` 的基础 hit index 消费 `LayoutBox`，但 `ComponentGeometry` 仍由 `world::geometry` 派生，并被 Runtime 组件、输入和 A11y 多处读取；未发现已统一的公开 `LayoutResult/Fragment` authority | #250/#251 只能证明当前 Runtime projection 一致，不能宣称已完成 #203 的唯一几何合同 |
| #234 ViewportTransform（OPEN） | 当前仓内未检索到 `ViewportTransform` canonical 类型；原 `PresentationCoordinateBridge` 已删除（没有非恒等变换的生产者） | #251 随 #234 / #247 的真实生产者重新设计 |
| #247 EmbeddedApplication / ExternalSurfaceNode（OPEN） | 当前输入 bridge 有 endpoint/document/generation 字段，但未发现 `EmbeddedApplication` 输入 seam、child presentation metadata producer 或 parent/child delegation fixture | nested NanaUI 的 focus/capture/IME 只能列为未验收 |
| #8 Performance Contract（CLOSED，现行合同仍要求 counter/CI 门禁） | Runtime/Scene 已有 WorkCounters，输入路由有 `InputCounters` 与分配/命中门禁；但本 Issue 的 240/1000Hz native frame counters 与独立 CI threshold 尚不存在 | #253 仍 OPEN；不能用单元 counter 代替 #8 的宿主性能门禁 |
| #242 Presentation Output Epic（OPEN） | 输入侧没有依赖第二棵渲染树，但真实 VisualEndpoint/RenderTarget/Presenter 生命周期尚未作为 adapter 证据接入 | #251/#253 的真实 presentation transform 和多输出验收仍缺 |

### 额外开放 Issue 审计

| Issue | 与 #243 的关系 | 当前决定 |
| --- | --- | --- |
| #174 Platform Input / UI Action / application input 分层（OPEN） | Canonical input 只覆盖 platform event；不能把连续轴、玩家映射或 universal ActionMap 塞进输入路由 | 已遵守边界；gamepad/连续业务输入不纳入本 Epic |
| #180 inert 子树（OPEN） | focus revoke、pointer hit 和 A11y action 的生命周期会影响 fade-out/disabled subtree | 当前路由只处理 endpoint/source 生命周期；inert 语义需作为独立 Runtime follow-up，不能假设已由 capture/focus 清理覆盖 |
| #201 Viewport / Clip behavior（OPEN） | #251 的 Viewport/Content 映射与 scroll-only hit-test、clip chain authority 直接相关 | scroll metrics/clip authority 继续依赖 #201/#203 |
| #206 Layout Foundation counters（OPEN） | #253 的高频输入门禁必须与 layout/measure/reflow counters 共用 #8 catalog | 当前已有 Runtime work counters，但没有完整 #206 structural gate 接入 |
| #214 Dynamic Layout performance（OPEN） | 输入事件风暴不能掩盖 dynamic solver/layout work；需要 cheap path、budget 和 resize-storm 证据 | 作为性能依赖记录，不在 #243 内复制 solver/profiler |
| #271 Windows CJK IME + AccessKit（OPEN） | 直接验证 #252 文本输入槽位（IME 候选框、焦点）与 Accessibility 的真实窗口行为 | 明确列为真实 Windows 验收阻断，headless IME 单测不能替代 |
| #276 Windows stylus（OPEN） | 验证 #249 Pen/pressure/hover/barrel-button 的真实设备 lowering | Canonical 数据模型可表达，但没有真实设备证据，保持 OPEN |
| #277 high-refresh acceptance（OPEN） | 覆盖 #253 的 120Hz、复杂 UI、多窗口、device loss、Windows/Android 平台性能矩阵 | 作为最终性能验收 Issue，不用当前 crate 单测宣称完成 |

这组依赖说明为什么本 Epic 当前保持 PARTIAL/OPEN：输入实现可以复用现有 `UiWorld`/Scene 路径，但关联 Issue 尚未提供所有目标 authority 或真实宿主 fixture。
