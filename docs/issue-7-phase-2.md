# Issue #7 Phase 2：UiWorld / Entity-backed Tree

本阶段在 `2026-08-14` 建立独立 `nana-ui-runtime`。范围只包含 retained identity、hierarchy、基础 component storage 和 frame mutation boundary；Vue 节点数据迁移、增量 systems、layout 与 renderer extraction 分别留在后续阶段。

## Gate A：ECS 选择

采用 `bevy_ecs 0.19.1`，关闭默认 feature，仅启用 `std`。不依赖 Bevy App、Window、UI、Renderer、Asset 或 Scene。

| 方案 | 优点 | 缺口 | 决策 |
| --- | --- | --- | --- |
| `bevy_ecs` minimal | generational Entity、archetype component storage、change tracking、query/system 基础和后续 plugin 扩展成熟 | 依赖与编译成本高于手写 arena；hierarchy 仍需 Nana 定义 | 采用，隐藏在 `UiWorld` 内部 |
| custom hybrid ECS | 最小体积，可针对 UI hierarchy 定制数据布局 | 需要重新实现 generation、component registration、change ticks、query/schedule 和扩展安全性 | 不采用；这些不是 NanaUI 的差异化价值 |
| 轻量 ECS crate | 可能比 Bevy 小 | 仍需额外 hierarchy/change tracking/plugin 设计，且迁移收益没有当前证据 | 暂不引入第二候选依赖 |

对外只暴露 `StableNodeId`、`DocumentId`、`NodeKind`、`NodeSnapshot`、`MutationQueue` 和 Nana 错误类型。`bevy_ecs::Entity/World/Component` 均不出现在公共签名，因此未来仍可替换内部实现。

## 权威状态

`UiWorld` 是 identity/hierarchy 的唯一权威：

- `Identity`、`Kind`、`Hierarchy` 分别存入 ECS component storage；
- `StableNodeId -> Entity` 映射只在 runtime 内部；Entity generation 不成为 JS、snapshot 或持久化 ABI；
- parent 使用稳定 ID，children 使用有序 `Vec<StableNodeId>`，保证 document order；
- 跨 document parenting、循环、无效 `before` anchor 被拒绝；
- `Remove` 只分离节点，允许保留 subtree 后重新插入；`DespawnSubtree` 销毁整棵子树；
- 已销毁 ID 进入 tombstone，不能在当前或以后批次复用，旧 handle 不会 ABA 指向新 Entity。

现有 Vue `NodeHandle(u64)` / `DocumentId(u64)` 已提供到 runtime stable ID 的无损转换。`NanaTreeDocument` 仍保持原有权威状态，直到 Phase 5（对应 Issue #6 的 Vue migration）一次性迁移；本阶段不双写两棵 retained tree。

## Mutation 原子性

`MutationQueue` 支持 create、insert/reparent、remove、subtree despawn。`UiWorld::commit` 先按命令顺序验证完整批次，再修改 ECS World；任一命令失败时整批不生效。

为避免每帧复制整棵树，验证计划只懒加载被命令直接访问或 cycle walk 经过的节点。阶段复核中曾测得全树复制会让“小 mutation / 大 world”退化，因此已在进入下一阶段前移除；现有 5000 节点世界中的一次 sibling reorder 不随总节点数线性增长。

## 性能基线

机器可读报告见 [`performance/2026-08-14-issue7-phase2-runtime.json`](performance/2026-08-14-issue7-phase2-runtime.json)。Apple M4 release，10 次 warmup、60 个样本；initial workload 创建平衡二叉层级，steady workload 在既有 world 中交替重排两个 sibling。

| 节点 | Enqueue P95 | Initial commit P95 | Initial P99 | Steady reorder P95 | Steady P99 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 0.005 ms | 0.158 ms | 0.165 ms | 0.002 ms | 0.003 ms |
| 500 | 0.014 ms | 0.513 ms | 0.585 ms | 0.001 ms | 0.001 ms |
| 1000 | 0.014 ms | 0.670 ms | 0.672 ms | 0.001 ms | 0.001 ms |
| 5000 | 0.059 ms | 2.921 ms | 2.932 ms | 0.001 ms | 0.001 ms |

benchmark feature 才启用 Serde/JSON；生产 `nana-ui-runtime --no-default-features` 的直接依赖只有 `bevy_ecs`。基准可执行文件为 835648 bytes，该数字不是最终应用体积。

复测命令：

```bash
cargo test -p nana-ui-runtime --locked
cargo run --release -p nana-ui-runtime --bin nana-runtime-benchmark --features benchmark --locked -- --output docs/performance/2026-08-14-issue7-phase2-runtime.json
cargo check -p nana-ui-runtime --no-default-features --locked --target aarch64-linux-android
cargo check -p nana-ui-runtime --no-default-features --locked --target x86_64-pc-windows-msvc
cargo check -p nana-ui-runtime --no-default-features --locked --target x86_64-unknown-linux-gnu
```

当前 macOS 测试和 backend-neutral portability check 均通过。cross-check 只证明 Rust 依赖和代码可编译，不代替真实桌面平台行为验收。

## 阶段复核

- 已覆盖 stable Entity、hierarchy、component storage、legacy NodeHandle mapping、mutation queue、reparent、subtree despawn 和 stale handle 行为。
- 已修复同批次 `despawn -> create same ID` 的复用缺口，失败保持批次原子。
- 已删除每次 commit 克隆全树的冗余验证状态，改为 touched-node overlay。
- 未把 `World` 暴露给业务，也未引入第二套窗口、事件循环、布局或 GPU renderer。
- tombstone 会随历史销毁 ID 增长；这是 stale handle 永不复活的明确成本，Phase 3 的内存/长时 churn 基线必须继续监控，不能静默改成可复用 ID。

据此，Phase 2 的身份、层级、映射和 mutation boundary 已闭环，可以进入 Phase 3。
