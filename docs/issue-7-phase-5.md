# Issue #7 Phase 5：Vue / component consumer 迁移

本阶段在 `2026-08-14` 将 Vue custom renderer 的 retained identity/hierarchy/tag/text/focus/layout/style/input/hit-test 接入 `nana-ui-runtime::UiWorld`。迁移保持现有 Vue hostOps、CSS compatibility、Iced widget 映射、QuickJS/V8 与 DOM facade 行为，不引入并行 DOM tree 或第二 renderer。

## 单一权威状态

`NanaTreeDocument` 现在内含一个 `UiWorld`。以下旧字段已删除：

- Node `parent` / element `children`；
- element `tag`、text/comment content；
- document `focused`；
- document layout `HashMap`。

相应读取全部从 Runtime 的 stable Entity/component/hierarchy 获取，create/insert/reparent/text/remove/focus/layout writeback 全部通过 `MutationQueue`。NodeHandle 仍是 Vue/JS 的 JSON-safe ABI，并无损映射到 `StableNodeId`。

Vue adapter 只保留其确实拥有的 compatibility metadata：namespace、attrs、scope ID、event flags、GPU slot、stylesheet diagnostics、theme 与 viewport。`MessageBridge` 仍缓存 semantic props 和 projection 所需索引，但 `semantic_snapshot` 在交付 renderer 前强制以 Runtime hierarchy 覆盖 parent/children/roots，并移除已不在 Runtime 的 widget，因此 bridge hierarchy 不再是可观察权威。

后续 Phase 9 审计又发现 pointer capture 仍留在 Vue `InputState`，与上述单一权威原则冲突。现已迁入 Runtime：Vue host ops 只提交/query Runtime capture，事件 transition path 使用 Runtime `EventRoute`；`InputState` 仅保留 pressed、hover、composition 与 keyboard 等事件流瞬态状态。

## Style / text / layout / input

- bridge 产出的 canonical `LayoutStyle` 写入 Runtime `NodeStyle`，muted semantics 映射到 semantic foreground；
- interaction 根据真实 widget kind、disabled 与 hidden 状态生成；
- 只在 bridge revision 改变时比较 style/interaction，未变化的 snapshot 直接走空闲路径；
- Iced adapter 使用 Phase 3 `IcedTextShaper` 处理 scheduled text；
- pre-paint measure 与 Iced `LayoutProbe` 继续是既有两个 geometry phase，但结果只写入 Runtime `LayoutBox`；完全相同的 writeback 不产生 mutation/generation；
- hit-test 先使用 Runtime 的 z-index/document-order 紧凑索引，再通过 `LayoutBoxStore` 对 affine transform 做逆变换精确判定；不恢复旧的全 tree/y-coordinate heuristic。

`set_element_text` 与 subtree remove 会一次性销毁 Runtime subtree 并清理所有 Vue compatibility metadata。已销毁 handle 继续遵循 tombstone 规则，不能 ABA 复活。

## 功能门禁

- `nana-ui-vue --features iced-view --lib`：380 项通过，覆盖 hostOps、tree、CSS、layout、scroll、IME、semantic/accessibility projection、Iced widgets、transform writeback、Runtime-owned transformed hit-test 与 authority/idle generation tests；
- `nana-js-quickjs`：14 项通过；
- `nana-js-v8 --features engine`：通过；
- `nana-ui-runtime --all-features`：18 项通过；
- Iced compatibility boundary：通过。

`nana-ui-vue` 全 crate `clippy -D warnings` 仍被 `nana-js-engine` 既有 large-error/len-without-is-empty 和 vendored `arboard` warnings 阻断；本阶段 Runtime 自身 clippy 为零问题，没有为通过门禁扩大修改无关错误类型 ABI。

完整 Vue cross-check 在当前 macOS 环境被目标 C toolchain 阻断：Android 缺 `aarch64-linux-android-clang`，Windows 缺 MSVC headers，Linux 缺 `x86_64-linux-gnu-gcc`；Phase 3/4 backend-neutral Runtime 的三目标 cross-check 已通过。这些结果不等同于真实平台验收。

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

多轮复核修复了：未知 layout handle 被误当有效节点、每帧不变 geometry 重复失效、semantic style 全量重复 commit、旧 hit-test 全 tree heuristic、affine AABB 误命中、Vue `InputState` 独立 pointer capture、`LayoutBoxStore` 二次决定 hit target，以及 bridge snapshot 可越过 Runtime hierarchy 的缺口。Runtime 现在统一处理累计 transform/clip、capture 与 event route；store 仅保留兼容 geometry 查询/offset projection。

当前保留 MessageBridge semantic props 是 Vue L1/L2 compatibility projection 的必要数据，并非第二个 ECS/DOM 权威。Phase 5 已满足 Vue/component consumer 迁移与现有产品行为门禁，可以进入 Phase 6 的 UiScene/RenderGraph；renderer extraction 在 Phase 6 前不伪装成已被 Iced painter 消费。
