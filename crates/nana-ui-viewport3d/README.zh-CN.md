# `nana-ui-viewport3d`

[English](README.md)

NanaUI 的可选轻量 3D 预览视口。

`nana-ui-viewport3d` 复用 NanaUI 共享的 `GpuContext`，通过 `OrbitCamera` 将 `Scene3d` 渲染到 4x MSAA + `Depth32Float` 的离屏目标（预乘线性 scRGB），并发布到 `TextureSlot`，让 `GpuTextureView` 节点可以和普通界面控件一起参与布局、裁剪与绘制顺序。

- 内置几何体（`cube`、`sphere`、`cylinder`、`plane`、`ground_grid`）、`Material3d` 材质与 `LightRig` 光照
- 可选 `gltf` 特性（`load_glb_bytes` / `load_glb_file`）
- 射线拾取（`OrbitCamera`、`Ray`、`Hit3d`）
