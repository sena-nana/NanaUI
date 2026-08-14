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
- `Text`：内容、颜色、size、weight、family、line height、letter spacing、wrap/ellipsis、水平/垂直对齐；其 bounds 是扣除 border/padding 后的 content box；
- `Custom`：renderer extension key、resource key、revision；
- 每个 primitive 共有 stable `PrimitiveId`、layout bounds、累计 affine transform、祖先 clip chain、累计 opacity、z-index 与 document order。

Clip 保留为带 transform 的 region chain，没有错误压平成 axis-aligned intersection。Opacity 按祖先到本节点逐层相乘；z-order 使用 Runtime 已建立的 `z-index + document order` 语义。Custom operation 与普通 draw operation 保持同一有序序列，因此 Live2D、particle、video 或 host texture 可以前后穿插，不要求先 CPU readback 或伪装为普通图片。

Scene 保存未变化的 primitive。普通 style/text/layout/custom 更新只重建 dirty node 自己的有限 primitive 集，并以 `BTreeSet` 增量维护绘制顺序；只有 hierarchy 改变才重新计算 document order。Checkbox/Switch/Slider 由 backend-neutral `StandardVisual` 展开为 indicator/track/fill/thumb，标签和标记拥有独立前景通道，不由 backend 匹配 element tag。祖先 transform/opacity/clip/scroll 改变时，Runtime 已把受影响 subtree 标记为 render dirty，Scene 不另建第二套 invalidation。scrollport clip 在未滚动 viewport transform 建立，只有其后代叠加负 offset，因此 viewport 本身不会跟随内容漂移。

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

`nana-ui::IcedSceneView` 现可把同一 UiScene 的标准 Quad/Text 直接画入 Iced compatibility renderer；Runtime Button → computed theme paint → extraction → UiScene → painter 有功能测试覆盖。它借用 retained `UiScene`，不复制 primitive、也不持有 widget retained state；input 仍由 UiWorld 处理。adapter 兑现 content-box padding、line-height、wrap/ellipsis 和 text anchor；对 Iced 无法忠实表达的 custom primitive、非 translation affine/clip、letter spacing 与不支持的命名字体返回显式 `ScenePaintError`，禁止静默漏画或近似降级。

Component Gallery snapshot runner 增加由 Runtime `Text`/`TextInput`/`Button`/`Table`/`Checkbox`/`Switch`/`Slider`/`Tab`/`ScrollView` 直接生成的 dark/light Scene 样例，并允许通过 `NANA_UI_SNAPSHOT_OUTPUT` 写到隔离目录。2026-08-14 在 macOS/WGPU headless MSAA×4 实际生成并检查两张 640×500 图；多轮检查依次发现并修复 Iced text anchor、selected label/indicator 前景混用与 scroll 首行裁切过重，最终复验输入 padding、按钮居中、表格 content box、toggle/slider 标记、tab selected state、scroll transform/viewport clip 与亮/暗主题均正确。验收输出位于临时目录，未覆盖仓库中用户已有的 `docs/ui-snapshots/`。

现有 Vue gallery 仍由既有 NanaUI/Iced widgets 绘制，避免在视觉 snapshot parity 前切换默认 painter；custom resource identity/order 已来自同一 UiScene。`IcedSceneView` 是 Nana-native 标准 primitive 的真实 compatibility painter，不等于现有全部组件已迁移，也不处理 custom GPU extension。

## 功能与平台门禁

- `nana-ui-runtime --all-features`：40 项通过；
- `nana-ui-scene --all-features`：8 项通过，包含 text content-box/alignment/line-height/wrap/ellipsis、scroll clip/transform 与 standard control visual contract；
- `nana-ui --lib --no-default-features`：110 项通过，含 Runtime Button 到标准 Scene painter 与 platform input adapter 链路；
- `nana-ui-vue --features iced-view --lib`：384 项通过；
- `nana-ui-vue --features hosted --lib`：编译通过；
- Scene 与 Runtime `clippy -D warnings`：零问题；
- Iced compatibility boundary：通过；
- `nana-ui-scene` backend-neutral portability check：通过；
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

多轮 review 修复了：Vue render dirty 被消费前丢弃、GPU slot 仍只存在 side table、Scene local mutation 全量 rebuild/sort、custom key 无验证、transient resource 未初始化读取、Iced 对累计 opacity 二次相乘、文本丢失 content-box/line-height/wrap 语义、错误 anchor 导致居中文字裁切、scroll clip 随内容错误平移，以及 compatibility view 每次构造复制全部 primitive。

当前没有第二 retained tree 或 backend object 泄漏。标准 Quad/Text 已有直接、零复制的 Scene compatibility painter和可复现 dark/light 离屏证据，但完整 affine/letter-spacing/named-font、custom extension 与既有全组件 gallery parity 仍是明确债务；本阶段没有把基础组件 painter 已存在误报为“Iced 已完全退出”。Phase 6 的 Scene/RenderGraph 与真实 hosted custom GPU 消费门禁已满足，可以进入 Phase 7 的原生 RHI evidence gate。
