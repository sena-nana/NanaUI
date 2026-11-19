---
title: Scene 提取
---

# Scene 提取：从 `UiWorld` 到保留绘制图

本章深入 `nana-ui-scene`：**保留树的一批脏节点如何转化为一份可增量更新、对图形后端完全中立的保留绘制图 `UiScene`**。读完后你能搞懂：`ExtractedNode` 里装了什么、`ScenePrimitive` 的四要素（位置、变换、共享裁剪链、全序键）分别由谁维护；`apply_delta` 的三档增量失效规则为什么这样设计；`FramePlan` 帧图如何在不依赖任何 GPU 类型的前提下编译出一帧的渲染程序；以及为什么命中测试与无障碍留在 `UiWorld` 而不放进 Scene。本章承接 [保留树：UiWorld](/learn/runtime-tree) 的 `flush` 事务，是 [GPU 与帧生命周期](/learn/gpu-frame) 的直接上游。

## 职责与边界

- **后端中立的绘制图权威**：`nana-ui-scene` 的 `lib.rs`（`crates/nana-ui-scene/src/lib.rs:1`）定位为 "Backend-neutral render scene and frame graph"，不持有任何应用业务状态、窗口句柄、GPU 设备或渲染管线对象；实际的 GPU 绘制由 `nana-ui` 中的 `SceneWgpuPainter` 完成。
- **为什么 `nana-ui-scene` 不依赖 WGPU**：这是核心架构合同。绘制图的全部输出（`ScenePrimitive`、`FramePlan`、`RenderGraph`）都是纯数据结构——没有纹理句柄、没有渲染通道对象、没有硬件像素格式枚举；资源内容与绑定在消费期才由 painter 解析（"a texture update does not compile a graph"，`composition.rs:8`）。这使无 GPU 单元测试、场景快照比对与跨后端演进完全不被图形 API 绑架。
- **保留式增量结构**：`UiScene` 按 `PrimitiveId` 在 `BTreeMap` 中保留全部图元，每帧只对脏子树重建图元，纯滚动变化直接走线性平移快路径（`crates/nana-ui-scene/src/scene.rs:1672`）。
- **明确不做的事**：不解析 CSS；不触碰 WGPU；**不做指针命中测试与无障碍树投影**（这两者需要布局与组件语义权威，已在 `RuntimeDocument::flush` 的前半段由 `UiWorld` 完成）。

## 依赖与邻居

在依赖层级上：`core(L1) → text/css/charts/input/window/gpu(L2) → runtime(L5) → scene(L6) → nana-ui(L7)`。`nana-ui-scene` 依赖 `nana-ui-runtime`、`nana-ui-core`、`nana-ui-charts` 与 `nana-diagnostics`，**不依赖** `nana-gpu` 与 `nana-ui-css`：

| 邻居 Crate | 协作关系 |
| --- | --- |
| `nana-ui-runtime` | `UiWorld::extract_nodes` 产出 `ExtractedNode`（`crates/nana-ui-runtime/src/world/extraction.rs:652`） |
| `nana-ui` | `SceneWgpuPainter` 消费 `UiScene` 的图元序列、`frame_plan()` 与 `instance` 代际号（见 [GPU 与帧生命周期](/learn/gpu-frame)） |
| `nana-ui-core` | 提供底层几何原语与 Motion 描述符打包数据 |
| `nana-text` | 保留文本布局句柄已装入 `ExtractedNode.text_layout`（`crates/nana-ui-runtime/src/components.rs:4555`） |
| `nana-diagnostics` | `flush` 以 `FrameStage::Extract` 记录抽取与 `apply_delta` 耗时；8 轮未收敛记入 `FLUSH_DID_NOT_SETTLE`（`runtime_document.rs:203`） |

## 关键类型与数据

| 类型 | 所在文件与行号 | 作用说明 |
| --- | --- | --- |
| `ExtractedNode` | `crates/nana-ui-runtime/src/components.rs:4514` | 单节点抽取快照（共享 `Arc<NodeKind>` 与 `Arc<ComputedStyle>`、布局盒、文本布局句柄、自定义录制等） |
| `UiScene` | `crates/nana-ui-scene/src/scene.rs:1357` | 保留绘制图：节点表 + 图元表 + 全序表 + 可见性与帧计划缓存，单调递增 `instance` |
| `ScenePrimitive` | `crates/nana-ui-scene/src/scene.rs:1257` | 单条绘制图元：`id`、`bounds`、`transform`、共享裁剪链 `clips`、`paint_opacity`、`z_index`、`document_order`、`kind` |
| `ScenePrimitiveKind` | `crates/nana-ui-scene/src/scene.rs` | 图元具体类别枚举：`Quad` / `Mesh` / `Icon` / `Text` / `Chart` / `HostTexture` / `Custom` 等 |
| `PrimitiveId` | `crates/nana-ui-scene/src/scene.rs:145` | `(StableNodeId, slot: u64)`；单个节点可展开为多个槽位图元（背景、文字、选区、滚动条各占一槽） |
| `AffineTransform` | `crates/nana-ui-scene/src/scene.rs:62` | `2×3` 仿射矩阵 + `[f32; 2]` 射影行，完整覆盖 2D 与 3D 透视变换 |
| `ClipRegion` | `crates/nana-ui-scene/src/scene.rs:114` | 裁剪盒 + 自身变换 + 圆角半径 / 多边形顶点 |
| `SceneOrderKey` | `crates/nana-ui-scene/src/scene.rs:1299` | 图元全序键：层叠上下文栈前缀 `stack` + `paint_layer` + `slot` + `node` |
| `SceneDelta` / `SceneDeltaStats` | `crates/nana-ui-scene/src/scene.rs:1315` / `:1334` | 单次 `apply_delta` 的增量变更集与重建统计计数 |
| `DrawAttributes` / `SceneDraw` | `crates/nana-ui-scene/src/scene/attributes.rs:5` / `:13` | 面向绘制器的继承属性缓存与单图元绘制问答快照 |
| `RuntimeDocument` / `RuntimeFrameUpdate` | `crates/nana-ui-scene/src/runtime_document.rs:18` / `:26` | 规范宿主的单文档事务所有者与单帧更新回执 |
| `FramePlan` | `crates/nana-ui-scene/src/scene/composition.rs:8` | 结构化帧程序：`RenderOperation` 序列 + 离屏 `preparations` + `custom_nodes` |
| `RenderGraph` / `CompiledRenderGraph` | `crates/nana-ui-scene/src/graph.rs` | 渲染通道有向无环图（DAG），`compile()` 完成读写冒险边补齐与 Kahn 拓扑排序 |
| `VisibilityIndex` | `crates/nana-ui-scene/src/scene/visibility.rs:6` | 有序包围盒层级索引（射影变换节点采用保守包含策略） |

## 内部结构

### 1. 抽取时机：`RuntimeDocument::flush` 的有界收敛事务

抽取由 `RuntimeDocument::flush`（`crates/nana-ui-scene/src/runtime_document.rs:109`）驱动。内部的 `flush_loop`（`:174`）最多执行 `MAX_FRAME_PASSES = 8` 轮系统工作排空，并在每轮布局后允许最多 `POST_LAYOUT_SHAPE_PASSES = 4` 轮（`:48`）「换行改变盒宽 → 重整形 → 再布局」反馈。

当循环收敛（settle）后，`flush` 在**同一个有界事务内**共享同一个 `layout_generation`（`:28`），依次执行四步收尾（`:264-330`）：

1. 更新增量命中索引（`apply_hit_test_work`）；
2. 生成无障碍增量事务（`project_accessibility_delta`）；
3. **抽取脏节点并更新绘制图**：调用 `world().extract_nodes(&render_dirty)`（隐藏节点仍保留在返回列表中且 `visible = false`，以便 `UiScene` 移除其旧图元，`world/extraction.rs:652`），随后调用 `Arc::make_mut(&mut self.scene).apply_delta(extracted, render_removed)`（无人共享快照时原地更新，耗时一并计入 `FrameStage::Extract`，`:291`）；
4. 绑定合成器呈现层（`apply_presentation`）。

`ExtractedNode`（`crates/nana-ui-runtime/src/components.rs:4514`）广泛采用 `Arc` 共享与按需封装：`kind: Arc<NodeKind>` 与 `style: Arc<ComputedStyle>` 直接克隆引用；各控件通过 `WidgetVisual::paint` 按需投影出的节点本地绘制输出放入 `widget_paint: Option<Arc<WidgetPaintOutput>>`（`:4601`）；`chrome_radii: ChromeRadii` 携带已按当前主题解析的圆角（`:4583`）；`compositor: ExtractedCompositor` 仅携带层提升提示，瞬态呈现值不烤入逻辑样式（`:4595`）。

### 2. 图元展开与槽位命名空间（`PrimitiveId`）

`UiScene` 将每个 `ExtractedNode` 展开为一至多条 `ScenePrimitive`（`crates/nana-ui-scene/src/scene.rs:1257`），通过 `PrimitiveId { node, slot }`（`:145`）唯一定位：

- **节点基础槽位（`0..=255`）**：如背景表面（slot 0）、边框、主文本等；
- **集合命名空间槽位（`namespace << 32 | index`，`collection_slot:152`）**：文本行标签（1）、诊断标记（2）、文档文本选区（6）、内联对象（7）、编辑器光标（8）、下拉菜单项（9–13）等；
- **特殊全局槽位**：`MODAL_SCRIM_SLOT = 10`（`:208`，模态遮罩）、`TRIGGERED_OVERLAY_SURFACE = 0xFFFF_FFE0`（`:216`，弹出菜单与气泡表面提升至根层叠上下文，脱离触发器的局部裁剪链）、`SCROLLBAR_CHROME = 0xFFFF_FFF8`（`:226`，滚动条始终排在容器全部后代之后绘制）。

应用通过 `Painter` API 录制的自定义矢量路径（`PaintRecording`，Issue #217）在 `crates/nana-ui-scene/src/scene/custom_paint.rs` 中经 `i_overlay` 布尔/填充求解与 `lyon` 曲面细分（容差 `TOLERANCE = 0.05` 逻辑像素，`MITER_LIMIT = 4.0`，`:45-48`）生成保留的 `PathMesh`——节点平移或滚动时直接重发缓存网格，无需重新三角化。

### 3. 祖先变换合成、共享裁剪链与全序键（`SceneOrderKey`）

- **祖先变换与滚动顺序（`compute_ancestor_state`，`scene.rs:2176`）**：`AffineTransform`（`:62`）包含 `2×3` 仿射分量与 `[f32; 2]` 射影行。`compute_ancestor_state` 自根向叶遍历祖先链（遇到 `PositionSpec::Fixed` 祖先时重置几何与裁剪链，但保留外层结构性不透明度），依次收集祖先的 `overflow` 裁剪区、模态框裁剪区与 `clip-path`；**最后才乘上容器自身的 `scroll_offset`**——因此滚动内容会被容器自身的 overflow 框正确裁剪，而容器裁剪框本身不会跟着内容滚走。
- **共享裁剪链（`Arc<[ClipRegion]>`）**：每条 `ScenePrimitive` 的 `clips: Arc<[ClipRegion]>` 直接共享同一裁剪作用域下的 `Arc` 切片（`:1257`），避免为成百上千个同容器子图元重复克隆裁剪数组。
- **全序键（`SceneOrderKey`，`scene.rs:1299`）**：由外到内每级层叠上下文（opacity 组、`isolation: isolate`、带 `z_index` 的定位节点）的 `(z_index, document_order)` 栈前缀 `stack` + `paint_layer` + `slot` + `node` 组成。它保证任何隔离组或带 `z-index` 的子树在绘制序列中严格连续，绝不被外部兄弟节点穿插打断。

### 4. `apply_delta` 的三档增量失效规则

`UiScene::apply_delta(extracted, removals) -> SceneDelta`（`crates/nana-ui-scene/src/scene.rs:1672`）按变更性质分三档处理：

1. **纯滚动平移快路径**：当节点仅发生 `scroll_offset` 变化且变换非射影时，直接对子树已有图元做线性平移（`scroll_translations`），不重建图元、不重排序。
2. **单节点原位更新**：普通绘制属性变化更新 `delta.paint`；盒位置变化更新 `transforms`；裁剪变化更新 `clips`。
3. **重排序与子树重建**：当 `paint_order_facts`（`z_index` 或层叠组开启标志）变化时触发 `stacking_changed` 重排序；当非隔离祖先的继承不透明度变化时触发 `subtree_rebuild`。任何结构或帧计划变化都会清空 `frame_plan` 与 `visibility` 缓存，并推进单调递增的 `UiScene.instance`。在 debug 构建下，`audit_retained_projection()`（`:1950`）还会对小规模树从零全量重算并比对增量结果，自动抓出任何增量失效漏算。

### 5. `FramePlan` 与 `RenderGraph` 编译

`UiScene::frame_plan()`（`crates/nana-ui-scene/src/scene/composition.rs:15`）将保留图元序列编译为结构化的 `FramePlan { operations, preparations, custom_nodes }`（缓存在 `OnceLock` 中）：

- 按渲染器名称建立离屏准备通道（每个渲染器一个 `prepare:{renderer}` pass，而非每个资源一个 pass，避免 `RenderGraph::compile` 的二次方复杂度）；
- 按 `SceneOrderKey` 全序扫描图元：遇到连续同渲染器的 `Custom` 节点即合并为一个 `custom:{renderer}` 操作，遇到标准 UI 图元则切回 `ui-standard` 操作——**严格保持文档遮挡顺序不变**；
- 若两个自定义节点以不同的 `(renderer, revision)` 声明同一个宿主纹理槽，`conflicting_external_resource()`（`:58`）会在绘制前直接以 `GraphError::ConflictingExternalResource` 拒绝整帧。

## 协作与数据流

```text
RuntimeDocument::flush（runtime_document.rs:109）
  ├─ 1. 样式 → 文本整形 → 增量布局收敛（共享本次 layout_generation）
  ├─ 2. UiWorld 更新增量命中索引 + 生成无障碍增量事务
  ├─ 3. UiWorld::extract_nodes(&render_dirty) ──► Vec<ExtractedNode>
  ├─ 4. UiScene::apply_delta(extracted, removed) ──► SceneDelta（滚动平移 / 原位更新 / 重排）
  └─ 5. UiScene::apply_presentation（绑定 CompositorLayer 与 MotionDescriptor）
        │
        ▼
SceneWgpuPainter 读取 shared_scene() 的 Arc<UiScene>、frame_plan() 与 SceneOrderKey 全序录制绘制
```

## 代码地图

| 文件路径 | 核心内容 |
| --- | --- |
| `crates/nana-ui-scene/src/lib.rs` | Crate 边界声明与公开导出清单 |
| `crates/nana-ui-scene/src/runtime_document.rs` | `RuntimeDocument`、`RuntimeFrameUpdate` 与 8-pass 有界 `flush` 事务 |
| `crates/nana-ui-scene/src/scene.rs` | `UiScene`、`ScenePrimitive`、`SceneOrderKey`、`apply_delta`、祖先变换与裁剪合成 |
| `crates/nana-ui-scene/src/scene/order.rs` | `sort_primitives` 与保留文档顺序重建 |
| `crates/nana-ui-scene/src/scene/visibility.rs` | `VisibilityIndex` 有序包围盒层级与保守包含裁剪 |
| `crates/nana-ui-scene/src/scene/composition.rs` | `FramePlan` 构建与外部资源冲突检测 |
| `crates/nana-ui-scene/src/scene/compositor.rs` | 合成器层提升/降级滞回（`16 ms / 120 ms`）与 `CompositorPaintEncode` |
| `crates/nana-ui-scene/src/scene/custom_paint.rs` | `PaintRecording` 矢量布尔运算与 `lyon` 网格细分（`PathMesh`） |
| `crates/nana-ui-scene/src/scene/attributes.rs` | `DrawAttributes` 与 `SceneDraw` 增量绘制问答层 |
| `crates/nana-ui-scene/src/scene/primitives/` | 各控件族的 `ScenePrimitiveKind` 图元展开实现 |
| `crates/nana-ui-scene/src/graph.rs` | `RenderGraph` 有向无环图编译、读写冒险检测与拓扑排序 |

## 延伸阅读

- [GPU 与帧生命周期](/learn/gpu-frame)：`UiScene` 与 `FramePlan` 如何被 `SceneWgpuPainter` 录制进宿主的帧
- [保留树：UiWorld](/learn/runtime-tree)：`flush` 事务的上游与 `SystemWork` 的生成
- [Runtime 与 Scene 渲染抽取合同（参考）](/reference/runtime-scene)
