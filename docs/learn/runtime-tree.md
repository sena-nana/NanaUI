---
title: 运行时保留树
---

# 运行时保留树：`UiWorld`

本章深入 `nana-ui-runtime` 的心脏：**`UiWorld` 这棵保留树（Retained Tree）如何存储节点、如何原子地接收批式变异、又如何将脏标记转化为一帧内的确定性系统工作**。读完后你能搞懂：一个 `StableNodeId` 背后的稠密表与稀疏侧表长什么样；`UiMutation` 与 `MutationQueue` 如何在提交边界整批校验；`ComponentRegistry` 如何统一内置与扩展控件；以及宿主驱动 `RuntimeDocument::flush` 时各阶段（`style → text → layout → hit-test → a11y → extract`）的严格顺序。本章承接 [模块与分层](/learn/modules-and-layers)，是 [布局引擎](/learn/runtime-layout) 与 [输入链路](/learn/runtime-input) 的共同基础。

## 职责与边界

- **唯一保留树权威**：`UiWorld`（`crates/nana-ui-runtime/src/world.rs:628`）是所有保留节点的身份、拓扑与逻辑状态权威。内部节点表（`NodeStore`）刻意不暴露进公共 API，因此优化底层存储结构不会破坏诊断记录、快照或持久化数据。
- **原子批式变异**：外部代码不能直接篡改节点表，只能将 `UiMutation` 排入 `MutationQueue`，由 `UiWorld::commit` 整批校验通过后一次性应用（`crates/nana-ui-runtime/src/world/mutation.rs:2915`）。
- **明确区分权威与投影**：布局输出（`LayoutResult`）是投影，帧内的 `MutationQueue` 是待提交的暂存缓冲，二者都不构成第二套树权威。
- **运行时零 CSS 解析**：`nana-ui-runtime` 的依赖表中没有 `nana-ui-css`，样式统一以 Style Model 数据（`LayoutStyle`）形式进入（见下文 [依赖与邻居](#依赖与邻居)）。

## 依赖与邻居

`nana-ui-runtime` 位于第 5 层（L5），向下必选依赖 `nana-ui-core`、`nana-text`、`nana-ui-input`、`nana-ui-view-schema` 与 `nana-diagnostics`，并按 feature 可选依赖 `nana-ui-charts`、`nana-ui-diagram` 与 `nana-ui-view-macros`；**不依赖** `nana-ui-css` 或任何 GPU crate：

| 邻居 Crate | 协作关系 |
| --- | --- |
| `nana-ui-core` | 提供节点样式、主题与几何合同（`NodeStyle` 内含的 `LayoutStyle`、`CompiledTheme` 等） |
| `nana-text` | 提供文本节点测量与保留文本布局权威，`TextLayoutStore` 挂载在 `NodeStore` 中 |
| `nana-ui-scene` | `RuntimeDocument` 持有 `AppContext` 与 `UiScene`，驱动单帧有界 `flush` 事务（`crates/nana-ui-scene/src/runtime_document.rs:18`） |

## 关键类型与数据

| 类型 | 所在文件与行号 | 作用说明 |
| --- | --- | --- |
| `UiWorld` | `crates/nana-ui-runtime/src/world.rs:628` | 保留树唯一权威：节点表 + 脏标记集合 + 世代号 + 各类索引与缓存 |
| `StableNodeId` | `crates/nana-ui-runtime/src/world.rs:70` | 对外稳定节点身份（`u64` 包装，`0` 保留为空，已销毁 ID 永不复用） |
| `NodeStore` | `crates/nana-ui-runtime/src/store.rs:379` | 稠密节点表 + 稀疏侧表（替代早期的 `bevy_ecs::World`） |
| `NodeRecord` | `crates/nana-ui-runtime/src/store.rs:81` | 单节点稠密记录：`hierarchy`、`style`、`resolved_layout`、`layout`、`interaction`、`dirty` 等 |
| `Hierarchy` | `crates/nana-ui-runtime/src/store.rs:46` | `parent` / `children: Arc<Vec<StableNodeId>>` / `index_in_parent` / `children_base` |
| `NodeKind` | `crates/nana-ui-runtime/src/world.rs` | `Document` / `Element` / `Text` / `Comment` 四种节点类别 |
| `UiMutation` | `crates/nana-ui-runtime/src/mutation.rs:15` | 保留树原子变异枚举（涵盖创建、插入、卸载、样式、文本、焦点、动画等） |
| `MutationQueue` | `crates/nana-ui-runtime/src/mutation.rs:306` | 帧局部命令队列，供声明式视图、组件装配器与宿主适配器写入 |
| `CommitReport` | `crates/nana-ui-runtime/src/world.rs:388` | 单次 `commit` 的统计回执（created / inserted / detached / reparented / despawned） |
| `ComponentRegistry` | `crates/nana-ui-runtime/src/component_registry.rs:249` | 统一组件类型注册表：`by_id` / `by_tag` / `by_rust` 三张索引 |
| `DirtyMask` | `crates/nana-ui-runtime/src/schedule.rs:9` | 每节点 9 位失效掩码（`STYLE`、`TEXT`、`LAYOUT`、`INPUT`、`FOCUS_IME`、`RENDER`、`ACCESSIBILITY`、`STATE`、`TRANSFORM`） |
| `SystemWork` | `crates/nana-ui-runtime/src/schedule.rs:64` | 单次 `take_system_work` 产出的确定性分阶段工作列表 |
| `AppContext` | `crates/nana-ui-runtime/src/framework.rs:1072` | 持有类型化组件状态与事件监听器，底层权威仍为 `UiWorld` |
| `RuntimeDocument` / `RuntimeFrameUpdate` | `crates/nana-ui-scene/src/runtime_document.rs:18` / `:26` | 文档 + 增量 `UiScene` 事务封装，以及单帧更新结果信封 |

## 内部结构

### 1. 节点身份与存储：稠密数组 + 稀疏侧表（`NodeStore`）

`StableNodeId`（`crates/nana-ui-runtime/src/world.rs:70`）是对外唯一的节点标识（`0` 映射为 `None`，配合专用的轻量整数扩散哈希 `IdHasher`，`:95`，避免在高频动画查询中为递增整数 ID 支付 SipHash 成本）。

`store.rs` 采用**「高频字段进稠密表、特定功能字段进稀疏侧表」**的双层布局（`crates/nana-ui-runtime/src/store.rs:300-445`）：

- **稠密表与分页压缩 `Slots<T>`（`store/paged.rs`）**：核心存储封装于 `Slots<T>`（`crates/nana-ui-runtime/src/store/paged.rs:267`）。批量连续建树时底层以紧凑线性槽位运行（配合 `overflow`）；当局部反复卸载使存活密度跌破 ID 跨度的 1/8 时自动切换为 64 槽位一页的 `Paged` 分页表，空页随即归还分配器，从而在宽销毁场景保持 `943 B/box`（10k 节点 `Stack` 为 `634 B/box`）。
- **`NodeRecord`（`:81`，`<= 152 B`）与写时解析策略**：存放每个节点最常访问的字段，包括 `hierarchy`（`24 B`，自带 `index_in_parent` 以避免找兄弟时线性扫表；从父节点的 `children_base` 数起，整段兄弟平移时只改基数）、`mount: MountState`、作者原始 `style: NodeStyle` 以及预解析好的 `resolved_layout: Arc<LayoutStyle>`。由于 `LayoutStyle` 结构体约 4.8 KB，框架在写入样式或切换主题时预先解析一次存入 `Arc`，后续布局与抽取阶段只需廉价克隆 `Arc` 指针（`:87`）。
- **稀疏侧表（`NodeStore` 其余字段，`:379-445`）**：按需存放 `visuals`、`custom_render`、`event_listeners`、`component_types`、`overlay_hosts`、`editors` 以及单独成表的 `text_nodes` 与 `text_layouts`（使大范围重排能快速遍历小表跳过无文本工作的节点）。
- **堆迭代建树弹跳器 `ViewBuilder::run_trampoline`（`view/node.rs:491`）**：声明式视图装配深层嵌套树时，通过显式堆任务队列迭代构建与收尾（`ViewTask::Build` / `ViewTask::LeaveEl`），不随视图嵌套深度累积原生调用栈帧。

### 2. 批式变异提交：`UiMutation` / `MutationQueue` / `commit`

所有对树的修改都表示为 `UiMutation` 枚举变体（`crates/nana-ui-runtime/src/mutation.rs:15-330`）：

- **拓扑操作**：`Create`、`Insert`（兼做移动位置，`before = None` 表示追加到末尾）、`ReorderChildren`（把一个父节点现有的子节点一次排成给定顺序，成本随表长走而不是随移动次数乘表长）、`Detach`（脱离父节点但保持 `Mounted`）、`ParkSubtree`（整棵子树停用隐藏但保留节点身份与内部状态）、`DespawnSubtree`（彻底销毁，ID 永久退役）。
- **样式、主题与文本**：`SetStyle`、`SetPresetTheme`、`SetThemeTokens`（整体携带 `Arc<CompiledTheme>`）、`SetText`、`SetRichText`、`SetLanguage`、`SetLocale`、`SetLocalizedText`。
- **布局写回与滚动**：`WriteLayout`（仅供布局引擎写回结果与测试使用）、`PatchPlacement`、`SetScrollOffset`。

`MutationQueue`（`:306`）收集当前帧的变异序列，调用 `UiWorld::commit`（`crates/nana-ui-runtime/src/world/mutation.rs:2915`）时先对整批队列做完整性校验（拦截重复 ID、已退役 ID、跨文档引用、父子成环 `Cycle` 以及超过最大深度 `TreeTooDeep` 等 `UiWorldError`），**全部通过后才原子写入 `NodeStore`**，绝不留下半成品树。

### 3. 三套代际号与精确失效隔离

`UiWorld` 维护三个相互独立的单调代际号，确保不同类型的变更只唤醒真正关心它的下游消费方：

1. **`generation`**：任何权威状态变更均递增，宿主据此判断世界是否发生变化。
2. **`layout_generation`（`world.rs:731`）**：仅当布局几何输出实际变化时才递增。纯绘制或合成器动画帧不递增它，因此依赖布局几何的消费方（Scene 几何、命中测试、无障碍边界）不会被纯颜色或透明度变化误唤醒。
3. **`layout_source_epoch`（`world.rs:735`）**：仅当**布局输入**（样式、文本内容、树结构）变化时递增；布局引擎通过 `WriteLayout` 将计算出的结果写回树时**不改动它**（`world/mutation.rs:2704`），防止「写回布局结果」被误判为「布局输入又变了」而引发死循环重排。

### 4. `ComponentRegistry`：内置与扩展控件的单一注册表

`ComponentRegistry`（`crates/nana-ui-runtime/src/component_registry.rs:119`）维护 `by_id`、`by_rust`、`by_tag` 三张索引。内置控件的静态身份与标签元数据由 `component_descriptors::BUILTIN_COMPONENT_IDS` 提供，应用通过 `register_component` 注册的扩展控件实现 `RegisterableComponent` trait（`:113`），提供 `TYPE_ID` 与 `TAGS`；控件行为通过 `ComponentView::BEHAVIOR` 在首次创建时按需安装。同时 `UiWorld` 维护 `nodes_by_component: HashMap<ComponentTypeId, HashSet<StableNodeId>>`（`world.rs:894`），让按组件类型的全局查询保持 $O(1)$。

### 5. `DirtyMask`、`SystemWork` 与 `RuntimeDocument::flush` 收敛顺序

每次 `commit` 会在受影响节点上标记 9 位 `DirtyMask`（`crates/nana-ui-runtime/src/schedule.rs:9`：`STYLE`、`TEXT`、`LAYOUT`、`INPUT`、`FOCUS_IME`、`RENDER`、`ACCESSIBILITY`、`STATE`、`TRANSFORM`）。

当宿主调用 `RuntimeDocument::flush`（`crates/nana-ui-scene/src/runtime_document.rs:109`）时，帧事务按以下严格顺序推进：

1. **排空响应式与异步任务**：`AppContext::take_system_work`（`framework/frame.rs:13`）先执行 `poll_tasks()` 与 `flush_reactive()`，再从 `UiWorld::take_system_work`（`world.rs:2286`）按排序后的脏实体分拣出 `SystemWork`。
2. **样式 → 文本整形 → 增量布局循环（上限 `MAX_FRAME_PASSES = 8`）**：依次执行 `resolve_styles` → `shape_text` → 合并文本产生的新布局种子 → `layout_document_for_viewport` / `layout_document_with_frontier`，并在需要时执行至多 `POST_LAYOUT_SHAPE_PASSES = 4` 轮「布局定宽后重整形 → 再布局」反馈直至收敛（超过 8 轮未收敛则返回 `FrameworkError::FrameDidNotSettle` 并还原脏工作）。
3. **一次性收尾（Settle 之后）**：在布局完全收敛后，一次性执行命中测试索引更新（`apply_hit_test_work`）→ 无障碍增量投影（`project_accessibility_delta`）→ 渲染抽取（`extract_nodes` + `scene.apply_delta`）→ 合成器呈现绑定（`apply_presentation`）。

## 协作与数据流

```text
应用 / 视图函数 / 组件     ──写入──▶ MutationQueue（帧内暂存）
                                         │ UiWorld::commit（整批校验 + 原子应用，world/mutation.rs:2915）
                                         ▼
                                 UiWorld（NodeStore + DirtyMask + LayoutFrontierSeed）
                                         │ RuntimeDocument::flush → AppContext::take_system_work
                                         ▼
                         SystemWork { style, text, layout_seeds, input, a11y, render }
                                         │ resolve_styles → shape_text → layout_document_*
                                         ▼
                         extract_nodes → UiScene::apply_delta（详见 Scene 提取章）
```

## 代码地图

| 文件路径 | 核心内容 |
| --- | --- |
| `crates/nana-ui-runtime/src/world.rs` | `UiWorld` 主体、`StableNodeId` / `IdHasher`、`CommitReport`、`take_system_work` |
| `crates/nana-ui-runtime/src/store.rs` | `NodeStore`、`NodeRecord`、`Hierarchy` 与全部稀疏侧表 |
| `crates/nana-ui-runtime/src/store/paged.rs` | `Slots<T>` 与 64 槽位稀疏压缩分页表（空页即时释放） |
| `crates/nana-ui-runtime/src/view/node.rs` | `ViewBuilder::run_trampoline` 堆迭代视图装配弹跳器 |
| `crates/nana-ui-runtime/src/mutation.rs` | `UiMutation` 枚举词表与 `MutationQueue` |
| `crates/nana-ui-runtime/src/world/mutation.rs` | `UiWorld::commit` / `commit_with_mount_lifecycle`、滚动偏移与布局结果发布 |
| `crates/nana-ui-runtime/src/component_registry.rs` | `ComponentRegistry`、`RegisterableComponent`、`ComponentTypeId` |
| `crates/nana-ui-runtime/src/schedule.rs` | `DirtyMask` 9 位失效掩码与 `SystemWork` |
| `crates/nana-ui-runtime/src/framework.rs` | `AppContext`（类型化组件视图状态与事件表） |
| `crates/nana-ui-runtime/src/framework/frame.rs` | 分阶段帧驱动入口：`resolve_styles`、`shape_text`、`layout_document_*` |
| `crates/nana-ui-runtime/src/world/extraction.rs` | `extract_nodes`（增量脏节点抽取）与 `extract_document`（全量快照） |
| `crates/nana-ui-scene/src/runtime_document.rs` | `RuntimeDocument::flush`：单帧有界收敛事务 |

## 延伸阅读

- [布局引擎](/learn/runtime-layout)：`commit` 产生的 `LayoutFrontierSeed` 如何在增量布局中被消费
- [输入链路](/learn/runtime-input)：`SystemWork.input_hit_test` 更新后如何路由指针与键盘事件
- [Scene 提取](/learn/scene-extraction)：`extract_nodes` 产出的 `ExtractedNode` 如何合成进 `UiScene`
- [合同与不变量](/learn/contracts-and-invariants)：「只有一棵权威树」等核心架构红线
