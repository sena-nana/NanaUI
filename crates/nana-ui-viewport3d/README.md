# `nana-ui-viewport3d`

[中文](README.zh-CN.md)

Optional lightweight 3D preview viewport for NanaUI.

`nana-ui-viewport3d` renders a `Scene3d` from an `OrbitCamera` into an offscreen 4x MSAA + `Depth32Float` target (premultiplied linear scRGB) on NanaUI's shared `GpuContext` and publishes frames to a `TextureSlot` so a `GpuTextureView` node can participate in layout, clipping, and document order alongside ordinary UI controls.

- Primitives (`cube`, `sphere`, `cylinder`, `plane`, `ground_grid`), PBR materials (`Material3d`), and `LightRig`
- Optional `gltf` feature (`load_glb_bytes` / `load_glb_file`)
- Ray-cast picking (`OrbitCamera`, `Ray`, `Hit3d`)
