# Issue #7 Phase 9：macOS milestone acceptance

本阶段在 `2026-08-14` 对 Phase 0–8 的实现和证据做 milestone 审计。结论是：**当前 macOS 范围的 Nana-owned Runtime / UiScene、组件、IME、accessibility 与 Live2D composition 已建立并验收；Iced/WGPU compatibility backend 仍是当前最佳平台与绘制实现，因此不删除。** Issue 原始 DoD 继续保持 OPEN；Windows/Linux 验收暂缓，Android 产品范围排除。当前结果只表示 macOS 阶段可用，不等于 Epic 完成，也不把暂缓平台标记为通过。

## 退出门禁

| 门禁 | 当前结果 | 证据或剩余缺口 |
| --- | --- | --- |
| Runtime coverage | macOS milestone 通过 | identity、hierarchy、style/theme/interaction、text、committed selection/preedit、layout/scroll、hit-test、pointer hover/press/capture/event route、focus/IME、animation/deadline、accessibility、render content 已有单一权威；canonical `RuntimeDocument::flush` 统一消费 systems，由 host text shaper 与 framework `RuntimeLayoutEngine` 完成 text/layout writeback，fixture 不再手写控件坐标。`RuntimeProgram`/`run_runtime` 是不暴露 Iced Message/Element/window ID 的 application host；`HostedProgram` 保留为 compatibility contract。 |
| Component parity | 部分完成 | `nana_ui::runtime` 提供 Nana-native retained primitives、typed events、Runtime theme/interaction/overlay authority、UiScene extraction、标准 Quad/Text/HostTexture painter，以及 keyed list/table materializer、O(log n) geometry、platform pointer/wheel/keyboard dispatcher 与 keyboard cursor。RenderGraph 现按标准/custom segment 产生显式 resource pass，注册的 `SceneGpuRenderer` 可直接使用当前 encoder/target。Workspace、Sidebar、SplitPane 与 Dock 专业 view 仍保留为 Iced compatibility adapter，完整外部消费应用的纯 Runtime gate 未完成。 |
| IME parity | 当前 macOS 门禁通过 | Runtime 有 backend-neutral composition/selection 生命周期及 CJK deterministic tests；macOS 26.5.2 的真实拼音 preedit、候选窗定位、commit、Vue v-model/AXValue 同步已通过。Windows/Linux 实机验收按用户要求暂缓，不阻塞当前 macOS 收口。Android 不属于 NanaUI Issue #7 的平台范围。 |
| Accessibility parity | 当前 macOS 门禁通过 | Runtime role/state/tree/bounds/focus/committed text selection projection 与 updated/removal delta 已补齐，并由 Vue semantic props 驱动；desktop hosted window 已在首次显示前安装 AccessKit adapter，按稳定 ID 增量同步并把受支持的 Focus/Click/SetValue/SetTextSelection 动作送回同一 Runtime/Vue 事件路径。macOS Accessibility Inspector、系统 AX Focus/SetValue/SetTextSelection/Press 与 VoiceOver 读取/键盘导航/激活已通过；NVDA/Orca 验收随 Windows/Linux 一并暂缓。 |
| Vue fixture parity | 通过 | `nana-ui-vue --features iced-view --lib` 389 项功能测试通过，UiWorld 是 retained authority；Iced wheel feedback 与程序化 scroll 共用 Runtime offset/metrics，paint geometry 按 VueHost 隔离。 |
| Desktop coverage | 当前范围通过 | macOS 真实窗口、输入、拼音 IME、AX 与 VoiceOver 已通过；Windows/Linux 按用户要求暂缓，其结果不由 macOS 或交叉编译代替。 |
| Performance no regression | 当前 macOS 门禁通过 | Phase 0/5 Iced 离屏红线未回退；同轮 release 门禁中 Runtime 5000 节点首批 systems P95 2.754 ms、idle 0，leaf paint mutation 固定 1 个 work node且 systems P95 0.003 ms；framework-owned 5000 节点 canonical layout P95 2.215 ms；Vue 5000 节点 construction P95 3.388 ms、idle semantic 0。Apple M4 / Metal 上真实 `live2d-wgpu` renderer 通过显式 `PrepareExternal` graph pass 与 HostTexture sample 合成，36-drawable synthetic workload 的 composed total P95 2.668 ms、CPU P95 0.228 ms。 |
| WGPU / native decision | 通过 | Phase 0/7 的证据支持保留 WGPU；五项 RHI 重开条件未满足，不实现 Nana Native RHI。 |

CPU Runtime/Scene 与 macOS UI/Live2D workload 已加入每周及手动触发的
`runtime-performance.yml`：前者持续约束 idle、局部 mutation、virtual list、Vue 与 frame
graph 和 5000 节点 canonical layout，后者运行真实 WGPU UI-only/Live2D-only/composed
workload并保存 JSON/截图。Live2D producer 现由 RenderGraph 的 `PrepareExternal` operation
管理，每个 preparation pass 使用 host-owned encoder，并由同一 Queue 在 UI sample 前有序提交；这仍不是
直接写当前 Surface target 的 `SceneGpuRenderer` pass，也不替代外部消费应用的真实验收。
首次执行该门禁定位并修复了 single-node create/append 验证每次扫描全 world、复制增长中
sibling list 的 O(n²) 根因；修复后本机 Vue 5000 节点 construction P95 为 3.477 ms，
idle semantic 仍为 0。报告保留在临时目录，本轮未覆盖历史 baseline。
最终同轮 Apple M4 / Metal 三组轮转 workload 的 UI-only total P95 为 1.898 ms、Live2D-only
total P95 为 0.574 ms、graph-managed HostTexture composed total P95 为 2.668 ms，截图 442 个
distinct colors；该临时报告不覆盖历史 baseline。

## 本阶段补齐的缺口

审计发现 Runtime 原先没有 backend-neutral accessibility 数据，导致 retained authority 在语义树处中断。本阶段加入：

- `AccessibilityRole`、`AccessibilityState` 与稳定 ID 的 `AccessibilityNode` 投影；
- role、label/value、disabled、checked、selected、focused、bounds 与 hierarchy；
- comment 排除、隐藏状态过滤、文本默认 label，以及 accessibility 独立 dirty work；
- subtree tombstone removals 与父节点 children 更新组成同一 generation delta；
- Vue widget semantic props 到 Runtime accessibility component 的映射；
- 全量 snapshot 用于首建，dirty-node projection 用于平台 adapter 的增量消费。

继续对照 Issue 原文审计时又发现 Vue `InputState` 独立持有 pointer capture、press 与 hover。该双权威已删除：Runtime 现在拥有 document-scoped per-pointer hover/press/capture、原子替换/释放、subtree/disabled/blur 失效、capture change stream 与稳定 event route；Vue host ops 只是 compatibility adapter。hover/pressed/focused/disabled semantic paint 由同一 style system 解析，old/new target 只调度至多两个节点；5000 节点 transition P95 为 0.000 ms。

AnimationSystem 现由 Runtime 持有 stable animation identity、目标 Entity、easing、deadline 与 active lifecycle；start/replace/stop 进入同一原子 mutation batch，subtree 销毁自动取消。host 从 `AppContext` 查询下一 deadline 并以显式单调时间采样，静态 UI 不轮询，采样不强制 Live2D 或整棵 UI redraw。现有 Iced-local component animation 尚未迁移，因此该 Runtime 完成项不被提升为 component/backend parity。

`RuntimeAnimationClock` 已把 Runtime duration epoch 接到 hosted `Instant` wakeup；它只转换 deadline/采样，不拥有 timer 或 redraw policy，因此可与 GPU、Web API、Live2D cadence 独立合并。

Workspace 阶段复核将持久 layout、viewport/scale、resize interaction 和 collapse transition 统一到 `nana-ui-core::WorkspaceModel`。模型以显式 `Duration` 驱动，反向动画从当前 extent 连续重定向，静态 geometry 不复制 layout；Iced `WorkspaceController` 只保留 window/frame subscription 与 view adapter。最终 20/50 region 复测的 CPU total P95 分别为 0.087/0.214 ms，对应 Phase 0 的 0.090/0.218 ms；total P95 为 6.345/16.199 ms，对应 6.382/16.200 ms，未见回退。Gallery workspace CPU total P95 为 0.069 ms，对应 0.072 ms。该结果不把完整 gallery painter 提升为已迁移。

同一阶段随后把 Sidebar disclosure 的 bool/transition 下沉到 `ExpansionState`，并把 SplitPane constraints/persistence/focus/hover/absolute resize 全部下沉到 `SplitPaneModel`。compatibility controller 不再持有第二份状态；SplitPane Reset 在默认尺寸下仍会正确报告被清除的 focus/drag 状态。

Dock 的时间审计已把 insertion dwell 从 Iced `Instant` 改为 controller monotonic `Duration` epoch，并增加 deterministic entry；frame subscription 使用明确的 `AdvanceDragDwell`，不再以 `Hover(false)` 伪装时钟 tick。合同复核确认 `DockUpdate.changed` 是 persisted `DockLayout` dirty，而非任意视觉变化，因此 dwell/preview 保持 false，避免消费应用在每次预览停留时误写配置；input/frame host 仍负责该次重绘。

随后加入 backend-neutral `DockMutation` / `LogicalPoint` 与 `update_mutation[_at]`：active drag、cross-surface hit-test、preview bounds 和 split resize 不再保存 Iced Point，resize 以初始 ratio + absolute scalar delta 计算并保留越界重入语义。Iced `DockAction` 只作为兼容转换层。Workspace/Split/Dock 原先共用、现在已无消费者的 Iced `ResizeDrag` 被删除；现有 resize、跨窗口 source/hover surface、Tab 与 edge placeholder 行为测试继续通过。

本轮继续补充 `DockController::surface_layout`，从同一 authority 投影 main/floating surface 的 active item content bounds、tab group 和 stable splitter path/hit bounds；主 Dock chrome 与 floating native title bar 的不同高度由 controller 统一处理。该 API 消除了纯 Runtime 消费应用重写 split/chrome 几何的需要，但不把尚未完成的外部 consumer gate 标记为完成。

Text input 审计补齐 committed value/selection authority：Runtime 使用 UTF-8-safe anchor/focus selection，preedit 与 committed text 分离，Vue selection replacement 不再无条件追加，native preedit selection 不再被事件 helper 丢弃。桌面 hosted adapter 继续只消费 winit 已实际提供的 Enabled/Disabled/Preedit/Commit，并保留 preedit selection；没有为未接入的平台预埋完整 editor snapshot 分支。macOS 的真实候选窗与 caret candidate rectangle 已通过，Windows/Linux 仍是平台验收缺口。

阶段复核又删除了无人消费的 `ImeRequest` / `ImeHost` / `UnsupportedIme` 预埋接口及其无功能测试。桌面真实输出路径仍由 hosted renderer 从 Iced `InputMethod` 状态调用 winit 的 `set_ime_allowed`、`set_ime_cursor_area` 与 `set_ime_purpose`；删除空抽象不改变启停、候选窗锚点或 purpose 行为。

Android 已明确排除在 NanaUI 当前产品范围之外。历史 Android host 只保留在 `experimental-android` feature 隔离层；默认 `nana-ui-platform` API 不再公开 `PlatformCapabilities`/`SurfacePhase` 来声称不存在的产品能力。未来若产品恢复 Android，边界必须是 Android 原生组件拥有 Activity、IME、Accessibility、原生控件与生命周期，NanaUI 仅作为可嵌入渲染内容参与混合合成，并通过平台中立契约交换状态、事件或纹理。

本轮架构收口另加入稳定 `WindowGeometry`、`WindowEvent`、`TextInputRequest` 平台合同，winit/AccessKit 保持 adapter 实现；`AppContext::world_mut` 已收为 crate-private，adapter 只能通过 commit、work drain、style/layout、hit-test 与 projection 的窄接口工作。`ComponentView` 更新先在 staged clone 上执行 handler 与投影，UiWorld commit 成功后才发布 typed state，失败不会推进 generation 或改变 Scene。

Pointer/wheel/keyboard event、modifier、pointer phase/type 与 input disposition 已从 Iced hosted runtime 下沉到 `nana-ui-platform`，旧 `Hosted*` 名称仅 re-export；winit conversion 留在 compatibility adapter。`RuntimeInputAdapter` 将 wheel 命中/边界冒泡和 focused table navigation 接到 AppContext，Winit named key 也保留稳定名称；这些 deterministic tests 不替代真实触控板、键盘布局和窗口验收。Window/surface lifecycle、platform accessibility 与真实 IME adapter 仍未完全脱离 hosted compatibility 层。

Platform core 与 fetch/clipboard 已解耦为 features，默认能力不变；`--no-default-features` 的 Core/Runtime/Scene/Platform core 保持 backend-neutral。跨平台编译只报告 contract 可移植性，不提升为目标桌面平台运行验收。

Canonical `nana_ui::runtime` component API 已加入 backend-neutral Text/Button/IconButton/Card/TextInput/TextArea/Checkbox/Switch/Slider/Tab/ScrollView/List/ListItem/Table/Row/Cell/OverlayHost/Dialog/Menu/MenuItem/Tooltip、hierarchy append、typed activation/change 与增量 projection；控件与单/多行 UTF-8 selection 都是实际接入的 closure-event 路径，不是展示型空壳。IconButton 的 glyph/accessibility label 分离，Card 不抢占 child action，ListItem 的 selected/disabled/activation 进入同一 retained state。Overlay exclusive active/focus restore 只存于 UiWorld，非 active subtree 不进入 layout/input/render/accessibility；modal focus、pointer capture release、close policy、非法 reparent 与 active destroy lifecycle 都有功能门禁。Runtime components 已通过 computed theme paint、UiScene 到 `IcedSceneView` 的标准绘制链；不支持的 affine/letter-spacing/named-font/custom 样式显式失败，禁止假兼容。Component Gallery runner 已生成并人工检查覆盖主要 normal/selected/disabled 状态的 native dark/light scene；现有完整 Gallery 继续作为 compatibility asset 与视觉参照，不需要为形式上的“全量重写”复制已有专业组件逻辑。

Accessibility 可达性复核补齐 hidden/overlay 子树事务：visibility 变化会向子树传播 style/accessibility dirty，并显式调度边界父节点；overlay active 变化调度 host。投影把本 generation 中已不可见的 dirty 节点转成整棵子树 tombstone，同时更新边界 children；恢复时同一 transaction 重建父连接和所有后代。普通文本、选区或 value 更新不会因此额外重投影父节点。

既有 virtual-list geometry 已下沉至 `nana-ui-core`，`nana-ui` 保持兼容 re-export；Fenwick 索引提供 O(log n) visible window/range/single-measurement update。list/table 共用两阶段 revision plan 与单次 Runtime commit，保留 overlap Entity 且只 retained 可见窗口：10k list 约 60 rows 的交替窗口 P95 0.043 ms、P99 0.046 ms；10k × 100 table 约 60 × 20 cell 可见交集的二维物化 P95 0.844 ms、P99 0.874 ms。Table 列宽限制/resize 与 keyboard cursor navigation intent 已接到 Runtime hierarchy/focus、typed event 和 platform input adapter。Theme mode、semantic interaction paint、scroll offset/metrics 已由 UiWorld 解析，Vue theme/hover/press/scroll 同步不再形成第二份 authority；40 个可见 retained 节点的 scroll mutation P95 0.003 ms、P99 0.004 ms且无 layout work。真实目标平台输入和 accessibility E2E 仍是 component gate。

Desktop accessibility adapter 通过当前 `accesskit 0.24.1` / `accesskit_winit 0.33.2` 接入 hosted primary/auxiliary window：窗口先以 invisible 创建，adapter 必须在首次 show 前安装，每个 `WindowEvent` 在应用处理前送入 adapter。平台树来自 `HostedProgram::accessibility_snapshot`，adapter 缓存增量投影，同时独立保存最新完整树，保证辅助技术 deactivate 后再次 activate 仍能同步返回当前 generation，而不是只返回首次快照；tombstone 会同时修正父 children，root 更换发送完整 tree。依赖解析固定为与 Iced 共用的单一 `winit 0.30.8`，没有第二套窗口类型，也没有启用 AccessKit Android adapter。

阶段性能复核把 Runtime 已有的 accessibility dirty/tombstone 事务真正接到 hosted adapter：Vue 文档按 stable ID 合并未消费变化，静态重绘返回 `None`，不再每帧构造全树并做 O(n) diff；窗口首次建树会先排空累计 delta，避免初始完整树后重复发布。累计变化超过 4096 个时清空集合并退化为一次完整快照，因此非 hosted 或暂时无消费者的文档不会无限积累。增量和 retained 完整树携带单调 generation，adapter 拒绝重复或乱序事务，避免旧更新把平台树回滚；snapshot-only host 使用无 generation 的兼容全量更新。DPI 改变仍发送当前完整树；即使 host 同时交付旧 generation，projector 也只用已保留的最新语义重算物理 bounds，不接受旧节点内容。非 retained 的通用 `HostedProgram` 默认继续走全量兼容路径。

动作能力不被虚标：默认 HostedProgram 只投影只读语义；只有显式启用并实现 `accessibility_action` 的程序才声明动作。Vue hosted 对 enabled Focus/Click、真实 editable TextInput 的 SetValue，以及已有 committed selection 的 TextInput/TextArea 声明 SetTextSelection；ARIA `textbox` 本身不会被推断为可写，disabled/readonly 输入也不会声明 SetValue。请求跨线程排队并唤醒窗口，在主线程按 root tree + stable ID 更新 retained focus，或走既有 Bridge/DOM click、`beforeinput → committed TextInputState → input`、selection `select` 路径；文本 Focus 还通过同一稳定控件 ID 同步 retained editor keyboard focus，不另存业务 focus。通用 `VueHostedProgram`、`vue-hosted-acceptance` 与 `vue-counter` 窗口包装器都显式委托同一 snapshot/action helper，避免真实二进制因包装层默认空实现而丢失语义。

动作回调队列有界为 256 项；满载时拒绝最新请求并保留已接受 FIFO 顺序。每个成功入队都会请求 redraw，而满载意味着队列中已有请求已安排排空，因此溢出不会取消既有唤醒，也不会让平台线程上的内存无界增长。

AccessKit 文本模型使用每个 TextInput 的稳定合成 TextRun；character length 与 Runtime 实际允许的 UTF-8 `char` boundary 一致，anchor/focus byte offset 与平台 character index 双向校验转换。合成 ID 从未占用的高位向下分配；若后续 Runtime ID 碰撞，会在同一 update 中重键 TextRun 并替换父 children。当前没有从总宽度猜测 character positions/widths；精确字符像素几何须由后续 text shaping contract 提供，未实现前不伪造该能力。

`UiWorld` 允许一个 document 暂时存在零个或多个 retained roots，而 AccessKit Tree 必须声明一个 root。单根窗口继续直接使用 Runtime stable ID；零根或多根时 adapter 使用 Runtime 永远不会分配的 `NodeId(0)` 作为仅平台可见的 GenericContainer，并按 stable ID 顺序引用全部根。默认程序的首次空快照仍表示不安装 adapter，且后续 redraw 不再无意义地消费空 accessibility update；异步 retained 程序可通过显式 `accessibility_adapter_enabled` opt-in 从 empty forest 启动，Vue 通用程序与两个真实 wrapper 均已委托。adapter 构造只接受已经规范化的 full generation + nodes，是否安装只由窗口宿主决定，不在 adapter 内重复表达失败分支。adapter 建立后的空更新切换到 empty forest，禁止保留旧根。根集合增删发送完整 tree，在 empty forest、单根与 multi-root forest 间原子切换；合成 root 不进入 Runtime action target，因此不能形成第二份业务节点权威。

本机重新审计发现旧 AX 结论来自探针错误：Spotlight 未索引 Xcode 包内实际存在的 Accessibility Inspector，旧脚本也把 Core Foundation 返回对象误读为 application。macOS 26.5.2 上，官方 Inspector 与 `System Events` 均能定位 `AXTextField`/`AXButton`，读取真实 value/focus/bounds，并通过 AX Focus、SetValue 与 Press 回流同一 Vue/Runtime；Press 打开真实 dialog。VoiceOver 首次启用后可读取“你好, text field, 有键盘焦点”，Shift+Tab 导航到 “Open dialog, button”，Control–Option–Space 激活并打开 dialog。验收后已关闭本轮启动的 VoiceOver、Inspector 与 acceptance，不保留系统辅助功能开关。

真实拼音验收同时发现并修复了此前 deterministic test 未覆盖的双权威：`Ime::Commit("你好")` 后 Iced 确实发出 `BridgeEvent::Input`，但 Bridge 只更新 widget props，DOM `input` 读取旧 `event.target.value`，导致可见输入/AXValue 为“你好”而 Vue 仍显示旧值。统一的 `BridgeEvent::Input` 入口现在先把完整 editor snapshot 提交到 Runtime `TextInputState` 与 DOM value，再发 `input/update:modelValue`；Runtime 通过新旧值的最小 UTF-8 变更区间推断 caret。复验候选窗锚定输入框下缘，提交后输入框、`Hello, 你好.` 与 AXValue 三者一致且只提交一次。

SetTextSelection 实机复核又暴露了 adapter 边界的第二处双权威：系统 AX 已回读 selection `0:4`，Runtime 也已提交该范围，但 retained Iced editor 仍保留旧 caret，下一次真实拼音提交因此错误追加为 `NanaUI你好`。hosted UI command 现在用稳定文本控件 ID 把 Runtime 的 UTF-8 byte anchor/focus 转换为 Iced 的多行 byte position，并在同次 rebuild 后同步 retained editor selection；该命令不拥有业务 selection。复验以 AX 选择 `Nana` 后直接输入拼音，实际输入框变为 `你好UI`，Vue 文本同步为 `Hello, 你好UI.`，证明平台动作、Runtime、editor 与 Vue 已收敛到同一选择状态。

## 验证结果

- `nana-ui-core --locked`：66 项通过；
- `nana-ui --lib --no-default-features --locked`：110 项通过；
- `nana-ui-runtime --all-features --locked`：43 项通过；
- `nana-ui-platform --all-features --locked`：14 项通过；
- `nana-ui-platform --no-default-features --locked`：3 项通过，renderer-neutral input/IME/window identity 不依赖 clipboard 或 Fetch/TLS；
- `nana-ui-scene --all-features --locked`：9 项通过；
- `nana-ui-vue --features iced-view --lib --locked`：389 项通过；
- `nana-ui-vue --features hosted --lib --locked`：391 项通过（包含 UTF-8/多行 selection 到 retained editor position 的转换门禁）；
- `nana-ui --features hosted --no-default-features`：152 项通过（含 Runtime pointer/keyboard/IME 路由，以及 AccessKit 全量/增量、generation 防回滚、hidden tombstone、empty/single/multi-root replacement、异步空树 opt-in、DPI、重复 activation 最新完整树、editable/read-only action、有界 FIFO action queue、Unicode TextRun selection round-trip、合成 ID collision 与 TextRun 角色/删除生命周期门禁）；
- `runtime-host-fixture --locked`：编译通过，fixture 的直接依赖不包含 Iced，应用仅实现 `RuntimeProgram` 并持有 `RuntimeDocument`；macOS 原生进程与窗口已创建且持续 present，无崩溃。当前系统截图权限只返回全黑桌面帧，因此不把这次启动提升为人工视觉或 IME/VoiceOver 复验；
- Runtime mixed Scene Live2D acceptance：Apple M4 / Metal，20 次预热 + 80 次交错样本，Quad → HostTexture → Text 通过同一 `RuntimeDocument → UiScene → IcedSceneView` 绘制；composed total P95 1.628 ms、P99 1.733 ms，CPU P95 0.248 ms，截图 455 个 distinct colors。临时报告写入 `/tmp`，未覆盖既有 baseline；
- `vue-hosted-acceptance --bins`：2 项通过，真实窗口 wrapper 的 Vue accessibility snapshot/action 委托在编译与功能测试中生效；
- Runtime/Scene 与 `nana-ui --features hosted --no-default-features` 的 `clippy -D warnings`：通过；hosted runner 仅在 Loading 阶段装箱大体积 window settings，避免整个事件循环状态枚举被冷路径数据撑大，Ready 热路径不增加间接访问；
- Core/Runtime/Scene/Platform core 的 backend-neutral 构建：通过；
- hosted Vue/Iced/AccessKit library check：通过；AccessKit 与 Iced 解析为同一个 `winit 0.30.8`，仅有 vendored `arboard` 既有 warnings；
- `nana-ui --features hosted --no-default-features` 的 `x86_64-pc-windows-msvc` 与 `x86_64-unknown-linux-gnu` 交叉 `cargo check`：通过。该门禁只证明目标平台条件编译与依赖边界，不替代 Windows/Linux CJK、NVDA 或 Orca 实机验收；
- `nana-ui-platform` 的 clipboard 与 Fetch 已拆成独立 features；`nana-ui` hosted 只启用实际使用的 clipboard，Web API owner 才启用 clipboard + Fetch，GUI 平台路径不再因无关 TLS transport 拉入 `ureq/ring`；
- 当前 active Xcode 未接受 license，直接链接会被 `xcrun` 拒绝；本轮使用已安装的 `/Library/Developer/CommandLineTools` SDK 通过显式 `DEVELOPER_DIR` 完成验证，没有修改系统 developer directory，也没有代替用户接受许可；
- Iced dependency boundary：通过；`nana-ui-runtime` / `nana-ui-scene` 不依赖 Iced、WGPU 或平台 GPU API。
- Workspace/Dock snapshot runner：通过；人工检查 dark/light 的 floating merge、drag window、四向 split、Tab、retarget、outside 与完整 Gallery workspace preview，未见状态迁移引入的视觉回退；输出仅写入 `/tmp`，未覆盖用户 baseline。

最新机器可读性能报告为：

- [`performance/2026-08-14-issue7-phase3-runtime.json`](performance/2026-08-14-issue7-phase3-runtime.json)
- [`performance/2026-08-14-issue7-phase4-framework.json`](performance/2026-08-14-issue7-phase4-framework.json)
- [`performance/2026-08-14-issue7-phase5-vue-runtime.json`](performance/2026-08-14-issue7-phase5-vue-runtime.json)
- [`performance/2026-08-14-issue7-phase6-scene.json`](performance/2026-08-14-issue7-phase6-scene.json)
- [`performance/2026-08-14-issue7-workspace.json`](performance/2026-08-14-issue7-workspace.json)
- [`performance/2026-08-14-issue7-live2d-composition.json`](performance/2026-08-14-issue7-live2d-composition.json)

这些结果证明 backend-neutral core 的确定性、增量边界、当前 macOS 真实桌面/辅助技术，以及真实 Live2D WGPU renderer 到 NanaUI host texture 的合成性能。合成 Live2D 模型负载不替代具体授权产品模型验收；Windows/Linux 仍未验收且已从当前门禁暂缓。

## Epic DoD 结论

已建立的根架构包括稳定 Runtime identity/ECS、原子 mutation、typed application API、active-only AnimationSystem、Vue retained authority 迁移、UiScene/RenderGraph 与同 WGPU context 的 custom texture 合成。Native RHI 和 Live2D native backend 经证据门禁得出 NO-GO，正确完成方式是不增加冗余后端。这里不再把 Phase 0–8 的阶段性实现等同于 Epic 全部 DoD；后续仍须逐项核对 compatibility animation migration、professional component、platform 与真实 workload 要求。

当前 macOS 产品范围内，macOS IME/VoiceOver、Nana-owned Runtime/UiScene、canonical `nana_ui::runtime` component API、基础 Quad/Text painter、desktop adapter 接线，以及真实 Live2D WGPU renderer 的共享纹理合成均已不再是缺口。Windows/Linux 真实 IME/辅助技术按用户要求暂缓，不计入当前 macOS 门禁，也不被误报为已通过；Android 明确排除。授权产品模型不是 framework owner 的内置资产，后续如有具体产品性能门禁，应以外部输入复跑当前 harness。

因此本阶段结论为 **macOS milestone accepted / Epic OPEN / Native RHI NO-GO**：保留 Iced/WGPU compatibility backend 是 benchmark、功能和维护成本共同支持的实现选择，但应用主合同已经是 `RuntimeProgram + RuntimeDocument + UiScene`。Issue 只有在外部消费应用完成 Dock、HostTexture、输入、多窗口与 Accessibility 的纯 Runtime gate，并补足原始平台验收后才可重新判断关闭；不得把本 milestone 当作 Epic complete。
