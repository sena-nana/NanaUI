# nana-ui

[English](README.md) · 简体中文

NanaUI 的门面 crate，也是应用直接依赖的那一个。

它提供应用入口（`NanaApplication`、`RuntimeApplication`、`RuntimeProgram`）、基于 winit 和 `nana-window` 的桌面宿主，以及把 `UiScene` 画进宿主 WGPU 帧的 `SceneWgpuPainter`。控件、视图和 `AppContext` 经 `nana_ui::runtime` 重新导出，`nana-gpu` 的 `GpuContext`、`FrameContext` 等合同类型也从这里拿。宿主纹理和 `CustomRenderNode` 让 GPU 画面作为普通节点进入界面树。

保留树、组件和布局在 [`nana-ui-runtime`](../nana-ui-runtime)，场景在 [`nana-ui-scene`](../nana-ui-scene)，本 crate 只负责把它们接到窗口和 GPU 上。

## 示例

```rust
fn main() -> Result<(), nana_ui::HostedRunError> {
    let identity = ApplicationIdentity::new(
        "dev.nanaui.counter",
        "NanaUI Counter",
        env!("CARGO_PKG_VERSION"),
    );
    NanaApplication::builder(identity)
        .diagnostics(DiagnosticsConfig::default())
        .run::<RuntimeApplication<Counter>>(
            WindowDescriptor::new("NanaUI Counter").initial_size(480.0, 320.0),
        )
}
```

`Counter` 实现了 `ApplicationState`，完整代码见 `examples/application-counter.rs`：

```bash
cargo run -p nana-ui --example application-counter --features hosted,bundled-fonts
```

## Features

默认不开启任何 feature（`default = []`）。桌面应用通常按需启用 `hosted`（及 `bundled-fonts`、`view-macro` 等所需特性），或直接使用桌面全量集合 `full`。

- `hosted`：桌面宿主，包括窗口、Surface、帧调度和平台无障碍适配，隐含 `gpu`。
- `gpu`：`SceneWgpuPainter`、宿主纹理、帧交换和无窗口输出。默认仅编译平台主图形后端（Apple 为 `metal`、Windows 为 `dx12`、Unix 为 `vulkan`）；如需全套标准后端可加 `wgpu-backends`。
- `wgpu-backends`（可选）：在启用 `gpu` 时打开全部跨平台标准 WGPU 后端（`dx12`、`vulkan`、`gles`、`metal`）。
- `icu`（可选）：用 ICU4X 做数字、日期和复数格式化（关闭时使用 `PlainFormatter`）。
- `snippet-transforms`（可选）：snippet 里的正则变换（`${1/(.*)/${1:/upcase}/}`）。关闭后 snippet 照常展开和联动，变换原样返回，不链接正则引擎（约省 0.7 MiB），首次命中正则变换时发出 `runtime.snippet.transform_unavailable` 诊断事件。
- `svg`（可选）：SVG 图片，即 `url()` 纹理和 `Image` 里的 SVG 文档，以及宿主包中只有 SVG、没有 shapes 的图标。内置矢量图标按自己的几何绘制，不需要它。关闭后 SVG 图片按不可解码文件处理，并发出 `resource.image_codec_unavailable` 事件，`usvg`、`resvg`、`tiny-skia`（约 0.8 MiB）不进二进制。只在 `gpu` 开启时生效。
- `http-fetch`（可选）：打开 `nana-ui-platform/fetch`，支持内置 `http(s)://` 网络图片拉取；关闭且未提供自定义 `FetchHost` 时请求被拒绝并发出 `resource.fetch_unavailable`。
- `image-codecs-extra`（可选）：打开 `image` 的 JPEG、GIF、WebP 解码（默认保留 PNG）；关闭时遇对应格式会跳过并发出 `resource.image_codec_unavailable`。
- `runtime-wgsl`（可选）：打开运行时 WGSL 编译（`naga` / `wgpu` WGSL 回退；`full` 包含此项）。关闭时仅加载预编译着色器归档（macOS 14+ / iOS 17+ 为 `stages.msl.zz`，其余平台为压缩 Naga IR）。
- `vector-boolean`（可选）：在 `nana-ui-scene` 中启用 `i_overlay` 路径布尔运算（`components` 与 `rich-text` 会启用）；关闭时自定义绘制改用内置整数平面扫描线布尔回退求解器，支持自交、孔洞/孤岛、裁剪交差、描边与 miter 轮廓化简。
- `full`：桌面全量集合，打开 `bundled-fonts`、`components`、`hosted`、`http-fetch`、`icu`、`image-codecs-extra`、`runtime-wgsl`、`snippet-transforms`、`svg`、`syntax-highlighting` 和 `wgpu-backends`。
- `bundled-fonts`：注册内置的 Noto Sans SC 并设为界面默认字体（约 4.2 MiB）。不开则界面文字与 SVG 文字都用系统字体；缺中日韩字体的 Linux 上会显示不出中文。
- `components`：打开 `calendar`、`charts`、`controls`、`diff-view`、`dock`、`graph-canvas`、`image-viewer`、`qr-code`、`rich-text`、`terminal` 与 `vector-boolean`。`dock` 同时带上宿主适配 `nana_ui::dock` 与 `runtime_dock_window_update`。
- `syntax-highlighting`：代码块语法高亮。
- `view-macro`：`view!`、`css!` 和 `stylesheet!` 宏。
- `icons-tabler`：完整的 Tabler 图标目录（`nana_ui::icons_tabler`）。
- `packaged-resources`：挂载 `.nrpack` 资源包并响应 `NANA_PACKAGE_VALIDATE` 自检。
- `wgpu-interop`：访问 `GpuContext` 背后的 WGPU 对象，自己录制管线时才需要。
- `native-export`（仅 Windows）：把窗口输出导出为 DX12 共享纹理。
- `accesskit-tree`：只产出 AccessKit `TreeUpdate`，给自己接平台适配器的宿主用。
- `reactive-trace`：记录响应式更新的因果链，用于调试。
- `no-default-app-icon`：不链接默认应用图标，只应由应用二进制打开。

## 测试

```bash
cargo test -p nana-ui --lib --features hosted,gpu -- --test-threads=1
```

不带 `gpu` / `hosted` 时，这两个 feature 后面的模块不会编译，测试也不会运行。GPU 相关测试需要可用的适配器，其余命令见 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。

更多内容见 [快速开始](../../docs/guide/quick-start.md)、[应用 API](../../docs/reference/application-api.md)、[布局与特性迁移指南](../../docs/guide/migration-layout-and-features.md) 和 [实时画面与 GPU 集成](../../docs/reference/gpu.md)。
