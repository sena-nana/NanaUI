# Issue #7 Phase 4：Entity / View / Context 与扩展契约

本阶段在 `2026-08-14` 为 Phase 3 runtime 增加 backend-neutral 应用 API。目标是让 Rust 组件、Vue adapter 和后续扩展共享同一个 typed state / action / async contract，不复制 Iced application model，也不在 Runtime 内创建第二个 executor。

## Typed Entity 与 Context

- `Entity<V>` 由 `StableNodeId` + phantom View type 构成；不暴露 Bevy `Entity`，复制 handle 不复制 state。
- `AppContext` 同时持有 `UiWorld` 与 typed View state。`create_view` 先成功创建 retained Entity，再发布 state；伪造错误类型或 stale handle 返回 `FrameworkError`，不会 panic 或移除原 state。
- `update(entity, |view, cx| ...)` 在 closure 内汇集 mutation 与事件，事件处理完后只 commit 一个 `MutationQueue`。
- `remove_view` 先取得完整 subtree，成功 despawn 后统一释放 root/descendant typed state 与两端事件 handler，不遗留 observer。
- `ViewContext` 提供当前 typed entity、frame mutation queue 和 `emit`；不暴露 ECS World。

View state mutation 本身遵循普通 Rust closure 语义；retained-tree mutation 仍由 `UiWorld::commit` 保证批次原子。业务不应把 `UiWorld` commit error 当作 View state rollback 机制。

## Closure event

`on` 处理 View 自身事件；`observe(source, observer, closure)` 支持子 View → 父 View等 typed 跨实体观察，不需要全局字符串消息总线。nested event 记录真实 emitter，并以 FIFO 顺序交付。

单次 update 最多交付 16384 个事件；递归 emission 超限返回 `EventOverflow`，避免一个 UI turn 永不返回。移除 source 或 observer subtree 都会移除相应 handler。

## Action / KeyContext

`ActionId`、`KeyContext`、`ContextPredicate` 已从 Iced-facing `nana-ui::command` 下沉到 `nana-ui-core::action`：

- `nana-ui` 继续 re-export 原名称，现有 `ActionRegistry`、`Keymap` 和调用方不需要迁移；
- `nana-ui-runtime` 使用相同 action identity/context predicate 注册 closure handler；
- 空 ID、重复注册、context 不匹配与缺失 action 都是显式错误；
- dispatch 临时取出 handler，允许 handler 安全访问其余 `AppContext` 状态，并在返回后恢复注册。

Action 的 label/category/search/key binding 仍由现有 NanaUI registry 拥有；Runtime 不复制面向 UI 的 command metadata。

## Task / Subscription 与宿主

- `Task<T>` 拥有一个标准 `Future`，支持 `ready`、`map` 和所有权转交；
- `Subscription<T>` 拥有带稳定 ID 的标准 wake-driven `Stream`；没有逐帧 polling callback；
- drop future/stream 即取消未完成工作，Runtime 不启动线程或 executor；
- `nana-ui::run_task` / `run_subscription` 将合同接入现有 Iced executor/subscription tracker。相同 subscription ID 保持宿主生命周期身份。

## Extension 安装

`UiExtension` 只能写 staging `ExtensionRegistrar`。Registrar 先校验扩展内重复项，`AppContext::install` 再检查与已安装 action 的冲突，全部通过后一次性发布。安装失败不会留下半注册 action，也不能在 install 阶段任意修改运行中的 World。扩展名称为空或重复均拒绝。

当前合同只支持进程生命周期内安装，不提供热卸载；这是有意约束，避免异步 task、View 与 handler 尚未定义取消顺序时暴露不安全的假卸载功能。

## 功能与性能门禁

Runtime 16 项测试覆盖 typed update/read、self/cross-view/nested event、错误类型 handle、事件上限、action context、扩展原子安装、subtree state 清理、Task/Future 和 Subscription/Stream 所有权。既有 NanaUI command tests 通过，证明 Action/KeyContext 下沉保持兼容。

机器可读报告见 [`performance/2026-08-14-issue7-phase4-framework.json`](performance/2026-08-14-issue7-phase4-framework.json)。Apple M4 release，100 次 warmup、1000 个样本：

- typed View update + closure event + text mutation + atomic commit：P50 < 0.0005 ms，P95/P99 0.001 ms；
- context action dispatch：P50/P95/P99 均 < 0.0005 ms。

该 microbenchmark 证明框架层没有可见固定开销，不代表完整渲染帧时间。

验证结果：runtime tests/clippy 通过；NanaUI/Iced adapter check 通过（仅 `vendor/arboard` 既有 warnings）；Android、Windows、Linux runtime cross-check 通过；Iced compatibility boundary script 通过。

## 阶段结论

Phase 4 已建立 typed Entity/View、closure Context、跨 View event、统一 Action/KeyContext、host-owned Task/Subscription 与原子 extension registration。多轮复核移除了 busy-poll subscription、半安装 extension、无界递归事件、subtree state 泄漏和仅能 self-event 的局部模型。可以进入 Phase 5，将 Vue/component consumer 迁移到同一个 Runtime，而不是双写 retained tree。
