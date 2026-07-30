# NanaUI

NanaUI 是 Nana 系列应用使用的 Rust 原生 UI 框架。当前基座为 Iced
`0.15.0-dev` 与 WGPU `30.0.0`，不依赖 WebView、DOM、CSS 或 JavaScript。

## 工作区框架

`crates/nana-ui` 以 LiliaUI 的视觉层级和工作区交互为基线，提供：

- `WorkspaceController`：统一管理 Region 布局、折叠、可见性、尺寸约束、
  分隔条拖动、双击复位、窗口尺寸和 DPI；
- `WorkspaceRegions` / `workspace_view`：按注册顺序接收任意 Region 内容，根据
  role、placement 与 scope 编排起始区、主区、结束区和上下区域；
- `WorkspaceSlots`：标准六区应用的便捷适配器，不限制框架的动态区域模型；
- `WorkspaceLayout`：提供稳定/自定义 Region ID、响应式策略、JSON 持久化以及
  逻辑/物理像素几何；
- `app_shell` / `app_title_bar`：提供可复用的应用壳层；
- light/dark 主题、基础控件、浮层和 WGPU 内容插槽。

`WorkspaceState` 只是可运行示例。Code、Github 与 Live2D 三套布局使用同一动态
Region 框架注册不同结构；节点添加、文档切换、搜索、预览刷新、参数调整、主题
切换和所有面板交互都连接到真实 Rust 状态，框架不依赖 Demo 的业务模型。

运行工作区 Demo：

```bash
cargo run -p nana-ui --example workspace-demo
```

运行组件状态画廊：

```bash
cargo run -p nana-ui --example component-gallery
```

运行自定义 WGPU 内容插槽：

```bash
cargo run -p nana-ui --example gpu-view-demo
```

运行由宿主掌握窗口与 WGPU 上下文的组合 Demo：

```bash
cargo run -p nana-ui --example hosted-gpu-demo
```

`hosted-gpu-demo` 由宿主创建 `winit::Window`、事件循环、WGPU
`Instance`、`Device`、`Queue` 与 `Surface`。Iced 和宿主场景复用同一 GPU
上下文，宿主纹理通过 `GpuTextureView` 直接进入 UI 合成，没有 CPU 回读、
图片编码或第二套 Device。

## 当前依赖基线

- Iced：`0.15.0-dev`，基于 `sena-nana/iced`；
- WGPU：`30.0.0`，依赖图中只有一个 WGPU 主版本；
- Cryoglyph：随 Iced 分叉同步迁移到 WGPU 30；
- Rust edition：2024，最低 Rust `1.92`。

开发工作区暂时通过相邻的 `../iced` 与 `../cryoglyph` 本地路径验证三个仓库的
同步修改；在变更经确认并形成固定 commit 后，再改为可复现的 Git revision。

## 验证

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

生成 Workspace 与 Component Gallery 的 dark/light 验收快照：

```bash
cargo run --release -p nana-ui --example ui-snapshots --locked
```

PNG 输出到 `target/ui-snapshots`。快照工具会执行 GPU→CPU 读取；正式窗口、
`GpuView` 和 `GpuTextureView` 渲染路径不会使用该流程。

Issue #1 的实现与验收记录见
[`docs/issue-1-acceptance.md`](docs/issue-1-acceptance.md)。本阶段没有修改
NanaShader。
