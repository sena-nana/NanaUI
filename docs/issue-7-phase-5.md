# Issue #7 Phase 5：Vue / component consumer 迁移

> 状态注记：历史证据。产品 JS 引擎现为单一 V8；`nana-js-quickjs` 已移除。Iced widget 映射已退出产品路径。

本阶段在 `2026-08-14` 将 Vue custom renderer 的 retained identity/hierarchy/tag/text/focus/layout/style/input/hit-test 接入 `nana-ui-runtime::UiWorld`。迁移保持现有 Vue hostOps、CSS compatibility、Iced widget 映射、QuickJS/V8 与 DOM facade 行为，不引入并行 DOM tree 或第二 renderer。

## 单一权威状态

`NanaTreeDocument` 现在内含一个 `UiWorld`。以下旧字段已删除：

- Node `parent` / element `children`；
- element `tag`、text/comment content；
- document `focused`；
- document layout `HashMap`。

相应读取全部从 Runtime 的 stable Entity/component/hierarchy 获取，create/insert/reparent/text/remove/focus/layout writeback 全部通过 `MutationQueue`。NodeHandle 仍是 Vue/JS 的 JSON-safe ABI，并无损映射到 `StableNodeId`。

后续 scroll 审计又移除了 process-global offset map：`ScrollOffsetStore` 的历史名称仅保留 Iced `scroll_to` command queue，不保存状态。`scrollTop` / `scrollIntoView` 写入 Runtime；真实 Iced 滚轮通过 `on_scroll` 同批回传绝对 offset 与 viewport/content metrics，按实际夹取后的增量更新 compatibility paint boxes且不回送命令，避免事件回环与重复平移。Runtime `LayoutBox` 始终保持未滚动坐标，Scene 与 hit-test 直接消费同一 offset。

并行门禁进一步暴露了 process-global `LayoutBoxStore` 会让两个 VueHost 的相同 NodeHandle 互相覆盖。现改为每个 `VueHost` 独立拥有 paint-geometry store，并把同一 Arc 显式传给 Iced probes、DOM geometry host ops、scroll 和 pointer offset 计算；全局 store 只保留给无 Host 的 standalone view compatibility API。两 Host 相同 handle、不同 geometry 的隔离测试和三次并行全套复跑均通过。

Vue adapter 只保留其确实拥有的 compatibility metadata：namespace、attrs、scope ID、event flags、GPU slot、stylesheet diagnostics、theme 与 viewport。`MessageBridge` 仍缓存 semantic props 和 projection 所需索引，但 `semantic_snapshot` 在交付 renderer 前强制以 Runtime hierarchy 覆盖 parent/children/roots，并移除已不在 Runtime 的 widget，因此 bridge hierarchy 不再是可观察权威。

后续 Phase 9 审计又发现 pointer capture、pressed 与 hover 仍留在 Vue `InputState`，与上述单一权威原则冲突。现已迁入 Runtime：Vue host ops 只提交/query Runtime per-pointer capture/press/hover，事件 transition path 使用 Runtime `EventRoute`；`InputState` 仅保留 keyboard repeat 所需的 pressed-key 瞬态集合。subtree 销毁、pointer-events 关闭与窗口 blur 会清理 Runtime interaction target。

继续审计 CJK 输入时发现 `InputState` 仍独立持有 composition/preedit，且 `commit_text` 无视 selection、始终追加到 value 末尾。现已删除这组双权威：Runtime `TextInputState` 持有 committed value 与 UTF-8-safe anchor/focus selection，`ImeComposition` 持有 preedit 与其 selection。synthetic composition 更新 Runtime 后发 DOM event；native IME 先保存平台 selection，再只复用事件转发逻辑，避免 helper 覆盖平台状态。accepted `beforeinput` 才原子提交 replacement，并同步 Vue compatibility attribute；accessibility value 与 render extraction 读取同一 Runtime state。

## Style / text / layout / input

- bridge 产出的 canonical `LayoutStyle` 写入 Runtime `NodeStyle`，muted semantics 映射到 semantic foreground；
- interaction 根据真实 widget kind、disabled 与 hidden 状态生成；
- 只在 bridge revision 改变时比较 style/interaction，未变化的 snapshot 直接走空闲路径；
- Iced adapter 使用 Phase 3 `IcedTextShaper` 处理 scheduled text；
- pre-paint measure 与 Iced `LayoutProbe` 继续是既有两个 geometry phase，但结果只写入 Runtime `LayoutBox`；完全相同的 writeback 不产生 mutation/generation；
- hit-test 先使用 Runtime 的 z-index/document-order 紧凑索引，再通过 `LayoutBoxStore` 对 affine transform 做逆变换精确判定；不恢复旧的全 tree/y-coordinate heuristic。

`set_element_text` 与 subtree remove 会一次性销毁 Runtime subtree 并清理所有 Vue compatibility metadata。已销毁 handle 继续遵循 tombstone 规则，不能 ABA 复活。

## 功能门禁

- `nana-ui-vue --features iced-view --lib`：384 项通过，覆盖 hostOps、tree、CSS、layout、程序化/真实滚动回写、host geometry isolation、CJK selection/IME、semantic/accessibility projection、Iced widgets、transform writeback、Runtime-owned transformed hit-test 与 authority/idle generation tests；
- `nana-js-quickjs`：14 项通过；
- `nana-js-v8 --features engine`：通过；
- `nana-ui-runtime --all-features`：40 项通过；
- Iced compatibility boundary：通过。

`nana-ui-vue` 全 crate `clippy -D warnings` 仍被 `nana-js-engine` 既有 large-error/len-without-is-empty 和 vendored `arboard` warnings 阻断；本阶段 Runtime 自身 clippy 为零问题，没有为通过门禁扩大修改无关错误类型 ABI。

完整 Vue cross-check 在当前 macOS 环境被目标 C toolchain 阻断：Windows 缺 MSVC headers，Linux 缺 `x86_64-linux-gnu-gcc`；Phase 3/4 backend-neutral Runtime portability check 已通过。这些结果不等同于真实桌面平台验收。

## 性能门禁

Vue adapter 报告见 [`performance/2026-08-14-issue7-phase5-vue-runtime.json`](performance/2026-08-14-issue7-phase5-vue-runtime.json)，release，10 次 warmup、60 个样本；所有列均为 P95：

| 节点 | Construction P95 | Initial semantic P95 | Idle semantic P95 | Initial layout P95 | Idle layout P95 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 0.331 ms | 0.246 ms | 0.000 ms | 0.156 ms | 0.005 ms |
| 500 | 0.676 ms | 0.521 ms | 0.000 ms | 0.321 ms | 0.010 ms |
| 1000 | 1.247 ms | 0.926 ms | 0.000 ms | 0.583 ms | 0.018 ms |
| 5000 | 9.999 ms | 4.695 ms | 0.000 ms | 2.988 ms | 0.091 ms |

Idle semantic 与 idle layout 都断言 Runtime generation 不变。Idle layout 仍需比较 Iced 提交的 box snapshot，5000 节点 P95 0.091 ms；没有调度 systems。Initial semantic 同时建立 style、interaction 与 accessibility component，并构建 transform/clip-aware hit index；5000 节点 P95 为 4.695 ms。Construction 是逐 hostOp 的冷 mount 成本，不是帧内常驻成本；常驻空闲路径保持零 semantic commit。报告 schema v2 修复了此前 initial/construction 只有单次冷样本、数字不可稳定比较的证据缺口。

完整 Iced 报告见 [`performance/2026-08-14-issue7-phase5-iced.json`](performance/2026-08-14-issue7-phase5-iced.json)。关键场景对 Phase 0：

| 场景 | Phase 0 Total P95 | Phase 5 Total P95 | Phase 5 P99 |
| --- | ---: | ---: | ---: |
| list-1000 | 1.252 ms | 1.259 ms | 1.300 ms |
| gallery-controls | 1.875 ms | 1.511 ms | 1.528 ms |
| workspace-20-regions | 6.382 ms | 6.358 ms | 6.378 ms |
| workspace-50-regions | 16.200 ms | 16.162 ms | 16.174 ms |

关键红线场景未回退；这些是 offscreen WGPU 结果，不代替真实窗口验收。

## 阶段复核

多轮复核修复了：未知 layout handle 被误当有效节点、每帧不变 geometry 重复失效、semantic style 全量重复 commit、旧 hit-test 全 tree heuristic、affine AABB 误命中、Vue `InputState` 独立 pointer capture/press/hover、process-global scroll offset、process-global host geometry、Iced wheel 未回写 Runtime、重复 scroll translation、`LayoutBoxStore` 二次决定 hit target，以及 bridge snapshot 可越过 Runtime hierarchy 的缺口。Runtime 现在统一处理累计 transform/clip/scroll、per-pointer interaction、capture 与 event route；window-local store 仅保留兼容 geometry 查询和 paint-phase projection。

当前保留 MessageBridge semantic props 是 Vue L1/L2 compatibility projection 的必要数据，并非第二个 ECS/DOM 权威。Phase 5 已满足 Vue/component consumer 迁移与现有产品行为门禁，可以进入 Phase 6 的 UiScene/RenderGraph；renderer extraction 在 Phase 6 前不伪装成已被 Iced painter 消费。

后续 Epic 审计已补入 Nana-native `Text` / `Button` / `TextInput` / `List` / `Table` retained primitives 与 typed activation/text-change 入口，证明 Rust component 可直接落到同一 UiWorld/UiScene，不要求全局 Message enum。Theme mode 与 semantic foreground/background/border 也已进入 Runtime computed style，Vue snapshot 只同步同一 authority。它们尚未覆盖完整 control interaction state、gallery 新旧 backend parity与可见 painter，因此这里将原“component consumer 迁移已满足”的表述收窄为 **Vue retained consumer 已满足，完整 component migration 仍进行中**。

Professional primitive 审计没有复制新列表模型：既有 `VirtualListLayout` 已从 Iced-dependent `nana-ui` 下沉到 `nana-ui-core`，旧路径仅 compatibility re-export。list/table 共用 keyed、revision-fenced 两阶段 materializer；AppContext 以单次 Runtime commit 构建可见 component、销毁离窗 subtree 并保留 overlap Entity，同时验证 typed view 与目标父节点所有权。10k list 约 60 rows 的交替窗口 materialization P95 0.043 ms、P99 0.046 ms；10k × 100 table 约 60 × 20 可见交集的二维物化 P95 0.844 ms、P99 0.874 ms。`VirtualTableLayout` 继续提供有界/resizable column model、二维 row/column window 与 clamped keyboard cursor navigation；Table/Row/Cell/ColumnHeader 进入 Runtime accessibility hierarchy。`RuntimeInputAdapter` 已把稳定 platform wheel/keyboard 输入接到最近 ScrollView 与 focused Table：内层滚动到边界后向外层冒泡，只有真实处理的导航键才 prevent-default；Winit named key 不再被 `to_text()` 丢失。本阶段当时缺少的真实平台交互与 accessibility 证据已在 Phase 9 由 macOS 输入、AX/VoiceOver 与 dark/light Runtime Scene 补齐；Windows/Linux 按用户要求暂缓。
