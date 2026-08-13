# Issue #7 Phase 6：UiScene / RenderGraph

本阶段在 `2026-08-14` 建立 backend-neutral `nana-ui-scene`，将 Phase 3 的 Runtime render extraction 真正接入 retained scene 和 frame graph。Scene 不拥有窗口、Device/Queue 或应用状态；Iced/WGPU 仍是迁移期兼容 backend，而不是新的 public contract。

## 边界与数据流

```text
UiWorld mutation
  -> SystemWork.render_extraction / render_removals
  -> ExtractedNode delta
  -> UiScene retained primitives
  -> compiled RenderGraph
  -> Iced/WGPU compatibility adapter or future backend
```

`NanaTreeDocument::flush_runtime_systems` 不再丢弃 render work，而是把 dirty extraction 和 tombstone removal 原子应用到窗口自己的 `UiScene`。静止状态没有 work，也不会调用 Scene 更新。

Runtime 新增 `CustomRenderNode { renderer, resource, revision }` component。它只包含 backend-neutral extension/resource key，不暴露 WGPU/Metal/D3D12/Vulkan 对象；空 key 在 mutation batch 验证阶段被原子拒绝。Vue 的 `data-nana-gpu` / `setGpuSlot` 现在写入这一 component，删除属性会清除它。

## Scene primitive ABI

当前最小 ABI 包含：

- `Quad`：background、border、corner radius；
- `Text`：内容、颜色、size、weight、family、letter spacing；
- `Custom`：renderer extension key、resource key、revision；
- 每个 primitive 共有 stable `PrimitiveId`、layout bounds、累计 affine transform、祖先 clip chain、累计 opacity、z-index 与 document order。

Clip 保留为带 transform 的 region chain，没有错误压平成 axis-aligned intersection。Opacity 按祖先到本节点逐层相乘；z-order 使用 Runtime 已建立的 `z-index + document order` 语义。Custom operation 与普通 draw operation 保持同一有序序列，因此 Live2D、particle、video 或 host texture 可以前后穿插，不要求先 CPU readback 或伪装为普通图片。

Scene 保存未变化的 primitive。普通 style/text/layout/custom 更新只重建 dirty node 的最多三个 primitive，并以 `BTreeSet` 增量维护绘制顺序；只有 hierarchy 改变才重新计算 document order。祖先 transform/opacity/clip 改变时，Runtime 已把受影响 subtree 标记为 render dirty，Scene 不另建第二套 invalidation。

## RenderGraph

`RenderGraph` 提供：

- external/transient resource 声明与 read/write/read-write access；
- explicit pass dependency；
- insertion-stable resource hazard dependency；
- deterministic topological compile；
- unknown resource/dependency、cycle、transient read-before-write 拒绝；
- `Draw` 与 `InvokeCustom` operation。

`UiScene::frame_graph` 生成默认 `ui-main` surface pass。通用 graph API 允许 backend/extension 增加 mask、effect、post-process 等 pass，不把这些 pass 硬编码进 Runtime。

## Iced/WGPU 兼容消费路径

Hosted Vue view 在构造 Iced tree 时编译当前 Scene frame graph，并只从 `InvokeCustom` operation 解析 `nana.host-texture` resource。`GpuTextureView` 随后在现有同一 WGPU Device/Queue/pass 中采样真实 host texture。

迁移期普通 Quad/Text 仍由现有 NanaUI/Iced widget 绘制，避免并行 painter 和视觉回退；custom resource identity/order 已来自同一 UiScene。祖先 opacity/transform/clip 仍由包围该节点的 Iced compatibility widgets 执行，因此 adapter 不再次应用 Scene 的累计 opacity，避免 double composition。后续 backend 可直接消费 Scene 的完整累计 paint state。

## 功能与平台门禁

- `nana-ui-runtime --all-features`：17 项通过；
- `nana-ui-scene --all-features`：5 项通过；
- `nana-ui-vue --features iced-view --lib`：378 项通过；
- `nana-ui-vue --features hosted --lib`：编译通过；
- Scene 与 Runtime `clippy -D warnings`：零问题；
- Iced compatibility boundary：通过；
- `nana-ui-scene` Android / Windows / Linux cross-check：通过；
- `cargo fmt` 与 `git diff --check`：通过。

Hosted build 仍报告 vendored `arboard` deprecated/unsafe warnings 和既有 `hosted_runtime` unused-mut warning；它们不是本阶段引入。交叉编译只证明 backend-neutral crate 可编译，不等同于真实平台窗口/GPU 验收。

## 性能门禁

报告见 [`performance/2026-08-14-issue7-phase6-scene.json`](performance/2026-08-14-issue7-phase6-scene.json)，release；local/idle 各 200 样本，frame graph 60 样本：

| 节点/primitive | Initial extraction | Local update P95 | Idle update P95 | Frame graph P95 |
| ---: | ---: | ---: | ---: | ---: |
| 100 | 0.241 ms | 0.001084 ms | 0.000042 ms | 0.0115 ms |
| 500 | 0.797 ms | 0.001000 ms | 0.000042 ms | 0.0280 ms |
| 1000 | 1.579 ms | 0.001084 ms | 0.000042 ms | 0.0605 ms |
| 5000 | 7.403 ms | 0.000792 ms | 0.000042 ms | 0.2989 ms |

Benchmark 断言 local update 不重建 order、只重建一个 primitive；idle delta 不重建任何 primitive。Frame graph 是每帧有 redraw 时的线性 operation plan，不在静止窗口运行。

## 阶段复核

多轮 review 修复了：Vue render dirty 被消费前丢弃、GPU slot 仍只存在 side table、Scene local mutation 全量 rebuild/sort、custom key 无验证、transient resource 未初始化读取，以及 Iced 对累计 opacity 二次相乘。

当前没有第二 renderer、第二 retained tree 或 backend object 泄漏。标准 Quad/Text 的 Iced 绘制仍是明确的兼容债务，将由 Phase 9 的退出指标处理；本阶段没有把“Scene 已存在”误报为“Iced 已完全退出”。Phase 6 的 Scene/RenderGraph 与真实 hosted custom GPU 消费门禁已满足，可以进入 Phase 7 的原生 RHI evidence gate。
