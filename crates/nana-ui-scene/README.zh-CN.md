# nana-ui-scene

[English](README.md) · 简体中文

把运行时的界面树变成可绘制的场景，不依赖 WGPU。

`RuntimeDocument` 持有一个文档的 `AppContext` 和 `UiScene`。每帧调用 `flush`，它依次完成样式、文本、布局、抽取，再把变化应用到 `UiScene`。场景按节点和槽位保留图元，只重建脏子树，纯滚动走平移快路径。合成层、透明度组和渲染图也在这里决定，产出的都是纯数据。

设备、纹理和管线在绘制时由 [`nana-ui`](../nana-ui) 的 `SceneWgpuPainter` 解析，布局和命中测试在 [`nana-ui-runtime`](../nana-ui-runtime)。

## 示例

```rust
use nana_ui_runtime::{DocumentId, LayoutViewport, MeasureTextShaper, Stack};
use nana_ui_scene::RuntimeDocument;

let id = DocumentId::new(1).unwrap();
let mut runtime = RuntimeDocument::new(id);
runtime
    .context_mut()
    .create_component(id, Stack::column(0.0))
    .unwrap();

runtime
    .flush(LayoutViewport::new(320.0, 240.0), &mut MeasureTextShaper)
    .unwrap();
let scene = runtime.scene();
```

`MeasureTextShaper` 是不经过字体的近似测量；要按真实字体排版，用 `nana_ui_runtime::NanaTextEngineShaper`。

## Features

默认不开启任何 feature（`default = []`）。

- `icu` 与 `calendar`、`charts`、`controls`、`graph-canvas`、`image-viewer`、`rich-text`、`components`（均默认关闭，按需启用）：转发到 `nana-ui-runtime` 的同名 feature，其中 `components` 和 `rich-text` 还会开启 `vector-boolean`。
- `vector-boolean`（可选）：在 `custom_paint` 中启用基于 `i_overlay` 的矢量路径布尔化简；关闭时不链接 `i_overlay`，描边、填充、裁剪与轮廓扩张改用内置整数平面扫描线布尔回退求解器（同样支持自交多边形、`EvenOdd`/`NonZero` 填充、嵌套孔洞与孤岛、裁剪交/差集与 miter 轮廓化简）。
- `benchmark`：只用于构建基准。

## 测试

```bash
cargo test -p nana-ui-scene
cargo run --release -p nana-ui-scene --features benchmark --bin nana-dirty-frame-benchmark
```

部分集成测试需要对应的控件族 feature，否则编译为空并显示通过。其余命令见 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。

更多内容见 [Scene 提取](../../docs/learn/scene-extraction.md)、[布局与特性迁移指南](../../docs/guide/migration-layout-and-features.md) 和 [Runtime 与 Scene 合同](../../docs/reference/runtime-scene.md)。
