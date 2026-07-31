# NanaUI

NanaUI 是 Nana 系列应用使用的 Rust 原生 UI 框架。当前基座为 Iced
`0.15.0-dev` 与 WGPU `30.0.0`，不依赖 WebView、DOM、CSS 或 JavaScript。

## 工作区框架

`crates/nana-ui` 以 LiliaUI 的视觉层级和工作区交互为基线，提供：

- `WorkspaceController`：统一管理 Region 布局、折叠、可见性、尺寸约束、
  分隔条拖动、双击复位、按需帧驱动的折叠过渡、窗口尺寸和 DPI；
- `WorkspaceRegions` / `workspace_view`：按注册顺序接收任意 Region 内容，根据
  role、placement 与 scope 编排起始区、主区、结束区和上下区域；
- `WorkspaceSlots`：标准六区应用的便捷适配器，不限制框架的动态区域模型；
- `WorkspaceLayout`：提供稳定/自定义 Region ID、响应式策略、JSON 持久化以及
  逻辑/物理像素几何；
- `SidebarFrame` / `SidebarSection` / `SidebarRow` / `SidebarFooter`：
  提供可滚动主体、固定外槽、带高度与箭头过渡的分区折叠、层级导航与真实
  Footer 操作；
- `SettingsModel` / `SettingsState`：提供稳定设置 Tab、别名恢复、普通/full-page
  页面和可序列化导航状态；
- `AppearanceSettings` / `ThemeTokens`：提供可序列化的标准圆角设置，内部派生
  四档圆角，并让宿主以 `UI_METRICS` 为默认值即时覆盖公共控件外观；
- `settings_sidebar` / `settings_page` / `SettingsRow` / `SettingsCard`：
  提供与 LiliaUI 一致的紧凑设置布局；
- `app_shell` / `app_title_bar`：提供可复用的应用壳层；
- light/dark 主题、LiliaUI 同源的 Noto Sans SC 400/500/600/700 字体、
  基础控件、浮层和 WGPU 内容插槽。

`GalleryState` 是唯一的标准 UI 示例。它以单个 220px `Resources` Region 承载
控件、表面、反馈和工作区分类及固定设置 Footer；进入设置后，同一位置切换为设置
分类。工作区分类会启用 Toolbar、Inspector 与 Bottom Region，其他分类保持简洁的
侧栏 + 主内容结构。主题、面板显隐、尺寸复位、返回导航和所有组件操作都连接到
真实 Rust 状态，框架不依赖 Gallery 的展示模型。

`ThemeMode`、`AppearanceSettings`、`SettingsState` 与 `WorkspaceLayout` 均可
序列化。NanaUI 不选择配置目录或自行写盘，消费应用负责将这些状态组合进自己的
配置文件。

消费应用创建 Iced application 时应注册 `ui_font_sources()` 并将
`ui_font(Normal)` 设为默认字体；仓库中的所有 Demo 和离屏 renderer 已执行该
注册。字体文件由 LiliaUI 使用的同一组 Noto Sans SC WOFF2 无损转换为 TTF，
许可见 `crates/nana-ui/assets/fonts/OFL.txt`。

运行 UI Gallery：

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

- Iced：`0.15.0-dev`，固定到 `sena-nana/iced`
  `f6fddd3ce0bc123ec64ce77c1839d56dc8465ba6`；
- WGPU：`30.0.0`，依赖图中只有一个 WGPU 主版本；
- Cryoglyph：随 Iced 分叉同步迁移到 WGPU 30，并固定到
  `sena-nana/cryoglyph` `3fe41b131eda1288d08df89ad5ba56de97713308`；
- Rust edition：2024，最低 Rust `1.92`。

Iced 与 Cryoglyph 均使用完整 Git revision，`Cargo.lock` 记录解析后的 commit。
GitHub Actions 从独立 checkout 运行 `--locked` 测试与全目标检查，不依赖相邻
仓库。

## 验证

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

生成 Gallery 的 Workspace、组件状态与 dark/light 验收快照：

```bash
cargo run --release -p nana-ui --example ui-snapshots --locked
```

PNG 输出到 `target/ui-snapshots`。快照工具会执行 GPU→CPU 读取；正式窗口、
`GpuView` 和 `GpuTextureView` 渲染路径不会使用该流程。

Issue #1 的实现与验收记录见
[`docs/issue-1-acceptance.md`](docs/issue-1-acceptance.md)。本阶段没有修改
NanaShader。
